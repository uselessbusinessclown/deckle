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

/// Write data pages (which may be colour) followed by black-only bootstrap
/// sheets.
///
/// Colour pages go out as three 1-bit image masks painted in DeviceCMYK cyan,
/// magenta and yellow: true separations rather than an RGB image the driver
/// would be free to reinterpret. Whether a commodity driver honours that,
/// instead of applying grey component replacement and rewriting C+M+Y as K, is
/// PLAN.md's open question OQ-10 and needs a real printer to answer.
pub fn write_pages(
    path: &std::path::Path,
    pages: &[crate::bitmap::Scan],
    bootstrap: &[Gray],
    page_w_mm: f64,
    page_h_mm: f64,
) -> std::io::Result<()> {
    let mut all: Vec<Page> = Vec::new();
    for s in pages {
        all.push(match &s.rgb {
            None => Page::Mono(s.luma.clone()),
            Some(c) => {
                // Each ink is dark in exactly one channel under the print model.
                let sep = std::array::from_fn(|ch| {
                    let mut g = Gray::new(c.w, c.h, 255);
                    for y in 0..c.h {
                        for x in 0..c.w {
                            // Structural black prints in all three separations.
                            g.set(x, y, c.get(x, y)[ch]);
                        }
                    }
                    g
                });
                Page::Separations(sep)
            }
        });
    }
    all.extend(bootstrap.iter().cloned().map(Page::Mono));
    write_mixed(path, &all, page_w_mm, page_h_mm)
}

pub enum Page {
    Mono(Gray),
    /// Cyan, magenta and yellow separations, each dark where that ink prints.
    Separations([Gray; 3]),
}

impl Page {
    fn images(&self) -> usize {
        match self {
            Page::Mono(_) => 1,
            Page::Separations(_) => 3,
        }
    }
}

fn write_mixed(
    path: &std::path::Path,
    pages: &[Page],
    page_w_mm: f64,
    page_h_mm: f64,
) -> std::io::Result<()> {
    let w_pt = page_w_mm * 72.0 / 25.4;
    let h_pt = page_h_mm * 72.0 / 25.4;
    // Object ids: 1 catalog, 2 page tree, then per page its images, its content
    // stream and its page object, all in order.
    let mut page_obj_id = Vec::with_capacity(pages.len());
    let mut next = 3usize;
    for p in pages {
        next += p.images();
        next += 1; // content
        page_obj_id.push(next);
        next += 1;
    }
    let total_objs = next - 1;

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
        let masks: Vec<&Gray> = match page {
            Page::Mono(g) => vec![g],
            Page::Separations(s) => s.iter().collect(),
        };
        let mut img_ids = Vec::new();
        for g in &masks {
            let data = flate(&pack_mask(g));
            begin(&mut out, &mut offsets, id);
            out.extend_from_slice(
                format!(
                    "<</Type/XObject/Subtype/Image/Width {}/Height {}/ImageMask true\
                     /Decode[1 0]/BitsPerComponent 1/Filter/FlateDecode/Length {}>>\nstream\n",
                    g.w,
                    g.h,
                    data.len()
                )
                .as_bytes(),
            );
            out.extend_from_slice(&data);
            out.extend_from_slice(b"\nendstream\nendobj\n");
            img_ids.push(id);
            id += 1;
        }
        let mut content = String::new();
        for (k, iid) in img_ids.iter().enumerate() {
            let ink = match (page, k) {
                (Page::Mono(_), _) => "0 0 0 1 k",
                (Page::Separations(_), 0) => "1 0 0 0 k",
                (Page::Separations(_), 1) => "0 1 0 0 k",
                _ => "0 0 1 0 k",
            };
            let _ = iid;
            content += &format!("q {ink} {w_pt:.4} 0 0 {h_pt:.4} 0 0 cm /Im{k} Do Q\n");
        }
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
        let xobjects: Vec<String> = img_ids
            .iter()
            .enumerate()
            .map(|(k, i)| format!("/Im{k} {i} 0 R"))
            .collect();
        begin(&mut out, &mut offsets, id);
        debug_assert_eq!(id, page_obj_id[pi]);
        out.extend_from_slice(
            format!(
                "<</Type/Page/Parent 2 0 R/MediaBox[0 0 {w_pt:.4} {h_pt:.4}]\
                 /Resources<</XObject<<{}>>>>/Contents {c_id} 0 R>>\nendobj\n",
                xobjects.join("")
            )
            .as_bytes(),
        );
        id += 1;
    }

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
