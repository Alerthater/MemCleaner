//! 配色：6 套强调色方案 × 暗色/浅色两套 + 面板不透明度。
//!
//! 「仿 Mica」思路：DWM Mica/亚克力依赖系统透明效果设置与本地会话能力，
//! 且软件光栅化 BitBlt 的像素全不透明、系统 backdrop 无法透出；因此这里用
//! 「渐变背景 + 柔和径向光晕」程序化模拟材质层次，跨 DPI/跨会话表现一致、零额外开销。
//!
//! `panel_opacity`（0–100）只作用于「卡片/面板」类半透明元素（card/border/hover/
//! pressed/divider/track/toast_bg），文字、按钮、对话框保持不透明以保证可读性。
#![allow(dead_code)]

use super::raster::Color;
use crate::config::AccentScheme;

#[derive(Clone, Copy)]
pub struct Theme {
    pub dark: bool,
    // 背景
    pub bg_top: Color,
    pub bg_bottom: Color,
    pub glow_a: Color,
    pub glow_b: Color,
    // 卡片 / 弹层
    pub card: Color,
    pub border: Color,
    pub dialog_bg: Color,
    pub overlay: Color,
    pub shadow: Color,
    // 文字
    pub text: Color,
    pub text2: Color,
    pub text3: Color,
    // 强调色
    pub accent: Color,
    pub accent2: Color,
    pub track: Color,
    // 状态色
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    // 交互反馈
    pub hover: Color,
    pub pressed: Color,
    pub close_hover: Color,
    // 其它
    pub button_text: Color,
    pub button_border: Color,
    pub divider: Color,
    pub switch_knob: Color,
    pub toast_bg: Color,
}

/// 一套强调色方案在指定主题下的取色（design 的配色预览也用它）。
#[derive(Clone, Copy, Debug)]
pub struct AccentPalette {
    /// 强调色（渐变起点 / 主色）。
    pub accent: Color,
    /// 强调色副色（渐变终点）。
    pub accent2: Color,
    /// 背景光晕主色。
    pub glow_a: Color,
    /// 背景光晕副色。
    pub glow_b: Color,
    /// 压在强调色块上的文字色（保证对比度）。
    pub on_accent: Color,
}

const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);

/// 取某套强调色方案在指定主题下的配色。
pub fn palette(dark: bool, scheme: AccentScheme) -> AccentPalette {
    match (scheme, dark) {
        // ---- 极光蓝（默认，保持 v1.0 取值）----
        (AccentScheme::Blue, true) => AccentPalette {
            accent: Color::rgb(0x4C, 0xC2, 0xFF),
            accent2: Color::rgb(0x7B, 0x8C, 0xFF),
            glow_a: Color::rgb(0x4C, 0xC2, 0xFF),
            glow_b: Color::rgb(0x7B, 0x8C, 0xFF),
            on_accent: WHITE,
        },
        (AccentScheme::Blue, false) => AccentPalette {
            accent: Color::rgb(0x00, 0x78, 0xD4),
            accent2: Color::rgb(0x4C, 0xC2, 0xFF),
            glow_a: Color::rgb(0x4C, 0xC2, 0xFF),
            glow_b: Color::rgb(0x7B, 0x8C, 0xFF),
            on_accent: WHITE,
        },
        // ---- 薄荷绿 ----
        (AccentScheme::Mint, true) => AccentPalette {
            accent: Color::rgb(0x47, 0xE0, 0xB0),
            accent2: Color::rgb(0x4C, 0xC2, 0xFF),
            glow_a: Color::rgb(0x47, 0xE0, 0xB0),
            glow_b: Color::rgb(0x3B, 0xC8, 0xFF),
            on_accent: Color::rgb(0x06, 0x23, 0x1B),
        },
        (AccentScheme::Mint, false) => AccentPalette {
            accent: Color::rgb(0x0B, 0x8F, 0x63),
            accent2: Color::rgb(0x34, 0xC7, 0x9A),
            glow_a: Color::rgb(0x4C, 0xE0, 0xB0),
            glow_b: Color::rgb(0x4C, 0xC2, 0xFF),
            on_accent: WHITE,
        },
        // ---- 紫罗兰 ----
        (AccentScheme::Violet, true) => AccentPalette {
            accent: Color::rgb(0x9B, 0x8C, 0xFF),
            accent2: Color::rgb(0xC8, 0x7B, 0xFF),
            glow_a: Color::rgb(0x9B, 0x8C, 0xFF),
            glow_b: Color::rgb(0xC8, 0x7B, 0xFF),
            on_accent: WHITE,
        },
        (AccentScheme::Violet, false) => AccentPalette {
            accent: Color::rgb(0x6A, 0x4F, 0xE0),
            accent2: Color::rgb(0x9B, 0x8C, 0xFF),
            glow_a: Color::rgb(0x9B, 0x8C, 0xFF),
            glow_b: Color::rgb(0xC8, 0x7B, 0xFF),
            on_accent: WHITE,
        },
        // ---- 琥珀橙 ----
        (AccentScheme::Amber, true) => AccentPalette {
            accent: Color::rgb(0xFF, 0xB4, 0x54),
            accent2: Color::rgb(0xFF, 0x8A, 0x6B),
            glow_a: Color::rgb(0xFF, 0xC0, 0x6A),
            glow_b: Color::rgb(0xFF, 0x9A, 0x5C),
            on_accent: Color::rgb(0x2B, 0x1A, 0x00),
        },
        (AccentScheme::Amber, false) => AccentPalette {
            accent: Color::rgb(0xB8, 0x5F, 0x00),
            accent2: Color::rgb(0xFF, 0xA9, 0x4D),
            glow_a: Color::rgb(0xFF, 0xC0, 0x6A),
            glow_b: Color::rgb(0xFF, 0x9A, 0x5C),
            on_accent: WHITE,
        },
        // ---- 樱粉 ----
        (AccentScheme::Rose, true) => AccentPalette {
            accent: Color::rgb(0xFF, 0x7B, 0xA9),
            accent2: Color::rgb(0xFF, 0x9B, 0x7A),
            glow_a: Color::rgb(0xFF, 0x8F, 0xB4),
            glow_b: Color::rgb(0xFF, 0x9B, 0x7A),
            on_accent: Color::rgb(0x2C, 0x07, 0x14),
        },
        (AccentScheme::Rose, false) => AccentPalette {
            accent: Color::rgb(0xC6, 0x32, 0x6E),
            accent2: Color::rgb(0xFF, 0x7B, 0xA9),
            glow_a: Color::rgb(0xFF, 0x8F, 0xB4),
            glow_b: Color::rgb(0xFF, 0x9B, 0x7A),
            on_accent: WHITE,
        },
        // ---- 石墨灰 ----
        (AccentScheme::Graphite, true) => AccentPalette {
            accent: Color::rgb(0xA9, 0xB4, 0xC4),
            accent2: Color::rgb(0x8A, 0x94, 0xA6),
            glow_a: Color::rgb(0xA9, 0xB4, 0xC4),
            glow_b: Color::rgb(0x8A, 0x94, 0xA6),
            on_accent: Color::rgb(0x14, 0x16, 0x1A),
        },
        (AccentScheme::Graphite, false) => AccentPalette {
            accent: Color::rgb(0x4A, 0x51, 0x60),
            accent2: Color::rgb(0x6B, 0x74, 0x85),
            glow_a: Color::rgb(0x8A, 0x94, 0xA6),
            glow_b: Color::rgb(0x6B, 0x74, 0x85),
            on_accent: WHITE,
        },
    }
}

/// 面板不透明度系数（0–100 -> 0.0–1.0）。
#[inline]
fn panel_k(panel_opacity: u8) -> f32 {
    panel_opacity.min(100) as f32 / 100.0
}

/// 构建一套主题。
pub fn build(dark: bool, accent: AccentScheme, panel_opacity: u8) -> Theme {
    let k = panel_k(panel_opacity);
    let pal = palette(dark, accent);
    if dark {
        Theme {
            dark: true,
            bg_top: Color::rgb(0x17, 0x18, 0x1B),
            bg_bottom: Color::rgb(0x1E, 0x20, 0x24),
            glow_a: pal.glow_a,
            glow_b: pal.glow_b,
            card: Color::rgba(0xFF, 0xFF, 0xFF, 0.06 * k),
            border: Color::rgba(0xFF, 0xFF, 0xFF, 0.10 * k),
            dialog_bg: Color::rgb(0x24, 0x26, 0x2B),
            overlay: Color::rgba(0, 0, 0, 0.55),
            shadow: Color::rgba(0, 0, 0, 0.35),
            text: Color::rgb(0xF2, 0xF2, 0xF5),
            text2: Color::rgb(0xA0, 0xA1, 0xA8),
            text3: Color::rgb(0x7A, 0x7C, 0x85),
            accent: pal.accent,
            accent2: pal.accent2,
            track: Color::rgba(0xFF, 0xFF, 0xFF, 0.08 * k),
            success: Color::rgb(0x4A, 0xDE, 0x80),
            warning: Color::rgb(0xFF, 0xC2, 0x4D),
            danger: Color::rgb(0xFF, 0x6B, 0x6B),
            hover: Color::rgba(0xFF, 0xFF, 0xFF, 0.08 * k),
            pressed: Color::rgba(0xFF, 0xFF, 0xFF, 0.04 * k),
            close_hover: Color::rgb(0xC4, 0x2B, 0x1C),
            button_text: pal.on_accent,
            button_border: Color::rgba(0xFF, 0xFF, 0xFF, 0.20),
            divider: Color::rgba(0xFF, 0xFF, 0xFF, 0.06 * k),
            switch_knob: WHITE,
            toast_bg: Color::rgba(0x2A, 0x2C, 0x32, 0.97 * k),
        }
    } else {
        Theme {
            dark: false,
            bg_top: Color::rgb(0xF7, 0xF7, 0xF9),
            bg_bottom: Color::rgb(0xFF, 0xFF, 0xFF),
            glow_a: pal.glow_a,
            glow_b: pal.glow_b,
            card: Color::rgba(0xFF, 0xFF, 0xFF, k),
            border: Color::rgba(0, 0, 0, (18.0 / 255.0) * k),
            dialog_bg: Color::rgb(0xFF, 0xFF, 0xFF),
            overlay: Color::rgba(0, 0, 0, 0.32),
            shadow: Color::rgba(15, 18, 25, 0.10),
            text: Color::rgb(0x17, 0x18, 0x1B),
            text2: Color::rgb(0x6B, 0x6D, 0x75),
            text3: Color::rgb(0x8A, 0x8C, 0x95),
            accent: pal.accent,
            accent2: pal.accent2,
            track: Color::rgba(0, 0, 0, 0.08 * k),
            success: Color::rgb(0x16, 0xA3, 0x4A),
            warning: Color::rgb(0xB9, 0x7A, 0x00),
            danger: Color::rgb(0xD6, 0x45, 0x45),
            hover: Color::rgba(0, 0, 0, 0.05 * k),
            pressed: Color::rgba(0, 0, 0, 0.03 * k),
            close_hover: Color::rgb(0xC4, 0x2B, 0x1C),
            button_text: pal.on_accent,
            button_border: Color::rgba(0xFF, 0xFF, 0xFF, 0.32),
            divider: Color::rgba(0, 0, 0, 0.06 * k),
            switch_knob: WHITE,
            toast_bg: Color::rgba(0xFF, 0xFF, 0xFF, 0.98 * k),
        }
    }
}

/// 暗色 + 默认强调色的便捷包装。
pub fn dark() -> Theme {
    build(true, AccentScheme::Blue, 100)
}

/// 浅色 + 默认强调色的便捷包装。
pub fn light() -> Theme {
    build(false, AccentScheme::Blue, 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blue_default_matches_legacy_values() {
        let d = build(true, AccentScheme::Blue, 100);
        assert!(d.dark);
        assert_eq!(d.bg_top.packed(), 0x17_18_1B);
        assert_eq!(d.accent.packed(), 0x4C_C2_FF);
        assert_eq!(d.accent2.packed(), 0x7B_8C_FF);
        assert_eq!(d.button_text.packed(), 0xFF_FF_FF);
        // dark()/light() 包装等价
        assert_eq!(dark().accent.packed(), d.accent.packed());

        let l = build(false, AccentScheme::Blue, 100);
        assert!(!l.dark);
        assert_eq!(l.bg_top.packed(), 0xF7_F7_F9);
        assert_eq!(l.accent.packed(), 0x00_78_D4);
        assert_eq!(l.card.packed(), 0xFF_FF_FF);
        assert_eq!(l.card.a, 1.0);
        assert_eq!(light().card.a, l.card.a);
    }

    #[test]
    fn panel_opacity_scales_only_panels() {
        let full = build(true, AccentScheme::Blue, 100);
        let half = build(true, AccentScheme::Blue, 50);
        for (a, b) in [
            (full.card.a, half.card.a),
            (full.border.a, half.border.a),
            (full.hover.a, half.hover.a),
            (full.pressed.a, half.pressed.a),
            (full.divider.a, half.divider.a),
            (full.track.a, half.track.a),
            (full.toast_bg.a, half.toast_bg.a),
        ] {
            assert!((b - a * 0.5).abs() < 1e-6, "{a} -> {b}");
        }
        // 文字 / 按钮 / 对话框不受影响
        assert_eq!(half.text.a, full.text.a);
        assert_eq!(half.accent.a, full.accent.a);
        assert_eq!(half.button_text.a, full.button_text.a);
        assert_eq!(half.dialog_bg.a, full.dialog_bg.a);
        // 0% 时面板完全透明但不 panic
        let zero = build(false, AccentScheme::Rose, 0);
        assert_eq!(zero.card.a, 0.0);
        assert_eq!(zero.text.a, 1.0);
    }

    #[test]
    fn accent_schemes_are_distinct() {
        for dark_mode in [true, false] {
            let mut seen: Vec<(u32, u32)> = Vec::new();
            for scheme in AccentScheme::ALL {
                let th = build(dark_mode, scheme, 100);
                assert_eq!(th.accent.a, 1.0);
                let key = (th.accent.packed(), th.accent2.packed());
                assert!(!seen.contains(&key), "{scheme:?} dark={dark_mode}");
                seen.push(key);
            }
            assert_eq!(seen.len(), AccentScheme::ALL.len());
        }
    }

    #[test]
    fn on_accent_has_contrast() {
        let lum = |c: Color| (c.r + c.g + c.b) / 3.0;
        for dark_mode in [true, false] {
            for scheme in AccentScheme::ALL {
                let p = palette(dark_mode, scheme);
                assert!(
                    (lum(p.on_accent) - lum(p.accent)).abs() > 70.0,
                    "{scheme:?} dark={dark_mode}"
                );
                assert!((lum(p.on_accent) - lum(p.accent2)).abs() > 50.0);
                assert_eq!(p.on_accent.a, 1.0);
            }
        }
    }
}
