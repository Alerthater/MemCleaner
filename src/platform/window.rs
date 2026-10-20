//! 主窗口：无边框、DWM 圆角、拖拽、物理/逻辑像素换算。

use super::wide;
use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// 窗口类名（单实例查找、托盘消息都依赖它）。
pub const CLASS_NAME: &str = "MemCleanerMainWindow";
/// 窗口标题（任务栏/Alt-Tab 显示）。
pub const WINDOW_TITLE: &str = "内存清理";

/// 设计稿逻辑尺寸（100% DPI）。
pub const LOGICAL_WIDTH: i32 = 400;
pub const LOGICAL_HEIGHT: i32 = 560;

/// 进程级 DPI 感知：Per-Monitor V2，必须在创建窗口前调用。
pub fn init_dpi_awareness() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// 主显示器 DPI（未创建窗口时用）。
pub fn system_dpi() -> u32 {
    let dpi = unsafe { GetDpiForSystem() };
    if dpi == 0 {
        96
    } else {
        dpi
    }
}

/// 窗口所在显示器 DPI。
pub fn window_dpi(hwnd: HWND) -> u32 {
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        96
    } else {
        dpi
    }
}

/// 逻辑像素 -> 物理像素。
pub fn scale(logical: i32, dpi: u32) -> i32 {
    ((logical as f64) * (dpi as f64) / 96.0).round() as i32
}

/// 物理像素 -> 逻辑像素（f32）。
pub fn to_logical(physical: i32, dpi: u32) -> f32 {
    physical as f32 * 96.0 / dpi as f32
}

/// 注册窗口类。返回是否成功（已注册也视为成功）。
pub fn register_class(wndproc: WNDPROC) -> bool {
    unsafe {
        let hinstance = GetModuleHandleW(std::ptr::null());
        let class_name = wide(CLASS_NAME);
        let cursor = LoadCursorW(std::ptr::null_mut(), IDC_ARROW);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: wndproc,
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: std::ptr::null_mut(),
            hCursor: cursor,
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        RegisterClassExW(&wc) != 0
    }
}

/// 创建主窗口（隐藏状态创建，随后由 app 决定显示时机）。失败返回 None。
pub fn create_main_window(wndproc: WNDPROC, dpi: u32) -> Option<HWND> {
    unsafe {
        let hinstance = GetModuleHandleW(std::ptr::null());
        let class_name = wide(CLASS_NAME);
        let title = wide(WINDOW_TITLE);
        let w = scale(LOGICAL_WIDTH, dpi);
        let h = scale(LOGICAL_HEIGHT, dpi);
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            w,
            h,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return None;
        }
        apply_dwm_effects(hwnd);
        center_on_screen(hwnd);
        Some(hwnd)
    }
}

/// DWM 圆角 + 扩展边框（获得窗口阴影）。
pub fn apply_dwm_effects(hwnd: HWND) {
    unsafe {
        // Win11 圆角：失败时忽略（Win10 无此属性）。
        let preference: i32 = DWMWCP_ROUND as i32;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &preference as *const i32 as *const core::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
        // 说明：不要用 DwmExtendFrameIntoClientArea(-1) 把边框扩到整个客户区，
        // 那会让客户区鼠标消息变成非客户区消息（WM_NCHITTEST 之后拿不到 WM_LBUTTONDOWN）。
        // 这里用 1px 边框保留系统阴影，同时客户区仍然是正常客户区。
        let margins = MARGINS {
            cxLeftWidth: 1,
            cxRightWidth: 1,
            cyTopHeight: 1,
            cyBottomHeight: 1,
        };
        let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
    }
}

/// 窗口居中到当前显示器工作区。
pub fn center_on_screen(hwnd: HWND) {
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return;
        }
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        let sw = GetSystemMetrics(SM_CXSCREEN);
        let sh = GetSystemMetrics(SM_CYSCREEN);
        let x = (sw - w) / 2;
        let y = (sh - h) / 2;
        let _ = SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// 客户区尺寸（物理像素）。
pub fn client_size(hwnd: HWND) -> (i32, i32) {
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        if GetClientRect(hwnd, &mut rect) == 0 {
            return (0, 0);
        }
        (rect.right - rect.left, rect.bottom - rect.top)
    }
}

/// 开始拖动无标题栏窗口（在客户区按下左键时调用）。
pub fn begin_drag(hwnd: HWND) {
    unsafe {
        SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as WPARAM, 0 as LPARAM);
    }
}

/// 显示/隐藏窗口（隐藏时同时从任务栏移除）。
pub fn set_visible(hwnd: HWND, visible: bool) {
    unsafe {
        ShowWindow(hwnd, if visible { SW_SHOWNORMAL } else { SW_HIDE });
        if visible {
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

/// 调整窗口大小到新的 DPI（DPI 变化时调用）。
pub fn resize_for_dpi(hwnd: HWND, dpi: u32) {
    unsafe {
        let w = scale(LOGICAL_WIDTH, dpi);
        let h = scale(LOGICAL_HEIGHT, dpi);
        let _ = SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            w,
            h,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// 允许低权限进程向本窗口发送自定义消息（UIPI 过滤放行）。
pub fn allow_message_from_lower_integrity(hwnd: HWND, msg: u32) {
    unsafe {
        let _ = ChangeWindowMessageFilterEx(hwnd, msg, MSGFLT_ALLOW, std::ptr::null_mut());
    }
}

/// 强制重绘整个窗口。
pub fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(hwnd, std::ptr::null(), 0);
    }
}

/// 光标是否在窗口上。
pub fn is_cursor_on_window(hwnd: HWND) -> bool {
    unsafe {
        let mut pt: windows_sys::Win32::Foundation::POINT = std::mem::zeroed();
        if GetCursorPos(&mut pt) == 0 {
            return false;
        }
        let hit = WindowFromPoint(pt);
        hit == hwnd
    }
}
