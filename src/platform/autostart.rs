//! 开机自启：通过「计划任务（最高权限）」实现登录时静默以管理员权限启动。
//!
//! ## 为什么必须是计划任务（v1.3.0 结论）
//!
//! 注册表 `HKCU\...\Run` 只能以普通权限启动进程，而内存清理必须管理员权限。
//! v1.2.0 曾改回 Run 键，导致「开机自启后清理提示需要管理员」——m00645 用户反馈。
//! 因此 v1.3.0 恢复计划任务作为唯一自启方式：
//! - `RunLevel=HighestAvailable` + `LogonType=InteractiveToken`：登录时由任务计划程序
//!   直接以管理员令牌启动（High 完整性），不弹 UAC，能直接清理内存；
//! - 安装/卸载任务需要管理员权限，普通权限下由界面引导用户 UAC 提权后再写。
//!
//! v1.1.1 的经验继续保留：
//! - `<Command>` 不加引号（Task Scheduler 的 Command 是路径文本）；
//! - 安装后回读 XML 校验路径，失败返回可读错误；
//! - 卸载幂等（任务/注册表项不存在也视为成功）；
//! - 顺带清掉 v1.2.0 遗留的 Run 键，避免任务 + Run 双启动。

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
    /// 最高权限计划任务已安装且指向当前 exe：登录后管理员权限静默启动。
    InstalledTask,
    /// 仅存在 v1.2.0 用户级 Run 项（普通权限启动；清理时仍需手动提权）。
    InstalledRun(String),
    NotInstalled,
    /// 存在自启项但指向的 exe 路径与当前程序不一致（例如程序被移动过）。
    Mismatched(String),
    /// 查询失败（计划任务服务异常等）。
    Error(String),
}

impl AutostartStatus {
    pub fn is_installed(&self) -> bool {
        matches!(
            self,
            AutostartStatus::InstalledTask | AutostartStatus::InstalledRun(_)
        )
    }

    /// 是否已实现「管理员权限静默自启」。
    pub fn is_elevated(&self) -> bool {
        matches!(self, AutostartStatus::InstalledTask)
    }
}

/// 查询当前 exe 的自启状态。
pub fn status() -> AutostartStatus {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    status_for(&exe)
}

/// 查询计划任务并校验它指向的 exe 是否就是 `exe_path`。
pub fn status_for(exe_path: &str) -> AutostartStatus {
    match task_command() {
        Some(cmd) => {
            if path_matches(&cmd, exe_path) {
                return AutostartStatus::InstalledTask;
            }
            return AutostartStatus::Mismatched(cmd);
        }
        None => {
            if task_exists_unknown() {
                // 任务存在但读不到 XML：无法确认路径，按未安装处理并由安装流程覆盖。
                return AutostartStatus::Mismatched(String::new());
            }
        }
    }
    match read_run_value() {
        Ok(Some(value)) => {
            if path_matches(&value, exe_path) {
                AutostartStatus::InstalledRun(value)
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
    let xml = query_task_xml()?;
    extract_xml_tag(&xml, "Command").map(|s| xml_unescape(s.trim()))
}

/// 查询任务的完整 XML。
fn query_task_xml() -> Option<String> {
    let mut cmd = Command::new("schtasks.exe");
    cmd.args(["/Query", "/TN", TASK_NAME, "/XML"]);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(decode_console(&out.stdout))
}

/// 计划任务是否存在（不校验路径）。
pub fn task_exists() -> bool {
    run_schtasks(&["/Query", "/TN", TASK_NAME]).unwrap_or(false)
}

/// 任务存在但 XML 读不出来的兜底判断。
fn task_exists_unknown() -> bool {
    task_exists()
}

/// 安装（或更新）开机自启计划任务。需要管理员权限。
///
/// 创建后会回读任务 XML 校验路径；成功时删除 v1.2.0 遗留的 Run 键。
pub fn install(exe_path: &str) -> Result<(), String> {
    if exe_path.trim().is_empty() {
        return Err("程序路径为空".to_string());
    }
    if !crate::sys::is_elevated() {
        return Err("安装管理员自启需要管理员权限：请先以管理员身份运行本程序".to_string());
    }
    install_task(exe_path)?;
    if let Err(e) = delete_run_value() {
        // Run 键删不掉不影响任务模式，但记录到诊断日志。
        crate::app::diag_log(&format!("autostart: cleanup run value failed: {e}"));
    }
    let _ = uninstall_clean_task();
    Ok(())
}

/// 卸载开机自启：删除计划任务与 v1.2.0 遗留 Run 键。需要管理员权限（任务部分）。
///
/// 任务不存在时视为成功（幂等）。
pub fn uninstall() -> Result<(), String> {
    if crate::sys::is_elevated() {
        uninstall_task()?;
        delete_run_value()?;
        let _ = uninstall_clean_task();
        Ok(())
    } else {
        delete_run_value()?;
        if task_exists() {
            Err("移除管理员自启任务需要管理员权限：请先以管理员身份运行本程序".to_string())
        } else {
            Ok(())
        }
    }
}

/// 创建计划任务（要求调用方已提权）。
fn install_task(exe_path: &str) -> Result<(), String> {
    let user = current_user_id();
    let xml = task_xml(exe_path, &user);
    let path = write_task_xml(&xml)?;
    let path_str = path.to_string_lossy().to_string();
    let result = run_schtasks(&["/Create", "/TN", TASK_NAME, "/XML", &path_str, "/F"]);
    let _ = std::fs::remove_file(&path);
    match result {
        Ok(true) => {}
        Ok(false) => return Err("创建自启计划任务失败（schtasks 返回错误）".to_string()),
        Err(e) => return Err(e),
    }
    // 回读校验：任务必须存在且指向当前 exe。
    match task_command() {
        Some(cmd) if path_matches(&cmd, exe_path) => Ok(()),
        Some(cmd) => Err(format!("计划任务指向的路径不一致：{cmd}")),
        None => Err("计划任务创建后查询不到（可能被杀毒软件拦截）".to_string()),
    }
}

/// 把任务 XML 写到临时文件（UTF-16LE + BOM，`schtasks /XML` 要求）。
fn write_task_xml(xml: &str) -> Result<std::path::PathBuf, String> {
    let mut path = std::env::temp_dir();
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    path.push(format!("memcleaner-task-{unique}.xml"));
    let mut data: Vec<u8> = vec![0xFF, 0xFE];
    for unit in xml.encode_utf16() {
        data.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(&path, data).map_err(|e| format!("写入任务定义失败：{e}"))?;
    Ok(path)
}

/// 删除计划任务（幂等；要求调用方已提权）。
fn uninstall_task() -> Result<(), String> {
    if run_schtasks(&["/Delete", "/TN", TASK_NAME, "/F"]).unwrap_or(false) {
        return Ok(());
    }
    if task_exists() {
        Err("删除自启计划任务失败（权限不足）".to_string())
    } else {
        Ok(())
    }
}

/// 启动计划任务（用于验证）。需要管理员权限。
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

/// 从「"C:\\path\\app.exe" --args」里取出 exe 路径。
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
// v1.2.0 遗留的用户级自启（HKCU\Software\Microsoft\Windows\CurrentVersion\Run）
//
// v1.3.0 起只用于迁移清理与诊断，不再作为安装入口。
// ---------------------------------------------------------------------------

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "MemCleaner";

/// 读取用户级自启命令行（诊断/迁移用）。
pub fn user_autostart_command() -> Option<String> {
    read_run_value().ok().flatten()
}

/// 清理 v1.1.1 可能存在的清理任务（幂等）。
pub fn uninstall_clean_task() -> Result<(), String> {
    let _ = run_schtasks(&["/Delete", "/TN", "MemCleaner Clean", "/F"]);
    Ok(())
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
        if rc == 0 || rc == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!("删除自启项失败（{rc}）"))
        }
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
        // 计划任务 XML 里的 Command 不带引号、不含参数。
        assert!(path_matches(
            r"C:\Users\admin\AppData\Local\MemCleaner\MemCleaner.exe",
            r"c:\users\admin\appdata\local\memcleaner\memcleaner.exe"
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
        let xml =
            r#"<Task><Actions><Exec><Command>C:\a&amp;b\MemCleaner.exe</Command></Exec></Actions></Task>"#;
        let cmd = extract_xml_tag(xml, "Command").unwrap();
        assert_eq!(xml_unescape(cmd.trim()), r"C:\a&b\MemCleaner.exe");
        assert!(extract_xml_tag(xml, "Arguments").is_none());
    }

    #[test]
    fn decode_console_handles_utf8() {
        let s = decode_console("任务".as_bytes());
        assert!(s.contains("任务"));
    }

    #[test]
    fn status_helpers_distinguish_task_and_run_modes() {
        let task = AutostartStatus::InstalledTask;
        assert!(task.is_installed());
        assert!(task.is_elevated());
        let run = AutostartStatus::InstalledRun("x".to_string());
        assert!(run.is_installed());
        assert!(!run.is_elevated());
        assert!(!AutostartStatus::NotInstalled.is_installed());
        assert!(!AutostartStatus::NotInstalled.is_elevated());
    }
}
