//! 主题：跟随系统深色/浅色模式。

use crate::config::ThemeMode;
use super::wide;
use windows_sys::Win32::System::Registry::{
    RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, REG_DWORD,
};

/// 读取系统「应用主题」是否为深色（设置 → 个性化 → 颜色）。
pub fn system_is_dark() -> bool {
    let mut value: u32 = 1; // 默认浅色
    unsafe {
        let mut hkey: HKEY = std::ptr::null_mut();
        let subkey = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_QUERY_VALUE, &mut hkey) != 0 {
            return false;
        }
        let name = wide("AppsUseLightTheme");
        let mut data_type: u32 = 0;
        let mut size: u32 = std::mem::size_of::<u32>() as u32;
        let mut buffer: u32 = 1;
        let result = RegQueryValueExW(
            hkey,
            name.as_ptr(),
            std::ptr::null(),
            &mut data_type as *mut u32,
            &mut buffer as *mut u32 as *mut u8,
            &mut size,
        );
        let _ = windows_sys::Win32::System::Registry::RegCloseKey(hkey);
        if result == 0 && data_type == REG_DWORD {
            value = buffer;
        }
    }
    value == 0
}

/// 根据配置解析实际使用的主题（返回 true 表示深色）。
pub fn resolve_is_dark(mode: ThemeMode) -> bool {
    match mode {
        ThemeMode::System => system_is_dark(),
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
    }
}
