//! Windows 平台层：窗口、托盘、自启、单实例、提权、主题。
//!
//! 所有 unsafe FFI 都封装在这一层，业务层（app/ui/engine）只使用安全接口。

pub mod autostart;
pub mod elevate;
pub mod icon;
pub mod single_instance;
pub mod theme;
pub mod tray;
pub mod window;

use windows_sys::Win32::UI::WindowsAndMessaging::{WM_APP, WM_SETTINGCHANGE};

/// 让已有实例显示主窗口的自定义消息（低权限实例 -> 高权限实例发送需要
/// 高权限窗口对该消息调用 ChangeWindowMessageFilterEx(MSGFLT_ALLOW)）。
pub const WM_APP_SHOW: u32 = WM_APP + 1;

/// 内存状态刷新定时器 ID。
pub const TIMER_TICK: usize = 1;
/// 动画定时器 ID。
pub const TIMER_ANIM: usize = 2;

/// Rust 字符串转以 NUL 结尾的 UTF-16（Windows API 入参）。
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 当前设置变更消息是否与主题/系统颜色有关。
pub fn is_theme_changed_message(msg: u32) -> bool {
    msg == WM_SETTINGCHANGE
}
