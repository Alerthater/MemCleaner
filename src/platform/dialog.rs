//! 系统文件选择对话框（选择自定义皮肤图片）。

use super::wide;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_NOCHANGEDIR,
    OFN_PATHMUSTEXIST, OPENFILENAMEW,
};

/// 弹出「打开文件」对话框让用户选择一张图片。返回选中的完整路径；取消返回 None。
pub fn pick_image_file(owner: HWND) -> Option<String> {
    // 过滤器格式：描述\0模式\0描述\0模式\0\0
    let filter: Vec<u16> = "图片文件 (*.png;*.jpg;*.jpeg;*.bmp;*.gif;*.webp)\0*.png;*.jpg;*.jpeg;*.bmp;*.gif;*.webp\0所有文件 (*.*)\0*.*\0\0"
        .encode_utf16()
        .collect();
    let title = wide("选择皮肤图片（支持 PNG/JPG/BMP/GIF/WebP）");
    let mut buffer = vec![0u16; 1024];

    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = owner;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = buffer.as_mut_ptr();
    ofn.nMaxFile = buffer.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    ofn.Flags = OFN_FILEMUSTEXIST
        | OFN_PATHMUSTEXIST
        | OFN_NOCHANGEDIR
        | OFN_EXPLORER
        | OFN_HIDEREADONLY;

    let ok = unsafe { GetOpenFileNameW(&mut ofn) };
    if ok == 0 {
        return None;
    }
    let len = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    if len == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buffer[..len]))
}
