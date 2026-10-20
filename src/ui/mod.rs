//! UI 层：界面状态、交互命中测试、软件光栅化渲染。
//!
//! 渲染路径：单张 32bpp 自上而下 DIB（内存 DC）上用 SDF 逐像素画背景/卡片/圆环/按钮，
//! 文字交给 GDI `DrawTextW`（ClearType，同一块 DIB），最后一次 `BitBlt` 到窗口 DC。
//! 无 Direct2D/Write/GDI+ 依赖（自定义皮肤图片解码在 platform/image.rs，用系统 GDI+ 按需加载）。
//!
//! 设计稿逻辑尺寸 400x560（100% DPI），所有绘制与命中测试都用逻辑坐标，
//! 由 [`Renderer`] 统一乘 dpi/96 换算成物理像素。

// 公开 API 中有一部分是 app.rs 依赖的稳定接口，保留以避免破坏调用方。
#![allow(dead_code)]

pub mod design;
pub mod raster;
pub mod text;
pub mod theme;

use crate::config::{AccentScheme, SkinPreset, ThemeMode};
use crate::sys::MemStatus;
use raster::Canvas;
pub use raster::Rect;
use raster::Color;
use std::sync::Arc;
use text::{Align, FontCache};
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::*;

/// 设计稿逻辑宽高（100% DPI）。
pub const LOGICAL_W: i32 = 400;
pub const LOGICAL_H: i32 = 560;

/// 界面页。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Main,
    Settings,
}

/// 设置页的两个分页。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Appearance,
}

/// 可调透明度的对象。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpacityKind {
    /// 自定义皮肤图片的叠加不透明度。
    Skin,
    /// 卡片/面板的不透明度。
    Panel,
    /// 整个窗口的不透明度（分层窗口 LWA_ALPHA）。
    Window,
}

/// 可交互元素 / 用户动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Clean,
    ToggleAutoClean,
    OpenSettings,
    CloseSettings,
    Minimize,
    Close,
    CycleTheme,
    CycleAccent,
    CycleSkinPreset,
    PickCustomSkin,
    ClearCustomSkin,
    ToggleTrayPercent,
    ToggleAutostart,
    ToggleMinimizeToTray,
    ToggleAnimations,
    ToggleCleanItem(CleanItem),
    /// 点击阈值行：进入数值输入模式。
    EditThreshold,
    /// 数值输入模式下点击其它区域：提交。
    CommitThreshold,
    /// 切换设置分页。
    SwitchTab(SettingsTab),
    /// 设置某个不透明度（值为 0..100，具体含义由 kind 决定）。
    SetOpacity(OpacityKind, u8),
    ConfirmAccept,
    ConfirmCancel,
}

/// 清理项标识。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanItem {
    WorkingSets,
    Modified,
    Standby,
    LowPriority,
    FileCache,
}

/// 设置页需要的配置镜像（UI 只读）。
#[derive(Clone, Debug)]
pub struct SettingsSnapshot {
    pub auto_enabled: bool,
    pub threshold: u8,
    pub clean_working_sets: bool,
    pub clean_modified: bool,
    pub clean_standby: bool,
    pub clean_low_priority: bool,
    pub trim_file_cache: bool,
    pub minimize_to_tray: bool,
    pub animations: bool,
    pub start_with_windows: bool,
    pub theme: ThemeMode,
    pub autostart_installed: bool,
    /// 托盘图标是否直接显示内存占用率。
    pub tray_show_percent: bool,
    /// 配色方案。
    pub accent: AccentScheme,
    /// 背景皮肤（预设/Custom）。
    pub skin_preset: SkinPreset,
    /// 自定义皮肤文件名（空 = 未选择，仅用于展示）。
    pub skin_image_name: String,
    pub skin_opacity: u8,
    pub panel_opacity: u8,
    pub window_opacity: u8,
}

/// 应用内确认框的用途（决定点「确定」后做什么）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmKind {
    ElevateClean,
    ElevateAutoClean,
    ElevateAutostart,
}

/// 应用内确认框（替代系统 MessageBox，风格统一且可自动化测试）。
#[derive(Clone, Debug)]
pub struct ConfirmDialog {
    pub title: String,
    pub message: String,
    pub confirm_label: String,
    pub cancel_label: String,
    pub kind: ConfirmKind,
}

/// 阈值输入框的编辑状态（由 app 维护，UI 只负责绘制）。
#[derive(Clone, Debug)]
pub struct ThresholdEdit {
    /// 当前输入的数字文本（可能为空）。
    pub text: String,
}

/// 轻提示类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

/// 轻提示。
#[derive(Clone, Debug)]
pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    /// 已存在秒数（由 app 累加）。
    pub age: f32,
}

/// 渲染一帧所需的全部状态快照。
pub struct UiState {
    pub dpi: u32,
    pub status: MemStatus,
    pub display_percent: f32,
    pub display_used: f64,
    pub display_avail: f64,
    pub elevated: bool,
    pub cleaning: bool,
    pub clean_progress: f32,
    pub clean_just_done: bool,
    pub is_dark: bool,
    pub page: Page,
    /// 设置页当前分页。
    pub settings_tab: SettingsTab,
    pub settings: SettingsSnapshot,
    pub last_freed_bytes: u64,
    pub last_clean_unix: u64,
    pub hover: Option<Action>,
    pub pressed: Option<Action>,
    pub anim_time: f32,
    pub toast: Option<Toast>,
    pub version: &'static str,
    pub confirm: Option<ConfirmDialog>,
    /// 阈值数值输入框编辑状态（Some 表示正在编辑）。
    pub threshold_editing: Option<ThresholdEdit>,
}

impl UiState {
    pub fn empty() -> Self {
        UiState {
            dpi: 96,
            status: MemStatus::default(),
            display_percent: 0.0,
            display_used: 0.0,
            display_avail: 0.0,
            elevated: false,
            cleaning: false,
            clean_progress: 0.0,
            clean_just_done: false,
            is_dark: true,
            page: Page::Main,
            settings_tab: SettingsTab::General,
            settings: SettingsSnapshot {
                auto_enabled: false,
                threshold: 85,
                clean_working_sets: true,
                clean_modified: true,
                clean_standby: true,
                clean_low_priority: false,
                trim_file_cache: false,
                minimize_to_tray: true,
                animations: true,
                start_with_windows: false,
                theme: ThemeMode::System,
                autostart_installed: false,
                tray_show_percent: true,
                accent: AccentScheme::Blue,
                skin_preset: SkinPreset::Default,
                skin_image_name: String::new(),
                skin_opacity: 55,
                panel_opacity: 100,
                window_opacity: 100,
            },
            last_freed_bytes: 0,
            last_clean_unix: 0,
            hover: None,
            pressed: None,
            anim_time: 0.0,
            toast: None,
            version: env!("CARGO_PKG_VERSION"),
            confirm: None,
            threshold_editing: None,
        }
    }
}

// ---------------------------------------------------------------------------
// 布局（全部逻辑像素：绘制与命中测试共用同一份常量）
// ---------------------------------------------------------------------------

pub mod layout {
    use super::raster::Rect;

    // ---- 顶部标题栏 ----
    pub const TITLE_Y: i32 = 20;
    pub const BTN_CLOSE: Rect = Rect::new(348, 14, 30, 30);
    pub const BTN_SETTINGS: Rect = Rect::new(312, 14, 30, 30);
    pub const BTN_BACK: Rect = Rect::new(14, 14, 30, 30);

    // ---- 主页面 ----
    pub const CARD: Rect = Rect::new(24, 52, 352, 302);
    pub const CARD_RADIUS: f32 = 20.0;
    pub const RING_CENTER: (f32, f32) = (200.0, 184.0);
    pub const RING_RADIUS: f32 = 92.0;
    pub const RING_THICKNESS: f32 = 10.0;
    pub const STATS_LABEL_Y: i32 = 294;
    pub const STATS_VALUE_Y: i32 = 312;
    pub const STATS_Y: i32 = STATS_LABEL_Y;
    pub const BTN_CLEAN: Rect = Rect::new(112, 372, 176, 46);
    pub const BTN_CLEAN_RADIUS: f32 = 13.0;
    pub const LAST_TEXT_Y: i32 = 427;
    pub const DIVIDER_Y: f32 = 470.0;
    pub const BOTTOM_Y: i32 = 504;
    pub const SWITCH: Rect = Rect::new(24, 504, 34, 20);

    // ---- 设置页：分页标签 + 卡片 + 行 ----
    pub const TAB_GENERAL: Rect = Rect::new(24, 52, 172, 34);
    pub const TAB_APPEARANCE: Rect = Rect::new(204, 52, 172, 34);
    pub const SETTINGS_CARD: Rect = Rect::new(24, 92, 352, 428);
    pub const SETTINGS_ROW_START: i32 = 98;
    pub const SETTINGS_ROW_H: i32 = 41;
    /// 一行内的滑块轨道（命中测试 & 绘制共用）。
    pub const SLIDER_X: i32 = 220;
    pub const SLIDER_W: i32 = 108;
    pub const SLIDER_Y_OFFSET: i32 = 24;
    pub const SLIDER_H: i32 = 8;

    /// 设置页第 n 行（0 起）的矩形。
    pub fn settings_row(n: i32) -> Rect {
        Rect::new(
            SETTINGS_CARD.x,
            SETTINGS_ROW_START + n * SETTINGS_ROW_H,
            SETTINGS_CARD.w,
            SETTINGS_ROW_H - 2,
        )
    }

    /// 滑块行对应的轨道矩形（逻辑坐标）。
    pub fn slider_track(row: Rect) -> Rect {
        Rect::new(row.x + SLIDER_X, row.y + SLIDER_Y_OFFSET, SLIDER_W, SLIDER_H)
    }

    /// 确认框卡片与两个按钮。
    pub const CONFIRM_CARD: Rect = Rect::new(40, 186, 320, 196);
    pub const CONFIRM_CANCEL: Rect = Rect::new(64, 318, 108, 38);
    pub const CONFIRM_ACCEPT: Rect = Rect::new(228, 318, 108, 38);
}

// ---------------------------------------------------------------------------
// 命中测试（逻辑坐标 -> 动作）
// ---------------------------------------------------------------------------

/// 命中测试：逻辑坐标 -> 动作。
pub fn hit_test(state: &UiState, lx: f32, ly: f32) -> Option<Action> {
    let (x, y) = (lx.round() as i32, ly.round() as i32);
    // 确认框是模态的：只响应确认框按钮；点卡片外=取消
    if state.confirm.is_some() {
        if layout::CONFIRM_ACCEPT.contains(x, y) {
            return Some(Action::ConfirmAccept);
        }
        if layout::CONFIRM_CANCEL.contains(x, y) || !layout::CONFIRM_CARD.contains(x, y) {
            return Some(Action::ConfirmCancel);
        }
        return None;
    }
    match state.page {
        Page::Main => {
            if layout::BTN_CLOSE.contains(x, y) {
                return Some(Action::Close);
            }
            if layout::BTN_SETTINGS.contains(x, y) {
                return Some(Action::OpenSettings);
            }
            if layout::BTN_CLEAN.contains(x, y) {
                return Some(Action::Clean);
            }
            if Rect::new(16, layout::BOTTOM_Y - 8, 170, 36).contains(x, y) {
                return Some(Action::ToggleAutoClean);
            }
            None
        }
        Page::Settings => {
            if layout::BTN_BACK.contains(x, y) {
                return Some(Action::CloseSettings);
            }
            if layout::BTN_CLOSE.contains(x, y) {
                return Some(Action::Close);
            }
            if layout::TAB_GENERAL.contains(x, y) {
                return Some(Action::SwitchTab(SettingsTab::General));
            }
            if layout::TAB_APPEARANCE.contains(x, y) {
                return Some(Action::SwitchTab(SettingsTab::Appearance));
            }
            // 阈值输入中：点其它位置 = 提交
            if state.threshold_editing.is_some() {
                let th_row = layout::settings_row(1);
                if th_row.contains(x, y) {
                    return None;
                }
                return Some(Action::CommitThreshold);
            }
            let tab = state.settings_tab;
            for n in 0..design::settings_row_count(tab) {
                let row = layout::settings_row(n);
                if !row.contains(x, y) {
                    continue;
                }
                return match design::settings_row_kind(tab, n) {
                    design::RowKind::Action(action) => Some(action),
                    design::RowKind::ThresholdInput => Some(Action::EditThreshold),
                    design::RowKind::Slider(kind) => {
                        // 滑块：按 x 位置换算 0..100
                        let track = layout::slider_track(row);
                        let rel = (x - track.x) as f32 / track.w.max(1) as f32;
                        let value = (rel * 100.0).round().clamp(0.0, 100.0) as u8;
                        Some(Action::SetOpacity(kind, value))
                    }
                };
            }
            None
        }
    }
}

// ---------------------------------------------------------------------------
// 格式化
// ---------------------------------------------------------------------------

/// 把字节格式化为 "8.2 GB" / "512 MB"。
pub fn format_bytes(bytes: f64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    if bytes >= GB {
        format!("{:.1} GB", bytes / GB)
    } else if bytes >= MB {
        format!("{:.0} MB", bytes / MB)
    } else {
        format!("{:.0} KB", bytes / 1024.0)
    }
}

/// 把 Unix 时间戳格式化为本地时区 "HH:MM"。
pub fn format_time(unix: u64) -> String {
    if unix == 0 {
        return "--:--".to_string();
    }
    let offset = local_utc_offset_minutes() as i64;
    let secs = unix as i64;
    let local = (secs + offset * 60).rem_euclid(86400);
    let h = local / 3600;
    let m = (local % 3600) / 60;
    format!("{h:02}:{m:02}")
}

fn local_utc_offset_minutes() -> i32 {
    use windows_sys::Win32::System::Time::GetTimeZoneInformation;
    let mut tzi: windows_sys::Win32::System::Time::TIME_ZONE_INFORMATION =
        unsafe { std::mem::zeroed() };
    let ret = unsafe { GetTimeZoneInformation(&mut tzi) };
    if ret == 0xFFFF_FFFF {
        return 0;
    }
    let extra = if ret == 2 { tzi.DaylightBias } else { tzi.StandardBias };
    -(tzi.Bias + extra)
}

// ---------------------------------------------------------------------------
// 皮肤图片
// ---------------------------------------------------------------------------

/// 解码后的自定义皮肤图片（0xAARRGGBB，直通 alpha，GDI+ 解码时已按目标尺寸缩放）。
#[derive(Clone)]
pub struct SkinImage {
    pub pixels: Vec<u32>,
    pub w: i32,
    pub h: i32,
}

/// 把源图按 cover 方式缩放到目标尺寸（双线性）。
fn scale_cover(src: &[u32], sw: i32, sh: i32, dw: i32, dh: i32) -> Vec<u32> {
    let mut out = vec![0u32; (dw * dh) as usize];
    if sw <= 0 || sh <= 0 || dw <= 0 || dh <= 0 {
        return out;
    }
    let scale = (dw as f32 / sw as f32).max(dh as f32 / sh as f32);
    let (crop_w, crop_h) = (dw as f32 / scale, dh as f32 / scale);
    let (off_x, off_y) = ((sw as f32 - crop_w) * 0.5, (sh as f32 - crop_h) * 0.5);
    for y in 0..dh {
        let sy = off_y + (y as f32 + 0.5) / scale - 0.5;
        let y0 = sy.floor().max(0.0) as i32;
        let y1 = (y0 + 1).min(sh - 1);
        let fy = (sy - y0 as f32).clamp(0.0, 1.0);
        for x in 0..dw {
            let sx = off_x + (x as f32 + 0.5) / scale - 0.5;
            let x0 = sx.floor().max(0.0) as i32;
            let x1 = (x0 + 1).min(sw - 1);
            let fx = (sx - x0 as f32).clamp(0.0, 1.0);
            let p00 = src[(y0 * sw + x0) as usize];
            let p10 = src[(y0 * sw + x1) as usize];
            let p01 = src[(y1 * sw + x0) as usize];
            let p11 = src[(y1 * sw + x1) as usize];
            let mut ch = [0u32; 4];
            for (i, c) in ch.iter_mut().enumerate() {
                let sh = 24 - 8 * i as u32;
                let f = |p: u32| ((p >> sh) & 0xFF) as f32;
                let top = f(p00) * (1.0 - fx) + f(p10) * fx;
                let bot = f(p01) * (1.0 - fx) + f(p11) * fx;
                *c = (top * (1.0 - fy) + bot * fy).round() as u32;
            }
            out[(y * dw + x) as usize] = (ch[0] << 24) | (ch[1] << 16) | (ch[2] << 8) | ch[3];
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 渲染器
// ---------------------------------------------------------------------------

/// UI 渲染器：一张 DIB 后备缓冲 + 字体缓存。
pub struct Renderer {
    mem_dc: HDC,
    dib: HBITMAP,
    old_bmp: HGDIOBJ,
    bits: *mut u32,
    w: i32,
    h: i32,
    dpi: u32,
    fonts: FontCache,
    window_icon: windows_sys::Win32::UI::WindowsAndMessaging::HICON,
    skin: Option<Arc<SkinImage>>,
    skin_scaled: Option<Vec<u32>>,
}

impl Renderer {
    /// 创建渲染器（按 DPI 计算物理尺寸）。失败返回 None。
    pub fn new(dpi: u32) -> Option<Renderer> {
        let mut r = Renderer {
            mem_dc: std::ptr::null_mut(),
            dib: std::ptr::null_mut(),
            old_bmp: std::ptr::null_mut(),
            bits: std::ptr::null_mut(),
            w: 0,
            h: 0,
            dpi: dpi.max(96),
            fonts: FontCache::new(),
            window_icon: std::ptr::null_mut(),
            skin: None,
            skin_scaled: None,
        };
        r.recreate(dpi);
        if r.mem_dc.is_null() || r.bits.is_null() {
            None
        } else {
            Some(r)
        }
    }

    /// 按新 DPI 重建后备缓冲（字体按设备像素缓存，一并重建；皮肤缓存同步失效）。
    pub fn recreate(&mut self, dpi: u32) {
        self.dpi = dpi.max(96);
        let s = self.dpi as f32 / 96.0;
        let w = (LOGICAL_W as f32 * s).round() as i32;
        let h = (LOGICAL_H as f32 * s).round() as i32;
        if w == self.w && h == self.h && !self.mem_dc.is_null() {
            return;
        }
        self.release();
        unsafe {
            self.mem_dc = CreateCompatibleDC(std::ptr::null_mut());
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w;
            bmi.bmiHeader.biHeight = -h;
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = 0;
            bmi.bmiHeader.biSizeImage = (w * h * 4) as u32;
            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            self.dib = CreateDIBSection(self.mem_dc, &bmi, 0, &mut bits, std::ptr::null_mut(), 0);
            self.bits = bits as *mut u32;
            if !self.dib.is_null() {
                self.old_bmp = SelectObject(self.mem_dc, self.dib as HGDIOBJ);
            }
            self.w = w;
            self.h = h;
            self.window_icon = crate::platform::tray::tray_icon_handle();
        }
        self.skin_scaled = None;
    }

    fn release(&mut self) {
        unsafe {
            if !self.mem_dc.is_null() {
                if !self.old_bmp.is_null() {
                    SelectObject(self.mem_dc, self.old_bmp);
                    self.old_bmp = std::ptr::null_mut();
                }
                if !self.dib.is_null() {
                    DeleteObject(self.dib as HGDIOBJ);
                    self.dib = std::ptr::null_mut();
                }
                DeleteDC(self.mem_dc);
                self.mem_dc = std::ptr::null_mut();
            }
        }
        self.fonts.clear();
        self.bits = std::ptr::null_mut();
        self.skin_scaled = None;
    }

    /// 设置自定义皮肤图片（None = 不使用图片皮肤）。缓存会在下次渲染时重建。
    pub fn set_skin(&mut self, skin: Option<Arc<SkinImage>>) {
        self.skin = skin;
        self.skin_scaled = None;
    }

    pub fn has_skin(&self) -> bool {
        self.skin.is_some()
    }

    /// 取按当前窗口尺寸 cover 缩放后的皮肤像素（惰性计算，失败/无皮肤返回 None）。
    pub(crate) fn skin_ready(&mut self) -> Option<&[u32]> {
        let skin = self.skin.clone()?;
        if self.skin_scaled.is_none() {
            let scaled = scale_cover(&skin.pixels, skin.w, skin.h, self.w.max(1), self.h.max(1));
            self.skin_scaled = Some(scaled);
        }
        self.skin_scaled.as_deref()
    }

    /// 把自定义皮肤以 cover 方式叠加到当前画布（由 design 在画背景时调用）。
    /// 返回 false 表示没有可用皮肤图片。
    pub(crate) fn draw_skin_background(&mut self, alpha: f32) -> bool {
        let Some(px) = self.skin_ready() else {
            return false;
        };
        let ptr = px.as_ptr();
        let len = px.len();
        {
            let mut cv = self.canvas();
            // SAFETY: skin_ready 返回的切片来自 self.skin_scaled，下面的 canvas 只写 DIB 像素，
            // 不会修改 skin_scaled；同一线程内顺序访问。
            let pixels = unsafe { std::slice::from_raw_parts(ptr, len) };
            cv.blit_overlay_argb(pixels, alpha);
        }
        true
    }

    pub fn size(&self) -> (i32, i32) {
        (self.w, self.h)
    }

    pub fn dpi(&self) -> u32 {
        self.dpi
    }

    #[inline]
    pub fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    #[inline]
    pub(crate) fn canvas(&mut self) -> Canvas {
        Canvas::new(self.w, self.h, self.bits)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn text(
        &mut self,
        value: &str,
        rect: Rect,
        size_logical: f32,
        weight: i32,
        face: u8,
        color: Color,
        align: Align,
    ) {
        if self.mem_dc.is_null() {
            return;
        }
        let s = self.scale();
        let rc = RECT {
            left: (rect.x as f32 * s).round() as i32,
            top: (rect.y as f32 * s).round() as i32,
            right: ((rect.x + rect.w) as f32 * s).round() as i32,
            bottom: ((rect.y + rect.h) as f32 * s).round() as i32,
        };
        let size_px = (size_logical * s).round() as i32;
        text::draw_text(
            self.mem_dc,
            &mut self.fonts,
            value,
            rc,
            size_px.max(1),
            weight,
            face,
            color.packed(),
            align,
        );
    }

    pub(crate) fn measure(&mut self, value: &str, size_logical: f32, weight: i32, face: u8) -> f32 {
        if self.mem_dc.is_null() {
            return 0.0;
        }
        let size_px = (size_logical * self.scale()).round() as i32;
        text::measure_text(
            self.mem_dc,
            &mut self.fonts,
            value,
            size_px.max(1),
            weight,
            face,
        ) as f32
    }

    /// 渲染一帧到后备缓冲。
    pub fn render(&mut self, st: &UiState) {
        if self.mem_dc.is_null() || self.bits.is_null() {
            return;
        }
        design::draw(self, st);
        unsafe {
            GdiFlush();
        }
    }

    /// 把后备缓冲 BitBlt 到窗口 DC。
    pub fn blit(&mut self, hwnd: HWND) {
        unsafe {
            if self.mem_dc.is_null() {
                return;
            }
            let hdc = GetDC(hwnd);
            if hdc.is_null() {
                return;
            }
            GdiFlush();
            let _ = BitBlt(hdc, 0, 0, self.w, self.h, self.mem_dc, 0, 0, SRCCOPY);
            ReleaseDC(hwnd, hdc);
        }
    }

    /// 窗口图标（由 app 设置到窗口/任务栏）。
    pub fn icon_handle(&self) -> windows_sys::Win32::UI::WindowsAndMessaging::HICON {
        self.window_icon
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.release();
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_units() {
        assert_eq!(format_bytes(1024.0 * 1024.0 * 1024.0 * 5.5), "5.5 GB");
        assert_eq!(format_bytes(1024.0 * 1024.0 * 300.0), "300 MB");
        assert_eq!(format_bytes(2048.0), "2 KB");
    }

    #[test]
    fn format_time_zero_and_regular() {
        assert_eq!(format_time(0), "--:--");
        let t = format_time(1_700_000_000);
        assert_eq!(t.len(), 5);
        assert_eq!(t.as_bytes()[2], b':');
    }

    #[test]
    fn hit_test_main_buttons() {
        let mut st = UiState::empty();
        st.page = Page::Main;
        assert_eq!(hit_test(&st, 130.0, 390.0), Some(Action::Clean));
        assert_eq!(hit_test(&st, 360.0, 28.0), Some(Action::Close));
        assert_eq!(hit_test(&st, 326.0, 28.0), Some(Action::OpenSettings));
        assert_eq!(hit_test(&st, 40.0, 514.0), Some(Action::ToggleAutoClean));
    }

    #[test]
    fn hit_test_settings_tabs_and_rows() {
        let mut st = UiState::empty();
        st.page = Page::Settings;
        // 分页标签
        assert_eq!(
            hit_test(&st, 100.0, 69.0),
            Some(Action::SwitchTab(SettingsTab::General))
        );
        assert_eq!(
            hit_test(&st, 300.0, 69.0),
            Some(Action::SwitchTab(SettingsTab::Appearance))
        );
        // 第 2 行（index 1）= 阈值输入（行中心 y = 98 + 41 + 20）
        assert_eq!(hit_test(&st, 100.0, 159.0), Some(Action::EditThreshold));
        // 第 8 行（index 7）= 主题
        assert_eq!(hit_test(&st, 200.0, 98.0 + (7 * 41) as f32 + 20.0), Some(Action::CycleTheme));
        // 外观分页：第 1 行 = 配色循环
        st.settings_tab = SettingsTab::Appearance;
        assert_eq!(hit_test(&st, 100.0, 118.0), Some(Action::CycleAccent));
        let row4 = layout::settings_row(4);
        let track = layout::slider_track(row4);
        let action = hit_test(
            &st,
            (track.x + track.w / 2) as f32,
            (row4.y + 10) as f32,
        );
        match action {
            Some(Action::SetOpacity(OpacityKind::Skin, v)) => assert!((45..=55).contains(&v)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn hit_test_threshold_editing_commits_on_outside_click() {
        let mut st = UiState::empty();
        st.page = Page::Settings;
        st.threshold_editing = Some(ThresholdEdit { text: "90".into() });
        assert_eq!(hit_test(&st, 100.0, 159.0), None);
        assert_eq!(hit_test(&st, 380.0, 500.0), Some(Action::CommitThreshold));
    }

    #[test]
    fn design_constants_sane() {
        for r in [
            layout::BTN_CLOSE,
            layout::BTN_SETTINGS,
            layout::BTN_BACK,
            layout::BTN_CLEAN,
            layout::CONFIRM_CANCEL,
            layout::CONFIRM_ACCEPT,
            layout::TAB_GENERAL,
            layout::TAB_APPEARANCE,
            layout::SETTINGS_CARD,
        ] {
            assert!(r.x >= 0 && r.y >= 0);
            assert!(r.right() <= LOGICAL_W && r.bottom() <= LOGICAL_H);
        }
        // 10 行必须全部落在卡片内
        for n in 0..design::settings_row_count(SettingsTab::General) {
            let row = layout::settings_row(n);
            assert!(row.bottom() <= layout::SETTINGS_CARD.bottom());
        }
        assert_eq!((LOGICAL_W, LOGICAL_H), (400, 560));
    }

    #[test]
    fn skin_cover_scale_keeps_target_size() {
        let src = vec![0xFF00_0000u32; 4];
        let out = scale_cover(&src, 2, 2, 3, 3);
        assert_eq!(out.len(), 9);
    }
}
