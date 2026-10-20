//! UAC 提权：以管理员身份重启自身。

use super::wide;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// 当前 exe 的完整路径。
pub fn current_exe() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// 以管理员身份重启自身（触发 UAC），附加参数原样传入。
/// 返回 true 表示 ShellExecuteW 调用成功（用户仍可能取消 UAC）。
pub fn relaunch_elevated(extra_args: &str) -> bool {
    let exe = current_exe();
    if exe.is_empty() {
        return false;
    }
    let op = wide("runas");
    let file = wide(&exe);
    let params = wide(extra_args);
    unsafe {
        let result = ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
        (result as isize) > 32
    }
}

/// 以普通权限启动自身（用于放弃管理员模式后重启）。
pub fn relaunch_normal(extra_args: &str) -> bool {
    let exe = current_exe();
    if exe.is_empty() {
        return false;
    }
    let op = wide("open");
    let file = wide(&exe);
    let params = wide(extra_args);
    unsafe {
        let result = ShellExecuteW(
            std::ptr::null_mut(),
            op.as_ptr(),
            file.as_ptr(),
            params.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        );
        (result as isize) > 32
    }
}
