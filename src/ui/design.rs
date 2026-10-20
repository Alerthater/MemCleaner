//! 界面绘制：主页面、设置页、应用内确认框、toast、清理动画。
//!
//! 全部使用逻辑像素（400x560 设计稿坐标），由 [`Renderer`] 统一乘 DPI 缩放。
//! 绘制顺序：背景（渐变 + 仿 Mica 光晕）→ 页面内容 → toast → 确认框（模态在最上层）。
#![allow(dead_code)]

use super::raster::Color;
use super::text::{FACE_DISPLAY_IDX, FACE_ICON_IDX, FACE_TEXT_IDX};
use super::theme::{self, Theme};
use super::{
    Action, CleanItem, ConfirmDialog, Page, Rect, Renderer, Toast, ToastKind, UiState, format_bytes,
    format_time, layout,
};
use std::f32::consts::TAU;

/// MDL2 图标字形常量。
const GLYPH_GEAR: &str = "\u{E713}";
const GLYPH_CLOSE: &str = "\u{E8BB}";
const GLYPH_BACK: &str = "\u{E72B}";
const GLYPH_CHEVRON: &str = "\u{E76C}";
const GLYPH_INFO: &str = "\u{E946}";
const GLYPH_SUCCESS: &str = "\u{E73E}";
const GLYPH_ERROR: &str = "\u{EA39}";

/// 渲染一帧完整界面。
pub fn draw(r: &mut Renderer, st: &UiState) {
    let th = if st.is_dark { theme::dark() } else { theme::light() };
    draw_background(r, &th);
    match st.page {
        Page::Main => main_page(r, st, &th),
        Page::Settings => settings_page(r, st, &th),
    }
    if let Some(t) = st.toast.as_ref() {
        toast(r, st, &th, t);
    }
    if let Some(d) = st.confirm.as_ref() {
        confirm_dialog(r, st, &th, d);
    }
}

/// 背景：竖向渐变 + 两团柔和光晕（仿 Win11 Mica 材质层次）。
fn draw_background(r: &mut Renderer, th: &Theme) {
    let mut cv = r.canvas();
    cv.fill_background(th.bg_top, th.bg_bottom);
    let (w, h) = (cv.w as f32, cv.h as f32);
    let a = if th.dark { 0.20 } else { 0.15 };
    cv.radial_glow(w * 0.24, h * 0.08, w * 0.85, th.glow_a.with_alpha(a), 0.35);
    cv.radial_glow(w * 0.88, h * 0.80, w * 0.80, th.glow_b.with_alpha(a * 0.9), 0.28);
}

// ---------------------------------------------------------------------------
// 主页面
// ---------------------------------------------------------------------------

fn main_page(r: &mut Renderer, st: &UiState, th: &Theme) {
    let s = r.scale();
    let p = |v: f32| v * s;

    // ---- 卡片 ----
    {
        let c = layout::CARD;
        let mut cv = r.canvas();
        let shadow = if th.dark {
            None
        } else {
            Some((th.shadow, p(18.0), p(6.0)))
        };
        cv.rounded_rect(
            p(c.x as f32),
            p(c.y as f32),
            p(c.w as f32),
            p(c.h as f32),
            p(layout::CARD_RADIUS),
            Some(th.card),
            Some((th.border, p(1.0).max(1.0))),
            shadow,
        );
        // 环内氛围光
        let (cx, cy) = layout::RING_CENTER;
        cv.radial_glow(p(cx), p(cy), p(120.0), th.accent.with_alpha(0.022), 1.0);
    }

    // ---- 进度环 ----
    draw_ring(r, st, th);

    // ---- 环内数字与标签 ----
    let pct = st.display_percent.clamp(0.0, 100.0);
    let num = format!("{pct:.0}%");
    r.text(&num, Rect::new(120, 158, 160, 52), 40.0, 600, FACE_DISPLAY_IDX, th.text, super::text::Align::Center);
    r.text(
        "内存使用率",
        Rect::new(120, 214, 160, 16),
        11.0,
        400,
        FACE_TEXT_IDX,
        th.text2,
        super::text::Align::Center,
    );

    // ---- 两列统计 ----
    {
        let mut cv = r.canvas();
        cv.fill_rect(p(200.0), p(292.0), p(1.0).max(1.0), p(42.0), th.divider);
    }
    let used = format_bytes(st.display_used);
    let avail = format_bytes(st.display_avail);
    r.text("已用", Rect::new(68, 294, 120, 16), 11.0, 400, FACE_TEXT_IDX, th.text3, super::text::Align::Center);
    r.text(&used, Rect::new(68, 312, 120, 20), 15.0, 600, FACE_DISPLAY_IDX, th.text, super::text::Align::Center);
    r.text("可用", Rect::new(212, 294, 120, 16), 11.0, 400, FACE_TEXT_IDX, th.text3, super::text::Align::Center);
    r.text(&avail, Rect::new(212, 312, 120, 20), 15.0, 600, FACE_DISPLAY_IDX, th.text, super::text::Align::Center);

    // ---- 主按钮 ----
    draw_main_button(r, st, th);

    // ---- 上次清理提示 ----
    let last = if st.cleaning {
        "正在清理系统内存…".to_string()
    } else if st.last_freed_bytes > 0 {
        format!(
            "上次清理释放 {} · {}",
            format_bytes(st.last_freed_bytes as f64),
            format_time(st.last_clean_unix)
        )
    } else if st.clean_just_done {
        "已优化到最佳状态".to_string()
    } else if !st.elevated {
        "当前为普通权限，清理需管理员模式".to_string()
    } else {
        "尚未清理，点上方按钮试试".to_string()
    };
    r.text(
        &last,
        Rect::new(40, 427, 320, 18),
        11.0,
        400,
        FACE_TEXT_IDX,
        th.text3,
        super::text::Align::Center,
    );

    // ---- 底部分隔线 + 底部栏 ----
    {
        let mut cv = r.canvas();
        cv.fill_rect(p(24.0), p(layout::DIVIDER_Y), p(352.0), p(1.0).max(1.0), th.divider);
    }
    draw_bottom_bar(r, st, th);

    // ---- 顶部标题栏 ----
    r.text("内存清理", Rect::new(24, 14, 200, 30), 13.0, 500, FACE_TEXT_IDX, th.text2, super::text::Align::Left);
    icon_button(r, st, th, layout::BTN_SETTINGS, GLYPH_GEAR, Action::OpenSettings, 13.0);
    icon_button(r, st, th, layout::BTN_CLOSE, GLYPH_CLOSE, Action::Close, 11.0);
}

/// 进度环（含清理动画与完成闪烁）。
fn draw_ring(r: &mut Renderer, st: &UiState, th: &Theme) {
    let s = r.scale();
    let p = |v: f32| v * s;
    let (cx, cy) = layout::RING_CENTER;
    let (ccx, ccy) = (p(cx), p(cy));
    let radius = p(layout::RING_RADIUS);
    let half = p(layout::RING_THICKNESS) * 0.5;
    let mut cv = r.canvas();
    cv.ring_full(ccx, ccy, radius, half, th.track);

    if st.cleaning {
        // 清理进度弧（0..1）+ 旋转扫光，表示「正在工作」
        let prog = st.clean_progress.clamp(0.0, 1.0);
        cv.arc(
            ccx,
            ccy,
            radius,
            half,
            0.0,
            TAU * prog.max(0.02),
            th.accent2,
            th.accent,
            Some((th.accent.with_alpha(1.0), p(9.0), if th.dark { 0.22 } else { 0.15 })),
        );
        let t = st.anim_time * 2.4;
        let start = t % TAU;
        cv.arc(
            ccx,
            ccy,
            radius,
            half,
            start,
            TAU * 0.10,
            th.accent,
            th.accent,
            Some((th.accent.with_alpha(1.0), p(10.0), if th.dark { 0.20 } else { 0.14 })),
        );
    } else {
        let sweep = TAU * (st.display_percent.clamp(0.0, 100.0) / 100.0);
        let glow = Some((th.accent.with_alpha(1.0), p(10.0), if th.dark { 0.22 } else { 0.16 }));
        cv.arc(ccx, ccy, radius, half, 0.0, sweep, th.accent, th.accent2, glow);
        if st.clean_just_done {
            cv.radial_glow(ccx, ccy, p(130.0), th.success.with_alpha(0.10), 0.55);
        }
    }
}

/// 主按钮：渐变强调色 + hover/press/清理中状态。
fn draw_main_button(r: &mut Renderer, st: &UiState, th: &Theme) {
    let s = r.scale();
    let p = |v: f32| v * s;
    let b = layout::BTN_CLEAN;
    let hovered = st.hover == Some(Action::Clean) && !st.cleaning;
    let pressed = st.pressed == Some(Action::Clean) && !st.cleaning;
    let (mut c1, mut c2) = (th.accent, th.accent2);
    if st.cleaning {
        // 清理中：压暗，避免误重复点击
        c1 = c1.lerp(th.card, 0.45);
        c2 = c2.lerp(th.card, 0.45);
    } else if pressed {
        c1 = c1.lerp(Color::rgb(0, 0, 0), 0.14);
        c2 = c2.lerp(Color::rgb(0, 0, 0), 0.14);
    } else if hovered {
        c1 = c1.lerp(Color::rgb(255, 255, 255), 0.10);
        c2 = c2.lerp(Color::rgb(255, 255, 255), 0.10);
    }
    {
        let mut cv = r.canvas();
        // 按钮下方柔和泛光（深色下更明显，浅色下收窄避免发白）
        cv.radial_glow(
            p(200.0),
            p(398.0),
            p(if th.dark { 110.0 } else { 92.0 }),
            th.accent.with_alpha(1.0),
            if th.dark { 0.10 } else { 0.040 },
        );
        cv.rounded_rect_vgrad(
            p(b.x as f32),
            p(b.y as f32),
            p(b.w as f32),
            p(b.h as f32),
            p(layout::BTN_CLEAN_RADIUS),
            c1,
            c2,
            Some((th.button_border, p(1.0).max(1.0))),
            None,
        );
    }
    let label = if st.cleaning { "清理中…" } else { "清理内存" };
    r.text(
        label,
        Rect::new(b.x, b.y, b.w, b.h),
        14.0,
        600,
        FACE_TEXT_IDX,
        th.button_text,
        super::text::Align::Center,
    );
}

/// 底部栏：自动清理开关 + 权限状态。
fn draw_bottom_bar(r: &mut Renderer, st: &UiState, th: &Theme) {
    let s = r.scale();
    let p = |v: f32| v * s;
    let y = layout::BOTTOM_Y;
    // 开关所在整行可点，hover 时给一点反馈
    let hit = Rect::new(16, y - 8, 170, 36);
    {
        let mut cv = r.canvas();
        if st.hover == Some(Action::ToggleAutoClean) {
            cv.rounded_rect(p(hit.x as f32), p(hit.y as f32), p(hit.w as f32), p(hit.h as f32), p(10.0), Some(th.hover), None, None);
        }
        draw_switch(&mut cv, th, p(24.0), p(y as f32), p(34.0), p(20.0), st.settings.auto_enabled);
    }
    r.text(
        "自动清理",
        Rect::new(68, y, 110, 20),
        12.0,
        400,
        FACE_TEXT_IDX,
        th.text2,
        super::text::Align::Left,
    );
    // 右侧权限状态：圆点 + 文字
    let (label, dot) = if st.elevated {
        ("管理员模式", th.success)
    } else {
        ("普通权限", th.warning)
    };
    let text_w = r.measure(label, 10.0, 400, FACE_TEXT_IDX) / s;
    let dot_x = 376.0 - text_w - 10.0;
    {
        let mut cv = r.canvas();
        cv.circle(p(dot_x), p(y as f32 + 10.0), p(3.2), Some(dot), None);
    }
    r.text(
        label,
        Rect::new(176, y, 200, 20),
        10.0,
        400,
        FACE_TEXT_IDX,
        th.text2,
        super::text::Align::Right,
    );
}

/// 画一个 switch（开：强调色渐变；关：轨道灰）。
fn draw_switch(cv: &mut super::raster::Canvas, th: &Theme, x: f32, y: f32, w: f32, h: f32, on: bool) {
    let r = h * 0.5;
    if on {
        cv.rounded_rect_vgrad(x, y, w, h, r, th.accent, th.accent2, None, None);
    } else {
        let off = if th.dark {
            th.track.with_alpha(0.20)
        } else {
            th.track.with_alpha(0.24)
        };
        cv.rounded_rect(x, y, w, h, r, Some(off), None, None);
    }
    let knob_x = if on { x + w - h * 0.5 } else { x + h * 0.5 };
    cv.circle(knob_x, y + h * 0.5, h * 0.5 - 3.0, Some(th.switch_knob), None);
}

/// 标题栏图标按钮（28x30 热区）。
fn icon_button(
    r: &mut Renderer,
    st: &UiState,
    th: &Theme,
    rect: Rect,
    glyph: &str,
    action: Action,
    glyph_size: f32,
) {
    let s = r.scale();
    let p = |v: f32| v * s;
    let hovered = st.hover == Some(action);
    let pressed = st.pressed == Some(action);
    let is_close = matches!(action, Action::Close);
    let mut icon_color = th.text2;
    {
        let mut cv = r.canvas();
        if hovered || pressed {
            let bg = if is_close {
                th.close_hover
            } else if pressed {
                th.pressed
            } else {
                th.hover
            };
            cv.rounded_rect(
                p(rect.x as f32 + 1.0),
                p(rect.y as f32 + 1.0),
                p(rect.w as f32 - 2.0),
                p(rect.h as f32 - 2.0),
                p(7.0),
                Some(bg),
                None,
                None,
            );
            if is_close {
                icon_color = Color::rgb(255, 255, 255);
            }
        }
    }
    r.text(
        glyph,
        Rect::new(rect.x, rect.y, rect.w, rect.h),
        glyph_size,
        400,
        FACE_ICON_IDX,
        icon_color,
        super::text::Align::Center,
    );
}

// ---------------------------------------------------------------------------
// 设置页
// ---------------------------------------------------------------------------

fn settings_page(r: &mut Renderer, st: &UiState, th: &Theme) {
    let s = r.scale();
    let p = |v: f32| v * s;
    // 列表卡片
    {
        let c = layout::SETTINGS_CARD;
        let mut cv = r.canvas();
        let shadow = if th.dark {
            None
        } else {
            Some((th.shadow, p(16.0), p(5.0)))
        };
        cv.rounded_rect(
            p(c.x as f32),
            p(c.y as f32),
            p(c.w as f32),
            p(c.h as f32),
            p(18.0),
            Some(th.card),
            Some((th.border, p(1.0).max(1.0))),
            shadow,
        );
    }

    for n in 0..9 {
        let row = layout::settings_row(n);
        let action = settings_row_action(n);
        let hovered = st.hover == Some(action);
        let pressed = st.pressed == Some(action);
        // 行 hover / press 底色 + 分隔线
        {
            let mut cv = r.canvas();
            if hovered || pressed {
                let bg = if pressed { th.pressed } else { th.hover };
                cv.rounded_rect(
                    p(row.x as f32 + 6.0),
                    p(row.y as f32 + 1.0),
                    p(row.w as f32 - 12.0),
                    p(row.h as f32 - 2.0),
                    p(10.0),
                    Some(bg),
                    None,
                    None,
                );
            }
            if n < 8 {
                cv.fill_rect(
                    p(row.x as f32 + 16.0),
                    p((row.y + row.h) as f32),
                    p(row.w as f32 - 32.0),
                    p(1.0).max(1.0),
                    th.divider,
                );
            }
        }

        let (label, caption): (&str, String) = settings_row_text(n, st);
        r.text(
            label,
            Rect::new(row.x + 18, row.y + 5, 240, 18),
            13.0,
            400,
            FACE_TEXT_IDX,
            th.text,
            super::text::Align::Left,
        );
        r.text(
            &caption,
            Rect::new(row.x + 18, row.y + 23, 270, 14),
            10.0,
            400,
            FACE_TEXT_IDX,
            th.text3,
            super::text::Align::Left,
        );

        // 右侧控件
        match row_control(n, st) {
            RowControl::Switch(on) => {
                let mut cv = r.canvas();
                draw_switch(
                    &mut cv,
                    th,
                    p((row.x + row.w - 50) as f32),
                    p((row.y + 11) as f32),
                    p(34.0),
                    p(20.0),
                    on,
                );
            }
            RowControl::Value(text) => {
                r.text(
                    &text,
                    Rect::new(row.x + row.w - 160, row.y + 5, 130, 18),
                    12.0,
                    400,
                    FACE_TEXT_IDX,
                    th.text2,
                    super::text::Align::Right,
                );
                r.text(
                    GLYPH_CHEVRON,
                    Rect::new(row.x + row.w - 28, row.y + 5, 16, 18),
                    10.0,
                    400,
                    FACE_ICON_IDX,
                    th.text3,
                    super::text::Align::Center,
                );
            }
        }
    }

    // 顶部：返回 + 标题 + 关闭
    r.text("设置", Rect::new(100, 14, 200, 30), 15.0, 600, FACE_TEXT_IDX, th.text, super::text::Align::Center);
    icon_button(r, st, th, layout::BTN_BACK, GLYPH_BACK, Action::CloseSettings, 12.0);
    icon_button(r, st, th, layout::BTN_CLOSE, GLYPH_CLOSE, Action::Close, 11.0);

    // 底部版本信息 / 返回提示
    let ver = format!("MemCleaner v{}", st.version);
    r.text(&ver, Rect::new(24, 512, 240, 16), 10.0, 400, FACE_TEXT_IDX, th.text3, super::text::Align::Left);
    r.text(
        "Esc 返回",
        Rect::new(176, 512, 200, 16),
        10.0,
        400,
        FACE_TEXT_IDX,
        th.text3,
        super::text::Align::Right,
    );
}

/// 设置页第 n 行对应的动作（与 app 的 handle_action 一一对应）。
pub(crate) fn settings_row_action(n: i32) -> Action {
    match n {
        0 => Action::ToggleAutoClean,
        1 => Action::CycleThreshold,
        2 => Action::ToggleCleanItem(CleanItem::WorkingSets),
        3 => Action::ToggleCleanItem(CleanItem::Modified),
        4 => Action::ToggleCleanItem(CleanItem::Standby),
        5 => Action::ToggleCleanItem(CleanItem::LowPriority),
        6 => Action::ToggleCleanItem(CleanItem::FileCache),
        7 => Action::CycleTheme,
        8 => Action::ToggleAutostart,
        _ => Action::ToggleMinimizeToTray,
    }
}

/// 设置页每行的标题与副标题。
fn settings_row_text(n: i32, st: &UiState) -> (&'static str, String) {
    let cfg = &st.settings;
    match n {
        0 => ("自动清理", format!("内存使用率超过 {}% 时自动清理", cfg.threshold)),
        1 => ("触发阈值", format!("当前 {}%，点击循环切换", cfg.threshold)),
        2 => ("清空工作集", "把所有进程的工作集移到待机列表".to_string()),
        3 => ("刷新已修改页面", "把已修改页面写回磁盘并释放".to_string()),
        4 => ("清除待机列表", "清空系统待机内存（效果最明显）".to_string()),
        5 => ("清除低优先级待机", "只清低优先级待机，更保守".to_string()),
        6 => ("收缩文件缓存", "收缩系统文件缓存工作集".to_string()),
        7 => (
            "主题",
            "跟随系统 / 深色 / 浅色".to_string(),
        ),
        8 => (
            "开机自启",
            if cfg.autostart_installed {
                "已安装管理员计划任务".to_string()
            } else {
                "需要管理员权限".to_string()
            },
        ),
        _ => ("", String::new()),
    }
}

enum RowControl {
    Switch(bool),
    Value(String),
}

/// 设置页每行右侧控件的类型与当前值。
fn row_control(n: i32, st: &UiState) -> RowControl {
    let cfg = &st.settings;
    match n {
        0 => RowControl::Switch(cfg.auto_enabled),
        1 => RowControl::Value(format!("{}%", cfg.threshold)),
        2 => RowControl::Switch(cfg.clean_working_sets),
        3 => RowControl::Switch(cfg.clean_modified),
        4 => RowControl::Switch(cfg.clean_standby),
        5 => RowControl::Switch(cfg.clean_low_priority),
        6 => RowControl::Switch(cfg.trim_file_cache),
        7 => RowControl::Value(
            match cfg.theme {
                crate::config::ThemeMode::System => "跟随系统",
                crate::config::ThemeMode::Dark => "深色",
                crate::config::ThemeMode::Light => "浅色",
            }
            .to_string(),
        ),
        8 => RowControl::Switch(cfg.start_with_windows),
        _ => RowControl::Switch(false),
    }
}

// ---------------------------------------------------------------------------
// 模态：应用内确认框（替代系统 MessageBox）
// ---------------------------------------------------------------------------

fn confirm_dialog(r: &mut Renderer, st: &UiState, th: &Theme, d: &ConfirmDialog) {
    let s = r.scale();
    let p = |v: f32| v * s;
    // 遮罩
    {
        let mut cv = r.canvas();
        cv.fill_rect(0.0, 0.0, cv.w as f32, cv.h as f32, th.overlay);
    }
    let card = layout::CONFIRM_CARD;
    {
        let mut cv = r.canvas();
        cv.rounded_rect(
            p(card.x as f32),
            p(card.y as f32),
            p(card.w as f32),
            p(card.h as f32),
            p(16.0),
            Some(th.dialog_bg),
            Some((th.border, p(1.0).max(1.0))),
            Some((th.shadow, p(22.0), p(8.0))),
        );
    }
    // 标题 + 正文（支持 \n 多行）
    r.text(
        &d.title,
        Rect::new(card.x, card.y + 24, card.w, 26),
        15.0,
        600,
        FACE_TEXT_IDX,
        th.text,
        super::text::Align::Center,
    );
    let mut y = card.y + 62;
    for line in d.message.split('\n') {
        r.text(
            line,
            Rect::new(card.x + 16, y, card.w - 32, 20),
            12.0,
            400,
            FACE_TEXT_IDX,
            th.text2,
            super::text::Align::Center,
        );
        y += 21;
    }
    // 按钮
    let cancel = layout::CONFIRM_CANCEL;
    let accept = layout::CONFIRM_ACCEPT;
    {
        let mut cv = r.canvas();
        // 取消：幽灵按钮 + 描边
        let c_bg = if st.pressed == Some(Action::ConfirmCancel) {
            th.pressed
        } else if st.hover == Some(Action::ConfirmCancel) {
            th.hover
        } else {
            Color::rgba(0, 0, 0, 0.0)
        };
        cv.rounded_rect(
            p(cancel.x as f32),
            p(cancel.y as f32),
            p(cancel.w as f32),
            p(cancel.h as f32),
            p(10.0),
            Some(c_bg),
            Some((th.border, p(1.0).max(1.0))),
            None,
        );
        // 确定：强调色渐变
        let (mut a1, mut a2) = (th.accent, th.accent2);
        if st.pressed == Some(Action::ConfirmAccept) {
            a1 = a1.lerp(Color::rgb(0, 0, 0), 0.14);
            a2 = a2.lerp(Color::rgb(0, 0, 0), 0.14);
        } else if st.hover == Some(Action::ConfirmAccept) {
            a1 = a1.lerp(Color::rgb(255, 255, 255), 0.10);
            a2 = a2.lerp(Color::rgb(255, 255, 255), 0.10);
        }
        cv.rounded_rect_vgrad(
            p(accept.x as f32),
            p(accept.y as f32),
            p(accept.w as f32),
            p(accept.h as f32),
            p(10.0),
            a1,
            a2,
            Some((th.button_border, p(1.0).max(1.0))),
            None,
        );
    }
    r.text(
        &d.cancel_label,
        Rect::new(cancel.x, cancel.y, cancel.w, cancel.h),
        13.0,
        400,
        FACE_TEXT_IDX,
        th.text2,
        super::text::Align::Center,
    );
    r.text(
        &d.confirm_label,
        Rect::new(accept.x, accept.y, accept.w, accept.h),
        13.0,
        600,
        FACE_TEXT_IDX,
        th.button_text,
        super::text::Align::Center,
    );
}

// ---------------------------------------------------------------------------
// 轻提示 toast
// ---------------------------------------------------------------------------

fn toast(r: &mut Renderer, st: &UiState, th: &Theme, t: &Toast) {
    let age = t.age.max(0.0);
    let alpha = if age < 0.18 {
        age / 0.18
    } else if age > 2.9 {
        ((3.25 - age) / 0.35).clamp(0.0, 1.0)
    } else {
        1.0
    };
    if alpha <= 0.0 {
        return;
    }
    let (glyph, color) = match t.kind {
        ToastKind::Info => (GLYPH_INFO, th.accent),
        ToastKind::Success => (GLYPH_SUCCESS, th.success),
        ToastKind::Error => (GLYPH_ERROR, th.danger),
    };
    let s = r.scale();
    let p = |v: f32| v * s;
    let text_w = r.measure(&t.text, 11.0, 400, FACE_TEXT_IDX) / s;
    let w = (text_w + 58.0).clamp(180.0, 368.0);
    let x = (400.0 - w) * 0.5;
    let y = 60.0 - (1.0 - alpha) * 10.0;
    let bg = th.toast_bg.with_alpha(0.97 * alpha);
    {
        let mut cv = r.canvas();
        cv.rounded_rect(
            p(x),
            p(y),
            p(w),
            p(36.0),
            p(18.0),
            Some(bg),
            Some((th.border, p(1.0).max(1.0))),
            Some((th.shadow, p(14.0), p(4.0))),
        );
        let _ = st;
    }
    r.text(
        glyph,
        Rect::new(x as i32 + 14, y as i32, 18, 36),
        12.0,
        400,
        FACE_ICON_IDX,
        color,
        super::text::Align::Center,
    );
    r.text(
        &t.text,
        Rect::new(x as i32 + 36, y as i32, w as i32 - 50, 36),
        11.0,
        400,
        FACE_TEXT_IDX,
        th.text,
        super::text::Align::Left,
    );
}
