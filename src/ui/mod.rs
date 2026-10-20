//! UI 层：界面状态、交互命中测试、软件光栅化渲染。
//!
//! 渲染路径（与 spikes/ui-probe 技术验证一致）：
//! 单张 32bpp 自上而下 DIB（内存 DC）上用 SDF 逐像素画背景/卡片/圆环/按钮，
//! 文字交给 GDI `DrawTextW`（ClearType，同一块 DIB），最后一次 `BitBlt` 到窗口 DC。
//! 无 Direct2D/Write/GDI+ 依赖；渲染热路径零堆分配，常驻 PrivateUsage 约 3~4MB。
//!
//! 设计稿逻辑尺寸 400x560（100% DPI），所有绘制与命中测试都用逻辑坐标，
//! 由 [`Renderer`] 统一乘 dpi/96 换算成物理像素。

// 公开 API 中有一部分是 app.rs 依赖的稳定接口（如 Action::Minimize、UiState::empty、
// SettingsSnapshot 的 minimize_to_tray/animations 等），当前页面未全部消费，保留避免破坏调用方。
#![allow(dead_code)]

pub mod design;
pub mod raster;
pub mod text;
pub mod theme;

use crate::config::ThemeMode;
use crate::sys::MemStatus;
use raster::Canvas;
pub use raster::Rect;
use raster::Color;
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
    ToggleAutostart,
    ToggleMinimizeToTray,
    ToggleAnimations,
    ToggleCleanItem(CleanItem),
    CycleThreshold,
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
#[derive(Clone, Copy, Debug)]
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
}

/// 应用内确认框的用途（决定点「确定」后做什么）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmKind {
    /// 普通权限下点击清理：以管理员身份重启。
    ElevateClean,
    /// 普通权限下开启自动清理：以管理员身份重启。
    ElevateAutoClean,
    /// 普通权限下设置开机自启：以管理员身份重启并应用。
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
    /// 缓动后的内存使用率（0..100）。
    pub display_percent: f32,
    /// 缓动后的已用字节。
    pub display_used: f64,
    /// 缓动后的可用字节。
    pub display_avail: f64,
    pub elevated: bool,
    pub cleaning: bool,
    /// 清理动画进度 0..1。
    pub clean_progress: f32,
    /// 清理是否刚完成（用于成功闪烁）。
    pub clean_just_done: bool,
    pub is_dark: bool,
    pub page: Page,
    pub settings: SettingsSnapshot,
    pub last_freed_bytes: u64,
    pub last_clean_unix: u64,
    pub hover: Option<Action>,
    pub pressed: Option<Action>,
    /// 动画时钟（秒），用于呼吸/扫光等循环动画。
    pub anim_time: f32,
    pub toast: Option<Toast>,
    pub version: &'static str,
    /// 应用内确认框（None 表示没有）。
    pub confirm: Option<ConfirmDialog>,
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
            },
            last_freed_bytes: 0,
            last_clean_unix: 0,
            hover: None,
            pressed: None,
            anim_time: 0.0,
            toast: None,
            version: env!("CARGO_PKG_VERSION"),
            confirm: None,
        }
    }
}

// ---------------------------------------------------------------------------
// 布局（全部逻辑像素：绘制与命中测试共用同一份常量）
// ---------------------------------------------------------------------------

pub mod layout {
    use super::raster::Rect;

    /// 主页面标题栏 / 图标按钮（30x30 热区，28x28 视觉）。
    pub const TITLE_Y: i32 = 20;
    pub const BTN_CLOSE: Rect = Rect::new(348, 14, 30, 30);
    pub const BTN_SETTINGS: Rect = Rect::new(312, 14, 30, 30);
    pub const BTN_BACK: Rect = Rect::new(14, 14, 30, 30);

    /// 中央卡片与进度环。
    pub const CARD: Rect = Rect::new(24, 52, 352, 302);
    pub const CARD_RADIUS: f32 = 20.0;
    pub const RING_CENTER: (f32, f32) = (200.0, 184.0);
    pub const RING_RADIUS: f32 = 92.0;
    pub const RING_THICKNESS: f32 = 10.0;

    /// 两列统计。
    pub const STATS_LABEL_Y: i32 = 294;
    pub const STATS_VALUE_Y: i32 = 312;
    /// 兼容旧命名（app.rs 未实际使用，保留避免外部引用失效）。
    pub const STATS_Y: i32 = STATS_LABEL_Y;

    /// 主按钮（176x46，圆角 13）。点击中心约 (200,395)。
    pub const BTN_CLEAN: Rect = Rect::new(112, 372, 176, 46);
    pub const BTN_CLEAN_RADIUS: f32 = 13.0;

    /// 上次清理提示。
    pub const LAST_TEXT_Y: i32 = 427;

    /// 底部分隔线与底部栏。
    pub const DIVIDER_Y: f32 = 470.0;
    pub const BOTTOM_Y: i32 = 504;
    pub const SWITCH: Rect = Rect::new(24, 504, 34, 20);

    /// 设置页列表：卡片 + 9 行。
    pub const SETTINGS_CARD: Rect = Rect::new(24, 62, 352, 430);
    pub const SETTINGS_ROW_START: i32 = 76;
    pub const SETTINGS_ROW_H: i32 = 46;

    /// 确认框卡片与两个按钮。
    pub const CONFIRM_CARD: Rect = Rect::new(40, 186, 320, 196);
    pub const CONFIRM_CANCEL: Rect = Rect::new(64, 318, 108, 38);
    pub const CONFIRM_ACCEPT: Rect = Rect::new(228, 318, 108, 38);

    /// 设置页第 n 行（0 起）的矩形。
    pub fn settings_row(n: i32) -> Rect {
        Rect::new(24, SETTINGS_ROW_START + n * SETTINGS_ROW_H, 352, 42)
    }
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
            // 底部自动清理开关：整个「开关 + 文字」区域可点
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
            for n in 0..9 {
                if layout::settings_row(n).contains(x, y) {
                    return Some(design::settings_row_action(n));
                }
            }
            None
        }
    }
}

/// 设置页行号对应的动作（供命中测试与绘制共用）。
pub fn settings_row_action(n: i32) -> Action {
    design::settings_row_action(n)
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
    // 只用于展示「上次清理时间」，用系统时区偏移动态换算。
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
    // Bias 是「本地时间 = UTC - Bias」的分钟数；夏令时再减 DaylightBias。
    let extra = if ret == 2 { tzi.DaylightBias } else { tzi.StandardBias };
    -(tzi.Bias + extra)
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
        };
        r.recreate(dpi);
        if r.mem_dc.is_null() || r.bits.is_null() {
            None
        } else {
            Some(r)
        }
    }

    /// 按新 DPI 重建后备缓冲（字体按设备像素缓存，一并重建）。
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
            bmi.bmiHeader.biHeight = -h; // 负高度 = 自上而下
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = 0; // BI_RGB
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
    }

    pub fn size(&self) -> (i32, i32) {
        (self.w, self.h)
    }

    pub fn dpi(&self) -> u32 {
        self.dpi
    }

    /// 当前 DPI 缩放倍率（1.0 = 96 DPI）。
    #[inline]
    pub fn scale(&self) -> f32 {
        self.dpi as f32 / 96.0
    }

    /// 获取软件光栅化画布（与文字共用同一块像素缓冲）。
    #[inline]
    pub(crate) fn canvas(&mut self) -> Canvas {
        Canvas::new(self.w, self.h, self.bits)
    }

    /// 画一行文字（输入逻辑坐标与逻辑字号，内部换算成物理像素）。
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

    /// 测量文字宽度（返回物理像素）。
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
        // 确保 GDI 文字已经写入 DIB，随后 BitBlt 才能拿到完整画面。
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
        // 任意时刻只校验格式（HH:MM）
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
        assert_eq!(hit_test(&st, 200.0, 100.0), None);
        // 底部自动清理开关
        assert_eq!(hit_test(&st, 40.0, 514.0), Some(Action::ToggleAutoClean));
    }

    #[test]
    fn hit_test_confirm_dialog() {
        let mut st = UiState::empty();
        st.confirm = Some(ConfirmDialog {
            title: "t".into(),
            message: "m".into(),
            confirm_label: "ok".into(),
            cancel_label: "cancel".into(),
            kind: ConfirmKind::ElevateClean,
        });
        assert_eq!(hit_test(&st, 282.0, 337.0), Some(Action::ConfirmAccept));
        assert_eq!(hit_test(&st, 118.0, 337.0), Some(Action::ConfirmCancel));
        // 点卡片外 = 取消
        assert_eq!(hit_test(&st, 10.0, 10.0), Some(Action::ConfirmCancel));
        // 点卡片内部空白 = 无动作
        assert_eq!(hit_test(&st, 200.0, 250.0), None);
    }

    #[test]
    fn hit_test_settings_rows() {
        let mut st = UiState::empty();
        st.page = Page::Settings;
        assert_eq!(hit_test(&st, 100.0, 90.0), Some(Action::ToggleAutoClean));
        // 第 1 行=阈值，第 2 行=清空工作集，第 4 行=待机列表
        assert_eq!(hit_test(&st, 100.0, 135.0), Some(Action::CycleThreshold));
        assert_eq!(
            hit_test(&st, 100.0, 181.0),
            Some(Action::ToggleCleanItem(CleanItem::WorkingSets))
        );
        assert_eq!(
            hit_test(&st, 100.0, 273.0),
            Some(Action::ToggleCleanItem(CleanItem::Standby))
        );
        assert_eq!(hit_test(&st, 20.0, 20.0), Some(Action::CloseSettings));
        // 第 8 行=主题，第 9 行=开机自启
        assert_eq!(hit_test(&st, 200.0, 419.0), Some(Action::CycleTheme));
        assert_eq!(hit_test(&st, 200.0, 465.0), Some(Action::ToggleAutostart));
    }

    #[test]
    fn design_constants_sane() {
        // 关键热区必须落在画布内
        for r in [
            layout::BTN_CLOSE,
            layout::BTN_SETTINGS,
            layout::BTN_BACK,
            layout::BTN_CLEAN,
            layout::CONFIRM_CANCEL,
            layout::CONFIRM_ACCEPT,
        ] {
            assert!(r.x >= 0 && r.y >= 0);
            assert!(r.right() <= LOGICAL_W && r.bottom() <= LOGICAL_H);
        }
        assert_eq!((LOGICAL_W, LOGICAL_H), (400, 560));
    }
}
