//! GDI 文字渲染：HFONT 缓存 + ClearType（画在 DIB 内存 DC 上）。
//!
//! 文字走 GDI 的原因：ClearType 质量好、零额外依赖；字体对象按 (字号,字重,字体) 缓存，
//! 渲染热路径零堆分配（UTF-16 转换用栈缓冲）。
#![allow(dead_code)]

use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::{
    CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH,
    DT_CENTER, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW,
    FF_DONTCARE, GetTextExtentPoint32W, HDC, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS, SelectObject,
    SetBkMode, SetTextColor,
};

/// 数值/拉丁展示字体（大号数字用 Variable Display）。
pub const FACE_DISPLAY: &str = "Segoe UI Variable Display";
/// 中文/正文（YaHei UI，与系统 UI 一致）。
pub const FACE_TEXT: &str = "Microsoft YaHei UI";
/// 图标字体（Segoe MDL2 Assets）。
pub const FACE_ICON: &str = "Segoe MDL2 Assets";

pub const FACE_DISPLAY_IDX: u8 = 0;
pub const FACE_TEXT_IDX: u8 = 1;
pub const FACE_ICON_IDX: u8 = 2;

/// 文字水平对齐。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

impl Align {
    fn dt(self) -> u32 {
        match self {
            Align::Left => DT_LEFT,
            Align::Center => DT_CENTER,
            Align::Right => DT_RIGHT,
        }
    }
}

/// 把 &str 拷进栈上 UTF-16 缓冲（≤127 字符），避免每帧堆分配。
#[inline]
fn utf16_stack(text: &str, buf: &mut [u16; 128]) -> usize {
    let mut n = 0usize;
    for u in text.encode_utf16() {
        if n + 1 >= buf.len() {
            break;
        }
        buf[n] = u;
        n += 1;
    }
    n
}

/// 字体缓存（(设备像素字号, 字重, 字体索引) -> HFONT）。
pub struct FontCache {
    items: Vec<(i32, i32, u8, HFONT)>,
}

impl FontCache {
    pub fn new() -> Self {
        FontCache { items: Vec::with_capacity(16) }
    }

    pub fn get(&mut self, size_px: i32, weight: i32, face: u8) -> HFONT {
        let size_px = size_px.max(1);
        if let Some(&(_, _, _, h)) = self
            .items
            .iter()
            .find(|(p, w, f, _)| *p == size_px && *w == weight && *f == face)
        {
            return h;
        }
        let face_name = match face {
            FACE_DISPLAY_IDX => FACE_DISPLAY,
            FACE_ICON_IDX => FACE_ICON,
            _ => FACE_TEXT,
        };
        let mut name = crate::platform::wide(face_name);
        let font = unsafe {
            CreateFontW(
                -size_px, // 负值 = 字符高度（em size）
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                CLEARTYPE_QUALITY as u32,
                (DEFAULT_PITCH | FF_DONTCARE) as u32,
                name.as_mut_ptr(),
            )
        };
        self.items.push((size_px, weight, face, font));
        font
    }

    pub fn clear(&mut self) {
        for (_, _, _, h) in self.items.drain(..) {
            if !h.is_null() {
                unsafe {
                    DeleteObject(h as HGDIOBJ);
                }
            }
        }
    }
}

impl Default for FontCache {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for FontCache {
    fn drop(&mut self) {
        self.clear();
    }
}

/// COLORREF 转换：0xRRGGBB -> 0x00BBGGRR。
#[inline]
pub fn colorref(rgb: u32) -> u32 {
    ((rgb & 0xFF) << 16) | (((rgb >> 8) & 0xFF) << 8) | ((rgb >> 16) & 0xFF)
}

/// 画一行文字（DT_SINGLELINE + 垂直居中）。rect 为设备像素。
#[allow(clippy::too_many_arguments)]
pub fn draw_text(
    hdc: HDC,
    fonts: &mut FontCache,
    text: &str,
    rect: RECT,
    size_px: i32,
    weight: i32,
    face: u8,
    rgb: u32,
    align: Align,
) {
    let mut buf = [0u16; 128];
    let n = utf16_stack(text, &mut buf);
    if n == 0 {
        return;
    }
    let font = fonts.get(size_px, weight, face);
    unsafe {
        let old = SelectObject(hdc, font as HGDIOBJ);
        SetTextColor(hdc, colorref(rgb));
        SetBkMode(hdc, 1); // TRANSPARENT
        let mut rc = rect;
        DrawTextW(
            hdc,
            buf.as_ptr(),
            n as i32,
            &mut rc,
            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | align.dt(),
        );
        SelectObject(hdc, old);
    }
}

/// 测量文字宽度（设备像素）。
pub fn measure_text(
    hdc: HDC,
    fonts: &mut FontCache,
    text: &str,
    size_px: i32,
    weight: i32,
    face: u8,
) -> i32 {
    let mut buf = [0u16; 128];
    let n = utf16_stack(text, &mut buf);
    if n == 0 {
        return 0;
    }
    let font = fonts.get(size_px, weight, face);
    unsafe {
        let old = SelectObject(hdc, font as HGDIOBJ);
        let mut size: windows_sys::Win32::Foundation::SIZE = std::mem::zeroed();
        GetTextExtentPoint32W(hdc, buf.as_ptr(), n as i32, &mut size);
        SelectObject(hdc, old);
        size.cx
    }
}
