use std::{ffi::c_void, mem::size_of, ptr, sync::Arc};

use ab_glyph::{Font, FontArc, PxScale, ScaleFont, point};
use anyhow::{Context, ensure};
use windows_sys::Win32::{
    Foundation::{POINT, SIZE},
    Graphics::Gdi::{
        AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
        CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP, HDC,
        HGDIOBJ, SelectObject,
    },
    UI::WindowsAndMessaging::{ULW_ALPHA, UpdateLayeredWindow},
};
use winit::{dpi::PhysicalSize, window::Window};

use crate::windows_hwnd;

// Some Windows/remote-display combinations report the desktop work area as the
// initial client size, which would create a giant transparent hit target.
const PET_LOGICAL_WIDTH: f64 = 144.0;
const PET_LOGICAL_HEIGHT: f64 = 220.0;

pub struct Renderer {
    hwnd: isize,
    dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    pixels: *mut u8,
    width: u32,
    height: u32,
    font: Option<FontArc>,
}

impl Renderer {
    pub async fn new(window: Arc<Window>) -> anyhow::Result<Self> {
        let hwnd = windows_hwnd(&window).context("missing Windows window handle")?;
        let scale_factor = window.scale_factor();
        let width = (PET_LOGICAL_WIDTH * scale_factor).round().max(1.0) as u32;
        let height = (PET_LOGICAL_HEIGHT * scale_factor).round().max(1.0) as u32;
        let dc = unsafe { CreateCompatibleDC(ptr::null_mut()) };
        ensure!(!dc.is_null(), "CreateCompatibleDC failed");
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = ptr::null_mut();
        let bitmap =
            unsafe { CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, ptr::null_mut(), 0) };
        if bitmap.is_null() || bits.is_null() {
            unsafe { DeleteDC(dc) };
            anyhow::bail!("CreateDIBSection failed");
        }
        let old_bitmap = unsafe { SelectObject(dc, bitmap) };
        Ok(Self {
            hwnd,
            dc,
            bitmap,
            old_bitmap,
            pixels: bits.cast(),
            width,
            height,
            font: load_bubble_font(),
        })
    }

    pub fn resize(&mut self, _size: PhysicalSize<u32>) {
        // The pet window is fixed-size; recreating the DIB is unnecessary.
    }

    pub fn render(
        &self,
        seconds: f32,
        petting: f32,
        skin: f32,
        bubble: Option<&str>,
    ) -> anyhow::Result<()> {
        let pixels = unsafe {
            std::slice::from_raw_parts_mut(self.pixels, (self.width * self.height * 4) as usize)
        };
        let idle_bob = (seconds * 4.0).sin() * 0.012;
        let happy_bob = (seconds * 12.0).sin().abs() * 0.035 * petting.clamp(0.0, 1.0);
        let (body, head) = palette(skin);
        pixels.fill(0);
        let pet_top = self.height.saturating_sub(144);
        for y in pet_top..self.height {
            for x in 0..self.width {
                let point = [
                    (x as f32 + 0.5) / self.width as f32,
                    (y.saturating_sub(pet_top) as f32 + 0.5) / 144.0 + idle_bob + happy_bob,
                ];
                let mut color = [0.0; 4];
                layer(&mut color, body, ellipse(point, [0.50, 0.60], [0.31, 0.27]));
                layer(&mut color, head, ellipse(point, [0.44, 0.45], [0.25, 0.26]));
                layer(
                    &mut color,
                    mix(body, [0.96, 0.67, 0.15, 1.0], 0.35),
                    ellipse(point, [0.64, 0.66], [0.17, 0.12]),
                );
                layer(
                    &mut color,
                    [0.88, 0.17, 0.10, 1.0],
                    ellipse(point, [0.43, 0.22], [0.07, 0.10]),
                );
                layer(
                    &mut color,
                    [0.88, 0.17, 0.10, 1.0],
                    ellipse(point, [0.51, 0.23], [0.07, 0.11]),
                );
                layer(
                    &mut color,
                    [0.18, 0.11, 0.06, 1.0],
                    ellipse(point, [0.37, 0.42], [0.035, 0.045]),
                );
                layer(
                    &mut color,
                    [0.18, 0.11, 0.06, 1.0],
                    ellipse(point, [0.51, 0.42], [0.035, 0.045]),
                );
                layer(
                    &mut color,
                    [0.98, 0.47, 0.08, 1.0],
                    ellipse(point, [0.44, 0.51], [0.08, 0.05]),
                );
                let alpha = color[3].clamp(0.0, 1.0);
                let offset = ((y * self.width + x) * 4) as usize;
                pixels[offset] = (color[2].clamp(0.0, 1.0) * 255.0) as u8;
                pixels[offset + 1] = (color[1].clamp(0.0, 1.0) * 255.0) as u8;
                pixels[offset + 2] = (color[0].clamp(0.0, 1.0) * 255.0) as u8;
                pixels[offset + 3] = (alpha * 255.0) as u8;
            }
        }
        if let Some(text) = bubble {
            draw_bubble(pixels, self.width, self.height, text, self.font.as_ref());
        }
        let size = SIZE {
            cx: self.width as i32,
            cy: self.height as i32,
        };
        let source = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let updated = unsafe {
            UpdateLayeredWindow(
                self.hwnd as _,
                ptr::null_mut(),
                ptr::null(),
                &size,
                self.dc,
                &source,
                0,
                &blend,
                ULW_ALPHA,
            )
        };
        ensure!(updated != 0, "UpdateLayeredWindow failed");
        Ok(())
    }
}

fn draw_bubble(pixels: &mut [u8], width: u32, height: u32, text: &str, font: Option<&FontArc>) {
    let bubble_height = height.saturating_sub(144).min(72);
    if bubble_height < 24 {
        return;
    }
    for y in 2..bubble_height.saturating_sub(7) {
        for x in 3..width.saturating_sub(3) {
            let radius = 10.0;
            let dx = if x < 13 {
                13 - x
            } else if x > width - 14 {
                x - (width - 14)
            } else {
                0
            } as f32;
            let dy = if y < 12 {
                12 - y
            } else if y > bubble_height - 17 {
                y - (bubble_height - 17)
            } else {
                0
            } as f32;
            if dx * dx + dy * dy <= radius * radius {
                put_premultiplied(pixels, width, x, y, [255, 247, 220, 245]);
            }
        }
    }
    for y in bubble_height.saturating_sub(8)..bubble_height {
        let half = (bubble_height - y) + 1;
        let center = width / 2;
        for x in center.saturating_sub(half)..=(center + half).min(width - 1) {
            put_premultiplied(pixels, width, x, y, [255, 247, 220, 245]);
        }
    }
    let Some(font) = font else { return };
    let scale = PxScale::from(15.0);
    let scaled = font.as_scaled(scale);
    let line_height = scaled.height() + 2.0;
    let lines = wrap_text(text, 15);
    for (line_index, line) in lines.iter().take(2).enumerate() {
        let mut caret = point(
            10.0,
            9.0 + scaled.ascent() + line_index as f32 * line_height,
        );
        for character in line.chars() {
            let glyph_id = scaled.glyph_id(character);
            let glyph = glyph_id.with_scale_and_position(scale, caret);
            caret.x += scaled.h_advance(glyph_id);
            let Some(outlined) = font.outline_glyph(glyph) else {
                continue;
            };
            let bounds = outlined.px_bounds();
            outlined.draw(|x, y, coverage| {
                let px = bounds.min.x as i32 + x as i32;
                let py = bounds.min.y as i32 + y as i32;
                if px >= 0 && py >= 0 && px < width as i32 && py < bubble_height as i32 {
                    put_premultiplied(
                        pixels,
                        width,
                        px as u32,
                        py as u32,
                        [55, 47, 36, (coverage * 255.0) as u8],
                    );
                }
            });
        }
    }
}

fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let chars: Vec<_> = text.chars().collect();
    chars
        .chunks(max_chars)
        .take(2)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

fn put_premultiplied(pixels: &mut [u8], width: u32, x: u32, y: u32, rgba: [u8; 4]) {
    let offset = ((y * width + x) * 4) as usize;
    let alpha = rgba[3] as u16;
    let inverse = 255 - alpha;
    let source = [rgba[2], rgba[1], rgba[0]];
    for channel in 0..3 {
        pixels[offset + channel] = ((source[channel] as u16 * alpha
            + pixels[offset + channel] as u16 * inverse)
            / 255) as u8;
    }
    pixels[offset + 3] = (alpha + pixels[offset + 3] as u16 * inverse / 255) as u8;
}

fn load_bubble_font() -> Option<FontArc> {
    [r"C:\Windows\Fonts\simhei.ttf", r"C:\Windows\Fonts\msyh.ttf"]
        .iter()
        .find_map(|path| std::fs::read(path).ok())
        .and_then(|bytes| FontArc::try_from_vec(bytes).ok())
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old_bitmap);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

fn ellipse(point: [f32; 2], center: [f32; 2], radius: [f32; 2]) -> f32 {
    let dx = (point[0] - center[0]) / radius[0];
    let dy = (point[1] - center[1]) / radius[1];
    1.0 - smoothstep(0.94, 1.0, (dx * dx + dy * dy).sqrt())
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn layer(base: &mut [f32; 4], color: [f32; 4], mask: f32) {
    let alpha = (mask * color[3]).clamp(0.0, 1.0);
    for channel in 0..3 {
        base[channel] = base[channel] * (1.0 - alpha) + color[channel] * alpha;
    }
    base[3] = alpha + base[3] * (1.0 - alpha);
}

fn mix(a: [f32; 4], b: [f32; 4], amount: f32) -> [f32; 4] {
    std::array::from_fn(|index| a[index] * (1.0 - amount) + b[index] * amount)
}

fn palette(skin: f32) -> ([f32; 4], [f32; 4]) {
    if skin > 1.5 {
        ([0.52, 0.29, 0.15, 1.0], [0.78, 0.55, 0.34, 1.0])
    } else if skin > 0.5 {
        ([0.36, 0.58, 0.84, 1.0], [0.66, 0.82, 1.00, 1.0])
    } else {
        ([0.92, 0.58, 0.18, 1.0], [1.00, 0.83, 0.39, 1.0])
    }
}
