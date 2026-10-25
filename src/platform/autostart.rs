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
pub fn status() -> AutostartStatus {
    if run_schtasks(&["/Query", "/TN", TASK_NAME]).unwrap_or(false) {
        AutostartStatus::Installed
    } else {
        AutostartStatus::NotInstalled
    }
}

/// 查询任务并校验它指向的 exe 是否就是 `exe_path`。
pub fn status_for(exe_path: &str) -> AutostartStatus {
    match task_command() {
        Some(command) => {
            if path_matches(&command, exe_path) {
                AutostartStatus::Installed
            } else {
                AutostartStatus::Mismatched(command)
            }
        }
        None => AutostartStatus::NotInstalled,
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
pub fn install(exe_path: &str) -> Result<(), String> {
    if exe_path.trim().is_empty() {
        return Err("程序路径为空".to_string());
    }
    let user = current_user_id();
    let xml = task_xml(exe_path, &user);
    let dir = std::env::temp_dir().join("MemCleaner");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建临时目录失败：{e}"))?;
    let xml_path = dir.join("autostart-task.xml");

    // Task Scheduler 要求 XML 为 UTF-16（带 BOM）。
    let mut utf16: Vec<u8> = vec![0xFF, 0xFE];
    for unit in xml.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(&xml_path, utf16).map_err(|e| format!("写入任务 XML 失败：{e}"))?;

    let xml_str = xml_path.to_string_lossy().to_string();
    if !run_schtasks(&["/Create", "/TN", TASK_NAME, "/XML", &xml_str, "/F"])? {
        return Err("创建计划任务失败（请确认以管理员身份运行）".to_string());
    }
    // 回读校验：确保任务确实存在且指向当前 exe。
    match status_for(exe_path) {
        AutostartStatus::Installed => Ok(()),
        AutostartStatus::Mismatched(cmd) => Err(format!(
            "计划任务已创建但指向的路径不一致（任务：{cmd}，当前：{exe_path}）"
        )),
        AutostartStatus::NotInstalled => Err("计划任务创建后查询不到，可能被系统策略阻止".to_string()),
        AutostartStatus::Error(e) => Err(format!("计划任务创建后校验失败：{e}")),
    }
}

/// 卸载自启计划任务。需要管理员权限；任务不存在时视为成功（幂等）。
pub fn uninstall() -> Result<(), String> {
    if matches!(status(), AutostartStatus::NotInstalled) {
        return Ok(());
    }
    let ok = run_schtasks(&["/Delete", "/TN", TASK_NAME, "/F"])?;
    if ok || matches!(status(), AutostartStatus::NotInstalled) {
        Ok(())
    } else {
        Err("删除计划任务失败（任务可能不存在或权限不足）".to_string())
    }
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
pub fn path_matches(a: &str, b: &str) -> bool {
    normalize_path(a) == normalize_path(b)
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
