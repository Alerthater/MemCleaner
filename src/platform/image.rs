//! 自定义皮肤图片解码。
//!
//! 使用系统自带的 GDI+ 平 API（gdiplus.dll）解码 PNG/JPEG/BMP/GIF 等格式，并按需缩小到
//! 合适分辨率后拷贝成 RGBA 像素缓冲；只在用户选择自定义皮肤时加载，默认配置下不加载任何
//! 图片解码库。GDI+ 调用通过手写 extern 声明，不引入额外 crate。

use crate::ui::SkinImage;
use std::ffi::c_void;

type GpStatus = i32;
type GpBitmap = c_void;
type GpImage = c_void;
type GpGraphics = c_void;

#[repr(C)]
struct GdiplusStartupInput {
    gdiplus_version: u32,
    debug_event_callback: isize,
    suppress_background_thread: i32,
    suppress_external_codecs: i32,
}

#[repr(C)]
struct RectI {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

#[repr(C)]
struct BitmapData {
    width: u32,
    height: u32,
    stride: i32,
    pixel_format: i32,
    scan0: *mut c_void,
    reserved: usize,
}

/// PixelFormat32bppARGB
const PIXEL_FORMAT_32BPP_ARGB: i32 = 0x0026_200A;
/// ImageLockModeRead
const IMAGE_LOCK_MODE_READ: u32 = 1;
/// InterpolationModeHighQualityBicubic
const INTERPOLATION_HIGH_QUALITY_BICUBIC: i32 = 7;
/// 自定义皮肤最大边长：更大的图片先缩小，控制常驻内存（约 1400×1400×4 ≈ 7.8 MB 上限）。
const MAX_SKIN_DIM: u32 = 1400;

#[link(name = "gdiplus")]
extern "system" {
    fn GdiplusStartup(
        token: *mut usize,
        input: *const GdiplusStartupInput,
        output: *mut c_void,
    ) -> GpStatus;
    fn GdiplusShutdown(token: usize);
    fn GdipCreateBitmapFromFile(filename: *const u16, bitmap: *mut *mut GpBitmap) -> GpStatus;
    fn GdipGetImageWidth(image: *mut GpImage, width: *mut u32) -> GpStatus;
    fn GdipGetImageHeight(image: *mut GpImage, height: *mut u32) -> GpStatus;
    fn GdipCreateBitmapFromScan0(
        width: i32,
        height: i32,
        stride: i32,
        format: i32,
        scan0: *mut u8,
        bitmap: *mut *mut GpBitmap,
    ) -> GpStatus;
    fn GdipGetImageGraphicsContext(image: *mut GpImage, graphics: *mut *mut GpGraphics) -> GpStatus;
    fn GdipSetInterpolationMode(graphics: *mut GpGraphics, mode: i32) -> GpStatus;
    fn GdipDrawImageRectI(
        graphics: *mut GpGraphics,
        image: *mut GpImage,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    ) -> GpStatus;
    fn GdipDeleteGraphics(graphics: *mut GpGraphics) -> GpStatus;
    fn GdipBitmapLockBits(
        bitmap: *mut GpBitmap,
        rect: *const RectI,
        flags: u32,
        format: i32,
        data: *mut BitmapData,
    ) -> GpStatus;
    fn GdipBitmapUnlockBits(bitmap: *mut GpBitmap, data: *mut BitmapData) -> GpStatus;
    fn GdipDisposeImage(image: *mut GpImage) -> GpStatus;
}

/// 解码图片文件为 RGBA 像素（0xAARRGGBB），并按比例缩小到 `MAX_SKIN_DIM` 以内。
/// 失败返回 None（不 panic）。
pub fn load_skin_image(path: &str) -> Option<SkinImage> {
    if path.trim().is_empty() {
        return None;
    }
    let wide_path = crate::platform::wide(path);

    unsafe {
        let mut token: usize = 0;
        let input = GdiplusStartupInput {
            gdiplus_version: 1,
            debug_event_callback: 0,
            suppress_background_thread: 0,
            suppress_external_codecs: 0,
        };
        if GdiplusStartup(&mut token, &input, std::ptr::null_mut()) != 0 {
            return None;
        }

        let mut source: *mut GpBitmap = std::ptr::null_mut();
        let status = GdipCreateBitmapFromFile(wide_path.as_ptr(), &mut source);
        if status != 0 || source.is_null() {
            GdiplusShutdown(token);
            return None;
        }

        let mut sw: u32 = 0;
        let mut sh: u32 = 0;
        if GdipGetImageWidth(source as *mut GpImage, &mut sw) != 0
            || GdipGetImageHeight(source as *mut GpImage, &mut sh) != 0
            || sw == 0
            || sh == 0
        {
            GdipDisposeImage(source as *mut GpImage);
            GdiplusShutdown(token);
            return None;
        }

        // 按比例缩小（不放大），控制内存占用。
        let scale = (MAX_SKIN_DIM as f32 / sw.max(sh) as f32).min(1.0);
        let nw = ((sw as f32 * scale).round() as i32).max(1);
        let nh = ((sh as f32 * scale).round() as i32).max(1);

        let mut dst: *mut GpBitmap = std::ptr::null_mut();
        if GdipCreateBitmapFromScan0(
            nw,
            nh,
            0,
            PIXEL_FORMAT_32BPP_ARGB,
            std::ptr::null_mut(),
            &mut dst,
        ) != 0
            || dst.is_null()
        {
            GdipDisposeImage(source as *mut GpImage);
            GdiplusShutdown(token);
            return None;
        }

        // 高质量缩放绘制到目标位图（同时完成像素格式转换）。
        let mut graphics: *mut GpGraphics = std::ptr::null_mut();
        if GdipGetImageGraphicsContext(dst as *mut GpImage, &mut graphics) == 0
            && !graphics.is_null()
        {
            GdipSetInterpolationMode(graphics, INTERPOLATION_HIGH_QUALITY_BICUBIC);
            GdipDrawImageRectI(graphics, source as *mut GpImage, 0, 0, nw, nh);
            GdipDeleteGraphics(graphics);
        }

        let mut result = None;
        let rect = RectI {
            x: 0,
            y: 0,
            w: nw,
            h: nh,
        };
        let mut data: BitmapData = std::mem::zeroed();
        if GdipBitmapLockBits(
            dst,
            &rect,
            IMAGE_LOCK_MODE_READ,
            PIXEL_FORMAT_32BPP_ARGB,
            &mut data,
        ) == 0
        {
            let w = data.width as i32;
            let h = data.height as i32;
            let stride = data.stride;
            let base = data.scan0 as *const u8;
            if !base.is_null() && w > 0 && h > 0 {
                let mut pixels = Vec::with_capacity((w * h) as usize);
                let abs_stride = stride.unsigned_abs() as usize;
                for y in 0..h {
                    // GDI+ 的 stride 可能为负（自下而上），需要从底部开始读。
                    let row_idx = if stride >= 0 { y } else { h - 1 - y };
                    let row = base.add(row_idx as usize * abs_stride) as *const u32;
                    for x in 0..w {
                        pixels.push(*row.add(x as usize));
                    }
                }
                result = Some(SkinImage { pixels, w, h });
            }
            GdipBitmapUnlockBits(dst, &mut data);
        }

        GdipDisposeImage(dst as *mut GpImage);
        GdipDisposeImage(source as *mut GpImage);
        GdiplusShutdown(token);
        result
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// 用仓库内的截图作为真实 PNG 输入，验证 GDI+ 解码链路可用。
    #[test]
    fn loads_real_png() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/docs/screenshots/ui-final-dark.png"
        );
        if !std::path::Path::new(path).exists() {
            return; // 测试环境裁剪了截图时跳过
        }
        let img = load_skin_image(path).expect("PNG 解码失败");
        assert!(img.w > 0 && img.h > 0);
        assert_eq!(img.pixels.len(), (img.w * img.h) as usize);
        // 至少有一个像素是不透明的
        assert!(img.pixels.iter().any(|p| (p >> 24) != 0));
    }

    #[test]
    fn missing_file_returns_none() {
        assert!(load_skin_image("Z:/definitely/not/exists.png").is_none());
    }
}
