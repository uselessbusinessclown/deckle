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
