//! 开机自启：通过「计划任务（最高权限）」实现静默自启，避免每次开机 UAC。
//!
//! 为什么不用注册表 Run：注册表自启只能以普通权限运行，而内存清理必须管理员权限。
//! 计划任务以 HighestAvailable 运行，登录时静默启动到托盘。

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
    NotInstalled,
    Installed,
    /// 查询失败（计划任务服务异常等）。
    Error(String),
}

/// 查询计划任务是否已安装。
pub fn status() -> AutostartStatus {
    let result = run_schtasks(&["/Query", "/TN", TASK_NAME]);
    match result {
        Ok(ok) => {
            if ok {
                AutostartStatus::Installed
            } else {
                AutostartStatus::NotInstalled
            }
        }
        Err(e) => AutostartStatus::Error(e),
    }
}

/// 安装（或更新）自启计划任务。需要管理员权限。
pub fn install(exe_path: &str) -> Result<(), String> {
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
    let ok = run_schtasks(&["/Create", "/TN", TASK_NAME, "/XML", &xml_str, "/F"])?;
    if ok {
        Ok(())
    } else {
        Err("创建计划任务失败（请确认以管理员身份运行）".to_string())
    }
}

/// 卸载自启计划任务。需要管理员权限。
pub fn uninstall() -> Result<(), String> {
    let ok = run_schtasks(&["/Delete", "/TN", TASK_NAME, "/F"])?;
    if ok {
        Ok(())
    } else {
        Err("删除计划任务失败（任务可能不存在或权限不足）".to_string())
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

/// 当前用户标识（DOMAIN\user）。
fn current_user_id() -> String {
    let domain = std::env::var("USERDOMAIN").unwrap_or_default();
    let user = std::env::var("USERNAME").unwrap_or_default();
    if domain.is_empty() {
        user
    } else {
        format!("{domain}\\{user}")
    }
}

/// 生成计划任务 XML。
fn task_xml(exe_path: &str, user: &str) -> String {
    let exe = xml_escape(exe_path);
    let user = xml_escape(user);
    let working_dir = std::path::Path::new(exe_path)
        .parent()
        .map(|p| xml_escape(&p.to_string_lossy()))
        .unwrap_or_default();
    // 含空格的路径按 Windows 惯例加引号。
    let command = if exe_path.contains(' ') {
        format!("\"{exe}\"")
    } else {
        exe
    };
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
    <StartWhenAvailable>false</StartWhenAvailable>
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
      <Command>{command}</Command>
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
        assert!(xml.contains("\"C:\\Program Files\\MemCleaner\\MemCleaner.exe\""));
        assert!(xml.contains("<LogonTrigger>"));
    }

    #[test]
    fn xml_escapes_special_chars() {
        let xml = task_xml(r"C:\a&b\c.exe", "u<ser");
        assert!(xml.contains("a&amp;b"));
        assert!(xml.contains("u&lt;ser"));
    }
}
