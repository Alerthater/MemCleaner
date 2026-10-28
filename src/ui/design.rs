//! 界面绘制：主页面、设置页（分页标签 / 行 / 滑块 / 阈值输入）、应用内确认框、toast、清理动画。
//!
//! 全部使用逻辑像素（400x560 设计稿坐标），由 [`Renderer`] 统一乘 DPI 缩放。
//! 绘制顺序：背景（皮肤 + 光晕）→ 页面内容 → toast → 确认框（模态在最上层）。
//!
//! 背景每帧零堆分配：全部是直接光栅化调用；自定义皮肤像素由 `Renderer` 按窗口尺寸
//! cover 缩放后缓存（`draw_skin_background`），失败时回退到默认程序化背景。
#![allow(dead_code)]

use super::raster::{Canvas, Color};
use super::text::{Align, FACE_DISPLAY_IDX, FACE_ICON_IDX, FACE_TEXT_IDX};
use super::theme::{self, Theme};
use super::{
    Action, CleanItem, ConfirmDialog, OpacityKind, Page, Rect, Renderer, SettingsTab, Toast,
    ToastKind, UiState, format_bytes, format_time, layout,
};
use crate::config::{AccentScheme, SkinPreset, ThemeMode};
use std::f32::consts::TAU;

/// MDL2 图标字形常量。
const GLYPH_GEAR: &str = "\u{E713}";
const GLYPH_CLOSE: &str = "\u{E8BB}";
const GLYPH_BACK: &str = "\u{E72B}";
const GLYPH_CHEVRON: &str = "\u{E76C}";
const GLYPH_EDIT: &str = "\u{E70F}";
const GLYPH_INFO: &str = "\u{E946}";
const GLYPH_SUCCESS: &str = "\u{E73E}";
const GLYPH_ERROR: &str = "\u{EA39}";

/// 渲染一帧完整界面。
pub fn draw(r: &mut Renderer, st: &UiState) {
    let th = theme::build(st.is_dark, st.settings.accent, st.settings.panel_opacity);
    draw_background(r, st, &th);
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

// ---------------------------------------------------------------------------
// 背景：默认 / 4 套程序化皮肤 / 自定义图片
// ---------------------------------------------------------------------------

/// 一处背景光晕（比例坐标 + 比例半径）。
#[derive(Clone, Copy)]
struct Glow {
    cx: f32,
    cy: f32,
    radius: f32,
    color: Color,
    alpha: f32,
    strength: f32,
}

/// 一套程序化皮肤背景（深/浅分别定义）。
struct Backdrop {
    top: Color,
    bottom: Color,
    glows: [Glow; 3],
}

macro_rules! glow {
    ($cx:expr, $cy:expr, $r:expr, $c:expr, $a:expr, $s:expr) => {
        Glow {
            cx: $cx,
            cy: $cy,
            radius: $r,
            color: $c,
            alpha: $a,
            strength: $s,
        }
    };
}

// ---- 极光 Aurora：青绿 + 冷蓝，纵向流动感 ----
const AURORA_DARK: Backdrop = Backdrop {
    top: Color::rgb(0x0A, 0x1A, 0x22),
    bottom: Color::rgb(0x0C, 0x10, 0x20),
    glows: [
        glow!(0.14, 0.04, 0.95, Color::rgb(0x3B, 0xE8, 0xB0), 0.27, 0.42),
        glow!(0.86, 0.40, 0.90, Color::rgb(0x4C, 0xC2, 0xFF), 0.22, 0.34),
        glow!(0.50, 1.02, 0.85, Color::rgb(0x35, 0x7B, 0xD8), 0.14, 0.30),
    ],
};
const AURORA_LIGHT: Backdrop = Backdrop {
    top: Color::rgb(0xEE, 0xFB, 0xF4),
    bottom: Color::rgb(0xF2, 0xF8, 0xFF),
    glows: [
        glow!(0.14, 0.04, 0.95, Color::rgb(0x3B, 0xE8, 0xB0), 0.13, 0.40),
        glow!(0.86, 0.40, 0.90, Color::rgb(0x4C, 0xC2, 0xFF), 0.11, 0.32),
        glow!(0.50, 1.02, 0.85, Color::rgb(0x9B, 0xF6, 0xD8), 0.10, 0.30),
    ],
};

// ---- 星云 Nebula：紫 + 品红 + 深蓝 ----
const NEBULA_DARK: Backdrop = Backdrop {
    top: Color::rgb(0x16, 0x0C, 0x26),
    bottom: Color::rgb(0x0A, 0x09, 0x16),
    glows: [
        glow!(0.72, 0.06, 0.90, Color::rgb(0xA4, 0x5C, 0xFF), 0.28, 0.40),
        glow!(0.16, 0.48, 0.85, Color::rgb(0xFF, 0x5C, 0xA8), 0.20, 0.36),
        glow!(0.90, 0.92, 0.85, Color::rgb(0x4C, 0x6B, 0xFF), 0.18, 0.32),
    ],
};
const NEBULA_LIGHT: Backdrop = Backdrop {
    top: Color::rgb(0xF7, 0xF1, 0xFF),
    bottom: Color::rgb(0xFF, 0xF7, 0xFC),
    glows: [
        glow!(0.72, 0.06, 0.90, Color::rgb(0xA4, 0x5C, 0xFF), 0.11, 0.38),
        glow!(0.16, 0.48, 0.85, Color::rgb(0xFF, 0x5C, 0xA8), 0.09, 0.34),
        glow!(0.90, 0.92, 0.85, Color::rgb(0x6C, 0x8B, 0xFF), 0.09, 0.30),
    ],
};

// ---- 晨雾 Dawn：暖桃 + 樱粉，柔和 ----
const DAWN_DARK: Backdrop = Backdrop {
    top: Color::rgb(0x24, 0x18, 0x18),
    bottom: Color::rgb(0x10, 0x0C, 0x0E),
    glows: [
        glow!(0.20, 0.10, 0.95, Color::rgb(0xFF, 0xB0, 0x7C), 0.26, 0.42),
        glow!(0.82, 0.52, 0.85, Color::rgb(0xFF, 0x8F, 0xA3), 0.19, 0.34),
        glow!(0.50, 0.95, 0.80, Color::rgb(0xFF, 0xD9, 0xA8), 0.11, 0.28),
    ],
};
const DAWN_LIGHT: Backdrop = Backdrop {
    top: Color::rgb(0xFF, 0xF6, 0xEE),
    bottom: Color::rgb(0xFF, 0xFB, 0xFD),
    glows: [
        glow!(0.20, 0.10, 0.95, Color::rgb(0xFF, 0xC9, 0x8A), 0.15, 0.40),
        glow!(0.82, 0.52, 0.85, Color::rgb(0xFF, 0x9F, 0xB0), 0.11, 0.32),
        glow!(0.50, 0.95, 0.80, Color::rgb(0xFF, 0xE2, 0xC0), 0.10, 0.28),
    ],
};

// ---- 暮色 Dusk：暖橙 + 深紫 ----
const DUSK_DARK: Backdrop = Backdrop {
    top: Color::rgb(0x1A, 0x12, 0x2E),
    bottom: Color::rgb(0x0B, 0x08, 0x14),
    glows: [
        glow!(0.84, 0.10, 0.90, Color::rgb(0xFF, 0x7A, 0x59), 0.24, 0.38),
        glow!(0.16, 0.76, 0.90, Color::rgb(0x8A, 0x5C, 0xFF), 0.26, 0.38),
        glow!(0.52, 1.00, 0.80, Color::rgb(0xFF, 0xA0, 0x70), 0.10, 0.26),
    ],
};
const DUSK_LIGHT: Backdrop = Backdrop {
    top: Color::rgb(0xFF, 0xF0, 0xE8),
    bottom: Color::rgb(0xF3, 0xEF, 0xFF),
    glows: [
        glow!(0.84, 0.10, 0.90, Color::rgb(0xFF, 0x8F, 0x70), 0.12, 0.36),
        glow!(0.16, 0.76, 0.90, Color::rgb(0x9B, 0x7B, 0xFF), 0.10, 0.36),
        glow!(0.52, 1.00, 0.80, Color::rgb(0xFF, 0xC2, 0x9B), 0.09, 0.26),
    ],
};

/// 程序化皮肤预设：按主题取色。
fn preset_backdrop(preset: SkinPreset, dark: bool) -> Option<&'static Backdrop> {
    match (preset, dark) {
        (SkinPreset::Aurora, true) => Some(&AURORA_DARK),
        (SkinPreset::Aurora, false) => Some(&AURORA_LIGHT),
        (SkinPreset::Nebula, true) => Some(&NEBULA_DARK),
        (SkinPreset::Nebula, false) => Some(&NEBULA_LIGHT),
        (SkinPreset::Dawn, true) => Some(&DAWN_DARK),
        (SkinPreset::Dawn, false) => Some(&DAWN_LIGHT),
        (SkinPreset::Dusk, true) => Some(&DUSK_DARK),
        (SkinPreset::Dusk, false) => Some(&DUSK_LIGHT),
        _ => None,
    }
}

/// 画一套程序化背景（渐变 + 三团光晕，无堆分配）。
fn paint_backdrop(r: &mut Renderer, b: &Backdrop) {
    let mut cv = r.canvas();
    cv.fill_background(b.top, b.bottom);
    let (w, h) = (cv.w as f32, cv.h as f32);
    for g in &b.glows {
        cv.radial_glow(
            w * g.cx,
            h * g.cy,
            w * g.radius,
            g.color.with_alpha(g.alpha),
            g.strength,
        );
    }
}

/// 默认背景：竖向渐变 + 两团柔和光晕（仿 Win11 Mica 材质层次）。
fn default_background(r: &mut Renderer, th: &Theme) {
    let mut cv = r.canvas();
    cv.fill_background(th.bg_top, th.bg_bottom);
    let (w, h) = (cv.w as f32, cv.h as f32);
    let a = if th.dark { 0.20 } else { 0.15 };
    cv.radial_glow(w * 0.24, h * 0.08, w * 0.85, th.glow_a.with_alpha(a), 0.35);
    cv.radial_glow(
        w * 0.88,
        h * 0.80,
        w * 0.80,
        th.glow_b.with_alpha(a * 0.9),
        0.28,
    );
}

/// 背景：默认 / 4 套程序化皮肤 / 自定义图片（每帧零堆分配）。
fn draw_background(r: &mut Renderer, st: &UiState, th: &Theme) {
    let preset = st.settings.skin_preset;

    if preset == SkinPreset::Custom {
        // 先铺主题底色：既是半透明 PNG 的衬底，也保证不残留上一帧像素。
        {
            let mut cv = r.canvas();
            cv.fill_background(th.bg_top, th.bg_bottom);
        }
        let alpha = st.settings.skin_opacity.min(100) as f32 / 100.0;
        if r.draw_skin_background(alpha) {
            let mut cv = r.canvas();
            let (w, h) = (cv.w as f32, cv.h as f32);
            // 可读性遮罩：压暗/压亮图片，保证卡片与文字对比度。
            let mask = if th.dark {
                th.bg_top.with_alpha(0.55)
            } else {
                th.bg_bottom.with_alpha(0.42)
            };
            cv.fill_rect(0.0, 0.0, w, h, mask);
            // 再叠一层主题光晕，让图片与整体 UI 色调统一。
            let a = if th.dark { 0.16 } else { 0.10 };
            cv.radial_glow(w * 0.20, h * 0.04, w * 0.95, th.glow_a.with_alpha(a), 0.40);
            cv.radial_glow(
                w * 0.86,
                h * 0.86,
                w * 0.85,
                th.glow_b.with_alpha(a * 0.9),
                0.30,
            );
            return;
        }
        // 图片未加载 / 解码失败：回退默认背景。
    }

    if let Some(backdrop) = preset_backdrop(preset, th.dark) {
        paint_backdrop(r, backdrop);
    } else {
        default_background(r, th);
    }
}

// ---------------------------------------------------------------------------
// 主页面（保持 v1.0 版式与交互热区不变，仅配色/背景跟随主题）
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
    r.text(
        &num,
        Rect::new(120, 158, 160, 52),
        40.0,
        600,
        FACE_DISPLAY_IDX,
        th.text,
        Align::Center,
    );
    r.text(
        "内存使用率",
        Rect::new(120, 214, 160, 16),
        11.0,
        400,
        FACE_TEXT_IDX,
        th.text2,
        Align::Center,
    );

    // ---- 两列统计 ----
    {
        let mut cv = r.canvas();
        cv.fill_rect(p(200.0), p(292.0), p(1.0).max(1.0), p(42.0), th.divider);
    }
    let used = format_bytes(st.display_used);
    let avail = format_bytes(st.display_avail);
    r.text(
        "已用",
        Rect::new(68, 294, 120, 16),
        11.0,
        400,
        FACE_TEXT_IDX,
        th.text3,
        Align::Center,
    );
    r.text(
        &used,
        Rect::new(68, 312, 120, 20),
        15.0,
        600,
        FACE_DISPLAY_IDX,
        th.text,
        Align::Center,
    );
    r.text(
        "可用",
        Rect::new(212, 294, 120, 16),
        11.0,
        400,
        FACE_TEXT_IDX,
        th.text3,
        Align::Center,
    );
    r.text(
        &avail,
        Rect::new(212, 312, 120, 20),
        15.0,
        600,
        FACE_DISPLAY_IDX,
        th.text,
        Align::Center,
    );

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
        Align::Center,
    );

    // ---- 底部分隔线 + 底部栏 ----
    {
        let mut cv = r.canvas();
        cv.fill_rect(
            p(24.0),
            p(layout::DIVIDER_Y),
            p(352.0),
            p(1.0).max(1.0),
            th.divider,
        );
    }
    draw_bottom_bar(r, st, th);

    // ---- 顶部标题栏 ----
    r.text(
        "内存清理",
        Rect::new(24, 14, 200, 30),
        13.0,
        500,
        FACE_TEXT_IDX,
        th.text2,
        Align::Left,
    );
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
        Align::Center,
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
            cv.rounded_rect(
                p(hit.x as f32),
                p(hit.y as f32),
                p(hit.w as f32),
                p(hit.h as f32),
                p(10.0),
                Some(th.hover),
                None,
                None,
            );
        }
        draw_switch(
            &mut cv,
            th,
            p(24.0),
            p(y as f32),
            p(34.0),
            p(20.0),
            st.settings.auto_enabled,
        );
    }
    r.text(
        "自动清理",
        Rect::new(68, y, 110, 20),
        12.0,
        400,
        FACE_TEXT_IDX,
        th.text2,
        Align::Left,
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
        Align::Right,
    );
}

/// 画一个 switch（开：强调色渐变；关：轨道灰）。
fn draw_switch(cv: &mut Canvas, th: &Theme, x: f32, y: f32, w: f32, h: f32, on: bool) {
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
        Align::Center,
    );
}

// ---------------------------------------------------------------------------
// 设置页：分页标签 + 行列表
// ---------------------------------------------------------------------------

/// 设置页第 n 行的控件类型（mod.rs 的命中测试与 design 的绘制共用）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowKind {
    /// 点击整行触发一个动作（开关 / 循环切换）。
    Action(Action),
    /// 触发阈值数值输入框。
    ThresholdInput,
    /// 透明度滑块。
    Slider(OpacityKind),
}

/// 某分页的行数。
pub(crate) fn settings_row_count(tab: SettingsTab) -> i32 {
    match tab {
        SettingsTab::General => 10,
        SettingsTab::Appearance => 7,
    }
}

/// 某分页第 n 行（0 起）对应的控件类型。
pub(crate) fn settings_row_kind(tab: SettingsTab, n: i32) -> RowKind {
    match (tab, n) {
        // ---- 通用 ----
        (SettingsTab::General, 0) => RowKind::Action(Action::ToggleAutoClean),
        (SettingsTab::General, 1) => RowKind::ThresholdInput,
        (SettingsTab::General, 2) => {
            RowKind::Action(Action::ToggleCleanItem(CleanItem::WorkingSets))
        }
        (SettingsTab::General, 3) => RowKind::Action(Action::ToggleCleanItem(CleanItem::Modified)),
        (SettingsTab::General, 4) => RowKind::Action(Action::ToggleCleanItem(CleanItem::Standby)),
        (SettingsTab::General, 5) => {
            RowKind::Action(Action::ToggleCleanItem(CleanItem::LowPriority))
        }
        (SettingsTab::General, 6) => RowKind::Action(Action::ToggleCleanItem(CleanItem::FileCache)),
        (SettingsTab::General, 7) => RowKind::Action(Action::CycleTheme),
        (SettingsTab::General, 8) => RowKind::Action(Action::ToggleAutostart),
        (SettingsTab::General, 9) => RowKind::Action(Action::ToggleTrayPercent),
        // ---- 外观 ----
        (SettingsTab::Appearance, 0) => RowKind::Action(Action::CycleAccent),
        (SettingsTab::Appearance, 1) => RowKind::Action(Action::CycleSkinPreset),
        (SettingsTab::Appearance, 2) => RowKind::Action(Action::PickCustomSkin),
        (SettingsTab::Appearance, 3) => RowKind::Action(Action::ClearCustomSkin),
        (SettingsTab::Appearance, 4) => RowKind::Slider(OpacityKind::Skin),
        (SettingsTab::Appearance, 5) => RowKind::Slider(OpacityKind::Panel),
        (SettingsTab::Appearance, 6) => RowKind::Slider(OpacityKind::Window),
        // 越界保护（命中测试只会在 0..count 内调用）。
        _ => RowKind::Action(Action::CloseSettings),
    }
}

/// 主题模式展示名。
fn theme_display(mode: ThemeMode) -> &'static str {
    match mode {
        ThemeMode::System => "跟随系统",
        ThemeMode::Dark => "深色",
        ThemeMode::Light => "浅色",
    }
}

/// 某行的标题 + 副标题。
pub(crate) fn settings_row_text(tab: SettingsTab, n: i32, st: &UiState) -> (&'static str, String) {
    let cfg = &st.settings;
    match (tab, n) {
        // ---- 通用 ----
        (SettingsTab::General, 0) => (
            "自动清理",
            format!("内存使用率超过 {}% 时自动清理", cfg.threshold),
        ),
        (SettingsTab::General, 1) => {
            if st.threshold_editing.is_some() {
                ("触发阈值", "输入 30–100，Enter 确认".to_string())
            } else {
                (
                    "触发阈值",
                    format!("当前 {}%，点击输入 30–100 的值", cfg.threshold),
                )
            }
        }
        (SettingsTab::General, 2) => (
            "清空工作集",
            "把所有进程的工作集移到待机列表".to_string(),
        ),
        (SettingsTab::General, 3) => (
            "刷新已修改页面",
            "把已修改页面写回磁盘并释放".to_string(),
        ),
        (SettingsTab::General, 4) => (
            "清除待机列表",
            "清空系统待机内存（效果最明显）".to_string(),
        ),
        (SettingsTab::General, 5) => (
            "清除低优先级待机",
            "只清低优先级待机，更保守".to_string(),
        ),
        (SettingsTab::General, 6) => ("收缩文件缓存", "收缩系统文件缓存工作集".to_string()),
        (SettingsTab::General, 7) => (
            "主题",
            format!("当前 {}，点击循环切换", theme_display(cfg.theme)),
        ),
        (SettingsTab::General, 8) => (
            "开机自启",
            if cfg.autostart_installed && cfg.autostart_elevated {
                "已安装管理员计划任务（登录静默启动）".to_string()
            } else if cfg.autostart_installed {
                "普通权限自启：点开关可升级为管理员".to_string()
            } else {
                "需要管理员权限（点开关时 UAC 授权）".to_string()
            },
        ),
        (SettingsTab::General, 9) => (
            "托盘显示占用率",
            "托盘图标直接显示内存百分比".to_string(),
        ),
        // ---- 外观 ----
        (SettingsTab::Appearance, 0) => (
            "配色方案",
            format!("当前 {}，点击循环切换", cfg.accent.display_name()),
        ),
        (SettingsTab::Appearance, 1) => {
            let caption = if cfg.skin_preset == SkinPreset::Custom {
                if cfg.skin_image_name.is_empty() {
                    "自定义图片（未选择文件）".to_string()
                } else {
                    format!("自定义图片：{}", cfg.skin_image_name)
                }
            } else {
                format!("当前预设：{}", cfg.skin_preset.display_name())
            };
            ("背景皮肤", caption)
        }
        (SettingsTab::Appearance, 2) => (
            "选择自定义图片",
            if cfg.skin_image_name.is_empty() {
                "选择一张本地图片作为背景".to_string()
            } else {
                format!("已选择：{}", cfg.skin_image_name)
            },
        ),
        (SettingsTab::Appearance, 3) => (
            "清除自定义皮肤",
            if cfg.skin_preset == SkinPreset::Custom {
                "恢复为程序化背景".to_string()
            } else {
                "当前未使用自定义图片".to_string()
            },
        ),
        (SettingsTab::Appearance, 4) => (
            "皮肤不透明度",
            format!("当前 {}%，自定义图片的叠加强度", cfg.skin_opacity),
        ),
        (SettingsTab::Appearance, 5) => (
            "面板不透明度",
            format!("当前 {}%，卡片与分隔线的透明度", cfg.panel_opacity),
        ),
        (SettingsTab::Appearance, 6) => (
            "窗口不透明度",
            format!("当前 {}%，整个窗口的透明度", cfg.window_opacity),
        ),
        _ => ("", String::new()),
    }
}

/// 开关类行 -> 当前开关状态。
fn switch_value(action: Action, st: &UiState) -> Option<bool> {
    let c = &st.settings;
    Some(match action {
        Action::ToggleAutoClean => c.auto_enabled,
        Action::ToggleCleanItem(CleanItem::WorkingSets) => c.clean_working_sets,
        Action::ToggleCleanItem(CleanItem::Modified) => c.clean_modified,
        Action::ToggleCleanItem(CleanItem::Standby) => c.clean_standby,
        Action::ToggleCleanItem(CleanItem::LowPriority) => c.clean_low_priority,
        Action::ToggleCleanItem(CleanItem::FileCache) => c.trim_file_cache,
        Action::ToggleAutostart => c.start_with_windows,
        Action::ToggleTrayPercent => c.tray_show_percent,
        Action::ToggleMinimizeToTray => c.minimize_to_tray,
        Action::ToggleAnimations => c.animations,
        _ => return None,
    })
}

/// 「值 + chevron」类行 -> 当前展示值。
fn action_value(action: Action, st: &UiState) -> Option<&'static str> {
    let c = &st.settings;
    Some(match action {
        Action::CycleTheme => theme_display(c.theme),
        Action::CycleAccent => c.accent.display_name(),
        Action::CycleSkinPreset => c.skin_preset.display_name(),
        Action::PickCustomSkin => {
            if c.skin_image_name.is_empty() {
                "选择…"
            } else {
                "更换…"
            }
        }
        Action::ClearCustomSkin => "清除",
        _ => return None,
    })
}

/// 滑块当前值（0–100）。
fn slider_value(kind: OpacityKind, st: &UiState) -> u8 {
    match kind {
        OpacityKind::Skin => st.settings.skin_opacity,
        OpacityKind::Panel => st.settings.panel_opacity,
        OpacityKind::Window => st.settings.window_opacity,
    }
}

/// 行是否处于 hover 状态（滑块按「同类动作」模糊匹配）。
fn row_hovered(st: &UiState, kind: RowKind) -> bool {
    match kind {
        RowKind::Action(a) => st.hover == Some(a),
        RowKind::ThresholdInput => st.hover == Some(Action::EditThreshold),
        RowKind::Slider(k) => matches!(st.hover, Some(Action::SetOpacity(x, _)) if x == k),
    }
}

/// 行是否处于按下状态。
fn row_pressed(st: &UiState, kind: RowKind) -> bool {
    match kind {
        RowKind::Action(a) => st.pressed == Some(a),
        RowKind::ThresholdInput => st.pressed == Some(Action::EditThreshold),
        RowKind::Slider(k) => matches!(st.pressed, Some(Action::SetOpacity(x, _)) if x == k),
    }
}

/// 设置页：分页标签 + 列表卡片 + 行 + 底部信息。
fn settings_page(r: &mut Renderer, st: &UiState, th: &Theme) {
    let s = r.scale();
    let p = |v: f32| v * s;
    draw_tabs(r, st, th);

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

    let tab = st.settings_tab;
    let count = settings_row_count(tab);
    for n in 0..count {
        let row = layout::settings_row(n);
        let kind = settings_row_kind(tab, n);
        let editing_row = tab == SettingsTab::General && n == 1 && st.threshold_editing.is_some();
        let hovered = row_hovered(st, kind);
        let pressed = row_pressed(st, kind);

        // 行 hover / press / 编辑中底色 + 分隔线
        {
            let mut cv = r.canvas();
            if hovered || pressed || editing_row {
                let bg = if editing_row {
                    th.accent.with_alpha(0.07)
                } else if pressed {
                    th.pressed
                } else {
                    th.hover
                };
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
            if n < count - 1 {
                cv.fill_rect(
                    p(row.x as f32 + 16.0),
                    p((row.y + row.h) as f32),
                    p(row.w as f32 - 32.0),
                    p(1.0).max(1.0),
                    th.divider,
                );
            }
        }

        let (title, caption) = settings_row_text(tab, n, st);
        r.text(
            title,
            Rect::new(row.x + 18, row.y + 3, 250, 17),
            13.0,
            400,
            FACE_TEXT_IDX,
            th.text,
            Align::Left,
        );
        let caption_w = if matches!(kind, RowKind::Slider(_)) { 196 } else { 270 };
        r.text(
            &caption,
            Rect::new(row.x + 18, row.y + 21, caption_w, 14),
            10.0,
            400,
            FACE_TEXT_IDX,
            th.text3,
            Align::Left,
        );

        // 右侧控件
        match kind {
            RowKind::Action(action) => {
                if let Some(on) = switch_value(action, st) {
                    let mut cv = r.canvas();
                    draw_switch(
                        &mut cv,
                        th,
                        p((row.x + row.w - 50) as f32),
                        p((row.y + 10) as f32),
                        p(34.0),
                        p(20.0),
                        on,
                    );
                } else {
                    let value = action_value(action, st).unwrap_or("");
                    draw_value(r, th, row, value);
                }
            }
            RowKind::ThresholdInput => draw_threshold(r, st, th, row),
            RowKind::Slider(kind) => draw_slider(r, st, th, row, kind),
        }
    }

    if tab == SettingsTab::Appearance {
        draw_appearance_footer(r, st, th);
    }

    // 顶部：返回 + 标题 + 关闭
    r.text(
        "设置",
        Rect::new(100, 14, 200, 30),
        15.0,
        600,
        FACE_TEXT_IDX,
        th.text,
        Align::Center,
    );
    icon_button(r, st, th, layout::BTN_BACK, GLYPH_BACK, Action::CloseSettings, 12.0);
    icon_button(r, st, th, layout::BTN_CLOSE, GLYPH_CLOSE, Action::Close, 11.0);

    // 底部版本信息 / 返回提示（卡片下方 520..560 区域）
    let ver = format!("MemCleaner v{}", st.version);
    r.text(
        &ver,
        Rect::new(24, 529, 240, 16),
        10.0,
        400,
        FACE_TEXT_IDX,
        th.text3,
        Align::Left,
    );
    r.text(
        "Esc 返回",
        Rect::new(176, 529, 200, 16),
        10.0,
        400,
        FACE_TEXT_IDX,
        th.text3,
        Align::Right,
    );
}

/// 分页标签：胶囊分段控件（选中页用强调色渐变，未选中用轨道底色 + hover）。
fn draw_tabs(r: &mut Renderer, st: &UiState, th: &Theme) {
    let s = r.scale();
    let p = |v: f32| v * s;
    let tabs = [
        (SettingsTab::General, layout::TAB_GENERAL, "通用"),
        (SettingsTab::Appearance, layout::TAB_APPEARANCE, "外观"),
    ];
    // 轨道
    {
        let mut cv = r.canvas();
        let x = layout::TAB_GENERAL.x as f32;
        let y = layout::TAB_GENERAL.y as f32;
        let w = (layout::TAB_APPEARANCE.right() - layout::TAB_GENERAL.x) as f32;
        let h = layout::TAB_GENERAL.h as f32;
        cv.rounded_rect(p(x), p(y), p(w), p(h), p(17.0), Some(th.track), None, None);
    }
    for (tab, rect, label) in tabs {
        let selected = st.settings_tab == tab;
        let hovered = st.hover == Some(Action::SwitchTab(tab));
        {
            let mut cv = r.canvas();
            let (x, y, w, h) = (
                rect.x as f32 + 3.0,
                rect.y as f32 + 3.0,
                rect.w as f32 - 6.0,
                rect.h as f32 - 6.0,
            );
            if selected {
                cv.rounded_rect_vgrad(
                    p(x),
                    p(y),
                    p(w),
                    p(h),
                    p(14.0),
                    th.accent,
                    th.accent2,
                    Some((th.button_border, p(1.0).max(1.0))),
                    None,
                );
            } else if hovered {
                cv.rounded_rect(
                    p(x),
                    p(y),
                    p(w),
                    p(h),
                    p(14.0),
                    Some(th.hover),
                    None,
                    None,
                );
            }
        }
        let color = if selected { th.button_text } else { th.text2 };
        r.text(
            label,
            Rect::new(rect.x, rect.y, rect.w, rect.h),
            12.5,
            if selected { 600 } else { 400 },
            FACE_TEXT_IDX,
            color,
            Align::Center,
        );
    }
}

/// 右侧「值 + chevron」。
fn draw_value(r: &mut Renderer, th: &Theme, row: Rect, value: &str) {
    r.text(
        value,
        Rect::new(row.x + row.w - 240, row.y + 3, 210, 17),
        12.5,
        400,
        FACE_TEXT_IDX,
        th.text2,
        Align::Right,
    );
    r.text(
        GLYPH_CHEVRON,
        Rect::new(row.x + row.w - 27, row.y + 3, 15, 17),
        10.0,
        400,
        FACE_ICON_IDX,
        th.text3,
        Align::Center,
    );
}

/// 阈值行：非编辑态显示「85% + 编辑图标」，编辑态显示强调色描边输入框 + 闪烁光标。
fn draw_threshold(r: &mut Renderer, st: &UiState, th: &Theme, row: Rect) {
    let s = r.scale();
    let p = |v: f32| v * s;
    match st.threshold_editing.as_ref() {
        Some(edit) => {
            let r#box = Rect::new(row.x + 216, row.y + 6, 118, 27);
            {
                let mut cv = r.canvas();
                cv.rounded_rect(
                    p(r#box.x as f32),
                    p(r#box.y as f32),
                    p(r#box.w as f32),
                    p(r#box.h as f32),
                    p(8.0),
                    Some(th.dialog_bg),
                    Some((th.accent, p(1.6).max(1.0))),
                    None,
                );
            }
            let text = if edit.text.is_empty() { "" } else { edit.text.as_str() };
            let color = if edit.text.is_empty() { th.text3 } else { th.text };
            r.text(
                text,
                Rect::new(r#box.x + 11, r#box.y, r#box.w - 22, r#box.h),
                12.5,
                400,
                FACE_TEXT_IDX,
                color,
                Align::Left,
            );
            // 空输入时展示范围提示，避免用户不知道能填什么
            if edit.text.is_empty() {
                r.text(
                    "30–100",
                    Rect::new(r#box.x + 11, r#box.y, r#box.w - 22, r#box.h),
                    11.0,
                    400,
                    FACE_TEXT_IDX,
                    th.text3,
                    Align::Left,
                );
            }
            // 光标：anim_time 每 0.5s 翻转一次（≈2Hz 闪烁）
            let blink_on = ((st.anim_time * 2.0) as i32) & 1 == 0;
            if blink_on {
                let text_w = if edit.text.is_empty() {
                    0.0
                } else {
                    r.measure(&edit.text, 12.5, 400, FACE_TEXT_IDX) / s
                };
                let caret_x = r#box.x as f32 + 11.0 + text_w;
                let mut cv = r.canvas();
                cv.fill_rect(
                    p(caret_x),
                    p(r#box.y as f32 + 7.0),
                    p(1.5).max(1.0),
                    p(r#box.h as f32 - 14.0),
                    th.accent,
                );
            }
        }
        None => {
            let value = format!("{}%", st.settings.threshold);
            r.text(
                &value,
                Rect::new(row.x + row.w - 240, row.y + 3, 210, 17),
                12.5,
                400,
                FACE_TEXT_IDX,
                th.text2,
                Align::Right,
            );
            r.text(
                GLYPH_EDIT,
                Rect::new(row.x + row.w - 27, row.y + 3, 15, 17),
                10.0,
                400,
                FACE_ICON_IDX,
                th.text3,
                Align::Center,
            );
        }
    }
}

/// 透明度滑块行：轨道 + 已填充渐变 + 圆形滑块 + 右侧百分比。
fn draw_slider(r: &mut Renderer, st: &UiState, th: &Theme, row: Rect, kind: OpacityKind) {
    let s = r.scale();
    let p = |v: f32| v * s;
    let track = layout::slider_track(row);
    let frac = slider_value(kind, st).min(100) as f32 / 100.0;
    let knob_x = track.x as f32 + track.w as f32 * frac;
    let cy = track.y as f32 + track.h as f32 * 0.5;
    let hovered = matches!(st.hover, Some(Action::SetOpacity(k, _)) if k == kind);
    let pressed = matches!(st.pressed, Some(Action::SetOpacity(k, _)) if k == kind);
    {
        let mut cv = r.canvas();
        // 轨道
        cv.rounded_rect(
            p(track.x as f32),
            p(track.y as f32),
            p(track.w as f32),
            p(track.h as f32),
            p(4.0),
            Some(th.track),
            None,
            None,
        );
        // 已填充部分（强调色渐变）
        if frac > 0.005 {
            let fill_w = (track.w as f32 * frac).max(1.5);
            cv.rounded_rect_vgrad(
                p(track.x as f32),
                p(track.y as f32),
                p(fill_w),
                p(track.h as f32),
                p(4.0),
                th.accent,
                th.accent2,
                None,
                None,
            );
        }
        // hover / press 时滑块外发光
        if hovered || pressed {
            cv.radial_glow(
                p(knob_x),
                p(cy),
                p(16.0),
                th.accent.with_alpha(1.0),
                if pressed { 0.22 } else { 0.16 },
            );
        }
        let knob_r = if pressed { 7.6 } else if hovered { 7.2 } else { 6.2 };
        cv.circle(
            p(knob_x),
            p(cy),
            p(knob_r),
            Some(th.switch_knob),
            Some((th.accent.with_alpha(0.9), p(1.6).max(1.0))),
        );
    }
    let value = format!("{}%", slider_value(kind, st));
    r.text(
        &value,
        Rect::new(row.x + row.w - 150, row.y + 3, 122, 17),
        11.5,
        500,
        FACE_TEXT_IDX,
        th.text2,
        Align::Right,
    );
}

/// 外观分页底部：配色预览 + 操作提示（非交互区域）。
fn draw_appearance_footer(r: &mut Renderer, st: &UiState, th: &Theme) {
    let s = r.scale();
    let p = |v: f32| v * s;
    let area = Rect::new(40, 394, 320, 112);
    {
        let mut cv = r.canvas();
        cv.rounded_rect(
            p(area.x as f32),
            p(area.y as f32),
            p(area.w as f32),
            p(area.h as f32),
            p(14.0),
            Some(th.hover),
            Some((th.border, p(1.0).max(1.0))),
            None,
        );
    }
    r.text(
        "外观预览",
        Rect::new(area.x + 18, area.y + 10, 200, 14),
        10.5,
        500,
        FACE_TEXT_IDX,
        th.text3,
        Align::Left,
    );
    // 6 套配色圆点，当前方案加一圈描边
    let current = st.settings.accent;
    let cy = area.y + 36;
    for (i, scheme) in AccentScheme::ALL.iter().enumerate() {
        let cx = area.x + 40 + i as i32 * 48;
        let color = theme::palette(st.is_dark, *scheme).accent;
        let mut cv = r.canvas();
        if *scheme == current {
            cv.circle(
                p(cx as f32),
                p(cy as f32),
                p(9.0),
                None,
                Some((th.text, p(1.6).max(1.0))),
            );
        }
        cv.circle(p(cx as f32), p(cy as f32), p(6.0), Some(color), None);
    }
    r.text(
        "点击行内数值循环切换 · 拖动滑块调整透明度",
        Rect::new(area.x + 18, area.y + 50, area.w - 36, 14),
        9.5,
        400,
        FACE_TEXT_IDX,
        th.text3,
        Align::Left,
    );
    r.text(
        "自定义图片按窗口比例等比裁切填充",
        Rect::new(area.x + 18, area.y + 66, area.w - 36, 14),
        9.5,
        400,
        FACE_TEXT_IDX,
        th.text3,
        Align::Left,
    );
    let skin = format!("当前皮肤：{}", st.settings.skin_preset.display_name());
    r.text(
        &skin,
        Rect::new(area.x + 18, area.y + 82, area.w - 36, 14),
        9.5,
        400,
        FACE_TEXT_IDX,
        th.text2,
        Align::Left,
    );
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
        Align::Center,
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
            Align::Center,
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
        Align::Center,
    );
    r.text(
        &d.confirm_label,
        Rect::new(accept.x, accept.y, accept.w, accept.h),
        13.0,
        600,
        FACE_TEXT_IDX,
        th.button_text,
        Align::Center,
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
    // 主页面 toast 在顶部（不遮挡内容）；设置页顶部有分页标签，改为底部卡片下方。
    let y = if st.page == Page::Settings {
        522.0 - (1.0 - alpha) * 8.0
    } else {
        60.0 - (1.0 - alpha) * 10.0
    };
    // 面板不透明度已折进 toast_bg 的 alpha，这里只叠加出现/消失动画的 alpha。
    let bg = th.toast_bg.with_alpha(th.toast_bg.a * alpha);
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
        Align::Center,
    );
    r.text(
        &t.text,
        Rect::new(x as i32 + 36, y as i32, w as i32 - 50, 36),
        11.0,
        400,
        FACE_TEXT_IDX,
        th.text,
        Align::Left,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_tab(tab: SettingsTab) -> UiState {
        let mut st = UiState::empty();
        st.page = Page::Settings;
        st.settings_tab = tab;
        st
    }

    #[test]
    fn row_counts_match_contract() {
        assert_eq!(settings_row_count(SettingsTab::General), 10);
        assert_eq!(settings_row_count(SettingsTab::Appearance), 7);
    }

    #[test]
    fn general_row_kinds() {
        let g = SettingsTab::General;
        assert!(matches!(
            settings_row_kind(g, 0),
            RowKind::Action(Action::ToggleAutoClean)
        ));
        assert!(matches!(settings_row_kind(g, 1), RowKind::ThresholdInput));
        for (n, item) in [
            (2, CleanItem::WorkingSets),
            (3, CleanItem::Modified),
            (4, CleanItem::Standby),
            (5, CleanItem::LowPriority),
            (6, CleanItem::FileCache),
        ] {
            assert!(matches!(
                settings_row_kind(g, n),
                RowKind::Action(Action::ToggleCleanItem(i)) if i == item
            ));
        }
        assert!(matches!(
            settings_row_kind(g, 7),
            RowKind::Action(Action::CycleTheme)
        ));
        assert!(matches!(
            settings_row_kind(g, 8),
            RowKind::Action(Action::ToggleAutostart)
        ));
        assert!(matches!(
            settings_row_kind(g, 9),
            RowKind::Action(Action::ToggleTrayPercent)
        ));
    }

    #[test]
    fn appearance_row_kinds() {
        let a = SettingsTab::Appearance;
        let expected = [
            Action::CycleAccent,
            Action::CycleSkinPreset,
            Action::PickCustomSkin,
            Action::ClearCustomSkin,
        ];
        for (n, action) in expected.iter().enumerate() {
            assert_eq!(
                settings_row_kind(a, n as i32),
                RowKind::Action(*action),
                "row {n}"
            );
        }
        assert_eq!(
            settings_row_kind(a, 4),
            RowKind::Slider(OpacityKind::Skin)
        );
        assert_eq!(
            settings_row_kind(a, 5),
            RowKind::Slider(OpacityKind::Panel)
        );
        assert_eq!(
            settings_row_kind(a, 6),
            RowKind::Slider(OpacityKind::Window)
        );
    }

    #[test]
    fn row_texts_are_complete() {
        for tab in [SettingsTab::General, SettingsTab::Appearance] {
            let st = state_with_tab(tab);
            for n in 0..settings_row_count(tab) {
                let (title, caption) = settings_row_text(tab, n, &st);
                assert!(!title.is_empty(), "{tab:?} row {n} title");
                assert!(!caption.is_empty(), "{tab:?} row {n} caption");
            }
        }
    }

    #[test]
    fn row_texts_reflect_state() {
        let mut st = state_with_tab(SettingsTab::General);
        st.settings.threshold = 92;
        st.settings.theme = ThemeMode::Light;
        let (_, caption) = settings_row_text(SettingsTab::General, 1, &st);
        assert!(caption.contains("92"), "threshold caption: {caption}");
        let (_, caption) = settings_row_text(SettingsTab::General, 7, &st);
        assert!(caption.contains("浅色"), "theme caption: {caption}");

        // 编辑态：提示范围
        st.threshold_editing = Some(super::super::ThresholdEdit {
            text: "9".to_string(),
        });
        let (_, caption) = settings_row_text(SettingsTab::General, 1, &st);
        assert!(caption.contains("30–100"), "editing caption: {caption}");

        // 外观：皮肤文件名、百分比
        let mut st = state_with_tab(SettingsTab::Appearance);
        st.settings.skin_preset = SkinPreset::Custom;
        st.settings.skin_image_name = "aurora.png".to_string();
        st.settings.panel_opacity = 64;
        let (_, caption) = settings_row_text(SettingsTab::Appearance, 1, &st);
        assert!(caption.contains("aurora.png"), "skin caption: {caption}");
        let (_, caption) = settings_row_text(SettingsTab::Appearance, 5, &st);
        assert!(caption.contains("64"), "panel caption: {caption}");
    }

    #[test]
    fn row_values_reflect_snapshot() {
        let mut st = state_with_tab(SettingsTab::Appearance);
        st.settings.accent = AccentScheme::Violet;
        st.settings.skin_preset = SkinPreset::Nebula;
        assert_eq!(
            action_value(Action::CycleAccent, &st),
            Some(AccentScheme::Violet.display_name())
        );
        assert_eq!(
            action_value(Action::CycleSkinPreset, &st),
            Some(SkinPreset::Nebula.display_name())
        );
        assert_eq!(action_value(Action::PickCustomSkin, &st), Some("选择…"));

        st.settings.window_opacity = 73;
        assert_eq!(slider_value(OpacityKind::Window, &st), 73);
        st.settings.skin_opacity = 21;
        assert_eq!(slider_value(OpacityKind::Skin, &st), 21);
        st.settings.panel_opacity = 99;
        assert_eq!(slider_value(OpacityKind::Panel, &st), 99);
    }

    #[test]
    fn switch_rows_have_bool_state() {
        let mut st = state_with_tab(SettingsTab::General);
        st.settings.auto_enabled = true;
        assert_eq!(switch_value(Action::ToggleAutoClean, &st), Some(true));
        st.settings.clean_low_priority = true;
        assert_eq!(
            switch_value(Action::ToggleCleanItem(CleanItem::LowPriority), &st),
            Some(true)
        );
        st.settings.tray_show_percent = false;
        assert_eq!(switch_value(Action::ToggleTrayPercent, &st), Some(false));
        // 非开关行没有布尔状态
        assert_eq!(switch_value(Action::CycleTheme, &st), None);
    }

    #[test]
    fn hover_and_press_match_slider_kind() {
        let mut st = state_with_tab(SettingsTab::Appearance);
        st.hover = Some(Action::SetOpacity(OpacityKind::Skin, 40));
        assert!(row_hovered(&st, RowKind::Slider(OpacityKind::Skin)));
        assert!(!row_hovered(&st, RowKind::Slider(OpacityKind::Panel)));
        st.hover = Some(Action::EditThreshold);
        assert!(row_hovered(&st, RowKind::ThresholdInput));
        st.hover = Some(Action::CycleAccent);
        assert!(row_hovered(&st, RowKind::Action(Action::CycleAccent)));
        st.pressed = Some(Action::SetOpacity(OpacityKind::Window, 10));
        assert!(row_pressed(&st, RowKind::Slider(OpacityKind::Window)));
    }

    #[test]
    fn all_rows_fit_inside_settings_card() {
        for tab in [SettingsTab::General, SettingsTab::Appearance] {
            let count = settings_row_count(tab);
            let mut prev_bottom = layout::SETTINGS_CARD.y;
            for n in 0..count {
                let row = layout::settings_row(n);
                assert!(row.y >= layout::SETTINGS_CARD.y, "{tab:?} row {n} top");
                assert!(
                    row.bottom() <= layout::SETTINGS_CARD.bottom(),
                    "{tab:?} row {n} bottom"
                );
                assert!(row.y >= prev_bottom, "{tab:?} row {n} overlaps");
                prev_bottom = row.bottom();
            }
        }
        // 外观分页底部信息区不能压到最后一行
        let last = layout::settings_row(settings_row_count(SettingsTab::Appearance) - 1);
        assert!(last.bottom() < 394);
    }

    #[test]
    fn preset_backdrops_cover_all_visual_presets() {
        for dark in [true, false] {
            for preset in [
                SkinPreset::Aurora,
                SkinPreset::Nebula,
                SkinPreset::Dawn,
                SkinPreset::Dusk,
            ] {
                let b = preset_backdrop(preset, dark).expect("preset backdrop");
                assert!(b.glows.iter().all(|g| g.alpha > 0.0 && g.radius > 0.0));
                // 深色预设背景要足够暗、浅色背景要足够亮，保证文字对比度
                let lum = (b.top.r + b.top.g + b.top.b) / 3.0;
                if dark {
                    assert!(lum < 90.0, "{preset:?} dark top too bright: {lum}");
                } else {
                    assert!(lum > 200.0, "{preset:?} light top too dark: {lum}");
                }
            }
            assert!(preset_backdrop(SkinPreset::Default, dark).is_none());
            assert!(preset_backdrop(SkinPreset::Custom, dark).is_none());
        }
    }

    // -----------------------------------------------------------------
    // 离线视觉预览：直接渲染到 DIB -> BMP，不启动 GUI、不碰单实例互斥体。
    //
    // 正常 `cargo test` 时（未设置环境变量）直接返回，不产生任何文件。
    // 生成预览：
    //   $env:MEMCLEANER_UI_PREVIEW='G:\minecraft\testdata\dev-ui\preview'
    //   $env:MEMCLEANER_UI_SKIN='G:\minecraft\docs\screenshots\ui-final-dark.png'
    //   cargo test --release render_previews_offline -- --nocapture
    // -----------------------------------------------------------------

    use super::super::{ConfirmKind, SkinImage, ThresholdEdit};

    /// 32bpp 自上而下 BMP（System.Drawing 可直接转 PNG）。
    fn write_bmp(path: &std::path::Path, w: i32, h: i32, px: &[u32]) {
        use std::io::Write;
        let row_bytes = (w * 4) as u32;
        let data_size = row_bytes * h as u32;
        let mut head = Vec::with_capacity(54);
        head.extend_from_slice(b"BM");
        head.extend_from_slice(&(54 + data_size).to_le_bytes());
        head.extend_from_slice(&0u16.to_le_bytes());
        head.extend_from_slice(&0u16.to_le_bytes());
        head.extend_from_slice(&54u32.to_le_bytes());
        head.extend_from_slice(&40u32.to_le_bytes());
        head.extend_from_slice(&w.to_le_bytes());
        head.extend_from_slice(&(-h).to_le_bytes()); // top-down
        head.extend_from_slice(&1u16.to_le_bytes());
        head.extend_from_slice(&32u16.to_le_bytes());
        head.extend_from_slice(&0u32.to_le_bytes());
        head.extend_from_slice(&data_size.to_le_bytes());
        head.extend_from_slice(&2835i32.to_le_bytes());
        head.extend_from_slice(&2835i32.to_le_bytes());
        head.extend_from_slice(&0u32.to_le_bytes());
        head.extend_from_slice(&0u32.to_le_bytes());
        let mut body = Vec::with_capacity(px.len() * 4);
        for &p in px {
            body.push((p & 0xFF) as u8);
            body.push(((p >> 8) & 0xFF) as u8);
            body.push(((p >> 16) & 0xFF) as u8);
            body.push(0xFF);
        }
        if let Ok(mut f) = std::fs::File::create(path) {
            let _ = f.write_all(&head);
            let _ = f.write_all(&body);
        }
    }

    fn preview_render(
        dir: &std::path::Path,
        name: &str,
        st: &UiState,
        skin: Option<std::sync::Arc<SkinImage>>,
    ) {
        let mut r = match Renderer::new(120) {
            Some(r) => r,
            None => panic!("Renderer::new failed (GDI unavailable?)"),
        };
        r.recreate(120);
        r.set_skin(skin);
        r.render(st);
        let (w, h, px) = {
            let mut cv = r.canvas();
            let (w, h) = (cv.w, cv.h);
            let mut out = Vec::with_capacity((w * h) as usize);
            unsafe {
                for i in 0..(w * h) as usize {
                    out.push(*cv.px.add(i));
                }
            }
            (w, h, out)
        };
        write_bmp(&dir.join(format!("{name}.bmp")), w, h, &px);
    }

    fn preview_state(page: Page, dark: bool) -> UiState {
        let mut st = UiState::empty();
        st.dpi = 120;
        st.page = page;
        st.is_dark = dark;
        st.display_percent = 59.0;
        st.display_used = 9.9 * 1024.0 * 1024.0 * 1024.0;
        st.display_avail = 6.3 * 1024.0 * 1024.0 * 1024.0;
        st.last_freed_bytes = (2.3 * 1024.0 * 1024.0 * 1024.0) as u64;
        st.last_clean_unix = 1_700_000_000;
        st
    }

    #[test]
    fn render_previews_offline() {
        let Ok(dir) = std::env::var("MEMCLEANER_UI_PREVIEW") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();

        let skin = std::env::var("MEMCLEANER_UI_SKIN")
            .ok()
            .and_then(|p| crate::platform::image::load_skin_image(&p))
            .map(std::sync::Arc::new);

        // 主页面 / 设置页（深/浅）
        preview_render(&dir, "main-dark", &preview_state(Page::Main, true), None);
        preview_render(&dir, "main-light", &preview_state(Page::Main, false), None);
        let mut st = preview_state(Page::Settings, true);
        st.settings_tab = SettingsTab::General;
        preview_render(&dir, "settings-general-dark", &st, None);
        st.settings_tab = SettingsTab::Appearance;
        preview_render(&dir, "settings-appearance-dark", &st, None);
        let mut st = preview_state(Page::Settings, false);
        st.settings_tab = SettingsTab::Appearance;
        preview_render(&dir, "settings-appearance-light", &st, None);

        // 阈值编辑态
        let mut st = preview_state(Page::Settings, true);
        st.settings_tab = SettingsTab::General;
        st.threshold_editing = Some(ThresholdEdit {
            text: "90".to_string(),
        });
        st.anim_time = 0.0; // 光标亮
        preview_render(&dir, "threshold-edit", &st, None);

        // 确认框
        let mut st = preview_state(Page::Main, true);
        st.confirm = Some(ConfirmDialog {
            title: "需要管理员权限".to_string(),
            message: "清理系统内存需要管理员权限。\n是否以管理员身份重新启动并清理？"
                .to_string(),
            confirm_label: "以管理员身份清理".to_string(),
            cancel_label: "取消".to_string(),
            kind: ConfirmKind::ElevateClean,
        });
        preview_render(&dir, "confirm", &st, None);

        // 自定义皮肤（真实图片走 GDI+ 解码，与 app 运行时同一条路径）
        if let Some(skin) = skin {
            let mut st = preview_state(Page::Main, true);
            st.settings.skin_preset = SkinPreset::Custom;
            st.settings.skin_image_name = "ui-final-dark.png".to_string();
            st.settings.skin_opacity = 72;
            st.settings.panel_opacity = 90;
            preview_render(&dir, "custom-skin", &st, Some(skin.clone()));
            st.page = Page::Settings;
            st.settings_tab = SettingsTab::Appearance;
            preview_render(&dir, "custom-skin-settings", &st, Some(skin.clone()));

            // 低不透明度版本（对比幽灵文字强度，用于挑最终截图配置）
            let mut st = preview_state(Page::Main, true);
            st.settings.skin_preset = SkinPreset::Custom;
            st.settings.skin_image_name = "ui-final-dark.png".to_string();
            st.settings.skin_opacity = 50;
            st.settings.panel_opacity = 100;
            preview_render(&dir, "custom-skin-50", &st, Some(skin));
        }

        // hover / press 反馈
        let mut st = preview_state(Page::Settings, true);
        st.settings_tab = SettingsTab::General;
        st.hover = Some(Action::ToggleCleanItem(CleanItem::WorkingSets));
        st.pressed = Some(Action::CycleTheme);
        preview_render(&dir, "settings-general-hover", &st, None);
        let mut st = preview_state(Page::Settings, true);
        st.settings_tab = SettingsTab::Appearance;
        st.hover = Some(Action::SetOpacity(OpacityKind::Panel, 80));
        preview_render(&dir, "settings-appearance-hover", &st, None);
        let mut st = preview_state(Page::Settings, true);
        st.settings_tab = SettingsTab::General;
        st.hover = Some(Action::SwitchTab(SettingsTab::Appearance));
        preview_render(&dir, "settings-tab-hover", &st, None);

        // 4 套程序化皮肤（深/浅）
        for (preset, tag) in [
            (SkinPreset::Aurora, "aurora"),
            (SkinPreset::Nebula, "nebula"),
            (SkinPreset::Dawn, "dawn"),
            (SkinPreset::Dusk, "dusk"),
        ] {
            let mut st = preview_state(Page::Main, true);
            st.settings.skin_preset = preset;
            preview_render(&dir, &format!("skin-{tag}-dark"), &st, None);
            let mut st = preview_state(Page::Settings, false);
            st.settings.skin_preset = preset;
            st.settings_tab = SettingsTab::Appearance;
            preview_render(&dir, &format!("skin-{tag}-light"), &st, None);
        }

        // 6 套强调色（深色主页面）
        for scheme in AccentScheme::ALL {
            let mut st = preview_state(Page::Main, true);
            st.settings.accent = scheme;
            preview_render(&dir, &format!("accent-{:?}", scheme).to_lowercase(), &st, None);
        }

        // 面板不透明度 40% 的效果（浅色外观页）
        let mut st = preview_state(Page::Settings, false);
        st.settings_tab = SettingsTab::Appearance;
        st.settings.panel_opacity = 40;
        preview_render(&dir, "panel-opacity-40-light", &st, None);

        // 帧耗时采样（含 GDI 文字，仅预览模式跑）：用于性能红线自查。
        let mut bench = match Renderer::new(120) {
            Some(r) => r,
            None => return,
        };
        bench.recreate(120);
        let mut frame_ms = |st: &UiState| {
            bench.render(st);
            let t0 = std::time::Instant::now();
            for _ in 0..120 {
                bench.render(st);
            }
            t0.elapsed().as_secs_f64() * 1000.0 / 120.0
        };
        let mut st = preview_state(Page::Main, true);
        println!("[perf] frame main-dark = {:.2} ms", frame_ms(&st));
        st.page = Page::Settings;
        st.settings_tab = SettingsTab::Appearance;
        println!("[perf] frame settings-appearance-dark = {:.2} ms", frame_ms(&st));
        let mut st = preview_state(Page::Main, true);
        st.settings.skin_preset = SkinPreset::Aurora;
        println!("[perf] frame main-aurora = {:.2} ms", frame_ms(&st));

        println!("previews written to {}", dir.display());
    }
}
