//! Degradation models for the software loop (PLAN.md section 15.1).
//!
//! render -> degrade -> decode, entirely in memory. These are deliberately
//! crude physical analogues: the point is not photorealism, it is exercising
//! every failure mode the decoder claims to survive, reproducibly, in CI.

use crate::bitmap::Gray;
use crate::rng::Rng;

#[derive(Clone, Debug, Default)]
pub struct Degradation {
    /// Gaussian blur sigma, in cell widths.
    pub blur_cells: f64,
    /// Additive Gaussian noise, in grey levels.
    pub noise: f64,
    /// Rotation in degrees.
    pub rotate_deg: f64,
    /// Uniform scale error, as a fraction (0.01 = 1% too large).
    pub scale: f64,
    /// Corner displacement as a fraction of page width.
    pub perspective: f64,
    /// Dot gain: positive dilates ink, negative erodes, in cell widths.
    pub dot_gain_cells: f64,
    /// Corner-to-corner illumination falloff, as a fraction.
    pub illumination: f64,
    /// Number of speckle blobs.
    pub blobs: usize,
    /// Blob radius range in mm-equivalent pixels.
    pub blob_px: (f64, f64),
    /// Fold lines: (count, width in cells).
    pub folds: (usize, f64),
    /// Stain radius as a fraction of page width, 0 for none.
    pub stain: f64,
    /// Full-width missing strip, as a fraction of page height.
    pub missing_strip: f64,
    /// Invert polarity.
    pub invert: bool,
    /// Mirror horizontally.
    pub mirror: bool,
    /// Quarter turns clockwise.
    pub rotate_quarters: u8,
    pub seed: u64,

    // ---- colour only (PLAN.md 18.16)
    /// Per-plane registration error in cell widths, applied to C, M and Y.
    pub reg_offset_cells: [f64; 3],
    /// Ink density lost per plane, 0..1. 1.0 removes the plane entirely.
    pub plane_fade: [f64; 3],
    /// White-point shift per channel, as a fraction. Scanner lamp ageing.
    pub colour_cast: [f64; 3],
    /// Extra noise on the blue channel, which is the noisiest in practice.
    pub blue_noise: f64,
    /// Ink crosstalk: how much each ink absorbs outside its own primary.
    pub crosstalk: f64,
    /// Collapse all channels to luminance - a colour archive scanned in mono.
    pub greyscale: bool,
}

impl Degradation {
    /// Parse `blur=0.6,noise=5,rotate=1.0` style specifications.
    pub fn parse(spec: &str) -> Result<Degradation, String> {
        let mut d = Degradation {
            blob_px: (2.0, 12.0),
            seed: 1,
            ..Default::default()
        };
        for part in spec.split(',').filter(|s| !s.trim().is_empty()) {
            let (k, v) = part.split_once('=').unwrap_or((part.trim(), "1"));
            let k = k.trim();
            let num = || -> Result<f64, String> {
                v.trim()
                    .parse::<f64>()
                    .map_err(|_| format!("{k}: '{v}' is not a number"))
            };
            match k {
                "blur" => d.blur_cells = num()?,
                "noise" => d.noise = num()?,
                "rotate" => d.rotate_deg = num()?,
                "scale" => d.scale = num()?,
                "perspective" => d.perspective = num()?,
                "dotgain" => d.dot_gain_cells = num()?,
                "illum" => d.illumination = num()?,
                "blobs" => d.blobs = num()? as usize,
                "folds" => d.folds = (num()? as usize, 2.0),
                "stain" => d.stain = num()?,
                "missing" => d.missing_strip = num()?,
                "invert" => d.invert = true,
                "mirror" => d.mirror = true,
                "quarters" => d.rotate_quarters = (num()? as i64).rem_euclid(4) as u8,
                "seed" => d.seed = num()? as u64,
                "reg" => d.reg_offset_cells = [num()?, -num()? * 0.6, num()? * 0.35],
                "regc" => d.reg_offset_cells[0] = num()?,
                "regm" => d.reg_offset_cells[1] = num()?,
                "regy" => d.reg_offset_cells[2] = num()?,
                "fade" => d.plane_fade = [num()?, num()?, num()?],
                "fadec" => d.plane_fade[0] = num()?,
                "fadem" => d.plane_fade[1] = num()?,
                "fadey" => d.plane_fade[2] = num()?,
                "cast" => d.colour_cast = [num()?, num()? * 0.5, -num()?],
                "bluenoise" => d.blue_noise = num()?,
                "crosstalk" => d.crosstalk = num()?,
                "greyscale" | "grayscale" => d.greyscale = true,
                other => return Err(format!("unknown degradation '{other}'")),
            }
        }
        Ok(d)
    }
}

/// Apply a degradation. `cell_px` is the printed cell pitch in pixels, which is
/// what makes blur and dot gain comparable across densities.
pub fn apply(src: &Gray, d: &Degradation, cell_px: f64) -> Gray {
    let mut img = src.clone();
    let mut rng = Rng::new(d.seed);

    if d.dot_gain_cells != 0.0 {
        img = morph(&img, d.dot_gain_cells * cell_px);
    }
    if d.blur_cells > 0.0 {
        img = blur(&img, d.blur_cells * cell_px);
    }
    if d.rotate_deg != 0.0 || d.scale != 0.0 || d.perspective != 0.0 {
        img = warp(&img, d, &mut rng);
    }
    if d.illumination > 0.0 {
        let (w, h) = (img.w as f64, img.h as f64);
        for y in 0..img.h {
            for x in 0..img.w {
                let t = (x as f64 / w + y as f64 / h) / 2.0;
                let f = 1.0 - d.illumination * t;
                let v = (img.get(x, y) as f64 * f).clamp(0.0, 255.0);
                img.set(x, y, v as u8);
            }
        }
    }
    for _ in 0..d.blobs {
        let cx = rng.next_f64() * img.w as f64;
        let cy = rng.next_f64() * img.h as f64;
        let r = d.blob_px.0 + rng.next_f64() * (d.blob_px.1 - d.blob_px.0);
        let ink = rng.next_bool();
        disc(&mut img, cx, cy, r, if ink { 0 } else { 255 }, 1.0);
    }
    if d.folds.0 > 0 {
        for i in 0..d.folds.0 {
            let vertical = i % 2 == 0;
            let wpx = d.folds.1 * cell_px;
            let at =
                (0.15 + 0.7 * rng.next_f64()) * if vertical { img.w as f64 } else { img.h as f64 };
            for y in 0..img.h {
                for x in 0..img.w {
                    let dist = if vertical {
                        (x as f64 - at).abs()
                    } else {
                        (y as f64 - at).abs()
                    };
                    if dist < wpx {
                        // A crease crushes contrast rather than erasing it.
                        let v = img.get(x, y) as f64;
                        let k = 1.0 - dist / wpx;
                        img.set(x, y, (v * (1.0 - k) + 128.0 * k) as u8);
                    }
                }
            }
        }
    }
    if d.stain > 0.0 {
        let r = d.stain * img.w as f64;
        let cx = rng.next_f64() * img.w as f64;
        let cy = rng.next_f64() * img.h as f64;
        disc(&mut img, cx, cy, r, 90, 0.75);
    }
    if d.missing_strip > 0.0 {
        let hpx = (d.missing_strip * img.h as f64) as usize;
        let y0 = ((img.h - hpx.min(img.h)) as f64 * rng.next_f64()) as usize;
        for y in y0..(y0 + hpx).min(img.h) {
            for x in 0..img.w {
                img.set(x, y, 255);
            }
        }
    }
    if d.noise > 0.0 {
        for i in 0..img.px.len() {
            let v = img.px[i] as f64 + rng.next_normal() * d.noise;
            img.px[i] = v.clamp(0.0, 255.0) as u8;
        }
    }
    if d.invert {
        for p in img.px.iter_mut() {
            *p = 255 - *p;
        }
    }
    for _ in 0..d.rotate_quarters {
        img = rot90(&img);
    }
    if d.mirror {
        let mut o = Gray::new(img.w, img.h, 255);
        for y in 0..img.h {
            for x in 0..img.w {
                o.set(img.w - 1 - x, y, img.get(x, y));
            }
        }
        img = o;
    }
    img
}

fn rot90(img: &Gray) -> Gray {
    let mut o = Gray::new(img.h, img.w, 255);
    for y in 0..img.h {
        for x in 0..img.w {
            o.set(img.h - 1 - y, x, img.get(x, y));
        }
    }
    o
}

fn disc(img: &mut Gray, cx: f64, cy: f64, r: f64, value: u8, alpha: f64) {
    let x0 = (cx - r).max(0.0) as usize;
    let x1 = ((cx + r) as usize + 1).min(img.w);
    let y0 = (cy - r).max(0.0) as usize;
    let y1 = ((cy + r) as usize + 1).min(img.h);
    for y in y0..y1 {
        for x in x0..x1 {
            let dx = x as f64 - cx;
            let dy = y as f64 - cy;
            if dx * dx + dy * dy <= r * r {
                let v = img.get(x, y) as f64 * (1.0 - alpha) + value as f64 * alpha;
                img.set(x, y, v as u8);
            }
        }
    }
}

/// Separable box blur applied three times, which approximates a Gaussian well
/// enough for this purpose and is linear in the radius.
fn blur(img: &Gray, sigma: f64) -> Gray {
    if sigma <= 0.05 {
        return img.clone();
    }
    // Three box passes of radius r approximate a Gaussian of
    // sigma = sqrt((2r+1)^2 - 1) / 2, so invert that rather than guessing.
    let r = (((4.0 * sigma * sigma + 1.0).sqrt() - 1.0) / 2.0)
        .round()
        .max(1.0) as usize;
    let mut cur = img.clone();
    for _ in 0..3 {
        cur = box_blur(&cur, r);
    }
    cur
}

fn box_blur(img: &Gray, r: usize) -> Gray {
    let n = (2 * r + 1) as f64;
    let mut tmp = Gray::new(img.w, img.h, 255);
    for y in 0..img.h {
        let mut sum: f64 = (0..=2 * r)
            .map(|k| {
                let x = (k as isize - r as isize).clamp(0, img.w as isize - 1) as usize;
                img.get(x, y) as f64
            })
            .sum();
        for x in 0..img.w {
            tmp.set(x, y, (sum / n) as u8);
            let out = (x as isize - r as isize).clamp(0, img.w as isize - 1) as usize;
            let inn = ((x + r + 1) as isize).clamp(0, img.w as isize - 1) as usize;
            sum += img.get(inn, y) as f64 - img.get(out, y) as f64;
        }
    }
    let mut out = Gray::new(img.w, img.h, 255);
    for x in 0..img.w {
        let mut sum: f64 = (0..=2 * r)
            .map(|k| {
                let y = (k as isize - r as isize).clamp(0, img.h as isize - 1) as usize;
                tmp.get(x, y) as f64
            })
            .sum();
        for y in 0..img.h {
            out.set(x, y, (sum / n) as u8);
            let o = (y as isize - r as isize).clamp(0, img.h as isize - 1) as usize;
            let i = ((y + r + 1) as isize).clamp(0, img.h as isize - 1) as usize;
            sum += tmp.get(x, i) as f64 - tmp.get(x, o) as f64;
        }
    }
    out
}

/// Ink dilation (positive) or erosion (negative) by a disc of the given radius.
fn morph(img: &Gray, radius: f64) -> Gray {
    let r = radius.abs().round() as isize;
    if r < 1 {
        return img.clone();
    }
    let dilate = radius > 0.0;
    let mut out = Gray::new(img.w, img.h, 255);
    for y in 0..img.h {
        for x in 0..img.w {
            let mut v = if dilate { 255u8 } else { 0u8 };
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx * dx + dy * dy > r * r {
                        continue;
                    }
                    let nx = (x as isize + dx).clamp(0, img.w as isize - 1) as usize;
                    let ny = (y as isize + dy).clamp(0, img.h as isize - 1) as usize;
                    let s = img.get(nx, ny);
                    v = if dilate { v.min(s) } else { v.max(s) };
                }
            }
            out.set(x, y, v);
        }
    }
    out
}

/// Rotation, scale and perspective in one resample.
fn warp(img: &Gray, d: &Degradation, _rng: &mut Rng) -> Gray {
    use crate::geom::{Homography, Point};
    let (w, h) = (img.w as f64, img.h as f64);
    let s = 1.0 + d.scale;
    let th = d.rotate_deg.to_radians();
    let (c, sn) = (th.cos(), th.sin());
    let (cx, cy) = (w / 2.0, h / 2.0);
    let e = d.perspective * w;

    let src = [
        Point::new(0.0, 0.0),
        Point::new(w, 0.0),
        Point::new(0.0, h),
        Point::new(w, h),
    ];
    let dst: [Point; 4] = std::array::from_fn(|i| {
        let p = src[i];
        let (dx, dy) = (p.x - cx, p.y - cy);
        let rx = cx + s * (dx * c - dy * sn);
        let ry = cy + s * (dx * sn + dy * c);
        let sign = if i == 0 || i == 3 { 1.0 } else { -1.0 };
        Point::new(rx + sign * e, ry - sign * e * 0.5)
    });
    // Grow the canvas to hold the transformed page. Rotating inside a fixed
    // canvas silently crops the corner fiducials, which would make this measure
    // the harness rather than the decoder.
    let (mut lo_x, mut lo_y, mut hi_x, mut hi_y) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in dst.iter() {
        lo_x = lo_x.min(p.x);
        lo_y = lo_y.min(p.y);
        hi_x = hi_x.max(p.x);
        hi_y = hi_y.max(p.y);
    }
    let pad_x = (-lo_x).max(0.0).ceil();
    let pad_y = (-lo_y).max(0.0).ceil();
    let out_w = ((hi_x + pad_x).ceil() as usize).clamp(img.w, img.w * 3);
    let out_h = ((hi_y + pad_y).ceil() as usize).clamp(img.h, img.h * 3);
    let dst: [Point; 4] = std::array::from_fn(|i| Point::new(dst[i].x + pad_x, dst[i].y + pad_y));

    let fwd = match Homography::from_four(&src, &dst) {
        Some(f) => f,
        None => return img.clone(),
    };
    let inv = match fwd.invert() {
        Some(i) => i,
        None => return img.clone(),
    };
    let mut out = Gray::new(out_w, out_h, 255);
    for y in 0..out_h {
        for x in 0..out_w {
            let p = inv.apply(Point::new(x as f64 + 0.5, y as f64 + 0.5));
            let v = if p.x < 0.0 || p.y < 0.0 || p.x >= w || p.y >= h {
                255.0
            } else {
                img.sample(p.x - 0.5, p.y - 0.5)
            };
            out.set(x, y, v.clamp(0.0, 255.0) as u8);
        }
    }
    out
}

// ---------------------------------------------------------------- colour

use crate::bitmap::{Rgb, Scan};

/// Apply a degradation to a scan, in whichever ink mode it is.
pub fn apply_scan(scan: &Scan, d: &Degradation, cell_px: f64) -> Scan {
    apply_scan_masked(scan, None, d, cell_px)
}

/// As `apply_scan`, told where black ink is so that ink fade leaves it alone.
pub fn apply_scan_masked(scan: &Scan, black: Option<&Gray>, d: &Degradation, cell_px: f64) -> Scan {
    match &scan.rgb {
        None => Scan::grey(apply(&scan.luma, d, cell_px)),
        Some(c) => {
            let dirty = apply_colour(c, black, d, cell_px);
            if d.greyscale {
                // What a colour archive scanned in mono actually looks like: the
                // three ink planes summed into one channel, unrecoverably.
                Scan::grey(dirty.to_luma())
            } else {
                Scan::colour(dirty)
            }
        }
    }
}

/// Colour degradations, then the shared geometric and optical ones per channel.
fn apply_colour(src: &Rgb, black: Option<&Gray>, d: &Degradation, cell_px: f64) -> Rgb {
    let mut planes: Vec<Gray> = (0..3)
        .map(|ch| {
            let mut g = Gray::new(src.w, src.h, 255);
            for i in 0..src.w * src.h {
                g.px[i] = src.px[i * 3 + ch];
            }
            g
        })
        .collect();

    for ch in 0..3 {
        // Ink fade: the paper shows through more, so the channel lightens.
        let fade = d.plane_fade[ch].clamp(0.0, 1.0);
        if fade > 0.0 {
            for (i, v) in planes[ch].px.iter_mut().enumerate() {
                if black.is_some_and(|b| b.px[i] == 0) {
                    continue; // carbon black is a different ink and does not fade with this one
                }
                *v = (255.0 - (255.0 - *v as f64) * (1.0 - fade)).round() as u8;
            }
        }
        // Registration: one ink lands offset from the others.
        let off = d.reg_offset_cells[ch] * cell_px;
        if off.abs() > 0.01 {
            planes[ch] = shift(&planes[ch], off, off * 0.4);
        }
    }

    // Real inks are not ideal: cyan absorbs a little green and blue, and so on.
    if d.crosstalk > 0.0 {
        let k = d.crosstalk.clamp(0.0, 0.6);
        let dens: Vec<Vec<f64>> = planes
            .iter()
            .map(|g| g.px.iter().map(|&v| 1.0 - v as f64 / 255.0).collect())
            .collect();
        for ch in 0..3 {
            for i in 0..planes[ch].px.len() {
                let bleed: f64 = (0..3).filter(|&o| o != ch).map(|o| dens[o][i]).sum();
                let total = (dens[ch][i] + k * bleed).min(1.0);
                planes[ch].px[i] = (255.0 * (1.0 - total)).round() as u8;
            }
        }
    }

    let mut per_channel: Vec<Gray> = Vec::with_capacity(3);
    for (ch, g) in planes.iter().enumerate() {
        let mut sub = d.clone();
        // Geometry must be identical across channels or the planes tear apart.
        sub.seed = d.seed;
        if ch == 2 && d.blue_noise > 0.0 {
            sub.noise = (sub.noise.powi(2) + d.blue_noise.powi(2)).sqrt();
        }
        let mut done = apply(g, &sub, cell_px);
        let cast = d.colour_cast[ch];
        if cast != 0.0 {
            for v in done.px.iter_mut() {
                *v = (*v as f64 * (1.0 + cast)).clamp(0.0, 255.0) as u8;
            }
        }
        per_channel.push(done);
    }

    let (w, h) = (per_channel[0].w, per_channel[0].h);
    let mut out = Rgb::new(w, h, [255; 3]);
    for i in 0..w * h {
        for ch in 0..3 {
            out.px[i * 3 + ch] = per_channel[ch].px[i];
        }
    }
    out
}

/// Sub-pixel translation, for per-plane registration error.
fn shift(img: &Gray, dx: f64, dy: f64) -> Gray {
    let mut out = Gray::new(img.w, img.h, 255);
    for y in 0..img.h {
        for x in 0..img.w {
            let v = img.sample(x as f64 - dx, y as f64 - dy);
            out.set(x, y, v.clamp(0.0, 255.0) as u8);
        }
    }
    out
}
