//! 开机自启：通过「计划任务（最高权限）」实现静默自启，避免每次开机 UAC。
//!
//! 为什么不用注册表 Run：注册表自启只能以普通权限运行，而内存清理必须管理员权限。
//! 计划任务以 HighestAvailable 运行，登录时静默启动到托盘。
//!
//! v1.1.1 加固：
//! - `<Command>` 不再对含空格路径加引号（Task Scheduler 的 Command 是路径文本，
//!   加引号会导致任务启动失败）；
//! - `status_for()` 会读取任务 XML，校验任务指向的 exe 路径是否与当前进程一致，
//!   用于「任务缺失/指向旧路径」时自动修复；
//! - `uninstall()` 幂等：任务本来不存在时也视为成功；
//! - `install()` 创建后立即回读校验，失败返回可读错误。

use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 计划任务名。
pub const TASK_NAME: &str = "MemCleaner AutoStart";

/// 自启状态。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutostartStatus {
    Installed,
    NotInstalled,
    /// 任务存在，但指向的 exe 路径与当前程序不一致（例如程序被移动过）。
    Mismatched(String),
    /// 查询失败（计划任务服务异常等）。
    Error(String),
}

impl AutostartStatus {
    pub fn is_installed(&self) -> bool {
        matches!(self, AutostartStatus::Installed)
    }
}

/// 查询计划任务是否存在（不校验路径）。
/// 查询用户级自启状态（不校验路径）。
pub fn status() -> AutostartStatus {
    match read_run_value() {
        Ok(Some(_)) => AutostartStatus::Installed,
        Ok(None) => AutostartStatus::NotInstalled,
        Err(e) => AutostartStatus::Error(e),
    }
}

/// 查询任务并校验它指向的 exe 是否就是 `exe_path`。
/// 查询用户级自启是否指向 `exe_path`。
pub fn status_for(exe_path: &str) -> AutostartStatus {
    match read_run_value() {
        Ok(Some(value)) => {
            if path_matches(&value, exe_path) {
                AutostartStatus::Installed
            } else {
                AutostartStatus::Mismatched(value)
            }
        }
        Ok(None) => AutostartStatus::NotInstalled,
        Err(e) => AutostartStatus::Error(e),
    }
}

/// 读取计划任务 XML 里的 `<Command>` 内容；任务不存在/查询失败返回 None。
pub fn task_command() -> Option<String> {
    let mut cmd = Command::new("schtasks.exe");
    cmd.args(["/Query", "/TN", TASK_NAME, "/XML"]);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    // schtasks 输出为系统 ANSI/UTF-8 混合，这里按 Windows 中文环境常见的 GBK 优先尝试。
    let text = decode_console(&out.stdout);
    extract_xml_tag(&text, "Command").map(|s| xml_unescape(s.trim()))
}

/// 安装（或更新）自启计划任务。需要管理员权限。
///
/// 创建后会回读任务 XML 校验路径；失败时返回可读错误。
/// 安装用户级自启（HKCU Run，不需要管理员），并清理旧版计划任务。
pub fn install(exe_path: &str) -> Result<(), String> {
    if exe_path.trim().is_empty() {
        return Err("程序路径为空".to_string());
    }
    let command = format!("\"{}\" --autostart", exe_path.trim());
    write_run_value(&command)?;
    match read_run_value() {
        Ok(Some(v)) if v == command => {}
        Ok(Some(v)) => return Err(format!("开机自启写入后不一致：{v}")),
        Ok(None) => return Err("开机自启写入后查询不到（可能被安全软件拦截）".to_string()),
        Err(e) => return Err(e),
    }
    if crate::sys::is_elevated() {
        let _ = uninstall_old_task();
        let _ = uninstall_clean_task();
    }
    Ok(())
}

/// 卸载自启计划任务。需要管理员权限；任务不存在时视为成功（幂等）。
/// 卸载用户级自启（HKCU Run，幂等），并清理旧版计划任务。
pub fn uninstall() -> Result<(), String> {
    delete_run_value()?;
    if crate::sys::is_elevated() {
        let _ = uninstall_old_task();
        let _ = uninstall_clean_task();
    }
    Ok(())
}

/// 启动一个计划任务（用于自检/验证）。需要管理员权限。
pub fn run_task() -> Result<(), String> {
    if run_schtasks(&["/Run", "/TN", TASK_NAME])? {
        Ok(())
    } else {
        Err("运行计划任务失败".to_string())
    }
}

fn run_schtasks(args: &[&str]) -> Result<bool, String> {
    let mut cmd = Command::new("schtasks.exe");
    cmd.args(args);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    match cmd.output() {
        Ok(out) => Ok(out.status.success()),
        Err(e) => Err(format!("无法运行 schtasks.exe：{e}")),
    }
}

/// 当前用户标识（DOMAIN\\user）。
fn current_user_id() -> String {
    let domain = std::env::var("USERDOMAIN").unwrap_or_default();
    let user = std::env::var("USERNAME").unwrap_or_default();
    if domain.is_empty() {
        user
    } else {
        format!("{domain}\\{user}")
    }
}

/// 把 exe 路径规范化后比较（去引号、统一斜杠、忽略大小写）。
pub fn path_matches(run_value: &str, exe_path: &str) -> bool {
    normalize_path(extract_exe_path(run_value))
        == normalize_path(exe_path.trim().trim_matches('"'))
        || normalize_path(run_value) == normalize_path(exe_path)
}

/// 从「"C:\path\app.exe" --args」里取出 exe 路径。
fn extract_exe_path(s: &str) -> &str {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return &t[1..end + 1];
        }
    }
    t.split_whitespace().next().unwrap_or("")
}

fn normalize_path(s: &str) -> String {
    s.trim()
        .trim_matches('"')
        .trim()
        .replace('/', "\\")
        .to_lowercase()
}

/// 从 XML 文本里提取第一个 `<tag>...</tag>` 的内容（标签可能带命名空间前缀）。
fn extract_xml_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].to_string())
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Windows 控制台输出解码：优先 UTF-8，失败则按系统 ANSI 代码页（中文系统为 GBK）。
///
/// schtasks 的 `/XML` 输出走控制台代码页；程序路径可能含中文，必须正确解码才能做路径比对。
fn decode_console(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }
    unsafe {
        use windows_sys::Win32::Globalization::MultiByteToWideChar;
        const CP_ACP: u32 = 0;
        let len = bytes.len() as i32;
        let need = MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0);
        if need > 0 {
            let mut buf = vec![0u16; need as usize];
            let n = MultiByteToWideChar(
                CP_ACP,
                0,
                bytes.as_ptr(),
                len,
                buf.as_mut_ptr(),
                need,
            );
            if n > 0 {
                return String::from_utf16_lossy(&buf[..n as usize]);
            }
        }
    }
    // 最后兜底：逐字节映射（只用于 ASCII 标签解析，不影响正常中文系统）
    bytes.iter().map(|b| *b as char).collect()
}

/// 生成计划任务 XML。
fn task_xml(exe_path: &str, user: &str) -> String {
    let exe = xml_escape(exe_path.trim());
    let user = xml_escape(user);
    let working_dir = std::path::Path::new(exe_path)
        .parent()
        .map(|p| xml_escape(&p.to_string_lossy()))
        .unwrap_or_default();
    // 注意：<Command> 是路径文本，不能加引号（含空格也一样），否则任务启动会失败。
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>MemCleaner 内存清理：登录时以管理员权限静默启动到托盘，用于自动清理内存。</Description>
    <URI>\{TASK_NAME}</URI>
  </RegistrationInfo>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
    </LogonTrigger>
  </Triggers>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>--autostart</Arguments>
      <WorkingDirectory>{working_dir}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ---------------------------------------------------------------------------
// 用户级自启（HKCU\Software\Microsoft\Windows\CurrentVersion\Run）
// ---------------------------------------------------------------------------

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "MemCleaner";

/// 读取用户级自启命令行。
pub fn user_autostart_command() -> Option<String> {
    read_run_value().ok().flatten()
}

/// 删除旧版计划任务 `MemCleaner AutoStart`（需管理员，幂等）。
pub fn uninstall_old_task() -> Result<(), String> {
    if run_schtasks(&["/Delete", "/TN", TASK_NAME, "/F"]).unwrap_or(false) {
        return Ok(());
    }
    if run_schtasks(&["/Query", "/TN", TASK_NAME]).unwrap_or(false) {
        Err("删除旧版计划任务失败（权限不足）".to_string())
    } else {
        Ok(())
    }
}

/// 清理 v1.1.1 可能存在的清理任务（幂等）。
pub fn uninstall_clean_task() -> Result<(), String> {
    let _ = run_schtasks(&["/Delete", "/TN", "MemCleaner Clean", "/F"]);
    Ok(())
}

/// 旧版计划任务是否存在（迁移/诊断用）。
pub fn old_task_exists() -> bool {
    run_schtasks(&["/Query", "/TN", TASK_NAME]).unwrap_or(false)
}

#[cfg(windows)]
fn read_run_value() -> Result<Option<String>, String> {
    use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE,
        REG_SZ,
    };
    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        let path = super::wide(RUN_KEY);
        let rc = RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_QUERY_VALUE, &mut key);
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        if rc != 0 {
            return Err(format!("打开注册表失败（{rc}）"));
        }
        let name = super::wide(RUN_VALUE);
        let mut ty: u32 = 0;
        let mut size: u32 = 0;
        let rc = RegQueryValueExW(
            key, name.as_ptr(), std::ptr::null_mut(), &mut ty, std::ptr::null_mut(), &mut size,
        );
        if rc == ERROR_FILE_NOT_FOUND || size == 0 {
            RegCloseKey(key);
            return Ok(None);
        }
        if rc != 0 {
            RegCloseKey(key);
            return Err(format!("查询自启项失败（{rc}）"));
        }
        let mut buf = vec![0u8; size as usize + 2];
        let mut real_size = size;
        let rc = RegQueryValueExW(
            key, name.as_ptr(), std::ptr::null_mut(), &mut ty, buf.as_mut_ptr(), &mut real_size,
        );
        RegCloseKey(key);
        if rc != 0 {
            return Err(format!("读取自启项失败（{rc}）"));
        }
        if ty != REG_SZ {
            return Err("自启项类型异常".to_string());
        }
        let u16s: Vec<u16> = buf[..real_size as usize]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|c| *c != 0)
            .collect();
        Ok(Some(String::from_utf16_lossy(&u16s)))
    }
}

#[cfg(windows)]
fn write_run_value(value: &str) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };
    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        let path = super::wide(RUN_KEY);
        let mut disposition = 0u32;
        let rc = RegCreateKeyExW(
            HKEY_CURRENT_USER, path.as_ptr(), 0, std::ptr::null_mut(),
            REG_OPTION_NON_VOLATILE, KEY_SET_VALUE, std::ptr::null_mut(), &mut key, &mut disposition,
        );
        if rc != 0 {
            return Err(format!("打开注册表失败（{rc}）"));
        }
        let name = super::wide(RUN_VALUE);
        let data: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = std::slice::from_raw_parts(
            data.as_ptr() as *const u8,
            data.len() * std::mem::size_of::<u16>(),
        );
        let rc = RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, bytes.as_ptr(), bytes.len() as u32);
        RegCloseKey(key);
        if rc != 0 {
            return Err(format!("写入自启项失败（{rc}），可能被安全软件拦截"));
        }
        Ok(())
    }
}

#[cfg(windows)]
fn delete_run_value() -> Result<(), String> {
    use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE,
    };
    unsafe {
        let mut key: HKEY = std::ptr::null_mut();
        let path = super::wide(RUN_KEY);
        let rc = RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_SET_VALUE, &mut key);
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        if rc != 0 {
            return Err(format!("打开注册表失败（{rc}）"));
        }
        let name = super::wide(RUN_VALUE);
        let rc = RegDeleteValueW(key, name.as_ptr());
        RegCloseKey(key);
        if rc == 0 || rc == ERROR_FILE_NOT_FOUND { Ok(()) } else { Err(format!("删除自启项失败（{rc}）")) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_contains_expected_bits() {
        let xml = task_xml(r"C:\Program Files\MemCleaner\MemCleaner.exe", r"PC\admin");
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(xml.contains("--autostart"));
        assert!(xml.contains(r"<UserId>PC\admin</UserId>"));
        assert!(xml.contains("<LogonTrigger>"));
        // v1.1.1: Command 不能带引号（含空格路径也一样）
        assert!(xml.contains(r"<Command>C:\Program Files\MemCleaner\MemCleaner.exe</Command>"));
        assert!(!xml.contains(r#"<Command>""#));
        assert!(xml.contains(r"<WorkingDirectory>C:\Program Files\MemCleaner</WorkingDirectory>"));
    }

    #[test]
    fn xml_escapes_special_chars() {
        let xml = task_xml(r"C:\a&b\c.exe", "u<ser");
        assert!(xml.contains("a&amp;b"));
        assert!(xml.contains("u&lt;ser"));
    }

    #[test]
    fn path_matching_handles_run_value_with_args() {
        assert!(path_matches(
            r#""C:\Program Files\MemCleaner\MemCleaner.exe" --autostart"#,
            r"C:\Program Files\MemCleaner\MemCleaner.exe"
        ));
        assert!(path_matches(
            r"C:\Tools\MemCleaner.exe --autostart",
            r"c:\tools\memcleaner.exe"
        ));
    }

    #[test]
    fn path_matching_normalizes_quotes_slashes_case() {
        assert!(path_matches(
            r#""C:\Program Files\MemCleaner\MemCleaner.exe""#,
            r"C:\Program Files\MemCleaner\MemCleaner.exe"
        ));
        assert!(path_matches(
            "C:/Program Files/MemCleaner/MemCleaner.exe",
            r"c:\program files\memcleaner\memcleaner.exe"
        ));
        assert!(!path_matches(r"C:\old\MemCleaner.exe", r"C:\new\MemCleaner.exe"));
    }

    #[test]
    fn xml_tag_extraction_and_unescape() {
        let xml = r#"<Task><Actions><Exec><Command>C:\a&amp;b\MemCleaner.exe</Command></Exec></Actions></Task>"#;
        let cmd = extract_xml_tag(xml, "Command").unwrap();
        assert_eq!(xml_unescape(cmd.trim()), r"C:\a&b\MemCleaner.exe");
        assert!(extract_xml_tag(xml, "Arguments").is_none());
    }

    #[test]
    fn decode_console_handles_utf8() {
        let s = decode_console("任务".as_bytes());
        assert!(s.contains("任务"));
    }
}