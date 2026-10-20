//! 系统托盘图标与右键菜单。
//!
//! 支持可选的「图标直接显示内存占用率」：由 [`Tray::set_percent`] 动态生成带百分比数字的
//! HICON 并通过 NIM_MODIFY 更新；关闭该选项时恢复普通应用图标。

use super::icon;
use super::wide;
use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// 托盘图标的回调消息（发到主窗口）。
pub const TRAY_CALLBACK_MSG: u32 = WM_APP + 2;
/// 托盘图标 ID。
const TRAY_ID: u32 = 1;

/// 托盘菜单命令。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    /// 显示主窗口。
    Show,
    /// 立即清理。
    Clean,
    /// 显示/隐藏主窗口。
    ToggleWindow,
    /// 退出应用。
    Exit,
}

const MENU_SHOW: usize = 1001;
const MENU_CLEAN: usize = 1002;
const MENU_EXIT: usize = 1003;

/// 托盘对象。持有图标句柄与菜单状态。
pub struct Tray {
    hwnd: HWND,
    /// 当前显示的图标。
    icon: HICON,
    /// 普通应用图标（关闭占用率显示时使用）。
    base_icon: HICON,
    /// 系统小图标尺寸（随 DPI 变化）。
    icon_size: i32,
    added: bool,
    /// 当前图标对应的占用率状态：None = 从未设置；Some(None) = 普通图标。
    shown_percent: Option<Option<u8>>,
}

impl Tray {
    /// 创建托盘（不自动添加图标，需调用 [`Tray::add`]）。
    pub fn new(hwnd: HWND) -> Self {
        let icon_size = unsafe {
            let s = GetSystemMetrics(SM_CXSMICON);
            if s > 0 {
                s
            } else {
                16
            }
        };
        let base_icon = icon::app_icon(icon_size);
        Tray {
            hwnd,
            icon: base_icon,
            base_icon,
            icon_size,
            added: false,
            shown_percent: Some(None),
        }
    }

    /// 添加图标到通知区。
    pub fn add(&mut self, tip: &str) -> bool {
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.hwnd;
        nid.uID = TRAY_ID;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = TRAY_CALLBACK_MSG;
        nid.hIcon = self.icon;
        copy_tip(&mut nid.szTip, tip);
        let ok = unsafe { Shell_NotifyIconW(NIM_ADD, &nid) } != 0;
        self.added = ok;
        ok
    }

    /// 修改提示文字（如「内存 62%」）。
    pub fn set_tip(&mut self, tip: &str) {
        if !self.added {
            return;
        }
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.hwnd;
        nid.uID = TRAY_ID;
        nid.uFlags = NIF_TIP;
        copy_tip(&mut nid.szTip, tip);
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    /// 设置托盘图标是否显示内存占用率。
    ///
    /// `percent = Some(p)` 时生成带百分比数字的图标；`None` 时恢复普通图标。
    /// 只有状态变化时才重建图标（避免每秒创建 GDI 对象）。
    pub fn set_percent(&mut self, percent: Option<u8>) {
        if self.shown_percent == Some(percent) {
            return;
        }
        self.shown_percent = Some(percent);
        let new_icon = match percent {
            Some(p) => icon::tray_percent_icon(p, self.icon_size),
            None => self.base_icon,
        };
        if new_icon.is_null() {
            return;
        }
        if !self.icon.is_null() && self.icon != self.base_icon {
            unsafe {
                DestroyIcon(self.icon);
            }
        }
        self.icon = new_icon;
        self.refresh_icon();
    }

    /// 通过 NIM_MODIFY 刷新通知区图标。
    fn refresh_icon(&mut self) {
        if !self.added {
            return;
        }
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.hwnd;
        nid.uID = TRAY_ID;
        nid.uFlags = NIF_ICON;
        nid.hIcon = self.icon;
        unsafe {
            Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    /// 从通知区移除图标。
    pub fn remove(&mut self) {
        if !self.added {
            return;
        }
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.hwnd;
        nid.uID = TRAY_ID;
        unsafe {
            Shell_NotifyIconW(NIM_DELETE, &nid);
        }
        self.added = false;
    }

    /// 处理托盘回调消息（wparam=图标 ID，lparam=鼠标消息）。
    /// 右键弹出菜单并返回选择的命令。
    pub fn on_callback(&mut self, lparam: LPARAM) -> Option<TrayCommand> {
        let event = (lparam as u32) & 0xFFFF;
        match event {
            WM_LBUTTONUP | WM_LBUTTONDBLCLK => Some(TrayCommand::Show),
            WM_RBUTTONUP | WM_CONTEXTMENU => self.show_menu(),
            _ => None,
        }
    }

    /// 弹出右键菜单。
    fn show_menu(&mut self) -> Option<TrayCommand> {
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return None;
            }
            let show_label = wide("打开主界面");
            let clean_label = wide("立即清理");
            let exit_label = wide("退出");
            AppendMenuW(menu, MF_STRING, MENU_SHOW, show_label.as_ptr());
            AppendMenuW(menu, MF_STRING, MENU_CLEAN, clean_label.as_ptr());
            AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
            AppendMenuW(menu, MF_STRING, MENU_EXIT, exit_label.as_ptr());

            let mut pt: POINT = std::mem::zeroed();
            let _ = GetCursorPos(&mut pt);
            let _ = SetForegroundWindow(self.hwnd);
            let cmd = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_LEFTALIGN,
                pt.x,
                pt.y,
                0,
                self.hwnd,
                std::ptr::null(),
            ) as usize;
            let _ = DestroyMenu(menu);
            let _ = PostMessageW(self.hwnd, WM_NULL, 0, 0);
            match cmd {
                MENU_SHOW => Some(TrayCommand::Show),
                MENU_CLEAN => Some(TrayCommand::Clean),
                MENU_EXIT => Some(TrayCommand::Exit),
                _ => None,
            }
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
        unsafe {
            if !self.icon.is_null() && self.icon != self.base_icon {
                let _ = DestroyIcon(self.icon);
            }
            if !self.base_icon.is_null() {
                let _ = DestroyIcon(self.base_icon);
            }
            self.icon = std::ptr::null_mut();
            self.base_icon = std::ptr::null_mut();
        }
    }
}

/// 把字符串写入 NOTIFYICONDATAW.szTip（最长 127 个 UTF-16 单元）。
fn copy_tip(dst: &mut [u16; 128], text: &str) {
    let encoded: Vec<u16> = text.encode_utf16().take(126).collect();
    for (i, ch) in encoded.iter().enumerate() {
        dst[i] = *ch;
    }
    dst[encoded.len()] = 0;
}

/// 托盘图标句柄（供外部设置窗口图标）。
pub fn tray_icon_handle() -> HICON {
    icon::app_icon(32)
}
