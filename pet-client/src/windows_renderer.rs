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
    scale_factor: f32,
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
        let Ok((bitmap, bits)) = create_dib(dc, width, height) else {
            unsafe { DeleteDC(dc) };
            anyhow::bail!("CreateDIBSection failed");
        };
        let old_bitmap = unsafe { SelectObject(dc, bitmap) };
        Ok(Self {
            hwnd,
            dc,
            bitmap,
            old_bitmap,
            pixels: bits.cast(),
            width,
            height,
            scale_factor: scale_factor as f32,
            font: load_bubble_font(),
        })
    }

    pub fn resize(&mut self, _size: PhysicalSize<u32>) {
        // The pet window is fixed-size; recreating the DIB is unnecessary.
    }

    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        let scale_factor = scale_factor.clamp(1.0, 4.0) as f32;
        let width = (PET_LOGICAL_WIDTH as f32 * scale_factor).round() as u32;
        let height = (PET_LOGICAL_HEIGHT as f32 * scale_factor).round() as u32;
        if width == self.width && height == self.height {
            self.scale_factor = scale_factor;
            return;
        }
        let Ok((bitmap, bits)) = create_dib(self.dc, width, height) else {
            return;
        };
        unsafe {
            SelectObject(self.dc, bitmap);
            DeleteObject(self.bitmap);
        }
        self.bitmap = bitmap;
        self.pixels = bits.cast();
        self.width = width;
        self.height = height;
        self.scale_factor = scale_factor;
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
        let pet_size = (144.0 * self.scale_factor).round().max(1.0) as u32;
        let pet_top = self.height.saturating_sub(pet_size);
        for y in pet_top..self.height {
            for x in 0..self.width {
                let point = [
                    (x as f32 + 0.5) / self.width as f32,
                    (y.saturating_sub(pet_top) as f32 + 0.5) / pet_size as f32
                        + idle_bob
                        + happy_bob,
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
            draw_bubble(
                pixels,
                self.width,
                self.height,
                self.scale_factor,
                text,
                self.font.as_ref(),
            );
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

fn create_dib(dc: HDC, width: u32, height: u32) -> anyhow::Result<(HBITMAP, *mut c_void)> {
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
    ensure!(
        !bitmap.is_null() && !bits.is_null(),
        "CreateDIBSection failed"
    );
    Ok((bitmap, bits))
}

fn draw_bubble(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    scale_factor: f32,
    text: &str,
    font: Option<&FontArc>,
) {
    let px = |logical: f32| (logical * scale_factor).round().max(1.0) as u32;
    let pet_height = px(144.0);
    let bubble_height = height.saturating_sub(pet_height).min(px(72.0));
    if bubble_height < px(24.0) {
        return;
    }
    let margin_x = px(3.0);
    let top = px(2.0);
    let bottom_gap = px(7.0);
    let corner_x = px(13.0);
    let corner_top = px(12.0);
    let corner_bottom = px(17.0);
    let radius = px(10.0) as f32;
    for y in top..bubble_height.saturating_sub(bottom_gap) {
        for x in margin_x..width.saturating_sub(margin_x) {
            let dx = if x < corner_x {
                corner_x - x
            } else if x > width.saturating_sub(corner_x + 1) {
                x - width.saturating_sub(corner_x + 1)
            } else {
                0
            } as f32;
            let dy = if y < corner_top {
                corner_top - y
            } else if y > bubble_height.saturating_sub(corner_bottom) {
                y - bubble_height.saturating_sub(corner_bottom)
            } else {
                0
            } as f32;
            if dx * dx + dy * dy <= radius * radius {
                put_premultiplied(pixels, width, x, y, [255, 247, 220, 245]);
            }
        }
    }
    for y in bubble_height.saturating_sub(px(8.0))..bubble_height {
        let half = (bubble_height - y) + px(1.0);
        let center = width / 2;
        for x in center.saturating_sub(half)..=(center + half).min(width - 1) {
            put_premultiplied(pixels, width, x, y, [255, 247, 220, 245]);
        }
    }
    let Some(font) = font else { return };
    let scale = PxScale::from(17.0 * scale_factor);
    let scaled = font.as_scaled(scale);
    let line_height = scaled.height() + 2.0 * scale_factor;
    let text_left = 10.0 * scale_factor;
    let max_text_width = width as f32 - 20.0 * scale_factor;
    let lines = wrap_text(font, scale, text, max_text_width, 2);
    for (line_index, line) in lines.iter().take(2).enumerate() {
        let mut caret = point(
            text_left,
            8.0 * scale_factor + scaled.ascent() + line_index as f32 * line_height,
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

fn wrap_text(
    font: &FontArc,
    scale: PxScale,
    text: &str,
    max_width: f32,
    max_lines: usize,
) -> Vec<String> {
    let scaled = font.as_scaled(scale);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width = 0.0;
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        let advance = scaled.h_advance(scaled.glyph_id(character));
        let starts_new_line =
            character == '\n' || (!line.is_empty() && line_width + advance > max_width);
        if starts_new_line {
            if lines.len() + 1 >= max_lines {
                add_ellipsis(font, scale, &mut line, max_width);
                lines.push(line);
                return lines;
            }
            lines.push(std::mem::take(&mut line));
            line_width = 0.0;
            if character == '\n' {
                continue;
            }
        }
        line.push(character);
        line_width += advance;
    }
    if !line.is_empty() && lines.len() < max_lines {
        lines.push(line);
    }
    lines
}

fn add_ellipsis(font: &FontArc, scale: PxScale, line: &mut String, max_width: f32) {
    let scaled = font.as_scaled(scale);
    let ellipsis = '…';
    let ellipsis_width = scaled.h_advance(scaled.glyph_id(ellipsis));
    while !line.is_empty()
        && line
            .chars()
            .map(|character| scaled.h_advance(scaled.glyph_id(character)))
            .sum::<f32>()
            + ellipsis_width
            > max_width
    {
        line.pop();
    }
    line.push(ellipsis);
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
