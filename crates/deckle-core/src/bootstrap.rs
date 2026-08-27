//! The bootstrap page (PLAN.md section 12.5).
//!
//! Every archive ends with a page that explains itself: what the format is, how
//! to decode it, and the complete source of a reference decoder as standard QR
//! symbols. This is what makes an archive recoverable from the paper alone,
//! without Deckle, by someone who has only a QR reader and a Python interpreter.
//!
//! Everything here is printed in black only, at low density, deliberately. This
//! page is the last thing standing between the reader and total loss, so it is
//! the most conservative thing on the sheet.

use crate::base45;
use crate::bitmap::Gray;
use crate::descriptor::Descriptor;
use crate::doc::deflate;
use crate::font;
use crate::layout::PageGeometry;
use crate::sha256::{hex, sha256};
use qrcodegen::{QrCode, QrCodeEcc, QrSegment, Version};

/// The two reference programs, compiled into the binary so the page can never
/// disagree with the files in the repository.
pub const DKL_REF: &str = include_str!("../../../reference/dkl_ref.py");
pub const DKL_FEC: &str = include_str!("../../../reference/dkl_fec.py");

const QR_QUIET: i32 = 4;
/// Preferred module size. Bigger than the data pages' cells by a wide margin,
/// because this page has to be readable when everything else has failed.
const TARGET_MODULE_MM: f64 = 0.4;
/// Never go below this, whatever the paper size. A commodity phone reader
/// manages 0.25 mm at close range; a flatbed manages it easily.
const MIN_MODULE_MM: f64 = 0.25;

struct Program {
    file: &'static str,
    source: &'static str,
    note: &'static str,
}

/// Where one QR symbol was placed. Reading the tiles in this order and
/// concatenating their text reproduces the Base45 payload exactly, which is what
/// the printed procedure asks a person to do by hand.
#[derive(Clone, Debug)]
pub struct TilePlacement {
    pub sheet: usize,
    pub x: usize,
    pub y: usize,
    /// Side of the symbol including its quiet zone, in pixels.
    pub px: usize,
    pub program: &'static str,
    pub index: usize,
    pub count: usize,
    /// The text this symbol carries.
    pub text: String,
}

/// A rendered bootstrap page set, plus enough structure to verify it.
pub struct Bootstrap {
    pub sheets: Vec<Gray>,
    pub tiles: Vec<TilePlacement>,
    /// (file name, SHA-256 of the source, the Base45 text split across its tiles)
    pub programs: Vec<(&'static str, String, String)>,
}

/// Lay out and render the bootstrap page(s), one image per sheet.
pub fn render_sheets(
    geo: &PageGeometry,
    desc: &Descriptor,
    data_sheets: usize,
    plain_sha256: &[u8; 32],
    tool_version: &str,
) -> Vec<Gray> {
    render(geo, desc, data_sheets, plain_sha256, tool_version).sheets
}

/// As `render_sheets`, but also reporting where every QR symbol landed.
pub fn render(
    geo: &PageGeometry,
    desc: &Descriptor,
    data_sheets: usize,
    plain_sha256: &[u8; 32],
    tool_version: &str,
) -> Bootstrap {
    let mut programs = vec![Program {
        file: "dkl_ref.py",
        source: DKL_REF,
        note: "Reads the pages and rebuilds the files. This is the one you need.",
    }];
    // The parity tool is only worth its tiles when there is parity to use.
    if desc.fec_parity_blocks > 0 {
        programs.push(Program {
            file: "dkl_fec.py",
            source: DKL_FEC,
            note: "Only needed if pages are missing or will not read.",
        });
    }

    let dpi = geo.render_dpi as f64;
    let mm2px = |mm: f64| (mm * dpi / 25.4).round() as usize;
    let page_w = mm2px(geo.page_w_mm);
    let page_h = mm2px(geo.page_h_mm);
    let margin = mm2px(geo.margin_mm);
    let text_w = page_w - 2 * margin;
    // Pick the symbol version and module size together. Bigger versions spend
    // proportionally less area on quiet zones and function patterns, so prefer
    // the largest one that still leaves two columns on this paper at a module
    // size we are willing to print. On A4 that is version 40 at 0.4 mm; on small
    // paper a version-40 tile would be wider than half the sheet, and holding
    // version fixed there cost ten bootstrap sheets instead of three.
    let (qr_version, module) = choose_symbol(text_w, dpi);
    // Type sizes are in device dots, so they must scale with the render
    // resolution. Body text lands at about 3 mm tall, which is ordinary
    // small print; at 600 dpi that is a glyph scale of 8, not 2.
    let body = ((dpi / 75.0).round() as usize).max(2);
    let head = (body * 3 / 2).max(3);
    let mast = (body * 5 / 2).max(4);

    let mut sheets: Vec<Gray> = Vec::new();
    let mut tiles: Vec<TilePlacement> = Vec::new();
    let mut prog_meta: Vec<(&'static str, String, String)> = Vec::new();
    let mut img = Gray::new(page_w, page_h, 255);
    let mut y = margin;
    let mut sheet_no = 1usize;

    macro_rules! newpage {
        () => {{
            footer(&mut img, page_w, page_h, margin, sheet_no);
            sheets.push(std::mem::replace(&mut img, Gray::new(page_w, page_h, 255)));
            sheet_no += 1;
            y = margin;
            heading(
                &mut img,
                margin,
                &mut y,
                text_w,
                head,
                "BOOTSTRAP PAGE, CONTINUED",
            );
        }};
    }
    macro_rules! need {
        ($h:expr) => {
            if y + $h > page_h - margin - font::line_height(body) * 2 {
                newpage!();
            }
        };
    }

    // ---- masthead
    font::draw(&mut img, margin, y, mast, "DECKLE PAPER ARCHIVE");
    y += font::line_height(mast);
    font::draw(
        &mut img,
        margin,
        y,
        head,
        "BOOTSTRAP PAGE - READ THIS FIRST",
    );
    y += font::line_height(head) + mm2px(2.0);
    rule(&mut img, margin, y, text_w);
    y += mm2px(3.0);

    // ---- what this is
    for line in font::wrap(
        "These sheets are a backup of computer files, printed as a grid of tiny black \
         squares. This page explains how to turn them back into files. You do not need \
         the program that made them.",
        body,
        text_w,
    ) {
        font::draw(&mut img, margin, y, body, &line);
        y += font::line_height(body);
    }
    y += mm2px(3.0);

    // ---- identity
    heading(&mut img, margin, &mut y, text_w, head, "THIS ARCHIVE");
    let sha = hex(plain_sha256);
    let facts = [
        format!("Document ID      {}", hex(&desc.doc_uuid)),
        format!("Contents SHA-256 {}", &sha[..32]),
        format!("                 {}", &sha[32..]),
        format!("Data sheets      {data_sheets}"),
        format!(
            "Format           version 0x{:04X}, symbology {} (native raster, black only)",
            desc.format_version, desc.symbology_id
        ),
        format!("Written by       deckle {tool_version}"),
    ];
    for f in &facts {
        font::draw(&mut img, margin, y, body, f);
        y += font::line_height(body);
    }
    y += mm2px(3.0);

    // ---- procedure
    heading(
        &mut img,
        margin,
        &mut y,
        text_w,
        head,
        "HOW TO READ THIS ARCHIVE",
    );
    let steps = [
        "Scan every sheet at 600 dpi or better, in 8-bit greyscale, as PNG or TIFF. \
         Turn OFF sharpening, descreening, auto-contrast and JPEG compression: they \
         destroy the small squares. Keep each sheet as its own file.",
        "Using any QR reader, read the QR squares below in order, left to right and \
         top to bottom. Each one gives a block of text. Paste them one after another, \
         in order, into a single file named tiles.txt, with no spaces or line breaks \
         between blocks.",
        "Save the short program printed under RECOVERING THE PROGRAMS as unpack.py, \
         then run: python3 unpack.py tiles.txt dkl_ref.py",
        "Check you got it right: the SHA-256 of dkl_ref.py must equal the value \
         printed beside its QR squares. On most systems: shasum -a 256 dkl_ref.py",
        "Run: python3 dkl_ref.py scan-*.png -o recovered",
        "The files appear in the folder named recovered, and the program prints \
         'document hash verified'. If it does, you are finished.",
        if programs.len() > 1 {
            "If it reports missing blocks, some sheets are damaged or lost. Recover \
             dkl_fec.py the same way from its own QR squares, then follow the \
             instructions the first program printed."
        } else {
            "If it reports missing blocks, some sheets are damaged or lost. This \
             archive was written without parity, so those blocks cannot be rebuilt: \
             rescan the sheets that failed, as carefully as you can."
        },
    ];
    for (i, s) in steps.iter().enumerate() {
        let indent = font::text_width("00. ", body);
        let lines = font::wrap(s, body, text_w - indent);
        need!(font::line_height(body) * lines.len());
        font::draw(&mut img, margin, y, body, &format!("{}.", i + 1));
        for line in &lines {
            font::draw(&mut img, margin + indent, y, body, line);
            y += font::line_height(body);
        }
        y += mm2px(1.0);
    }
    y += mm2px(2.0);

    // ---- unpack helper
    need!(font::line_height(body) * 14);
    heading(
        &mut img,
        margin,
        &mut y,
        text_w,
        head,
        "RECOVERING THE PROGRAMS (unpack.py)",
    );
    for line in font::wrap(
        "The QR text is the program, compressed and written in the 45-character \
         alphabet QR uses for text. This turns it back:",
        body,
        text_w,
    ) {
        font::draw(&mut img, margin, y, body, &line);
        y += font::line_height(body);
    }
    y += mm2px(1.5);
    for line in UNPACK_PY.lines() {
        font::draw(&mut img, margin + mm2px(4.0), y, body, line);
        y += font::line_height(body);
    }
    y += mm2px(3.0);

    // ---- page parameters
    need!(font::line_height(body) * 10);
    heading(&mut img, margin, &mut y, text_w, head, "PAGE PARAMETERS");
    for line in [
        format!(
            "Grid             {} x {} cells of {} um, {} rows per interleave band",
            desc.grid_cols, desc.grid_rows, desc.cell_um, desc.band_rows
        ),
        format!(
            "Corner markers   {} cells square; sync marks every {} cells",
            desc.fid_cells, desc.sync_period
        ),
        format!(
            "Error correction Reed-Solomon ({}, {}) over GF(2^8), {} bytes per block",
            desc.rs_n, desc.rs_k, desc.block_payload
        ),
        format!(
            "Across sheets    {} data blocks, {} parity per group of {}",
            desc.total_data_blocks, desc.fec_parity_blocks, desc.fec_data_blocks
        ),
        format!(
            "Compression      {}",
            match desc.compression {
                0 => "none",
                1 => "deflate (RFC 1951), raw stream",
                _ => "unknown",
            }
        ),
        "Full bit-level specification: docs/FORMAT.md in the deckle source.".to_string(),
    ] {
        font::draw(&mut img, margin, y, body, &line);
        y += font::line_height(body);
    }
    y += mm2px(4.0);

    // ---- the programs as QR
    for prog in programs.iter() {
        let blob = deflate(prog.source.as_bytes());
        let text = base45::encode(&blob);
        let (codes, chunks) = split_into_tiles(&text, qr_version);
        prog_meta.push((
            prog.file,
            hex(&sha256(prog.source.as_bytes())),
            text.clone(),
        ));
        let tile_px = (qr_size(qr_version) + 2 * QR_QUIET) as usize * module;
        let cols = (text_w / tile_px).max(1);

        need!(font::line_height(head) + font::line_height(body) * 4 + tile_px);
        heading(
            &mut img,
            margin,
            &mut y,
            text_w,
            head,
            &format!("PROGRAM: {}", prog.file),
        );
        let d = hex(&sha256(prog.source.as_bytes()));
        for line in [
            prog.note.to_string(),
            format!("SHA-256 of {}  {}", prog.file, &d[..32]),
            format!("{:width$}{}", "", &d[32..], width = 12 + prog.file.len()),
            format!(
                "{} QR squares, {} bytes of source. Read in order, left to right.",
                codes.len(),
                prog.source.len()
            ),
        ] {
            font::draw(&mut img, margin, y, body, &line);
            y += font::line_height(body);
        }
        y += mm2px(2.0);

        for (i, code) in codes.iter().enumerate() {
            let col = i % cols;
            if col == 0 && i > 0 {
                y += tile_px + font::line_height(body) + mm2px(2.0);
            }
            need!(tile_px + font::line_height(body));
            let x = margin + col * tile_px;
            draw_qr(&mut img, code, x, y, module);
            tiles.push(TilePlacement {
                sheet: sheet_no - 1,
                x,
                y,
                px: tile_px,
                program: prog.file,
                index: i,
                count: codes.len(),
                text: chunks[i].clone(),
            });
            font::draw(
                &mut img,
                x + QR_QUIET as usize * module,
                y + tile_px,
                body,
                &format!("{} {} of {}", prog.file, i + 1, codes.len()),
            );
        }
        y += tile_px + font::line_height(body) + mm2px(5.0);
    }

    footer(&mut img, page_w, page_h, margin, sheet_no);
    sheets.push(img);
    let total = sheets.len();
    for (i, s) in sheets.iter_mut().enumerate() {
        footer_total(s, page_w, page_h, margin, body, i + 1, total);
    }
    Bootstrap {
        sheets,
        tiles,
        programs: prog_meta,
    }
}

fn qr_size(version: u8) -> i32 {
    4 * version as i32 + 17
}

/// Largest version, and the module size to draw it at, that fits two columns.
fn choose_symbol(text_w_px: usize, dpi: f64) -> (u8, usize) {
    let target = ((TARGET_MODULE_MM * dpi / 25.4).round() as usize).max(2);
    let floor = ((MIN_MODULE_MM * dpi / 25.4).ceil() as usize).max(2);
    for v in (10u8..=40).rev() {
        let units = (qr_size(v) + 2 * QR_QUIET) as usize;
        let m = target.min(text_w_px / (2 * units));
        if m >= floor {
            return (v, m);
        }
    }
    // Paper too narrow for two columns of even a small symbol: one column, floor size.
    (10, floor)
}

const UNPACK_PY: &str = "\
import sys, zlib
A = \"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:\"
t = \"\".join(open(sys.argv[1]).read().split())
v = [A.index(c) for c in t]
b = bytearray()
for i in range(0, len(v) - 2, 3):
    n = v[i] + v[i+1]*45 + v[i+2]*45*45
    b += bytes([n >> 8, n & 255])
if len(v) % 3 == 2:
    b += bytes([v[-2] + v[-1]*45])
open(sys.argv[2], \"wb\").write(zlib.decompress(bytes(b), -15))";

/// Split Base45 text into the largest chunks that fit one fixed-version symbol.
/// Returns the symbols and the exact text each carries, so a test can prove that
/// reading them in order reproduces the payload.
fn split_into_tiles(text: &str, version: u8) -> (Vec<QrCode>, Vec<String>) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut chunks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (mut lo, mut hi) = (1usize, chars.len() - i);
        while lo < hi {
            let mid = lo + (hi - lo).div_ceil(2);
            if encode_fixed(&chars[i..i + mid], version).is_some() {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        let code = encode_fixed(&chars[i..i + lo], version)
            .expect("a single character always fits the largest QR symbol");
        out.push(code);
        chunks.push(chars[i..i + lo].iter().collect());
        i += lo;
    }
    (out, chunks)
}

fn encode_fixed(chars: &[char], version: u8) -> Option<QrCode> {
    let s: String = chars.iter().collect();
    let segs = QrSegment::make_segments(&s);
    QrCode::encode_segments_advanced(
        &segs,
        QrCodeEcc::Medium,
        Version::new(version),
        Version::new(version),
        None,
        false,
    )
    .ok()
}

fn draw_qr(img: &mut Gray, code: &QrCode, x: usize, y: usize, module: usize) {
    let n = code.size();
    for my in 0..n {
        for mx in 0..n {
            if !code.get_module(mx, my) {
                continue;
            }
            let px0 = x + (mx + QR_QUIET) as usize * module;
            let py0 = y + (my + QR_QUIET) as usize * module;
            for py in py0..(py0 + module).min(img.h) {
                for px in px0..(px0 + module).min(img.w) {
                    img.set(px, py, 0);
                }
            }
        }
    }
}

fn heading(img: &mut Gray, x: usize, y: &mut usize, w: usize, scale: usize, text: &str) {
    font::draw(img, x, *y, scale, text);
    *y += font::line_height(scale);
    rule(img, x, *y, w);
    *y += (img.h / 500).max(3) + 4;
}

fn rule(img: &mut Gray, x: usize, y: usize, w: usize) {
    let t = (img.h / 1200).max(2);
    for py in y..(y + t).min(img.h) {
        for px in x..(x + w).min(img.w) {
            img.set(px, py, 0);
        }
    }
}

fn footer(img: &mut Gray, w: usize, h: usize, margin: usize, _n: usize) {
    let _ = (w, h, margin, img);
}

fn footer_total(
    img: &mut Gray,
    _w: usize,
    h: usize,
    margin: usize,
    scale: usize,
    n: usize,
    total: usize,
) {
    let y = h - margin - font::line_height(scale);
    font::draw(
        img,
        margin,
        y,
        scale,
        &format!("DECKLE BOOTSTRAP PAGE {n} OF {total} - KEEP WITH THE ARCHIVE"),
    );
}
