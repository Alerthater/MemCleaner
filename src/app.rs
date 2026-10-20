//! 应用主逻辑：窗口消息、状态机、定时器、清理工作线程、托盘交互。
//!
//! 线程模型：UI/主线程只做绘制与状态切换；清理（0.2~2s 的阻塞系统调用）放到
//! 一次性工作线程，完成后 PostMessage 回主线程，避免界面卡顿。

use crate::config::{self, Config, SkinPreset, ThemeMode};
use crate::engine;
use crate::platform::{
    self, autostart, dialog, elevate, image, theme, tray::Tray, tray::TrayCommand, window,
    WM_APP_SHOW, TIMER_ANIM, TIMER_TICK,
};
use crate::sys::{self, CleanError, CleanOutcome, MemStatus};
use crate::ui::{
    self, format_bytes, Action, CleanItem, ConfirmDialog, ConfirmKind, OpacityKind, Page,
    Renderer, SettingsTab, SkinImage, ThresholdEdit, Toast, ToastKind, UiState,
};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BeginPaint, EndPaint, ScreenToClient, PAINTSTRUCT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TRACKMOUSEEVENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// 清理工作线程完成通知。
const WM_APP_CLEAN_DONE: u32 = WM_APP + 3;
/// 鼠标离开窗口。
const WM_MOUSELEAVE: u32 = 0x02A3;
/// DPI 变化。
const WM_DPICHANGED: u32 = 0x02E0;
/// 字符输入（阈值编辑用）。
const WM_CHAR_MSG: u32 = 0x0102;
/// 鼠标滚轮（阈值/滑块微调）。
const WM_MOUSEWHEEL_MSG: u32 = 0x020A;

/// 应用主体。
pub struct App {
    hwnd: HWND,
    cfg: Config,
    status: MemStatus,
    elevated: bool,
    is_dark: bool,
    page: Page,
    settings_tab: SettingsTab,
    renderer: Renderer,
    tray: Tray,

    // 动画
    anim_time: f32,
    last_frame: Instant,
    anim_timer_on: bool,
    caret_phase: u32,
    display_percent: f32,
    display_used: f64,
    display_avail: f64,
    clean_progress: f32,
    clean_flash_until: Option<Instant>,

    // 清理
    cleaning: bool,
    clean_started: Instant,
    clean_rx: Option<mpsc::Receiver<Result<CleanOutcome, CleanError>>>,
    last_freed: u64,

    // 外观
    skin: Option<Arc<SkinImage>>,
    skin_loaded_path: String,

    // 交互
    hover: Option<Action>,
    pressed: Option<Action>,
    drag_slider: Option<OpacityKind>,
    opacity_dirty: bool,
    tracking_mouse: bool,
    visible: bool,
    threshold_edit: Option<String>,
    /// 刚进入阈值编辑：第一个数字键是否清空旧值（模拟全选替换）。
    threshold_edit_fresh: bool,

    // 轻提示
    toast: Option<Toast>,
    toast_born: Option<Instant>,

    // 确认框
    confirm: Option<ConfirmKind>,

    // 状态
    autostart_installed: bool,
    last_stats: Instant,
    apply_autostart_pending: bool,
}

impl App {
    /// 创建应用（窗口必须已创建，随后由 main 把它挂到 GWLP_USERDATA）。
    pub fn new(
        hwnd: HWND,
        mut cfg: Config,
        visible: bool,
        apply_autostart: bool,
        clean_on_start: bool,
    ) -> App {
        cfg.sanitize();
        let dpi = window::window_dpi(hwnd).max(96);
        // 窗口初建时按系统 DPI 估算尺寸，这里按窗口实际所在显示器 DPI 校正。
        window::resize_for_dpi(hwnd, dpi);
        window::center_on_screen(hwnd);
        let is_dark = theme::resolve_is_dark(cfg.theme);
        let elevated = sys::is_elevated();
        let status = sys::mem_status();
        let mut renderer = match Renderer::new(dpi) {
            Some(r) => r,
            None => panic!("无法创建渲染器"),
        };

        // 载入自定义皮肤（仅当配置为 Custom 且图片可解码）。
        let mut skin: Option<Arc<SkinImage>> = None;
        if cfg.skin_preset == SkinPreset::Custom && !cfg.skin_image.trim().is_empty() {
            if let Some(img) = image::load_skin_image(&cfg.skin_image) {
                let arc = Arc::new(img);
                renderer.set_skin(Some(arc.clone()));
                skin = Some(arc);
            }
        }
        log_line(&format!(
            "skin: preset={:?} image={} loaded={}",
            cfg.skin_preset,
            cfg.skin_image,
            skin.is_some()
        ));
        // 应用整体窗口不透明度（分层窗口 alpha）。
        window::set_window_opacity(hwnd, cfg.window_opacity);

        // 窗口图标
        unsafe {
            let icon = renderer.icon_handle();
            SendMessageW(hwnd, WM_SETICON, 1 /*ICON_BIG*/ as WPARAM, icon as LPARAM);
            SendMessageW(hwnd, WM_SETICON, 0 /*ICON_SMALL*/ as WPARAM, icon as LPARAM);
        }

        let mut tray = Tray::new(hwnd);
        tray.add("内存清理");

        // 主界面 1s 刷新；动画定时器按需开启。
        unsafe {
            SetTimer(hwnd, TIMER_TICK, 1000, None);
        }

        let mut app = App {
            hwnd,
            cfg,
            status,
            elevated,
            is_dark,
            page: Page::Main,
            settings_tab: SettingsTab::General,
            renderer,
            tray,
            anim_time: 0.0,
            last_frame: Instant::now(),
            anim_timer_on: false,
            caret_phase: 0,
            display_percent: 0.0,
            display_used: 0.0,
            display_avail: 0.0,
            clean_progress: 0.0,
            clean_flash_until: None,
            cleaning: false,
            clean_started: Instant::now(),
            clean_rx: None,
            last_freed: 0,
            skin,
            skin_loaded_path: String::new(),
            hover: None,
            pressed: None,
            drag_slider: None,
            opacity_dirty: false,
            tracking_mouse: false,
            visible,
            threshold_edit: None,
            threshold_edit_fresh: false,
            toast: None,
            toast_born: None,
            confirm: None,
            autostart_installed: false,
            last_stats: Instant::now(),
            apply_autostart_pending: apply_autostart,
        };
        app.skin_loaded_path = app.cfg.skin_image.clone();
        app.sync_display_values();
        log_line("app started");
        app.autostart_installed = matches!(
            autostart::status(),
            crate::platform::autostart::AutostartStatus::Installed
        );
        if app.apply_autostart_pending {
            app.apply_autostart_setting();
        }
        if visible {
            window::set_visible(hwnd, true);
            app.request_animation();
        }
        app.update_tray();
        if clean_on_start {
            app.start_clean();
        }
        app
    }

    /// 窗口消息分发。
    fn on_message(&mut self, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_PAINT => {
                self.paint(hwnd);
                0
            }
            WM_ERASEBKGND => 1,
            WM_TIMER => {
                match wparam {
                    TIMER_TICK => self.on_stats_tick(),
                    TIMER_ANIM => self.on_anim_tick(),
                    _ => {}
                }
                0
            }
            WM_MOUSEMOVE => {
                self.on_mouse_move(lparam);
                0
            }
            WM_MOUSELEAVE => {
                self.tracking_mouse = false;
                if self.hover.take().is_some() {
                    window::invalidate(hwnd);
                }
                0
            }
            WM_LBUTTONDOWN => {
                self.on_mouse_down(lparam);
                0
            }
            WM_LBUTTONUP => {
                self.on_mouse_up(lparam);
                0
            }
            WM_CHAR_MSG => {
                self.on_char(wparam as u32);
                0
            }
            WM_MOUSEWHEEL_MSG => {
                self.on_mouse_wheel(wparam, lparam);
                0
            }
            WM_APP_SHOW => {
                self.show_window();
                0
            }
            WM_APP_CLEAN_DONE => {
                self.poll_clean_result();
                0
            }
            WM_SETTINGCHANGE => {
                if self.cfg.theme == ThemeMode::System {
                    let new_dark = theme::system_is_dark();
                    if new_dark != self.is_dark {
                        self.is_dark = new_dark;
                        window::invalidate(hwnd);
                    }
                }
                0
            }
            WM_DPICHANGED => {
                let new_dpi = (wparam >> 16) as u32;
                self.on_dpi_changed(new_dpi, lparam);
                0
            }
            WM_KEYDOWN => {
                match wparam {
                    // Esc：先取消阈值编辑，再关确认框，再关设置页，最后最小化到托盘
                    0x1B => {
                        if self.threshold_edit.take().is_some() {
                            self.threshold_edit_fresh = false;
                            window::invalidate(hwnd);
                            return 0;
                        }
                        if self.confirm.take().is_some() {
                            window::invalidate(hwnd);
                            return 0;
                        }
                        if self.page == Page::Settings {
                            self.page = Page::Main;
                            window::invalidate(hwnd);
                        } else if self.cfg.minimize_to_tray {
                            self.hide_window();
                        }
                    }
                    // Enter：提交阈值输入（部分输入法/键盘路径不会产生 WM_CHAR '\r'）
                    0x0D => {
                        if self.threshold_edit.is_some() {
                            self.commit_threshold();
                        }
                    }
                    _ => {}
                }
                0
            }
            WM_CLOSE => {
                if self.cfg.minimize_to_tray {
                    self.hide_window();
                } else {
                    self.exit();
                }
                0
            }
            WM_DESTROY => {
                unsafe {
                    PostQuitMessage(0);
                }
                0
            }
            WM_NCLBUTTONDOWN => {
                log_line(&format!("WM_NCLBUTTONDOWN wparam={wparam}"));
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
            WM_NCHITTEST => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
            msg if msg == platform::tray::TRAY_CALLBACK_MSG => {
                if let Some(cmd) = self.tray.on_callback(lparam) {
                    self.on_tray_command(cmd);
                }
                0
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    // ------------------------------------------------------------------
    // 定时器
    // ------------------------------------------------------------------

    fn on_stats_tick(&mut self) {
        self.status = sys::mem_status();
        self.sync_display_values();
        self.update_tray();
        // 自动清理判定
        self.maybe_auto_clean();
        // 轻提示老化
        if let (Some(t), Some(born)) = (self.toast.as_mut(), self.toast_born) {
            t.age = born.elapsed().as_secs_f32();
            if t.age > 3.2 {
                self.toast = None;
                self.toast_born = None;
                window::invalidate(self.hwnd);
            }
        }
        // 清理兜底（防止工作线程异常丢失通知）
        if self.cleaning && self.clean_started.elapsed() > Duration::from_secs(10) {
            self.finish_clean_with_error(CleanError::Os("清理超时".to_string()));
        }
        if self.visible {
            window::invalidate(self.hwnd);
        }
    }

    fn on_anim_tick(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last_frame).as_secs_f32().min(0.1);
        self.last_frame = now;
        self.anim_time += dt;
        if self.cleaning {
            self.clean_progress = (self.clean_progress + dt / 1.2).min(1.0);
        }

        // 轻提示老化：动画定时器运行期间也更新 age，避免首次出现有 0–1s 延迟（fade-in 直接跳变）。
        if self.toast_born.is_some() {
            let age = self
                .toast_born
                .map(|b| b.elapsed().as_secs_f32())
                .unwrap_or(0.0);
            if let Some(t) = self.toast.as_mut() {
                t.age = age;
            }
            if age > 3.2 {
                self.toast = None;
                self.toast_born = None;
            }
        }

        // 阈值编辑时只需要在「光标闪烁相位」变化时重绘，避免 60fps 全速重绘。
        let editing = self.threshold_edit.is_some();
        let mut need_redraw = false;
        if editing {
            let phase = ((self.anim_time * 2.0) as u32) & 1;
            if phase != self.caret_phase {
                self.caret_phase = phase;
                need_redraw = true;
            }
        }
        if self.visible
            && (need_redraw
                || self.cleaning
                || self.clean_flash_until.map(|t| t > now).unwrap_or(false)
                || self.toast_born.map(|b| b.elapsed().as_secs_f32() < 0.4).unwrap_or(false))
        {
            window::invalidate(self.hwnd);
        }

        let need = self.cleaning
            || self.clean_flash_until.map(|t| t > now).unwrap_or(false)
            || self.toast_born.map(|b| b.elapsed().as_secs_f32() < 0.4).unwrap_or(false)
            || editing;
        if !need {
            unsafe {
                KillTimer(self.hwnd, TIMER_ANIM);
            }
            self.anim_timer_on = false;
        }
    }

    fn request_animation(&mut self) {
        if !self.anim_timer_on {
            self.last_frame = Instant::now();
            unsafe {
                SetTimer(self.hwnd, TIMER_ANIM, 16, None);
            }
            self.anim_timer_on = true;
        }
    }

    // ------------------------------------------------------------------
    // 鼠标 / 键盘交互
    // ------------------------------------------------------------------

    fn logical_pos(&self, lparam: LPARAM) -> (f32, f32) {
        let lp = lparam as u32;
        let x = (lp & 0xFFFF) as i16 as i32;
        let y = ((lp >> 16) & 0xFFFF) as i16 as i32;
        let dpi = window::window_dpi(self.hwnd) as f32 / 96.0;
        (x as f32 / dpi, y as f32 / dpi)
    }

    fn current_ui_state(&self) -> UiState {
        UiState {
            dpi: window::window_dpi(self.hwnd),
            status: self.status,
            display_percent: self.display_percent,
            display_used: self.display_used,
            display_avail: self.display_avail,
            elevated: self.elevated,
            cleaning: self.cleaning,
            clean_progress: self.clean_progress,
            clean_just_done: self
                .clean_flash_until
                .map(|t| t > Instant::now())
                .unwrap_or(false),
            is_dark: self.is_dark,
            page: self.page,
            settings_tab: self.settings_tab,
            settings: ui::SettingsSnapshot {
                auto_enabled: self.cfg.auto_clean_enabled,
                threshold: self.cfg.auto_threshold_percent,
                clean_working_sets: self.cfg.clean_working_sets,
                clean_modified: self.cfg.clean_modified,
                clean_standby: self.cfg.clean_standby,
                clean_low_priority: self.cfg.clean_low_priority,
                trim_file_cache: self.cfg.trim_file_cache,
                minimize_to_tray: self.cfg.minimize_to_tray,
                animations: self.cfg.animations,
                start_with_windows: self.cfg.start_with_windows,
                theme: self.cfg.theme,
                autostart_installed: self.autostart_installed,
                tray_show_percent: self.cfg.tray_show_percent,
                accent: self.cfg.accent,
                skin_preset: self.cfg.skin_preset,
                skin_image_name: self.cfg.skin_file_name(),
                skin_opacity: self.cfg.skin_opacity,
                panel_opacity: self.cfg.panel_opacity,
                window_opacity: self.cfg.window_opacity,
            },
            last_freed_bytes: if self.last_freed > 0 {
                self.last_freed
            } else {
                self.cfg.last_freed_bytes
            },
            last_clean_unix: self.cfg.last_clean_unix,
            hover: self.hover,
            pressed: self.pressed,
            anim_time: self.anim_time,
            toast: self.toast.clone(),
            version: env!("CARGO_PKG_VERSION"),
            confirm: self.confirm.map(build_confirm_dialog),
            threshold_editing: self
                .threshold_edit
                .as_ref()
                .map(|t| ThresholdEdit { text: t.clone() }),
        }
    }

    fn on_char(&mut self, ch: u32) {
        let fresh = self.threshold_edit_fresh;
        let Some(edit) = self.threshold_edit.as_mut() else {
            return;
        };
        match ch {
            0x08 => {
                // Backspace
                edit.pop();
                self.threshold_edit_fresh = false;
                window::invalidate(self.hwnd);
            }
            0x0D => {
                // Enter
                self.commit_threshold();
            }
            0x1B => {
                // Esc
                self.threshold_edit = None;
                window::invalidate(self.hwnd);
            }
            // 只接受数字，最多 3 位（30–100）；首次输入清空旧值（相当于全选）
            0x30..=0x39 => {
                if fresh {
                    edit.clear();
                    self.threshold_edit_fresh = false;
                }
                if edit.len() < 3 {
                    if let Some(c) = char::from_u32(ch) {
                        edit.push(c);
                        window::invalidate(self.hwnd);
                    }
                }
            }
            _ => {}
        }
    }

    fn on_mouse_wheel(&mut self, wparam: WPARAM, lparam: LPARAM) {
        let delta = ((wparam >> 16) as u16 as i16) as i32;
        if delta == 0 {
            return;
        }
        // 光标应位于窗口内才响应（Windows 会把滚轮发给焦点窗口，这里再校验一次位置）。
        let mut pt: POINT = unsafe { std::mem::zeroed() };
        unsafe {
            if GetCursorPos(&mut pt) == 0 {
                return;
            }
            if ScreenToClient(self.hwnd, &mut pt) == 0 {
                return;
            }
        }
        let dpi = window::window_dpi(self.hwnd) as f32 / 96.0;
        let (lx, ly) = (pt.x as f32 / dpi, pt.y as f32 / dpi);
        let state = self.current_ui_state();
        let Some(hit) = ui::hit_test(&state, lx, ly) else {
            return;
        };
        let step = if delta > 0 { 1i32 } else { -1i32 };
        match hit {
            Action::EditThreshold => {
                let v = (self.cfg.auto_threshold_percent as i32 + step).clamp(30, 100) as u8;
                self.cfg.auto_threshold_percent = v;
                self.save_config();
                window::invalidate(self.hwnd);
            }
            Action::SetOpacity(kind, _) => {
                let current = match kind {
                    OpacityKind::Skin => self.cfg.skin_opacity,
                    OpacityKind::Panel => self.cfg.panel_opacity,
                    OpacityKind::Window => self.cfg.window_opacity,
                } as i32;
                let next = (current + step * 2).clamp(0, 100) as u8;
                self.apply_opacity(kind, next);
                self.save_config();
            }
            _ => {}
        }
        let _ = lparam;
    }

    fn on_mouse_move(&mut self, lparam: LPARAM) {
        let (lx, ly) = self.logical_pos(lparam);
        let state = self.current_ui_state();
        let hit = ui::hit_test(&state, lx, ly);

        // 拖动滑块：实时更新数值（松手时统一保存配置）
        if let Some(kind) = self.drag_slider {
            if let Some(Action::SetOpacity(k, v)) = hit {
                if k == kind {
                    self.apply_opacity(kind, v);
                    self.opacity_dirty = true;
                }
            }
        }

        if hit != self.hover {
            self.hover = hit;
            window::invalidate(self.hwnd);
        }
        if !self.tracking_mouse {
            self.tracking_mouse = true;
            unsafe {
                let mut tme: TRACKMOUSEEVENT = std::mem::zeroed();
                tme.cbSize = std::mem::size_of::<TRACKMOUSEEVENT>() as u32;
                tme.dwFlags = 0x0000_0002; // TME_LEAVE
                tme.hwndTrack = self.hwnd;
                TrackMouseEvent(&mut tme);
            }
        }
    }

    fn on_mouse_down(&mut self, lparam: LPARAM) {
        let (lx, ly) = self.logical_pos(lparam);
        let state = self.current_ui_state();
        let hit = ui::hit_test(&state, lx, ly);
        log_line(&format!(
            "mouse_down raw=0x{:08X} dpi={} at ({lx:.2},{ly:.2}) hit={hit:?}",
            lparam as u32,
            window::window_dpi(self.hwnd)
        ));
        self.pressed = hit;
        self.drag_slider = None;
        if let Some(Action::SetOpacity(kind, v)) = hit {
            // 按下即跳到该位置，并进入拖动状态
            self.drag_slider = Some(kind);
            self.apply_opacity(kind, v);
            self.opacity_dirty = true;
        }
        if hit.is_some() {
            unsafe {
                SetCapture(self.hwnd);
            }
            window::invalidate(self.hwnd);
        } else {
            // 空白区域：拖动窗口
            window::begin_drag(self.hwnd);
        }
    }

    fn on_mouse_up(&mut self, lparam: LPARAM) {
        let pressed = self.pressed.take();
        self.drag_slider = None;
        log_line(&format!("mouse_up pressed={pressed:?}"));
        unsafe {
            ReleaseCapture();
        }
        if self.opacity_dirty {
            self.save_config();
            self.opacity_dirty = false;
        }
        if let Some(action) = pressed {
            // 滑块在按下/移动阶段已生效，松手不再重复触发动作。
            if matches!(action, Action::SetOpacity(_, _)) {
                window::invalidate(self.hwnd);
                return;
            }
            let (lx, ly) = self.logical_pos(lparam);
            let state = self.current_ui_state();
            if ui::hit_test(&state, lx, ly) == Some(action) {
                self.handle_action(action);
            }
            window::invalidate(self.hwnd);
        }
    }

    // ------------------------------------------------------------------
    // 动作
    // ------------------------------------------------------------------

    fn handle_action(&mut self, action: Action) {
        log_line(&format!("handle_action {action:?}"));
        match action {
            Action::ConfirmAccept => {
                let kind = self.confirm.take();
                match kind {
                    Some(ConfirmKind::ElevateClean) => self.relaunch_elevated("--clean-on-start"),
                    Some(ConfirmKind::ElevateAutoClean) => {
                        self.cfg.auto_clean_enabled = true;
                        if self.cfg.auto_threshold_percent == 0 && self.cfg.auto_interval_minutes == 0
                        {
                            self.cfg.auto_threshold_percent = 85;
                        }
                        self.save_config();
                        self.relaunch_elevated("");
                    }
                    Some(ConfirmKind::ElevateAutostart) => {
                        self.cfg.start_with_windows = !self.cfg.start_with_windows;
                        self.save_config();
                        self.relaunch_elevated("--apply-autostart");
                    }
                    None => {}
                }
            }
            Action::ConfirmCancel => {
                self.confirm = None;
                window::invalidate(self.hwnd);
            }
            Action::Clean => self.start_clean(),
            Action::ToggleAutoClean => self.toggle_auto_clean(),
            Action::OpenSettings => {
                self.page = Page::Settings;
                self.settings_tab = SettingsTab::General;
                self.request_animation();
                window::invalidate(self.hwnd);
            }
            Action::CloseSettings => {
                self.finish_threshold_edit();
                self.page = Page::Main;
                self.request_animation();
                window::invalidate(self.hwnd);
            }
            Action::Minimize => self.hide_window(),
            Action::Close => {
                if self.cfg.minimize_to_tray {
                    self.hide_window();
                } else {
                    self.exit();
                }
            }
            Action::CycleTheme => {
                self.cfg.theme = match self.cfg.theme {
                    ThemeMode::System => ThemeMode::Dark,
                    ThemeMode::Dark => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::System,
                };
                self.is_dark = theme::resolve_is_dark(self.cfg.theme);
                self.save_config();
                window::invalidate(self.hwnd);
            }
            Action::CycleAccent => {
                self.cfg.accent = self.cfg.accent.next();
                self.save_config();
                self.show_toast(&format!("配色：{}", self.cfg.accent.display_name()), ToastKind::Info);
            }
            Action::CycleSkinPreset => {
                self.cfg.skin_preset = self.cfg.skin_preset.next();
                if self.cfg.skin_preset != SkinPreset::Custom {
                    self.apply_skin(None);
                }
                self.save_config();
                self.show_toast(
                    &format!("皮肤：{}", self.cfg.skin_preset.display_name()),
                    ToastKind::Info,
                );
            }
            Action::PickCustomSkin => self.pick_custom_skin(),
            Action::ClearCustomSkin => self.clear_custom_skin(),
            Action::ToggleTrayPercent => {
                self.cfg.tray_show_percent = !self.cfg.tray_show_percent;
                self.save_config();
                self.update_tray();
                self.show_toast(
                    if self.cfg.tray_show_percent {
                        "托盘图标：显示内存占用率"
                    } else {
                        "托盘图标：普通图标"
                    },
                    ToastKind::Info,
                );
            }
            Action::ToggleAutostart => self.toggle_autostart(),
            Action::ToggleMinimizeToTray => {
                self.cfg.minimize_to_tray = !self.cfg.minimize_to_tray;
                self.save_config();
                window::invalidate(self.hwnd);
            }
            Action::ToggleAnimations => {
                self.cfg.animations = !self.cfg.animations;
                self.save_config();
                window::invalidate(self.hwnd);
            }
            Action::ToggleCleanItem(item) => {
                match item {
                    CleanItem::WorkingSets => {
                        self.cfg.clean_working_sets = !self.cfg.clean_working_sets
                    }
                    CleanItem::Modified => self.cfg.clean_modified = !self.cfg.clean_modified,
                    CleanItem::Standby => self.cfg.clean_standby = !self.cfg.clean_standby,
                    CleanItem::LowPriority => {
                        self.cfg.clean_low_priority = !self.cfg.clean_low_priority
                    }
                    CleanItem::FileCache => self.cfg.trim_file_cache = !self.cfg.trim_file_cache,
                }
                self.save_config();
                window::invalidate(self.hwnd);
            }
            Action::EditThreshold => self.begin_threshold_edit(),
            Action::CommitThreshold => self.commit_threshold(),
            Action::SwitchTab(tab) => {
                self.finish_threshold_edit();
                self.settings_tab = tab;
                self.hover = None;
                self.pressed = None;
                self.request_animation();
                window::invalidate(self.hwnd);
            }
            Action::SetOpacity(kind, value) => {
                self.apply_opacity(kind, value);
                self.opacity_dirty = true;
            }
        }
    }

    fn toggle_auto_clean(&mut self) {
        let enable = !self.cfg.auto_clean_enabled;
        if enable && !self.elevated {
            self.confirm = Some(ConfirmKind::ElevateAutoClean);
            window::invalidate(self.hwnd);
            return;
        }
        self.cfg.auto_clean_enabled = enable;
        self.save_config();
        self.show_toast(
            if enable {
                "已开启自动清理"
            } else {
                "已关闭自动清理"
            },
            ToastKind::Info,
        );
    }

    fn toggle_autostart(&mut self) {
        if !self.elevated {
            self.confirm = Some(ConfirmKind::ElevateAutostart);
            window::invalidate(self.hwnd);
            return;
        }
        self.cfg.start_with_windows = !self.cfg.start_with_windows;
        self.apply_autostart_setting();
    }

    fn apply_autostart_setting(&mut self) {
        let exe = elevate::current_exe();
        let result = if self.cfg.start_with_windows {
            autostart::install(&exe)
        } else {
            autostart::uninstall()
        };
        match result {
            Ok(()) => {
                self.autostart_installed = self.cfg.start_with_windows;
                let text = if self.cfg.start_with_windows {
                    "开机自启已启用"
                } else {
                    "开机自启已关闭"
                };
                self.show_toast(text, ToastKind::Success);
            }
            Err(e) => {
                self.cfg.start_with_windows = !self.cfg.start_with_windows;
                self.save_config();
                self.show_toast(&format!("设置失败：{e}"), ToastKind::Error);
            }
        }
        window::invalidate(self.hwnd);
    }

    // ------------------------------------------------------------------
    // 外观：阈值输入 / 皮肤 / 透明度
    // ------------------------------------------------------------------

    fn begin_threshold_edit(&mut self) {
        self.page = Page::Settings;
        self.settings_tab = SettingsTab::General;
        self.threshold_edit = Some(self.cfg.auto_threshold_percent.to_string());
        self.threshold_edit_fresh = true;
        self.caret_phase = 0;
        self.request_animation();
        window::invalidate(self.hwnd);
    }

    /// 解析并应用阈值输入；无效输入或空输入保持原值。
    fn commit_threshold(&mut self) {
        let Some(text) = self.threshold_edit.take() else {
            return;
        };
        self.threshold_edit_fresh = false;
        let trimmed = text.trim();
        let new_value = trimmed
            .parse::<u16>()
            .ok()
            .map(|v| v.clamp(30, 100) as u8)
            .unwrap_or(self.cfg.auto_threshold_percent);
        if new_value != self.cfg.auto_threshold_percent {
            self.cfg.auto_threshold_percent = new_value;
            self.save_config();
            self.show_toast(&format!("触发阈值已设为 {new_value}%"), ToastKind::Success);
        }
        window::invalidate(self.hwnd);
    }

    /// 离开设置页/切分页时若有未提交的输入则提交。
    fn finish_threshold_edit(&mut self) {
        if self.threshold_edit.is_some() {
            self.commit_threshold();
        }
    }

    fn apply_opacity(&mut self, kind: OpacityKind, value: u8) {
        match kind {
            OpacityKind::Skin => self.cfg.skin_opacity = value.min(100),
            OpacityKind::Panel => self.cfg.panel_opacity = value.min(100),
            OpacityKind::Window => {
                let v = value.clamp(50, 100);
                if self.cfg.window_opacity != v {
                    self.cfg.window_opacity = v;
                    window::set_window_opacity(self.hwnd, v);
                }
            }
        }
        window::invalidate(self.hwnd);
    }

    fn apply_skin(&mut self, skin: Option<Arc<SkinImage>>) {
        self.skin = skin.clone();
        self.renderer.set_skin(skin);
        window::invalidate(self.hwnd);
    }

    fn pick_custom_skin(&mut self) {
        let Some(path) = dialog::pick_image_file(self.hwnd) else {
            return;
        };
        match image::load_skin_image(&path) {
            Some(img) => {
                let arc = Arc::new(img);
                self.cfg.skin_image = path.clone();
                self.cfg.skin_preset = SkinPreset::Custom;
                self.apply_skin(Some(arc));
                self.skin_loaded_path = path;
                self.save_config();
                self.show_toast("自定义皮肤已应用", ToastKind::Success);
            }
            None => {
                self.show_toast("图片解码失败，请换一张图片", ToastKind::Error);
            }
        }
    }

    fn clear_custom_skin(&mut self) {
        if self.cfg.skin_image.is_empty() && self.cfg.skin_preset != SkinPreset::Custom {
            self.show_toast("当前没有自定义皮肤", ToastKind::Info);
            return;
        }
        self.cfg.skin_image.clear();
        self.cfg.skin_preset = SkinPreset::Default;
        self.skin_loaded_path.clear();
        self.apply_skin(None);
        self.save_config();
        self.show_toast("已恢复默认皮肤", ToastKind::Info);
    }

    // ------------------------------------------------------------------
    // 清理流程
    // ------------------------------------------------------------------

    fn start_clean(&mut self) {
        if self.cleaning {
            self.show_toast("正在清理，请稍候…", ToastKind::Info);
            return;
        }
        if !self.cfg.has_any_item() {
            self.show_toast("请先在设置中选择至少一个清理项", ToastKind::Error);
            return;
        }
        if !self.elevated {
            self.confirm = Some(ConfirmKind::ElevateClean);
            window::invalidate(self.hwnd);
            return;
        }
        let opts = engine::options_from_config(&self.cfg);
        let (tx, rx) = mpsc::channel();
        let hwnd_val = self.hwnd as isize;
        std::thread::spawn(move || {
            let result = sys::clean(&opts);
            let _ = tx.send(result);
            unsafe {
                PostMessageW(hwnd_val as HWND, WM_APP_CLEAN_DONE, 0, 0);
            }
        });
        self.clean_rx = Some(rx);
        self.cleaning = true;
        self.clean_started = Instant::now();
        self.clean_progress = 0.0;
        self.request_animation();
        self.update_tray();
        window::invalidate(self.hwnd);
    }

    fn poll_clean_result(&mut self) {
        let Some(rx) = self.clean_rx.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(outcome)) => {
                self.clean_rx = None;
                self.finish_clean_success(outcome);
            }
            Ok(Err(err)) => {
                self.clean_rx = None;
                self.finish_clean_with_error(err);
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.request_animation();
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.clean_rx = None;
                self.finish_clean_with_error(CleanError::Os("清理线程异常退出".to_string()));
            }
        }
    }

    fn finish_clean_success(&mut self, outcome: CleanOutcome) {
        self.cleaning = false;
        self.clean_progress = 1.0;
        self.clean_flash_until = Some(Instant::now() + Duration::from_secs(2));
        // 展示口径：优先用「实际回收的缓存量」（standby+modified 下降），
        // 因为 Windows 的可用内存包含 standby，只清 standby 时可用内存几乎不变。
        let reported = outcome.reclaimed_bytes.max(0) as i64;
        let freed = outcome.freed_bytes;
        self.cfg.record_clean(reported);
        self.last_freed = self.cfg.last_freed_bytes;
        self.save_config();
        self.status = sys::mem_status();
        self.sync_display_values();
        let text = if reported >= 1024 * 1024 {
            format!(
                "已清理 {} 缓存，可用内存 +{}",
                format_bytes(reported as f64),
                format_bytes(freed.max(0) as f64)
            )
        } else if freed > 0 {
            format!("已清理 {} 内存", format_bytes(freed as f64))
        } else {
            "已优化到最佳状态".to_string()
        };
        self.show_toast(&text, ToastKind::Success);
        self.update_tray();
        log_line(&format!(
            "clean ok freed={} reclaimed={} bytes before={} after={} duration={}ms steps={:?}",
            outcome.reclaimed_bytes,
            outcome.freed_bytes,
            outcome.before_avail,
            outcome.after_avail,
            outcome.duration_ms,
            outcome
                .steps
                .iter()
                .map(|s| (s.name, s.success, s.error_code))
                .collect::<Vec<_>>()
        ));
        window::invalidate(self.hwnd);
        self.request_animation();
    }

    fn finish_clean_with_error(&mut self, err: CleanError) {
        self.cleaning = false;
        self.clean_progress = 0.0;
        let text = if matches!(err, CleanError::NotElevated) {
            "清理失败：请以管理员身份运行".to_string()
        } else {
            format!("清理失败：{err}")
        };
        self.show_toast(&text, ToastKind::Error);
        log_line(&format!("clean error: {err}"));
        window::invalidate(self.hwnd);
    }

    fn maybe_auto_clean(&mut self) {
        if self.cleaning || !self.elevated || !self.cfg.auto_clean_configured() {
            return;
        }
        let since = if self.cfg.last_clean_unix > 0 {
            Some(config::unix_now().saturating_sub(self.cfg.last_clean_unix))
        } else {
            None
        };
        if engine::should_auto_clean(&self.cfg, &self.status, since) {
            log_line(&format!(
                "auto clean triggered: usage={}% threshold={}% since={:?}s",
                self.status.percent, self.cfg.auto_threshold_percent, since
            ));
            self.start_clean();
        }
    }

    // ------------------------------------------------------------------
    // 窗口显示 / 托盘
    // ------------------------------------------------------------------

    fn on_tray_command(&mut self, cmd: TrayCommand) {
        match cmd {
            TrayCommand::Show | TrayCommand::ToggleWindow => self.show_window(),
            TrayCommand::Clean => {
                if !self.visible {
                    self.show_window();
                }
                self.start_clean();
            }
            TrayCommand::Exit => self.exit(),
        }
    }

    fn show_window(&mut self) {
        self.visible = true;
        window::set_visible(self.hwnd, true);
        window::allow_message_from_lower_integrity(self.hwnd, WM_APP_SHOW);
        window::invalidate(self.hwnd);
        self.request_animation();
    }

    fn hide_window(&mut self) {
        self.finish_threshold_edit();
        if self.opacity_dirty {
            self.save_config();
            self.opacity_dirty = false;
        }
        self.visible = false;
        window::set_visible(self.hwnd, false);
    }

    fn exit(&mut self) {
        self.finish_threshold_edit();
        if self.opacity_dirty {
            self.save_config();
            self.opacity_dirty = false;
        }
        self.tray.remove();
        unsafe {
            DestroyWindow(self.hwnd);
        }
    }

    fn relaunch_elevated(&mut self, extra: &str) {
        let args = if extra.is_empty() {
            "--elevated".to_string()
        } else {
            format!("--elevated {extra}")
        };
        if elevate::relaunch_elevated(&args) {
            self.exit();
        } else {
            self.show_toast("请求管理员权限失败", ToastKind::Error);
            window::invalidate(self.hwnd);
        }
    }

    fn on_dpi_changed(&mut self, new_dpi: u32, lparam: LPARAM) {
        let rect = unsafe { &*(lparam as *const RECT) };
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                std::ptr::null_mut(),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        self.renderer.recreate(new_dpi);
        // 分层窗口属性在尺寸变化后重新应用一次。
        window::set_window_opacity(self.hwnd, self.cfg.window_opacity);
        window::invalidate(self.hwnd);
    }

    // ------------------------------------------------------------------
    // 杂项
    // ------------------------------------------------------------------

    fn sync_display_values(&mut self) {
        self.display_percent = self.status.percent as f32;
        self.display_used = self.status.used_bytes as f64;
        self.display_avail = self.status.avail_bytes as f64;
    }

    fn paint(&mut self, hwnd: HWND) {
        unsafe {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let _hdc = BeginPaint(hwnd, &mut ps);
            let state = self.current_ui_state();
            self.renderer.render(&state);
            self.renderer.blit(hwnd);
            EndPaint(hwnd, &ps);
        }
    }

    fn save_config(&self) {
        if let Err(e) = config::save(&self.cfg) {
            log_line(&format!("save config failed: {e}"));
        }
    }

    /// 刷新托盘提示文字，并按配置更新「图标显示占用率」。
    fn update_tray(&mut self) {
        let tip = format!(
            "内存清理 · {}% 已用 {}",
            self.status.percent,
            format_bytes(self.status.used_bytes as f64)
        );
        self.tray.set_tip(&tip);
        let percent = if self.cfg.tray_show_percent {
            Some(self.status.percent)
        } else {
            None
        };
        self.tray.set_percent(percent);
    }

    fn show_toast(&mut self, text: &str, kind: ToastKind) {
        self.toast = Some(Toast {
            text: text.to_string(),
            kind,
            age: 0.0,
        });
        self.toast_born = Some(Instant::now());
        self.request_animation();
        window::invalidate(self.hwnd);
    }
}

/// 构造应用内确认框文案。
fn build_confirm_dialog(kind: ConfirmKind) -> ConfirmDialog {
    match kind {
        ConfirmKind::ElevateClean => ConfirmDialog {
            title: "需要管理员权限".to_string(),
            message: "系统级内存清理需要管理员权限。\n以管理员身份重启并立即清理？".to_string(),
            confirm_label: "重启并清理".to_string(),
            cancel_label: "取消".to_string(),
            kind,
        },
        ConfirmKind::ElevateAutoClean => ConfirmDialog {
            title: "开启自动清理".to_string(),
            message: "自动清理需要管理员权限。\n以管理员身份重启并开启？".to_string(),
            confirm_label: "重启并开启".to_string(),
            cancel_label: "取消".to_string(),
            kind,
        },
        ConfirmKind::ElevateAutostart => ConfirmDialog {
            title: "设置开机自启".to_string(),
            message: "开机自启会创建管理员权限计划任务，\n登录后免 UAC 自动清理内存。\n以管理员身份重启并设置？".to_string(),
            confirm_label: "重启并设置".to_string(),
            cancel_label: "取消".to_string(),
            kind,
        },
    }
}

/// 窗口过程：从 GWLP_USERDATA 取回 App 并分发。
pub unsafe extern "system" fn wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    if ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    (*ptr).on_message(hwnd, msg, wparam, lparam)
}

/// 诊断日志（仅在设置 MEMCLEANER_LOG=1 或 debug 构建时写入文件，避免常驻开销）。
pub fn log_line(text: &str) {
    let enabled = cfg!(debug_assertions)
        || std::env::var_os("MEMCLEANER_LOG").map(|v| v == "1").unwrap_or(false);
    if !enabled {
        return;
    }
    let dir = config::config_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("memcleaner.log");
    let ts = config::unix_now();
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write;
        let _ = writeln!(file, "[{ts}] {text}");
    }
}

/// 供 main 使用的窗口提示：当前是否可见（决定启动时是否 PreShow）。
pub fn start_hidden(args: &[String]) -> bool {
    args.iter().any(|a| a == "--autostart")
}

/// 把逻辑坐标缩放到物理像素（给 main 计算窗口尺寸用）。
pub fn physical_size(dpi: u32) -> (i32, i32) {
    (
        window::scale(ui::LOGICAL_W, dpi),
        window::scale(ui::LOGICAL_H, dpi),
    )
}
