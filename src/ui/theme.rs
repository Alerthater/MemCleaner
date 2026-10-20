//! 配色：暗色（默认）+ 浅色两套，取值与设计稿一致。
//!
//! 「仿 Mica」思路：DWM Mica/亚克力依赖系统透明效果设置与本地会话能力，
//! 且软件光栅化 BitBlt 的像素全不透明、系统 backdrop 无法透出；因此这里用
//! 「渐变背景 + 柔和径向光晕」程序化模拟材质层次，跨 DPI/跨会话表现一致、零额外开销。
#![allow(dead_code)]

use super::raster::Color;

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

pub fn dark() -> Theme {
    Theme {
        dark: true,
        bg_top: Color::rgb(0x17, 0x18, 0x1B),
        bg_bottom: Color::rgb(0x1E, 0x20, 0x24),
        glow_a: Color::rgb(0x4C, 0xC2, 0xFF),
        glow_b: Color::rgb(0x7B, 0x8C, 0xFF),
        card: Color::rgba(0xFF, 0xFF, 0xFF, 0.06),
        border: Color::rgba(0xFF, 0xFF, 0xFF, 0.10),
        dialog_bg: Color::rgb(0x24, 0x26, 0x2B),
        overlay: Color::rgba(0, 0, 0, 0.55),
        shadow: Color::rgba(0, 0, 0, 0.35),
        text: Color::rgb(0xF2, 0xF2, 0xF5),
        text2: Color::rgb(0xA0, 0xA1, 0xA8),
        text3: Color::rgb(0x7A, 0x7C, 0x85),
        accent: Color::rgb(0x4C, 0xC2, 0xFF),
        accent2: Color::rgb(0x7B, 0x8C, 0xFF),
        track: Color::rgba(0xFF, 0xFF, 0xFF, 0.08),
        success: Color::rgb(0x4A, 0xDE, 0x80),
        warning: Color::rgb(0xFF, 0xC2, 0x4D),
        danger: Color::rgb(0xFF, 0x6B, 0x6B),
        hover: Color::rgba(0xFF, 0xFF, 0xFF, 0.08),
        pressed: Color::rgba(0xFF, 0xFF, 0xFF, 0.04),
        close_hover: Color::rgb(0xC4, 0x2B, 0x1C),
        button_text: Color::rgb(0xFF, 0xFF, 0xFF),
        button_border: Color::rgba(0xFF, 0xFF, 0xFF, 0.20),
        divider: Color::rgba(0xFF, 0xFF, 0xFF, 0.06),
        switch_knob: Color::rgb(0xFF, 0xFF, 0xFF),
        toast_bg: Color::rgba(0x2A, 0x2C, 0x32, 0.97),
    }
}

pub fn light() -> Theme {
    Theme {
        dark: false,
        bg_top: Color::rgb(0xF7, 0xF7, 0xF9),
        bg_bottom: Color::rgb(0xFF, 0xFF, 0xFF),
        glow_a: Color::rgb(0x4C, 0xC2, 0xFF),
        glow_b: Color::rgb(0x7B, 0x8C, 0xFF),
        card: Color::rgb(0xFF, 0xFF, 0xFF),
        border: Color::rgba(0, 0, 0, 18.0 / 255.0),
        dialog_bg: Color::rgb(0xFF, 0xFF, 0xFF),
        overlay: Color::rgba(0, 0, 0, 0.32),
        shadow: Color::rgba(15, 18, 25, 0.10),
        text: Color::rgb(0x17, 0x18, 0x1B),
        text2: Color::rgb(0x6B, 0x6D, 0x75),
        text3: Color::rgb(0x8A, 0x8C, 0x95),
        accent: Color::rgb(0x00, 0x78, 0xD4),
        accent2: Color::rgb(0x4C, 0xC2, 0xFF),
        track: Color::rgba(0, 0, 0, 0.08),
        success: Color::rgb(0x16, 0xA3, 0x4A),
        warning: Color::rgb(0xB9, 0x7A, 0x00),
        danger: Color::rgb(0xD6, 0x45, 0x45),
        hover: Color::rgba(0, 0, 0, 0.05),
        pressed: Color::rgba(0, 0, 0, 0.03),
        close_hover: Color::rgb(0xC4, 0x2B, 0x1C),
        button_text: Color::rgb(0xFF, 0xFF, 0xFF),
        button_border: Color::rgba(0xFF, 0xFF, 0xFF, 0.32),
        divider: Color::rgba(0, 0, 0, 0.06),
        switch_knob: Color::rgb(0xFF, 0xFF, 0xFF),
        toast_bg: Color::rgba(0xFF, 0xFF, 0xFF, 0.98),
    }
}
