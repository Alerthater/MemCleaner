//! 应用图标：纯软件绘制成 HICON（无需 .ico 资源文件，也不依赖 rc.exe）。
//!
//! 图形：圆角方形强调色渐变底 + 白色 "清理" 圆环（270° 弧，圆头端点）。

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;

/// 生成指定像素尺寸的应用图标（建议 16/20/24/32/48/256）。
pub fn app_icon(size: i32) -> HICON {
    let size = size.clamp(8, 256);
    let mut pixels = vec![0u32; (size * size) as usize];

    let s = size as f32;
    // 圆角方形底
    let r = s * 0.28;
    let (c1, c2) = (0x4C_C2_FF_u32, 0x7B_8C_FF_u32); // #4CC2FF -> #7B8CFF
    // 圆环参数
    let center = s * 0.5;
    let ring_r = s * 0.30;
    let thickness = (s * 0.125).max(1.4);
    let start_angle = -210.0_f32.to_radians();
    let sweep = 270.0_f32.to_radians();

    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            // 圆角矩形 SDF 覆盖率
            let cov = rounded_rect_coverage(px, py, s, s, r);
            if cov <= 0.0 {
                continue;
            }
            // 对角渐变
            let t = ((px + py) / (2.0 * s)).clamp(0.0, 1.0);
            let bg = lerp_color(c1, c2, t);
            let mut color = bg;
            // 白色圆环（带缺口，模拟仪表盘）
            let dx = px - center;
            let dy = py - center;
            let dist = (dx * dx + dy * dy).sqrt();
            let edge = (thickness * 0.5 - (dist - ring_r).abs()) / 1.2 + 0.5;
            let ring_cov = edge.clamp(0.0, 1.0);
            if ring_cov > 0.0 {
                let mut ang = dy.atan2(dx);
                // 归一到 [start, start+2pi)
                while ang < start_angle {
                    ang += std::f32::consts::TAU;
                }
                let in_arc = ang <= start_angle + sweep;
                if in_arc {
                    color = blend(color, 0x00EE_F4_FF, ring_cov);
                }
            }
            // 圆角端点补丁（简单起见在弧两端画圆点）
            for cap_sign in [-1.0_f32, 1.0] {
                let cap_ang = start_angle + if cap_sign > 0.0 { sweep } else { 0.0 };
                let cx = center + cap_ang.cos() * ring_r;
                let cy = center + cap_ang.sin() * ring_r;
                let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
                let cc = ((thickness * 0.5 - d) / 1.2 + 0.5).clamp(0.0, 1.0);
                if cc > 0.0 {
                    color = blend(color, 0x00EE_F4_FF, cc);
                }
            }
            let a = (cov * 255.0).round() as u32;
            pixels[(y * size + x) as usize] = pack(bg_rgb(color, a));
        }
    }

    create_icon_from_argb(&pixels, size, size)
}

/// 圆角矩形像素覆盖率（1 = 完全在内部，0 = 完全在外部），1px 抗锯齿。
fn rounded_rect_coverage(px: f32, py: f32, w: f32, h: f32, r: f32) -> f32 {
    let qx = (px - w * 0.5).abs() - (w * 0.5 - r);
    let qy = (py - h * 0.5).abs() - (h * 0.5 - r);
    let d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
    (-d + 0.5).clamp(0.0, 1.0)
}

fn lerp_color(a: u32, b: u32, t: f32) -> u32 {
    let (ar, ag, ab) = split(a);
    let (br, bg, bb) = split(b);
    join(
        (ar as f32 + (br as f32 - ar as f32) * t) as u32,
        (ag as f32 + (bg as f32 - ag as f32) * t) as u32,
        (ab as f32 + (bb as f32 - ab as f32) * t) as u32,
    )
}

fn blend(dst: u32, src: u32, alpha: f32) -> u32 {
    let (dr, dg, db) = split(dst);
    let (sr, sg, sb) = split(src);
    join(
        (dr as f32 + (sr as f32 - dr as f32) * alpha) as u32,
        (dg as f32 + (sg as f32 - dg as f32) * alpha) as u32,
        (db as f32 + (sb as f32 - db as f32) * alpha) as u32,
    )
}

fn split(c: u32) -> (u32, u32, u32) {
    ((c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF)
}

fn join(r: u32, g: u32, b: u32) -> u32 {
    ((r.clamp(0, 255)) << 16) | ((g.clamp(0, 255)) << 8) | b.clamp(0, 255)
}

/// 保留 RGB、替换 alpha（返回 BGRA 小端 u32）。
fn bg_rgb(color: u32, alpha: u32) -> u32 {
    let (r, g, b) = split(color);
    (alpha << 24) | (r << 16) | (g << 8) | b
}

fn pack(v: u32) -> u32 {
    v
}

/// 把 BGRA 像素数组（u32，alpha 在最高字节）做成 HICON。
fn create_icon_from_argb(pixels: &[u32], w: i32, h: i32) -> HICON {
    unsafe {
        let hdc = CreateCompatibleDC(std::ptr::null_mut());
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h; // top-down
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        bmi.bmiHeader.biSizeImage = (w * h * 4) as u32;
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color_bmp = CreateDIBSection(hdc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if color_bmp.is_null() || bits.is_null() {
            DeleteDC(hdc);
            return std::ptr::null_mut();
        }
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u32, pixels.len());
        // 掩码位图（单色，全 0 = 使用 alpha 通道）
        let mask_bytes = vec![0u8; ((w as usize + 7) / 8 * h as usize).max(1)];
        let mask_bmp = CreateBitmap(w, h, 1, 1, mask_bytes.as_ptr() as *const core::ffi::c_void);
        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask_bmp,
            hbmColor: color_bmp,
        };
        let icon = CreateIconIndirect(&info);
        DeleteObject(color_bmp as HGDIOBJ);
        DeleteObject(mask_bmp as HGDIOBJ);
        DeleteDC(hdc);
        icon
    }
}


// ---------------------------------------------------------------------------
// 托盘「内存占用率」图标
// ---------------------------------------------------------------------------

/// 生成托盘用的内存占用率图标：深色圆底 + 使用率配色圆环 + 白色百分比数字。
///
/// 颜色规则：<75% 蓝色、75–89% 琥珀、>=90% 红色，方便一眼看出内存压力。
pub fn tray_percent_icon(percent: u8, size: i32) -> HICON {
    let size = size.clamp(12, 64);
    let s = size as f32;
    let mut pixels = vec![0u32; (size * size) as usize];
    let center = s * 0.5;
    let outer = s * 0.47;
    let ring_w = (s * 0.12).max(1.6);
    let ring_r = outer - ring_w * 0.5;
    let ring_half = ring_w * 0.5;
    let (cr, cg, cb) = if percent >= 90 {
        (0xEF_u32, 0x44_u32, 0x44_u32)
    } else if percent >= 75 {
        (0xF5, 0x9E, 0x0B)
    } else {
        (0x4C, 0xC2, 0xFF)
    };
    let ring_color = (cr << 16) | (cg << 8) | cb;

    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let dx = px - center;
            let dy = py - center;
            let dist = (dx * dx + dy * dy).sqrt();
            // 圆盘覆盖率（1px 抗锯齿）
            let disk = (outer - dist + 0.5).clamp(0.0, 1.0);
            if disk <= 0.0 {
                continue;
            }
            // 深色底（自上而下轻微渐变，营造立体感）
            let top = 0x27_2A_30_u32;
            let bottom = 0x16_17_1B_u32;
            let mut color = lerp_color(top, bottom, py / s);
            // 外环
            let ring_cov = (ring_half - (dist - ring_r).abs() + 0.5).clamp(0.0, 1.0);
            if ring_cov > 0.0 {
                color = blend(color, ring_color, ring_cov);
            }
            let a = (disk * 255.0).round() as u32;
            pixels[(y * size + x) as usize] = bg_rgb(color, a);
        }
    }

    // 数字：<=99 显示两位；100 显示三位（字体略小）
    let text = format!("{percent}");
    let font_px = if percent >= 100 {
        ((s * 0.36).round() as i32).max(6)
    } else {
        ((s * 0.44).round() as i32).max(6)
    };
    create_icon_with_text(&pixels, size, size, &text, font_px, 0xF2_F2_F5)
}

/// 把 BGRA 像素数组做成 HICON，并在中央绘制一行文字（ClearType/抗锯齿）。
fn create_icon_with_text(
    pixels: &[u32],
    w: i32,
    h: i32,
    text: &str,
    font_px: i32,
    color: u32,
) -> HICON {
    unsafe {
        let hdc = CreateCompatibleDC(std::ptr::null_mut());
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        bmi.bmiHeader.biSizeImage = (w * h * 4) as u32;
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color_bmp = CreateDIBSection(hdc, &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if color_bmp.is_null() || bits.is_null() {
            DeleteDC(hdc);
            return std::ptr::null_mut();
        }
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u32, pixels.len());

        // 在 DIB 上绘制数字
        let old = SelectObject(hdc, color_bmp as HGDIOBJ);
        let face = super::wide("Segoe UI");
        let font = CreateFontW(
            -font_px,
            0,
            0,
            0,
            700,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            ANTIALIASED_QUALITY as u32,
            (DEFAULT_PITCH | 1) as u32,
            face.as_ptr(),
        );
        let old_font = SelectObject(hdc, font as HGDIOBJ);
        SetBkMode(hdc, 1); // TRANSPARENT
        SetTextColor(hdc, colorref(color));
        let mut wtext = super::wide(text);
        let mut rc = RECT {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        };
        DrawTextW(
            hdc,
            wtext.as_mut_ptr(),
            -1,
            &mut rc,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(hdc, old_font);
        DeleteObject(font as HGDIOBJ);
        SelectObject(hdc, old);

        // 掩码位图（单色，全 0 = 使用 alpha 通道）
        let mask_bytes = vec![0u8; ((w as usize + 7) / 8 * h as usize).max(1)];
        let mask_bmp = CreateBitmap(w, h, 1, 1, mask_bytes.as_ptr() as *const core::ffi::c_void);
        let info = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask_bmp,
            hbmColor: color_bmp,
        };
        let icon = CreateIconIndirect(&info);
        DeleteObject(color_bmp as HGDIOBJ);
        DeleteObject(mask_bmp as HGDIOBJ);
        DeleteDC(hdc);
        icon
    }
}

fn colorref(rgb: u32) -> u32 {
    ((rgb & 0xFF) << 16) | (((rgb >> 8) & 0xFF) << 8) | ((rgb >> 16) & 0xFF)
}
