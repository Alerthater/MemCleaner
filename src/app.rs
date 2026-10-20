//! 应用主逻辑：窗口消息、状态机、定时器、清理工作线程、托盘交互。
//!
//! 线程模型：UI/主线程只做绘制与状态切换；清理（0.2~2s 的阻塞系统调用）放到
//! 一次性工作线程，完成后 PostMessage 回主线程，避免界面卡顿。

use crate::config::{self, Config, ThemeMode};
use crate::engine;
use crate::platform::{
    self, autostart, elevate, theme, tray::Tray, tray::TrayCommand, window, WM_APP_SHOW,
    TIMER_ANIM, TIMER_TICK,
};
use crate::sys::{self, CleanError, CleanOutcome, MemStatus};
use crate::ui::{self, format_bytes, layout, Action, CleanItem, ConfirmDialog, ConfirmKind, Page, Renderer, Toast, ToastKind, UiState};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BeginPaint, EndPaint, PAINTSTRUCT};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TrackMouseEvent, TRACKMOUSEEVENT};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// 清理工作线程完成通知。
const WM_APP_CLEAN_DONE: u32 = WM_APP + 3;
/// 鼠标离开窗口。
const WM_MOUSELEAVE: u32 = 0x02A3;
/// DPI 变化。
const WM_DPICHANGED: u32 = 0x02E0;

/// 应用主体。
pub struct App {
    hwnd: HWND,
    cfg: Config,
    status: MemStatus,
    elevated: bool,
    is_dark: bool,
    page: Page,
    renderer: Renderer,
    tray: Tray,

    // 动画
    anim_time: f32,
    last_frame: Instant,
    anim_timer_on: bool,
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

    // 交互
    hover: Option<Action>,
    pressed: Option<Action>,
    tracking_mouse: bool,
    visible: bool,

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
    pub fn new(hwnd: HWND, cfg: Config, visible: bool, apply_autostart: bool, clean_on_start: bool) -> App {
        let dpi = window::window_dpi(hwnd).max(96);
        // 窗口初建时按系统 DPI 估算尺寸，这里按窗口实际所在显示器 DPI 校正，
        // 否则 125%/150% 缩放下会出现绘制区与窗口尺寸不一致。
        window::resize_for_dpi(hwnd, dpi);
        window::center_on_screen(hwnd);
        let is_dark = theme::resolve_is_dark(cfg.theme);
        let elevated = sys::is_elevated();
        let status = sys::mem_status();
        let renderer = match Renderer::new(dpi) {
            Some(r) => r,
            None => panic!("无法创建渲染器"),
        };

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
            renderer,
            tray,
            anim_time: 0.0,
            last_frame: Instant::now(),
            anim_timer_on: false,
            display_percent: 0.0,
            display_used: 0.0,
            display_avail: 0.0,
            clean_progress: 0.0,
            clean_flash_until: None,
            cleaning: false,
            clean_started: Instant::now(),
            clean_rx: None,
            last_freed: 0,
            hover: None,
            pressed: None,
            tracking_mouse: false,
            visible,
            toast: None,
            toast_born: None,
            confirm: None,
            autostart_installed: false,
            last_stats: Instant::now(),
            apply_autostart_pending: apply_autostart,
        };
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
        app.update_tray_tip();
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
                if wparam == 0x1B {
                    // Esc：先关确认框，再关设置页，最后最小化到托盘
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
            WM_NCHITTEST => {
                // 让整个客户区都可点击（返回 HTCLIENT 由默认处理即可，这里只记录）
                let r = unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
                r
            }
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
        self.update_tray_tip();
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
        if self.visible {
            window::invalidate(self.hwnd);
        }
        // 动画是否还需要继续
        let need = self.cleaning
            || self.clean_flash_until.map(|t| t > now).unwrap_or(false)
            || self.toast_born.map(|b| b.elapsed().as_secs_f32() < 0.4).unwrap_or(false);
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
    // 鼠标交互
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
        }
    }

    fn on_mouse_move(&mut self, lparam: LPARAM) {
        let (lx, ly) = self.logical_pos(lparam);
        let state = self.current_ui_state();
        let hit = ui::hit_test(&state, lx, ly);
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
        log_line(&format!("mouse_up pressed={pressed:?}"));
        unsafe {
            ReleaseCapture();
        }
        if let Some(action) = pressed {
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
                    Some(ConfirmKind::ElevateClean) => {
                        // 以管理员身份重启并立刻清理
                        self.relaunch_elevated("--clean-on-start");
                    }
                    Some(ConfirmKind::ElevateAutoClean) => {
                        self.cfg.auto_clean_enabled = true;
                        if self.cfg.auto_threshold_percent == 0 && self.cfg.auto_interval_minutes == 0 {
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
                self.request_animation();
                window::invalidate(self.hwnd);
            }
            Action::CloseSettings => {
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
                    CleanItem::WorkingSets => self.cfg.clean_working_sets = !self.cfg.clean_working_sets,
                    CleanItem::Modified => self.cfg.clean_modified = !self.cfg.clean_modified,
                    CleanItem::Standby => self.cfg.clean_standby = !self.cfg.clean_standby,
                    CleanItem::LowPriority => self.cfg.clean_low_priority = !self.cfg.clean_low_priority,
                    CleanItem::FileCache => self.cfg.trim_file_cache = !self.cfg.trim_file_cache,
                }
                self.save_config();
                window::invalidate(self.hwnd);
            }
            Action::CycleThreshold => {
                self.cfg.auto_threshold_percent = match self.cfg.auto_threshold_percent {
                    0..=64 => 70,
                    65..=74 => 80,
                    75..=84 => 85,
                    85..=89 => 90,
                    _ => 60,
                };
                self.save_config();
                window::invalidate(self.hwnd);
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
        if enable && self.cfg.auto_threshold_percent == 0 && self.cfg.auto_interval_minutes == 0 {
            self.cfg.auto_threshold_percent = 85;
        }
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
        self.update_tray_tip();
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
                // 还没完成，继续等
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
            format!("已清理 {} 缓存，可用内存 +{}", format_bytes(reported as f64), format_bytes(freed.max(0) as f64))
        } else if freed > 0 {
            format!("已清理 {} 内存", format_bytes(freed as f64))
        } else {
            "已优化到最佳状态".to_string()
        };
        self.show_toast(&text, ToastKind::Success);
        self.update_tray_tip();
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
        self.visible = false;
        window::set_visible(self.hwnd, false);
    }

    fn exit(&mut self) {
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

    fn update_tray_tip(&mut self) {
        let tip = format!(
            "内存清理 · {}% 已用 {}",
            self.status.percent,
            format_bytes(self.status.used_bytes as f64)
        );
        self.tray.set_tip(&tip);
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
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
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

