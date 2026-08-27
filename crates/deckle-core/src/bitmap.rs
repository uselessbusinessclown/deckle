//! 8-bit greyscale images and PNG interchange. Greyscale rather than 1-bit
//! because the decoder's soft information (PLAN.md section 5.5) is exactly what
//! a 1-bit scan throws away.

use std::path::Path;

#[derive(Clone)]
pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Gray {
    pub fn new(w: usize, h: usize, fill: u8) -> Self {
        Gray {
            w,
            h,
            px: vec![fill; w * h],
        }
    }
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> u8 {
        self.px[y * self.w + x]
    }
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, v: u8) {
        self.px[y * self.w + x] = v;
    }
    /// Bilinear sample with edge clamping; out-of-range reads return paper white.
    pub fn sample(&self, x: f64, y: f64) -> f64 {
        if !x.is_finite() || !y.is_finite() {
            return 255.0;
        }
        let x = x.clamp(0.0, self.w as f64 - 1.001);
        let y = y.clamp(0.0, self.h as f64 - 1.001);
        let x0 = x.floor() as usize;
        let y0 = y.floor() as usize;
        let fx = x - x0 as f64;
        let fy = y - y0 as f64;
        let x1 = (x0 + 1).min(self.w - 1);
        let y1 = (y0 + 1).min(self.h - 1);
        let a = self.get(x0, y0) as f64;
        let b = self.get(x1, y0) as f64;
        let c = self.get(x0, y1) as f64;
        let d = self.get(x1, y1) as f64;
        a * (1.0 - fx) * (1.0 - fy) + b * fx * (1.0 - fy) + c * (1.0 - fx) * fy + d * fx * fy
    }

    pub fn write_png(&self, path: &Path) -> std::io::Result<()> {
        let file = std::fs::File::create(path)?;
        let w = std::io::BufWriter::new(file);
        let mut enc = png::Encoder::new(w, self.w as u32, self.h as u32);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc
            .write_header()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        writer
            .write_image_data(&self.px)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(())
    }

    pub fn read_png(path: &Path) -> std::io::Result<Gray> {
        let file = std::fs::File::open(path)?;
        let dec = png::Decoder::new(std::io::BufReader::new(file));
        let mut reader = dec
            .read_info()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let (w, h) = (info.width as usize, info.height as usize);
        let bytes = &buf[..info.buffer_size()];
        let px = match (info.color_type, info.bit_depth) {
            (png::ColorType::Grayscale, png::BitDepth::Eight) => bytes.to_vec(),
            (png::ColorType::GrayscaleAlpha, png::BitDepth::Eight) => {
                bytes.chunks_exact(2).map(|c| c[0]).collect()
            }
            (png::ColorType::Rgb, png::BitDepth::Eight) => bytes
                .chunks_exact(3)
                .map(|c| luma(c[0], c[1], c[2]))
                .collect(),
            (png::ColorType::Rgba, png::BitDepth::Eight) => bytes
                .chunks_exact(4)
                .map(|c| luma(c[0], c[1], c[2]))
                .collect(),
            (ct, bd) => {
                return Err(std::io::Error::other(format!(
                    "unsupported PNG format {ct:?}/{bd:?}; use 8-bit greyscale or RGB"
                )))
            }
        };
        Ok(Gray { w, h, px })
    }
}

#[inline]
pub fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000) as u8
}

/// Summed-area table over the image, for O(1) window means.
pub struct Integral {
    w: usize,
    h: usize,
    s: Vec<f64>,
    s2: Vec<f64>,
}

impl Integral {
    pub fn new(img: &Gray) -> Self {
        let (w, h) = (img.w, img.h);
        let mut s = vec![0.0; (w + 1) * (h + 1)];
        let mut s2 = vec![0.0; (w + 1) * (h + 1)];
        for y in 0..h {
            let mut row = 0.0;
            let mut row2 = 0.0;
            for x in 0..w {
                let v = img.get(x, y) as f64;
                row += v;
                row2 += v * v;
                s[(y + 1) * (w + 1) + x + 1] = s[y * (w + 1) + x + 1] + row;
                s2[(y + 1) * (w + 1) + x + 1] = s2[y * (w + 1) + x + 1] + row2;
            }
        }
        Integral { w, h, s, s2 }
    }

    /// Mean and standard deviation over the axis-aligned window, clamped to the image.
    pub fn stats(&self, cx: f64, cy: f64, half: f64) -> (f64, f64) {
        let x0 = (cx - half).floor().max(0.0) as usize;
        let y0 = (cy - half).floor().max(0.0) as usize;
        let x1 = ((cx + half).ceil() as isize).clamp(0, self.w as isize) as usize;
        let y1 = ((cy + half).ceil() as isize).clamp(0, self.h as isize) as usize;
        if x1 <= x0 || y1 <= y0 {
            return (255.0, 0.0);
        }
        let n = ((x1 - x0) * (y1 - y0)) as f64;
        let st = self.w + 1;
        let sum = self.s[y1 * st + x1] - self.s[y0 * st + x1] - self.s[y1 * st + x0]
            + self.s[y0 * st + x0];
        let sum2 = self.s2[y1 * st + x1] - self.s2[y0 * st + x1] - self.s2[y1 * st + x0]
            + self.s2[y0 * st + x0];
        let mean = sum / n;
        let var = (sum2 / n - mean * mean).max(0.0);
        (mean, var.sqrt())
    }
}

/// An 8-bit RGB image. Colour mode needs the channels kept apart: under the
/// subtractive model each ink absorbs one primary, so red carries cyan, green
/// carries magenta and blue carries yellow.
#[derive(Clone)]
pub struct Rgb {
    pub w: usize,
    pub h: usize,
    /// Three interleaved bytes per pixel.
    pub px: Vec<u8>,
}

impl Rgb {
    pub fn new(w: usize, h: usize, fill: [u8; 3]) -> Self {
        let mut px = Vec::with_capacity(w * h * 3);
        for _ in 0..w * h {
            px.extend_from_slice(&fill);
        }
        Rgb { w, h, px }
    }
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> [u8; 3] {
        let i = (y * self.w + x) * 3;
        [self.px[i], self.px[i + 1], self.px[i + 2]]
    }
    #[inline]
    pub fn set(&mut self, x: usize, y: usize, v: [u8; 3]) {
        let i = (y * self.w + x) * 3;
        self.px[i] = v[0];
        self.px[i + 1] = v[1];
        self.px[i + 2] = v[2];
    }
    /// Bilinear sample of one channel; out-of-range reads return paper white.
    pub fn sample(&self, ch: usize, x: f64, y: f64) -> f64 {
        if !x.is_finite() || !y.is_finite() {
            return 255.0;
        }
        let x = x.clamp(0.0, self.w as f64 - 1.001);
        let y = y.clamp(0.0, self.h as f64 - 1.001);
        let x0 = x.floor() as usize;
        let y0 = y.floor() as usize;
        let fx = x - x0 as f64;
        let fy = y - y0 as f64;
        let x1 = (x0 + 1).min(self.w - 1);
        let y1 = (y0 + 1).min(self.h - 1);
        let a = self.get(x0, y0)[ch] as f64;
        let b = self.get(x1, y0)[ch] as f64;
        let c = self.get(x0, y1)[ch] as f64;
        let d = self.get(x1, y1)[ch] as f64;
        a * (1.0 - fx) * (1.0 - fy) + b * fx * (1.0 - fy) + c * (1.0 - fx) * fy + d * fx * fy
    }
    pub fn to_luma(&self) -> Gray {
        let mut g = Gray::new(self.w, self.h, 255);
        for i in 0..self.w * self.h {
            g.px[i] = luma(self.px[i * 3], self.px[i * 3 + 1], self.px[i * 3 + 2]);
        }
        g
    }
    /// Mean absolute channel spread. A greyscale scan collapses this to ~0,
    /// which is how the decoder catches a colour archive scanned in mono.
    pub fn channel_spread(&self) -> f64 {
        let step = (self.w * self.h / 20_000).max(1);
        let mut s = 0.0;
        let mut n = 0.0;
        for i in (0..self.w * self.h).step_by(step) {
            let (r, g, b) = (
                self.px[i * 3] as f64,
                self.px[i * 3 + 1] as f64,
                self.px[i * 3 + 2] as f64,
            );
            let m = (r + g + b) / 3.0;
            s += (r - m).abs() + (g - m).abs() + (b - m).abs();
            n += 3.0;
        }
        if n > 0.0 {
            s / n
        } else {
            0.0
        }
    }

    pub fn write_png(&self, path: &Path) -> std::io::Result<()> {
        let file = std::fs::File::create(path)?;
        let w = std::io::BufWriter::new(file);
        let mut enc = png::Encoder::new(w, self.w as u32, self.h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc
            .write_header()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        writer
            .write_image_data(&self.px)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(())
    }
}

/// A scanned page. Colour is optional: a mono archive needs only luminance, and
/// a colour archive scanned in greyscale must be refused rather than guessed at.
pub struct Scan {
    pub luma: Gray,
    pub rgb: Option<Rgb>,
}

impl Scan {
    pub fn grey(g: Gray) -> Scan {
        Scan { luma: g, rgb: None }
    }
    pub fn colour(c: Rgb) -> Scan {
        Scan {
            luma: c.to_luma(),
            rgb: Some(c),
        }
    }
    pub fn mirrored(&self) -> Scan {
        let mut luma = Gray::new(self.luma.w, self.luma.h, 255);
        for y in 0..self.luma.h {
            for x in 0..self.luma.w {
                luma.set(self.luma.w - 1 - x, y, self.luma.get(x, y));
            }
        }
        let rgb = self.rgb.as_ref().map(|c| {
            let mut o = Rgb::new(c.w, c.h, [255; 3]);
            for y in 0..c.h {
                for x in 0..c.w {
                    o.set(c.w - 1 - x, y, c.get(x, y));
                }
            }
            o
        });
        Scan { luma, rgb }
    }

    /// Read a page, keeping colour when the file has it.
    pub fn read_png(path: &Path) -> std::io::Result<Scan> {
        let file = std::fs::File::open(path)?;
        let dec = png::Decoder::new(std::io::BufReader::new(file));
        let mut reader = dec
            .read_info()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let (w, h) = (info.width as usize, info.height as usize);
        let bytes = &buf[..info.buffer_size()];
        Ok(match (info.color_type, info.bit_depth) {
            (png::ColorType::Rgb, png::BitDepth::Eight) => Scan::colour(Rgb {
                w,
                h,
                px: bytes.to_vec(),
            }),
            (png::ColorType::Rgba, png::BitDepth::Eight) => {
                let mut px = Vec::with_capacity(w * h * 3);
                for c in bytes.chunks_exact(4) {
                    px.extend_from_slice(&c[..3]);
                }
                Scan::colour(Rgb { w, h, px })
            }
            _ => Scan::grey(Gray::read_png(path)?),
        })
    }
}
