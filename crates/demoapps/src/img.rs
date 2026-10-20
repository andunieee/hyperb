//! Tiny image helpers shared by the demos that draw: PNG encoding into
//! `data:` URIs (the only image sources the renderers show) and a canvas
//! with just enough drawing for charts.

use base64::Engine;

/// A `data:` URI for PNG bytes.
pub fn data_uri(png: &[u8]) -> String {
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    )
}

/// Encodes a 1-bit image, row-major, `true` meaning white.
pub fn png_bilevel(width: u32, height: u32, white: &[bool]) -> Vec<u8> {
    let row = (width as usize).div_ceil(8);
    let mut data = vec![0u8; row * height as usize];
    for y in 0..height as usize {
        for x in 0..width as usize {
            if white[y * width as usize + x] {
                data[y * row + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    encode(
        width,
        height,
        png::ColorType::Grayscale,
        png::BitDepth::One,
        &data,
    )
}

fn encode(
    width: u32,
    height: u32,
    color: png::ColorType,
    depth: png::BitDepth,
    data: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, width, height);
    enc.set_color(color);
    enc.set_depth(depth);
    let mut writer = enc.write_header().expect("png header");
    writer.write_image_data(data).expect("png data");
    writer.finish().expect("png end");
    out
}

pub type Rgb = [u8; 3];

/// An RGB canvas with the few drawing operations the charts need.
pub struct Canvas {
    pub width: i32,
    pub height: i32,
    pixels: Vec<u8>,
}

impl Canvas {
    pub fn new(width: i32, height: i32, background: Rgb) -> Self {
        let pixels = background
            .iter()
            .copied()
            .cycle()
            .take((width * height * 3) as usize)
            .collect();
        Canvas {
            width,
            height,
            pixels,
        }
    }

    pub fn png(&self) -> Vec<u8> {
        encode(
            self.width as u32,
            self.height as u32,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &self.pixels,
        )
    }

    pub fn put(&mut self, x: i32, y: i32, c: Rgb) {
        if x >= 0 && y >= 0 && x < self.width && y < self.height {
            let i = ((y * self.width + x) * 3) as usize;
            self.pixels[i..i + 3].copy_from_slice(&c);
        }
    }

    pub fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgb) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.put(xx, yy, c);
            }
        }
    }

    /// A line `thick` pixels wide.
    pub fn line(&mut self, (x0, y0): (i32, i32), (x1, y1): (i32, i32), thick: i32, c: Rgb) {
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
        let r = thick / 2;
        for s in 0..=steps {
            let x = x0 + (x1 - x0) * s / steps;
            let y = y0 + (y1 - y0) * s / steps;
            self.rect(x - r, y - r, thick, thick, c);
        }
    }

    pub fn disc(&mut self, cx: i32, cy: i32, r: i32, c: Rgb) {
        for y in -r..=r {
            for x in -r..=r {
                if x * x + y * y <= r * r {
                    self.put(cx + x, cy + y, c);
                }
            }
        }
    }

    /// A pie slice between two angles in radians, measured clockwise from
    /// twelve o'clock.
    pub fn slice(&mut self, cx: i32, cy: i32, r: i32, from: f64, to: f64, c: Rgb) {
        for y in -r..=r {
            for x in -r..=r {
                if x * x + y * y > r * r {
                    continue;
                }
                let mut a = (x as f64).atan2(-(y as f64));
                if a < 0.0 {
                    a += std::f64::consts::TAU;
                }
                if a >= from && a < to {
                    self.put(cx + x, cy + y, c);
                }
            }
        }
    }

    /// Text in a 5x7 bitmap font at `scale`, with its top-left corner at
    /// `(x, y)`. Unknown characters are skipped; lowercase is drawn as
    /// uppercase.
    pub fn text(&mut self, x: i32, y: i32, scale: i32, s: &str, c: Rgb) {
        let mut cx = x;
        for ch in s.chars() {
            if let Some(g) = glyph(ch) {
                for (row, bits) in g.iter().enumerate() {
                    for col in 0..5 {
                        if bits & (0x10 >> col) != 0 {
                            self.rect(cx + col * scale, y + row as i32 * scale, scale, scale, c);
                        }
                    }
                }
            }
            cx += 6 * scale;
        }
    }
}

/// The width of `s` drawn by [`Canvas::text`].
pub fn text_width(s: &str, scale: i32) -> i32 {
    s.chars().count() as i32 * 6 * scale
}

fn glyph(c: char) -> Option<[u8; 7]> {
    Some(match c.to_ascii_uppercase() {
        ' ' => [0, 0, 0, 0, 0, 0, 0],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        '.' => [0, 0, 0, 0, 0, 0x0C, 0x0C],
        ',' => [0, 0, 0, 0, 0x0C, 0x04, 0x08],
        ':' => [0, 0x0C, 0x0C, 0, 0x0C, 0x0C, 0],
        '-' => [0, 0, 0, 0x1F, 0, 0, 0],
        '+' => [0, 0x04, 0x04, 0x1F, 0x04, 0x04, 0],
        '%' => [0x18, 0x19, 0x02, 0x04, 0x08, 0x13, 0x03],
        '/' => [0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '$' => [0x04, 0x0F, 0x14, 0x0E, 0x05, 0x1E, 0x04],
        '\'' => [0x04, 0x04, 0x08, 0, 0, 0, 0],
        '!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0, 0x04],
        '?' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0, 0x04],
        '_' => [0, 0, 0, 0, 0, 0, 0x1F],
        '&' => [0x0C, 0x12, 0x14, 0x08, 0x15, 0x12, 0x0D],
        '#' => [0x0A, 0x0A, 0x1F, 0x0A, 0x1F, 0x0A, 0x0A],
        '*' => [0, 0x04, 0x15, 0x0E, 0x15, 0x04, 0],
        '=' => [0, 0, 0x1F, 0, 0x1F, 0, 0],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_signature_and_uri() {
        let c = Canvas::new(4, 4, [255, 255, 255]);
        let png = c.png();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert!(data_uri(&png).starts_with("data:image/png;base64,iVBOR"));
        let bw = png_bilevel(3, 2, &[true, false, true, false, true, false]);
        assert_eq!(&bw[..4], b"\x89PNG");
    }

    #[test]
    fn text_draws_pixels() {
        let mut c = Canvas::new(20, 10, [0, 0, 0]);
        c.text(0, 0, 1, "A", [255, 255, 255]);
        assert!(c.pixels.contains(&255));
        assert_eq!(text_width("AB", 2), 24);
    }
}
