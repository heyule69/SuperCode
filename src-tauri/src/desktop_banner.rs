//! Small, cached native cue artwork. Rendering uses a memory DC, never a desktop window.
use std::ptr::null_mut;
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*};

pub const TEXT: &str = "SuperCode 正在控制电脑  ·  按 Esc 取消";
pub struct Artwork {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u8>,
}
struct Dc(HDC);
impl Drop for Dc {
    fn drop(&mut self) {
        unsafe {
            DeleteDC(self.0);
        }
    }
}
struct Selected {
    dc: HDC,
    object: HGDIOBJ,
    previous: HGDIOBJ,
}
impl Selected {
    unsafe fn new(dc: HDC, object: HGDIOBJ) -> Result<Self, String> {
        if object.is_null() {
            return Err("无法创建电脑控制提示绘图资源".into());
        }
        let previous = SelectObject(dc, object);
        if previous.is_null() || previous as isize == -1 {
            DeleteObject(object);
            return Err("无法选择电脑控制提示绘图资源".into());
        }
        Ok(Self {
            dc,
            object,
            previous,
        })
    }
}
impl Drop for Selected {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.object);
        }
    }
}
fn coverage(x: i32, y: i32, width: i32, height: i32, radius: f32) -> u8 {
    let dx = (radius - x as f32 - 0.5)
        .max(x as f32 + 0.5 - (width as f32 - radius))
        .max(0.0);
    let dy = (radius - y as f32 - 0.5)
        .max(y as f32 + 0.5 - (height as f32 - radius))
        .max(0.0);
    ((radius + 0.5 - dx.hypot(dy)).clamp(0.0, 1.0) * 255.0) as u8
}
pub fn render(scale: f32, max_width: i32) -> Result<Artwork, String> {
    if max_width < 160 {
        return Err("屏幕空间不足，无法完整显示电脑控制提示".into());
    }
    let scale = if scale.is_finite() {
        scale.clamp(0.75, 4.0)
    } else {
        1.0
    };
    let text: Vec<u16> = TEXT.encode_utf16().collect();
    let face: Vec<u16> = "Microsoft YaHei UI".encode_utf16().chain(Some(0)).collect();
    unsafe {
        let dc = Dc(CreateCompatibleDC(null_mut()));
        if dc.0.is_null() {
            return Err("无法创建电脑控制提示画布".into());
        }
        let padding = (14.0 * scale).round() as i32;
        let mut font_size = (14.0 * scale).round() as i32;
        let (_font, measured) = loop {
            let font = Selected::new(
                dc.0,
                CreateFontW(
                    -font_size,
                    0,
                    0,
                    0,
                    FW_MEDIUM as i32,
                    0,
                    0,
                    0,
                    DEFAULT_CHARSET as u32,
                    OUT_DEFAULT_PRECIS as u32,
                    CLIP_DEFAULT_PRECIS as u32,
                    ANTIALIASED_QUALITY as u32,
                    (DEFAULT_PITCH | FF_DONTCARE) as u32,
                    face.as_ptr(),
                ),
            )?;
            let mut measured: SIZE = std::mem::zeroed();
            if GetTextExtentPoint32W(dc.0, text.as_ptr(), text.len() as i32, &mut measured) == 0 {
                return Err("无法测量电脑控制中文提示".into());
            }
            if measured.cx + padding * 2 <= max_width {
                break (font, measured);
            }
            drop(font);
            font_size -= 1;
            if font_size < 8 {
                return Err("屏幕空间不足，无法完整显示取消说明".into());
            }
        };
        let width = measured.cx + padding * 2;
        let height = (38.0 * scale).round() as i32;
        let mut info: BITMAPINFO = std::mem::zeroed();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits = null_mut();
        let _bitmap = Selected::new(
            dc.0,
            CreateDIBSection(dc.0, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0),
        )?;
        if bits.is_null() {
            return Err("无法绘制电脑控制中文提示".into());
        }
        let buffer =
            std::slice::from_raw_parts_mut(bits.cast::<u8>(), (width * height * 4) as usize);
        let radius = 9.0 * scale;
        for y in 0..height {
            for x in 0..width {
                let a = coverage(x, y, width, height, radius);
                let i = ((y * width + x) * 4) as usize;
                for (channel, color) in [212u16, 139, 7].into_iter().enumerate() {
                    buffer[i + channel] = (color * a as u16 / 255) as u8;
                }
                buffer[i + 3] = a;
            }
        }
        SetBkMode(dc.0, TRANSPARENT as i32);
        SetTextColor(dc.0, 0x00ff_ffff);
        let mut rect = RECT {
            left: padding,
            top: 0,
            right: width - padding,
            bottom: height,
        };
        if DrawTextW(
            dc.0,
            text.as_ptr(),
            text.len() as i32,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        ) == 0
        {
            return Err("无法绘制电脑控制中文提示".into());
        }
        GdiFlush();
        // GDI text clears alpha on glyph pixels. Restore the opaque interior;
        // the rounded edge already contains premultiplied background colors.
        for y in 0..height {
            for x in 0..width {
                buffer[((y * width + x) * 4 + 3) as usize] = coverage(x, y, width, height, radius);
            }
        }
        Ok(Artwork {
            width,
            height,
            pixels: buffer.to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chinese_banner_renders_offscreen_at_multiple_dpi_scales() {
        for scale in [0.75, 1.0, 1.5, 2.0, 3.0] {
            let art = render(scale, 900).unwrap();
            assert!(art.width <= 900 && art.height > 24);
            assert_eq!(art.pixels[3], 0);
            assert!(
                art.pixels
                    .chunks_exact(4)
                    .filter(|p| p[0] > 235 && p[1] > 235 && p[2] > 235 && p[3] == 255)
                    .count()
                    > 80
            );
        }
        assert!(render(1.0, 120).is_err());
        let art = render(1.5, 900).unwrap();
        if let Ok(path) = std::env::var("SUPERCODE_BANNER_PREVIEW") {
            image::RgbaImage::from_fn(art.width as u32, art.height as u32, |x, y| {
                let i = ((y * art.width as u32 + x) * 4) as usize;
                image::Rgba([
                    art.pixels[i + 2],
                    art.pixels[i + 1],
                    art.pixels[i],
                    art.pixels[i + 3],
                ])
            })
            .save(path)
            .unwrap();
        }
    }
}
