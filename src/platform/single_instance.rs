//! 单实例：命名互斥体 + 唤醒已有窗口。

use super::window::CLASS_NAME;
use super::wide;
use super::WM_APP_SHOW;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, HANDLE,
};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW};

/// 单实例守卫，进程存活期间持有互斥体。
pub struct InstanceGuard {
    handle: HANDLE,
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}

/// 尝试获取单实例所有权。返回 None 表示已有实例在运行（含更高完整性的实例）。
///
/// 关键场景：计划任务以管理员权限常驻托盘后，用户又双击了普通权限的 exe。
/// 此时同名互斥体被高完整性实例持有，`CreateMutexW` 会返回 `ERROR_ACCESS_DENIED(5)`
/// 而不是 `ERROR_ALREADY_EXISTS(183)`。这种情况必须视为「已有实例」并唤醒它，
/// 否则会出现双实例、配置互相覆盖（QA Bug-01）。
pub fn acquire() -> Option<InstanceGuard> {
    // 开发/自动化测试可用 MEMCLEANER_MUTEX_SUFFIX 隔离实例（正常用户不会设置）。
    let suffix = std::env::var_os("MEMCLEANER_MUTEX_SUFFIX")
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_default();
    let name = wide(&format!("Local\\MemCleaner.SingleInstance.v1{suffix}"));
    unsafe {
        let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        let err = GetLastError();
        if handle.is_null() {
            if err == ERROR_ACCESS_DENIED {
                // 已有高完整性实例：交给调用方唤醒并退出。
                return None;
            }
            // 其它未知错误才放行，避免完全无法启动。
            return Some(InstanceGuard {
                handle: std::ptr::null_mut(),
            });
        }
        if err == ERROR_ALREADY_EXISTS {
            CloseHandle(handle);
            None
        } else {
            Some(InstanceGuard { handle })
        }
    }
}

/// 通知已有实例显示主窗口。
pub fn wake_existing_instance() -> bool {
    unsafe {
        let class_name = wide(CLASS_NAME);
        let hwnd = FindWindowW(class_name.as_ptr(), std::ptr::null());
        if hwnd.is_null() {
            return false;
        }
        PostMessageW(hwnd, WM_APP_SHOW, 0, 0) != 0
    }
}

/// 找到已有主窗口句柄。
pub fn find_main_window() -> windows_sys::Win32::Foundation::HWND {
    unsafe {
        let class_name = wide(CLASS_NAME);
        FindWindowW(class_name.as_ptr(), std::ptr::null())
    }
}
