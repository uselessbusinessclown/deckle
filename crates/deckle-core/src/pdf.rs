//! Minimal PDF writer for the image-mask render path (PLAN.md 4.1).
//!
//! Cell size is a whole number of device dots, so a 1:1 `/ImageMask` placed at
//! exact physical size is resampled by the RIP at an integer ratio. That keeps
//! output device-identical to the vector path while producing a file roughly
//! twenty times smaller at high density.

use crate::bitmap::Gray;
use std::io::Write;

fn flate(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).expect("in-memory zlib");
    e.finish().expect("in-memory zlib")
}

/// Pack a rendered page into a 1-bit mask, one sample per device dot.
fn pack_mask(img: &Gray) -> Vec<u8> {
    let stride = img.w.div_ceil(8);
    let mut out = vec![0u8; stride * img.h];
    for y in 0..img.h {
        for x in 0..img.w {
            if img.get(x, y) < 128 {
                out[y * stride + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    out
}

/// Write `pages` (each already rendered at `dpi`) as a PDF at exact physical size.
///
/// Object numbering is fixed up front - page `i` owns ids `3+3i` (image),
/// `4+3i` (content) and `5+3i` (page) - so every object is emitted once, in
/// order, with its true byte offset recorded. No back-patching, which is where
/// the first version of this got the cross-reference table wrong.
pub fn write_pdf(
    path: &std::path::Path,
    pages: &[Gray],
    page_w_mm: f64,
    page_h_mm: f64,
) -> std::io::Result<()> {
    let n = pages.len();
    let w_pt = page_w_mm * 72.0 / 25.4;
    let h_pt = page_h_mm * 72.0 / 25.4;
    let total_objs = 2 + 3 * n;

    let mut out: Vec<u8> = Vec::new();
    let mut offsets: Vec<usize> = Vec::with_capacity(total_objs);
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

    let begin = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, id: usize| {
        offsets.push(out.len());
        debug_assert_eq!(offsets.len(), id);
        out.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
    };

    begin(&mut out, &mut offsets, 1);
    out.extend_from_slice(b"<</Type/Catalog/Pages 2 0 R>>\nendobj\n");

    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 5 + 3 * i)).collect();
    begin(&mut out, &mut offsets, 2);
    out.extend_from_slice(
        format!(
            "<</Type/Pages/Kids[{}]/Count {n}>>\nendobj\n",
            kids.join(" ")
        )
        .as_bytes(),
    );

    for (i, img) in pages.iter().enumerate() {
        let (img_id, c_id, p_id) = (3 + 3 * i, 4 + 3 * i, 5 + 3 * i);
        let mask = flate(&pack_mask(img));

        begin(&mut out, &mut offsets, img_id);
        out.extend_from_slice(
            format!(
                "<</Type/XObject/Subtype/Image/Width {}/Height {}/ImageMask true/Decode[1 0]\
                 /BitsPerComponent 1/Filter/FlateDecode/Length {}>>\nstream\n",
                img.w,
                img.h,
                mask.len()
            )
            .as_bytes(),
        );
        out.extend_from_slice(&mask);
        out.extend_from_slice(b"\nendstream\nendobj\n");

        // Map the unit image square onto the whole page, at exact physical size.
        let content = format!("q {w_pt:.4} 0 0 {h_pt:.4} 0 0 cm /Im0 Do Q");
        begin(&mut out, &mut offsets, c_id);
        out.extend_from_slice(
            format!(
                "<</Length {}>>\nstream\n{content}\nendstream\nendobj\n",
                content.len()
            )
            .as_bytes(),
        );

        begin(&mut out, &mut offsets, p_id);
        out.extend_from_slice(
            format!(
                "<</Type/Page/Parent 2 0 R/MediaBox[0 0 {w_pt:.4} {h_pt:.4}]\
                 /Resources<</XObject<</Im0 {img_id} 0 R>>>>/Contents {c_id} 0 R>>\nendobj\n"
            )
            .as_bytes(),
        );
    }
    debug_assert_eq!(offsets.len(), total_objs);

    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", total_objs + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for &o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{xref_at}\n%%EOF\n",
            total_objs + 1
        )
        .as_bytes(),
    );
    std::fs::write(path, out)
}

/// One page of output.
pub enum Page {
    /// A black-only page: a 1-bit image mask.
    Mono(Gray),
    /// A colour page: one palette index per device pixel, over an explicit
    /// CMYK palette.
    ///
    /// Not three overlapping image masks in cyan, magenta and yellow. That looks
    /// like separations but is wrong: PDF paints opaquely, so where two inks
    /// coincide the second erases the first and a blue cell comes out magenta.
    /// Getting it right with masks would need overprint control that many
    /// renderers ignore. An indexed image states the ink combination for every
    /// pixel outright, with nothing left to interpret.
    IndexedCmyk { w: usize, h: usize, idx: Vec<u8> },
}

/// Palette: index i carries cyan in bit 0, magenta in bit 1, yellow in bit 2.
/// Index 8 is structural black, the only place K ink is used.
const CMYK_PALETTE: [[u8; 4]; 9] = [
    [0, 0, 0, 0],
    [255, 0, 0, 0],
    [0, 255, 0, 0],
    [255, 255, 0, 0],
    [0, 0, 255, 0],
    [255, 0, 255, 0],
    [0, 255, 255, 0],
    [255, 255, 255, 0],
    [0, 0, 0, 255],
];

impl Page {
    /// Build a colour page from a rendered image and the mask saying where black
    /// ink went. Under the print model an ink is absent where its channel is
    /// bright, so the index falls straight out of the three channels.
    pub fn indexed_cmyk(rgb: &crate::bitmap::Rgb, black: &Gray) -> Page {
        let mut idx = vec![0u8; rgb.w * rgb.h];
        for i in 0..rgb.w * rgb.h {
            idx[i] = if black.px[i] == 0 {
                8
            } else {
                let p = [rgb.px[i * 3], rgb.px[i * 3 + 1], rgb.px[i * 3 + 2]];
                (p[0] < 128) as u8 | ((p[1] < 128) as u8) << 1 | ((p[2] < 128) as u8) << 2
            };
        }
        Page::IndexedCmyk {
            w: rgb.w,
            h: rgb.h,
            idx,
        }
    }
}

/// Write pages at exact physical size.
///
/// Whether a commodity driver keeps the separations apart, rather than applying
/// grey component replacement and rewriting C+M+Y as K, is PLAN.md's open
/// question OQ-10 and needs a real printer to answer.
pub fn write_pages(
    path: &std::path::Path,
    pages: &[Page],
    page_w_mm: f64,
    page_h_mm: f64,
) -> std::io::Result<()> {
    let w_pt = page_w_mm * 72.0 / 25.4;
    let h_pt = page_h_mm * 72.0 / 25.4;
    // Object ids: 1 catalog, 2 page tree, then per page an image, a content
    // stream and the page object, in that order.
    let total_objs = 2 + 3 * pages.len();
    let page_obj_id: Vec<usize> = (0..pages.len()).map(|i| 5 + 3 * i).collect();

    let mut out: Vec<u8> = Vec::new();
    let mut offsets: Vec<usize> = Vec::with_capacity(total_objs);
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let begin = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, id: usize| {
        offsets.push(out.len());
        debug_assert_eq!(offsets.len(), id);
        out.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
    };

    begin(&mut out, &mut offsets, 1);
    out.extend_from_slice(b"<</Type/Catalog/Pages 2 0 R>>\nendobj\n");
    let kids: Vec<String> = page_obj_id.iter().map(|id| format!("{id} 0 R")).collect();
    begin(&mut out, &mut offsets, 2);
    out.extend_from_slice(
        format!(
            "<</Type/Pages/Kids[{}]/Count {}>>\nendobj\n",
            kids.join(" "),
            pages.len()
        )
        .as_bytes(),
    );

    let mut id = 3usize;
    for (pi, page) in pages.iter().enumerate() {
        let (dict, data) = match page {
            Page::Mono(g) => (
                format!(
                    "<</Type/XObject/Subtype/Image/Width {}/Height {}/ImageMask true\
                     /Decode[1 0]/BitsPerComponent 1/Filter/FlateDecode",
                    g.w, g.h
                ),
                flate(&pack_mask(g)),
            ),
            Page::IndexedCmyk { w, h, idx } => {
                let pal: String = CMYK_PALETTE
                    .iter()
                    .flat_map(|e| e.iter())
                    .map(|b| format!("{b:02X}"))
                    .collect();
                (
                    format!(
                        "<</Type/XObject/Subtype/Image/Width {w}/Height {h}\
                         /ColorSpace[/Indexed/DeviceCMYK {} <{pal}>]/BitsPerComponent 4\
                         /Filter/FlateDecode",
                        CMYK_PALETTE.len() - 1
                    ),
                    flate(&pack_nibbles(*w, *h, idx)),
                )
            }
        };
        begin(&mut out, &mut offsets, id);
        out.extend_from_slice(format!("{dict}/Length {}>>\nstream\n", data.len()).as_bytes());
        out.extend_from_slice(&data);
        out.extend_from_slice(b"\nendstream\nendobj\n");
        let img_id = id;
        id += 1;

        // Map the unit image square onto the whole page, at exact physical size.
        let content = format!("q {w_pt:.4} 0 0 {h_pt:.4} 0 0 cm /Im0 Do Q");
        begin(&mut out, &mut offsets, id);
        out.extend_from_slice(
            format!(
                "<</Length {}>>\nstream\n{content}\nendstream\nendobj\n",
                content.len()
            )
            .as_bytes(),
        );
        let c_id = id;
        id += 1;

        begin(&mut out, &mut offsets, id);
        debug_assert_eq!(id, page_obj_id[pi]);
        out.extend_from_slice(
            format!(
                "<</Type/Page/Parent 2 0 R/MediaBox[0 0 {w_pt:.4} {h_pt:.4}]\
                 /Resources<</XObject<</Im0 {img_id} 0 R>>>>/Contents {c_id} 0 R>>\nendobj\n"
            )
            .as_bytes(),
        );
        id += 1;
    }
    debug_assert_eq!(offsets.len(), total_objs);

    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", total_objs + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for &o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{xref_at}\n%%EOF\n",
            total_objs + 1
        )
        .as_bytes(),
    );
    std::fs::write(path, out)
}

/// Two 4-bit samples per byte, rows padded to a byte boundary.
fn pack_nibbles(w: usize, h: usize, idx: &[u8]) -> Vec<u8> {
    let stride = w.div_ceil(2);
    let mut out = vec![0u8; stride * h];
    for y in 0..h {
        for x in 0..w {
            let v = idx[y * w + x] & 0x0F;
            let o = y * stride + x / 2;
            if x % 2 == 0 {
                out[o] |= v << 4;
            } else {
                out[o] |= v;
            }
        }
    }
    out
}
