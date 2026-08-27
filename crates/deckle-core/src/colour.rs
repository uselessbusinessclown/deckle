//! Colour mode: three binary ink planes (PLAN.md section 18, v1.1 "Chroma").
//!
//! Three bits per cell, not four. An RGB scanner takes three measurements, so it
//! cannot separate a fourth ink plane; carbon black is spectrally flat and sits
//! almost on top of a C+M+Y overprint. Black is therefore reserved for structure
//! — corner markers, sync marks, the descriptor strip — which also means page
//! location, orientation and descriptor reading all work before any colour
//! calibration exists, using exactly the same code as a mono page.
//!
//! Under the subtractive model each ink absorbs one primary, so the red channel
//! carries cyan, green carries magenta, and blue carries yellow.

use crate::bitmap::{Gray, Rgb};
use crate::geom::Point;
use crate::layout::*;
use crate::raster::{refine_dark, WHITEN_SEED_DATA};
use crate::rng::{Rng, Whitener};

/// Cell byte: bits 0..2 are cyan, magenta, yellow; bit 3 marks structural black.
pub const INK_C: u8 = 0b001;
pub const INK_M: u8 = 0b010;
pub const INK_Y: u8 = 0b100;
pub const INK_K: u8 = 0b1000;

/// Which scanner channel each ink absorbs in.
pub const PLANE_CHANNEL: [usize; 3] = [0, 1, 2];

/// Per-plane interleave offset, so the three planes do not share one spatial map.
pub fn plane_offset_pub(seed: u32, band: usize, plane: usize, cells: usize) -> u64 {
    plane_offset(seed, band, plane, cells)
}

fn plane_offset(seed: u32, band: usize, plane: usize, cells: usize) -> u64 {
    (seed as u64)
        .wrapping_add(band as u64 * 7919)
        .wrapping_add(plane as u64 * 104_729)
        % cells.max(1) as u64
}

// ------------------------------------------------------------------ encoding

/// Lay out one colour page's cells.
pub fn build_cells(
    geo: &PageGeometry,
    codewords: &[Vec<u8>],
    page_index: u16,
    seed: u32,
) -> Vec<u8> {
    assert_eq!(codewords.len(), geo.codewords);
    let (cols, rows, f, u) = (geo.cols, geo.rows, geo.fid_cells, geo.fid_unit);
    let mut cells = vec![0u8; cols * rows];

    crate::raster::draw_structure(&mut cells, geo, INK_K);
    for (i, &(sx, sy)) in geo.reg_strips().iter().enumerate() {
        let _ = i;
        for p in 0..3 {
            // A solid two-unit square inside a three-unit slot, so the mark keeps
            // a white border and its centroid is unambiguous.
            let ink = 1u8 << p;
            let (ox, oy) = reg_mark_origin(sx, sy, u, p);
            for dy in 0..2 * u {
                for dx in 0..2 * u {
                    let (x, y) = (ox + dx, oy + dy);
                    if x < cols && y < rows {
                        cells[y * cols + x] = ink;
                    }
                }
            }
        }
    }
    for &(px, py, state) in &geo.cal_patches {
        for dy in 0..CAL_BLOCK {
            for dx in 0..CAL_BLOCK {
                cells[(py + dy) * cols + px + dx] = state & 0b111;
            }
        }
    }

    let mut wh = Whitener::new(WHITEN_SEED_DATA ^ page_index as u64);
    let mut filler = Rng::new(0xF111_E7C0 ^ page_index as u64);
    for (bi, band) in geo.bands.iter().enumerate() {
        let a = choose_interleave_a(band.cells) as u64;
        let per_plane = band.codewords / 3;
        let used = per_plane * RS_N * 8;
        let mut i: usize = 0;
        for y in band.row0..band.row1 {
            for x in 0..cols {
                if is_reserved_ink(cols, rows, f, u, InkPlanes::Cmy, x, y) {
                    continue;
                }
                let mut v = 0u8;
                for p in 0..3 {
                    let off = plane_offset(seed, bi, p, band.cells);
                    let pp = ((a * i as u64 + off) % band.cells as u64) as usize;
                    let bit = if pp < used && per_plane > 0 {
                        let cw = band.first_cw + p * per_plane + pp % per_plane;
                        let bi2 = pp / per_plane;
                        codewords[cw][bi2 / 8] >> (7 - (bi2 % 8)) & 1 != 0
                    } else {
                        filler.next_bool()
                    };
                    if bit ^ wh.next_bit() {
                        v |= 1 << p;
                    }
                }
                cells[y * cols + x] = v;
                i += 1;
            }
        }
        debug_assert_eq!(i, band.cells);
    }
    cells
}

/// Ideal subtractive rendering: cyan removes red, magenta green, yellow blue.
#[inline]
pub fn cell_rgb(v: u8) -> [u8; 3] {
    if v & INK_K != 0 {
        return [0, 0, 0];
    }
    [
        if v & INK_C != 0 { 0 } else { 255 },
        if v & INK_M != 0 { 0 } else { 255 },
        if v & INK_Y != 0 { 0 } else { 255 },
    ]
}

pub fn render(geo: &PageGeometry, cells: &[u8], strip: &[bool]) -> Rgb {
    render_masked(geo, cells, strip).0
}

/// Render, and report where black ink was laid down.
///
/// A scanner cannot tell K from a C+M+Y overprint, and neither can this image -
/// but the *degradation* harness must, because black ink does not fade when
/// magenta does. Without the mask, modelling a dead magenta plane also erases
/// the corner markers, and the page stops being locatable for the wrong reason.
pub fn render_masked(geo: &PageGeometry, cells: &[u8], strip: &[bool]) -> (Rgb, Gray) {
    let dpi = geo.render_dpi as f64;
    let mm2px = |mm: f64| mm * dpi / 25.4;
    let w = mm2px(geo.page_w_mm).round() as usize;
    let h = mm2px(geo.page_h_mm).round() as usize;
    let mut img = Rgb::new(w, h, [255; 3]);
    let mut kmask = Gray::new(w, h, 255);

    let gx = mm2px(geo.grid_x_mm).round() as usize;
    let gy = mm2px(geo.grid_y_mm).round() as usize;
    let cd = geo.cell_dots as usize;
    for y in 0..geo.rows {
        for x in 0..geo.cols {
            let v = cells[y * geo.cols + x];
            if v == 0 {
                continue;
            }
            let rgb = cell_rgb(v);
            let k = v & INK_K != 0;
            for py in gy + y * cd..(gy + (y + 1) * cd).min(h) {
                for px in gx + x * cd..(gx + (x + 1) * cd).min(w) {
                    img.set(px, py, rgb);
                    if k {
                        kmask.set(px, py, 0);
                    }
                }
            }
        }
    }
    // The descriptor strip is black only, always (PLAN.md 18.7).
    let ds = geo.desc_cell_mm();
    let left = geo.desc_left_mm();
    let top = geo.desc_top_mm();
    for y in 0..DESC_BLOCK_ROWS {
        for x in 0..DESC_BLOCK_COLS {
            if !strip[y * DESC_BLOCK_COLS + x] {
                continue;
            }
            let x0 = mm2px(left + x as f64 * ds).round() as usize;
            let x1 = (mm2px(left + (x + 1) as f64 * ds).round() as usize).min(w);
            let y0 = mm2px(top + y as f64 * ds).round() as usize;
            let y1 = (mm2px(top + (y + 1) as f64 * ds).round() as usize).min(h);
            for py in y0..y1 {
                for px in x0..x1 {
                    img.set(px, py, [0, 0, 0]);
                    kmask.set(px, py, 0);
                }
            }
        }
    }
    (img, kmask)
}

// ------------------------------------------------------------------ decoding

/// Paper white per channel, on a coarse grid.
///
/// An illumination gradient scales the whole neighbourhood, so normalising by a
/// local white level turns it into a constant that the ink model absorbs. The
/// estimate is the brightest sample in each tile: white cells are common enough
/// that every tile has one, and any bias cancels because the ink matrix is fitted
/// through the same normalisation.
pub struct WhiteMap {
    tw: usize,
    th: usize,
    tile: usize,
    v: Vec<[f32; 3]>,
    mean: Vec<[f32; 3]>,
}

impl WhiteMap {
    pub fn new(img: &Rgb, tile: usize) -> WhiteMap {
        let tw = img.w.div_ceil(tile);
        let th = img.h.div_ceil(tile);
        let mut v = vec![[1.0f32; 3]; tw * th];
        let mut sum = vec![[0.0f64; 3]; tw * th];
        let mut n = vec![0.0f64; tw * th];
        for y in (0..img.h).step_by(2) {
            let row = (y / tile) * tw;
            for x in (0..img.w).step_by(2) {
                let p = img.get(x, y);
                let i = row + x / tile;
                n[i] += 1.0;
                for c in 0..3 {
                    if p[c] as f32 > v[i][c] {
                        v[i][c] = p[c] as f32;
                    }
                    sum[i][c] += p[c] as f64;
                }
            }
        }
        let mean = (0..tw * th)
            .map(|i| {
                let k = n[i].max(1.0);
                std::array::from_fn(|c| (sum[i][c] / k) as f32)
            })
            .collect();
        WhiteMap {
            tw,
            th,
            tile,
            v,
            mean,
        }
    }
    pub fn at(&self, x: f64, y: f64, ch: usize) -> f64 {
        self.lookup(&self.v, x, y, ch).max(1.0)
    }

    /// Local mean channel value - the level a cell is bright or dark *relative
    /// to*. Blur leaves this alone while shrinking the swing around it, which is
    /// why thresholding here and not at a fixed level survives a soft scan.
    pub fn mean_at(&self, x: f64, y: f64, ch: usize) -> f64 {
        self.lookup(&self.mean, x, y, ch).max(1.0)
    }

    fn lookup(&self, src: &[[f32; 3]], x: f64, y: f64, ch: usize) -> f64 {
        let gx = (x / self.tile as f64 - 0.5).clamp(0.0, self.tw as f64 - 1.0);
        let gy = (y / self.tile as f64 - 0.5).clamp(0.0, self.th as f64 - 1.0);
        let (x0, y0) = (gx.floor() as usize, gy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.tw - 1), (y0 + 1).min(self.th - 1));
        let (fx, fy) = (gx - x0 as f64, gy - y0 as f64);
        let g = |xi: usize, yi: usize| src[yi * self.tw + xi][ch] as f64;
        (g(x0, y0) * (1.0 - fx) + g(x1, y0) * fx) * (1.0 - fy)
            + (g(x0, y1) * (1.0 - fx) + g(x1, y1) * fx) * fy
    }
}

#[inline]
fn density(sample: f64, white: f64) -> f64 {
    -((sample.max(1.0) / white).min(1.0)).ln()
}

/// The fitted colour model for one page.
pub struct Calibration {
    /// Inverse of the 3x3 ink density matrix: density vector -> (c, m, y).
    inv: [[f64; 3]; 3],
    pub white: WhiteMap,
    pub patches: usize,
    /// Planes with no measurable ink left. Their bits are unreadable, so they
    /// are reported with zero confidence and rebuilt from cross-block parity -
    /// which is the whole reason a plane owns its codewords outright.
    pub dead: [bool; 3],
}

/// Fit the ink model from the calibration lattice by least squares.
///
/// Each patch prints a known combination of inks, so the page carries its own
/// answer to "what does cyan look like here, today, on this paper, through this
/// scanner". The patches fade with the data, which is the whole point.
pub fn calibrate(
    img: &Rgb,
    geo_cols: usize,
    patches: &[(usize, usize, u8)],
    plane_pos: &dyn Fn(usize, f64, f64) -> Point,
    white: WhiteMap,
    cell_px: f64,
) -> Option<Calibration> {
    let _ = geo_cols;
    // Normal equations for D minimising |d - D s|^2 over patches.
    let mut sts = [[0.0f64; 3]; 3];
    let mut dst = [[0.0f64; 3]; 3];
    let mut n = 0usize;
    for &(px, py, state) in patches {
        let s = [
            (state & 1) as f64,
            (state >> 1 & 1) as f64,
            (state >> 2 & 1) as f64,
        ];
        let mut d = [0.0f64; 3];
        for c in 0..3 {
            let p = plane_pos(c, px as f64 + 2.0, py as f64 + 2.0);
            // The patch is four cells across, so average well inside it.
            let o = cell_px * 0.8;
            let v = (img.sample(c, p.x, p.y)
                + img.sample(c, p.x - o, p.y - o)
                + img.sample(c, p.x + o, p.y - o)
                + img.sample(c, p.x - o, p.y + o)
                + img.sample(c, p.x + o, p.y + o))
                / 5.0;
            d[c] = density(v, white.at(p.x, p.y, c));
        }
        for i in 0..3 {
            for j in 0..3 {
                sts[i][j] += s[i] * s[j];
                dst[i][j] += d[i] * s[j];
            }
        }
        n += 1;
    }
    if n < 8 {
        return None;
    }
    let sts_inv = invert3(&sts)?;
    // D = dst * sts^-1
    let mut dmat = [[0.0f64; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            dmat[i][j] = (0..3).map(|k| dst[i][k] * sts_inv[k][j]).sum();
        }
    }
    // A plane that has faded to nothing leaves a near-zero column, which would
    // make the matrix singular and lose the two planes that are still fine. Give
    // it a nominal column instead and mark it dead: the other planes decode, and
    // this one's third of the blocks goes to parity.
    let norms: [f64; 3] =
        std::array::from_fn(|j| (0..3).map(|i| dmat[i][j] * dmat[i][j]).sum::<f64>().sqrt());
    let strongest = norms.iter().cloned().fold(0.0f64, f64::max);
    let mut dead = [false; 3];
    for j in 0..3 {
        if strongest <= 1e-6 || norms[j] < strongest * 0.25 {
            dead[j] = true;
            for i in 0..3 {
                dmat[i][j] = if i == j { strongest.max(1.0) } else { 0.0 };
            }
        }
    }
    Some(Calibration {
        inv: invert3(&dmat)?,
        white,
        patches: n,
        dead,
    })
}

impl Calibration {
    /// Estimated ink coverage from a density vector.
    #[inline]
    pub fn solve(&self, d: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|i| (0..3).map(|j| self.inv[i][j] * d[j]).sum())
    }

    /// Decide each plane against a locally adaptive threshold.
    ///
    /// `d` is the cell's density vector, `d_mean` the neighbourhood's. Whitened
    /// payload puts each ink on about half the cells, so the local mean sits
    /// between the two states and tracks anything that shrinks the swing.
    #[inline]
    pub fn decide(&self, d: [f64; 3], d_mean: [f64; 3]) -> ([bool; 3], [f64; 3]) {
        let est = self.solve(d);
        let thr = self.solve(d_mean);
        let mut bits = [false; 3];
        let mut conf = [0.0f64; 3];
        for i in 0..3 {
            let t = thr[i].clamp(0.25, 0.75);
            bits[i] = est[i] > t;
            conf[i] = if self.dead[i] {
                0.0
            } else {
                ((est[i] - t).abs() * 2.5).min(1.0)
            };
        }
        (bits, conf)
    }
    /// Ink density at a cell centre, averaged over the middle of the cell.
    ///
    /// A single sample is far too sensitive to sub-pixel placement: it left a
    /// clean colour page consuming its whole correction budget. The mono path
    /// has always averaged an aperture; colour needs it more, not less, because
    /// three bits ride on each cell.
    /// Density of the local mean, for the adaptive threshold.
    #[inline]
    pub fn mean_density_at(&self, ch: usize, p: Point) -> f64 {
        density(
            self.white.mean_at(p.x, p.y, ch),
            self.white.at(p.x, p.y, ch),
        )
    }

    #[inline]
    pub fn density_at(&self, img: &Rgb, ch: usize, p: Point, cell_px: f64) -> f64 {
        let o = cell_px * 0.13;
        let v = (img.sample(ch, p.x, p.y)
            + img.sample(ch, p.x - o, p.y - o)
            + img.sample(ch, p.x + o, p.y - o)
            + img.sample(ch, p.x - o, p.y + o)
            + img.sample(ch, p.x + o, p.y + o))
            / 5.0;
        density(v, self.white.at(p.x, p.y, ch))
    }
}

fn invert3(m: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-9 {
        return None;
    }
    let c = |a: usize, b: usize, x: usize, y: usize| m[a][b] * m[x][y];
    let mut o = [[0.0f64; 3]; 3];
    o[0][0] = (c(1, 1, 2, 2) - c(1, 2, 2, 1)) / det;
    o[0][1] = (c(0, 2, 2, 1) - c(0, 1, 2, 2)) / det;
    o[0][2] = (c(0, 1, 1, 2) - c(0, 2, 1, 1)) / det;
    o[1][0] = (c(1, 2, 2, 0) - c(1, 0, 2, 2)) / det;
    o[1][1] = (c(0, 0, 2, 2) - c(0, 2, 2, 0)) / det;
    o[1][2] = (c(0, 2, 1, 0) - c(0, 0, 1, 2)) / det;
    o[2][0] = (c(1, 0, 2, 1) - c(1, 1, 2, 0)) / det;
    o[2][1] = (c(0, 1, 2, 0) - c(0, 0, 2, 1)) / det;
    o[2][2] = (c(0, 0, 1, 1) - c(0, 1, 1, 0)) / det;
    Some(o)
}

/// Per-plane registration, as a correction on top of the warped black geometry.
///
/// Ink planes do not land on top of each other: a colour laser exposes each from
/// a separate drum. Most of that error is systematic — an offset with a little
/// scale and rotation — so it is measurable and removable (PLAN.md 18.4).
///
/// It has to be expressed as a *delta* from the already-warped black position,
/// not as a homography of its own. Fitting a plane transform straight to the
/// mark positions makes it absorb the local sync warp near the corners, and
/// adding the warp again then double-counts it — which drifted the sampling
/// point by a third of a cell by the bottom of the page and put the cell-bit
/// error rate into double figures.
pub struct PlaneWarp {
    /// Per plane, the four corner deltas in pixels, in TL, TR, BL, BR order.
    delta: [[(f64, f64); 4]; 3],
    x0: f64,
    x1: f64,
    y0: [f64; 3],
    y1: [f64; 3],
}

impl PlaneWarp {
    /// Mean measured offset per plane, in cells, for reporting.
    pub fn mean_offset_cells(&self, cell_px: f64) -> [f64; 3] {
        std::array::from_fn(|p| {
            self.delta[p]
                .iter()
                .map(|d| (d.0 * d.0 + d.1 * d.1).sqrt())
                .sum::<f64>()
                / 4.0
                / cell_px.max(1e-9)
        })
    }

    /// Bilinear over the four marks, in cell coordinates.
    #[inline]
    pub fn at(&self, plane: usize, cx: f64, cy: f64) -> (f64, f64) {
        let s = ((cx - self.x0) / (self.x1 - self.x0)).clamp(-0.2, 1.2);
        let t = ((cy - self.y0[plane]) / (self.y1[plane] - self.y0[plane])).clamp(-0.2, 1.2);
        let d = &self.delta[plane];
        let top = (
            d[0].0 * (1.0 - s) + d[1].0 * s,
            d[0].1 * (1.0 - s) + d[1].1 * s,
        );
        let bot = (
            d[2].0 * (1.0 - s) + d[3].0 * s,
            d[2].1 * (1.0 - s) + d[3].1 * s,
        );
        (top.0 * (1.0 - t) + bot.0 * t, top.1 * (1.0 - t) + bot.1 * t)
    }
}

/// `warped(cx, cy)` must give the black-ink image position of a cell centre,
/// sync correction included.
pub fn plane_warp(
    img: &Rgb,
    warped: &dyn Fn(f64, f64) -> Point,
    cols: usize,
    rows: usize,
    f: usize,
    unit: usize,
    cell_px: f64,
) -> Option<PlaneWarp> {
    let strips = reg_strips_for(cols, rows, f, unit);
    let mut delta = [[(0.0, 0.0); 4]; 3];
    let mut y0 = [0.0f64; 3];
    let mut y1 = [0.0f64; 3];
    let (mut x0, mut x1) = (0.0f64, 0.0f64);
    for p in 0..3 {
        let ch = PLANE_CHANNEL[p];
        for (k, &(sx, sy)) in strips.iter().enumerate() {
            let (cx, cy) = reg_mark_centre(sx, sy, unit, p);
            let pred = warped(cx, cy);
            // Two passes: a wide window to find it, then a tight one centred on
            // that estimate, so nothing outside the mark's white surround can
            // drag the centroid.
            let found = refine_dark_channel(img, ch, pred, cell_px * unit as f64 * 1.6)
                .and_then(|rough| refine_dark_channel(img, ch, rough, cell_px * unit as f64 * 1.15))
                .unwrap_or(pred);
            delta[p][k] = (found.x - pred.x, found.y - pred.y);
            if p == 0 && k == 0 {
                x0 = cx;
            }
            if p == 0 && k == 1 {
                x1 = cx;
            }
            if k == 0 {
                y0[p] = cy;
            }
            if k == 2 {
                y1[p] = cy;
            }
        }
    }
    // At each corner, subtract what all three planes have in common.
    //
    // The marks sit in the corner bands, where the sync lattice is sparse, so
    // the warped prediction there carries some extrapolation error. That error
    // is identical for all three inks, and it is *local to the corner* - feeding
    // it into a bilinear that spans the page spreads a corner's noise over
    // everything. At six device dots per cell that cost 0.08 cells and did not
    // matter; at four it cost 0.23 and broke two planes outright. What is left
    // after the subtraction is the quantity actually wanted: how far each ink
    // lands from the other two.
    for k in 0..4 {
        let cx = (0..3).map(|p| delta[p][k].0).sum::<f64>() / 3.0;
        let cy = (0..3).map(|p| delta[p][k].1).sum::<f64>() / 3.0;
        for p in 0..3 {
            delta[p][k].0 -= cx;
            delta[p][k].1 -= cy;
        }
    }

    if (x1 - x0).abs() < 1.0 {
        return None;
    }
    for p in 0..3 {
        if (y1[p] - y0[p]).abs() < 1.0 {
            return None;
        }
    }
    Some(PlaneWarp {
        delta,
        x0,
        x1,
        y0,
        y1,
    })
}

fn refine_dark_channel(img: &Rgb, ch: usize, at: Point, radius: f64) -> Option<Point> {
    let mut g = Gray::new(0, 0, 0);
    let r = radius.max(2.0);
    let x0 = ((at.x - r).floor().max(0.0)) as usize;
    let y0 = ((at.y - r).floor().max(0.0)) as usize;
    let x1 = ((at.x + r).ceil() as isize).clamp(0, img.w as isize) as usize;
    let y1 = ((at.y + r).ceil() as isize).clamp(0, img.h as isize) as usize;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    g.w = x1 - x0;
    g.h = y1 - y0;
    g.px = Vec::with_capacity(g.w * g.h);
    for y in y0..y1 {
        for x in x0..x1 {
            g.px.push(img.get(x, y)[ch]);
        }
    }
    let local = refine_dark(&g, Point::new(at.x - x0 as f64, at.y - y0 as f64), r)?;
    Some(Point::new(local.x + x0 as f64, local.y + y0 as f64))
}
