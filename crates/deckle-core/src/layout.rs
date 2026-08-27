//! Paper geometry and the capacity oracle.
//!
//! This module is the single place capacity is computed. `estimate` is this code
//! in dry-run mode, not a parallel formula (PLAN.md fixed decision 6, section 10),
//! and `tests/invariants.rs` asserts the two agree.

use std::fmt;

pub const DESC_COLS: usize = 85;
pub const DESC_ROWS: usize = 24;
pub const DESC_BLOCK_COLS: usize = 91; // data plus the corner-marker ring
pub const DESC_BLOCK_ROWS: usize = 30;
pub const DESC_MARKER: usize = 3; // corner marker side, in descriptor cells
                                  // Clearance between the descriptor strip and the fiducial-centre row is not a
                                  // constant: the finder's reserved corner square reaches 4.5 finder-units above
                                  // that row, and how many strip cells that is depends on the configuration.
                                  // `desc_gap_cells` derives it from a ratio both sides can measure - the encoder
                                  // from its geometry, the decoder from the finder unit it just found in pixels.
pub const DESC_UNITS_ACROSS: f64 = 272.0; // strip cell = fiducial width / this
pub const DESC_RS_K: usize = 127; // RS(255,127): corrects 64 of 255 symbols

pub const SYNC_PERIOD: usize = 32; // cells between sync marks
/// Colour calibration patches: a 4x4 square every 64 cells, offset from the sync
/// lattice so the two never collide. 16 cells in 4096 is 0.39% (PLAN.md 18.5).
pub const CAL_PERIOD: usize = 64;
pub const CAL_BLOCK: usize = 4;
pub const CAL_OFFSET: usize = 16;
/// Per-plane registration marks sit in a strip beside each corner square, three
/// marks of one finder-unit pitch each, one per ink (PLAN.md 18.4).
///
/// The strip is four units wide for a two-unit mark, so the mark keeps a full
/// unit of white on each side. Three units left only 1.5 cells of margin, and a
/// centroid window that reached past it into payload cells pulled the fit half a
/// cell out - enough to put the cell-bit error rate above 10%.
pub const REG_UNITS: usize = 4;
/// Interleaving is confined to horizontal bands of this many cell rows.
///
/// PLAN.md 5.6 specifies a single affine permutation over the whole page, which
/// is right for a thin burst - a fold disperses across every codeword and costs
/// each one a byte or two. Measured against the missing-strip model it is wrong
/// for large-area loss: dispersing a 6%-of-page hole puts ~120 bad bits in
/// *every* codeword, past RS capacity, so the whole page dies. Banding keeps the
/// fold behaviour and turns a hole into a few whole erased codewords, which is
/// what cross-block parity exists to rebuild. Costs about 1% of capacity to the
/// partial codeword at each band's end.
pub const BAND_ROWS: usize = 128;
pub const SYNC_BLOCK: usize = 4; // sync mark reserved square, in cells
pub const RS_N: usize = 255;
pub const BLOCK_HEADER: usize = 8; // index(3) + flags(1) + crc32c(4)

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ecc {
    L,
    M,
    Q,
    H,
}

impl Ecc {
    pub fn k(self) -> usize {
        match self {
            Ecc::L => 239,
            Ecc::M => 223,
            Ecc::Q => 191,
            Ecc::H => 159,
        }
    }
    pub fn nsym(self) -> usize {
        RS_N - self.k()
    }
    pub fn payload(self) -> usize {
        self.k() - BLOCK_HEADER
    }
    pub fn parse(s: &str) -> Option<Ecc> {
        match s.to_ascii_uppercase().as_str() {
            "L" => Some(Ecc::L),
            "M" => Some(Ecc::M),
            "Q" => Some(Ecc::Q),
            "H" => Some(Ecc::H),
            _ => None,
        }
    }
    pub fn from_k(k: usize) -> Option<Ecc> {
        [Ecc::L, Ecc::M, Ecc::Q, Ecc::H]
            .into_iter()
            .find(|e| e.k() == k)
    }
}

impl fmt::Display for Ecc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = match self {
            Ecc::L => 'L',
            Ecc::M => 'M',
            Ecc::Q => 'Q',
            Ecc::H => 'H',
        };
        write!(f, "{c}")
    }
}

/// Which ink planes carry payload.
///
/// PLAN.md section 18 specifies an optional colour mode: three binary ink planes
/// give 3 bits per cell, not 4, because an RGB scanner takes three measurements
/// and cannot separate a fourth. K is reserved for structure. The mode is
/// designed but not built, so `Cmy` is rejected rather than silently ignored.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum InkPlanes {
    /// Black only. The default, and the only mode rated for long-term storage.
    #[default]
    K,
    /// Cyan, magenta and yellow. Roughly doubles capacity; not archival.
    Cmy,
}

impl InkPlanes {
    /// Bits carried by one payload cell.
    pub fn bits_per_cell(self) -> usize {
        match self {
            InkPlanes::K => 1,
            InkPlanes::Cmy => 3,
        }
    }
    pub fn count(self) -> usize {
        self.bits_per_cell()
    }
    pub fn code(self) -> u8 {
        match self {
            InkPlanes::K => 0,
            // bit0 = cyan, bit1 = magenta, bit2 = yellow (PLAN.md 18.9)
            InkPlanes::Cmy => 0b111,
        }
    }
    pub fn from_code(c: u8) -> Option<InkPlanes> {
        match c {
            0 => Some(InkPlanes::K),
            0b111 => Some(InkPlanes::Cmy),
            _ => None,
        }
    }
    pub fn parse(s: &str) -> Option<InkPlanes> {
        match s.to_ascii_lowercase().as_str() {
            "k" | "black" | "mono" => Some(InkPlanes::K),
            "cmy" | "cmyk" | "color" | "colour" => Some(InkPlanes::Cmy),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Paper {
    pub w_mm: f64,
    pub h_mm: f64,
}

impl Paper {
    pub fn parse(s: &str) -> Option<Paper> {
        let p = match s.to_ascii_lowercase().as_str() {
            "a4" => (210.0, 297.0),
            "a3" => (297.0, 420.0),
            "letter" => (215.9, 279.4),
            "legal" => (215.9, 355.6),
            other => {
                let (a, b) = other.split_once('x')?;
                (a.trim().parse().ok()?, b.trim().parse().ok()?)
            }
        };
        Some(Paper {
            w_mm: p.0,
            h_mm: p.1,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub paper: Paper,
    pub landscape: bool,
    pub margin_mm: f64,
    pub cell_um: u32,
    pub render_dpi: u32,
    pub ecc: Ecc,
    pub parity_ratio: f64,
    pub ink_planes: InkPlanes,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            paper: Paper {
                w_mm: 210.0,
                h_mm: 297.0,
            },
            landscape: false,
            margin_mm: 12.7,
            cell_um: 254,
            render_dpi: 600,
            ecc: Ecc::Q,
            parity_ratio: 0.20,
            ink_planes: InkPlanes::K,
        }
    }
}

#[derive(Debug)]
pub enum LayoutError {
    CellNotIntegerDots { cell_um: u32, dpi: u32, dots: f64 },
    PageTooSmall(String),
    HeaderTooSmall { need_mm: f64, have_mm: f64 },
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutError::CellNotIntegerDots { cell_um, dpi, dots } => write!(
                f,
                "cell size {cell_um} um is {dots:.3} device dots at {dpi} dpi; it must be a whole \
                 number of dots so the render path never resamples (PLAN.md 4.1). \
                 The nearest whole dot count is {} dots = {:.0} um.",
                dots.round().max(1.0) as u32,
                dots.round().max(1.0) * 25400.0 / *dpi as f64
            ),
            LayoutError::PageTooSmall(s) => write!(f, "page too small: {s}"),
            LayoutError::HeaderTooSmall { need_mm, have_mm } => write!(
                f,
                "header band needs {need_mm:.1} mm but only {have_mm:.1} mm is available"
            ),
        }
    }
}

/// The full geometry of one page. Every dimension is physical; the renderer
/// converts to device dots exactly once, at the last step.
#[derive(Clone, Debug)]
pub struct PageGeometry {
    pub page_w_mm: f64,
    pub page_h_mm: f64,
    pub margin_mm: f64,
    pub header_mm: f64,
    pub cell_mm: f64,
    pub cols: usize,
    pub rows: usize,
    /// Reserved corner square side, in cells: nine finder units, so the seven-unit
    /// finder keeps a one-unit white ring and adjacent payload cells cannot extend
    /// its outer run and break the 1:1:3:1:1 ratio scan.
    pub fid_cells: usize,
    /// Finder unit, in cells. Finder pattern side is seven of these.
    pub fid_unit: usize,
    pub grid_x_mm: f64,
    pub grid_y_mm: f64,
    pub sync_marks: Vec<(usize, usize)>,
    pub bands: Vec<Band>,
    pub band_rows: usize,
    /// Cells available to payload after fiducials and sync marks.
    pub usable_cells: usize,
    /// Codewords per page.
    pub codewords: usize,
    pub ecc: Ecc,
    pub ink: InkPlanes,
    pub cal_patches: Vec<(usize, usize, u8)>,
    pub cell_dots: u32,
    pub render_dpi: u32,
}

impl PageGeometry {
    pub fn plan(cfg: &Config) -> Result<PageGeometry, LayoutError> {
        let (page_w, page_h) = if cfg.landscape {
            (cfg.paper.h_mm, cfg.paper.w_mm)
        } else {
            (cfg.paper.w_mm, cfg.paper.h_mm)
        };

        // The device dot is the quantum: a cell must be a whole number of them so
        // the render path never resamples (PLAN.md 4.1). The requested size in
        // micrometres need only round to one, since most useful dot counts are not
        // an integer number of micrometres (4 dots at 600 dpi is 169.33 um).
        let dots = cfg.cell_um as f64 * cfg.render_dpi as f64 / 25400.0;
        let n = dots.round();
        if n < 1.0 || (dots - n).abs() > n * 0.01 {
            return Err(LayoutError::CellNotIntegerDots {
                cell_um: cfg.cell_um,
                dpi: cfg.render_dpi,
                dots,
            });
        }
        // Physical cell size follows the dot count, not the request.
        let cell_mm = n * 25.4 / cfg.render_dpi as f64;

        let data_w = page_w - 2.0 * cfg.margin_mm;
        if data_w <= 20.0 {
            return Err(LayoutError::PageTooSmall(format!(
                "{data_w:.1} mm of printable width"
            )));
        }
        // The descriptor strip scales with the fiducial span, so the band it needs
        // grows with the page. 25 mm covers A4 and Letter; A3 needs more.
        let header_mm = (0.095 * data_w + 6.0).max(25.0);
        let data_h = page_h - 2.0 * cfg.margin_mm - header_mm;
        if data_h <= 20.0 {
            return Err(LayoutError::PageTooSmall(format!(
                "{data_h:.1} mm of printable height below the header band"
            )));
        }

        let cols = (data_w / cell_mm).floor() as usize;
        let rows = (data_h / cell_mm).floor() as usize;

        // Finder side targets data_w/40; rounded to a multiple of 7 cells so the
        // 1:1:3:1:1 finder pattern lands on exact cell boundaries.
        // Target a finder about data_w/40 across, but never below three cells per
        // unit: a seven-cell finder is small enough that random payload mimics its
        // 1:1:3:1:1 signature often enough to mislead orientation.
        let units = (data_w / (280.0 * cell_mm)).round().max(3.0) as usize;
        let fid_cells = 9 * units;
        if cols < 4 * fid_cells || rows < 4 * fid_cells {
            return Err(LayoutError::PageTooSmall(format!(
                "{cols}x{rows} cells cannot hold four {fid_cells}-cell finders"
            )));
        }

        let ink = cfg.ink_planes;
        let sync_marks = sync_marks_ink(cols, rows, fid_cells, units, ink);
        let cal_patches = cal_patches_for(cols, rows, fid_cells, units, ink);
        let usable_cells = usable_cells_ink(cols, rows, fid_cells, units, ink);
        let bands = bands_ink(cols, rows, fid_cells, units, ink, BAND_ROWS);
        let codewords: usize = bands.iter().map(|b| b.codewords).sum();

        let geo = PageGeometry {
            page_w_mm: page_w,
            page_h_mm: page_h,
            margin_mm: cfg.margin_mm,
            header_mm,
            cell_mm,
            cols,
            rows,
            fid_cells,
            fid_unit: units,
            grid_x_mm: cfg.margin_mm,
            grid_y_mm: cfg.margin_mm + header_mm,
            sync_marks,
            cal_patches,
            ink,
            bands,
            band_rows: BAND_ROWS,
            usable_cells,
            codewords,
            ecc: cfg.ecc,
            cell_dots: n as u32,
            render_dpi: cfg.render_dpi,
        };

        // The descriptor strip lives above the grid, inside the header band.
        let need = geo.desc_top_mm();
        if need < cfg.margin_mm - 1e-9 {
            return Err(LayoutError::HeaderTooSmall {
                need_mm: header_mm + (cfg.margin_mm - need),
                have_mm: header_mm,
            });
        }
        Ok(geo)
    }

    pub fn payload_bytes_per_page(&self) -> usize {
        self.codewords * self.ecc.payload()
    }

    /// Registration-strip origins for colour mode, in cells.
    pub fn reg_strips(&self) -> [(usize, usize); 4] {
        reg_strips_for(self.cols, self.rows, self.fid_cells, self.fid_unit)
    }

    /// Fiducial centre positions in cell coordinates, in TL, TR, BL, BR order.
    pub fn fiducial_centres_cells(&self) -> [(f64, f64); 4] {
        let f = self.fid_cells as f64 / 2.0;
        [
            (f, f),
            (self.cols as f64 - f, f),
            (f, self.rows as f64 - f),
            (self.cols as f64 - f, self.rows as f64 - f),
        ]
    }

    /// Width and height in mm of the quad spanned by the fiducial centres.
    pub fn fid_span_mm(&self) -> (f64, f64) {
        (
            (self.cols - self.fid_cells) as f64 * self.cell_mm,
            (self.rows - self.fid_cells) as f64 * self.cell_mm,
        )
    }

    /// Descriptor-strip cell size in mm. Defined as a fraction of the fiducial
    /// span so the decoder can place the strip before it has read anything.
    pub fn desc_cell_mm(&self) -> f64 {
        self.fid_span_mm().0 / DESC_UNITS_ACROSS
    }

    pub fn desc_gap_cells(&self) -> f64 {
        desc_gap_cells(self.fid_unit as f64 * self.cell_mm / self.desc_cell_mm())
    }

    /// Top edge of the descriptor strip in page mm.
    pub fn desc_top_mm(&self) -> f64 {
        let ds = self.desc_cell_mm();
        let fid_centre_y = self.grid_y_mm + (self.fid_cells as f64 / 2.0) * self.cell_mm;
        fid_centre_y - (DESC_BLOCK_ROWS as f64 + self.desc_gap_cells()) * ds
    }

    pub fn desc_left_mm(&self) -> f64 {
        self.grid_x_mm + (self.fid_cells as f64 / 2.0) * self.cell_mm
    }

    /// Descriptor strip corner-marker centres, in the fiducial frame (u, v).
    /// (0,0) is the top-left fiducial centre; (1,1) the bottom-right.
    pub fn desc_markers_uv(&self) -> [(f64, f64); 4] {
        let (wf, hf) = self.fid_span_mm();
        let ds = self.desc_cell_mm();
        let du = ds / wf;
        let dv = ds / hf;
        let top = -(DESC_BLOCK_ROWS as f64 + self.desc_gap_cells()) * dv;
        let m = DESC_MARKER as f64 / 2.0;
        [
            (m * du, top + m * dv),
            ((DESC_BLOCK_COLS as f64 - m) * du, top + m * dv),
            (m * du, top + (DESC_BLOCK_ROWS as f64 - m) * dv),
            (
                (DESC_BLOCK_COLS as f64 - m) * du,
                top + (DESC_BLOCK_ROWS as f64 - m) * dv,
            ),
        ]
    }
}

/// Strip-cell clearance above the fiducial-centre row, from the ratio of the
/// finder unit to the descriptor cell.
pub fn desc_gap_cells(finder_unit_over_desc_cell: f64) -> f64 {
    4.5 * finder_unit_over_desc_cell + 2.0
}

#[inline]
pub fn in_corner(x: usize, y: usize, cols: usize, rows: usize, f: usize) -> bool {
    (x < f || x >= cols - f) && (y < f || y >= rows - f)
}

/// Sync-mark block origins, derived from grid dimensions alone. The decoder
/// recomputes this from the page descriptor, so encoder and decoder cannot drift.
pub fn sync_marks_for(cols: usize, rows: usize, f: usize) -> Vec<(usize, usize)> {
    sync_marks_ink(cols, rows, f, 0, InkPlanes::K)
}

/// As `sync_marks_for`, skipping blocks that a colour page's registration strips
/// would overwrite. A sync mark printed under an ink mark is not a sync mark: it
/// drags both the warp field and the mark's own centroid.
pub fn sync_marks_ink(
    cols: usize,
    rows: usize,
    f: usize,
    unit: usize,
    ink: InkPlanes,
) -> Vec<(usize, usize)> {
    let mut marks = Vec::new();
    let mut sy = 0;
    while sy + SYNC_BLOCK <= rows {
        let mut sx = 0;
        while sx + SYNC_BLOCK <= cols {
            let clash = ink != InkPlanes::K
                && (in_reg_strip(cols, rows, f, unit, sx, sy)
                    || in_reg_strip(
                        cols,
                        rows,
                        f,
                        unit,
                        sx + SYNC_BLOCK - 1,
                        sy + SYNC_BLOCK - 1,
                    ));
            if !clash
                && !in_corner(sx, sy, cols, rows, f)
                && !in_corner(sx + SYNC_BLOCK - 1, sy + SYNC_BLOCK - 1, cols, rows, f)
            {
                marks.push((sx, sy));
            }
            sx += SYNC_PERIOD;
        }
        sy += SYNC_PERIOD;
    }
    marks
}

/// Colour calibration patch origins, and the state each one prints.
///
/// The eight states cycle across the page so every one is measured close to
/// every part of it. The patches age with the data, which is what lets a decoder
/// twenty years from now find the decision boundaries where the ink actually is.
pub fn cal_patches_for(
    cols: usize,
    rows: usize,
    f: usize,
    unit: usize,
    ink: InkPlanes,
) -> Vec<(usize, usize, u8)> {
    let mut out = Vec::new();
    if ink == InkPlanes::K {
        return out;
    }
    let mut gy = 0;
    while CAL_OFFSET + gy * CAL_PERIOD + CAL_BLOCK <= rows {
        let mut gx = 0;
        while CAL_OFFSET + gx * CAL_PERIOD + CAL_BLOCK <= cols {
            let x = CAL_OFFSET + gx * CAL_PERIOD;
            let y = CAL_OFFSET + gy * CAL_PERIOD;
            // A patch that lands on a registration strip would be drawn over an
            // ink mark and destroy it. Whether that happens depends on how the
            // 64-cell lattice lines up with the strips, so it appears at some
            // cell sizes and not others.
            let clash = in_reg_strip(cols, rows, f, unit, x, y)
                || in_reg_strip(cols, rows, f, unit, x + CAL_BLOCK - 1, y + CAL_BLOCK - 1);
            if !clash
                && !in_corner(x, y, cols, rows, f)
                && !in_corner(x + CAL_BLOCK - 1, y + CAL_BLOCK - 1, cols, rows, f)
            {
                out.push((x, y, ((gx + gy * 3) % 8) as u8));
            }
            gx += 1;
        }
        gy += 1;
    }
    out
}

/// Registration-mark strips: (x0, y0, unit) of each corner's three-mark strip.
/// Marks run down the strip in plane order: cyan, magenta, yellow.
pub fn reg_strips_for(cols: usize, rows: usize, f: usize, u: usize) -> [(usize, usize); 4] {
    let w = REG_UNITS * u;
    [
        (f, 0),
        (cols - f - w, 0),
        (f, rows - f),
        (cols - f - w, rows - f),
    ]
}

/// Centre of one plane's registration mark, in cell coordinates.
///
/// Encoder and decoder must agree on this to well under a cell, so it lives in
/// one place: computing it twice from a description in prose cost half a cell of
/// offset and a page of garbage.
pub fn reg_mark_centre(sx: usize, sy: usize, unit: usize, plane: usize) -> (f64, f64) {
    let (ox, oy) = reg_mark_origin(sx, sy, unit, plane);
    (ox as f64 + unit as f64, oy as f64 + unit as f64)
}

/// Top-left cell of that mark's solid two-unit square. Horizontally centred in
/// the four-unit strip; vertically the neighbours are other planes' marks, which
/// are white in this plane's channel, so a tighter inset is safe there.
pub fn reg_mark_origin(sx: usize, sy: usize, unit: usize, plane: usize) -> (usize, usize) {
    (sx + unit, sy + 3 * plane * unit + unit / 2)
}

#[inline]
fn in_reg_strip(cols: usize, rows: usize, f: usize, u: usize, x: usize, y: usize) -> bool {
    let w = REG_UNITS * u;
    let in_x = (x >= f && x < f + w) || (x >= cols - f - w && x < cols - f);
    let in_y = y < f || y >= rows - f;
    in_x && in_y
}

/// True when the cell is reserved for structure and carries no payload.
/// Recomputes the sync-mark condition rather than searching `sync_marks`, which
/// is stored in row-major order and is therefore not sorted by (x, y).
#[inline]
pub fn is_reserved_at(cols: usize, rows: usize, f: usize, x: usize, y: usize) -> bool {
    is_reserved_ink(cols, rows, f, 0, InkPlanes::K, x, y)
}

/// As `is_reserved_at`, but also excluding the structure colour mode adds.
#[inline]
pub fn is_reserved_ink(
    cols: usize,
    rows: usize,
    f: usize,
    unit: usize,
    ink: InkPlanes,
    x: usize,
    y: usize,
) -> bool {
    if in_corner(x, y, cols, rows, f) {
        return true;
    }
    if ink != InkPlanes::K {
        if in_reg_strip(cols, rows, f, unit, x, y) {
            return true;
        }
        if x >= CAL_OFFSET
            && y >= CAL_OFFSET
            && (x - CAL_OFFSET) % CAL_PERIOD < CAL_BLOCK
            && (y - CAL_OFFSET) % CAL_PERIOD < CAL_BLOCK
        {
            let (bx, by) = (
                x - (x - CAL_OFFSET) % CAL_PERIOD,
                y - (y - CAL_OFFSET) % CAL_PERIOD,
            );
            if bx + CAL_BLOCK <= cols
                && by + CAL_BLOCK <= rows
                && !in_corner(bx, by, cols, rows, f)
                && !in_corner(bx + CAL_BLOCK - 1, by + CAL_BLOCK - 1, cols, rows, f)
            {
                return true;
            }
        }
    }
    if x % SYNC_PERIOD < SYNC_BLOCK && y % SYNC_PERIOD < SYNC_BLOCK {
        let (bx, by) = (x - x % SYNC_PERIOD, y - y % SYNC_PERIOD);
        return bx + SYNC_BLOCK <= cols
            && by + SYNC_BLOCK <= rows
            && !in_corner(bx, by, cols, rows, f)
            && !in_corner(bx + SYNC_BLOCK - 1, by + SYNC_BLOCK - 1, cols, rows, f);
    }
    false
}

#[inline]
pub fn is_reserved(geo: &PageGeometry, x: usize, y: usize) -> bool {
    is_reserved_at(geo.cols, geo.rows, geo.fid_cells, x, y)
}

/// One interleaving band: a run of cell rows carrying its own codewords.
#[derive(Clone, Copy, Debug)]
pub struct Band {
    pub row0: usize,
    pub row1: usize,
    pub cells: usize,
    pub codewords: usize,
    pub first_cw: usize,
}

/// Bands for a grid, derived from descriptor fields alone so the encoder and the
/// decoder cannot disagree.
pub fn bands_for(cols: usize, rows: usize, f: usize, band_rows: usize) -> Vec<Band> {
    bands_ink(cols, rows, f, 0, InkPlanes::K, band_rows)
}

/// Bands for a grid. In colour mode a band's codewords divide evenly across the
/// three ink planes, because a plane owns its codewords outright (PLAN.md 18.3):
/// a faded plane then erases one third of the blocks instead of putting a wrong
/// bit in every one of them.
pub fn bands_ink(
    cols: usize,
    rows: usize,
    f: usize,
    unit: usize,
    ink: InkPlanes,
    band_rows: usize,
) -> Vec<Band> {
    let planes = ink.count();
    let mut out = Vec::new();
    let mut first_cw = 0usize;
    let mut r = 0usize;
    while r < rows {
        let r1 = (r + band_rows).min(rows);
        let mut cells = 0usize;
        for y in r..r1 {
            for x in 0..cols {
                if !is_reserved_ink(cols, rows, f, unit, ink, x, y) {
                    cells += 1;
                }
            }
        }
        // Per plane, then multiplied back up: every plane gets the same count.
        let codewords = (cells / (RS_N * 8)) * planes;
        out.push(Band {
            row0: r,
            row1: r1,
            cells,
            codewords,
            first_cw,
        });
        first_cw += codewords;
        r = r1;
    }
    out
}

/// Per-band interleave coefficients. Both sides derive these from the band's own
/// cell count, so only the page-level seed has to travel in the descriptor.
pub fn band_interleave(band: &Band, index: usize, seed: u32) -> (u64, u64) {
    let a = choose_interleave_a(band.cells) as u64;
    let b = (seed as u64).wrapping_add(index as u64 * 7919) % band.cells.max(1) as u64;
    (a, b)
}

/// Usable payload cells for a grid, from descriptor fields alone.
pub fn usable_cells_for(cols: usize, rows: usize, f: usize) -> usize {
    usable_cells_ink(cols, rows, f, 0, InkPlanes::K)
}

pub fn usable_cells_ink(cols: usize, rows: usize, f: usize, unit: usize, ink: InkPlanes) -> usize {
    let mut n = 0;
    for y in 0..rows {
        for x in 0..cols {
            if !is_reserved_ink(cols, rows, f, unit, ink, x, y) {
                n += 1;
            }
        }
    }
    let _ = sync_marks_for(cols, rows, f);
    n
}

/// Interleave multiplier: coprime to the cell count so the map is a bijection,
/// and near the golden ratio so a contiguous 2D burst disperses (PLAN.md 5.6).
pub fn choose_interleave_a(c: usize) -> u32 {
    fn gcd(mut a: usize, mut b: usize) -> usize {
        while b != 0 {
            let t = a % b;
            a = b;
            b = t;
        }
        a
    }
    if c < 3 {
        return 1;
    }
    let start = ((c as f64) * 0.618_033_988_749_895).round() as usize;
    let mut a = start.clamp(2, c - 1);
    for _ in 0..c {
        if gcd(a, c) == 1 {
            return a as u32;
        }
        a += 1;
        if a >= c {
            a = 2;
        }
    }
    1
}
