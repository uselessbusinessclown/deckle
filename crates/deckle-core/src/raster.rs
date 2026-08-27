//! The native raster symbology: cell layout, rendering, and decoding.
//!
//! Page anatomy (PLAN.md section 5.2), all of it black-only:
//!
//! ```text
//!   +--------------------------------------------------+
//!   |  header band: descriptor strip                   |
//!   +--------------------------------------------------+
//!   | [F]                                          [F] |
//!   |      data cells, sync marks every 32 cells        |
//!   | [F]                                          [B] |
//!   +--------------------------------------------------+
//! ```
//!
//! Three identical finder patterns and one distinct bottom-right marker resolve
//! rotation; a mirrored page is caught by the descriptor failing to decode and
//! retried flipped.

use crate::bitmap::{Gray, Integral, Scan};
use crate::block::{decode_codeword, Block, BlockDecode, BlockError, FILLER_INDEX, FLAG_FILLER};
use crate::colour::{self, PLANE_CHANNEL};
use crate::descriptor::Descriptor;
use crate::geom::{Homography, Point};
use crate::gf256::{rs_decode, rs_encode_parity};
use crate::layout::*;
use crate::rng::{Rng, Whitener};

pub const WHITEN_SEED_DESC: u64 = 0x0DEC_1E5C_0DE5_C001;
pub const WHITEN_SEED_DATA: u64 = 0x0DEC_1E5D_A7A0_0001;
const SAUVOLA_K: f64 = 0.2;
const SAUVOLA_R: f64 = 128.0;

// ---------------------------------------------------------------- encoding

/// Lay out one page's cells. `codewords` must hold `geo.codewords` entries of
/// 255 symbols each.
pub fn build_cells(
    geo: &PageGeometry,
    codewords: &[Vec<u8>],
    page_index: u16,
    seed: u32,
) -> Vec<bool> {
    assert_eq!(codewords.len(), geo.codewords);
    let (cols, rows, f) = (geo.cols, geo.rows, geo.fid_cells);
    let mut cells = vec![false; cols * rows];

    // Structure first; payload flows around it.
    draw_finders(&mut cells, cols, rows, f, geo.fid_unit);
    for &(bx, by) in &geo.sync_marks {
        draw_sync(&mut cells, cols, bx, by);
    }

    let mut wh = Whitener::new(WHITEN_SEED_DATA ^ page_index as u64);
    let mut filler = Rng::new(0xF111_E700 ^ page_index as u64);
    for (bi_idx, band) in geo.bands.iter().enumerate() {
        let (ba, bb) = band_interleave(band, bi_idx, seed);
        let used = band.codewords * RS_N * 8;
        let mut p: usize = 0;
        for y in band.row0..band.row1 {
            for x in 0..cols {
                if is_reserved_at(cols, rows, f, x, y) {
                    continue;
                }
                let pp = ((ba * p as u64 + bb) % band.cells as u64) as usize;
                let bit = if pp < used && band.codewords > 0 {
                    let cw = band.first_cw + pp % band.codewords;
                    let bit_i = pp / band.codewords;
                    codewords[cw][bit_i / 8] >> (7 - (bit_i % 8)) & 1 != 0
                } else {
                    // Cells past the band's last whole codeword carry noise, so
                    // the page stays uniform for the adaptive threshold.
                    filler.next_bool()
                };
                cells[y * cols + x] = bit ^ wh.next_bit();
                p += 1;
            }
        }
        debug_assert_eq!(p, band.cells);
    }
    cells
}

fn draw_finders(cells: &mut [bool], cols: usize, rows: usize, f: usize, u: usize) {
    let corners = [
        (0usize, 0usize),
        (cols - f, 0),
        (0, rows - f),
        (cols - f, rows - f),
    ];
    for (i, &(ox, oy)) in corners.iter().enumerate() {
        for uy in 0..9 {
            for ux in 0..9 {
                let on = if i == 3 {
                    // Distinct bottom-right marker: a solid five-unit square.
                    (2..7).contains(&ux) && (2..7).contains(&uy)
                } else {
                    // Seven-unit finder inside a one-unit white ring.
                    if !(1..8).contains(&ux) || !(1..8).contains(&uy) {
                        false
                    } else {
                        let (fx, fy) = (ux - 1, uy - 1);
                        fx == 0
                            || fx == 6
                            || fy == 0
                            || fy == 6
                            || ((2..=4).contains(&fx) && (2..=4).contains(&fy))
                    }
                };
                if on {
                    for dy in 0..u {
                        for dx in 0..u {
                            let (x, y) = (ox + ux * u + dx, oy + uy * u + dy);
                            cells[y * cols + x] = true;
                        }
                    }
                }
            }
        }
    }
}

/// Paint the black structure - finders and sync marks - into a cell buffer that
/// uses one byte per cell. Colour pages share exactly this structure, which is
/// what lets page location and orientation run before any colour calibration.
pub(crate) fn draw_structure(cells: &mut [u8], geo: &PageGeometry, ink: u8) {
    let mut tmp = vec![false; geo.cols * geo.rows];
    draw_finders(&mut tmp, geo.cols, geo.rows, geo.fid_cells, geo.fid_unit);
    for &(bx, by) in &geo.sync_marks {
        draw_sync(&mut tmp, geo.cols, bx, by);
    }
    for (i, on) in tmp.iter().enumerate() {
        if *on {
            cells[i] = ink;
        }
    }
}

fn draw_sync(cells: &mut [bool], cols: usize, bx: usize, by: usize) {
    for dy in 1..3 {
        for dx in 1..3 {
            cells[(by + dy) * cols + bx + dx] = true;
        }
    }
}

/// Descriptor strip cells, as a `DESC_BLOCK_COLS x DESC_BLOCK_ROWS` grid.
pub fn build_descriptor_strip(desc: &Descriptor) -> Vec<bool> {
    let msg = desc.to_message();
    let mut cw = msg.clone();
    cw.extend(rs_encode_parity(&msg, RS_N - DESC_RS_K));
    assert_eq!(cw.len(), RS_N);

    let mut g = vec![false; DESC_BLOCK_COLS * DESC_BLOCK_ROWS];
    // Corner markers.
    for &(mx, my) in &[
        (0usize, 0usize),
        (DESC_BLOCK_COLS - DESC_MARKER, 0),
        (0, DESC_BLOCK_ROWS - DESC_MARKER),
        (DESC_BLOCK_COLS - DESC_MARKER, DESC_BLOCK_ROWS - DESC_MARKER),
    ] {
        for y in my..my + DESC_MARKER {
            for x in mx..mx + DESC_MARKER {
                g[y * DESC_BLOCK_COLS + x] = true;
            }
        }
    }
    let mut wh = Whitener::new(WHITEN_SEED_DESC);
    for r in 0..DESC_ROWS {
        for c in 0..DESC_COLS {
            let i = r * DESC_COLS + c;
            let bit = cw[i / 8] >> (7 - (i % 8)) & 1 != 0;
            g[(r + DESC_MARKER) * DESC_BLOCK_COLS + c + DESC_MARKER] = bit ^ wh.next_bit();
        }
    }
    g
}

/// Filler block for the unused codewords of a partly-filled last page.
pub fn filler_block(ecc: Ecc, seed: u64) -> Block {
    let mut r = Rng::new(seed);
    Block {
        index: FILLER_INDEX,
        flags: FLAG_FILLER,
        payload: (0..ecc.payload()).map(|_| r.next_u32() as u8).collect(),
    }
}

// ---------------------------------------------------------------- rendering

/// Rasterise a page at the profile's nominal dpi.
///
/// Cell size is an integer number of device dots by construction, so this is the
/// 1:1 image-mask path of PLAN.md 4.1: no resampling happens anywhere.
pub fn render(geo: &PageGeometry, cells: &[bool], strip: &[bool]) -> Gray {
    let dpi = geo.render_dpi as f64;
    let mm2px = |mm: f64| mm * dpi / 25.4;
    let w = mm2px(geo.page_w_mm).round() as usize;
    let h = mm2px(geo.page_h_mm).round() as usize;
    let mut img = Gray::new(w, h, 255);

    let gx = mm2px(geo.grid_x_mm).round() as usize;
    let gy = mm2px(geo.grid_y_mm).round() as usize;
    let cd = geo.cell_dots as usize;
    for y in 0..geo.rows {
        for x in 0..geo.cols {
            if !cells[y * geo.cols + x] {
                continue;
            }
            for py in gy + y * cd..(gy + (y + 1) * cd).min(h) {
                for px in gx + x * cd..(gx + (x + 1) * cd).min(w) {
                    img.set(px, py, 0);
                }
            }
        }
    }

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
                    img.set(px, py, 0);
                }
            }
        }
    }
    img
}

// ---------------------------------------------------------------- decoding

#[derive(Debug)]
pub struct PageDecode {
    pub descriptor: Descriptor,
    /// Fraction of correction capacity consumed, per ink plane. Colour only.
    /// This is what makes a fading plane visible before it becomes loss.
    pub plane_margin: Option<[f64; 3]>,
    /// Mean measured registration offset per ink plane, in cells. Colour only.
    pub plane_registration: Option<[f64; 3]>,
    /// Ink planes whose density was too low to model - a plane that has faded
    /// away. Its codewords are handed to cross-block parity.
    pub dead_planes: Option<[bool; 3]>,
    pub blocks: Vec<BlockDecode>,
    /// Codewords on this page that no amount of retrying recovered.
    pub erased: usize,
    pub mirrored: bool,
    pub worst_margin: f64,
    pub mean_margin: f64,
    /// Residual of the sync-mark fit, in cells. High values mean bad geometry.
    pub geometry_residual: f64,
}

#[derive(Debug)]
pub enum DecodeError {
    NoFinders(usize),
    NoDescriptor,
    BadDescriptor(String),
    ScannedInGreyscale,
    ColourFitFailed(&'static str),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::NoFinders(n) => write!(
                f,
                "found {n} finder patterns, need 3; the page may be cropped, \
                 badly lit, or scanned at too low a resolution"
            ),
            DecodeError::NoDescriptor => write!(
                f,
                "could not read the descriptor strip in either orientation"
            ),
            DecodeError::BadDescriptor(s) => write!(f, "descriptor rejected: {s}"),
            DecodeError::ScannedInGreyscale => write!(
                f,
                "this is a colour archive but the scan has no colour in it. Rescan in \
                 colour, 24-bit RGB, with colour correction and auto-tone off. A \
                 greyscale scan sums the three ink planes together and cannot be undone."
            ),
            DecodeError::ColourFitFailed(s) => write!(
                f,
                "could not fit the colour model: {s}. The calibration patches or the \
                 registration marks were not readable; check the scan is in colour, in \
                 focus, and not colour-managed into a different space."
            ),
        }
    }
}

/// Decode a page from a greyscale scan. Colour archives need `decode_scan`.
pub fn decode_page(img: &Gray) -> Result<PageDecode, DecodeError> {
    decode_scan(&Scan::grey(img.clone()))
}

pub fn decode_scan(scan: &Scan) -> Result<PageDecode, DecodeError> {
    match decode_oriented(scan, false) {
        Ok(d) => Ok(d),
        Err(DecodeError::NoDescriptor) | Err(DecodeError::NoFinders(_)) => {
            // A mirrored page still shows three finders but the descriptor is
            // unreadable, so the flip retry is the cheapest disambiguation.
            decode_oriented(&scan.mirrored(), true)
        }
        Err(e) => Err(e),
    }
}

fn decode_oriented(scan: &Scan, mirrored: bool) -> Result<PageDecode, DecodeError> {
    let img = &scan.luma;
    let thr = otsu(img);
    let finders = find_finders(img, thr);
    if finders.len() < 3 {
        return Err(DecodeError::NoFinders(finders.len()));
    }
    let uv = [
        Point::new(0.0, 0.0),
        Point::new(1.0, 0.0),
        Point::new(0.0, 1.0),
        Point::new(1.0, 1.0),
    ];
    let integral = Integral::new(img);
    let mut found = None;
    for (tl, tr, bl, unit_px) in orient_candidates(&finders) {
        let br_pred = Point::new(tr.x + bl.x - tl.x, tr.y + bl.y - tl.y);
        let Some(br) = locate_br(img, &integral, br_pred, unit_px) else {
            continue;
        };
        let Some(h) = Homography::from_four(&uv, &[tl, tr, bl, br]) else {
            continue;
        };
        let span_x = tl.dist(tr);
        let span_y = tl.dist(bl);
        let aspect = if span_y > 0.0 { span_x / span_y } else { 1.0 };
        if let Some(d) = read_descriptor(img, &integral, &h, aspect, span_x, unit_px) {
            found = Some((d, h, span_x));
            break;
        }
    }
    let (desc, h_uv2img, span_x) = found.ok_or(DecodeError::NoDescriptor)?;
    let ecc = desc
        .ecc()
        .ok_or_else(|| DecodeError::BadDescriptor(format!("unknown RS(255,{})", desc.rs_k)))?;

    let (cols, rows, f) = (
        desc.grid_cols as usize,
        desc.grid_rows as usize,
        desc.fid_cells as usize,
    );
    if cols <= f || rows <= f || desc.sync_period as usize != SYNC_PERIOD {
        return Err(DecodeError::BadDescriptor("implausible grid".into()));
    }
    let ink = desc
        .ink()
        .ok_or(DecodeError::BadDescriptor("unknown ink planes".into()))?;
    let unit = f / 9;

    let cell_px = span_x / (cols - f) as f64;
    let cell_uv = |cx: f64, cy: f64| {
        Point::new(
            (cx - f as f64 / 2.0) / (cols - f) as f64,
            (cy - f as f64 / 2.0) / (rows - f) as f64,
        )
    };

    // Local warp: measure each sync mark's true position and interpolate between.
    let marks = sync_marks_ink(cols, rows, f, unit, ink);
    let nx = cols.div_ceil(SYNC_PERIOD);
    let ny = rows.div_ceil(SYNC_PERIOD);
    let mut disp = vec![None; nx * ny];
    let mut resid_sum = 0.0;
    let mut resid_n = 0usize;
    for &(bx, by) in &marks {
        let pred = h_uv2img.apply(cell_uv((bx + 2) as f64, (by + 2) as f64));
        if let Some(found) = refine_dark(img, pred, cell_px * 1.1) {
            let d = (found.x - pred.x, found.y - pred.y);
            let mag = (d.0 * d.0 + d.1 * d.1).sqrt() / cell_px.max(1e-9);
            if mag < 1.5 {
                disp[(by / SYNC_PERIOD) * nx + bx / SYNC_PERIOD] = Some(d);
                resid_sum += mag;
                resid_n += 1;
            }
        }
    }
    let geometry_residual = if resid_n > 0 {
        resid_sum / resid_n as f64
    } else {
        0.0
    };
    let lookup = |cx: f64, cy: f64| -> (f64, f64) {
        let gx = (cx / SYNC_PERIOD as f64).clamp(0.0, (nx - 1) as f64);
        let gy = (cy / SYNC_PERIOD as f64).clamp(0.0, (ny - 1) as f64);
        let (x0, y0) = (gx.floor() as usize, gy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(nx - 1), (y0 + 1).min(ny - 1));
        let (fx, fy) = (gx - x0 as f64, gy - y0 as f64);
        let mut acc = (0.0, 0.0);
        let mut wsum = 0.0;
        for (xi, yi, w) in [
            (x0, y0, (1.0 - fx) * (1.0 - fy)),
            (x1, y0, fx * (1.0 - fy)),
            (x0, y1, (1.0 - fx) * fy),
            (x1, y1, fx * fy),
        ] {
            if let Some(d) = disp[yi * nx + xi] {
                acc.0 += d.0 * w;
                acc.1 += d.1 * w;
                wsum += w;
            }
        }
        if wsum > 1e-6 {
            (acc.0 / wsum, acc.1 / wsum)
        } else {
            (0.0, 0.0)
        }
    };

    // Sample every payload cell, keeping the soft value the retry ladder needs.
    let band_rows = if desc.band_rows == 0 {
        BAND_ROWS
    } else {
        desc.band_rows as usize
    };
    let bands = bands_ink(cols, rows, f, unit, ink, band_rows);
    let n: usize = bands.iter().map(|b| b.codewords).sum();
    if n == 0 {
        return Err(DecodeError::BadDescriptor("no codewords fit".into()));
    }
    let mut cws = vec![vec![0u8; RS_N]; n];
    let mut conf = vec![vec![f32::MAX; RS_N]; n];
    let mut wh = Whitener::new(WHITEN_SEED_DATA ^ desc.page_index as u64);
    let half = (cell_px * 2.0).max(2.0);
    let planes = ink.count();

    // Colour mode needs a colour scan, per-plane geometry, and an ink model
    // fitted from the page's own calibration patches (PLAN.md 18.4-18.6).
    let colour_ctx = if ink != InkPlanes::K {
        let rgb = scan.rgb.as_ref().ok_or(DecodeError::ScannedInGreyscale)?;
        if rgb.channel_spread() < 4.0 {
            return Err(DecodeError::ScannedInGreyscale);
        }
        // Black geometry first, sync warp included; the ink planes are measured
        // as departures from it.
        let warped = |cx: f64, cy: f64| {
            let p = h_uv2img.apply(cell_uv(cx, cy));
            let d = lookup(cx - 0.5, cy - 0.5);
            Point::new(p.x + d.0, p.y + d.1)
        };
        let pw = colour::plane_warp(rgb, &warped, cols, rows, f, unit, cell_px)
            .ok_or(DecodeError::ColourFitFailed("registration marks not found"))?;
        let white = colour::WhiteMap::new(rgb, (cell_px * 8.0).max(16.0) as usize);
        let patches = cal_patches_for(cols, rows, f, unit, ink);
        let pos = |plane: usize, cx: f64, cy: f64| {
            let base = warped(cx, cy);
            let d = pw.at(plane, cx, cy);
            Point::new(base.x + d.0, base.y + d.1)
        };
        let cal = colour::calibrate(rgb, cols, &patches, &pos, white, cell_px)
            .ok_or(DecodeError::ColourFitFailed("calibration patches unusable"))?;
        Some((rgb, pw, cal))
    } else {
        None
    };

    for (bi_idx, band) in bands.iter().enumerate() {
        let per_plane = band.codewords / planes;
        let used = per_plane * RS_N * 8;
        let a = choose_interleave_a(band.cells) as u64;
        let (ba, bb) = band_interleave(band, bi_idx, desc.interleave_seed);
        let mut i: usize = 0;
        for y in band.row0..band.row1 {
            for x in 0..cols {
                if is_reserved_ink(cols, rows, f, unit, ink, x, y) {
                    continue;
                }
                let base = cell_uv(x as f64 + 0.5, y as f64 + 0.5);
                let d = lookup(x as f64, y as f64);
                let idx = i;
                i += 1;

                // Read every plane of this cell, then place the bits.
                let mut bits = [false; 3];
                let mut strengths = [0.0f32; 3];
                match &colour_ctx {
                    None => {
                        let p0 = h_uv2img.apply(base);
                        let (px, py) = (p0.x + d.0, p0.y + d.1);
                        let (v, sp) = sample_cell(img, px, py, cell_px);
                        let (m, sd) = integral.stats(px, py, half);
                        let t = m * (1.0 + SAUVOLA_K * (sd / SAUVOLA_R - 1.0));
                        bits[0] = v < t;
                        strengths[0] = ((v - t).abs() / sd.max(6.0)) as f32 - (sp / 64.0) as f32;
                    }
                    Some((rgb, pw, cal)) => {
                        let p0 = h_uv2img.apply(base);
                        let (bx, by) = (p0.x + d.0, p0.y + d.1);
                        let mut dens = [0.0f64; 3];
                        let mut dmean = [0.0f64; 3];
                        for p in 0..3 {
                            let (dx, dy) = pw.at(p, x as f64 + 0.5, y as f64 + 0.5);
                            let at = Point::new(bx + dx, by + dy);
                            dens[p] = cal.density_at(rgb, PLANE_CHANNEL[p], at, cell_px);
                            dmean[p] = cal.mean_density_at(PLANE_CHANNEL[p], at);
                        }
                        let (b, c) = cal.decide(dens, dmean);
                        bits = b;
                        for p in 0..3 {
                            strengths[p] = c[p] as f32;
                        }
                    }
                }

                for p in 0..planes {
                    let pp = if planes == 1 {
                        ((ba * idx as u64 + bb) % band.cells as u64) as usize
                    } else {
                        let off =
                            colour::plane_offset_pub(desc.interleave_seed, bi_idx, p, band.cells);
                        ((a * idx as u64 + off) % band.cells as u64) as usize
                    };
                    let bit = bits[p] ^ wh.next_bit();
                    if pp >= used || per_plane == 0 {
                        continue;
                    }
                    let cw = band.first_cw + p * per_plane + pp % per_plane;
                    let bit_i = pp / per_plane;
                    if bit {
                        cws[cw][bit_i / 8] |= 1 << (7 - (bit_i % 8));
                    }
                    let e = &mut conf[cw][bit_i / 8];
                    if strengths[p] < *e {
                        *e = strengths[p];
                    }
                }
            }
        }
    }

    // Retry ladder (PLAN.md 5.8): escalate erasure flagging, cheapest first.
    let nsym = ecc.nsym();
    let mut blocks = Vec::new();
    let mut erased = 0usize;
    let mut plane_worst = [0.0f64; 3];
    let plane_of = |cw: usize| -> usize {
        if planes == 1 {
            return 0;
        }
        for band in &bands {
            if cw >= band.first_cw && cw < band.first_cw + band.codewords {
                let per = band.codewords / planes;
                return ((cw - band.first_cw) / per.max(1)).min(2);
            }
        }
        0
    };
    for i in 0..n {
        let mut order: Vec<usize> = (0..RS_N).collect();
        order.sort_by(|&x, &y| conf[i][x].partial_cmp(&conf[i][y]).unwrap());
        let mut done = None;
        for &frac in &[0.0f64, 0.02, 0.05, 0.10, 0.20, 0.35] {
            let take = ((RS_N as f64 * frac).round() as usize).min(nsym);
            match decode_codeword(&cws[i], ecc, &order[..take]) {
                Ok(d) => {
                    done = Some(d);
                    break;
                }
                Err(BlockError::Rs) | Err(BlockError::Crc) => continue,
            }
        }
        match done {
            Some(d) => {
                let pl = plane_of(i);
                if d.margin > plane_worst[pl] {
                    plane_worst[pl] = d.margin;
                }
                if !d.block.is_filler() {
                    blocks.push(d);
                }
            }
            None => {
                erased += 1;
                plane_worst[plane_of(i)] = 1.0;
            }
        }
    }

    let worst = blocks.iter().map(|b| b.margin).fold(0.0f64, f64::max);
    let mean = if blocks.is_empty() {
        0.0
    } else {
        blocks.iter().map(|b| b.margin).sum::<f64>() / blocks.len() as f64
    };
    Ok(PageDecode {
        plane_margin: if planes == 3 { Some(plane_worst) } else { None },
        plane_registration: colour_ctx
            .as_ref()
            .map(|(_, pw, _)| pw.mean_offset_cells(cell_px)),
        dead_planes: colour_ctx.as_ref().map(|(_, _, c)| c.dead),
        descriptor: desc,
        blocks,
        erased,
        mirrored,
        worst_margin: worst,
        mean_margin: mean,
        geometry_residual,
    })
}

/// Mean of the central part of a cell, avoiding the edges where dot gain and
/// scanner MTF do their damage, plus the spread across those samples.
///
/// PLAN.md 5.8 proposed the central 50%. Measured against the blur and dot-gain
/// models, a tighter aperture is better: at 0.4-cell blur the worst-case
/// correction margin drops from 66% to 44% of capacity going from +/-0.18 to
/// +/-0.13 cell. Narrower still gains nothing and costs noise immunity.
pub(crate) fn sample_cell(img: &Gray, px: f64, py: f64, cell_px: f64) -> (f64, f64) {
    let o = cell_px * 0.13;
    let pts = [(0.0, 0.0), (-o, -o), (o, -o), (-o, o), (o, o)];
    let mut vs = [0.0f64; 5];
    for (i, (dx, dy)) in pts.iter().enumerate() {
        vs[i] = img.sample(px + dx, py + dy);
    }
    let m = vs.iter().sum::<f64>() / 5.0;
    let var = vs.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / 5.0;
    (m, var.sqrt())
}

fn read_descriptor(
    img: &Gray,
    integral: &Integral,
    h: &Homography,
    aspect: f64,
    span_x: f64,
    finder_unit_px: f64,
) -> Option<Descriptor> {
    let du = 1.0 / DESC_UNITS_ACROSS;
    let dv = aspect / DESC_UNITS_ACROSS;
    let ds_px_pre = du * span_x;
    let top = -(DESC_BLOCK_ROWS as f64 + desc_gap_cells(finder_unit_px / ds_px_pre)) * dv;
    let m = DESC_MARKER as f64 / 2.0;
    let nominal = [
        (m, m),
        (DESC_BLOCK_COLS as f64 - m, m),
        (m, DESC_BLOCK_ROWS as f64 - m),
        (DESC_BLOCK_COLS as f64 - m, DESC_BLOCK_ROWS as f64 - m),
    ];
    let ds_px = du * span_x;
    let mut dst = [Point::new(0.0, 0.0); 4];
    for (i, &(cx, cy)) in nominal.iter().enumerate() {
        let pred = h.apply(Point::new(cx * du, top + cy * dv));
        dst[i] = refine_dark(img, pred, ds_px * 2.5).unwrap_or(pred);
    }
    let src = nominal.map(|(x, y)| Point::new(x, y));
    let hs = Homography::from_four(&src, &dst)?;

    let mut cw = vec![0u8; RS_N];
    let mut wh = Whitener::new(WHITEN_SEED_DESC);
    let half = (ds_px * 1.5).max(2.0);
    for r in 0..DESC_ROWS {
        for c in 0..DESC_COLS {
            let p = hs.apply(Point::new(
                (c + DESC_MARKER) as f64 + 0.5,
                (r + DESC_MARKER) as f64 + 0.5,
            ));
            let (v, _) = sample_cell(img, p.x, p.y, ds_px);
            let (mm, sd) = integral.stats(p.x, p.y, half);
            let t = mm * (1.0 + SAUVOLA_K * (sd / SAUVOLA_R - 1.0));
            let bit = (v < t) ^ wh.next_bit();
            if bit {
                let i = r * DESC_COLS + c;
                cw[i / 8] |= 1 << (7 - (i % 8));
            }
        }
    }
    rs_decode(&mut cw, RS_N - DESC_RS_K, &[]).ok()?;
    Descriptor::from_message(&cw[..DESC_RS_K])
}

/// Darkness-weighted centroid inside a window; `None` if the window is blank.
pub(crate) fn refine_dark(img: &Gray, at: Point, radius: f64) -> Option<Point> {
    let r = radius.max(1.0);
    let x0 = ((at.x - r).floor().max(0.0)) as usize;
    let y0 = ((at.y - r).floor().max(0.0)) as usize;
    let x1 = ((at.x + r).ceil() as isize).clamp(0, img.w as isize) as usize;
    let y1 = ((at.y + r).ceil() as isize).clamp(0, img.h as isize) as usize;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut lo = 255.0f64;
    let mut hi = 0.0f64;
    for y in y0..y1 {
        for x in x0..x1 {
            let v = img.get(x, y) as f64;
            lo = lo.min(v);
            hi = hi.max(v);
        }
    }
    if hi - lo < 24.0 {
        return None;
    }
    let cut = (lo + hi) * 0.5;
    let (mut sx, mut sy, mut sw) = (0.0, 0.0, 0.0);
    for y in y0..y1 {
        for x in x0..x1 {
            let v = img.get(x, y) as f64;
            if v < cut {
                let w = cut - v;
                sx += x as f64 * w;
                sy += y as f64 * w;
                sw += w;
            }
        }
    }
    if sw <= 0.0 {
        return None;
    }
    Some(Point::new(sx / sw + 0.5, sy / sw + 0.5))
}

fn otsu(img: &Gray) -> u8 {
    let mut hist = [0u64; 256];
    for &p in &img.px {
        hist[p as usize] += 1;
    }
    let total: u64 = img.px.len() as u64;
    let sum: f64 = (0..256).map(|i| i as f64 * hist[i] as f64).sum();
    let (mut wb, mut sb, mut best, mut thr) = (0u64, 0.0f64, -1.0f64, 128u8);
    for t in 0..256 {
        wb += hist[t];
        if wb == 0 {
            continue;
        }
        let wf = total - wb;
        if wf == 0 {
            break;
        }
        sb += t as f64 * hist[t] as f64;
        let mb = sb / wb as f64;
        let mf = (sum - sb) / wf as f64;
        let v = wb as f64 * wf as f64 * (mb - mf) * (mb - mf);
        if v > best {
            best = v;
            thr = t as u8;
        }
    }
    thr
}

/// Locate finder patterns by the 1:1:3:1:1 run-length signature, verified in
/// both axes and clustered. Candidates are restricted to the page corners,
/// which is what keeps random payload cells from producing false positives.
fn find_finders(img: &Gray, thr: u8) -> Vec<(Point, f64)> {
    let dark = |x: usize, y: usize| img.get(x, y) <= thr;
    let mut cands: Vec<(f64, f64, f64)> = Vec::new();

    for y in (0..img.h).step_by(2) {
        let mut runs: Vec<(bool, usize, usize)> = Vec::new(); // colour, start, len
        let mut cur = dark(0, y);
        let mut start = 0usize;
        for x in 1..=img.w {
            let d = if x < img.w { dark(x, y) } else { !cur };
            if d != cur {
                runs.push((cur, start, x - start));
                cur = d;
                start = x;
            }
        }
        for w in runs.windows(5) {
            if !(w[0].0 && !w[1].0 && w[2].0 && !w[3].0 && w[4].0) {
                continue;
            }
            let total: usize = w.iter().map(|r| r.2).sum();
            let m = total as f64 / 7.0;
            if m < 1.2 {
                continue;
            }
            let v = m * 0.6;
            let ok = (w[0].2 as f64 - m).abs() < v
                && (w[1].2 as f64 - m).abs() < v
                && (w[2].2 as f64 - 3.0 * m).abs() < 3.0 * v
                && (w[3].2 as f64 - m).abs() < v
                && (w[4].2 as f64 - m).abs() < v;
            if !ok {
                continue;
            }
            let cx = w[2].1 as f64 + w[2].2 as f64 / 2.0;
            if let Some(cy) = verify_vertical(img, thr, cx.round() as usize, y, m) {
                cands.push((cx, cy, m));
            }
        }
    }

    // Cluster.
    let mut clusters: Vec<(f64, f64, f64, usize)> = Vec::new();
    for (x, y, m) in cands {
        let mut hit = false;
        for c in clusters.iter_mut() {
            if (c.0 / c.3 as f64 - x).abs() < m * 2.0 && (c.1 / c.3 as f64 - y).abs() < m * 2.0 {
                c.0 += x;
                c.1 += y;
                c.2 += m;
                c.3 += 1;
                hit = true;
                break;
            }
        }
        if !hit {
            clusters.push((x, y, m, 1));
        }
    }
    let mut out: Vec<(Point, f64, usize)> = clusters
        .into_iter()
        .filter(|c| c.3 >= 3)
        .map(|c| {
            let n = c.3 as f64;
            (Point::new(c.0 / n, c.1 / n), c.2 / n, c.3)
        })
        .collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.2));
    if out.is_empty() {
        return Vec::new();
    }
    // Random payload cells occasionally produce a 1:1:3:1:1 run by chance, but
    // never with consistent support or a consistent module size. Real finders
    // are crossed by dozens of scan lines and all share one unit, so keep only
    // candidates agreeing with the best-supported one.
    let top = out[0].2 as f64;
    let unit0 = out[0].1;
    out.retain(|c| c.2 as f64 >= (top * 0.25).max(3.0) && (c.1 - unit0).abs() / unit0 < 0.25);
    out.truncate(12);
    out.into_iter().map(|c| (c.0, c.1)).collect()
}

fn verify_vertical(img: &Gray, thr: u8, x: usize, y: usize, m: f64) -> Option<f64> {
    if x >= img.w {
        return None;
    }
    let dark = |yy: usize| img.get(x, yy) <= thr;
    if !dark(y) {
        return None;
    }
    let mut up = y;
    let mut runs_up = [0usize; 3];
    let mut colour = true;
    for k in 0..3 {
        let mut n = 0;
        while up > 0 && dark(up - 1) == colour {
            up -= 1;
            n += 1;
            if n as f64 > m * 6.0 {
                return None;
            }
        }
        runs_up[k] = n;
        colour = !colour;
        if up == 0 {
            break;
        }
    }
    let mut down = y;
    let mut runs_dn = [0usize; 3];
    colour = true;
    for k in 0..3 {
        let mut n = 0;
        while down + 1 < img.h && dark(down + 1) == colour {
            down += 1;
            n += 1;
            if n as f64 > m * 6.0 {
                return None;
            }
        }
        runs_dn[k] = n;
        colour = !colour;
        if down + 1 >= img.h {
            break;
        }
    }
    let centre_run = (runs_up[0] + runs_dn[0] + 1) as f64;
    if (centre_run - 3.0 * m).abs() > 2.0 * m {
        return None;
    }
    for k in 1..3 {
        if (runs_up[k] as f64 - m).abs() > m * 0.8 || (runs_dn[k] as f64 - m).abs() > m * 0.8 {
            return None;
        }
    }
    Some((y as f64 - runs_up[0] as f64 + y as f64 + runs_dn[0] as f64) / 2.0)
}

/// Enumerate plausible finder triples, largest right-angled corner first.
///
/// Area alone is not enough: a stray candidate inside the page can form a
/// slightly larger triangle than the true corners. The caller verifies each
/// triple against the distinct bottom-right marker, so this returns candidates
/// rather than an answer. Rotation by any multiple of 90 degrees resolves here;
/// mirroring does not, and is left to the descriptor retry.
fn orient_candidates(f: &[(Point, f64)]) -> Vec<(Point, Point, Point, f64)> {
    let n = f.len();
    let mut scored: Vec<(f64, usize, usize, usize)> = Vec::new();
    for i in 0..n {
        for j in 0..n {
            for k in j + 1..n {
                if i == j || i == k {
                    continue;
                }
                let (a, b, c) = (f[i].0, f[j].0, f[k].0);
                let v1 = (b.x - a.x, b.y - a.y);
                let v2 = (c.x - a.x, c.y - a.y);
                let l1 = (v1.0 * v1.0 + v1.1 * v1.1).sqrt();
                let l2 = (v2.0 * v2.0 + v2.1 * v2.1).sqrt();
                if l1 < 1.0 || l2 < 1.0 {
                    continue;
                }
                let cos = (v1.0 * v2.0 + v1.1 * v2.1).abs() / (l1 * l2);
                if cos > 0.18 {
                    continue;
                }
                scored.push(((v1.0 * v2.1 - v1.1 * v2.0).abs(), i, j, k));
            }
        }
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    scored.truncate(24);
    scored
        .into_iter()
        .map(|(_, i, j, k)| {
            let tl = f[i].0;
            let (mut tr, mut bl) = (f[j].0, f[k].0);
            let cross = (tr.x - tl.x) * (bl.y - tl.y) - (tr.y - tl.y) * (bl.x - tl.x);
            if cross < 0.0 {
                std::mem::swap(&mut tr, &mut bl);
            }
            (tl, tr, bl, (f[i].1 + f[j].1 + f[k].1) / 3.0)
        })
        .collect()
}

/// Locate the solid bottom-right marker near a predicted position.
///
/// The prediction comes from the parallelogram TR + BL - TL, which a homography
/// does *not* preserve: under perspective the true corner drifts away from it.
/// So search a neighbourhood for the darkest three-unit box. A five-unit solid
/// square scores near zero there and random payload cells score near mid-grey,
/// which makes this both the search and the verification.
fn locate_br(img: &Gray, integral: &Integral, at: Point, unit: f64) -> Option<Point> {
    let half = (unit * 1.5).max(2.0);
    // The parallelogram prediction drifts with perspective, so search widely and
    // let the two-part test below reject everything that is not the marker.
    let search = (unit * 8.0).max(8.0);
    let step = (unit * 0.35).max(1.0);
    let mut best: Option<(f64, Point)> = None;
    let mut dy = -search;
    while dy <= search {
        let mut dx = -search;
        while dx <= search {
            let p = Point::new(at.x + dx, at.y + dy);
            if p.x >= half && p.y >= half && p.x + half < img.w as f64 && p.y + half < img.h as f64
            {
                let (m, _) = integral.stats(p.x, p.y, half);
                if best.map_or(true, |(bm, _)| m < bm) {
                    best = Some((m, p));
                }
            }
            dx += step;
        }
        dy += step;
    }
    let (core, p) = best?;
    // Two conditions, because a darkest-box search over a large window will
    // always find *something*. Both are relative, not absolute: an illumination
    // gradient scales the whole neighbourhood, so a fixed brightness threshold
    // would reject a perfectly good marker in a dim corner of the scan.
    let ring = ring_mean(img, p, unit)?;
    if ring - core < 90.0 || core > ring * 0.45 {
        return None;
    }
    Some(refine_dark(img, p, unit * 3.0).unwrap_or(p))
}

/// Mean brightness on a *square* ring inside the marker's white ring.
///
/// The white ring is the square annulus from 2.5 to 4.5 units; a circle of any
/// radius that fits along the axes clips the marker's corners at 45 degrees, so
/// sample the square instead. Every point below has one coordinate at 3.5 units
/// and the other within 2 units, which is inside the annulus by construction.
fn ring_mean(img: &Gray, at: Point, unit: f64) -> Option<f64> {
    let a = unit * 3.5;
    let mut sum = 0.0;
    let mut n = 0.0;
    for k in 0..5 {
        let t = unit * (k as f64 - 2.0);
        for (dx, dy) in [(a, t), (-a, t), (t, a), (t, -a)] {
            let (x, y) = (at.x + dx, at.y + dy);
            if x < 0.0 || y < 0.0 || x >= img.w as f64 || y >= img.h as f64 {
                return None;
            }
            sum += img.sample(x, y);
            n += 1.0;
        }
    }
    Some(sum / n)
}

// ---------------------------------------------------------------- diagnostics

/// Intermediate decoder state, for diagnosing pages that will not read.
pub struct Probe {
    pub otsu: u8,
    pub finders: Vec<(Point, f64)>,
    pub corners: Option<(Point, Point, Point, Point)>,
    pub aspect: f64,
    pub desc_predicted: Vec<Point>,
    pub desc_refined: Vec<Point>,
    pub desc_ok: bool,
}

pub fn probe(img: &Gray) -> Probe {
    let thr = otsu(img);
    let finders = find_finders(img, thr);
    let mut p = Probe {
        otsu: thr,
        finders: finders.clone(),
        corners: None,
        aspect: 0.0,
        desc_predicted: Vec::new(),
        desc_refined: Vec::new(),
        desc_ok: false,
    };
    let integral = Integral::new(img);
    let cands = orient_candidates(&finders);
    let Some(&(tl, tr, bl, unit)) = cands
        .iter()
        .find(|c| {
            locate_br(
                img,
                &integral,
                Point::new(c.1.x + c.2.x - c.0.x, c.1.y + c.2.y - c.0.y),
                c.3,
            )
            .is_some()
        })
        .or(cands.first())
    else {
        return p;
    };
    let br_pred = Point::new(tr.x + bl.x - tl.x, tr.y + bl.y - tl.y);
    let br = locate_br(img, &integral, br_pred, unit).unwrap_or(br_pred);
    p.corners = Some((tl, tr, bl, br));
    let uv = [
        Point::new(0.0, 0.0),
        Point::new(1.0, 0.0),
        Point::new(0.0, 1.0),
        Point::new(1.0, 1.0),
    ];
    let Some(h) = Homography::from_four(&uv, &[tl, tr, bl, br]) else {
        return p;
    };
    let span_x = tl.dist(tr);
    p.aspect = span_x / tl.dist(bl);
    let du = 1.0 / DESC_UNITS_ACROSS;
    let dv = p.aspect / DESC_UNITS_ACROSS;
    let ds_px = du * span_x;
    let top = -(DESC_BLOCK_ROWS as f64 + desc_gap_cells(unit / ds_px)) * dv;
    let m = DESC_MARKER as f64 / 2.0;
    for (cx, cy) in [
        (m, m),
        (DESC_BLOCK_COLS as f64 - m, m),
        (m, DESC_BLOCK_ROWS as f64 - m),
        (DESC_BLOCK_COLS as f64 - m, DESC_BLOCK_ROWS as f64 - m),
    ] {
        let pr = h.apply(Point::new(cx * du, top + cy * dv));
        p.desc_predicted.push(pr);
        p.desc_refined
            .push(refine_dark(img, pr, ds_px * 2.5).unwrap_or(Point::new(f64::NAN, f64::NAN)));
    }
    p.desc_ok = read_descriptor(img, &integral, &h, p.aspect, span_x, unit).is_some();
    p
}
