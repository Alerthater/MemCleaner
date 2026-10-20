//! 软件光栅化核心：在 32bpp DIB 像素缓冲上用有符号距离场(SDF)做解析抗锯齿。
//!
//! 性能要点（16ms 帧预算）：
//! - 颜色混合用 8bit 定点整数运算，避免逐像素 f32↔u8 转换；
//! - 背景渐变直接覆写像素（最底层无需混合）；
//! - 圆角矩形内部区域跳过 sqrt；
//! - 圆弧角度用多项式 `fast_atan2` 近似（UI 渐变精度足够）；
//! - 径向光晕用 r² 四次衰减，省掉 sqrt。
//!
//! 坐标均为设备像素（f32）；颜色为直通 alpha 的 RGBA（0..255 / 0..1）。
#![allow(dead_code)]

use std::f32::consts::{PI, TAU};

/// 直通 alpha 颜色。
#[derive(Clone, Copy, Debug)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r: r as f32, g: g as f32, b: b as f32, a: 1.0 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Self {
        Self { r: r as f32, g: g as f32, b: b as f32, a }
    }

    pub fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    /// 线性插值（直通空间，足够用于 UI 渐变）。
    pub fn lerp(self, o: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            r: self.r + (o.r - self.r) * t,
            g: self.g + (o.g - self.g) * t,
            b: self.b + (o.b - self.b) * t,
            a: self.a + (o.a - self.a) * t,
        }
    }

    /// 打包为 0xRRGGBB（忽略 alpha）。
    #[inline]
    pub fn packed(self) -> u32 {
        ((self.r.clamp(0.0, 255.0) as u32) << 16)
            | ((self.g.clamp(0.0, 255.0) as u32) << 8)
            | (self.b.clamp(0.0, 255.0) as u32)
    }
}

/// 整数 RGB 颜色（0xRRGGBB）。
pub type Rgb = u32;

/// 两个 0xRRGGBB 颜色线性插值，结果为 0xRRGGBB。
pub fn lerp_rgb(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let mix = |sa: u32, sb: u32| -> u32 {
        let va = (sa & 0xFF) as f32;
        let vb = (sb & 0xFF) as f32;
        (va + (vb - va) * t) as u32
    };
    (mix(a >> 16, b >> 16) << 16) | (mix(a >> 8, b >> 8) << 8) | mix(a, b)
}

#[inline]
fn packed_rgb(p: u32) -> (u8, u8, u8) {
    (((p >> 16) & 0xFF) as u8, ((p >> 8) & 0xFF) as u8, (p & 0xFF) as u8)
}

/// 32 位 RGBA 像素混合（cov: 0..255）。
#[inline]
fn mix(dst: u32, r: u8, g: u8, b: u8, cov: u32) -> u32 {
    if cov >= 255 {
        return 0xFF00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
    }
    let inv = 255 - cov;
    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;
    let nr = (r as u32 * cov + dr * inv) >> 8;
    let ng = (g as u32 * cov + dg * inv) >> 8;
    let nb = (b as u32 * cov + db * inv) >> 8;
    0xFF00_0000 | (nr << 16) | (ng << 8) | nb
}

/// 快速 atan2 近似（误差 < 1e-4 rad），返回 [-PI, PI]。
#[inline]
fn fast_atan2(y: f32, x: f32) -> f32 {
    let ax = x.abs();
    let ay = y.abs();
    let mx = ax.max(ay);
    if mx < 1e-9 {
        return 0.0;
    }
    let a = ax.min(ay) / mx; // 0..1
    let s = a * a;
    let r = (((-0.046_496_475 * s + 0.159_314_22) * s - 0.327_622_76) * s) * a + a;
    let mut angle = r;
    if ay > ax {
        angle = PI * 0.5 - angle;
    }
    if x < 0.0 {
        angle = PI - angle;
    }
    if y < 0.0 {
        angle = -angle;
    }
    angle
}

/// 圆角矩形 SDF（中心点 + 半宽半高 + 圆角半径）。
#[inline]
fn sd_round_rect(px: f32, py: f32, cx: f32, cy: f32, hw: f32, hh: f32, r: f32) -> f32 {
    let r = r.min(hw).min(hh).max(0.0);
    let qx = (px - cx).abs() - (hw - r);
    let qy = (py - cy).abs() - (hh - r);
    if qx <= 0.0 && qy <= 0.0 {
        return qx.max(qy) - r;
    }
    let ax = qx.max(0.0);
    let ay = qy.max(0.0);
    (ax * ax + ay * ay).sqrt() + qx.max(qy).min(0.0) - r
}

/// 逻辑矩形（布局与命中测试共用）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    #[inline]
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    #[inline]
    pub fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    #[inline]
    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    #[inline]
    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
}

/// DIB 画布。px 指向 CreateDIBSection 的自上而下 32bpp 缓冲区。
pub struct Canvas {
    pub w: i32,
    pub h: i32,
    pub px: *mut u32,
}

impl Canvas {
    #[inline]
    pub fn new(w: i32, h: i32, px: *mut u32) -> Self {
        Self { w, h, px }
    }

    #[inline]
    fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.w && y < self.h
    }

    #[inline]
    fn put(&mut self, x: i32, y: i32, c: u32, cov: u32) {
        if cov == 0 || !self.in_bounds(x, y) {
            return;
        }
        unsafe {
            let p = self.px.add((y * self.w + x) as usize);
            let (r, g, b) = packed_rgb(c);
            *p = mix(*p, r, g, b, cov);
        }
    }

    /// 加法混合（用于外发光/泛光）。
    #[inline]
    pub fn put_add(&mut self, x: i32, y: i32, c: u32, amount: u32) {
        if amount == 0 || !self.in_bounds(x, y) {
            return;
        }
        unsafe {
            let p = self.px.add((y * self.w + x) as usize);
            let d = *p;
            let (r, g, b) = packed_rgb(c);
            let nr = ((d >> 16) & 0xFF) + (r as u32 * amount >> 8);
            let ng = ((d >> 8) & 0xFF) + (g as u32 * amount >> 8);
            let nb = (d & 0xFF) + (b as u32 * amount >> 8);
            *p = 0xFF00_0000 | (nr.min(255) << 16) | (ng.min(255) << 8) | nb.min(255);
        }
    }

    /// 纯色矩形（f32 边界，边缘按覆盖面积抗锯齿）。
    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: Color) {
        let pk = c.packed();
        let alpha = (c.a * 255.0).clamp(0.0, 255.0) as u32;
        if alpha == 0 {
            return;
        }
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let x1 = (x + w).ceil() as i32;
        let y1 = (y + h).ceil() as i32;
        for py in y0..y1 {
            let cov_y = ((py as f32 + 1.0).min(y + h) - (py as f32).max(y)).clamp(0.0, 1.0);
            if cov_y <= 0.0 {
                continue;
            }
            for px in x0..x1 {
                let cov_x = ((px as f32 + 1.0).min(x + w) - (px as f32).max(x)).clamp(0.0, 1.0);
                if cov_x > 0.0 {
                    self.put(px, py, pk, (cov_x * cov_y * alpha as f32) as u32);
                }
            }
        }
    }

    /// 竖向线性渐变矩形（与下方内容混合；用于按钮等）。
    pub fn fill_rect_vgrad(&mut self, x: f32, y: f32, w: f32, h: f32, top: Color, bottom: Color) {
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let x1 = (x + w).ceil() as i32;
        let y1 = (y + h).ceil() as i32;
        for py in y0..y1 {
            let cov_y = ((py as f32 + 1.0).min(y + h) - (py as f32).max(y)).clamp(0.0, 1.0);
            if cov_y <= 0.0 {
                continue;
            }
            let t = (((py as f32 + 0.5) - y) / h).clamp(0.0, 1.0);
            let c = top.lerp(bottom, t);
            let pk = c.packed();
            let alpha = (c.a * 255.0 * cov_y).clamp(0.0, 255.0) as u32;
            for px in x0..x1 {
                let cov_x = ((px as f32 + 1.0).min(x + w) - (px as f32).max(x)).clamp(0.0, 1.0);
                if cov_x > 0.0 {
                    self.put(px, py, pk, (cov_x * alpha as f32) as u32);
                }
            }
        }
    }

    /// 整窗竖向渐变背景：直接覆写像素（最底层，无需混合，最快路径）。
    pub fn fill_background(&mut self, top: Color, bottom: Color) {
        let (tr, tg, tb) = (top.r, top.g, top.b);
        let (br, bg, bb) = (bottom.r, bottom.g, bottom.b);
        let hh = self.h as f32;
        for y in 0..self.h {
            let t = (y as f32 + 0.5) / hh;
            let r = (tr + (br - tr) * t) as u32;
            let g = (tg + (bg - tg) * t) as u32;
            let b = (tb + (bb - tb) * t) as u32;
            let pk = 0xFF00_0000 | (r << 16) | (g << 8) | b;
            unsafe {
                let row = self.px.add((y * self.w) as usize);
                for x in 0..self.w {
                    *row.add(x as usize) = pk;
                }
            }
        }
    }

    /// 径向柔和光斑（仿 Mica 背景光晕 / 泛光）。r² 四次衰减，无 sqrt。
    pub fn radial_glow(&mut self, cx: f32, cy: f32, radius: f32, c: Color, strength: f32) {
        if radius <= 0.0 || strength <= 0.0 {
            return;
        }
        let pk = c.packed();
        let ca = (c.a * 255.0).clamp(0.0, 255.0) as u32;
        let inv_r2 = 1.0 / (radius * radius);
        let x0 = (cx - radius).floor() as i32;
        let y0 = (cy - radius).floor() as i32;
        let x1 = (cx + radius).ceil() as i32;
        let y1 = (cy + radius).ceil() as i32;
        for py in y0..y1 {
            let dy = py as f32 + 0.5 - cy;
            let dy2 = dy * dy;
            for px in x0..x1 {
                let dx = px as f32 + 0.5 - cx;
                let d2 = (dx * dx + dy2) * inv_r2;
                if d2 >= 1.0 {
                    continue;
                }
                let f = 1.0 - d2;
                let cov = (f * f * strength * ca as f32) as u32;
                if cov > 0 {
                    self.put(px, py, pk, cov);
                }
            }
        }
    }

    /// 圆角矩形：填充 / 描边 / 卡片阴影。
    #[allow(clippy::too_many_arguments)]
    pub fn rounded_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        fill: Option<Color>,
        stroke: Option<(Color, f32)>,
        shadow: Option<(Color, f32, f32)>,
    ) {
        let cx = x + w * 0.5;
        let cy = y + h * 0.5;
        let hw = w * 0.5;
        let hh = h * 0.5;
        let fill_pk = fill.map(|f| f.packed());
        let fill_a = fill.map(|f| (f.a * 255.0).clamp(0.0, 255.0) as u32).unwrap_or(0);
        let stroke_pk = stroke.map(|s| s.0.packed());
        let stroke_hwid = stroke.map(|s| s.1 * 0.5).unwrap_or(0.0);
        let stroke_a = stroke.map(|s| (s.0.a * 255.0).clamp(0.0, 255.0) as u32).unwrap_or(0);
        let (sx, sy) = match shadow {
            Some((_, blur, dy)) => (blur + 2.0, blur + 2.0 + dy.abs()),
            None => (1.0, 1.0),
        };
        let x0 = (x - sx).floor() as i32;
        let y0 = (y - sy).floor() as i32;
        let x1 = (x + w + sx).ceil() as i32;
        let y1 = (y + h + sy).ceil() as i32;
        for py in y0..y1 {
            let fy = py as f32 + 0.5;
            for px in x0..x1 {
                let fx = px as f32 + 0.5;
                if let Some((sc, blur, dy)) = shadow {
                    if blur > 0.0 {
                        let d2 = sd_round_rect(fx, fy - dy, cx, cy, hw, hh, r);
                        if d2 > 0.0 {
                            let t = 1.0 - d2 / blur;
                            if t > 0.0 {
                                self.put(px, py, sc.packed(), (t * t * sc.a * 255.0) as u32);
                            }
                        }
                    }
                }
                let d = sd_round_rect(fx, fy, cx, cy, hw, hh, r);
                if let Some(pk) = fill_pk {
                    let cov = (0.5 - d).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        self.put(px, py, pk, (cov * fill_a as f32) as u32);
                    }
                }
                if let Some(pk) = stroke_pk {
                    let cov = (stroke_hwid - d.abs() + 0.5).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        self.put(px, py, pk, (cov * stroke_a as f32) as u32);
                    }
                }
            }
        }
    }

    /// 圆角矩形：竖向渐变填充（按钮/强调色块），带描边与投影。
    #[allow(clippy::too_many_arguments)]
    pub fn rounded_rect_vgrad(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        top: Color,
        bottom: Color,
        stroke: Option<(Color, f32)>,
        shadow: Option<(Color, f32, f32)>,
    ) {
        let cx = x + w * 0.5;
        let cy = y + h * 0.5;
        let hw = w * 0.5;
        let hh = h * 0.5;
        let stroke_pk = stroke.map(|s| s.0.packed());
        let stroke_hwid = stroke.map(|s| s.1 * 0.5).unwrap_or(0.0);
        let stroke_a = stroke.map(|s| (s.0.a * 255.0).clamp(0.0, 255.0) as u32).unwrap_or(0);
        let sy = match shadow {
            Some((_, blur, dy)) => blur + 2.0 + dy.abs(),
            None => 1.0,
        };
        let x0 = (x - sy).floor() as i32;
        let y0 = (y - sy).floor() as i32;
        let x1 = (x + w + sy).ceil() as i32;
        let y1 = (y + h + sy).ceil() as i32;
        for py in y0..y1 {
            let fy = py as f32 + 0.5;
            let t = ((fy - y) / h).clamp(0.0, 1.0);
            let c = top.lerp(bottom, t);
            let pk = c.packed();
            for px in x0..x1 {
                let fx = px as f32 + 0.5;
                if let Some((sc, blur, dy)) = shadow {
                    if blur > 0.0 {
                        let d2 = sd_round_rect(fx, fy - dy, cx, cy, hw, hh, r);
                        if d2 > 0.0 {
                            let tt = (1.0 - d2 / blur).clamp(0.0, 1.0);
                            self.put(px, py, sc.packed(), (tt * tt * sc.a * 255.0) as u32);
                        }
                    }
                }
                let d = sd_round_rect(fx, fy, cx, cy, hw, hh, r);
                let cov = (0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.put(px, py, pk, (cov * c.a * 255.0) as u32);
                }
                if let Some(spk) = stroke_pk {
                    let cov = (stroke_hwid - d.abs() + 0.5).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        self.put(px, py, spk, (cov * stroke_a as f32) as u32);
                    }
                }
            }
        }
    }

    /// 圆：填充 + 可选描边。
    pub fn circle(&mut self, cx: f32, cy: f32, r: f32, fill: Option<Color>, stroke: Option<(Color, f32)>) {
        let ext = r + stroke.map(|s| s.1).unwrap_or(0.0) + 1.5;
        let fill_pk = fill.map(|f| f.packed());
        let fill_a = fill.map(|f| (f.a * 255.0) as u32).unwrap_or(0);
        let stroke_pk = stroke.map(|s| s.0.packed());
        let stroke_hwid = stroke.map(|s| s.1 * 0.5).unwrap_or(0.0);
        let stroke_a = stroke.map(|s| (s.0.a * 255.0) as u32).unwrap_or(0);
        let x0 = (cx - ext).floor() as i32;
        let y0 = (cy - ext).floor() as i32;
        let x1 = (cx + ext).ceil() as i32;
        let y1 = (cy + ext).ceil() as i32;
        for py in y0..y1 {
            let dy = py as f32 + 0.5 - cy;
            for px in x0..x1 {
                let dx = px as f32 + 0.5 - cx;
                let d = (dx * dx + dy * dy).sqrt() - r;
                if let Some(pk) = fill_pk {
                    let cov = (0.5 - d).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        self.put(px, py, pk, (cov * fill_a as f32) as u32);
                    }
                }
                if let Some(pk) = stroke_pk {
                    let cov = (stroke_hwid - d.abs() + 0.5).clamp(0.0, 1.0);
                    if cov > 0.0 {
                        self.put(px, py, pk, (cov * stroke_a as f32) as u32);
                    }
                }
            }
        }
    }

    /// 圆弧：SDF 圆环 + 圆角端帽 + 角度渐变 + 可选外发光。
    /// 角度定义：start=0 从正上方开始，顺时针，单位弧度。
    #[allow(clippy::too_many_arguments)]
    pub fn arc(
        &mut self,
        cx: f32,
        cy: f32,
        radius: f32,
        half_w: f32,
        start: f32,
        sweep: f32,
        c1: Color,
        c2: Color,
        glow: Option<(Color, f32, f32)>,
    ) {
        if sweep <= 0.0 {
            return;
        }
        let glow_w = glow.map(|g| g.1).unwrap_or(0.0);
        let ext = radius + half_w + glow_w + 2.0;
        let x0 = (cx - ext).floor().max(0.0) as i32;
        let y0 = (cy - ext).floor().max(0.0) as i32;
        let x1 = (cx + ext).ceil().min(self.w as f32) as i32;
        let y1 = (cy + ext).ceil().min(self.h as f32) as i32;
        let full = sweep >= TAU - 1e-4;

        // 角度渐变 LUT（32 级足够平滑）
        let mut lut = [0u32; 33];
        for (i, slot) in lut.iter_mut().enumerate() {
            *slot = c1.lerp(c2, i as f32 / 32.0).packed();
        }
        let glow_pk = glow.map(|g| g.0.packed());
        let glow_a = glow.map(|g| (g.2 * 255.0) as u32).unwrap_or(0);

        let (sin0, cos0) = (start.sin(), start.cos());
        let (sin1, cos1) = ((start + sweep).sin(), (start + sweep).cos());
        let start_pt = (cx + radius * sin0, cy - radius * cos0);
        let end_pt = (cx + radius * sin1, cy - radius * cos1);

        for py in y0..y1 {
            let dy = py as f32 + 0.5 - cy;
            for px in x0..x1 {
                let fx = px as f32 + 0.5;
                let dx = fx - cx;
                let len2 = dx * dx + dy * dy;
                if len2 < 1e-6 {
                    continue;
                }
                let len = len2.sqrt();
                let mut rel = 0.0f32;
                if !full {
                    let mut theta = fast_atan2(dx, -dy);
                    if theta < 0.0 {
                        theta += TAU;
                    }
                    rel = theta - start;
                    while rel < 0.0 {
                        rel += TAU;
                    }
                    while rel >= TAU {
                        rel -= TAU;
                    }
                }
                let d = if full || rel <= sweep {
                    (len - radius).abs()
                } else {
                    let ds = (fx - start_pt.0).powi(2) + (py as f32 + 0.5 - start_pt.1).powi(2);
                    let de = (fx - end_pt.0).powi(2) + (py as f32 + 0.5 - end_pt.1).powi(2);
                    ds.sqrt().min(de.sqrt())
                };
                if let Some(gpk) = glow_pk {
                    if glow_w > 0.0 && d > half_w {
                        let tt = 1.0 - (d - half_w) / glow_w;
                        if tt > 0.0 {
                            let amount = (tt * tt * glow_a as f32) as u32;
                            if amount > 2 {
                                self.put_add(px, py, gpk, amount);
                            }
                        }
                    }
                }
                let cov = ((half_w + 0.5 - d) * 255.0).clamp(0.0, 255.0) as u32;
                if cov > 0 {
                    let idx = if full {
                        ((rel / TAU) * 32.0) as usize
                    } else {
                        ((rel / sweep) * 32.0).clamp(0.0, 32.0) as usize
                    };
                    self.put(px, py, lut[idx.min(32)], cov);
                }
            }
        }
    }

    /// 整圈轨道环（无角度计算，纯径向 SDF，比 arc 快）。
    pub fn ring_full(&mut self, cx: f32, cy: f32, radius: f32, half_w: f32, c: Color) {
        let pk = c.packed();
        let alpha = (c.a * 255.0).clamp(0.0, 255.0) as u32;
        let ext = radius + half_w + 2.0;
        let x0 = (cx - ext).floor().max(0.0) as i32;
        let y0 = (cy - ext).floor().max(0.0) as i32;
        let x1 = (cx + ext).ceil().min(self.w as f32) as i32;
        let y1 = (cy + ext).ceil().min(self.h as f32) as i32;
        for py in y0..y1 {
            let dy = py as f32 + 0.5 - cy;
            let dy2 = dy * dy;
            for px in x0..x1 {
                let dx = px as f32 + 0.5 - cx;
                let d = ((dx * dx + dy2).sqrt() - radius).abs();
                if d > half_w + 1.0 {
                    continue;
                }
                let cov = (half_w + 0.5 - d).clamp(0.0, 1.0);
                if cov > 0.0 {
                    self.put(px, py, pk, (cov * alpha as f32) as u32);
                }
            }
        }
    }
}

/// 角度 -> 弧度便捷函数。
#[inline]
pub fn deg(d: f32) -> f32 {
    d * PI / 180.0
}
