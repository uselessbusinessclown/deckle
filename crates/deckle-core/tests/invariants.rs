//! The invariant tests of PLAN.md section 15.2.
//!
//! These are the tests that catch the failures that would be worst: an estimator
//! that drifts from the encoder, a silent mis-correction, a page from another
//! document being folded into a recovery.

use deckle_core::degrade::{apply, Degradation};
use deckle_core::doc::{self, FileEntry};
use deckle_core::layout::{Config, Ecc, InkPlanes, LayoutError, Paper};
use deckle_core::raster;
use deckle_core::rng::Rng;

/// A6 at 600 dpi keeps pages small enough for CI while staying at a real density.
fn cfg(cell_um: u32, ecc: Ecc, parity: f64) -> Config {
    Config {
        paper: Paper::parse("105x148").unwrap(),
        cell_um,
        ecc,
        parity_ratio: parity,
        ..Config::default()
    }
}

fn payload(n: usize, seed: u64) -> Vec<FileEntry> {
    let mut r = Rng::new(seed);
    vec![FileEntry {
        name: "payload.bin".into(),
        // Incompressible, so the sheet count is not accidentally trivial.
        data: (0..n).map(|_| r.next_u32() as u8).collect(),
    }]
}

fn round_trip(cfg: &Config, files: &[FileEntry], deg: &Degradation, drop: &[usize]) -> String {
    let enc = doc::encode(cfg, files).expect("encode");
    let geo = &enc.plan.geo;
    let mut decoded = Vec::new();
    let mut failed = Vec::new();
    for (i, p) in enc.pages.iter().enumerate() {
        if drop.contains(&i) {
            continue;
        }
        let img = apply(
            &raster::render(geo, &p.cells, &p.strip),
            deg,
            geo.cell_dots as f64,
        );
        match raster::decode_page(&img) {
            Ok(d) => decoded.push(d),
            Err(e) => failed.push(format!("page {}: {e}", i + 1)),
        }
    }
    if decoded.is_empty() {
        return format!("no pages decoded ({})", failed.join("; "));
    }
    match doc::reassemble(decoded) {
        Err(e) => format!("{e} [{}]", failed.join("; ")),
        Ok(rec) => {
            if !rec.hash_ok {
                return "hash mismatch".into();
            }
            if rec.files.len() != files.len() {
                return format!(
                    "recovered {} files, expected {}",
                    rec.files.len(),
                    files.len()
                );
            }
            for (a, b) in rec.files.iter().zip(files) {
                if a.data != b.data || a.name != b.name {
                    return format!("file '{}' differs", b.name);
                }
            }
            String::new()
        }
    }
}

#[test]
fn estimator_equals_encoder() {
    // PLAN.md fixed decision 6: the estimator IS the layout engine. If these
    // ever disagree, users print the wrong number of sheets.
    for &cell in &[254u32, 212, 169] {
        for &ecc in &[Ecc::L, Ecc::M, Ecc::Q, Ecc::H] {
            for &parity in &[0.0f64, 0.2, 0.5] {
                for &bytes in &[1usize, 5_000, 40_000] {
                    let c = cfg(cell, ecc, parity);
                    let files = payload(bytes, bytes as u64 + cell as u64);
                    let est = doc::estimate(&c, &files).expect("estimate");
                    let enc = doc::encode(&c, &files).expect("encode");
                    assert_eq!(
                        est.plan.pages,
                        enc.pages.len(),
                        "cell={cell} ecc={ecc} parity={parity} bytes={bytes}"
                    );
                    assert_eq!(est.plan.total_blocks, enc.plan.total_blocks);
                }
            }
        }
    }
}

#[test]
fn clean_round_trip_is_exact() {
    let c = cfg(254, Ecc::Q, 0.2);
    let files = payload(25_000, 1);
    assert_eq!(round_trip(&c, &files, &Degradation::default(), &[]), "");
}

#[test]
fn round_trip_without_parity() {
    let c = cfg(254, Ecc::Q, 0.0);
    let files = payload(8_000, 2);
    assert_eq!(round_trip(&c, &files, &Degradation::default(), &[]), "");
}

#[test]
fn multiple_files_round_trip() {
    let c = cfg(254, Ecc::M, 0.2);
    let mut files = payload(6_000, 3);
    files.push(FileEntry {
        name: "notes.txt".into(),
        data: b"deckle multi-file test\n".repeat(40).to_vec(),
    });
    assert_eq!(round_trip(&c, &files, &Degradation::default(), &[]), "");
}

#[test]
fn survives_the_degradation_matrix() {
    let c = cfg(254, Ecc::Q, 0.2);
    let files = payload(12_000, 4);
    // Each entry is a degradation the decoder claims to survive (PLAN.md 15.1).
    // Blur and dot gain are in cell widths, so they mean the same thing at any
    // density; noise is in grey levels.
    let cases = [
        "blur=0.3",
        "noise=25",
        "rotate=2.5",
        "rotate=-1.0",
        "scale=0.012",
        "perspective=0.006",
        "dotgain=0.2",
        "dotgain=-0.2",
        "illum=0.4",
        "blobs=250",
        "folds=3",
        "stain=0.06",
        "quarters=1",
        "quarters=2",
        "quarters=3",
        "mirror",
        "blur=0.25,noise=12,rotate=1.0,illum=0.3,dotgain=0.1,blobs=120",
    ];
    let mut bad = Vec::new();
    for spec in cases {
        let d = Degradation::parse(spec).unwrap();
        let e = round_trip(&c, &files, &d, &[]);
        if !e.is_empty() {
            bad.push(format!("{spec}: {e}"));
        }
    }
    assert!(
        bad.is_empty(),
        "degradations failed:\n  {}",
        bad.join("\n  ")
    );
}

#[test]
fn recovers_a_lost_page_at_sufficient_parity() {
    // With parity at 0.6 the archive survives losing any one sheet outright.
    let c = cfg(254, Ecc::Q, 0.6);
    let files = payload(28_000, 5);
    let pages = doc::estimate(&c, &files).unwrap().plan.pages;
    assert!(pages >= 3, "expected a multi-page archive, got {pages}");
    for lost in 0..pages {
        assert_eq!(
            round_trip(&c, &files, &Degradation::default(), &[lost]),
            "",
            "losing page {lost} of {pages} should still recover"
        );
    }
}

#[test]
fn reports_rather_than_guesses_when_parity_is_exhausted() {
    // Losing a sheet with no parity must fail loudly, never silently truncate.
    let c = cfg(254, Ecc::Q, 0.0);
    let files = payload(28_000, 6);
    let pages = doc::estimate(&c, &files).unwrap().plan.pages;
    assert!(pages >= 3);
    let e = round_trip(&c, &files, &Degradation::default(), &[1]);
    assert!(!e.is_empty(), "must not claim success");
    assert!(
        e.contains("unrecoverable") || e.contains("blocks"),
        "error should say what is missing, got: {e}"
    );
}

#[test]
fn rejects_pages_from_another_document() {
    let c = cfg(254, Ecc::Q, 0.2);
    let a = doc::encode(&c, &payload(6_000, 7)).unwrap();
    let b = doc::encode(&c, &payload(6_000, 8)).unwrap();
    let geo = &a.plan.geo;
    let mut pages = Vec::new();
    for src in [&a.pages[0], &b.pages[0]] {
        let img = raster::render(geo, &src.cells, &src.strip);
        pages.push(raster::decode_page(&img).expect("decode"));
    }
    let err = doc::reassemble(pages).expect_err("mixed documents must be refused");
    assert!(err.contains("different document"), "got: {err}");
}

#[test]
fn encoding_is_deterministic() {
    let c = cfg(254, Ecc::Q, 0.3);
    let files = payload(9_000, 9);
    let a = doc::encode(&c, &files).unwrap();
    let b = doc::encode(&c, &files).unwrap();
    assert_eq!(a.doc_uuid, b.doc_uuid);
    assert_eq!(a.pages.len(), b.pages.len());
    for (x, y) in a.pages.iter().zip(&b.pages) {
        assert_eq!(x.cells, y.cells);
        assert_eq!(x.strip, y.strip);
    }
}

#[test]
fn descriptor_carries_everything_the_decoder_needs() {
    // PLAN.md fixed decision 3: decoding must need no user-supplied configuration.
    // The decoder below is given only pixels.
    for &(cell, ecc) in &[(254u32, Ecc::L), (212, Ecc::Q), (169, Ecc::H)] {
        let c = cfg(cell, ecc, 0.2);
        let files = payload(4_000, cell as u64);
        let enc = doc::encode(&c, &files).unwrap();
        let img = raster::render(&enc.plan.geo, &enc.pages[0].cells, &enc.pages[0].strip);
        let d = raster::decode_page(&img).expect("decode");
        assert_eq!(d.descriptor.rs_k as usize, ecc.k());
        assert_eq!(d.descriptor.grid_cols as usize, enc.plan.geo.cols);
        assert_eq!(d.descriptor.page_count as usize, enc.plan.pages);
        assert!(!d.blocks.is_empty());
        assert_eq!(d.erased, 0, "clean page should have no erased codewords");
    }
}

#[test]
fn structure_cells_never_carry_payload() {
    use deckle_core::layout::{is_reserved_at, usable_cells_for, PageGeometry};
    let c = cfg(254, Ecc::Q, 0.2);
    let geo = PageGeometry::plan(&c).unwrap();
    let mut counted = 0usize;
    for y in 0..geo.rows {
        for x in 0..geo.cols {
            if !is_reserved_at(geo.cols, geo.rows, geo.fid_cells, x, y) {
                counted += 1;
            }
        }
    }
    assert_eq!(counted, geo.usable_cells);
    assert_eq!(
        counted,
        usable_cells_for(geo.cols, geo.rows, geo.fid_cells),
        "the decoder recomputes this from the descriptor; it must match"
    );
}

#[test]
fn pdf_cross_reference_table_is_correct() {
    // CoreGraphics rejects a PDF whose xref offsets are wrong, and a structural
    // smoke test that only greps for keywords will not notice. Walk the table.
    let dir = std::env::temp_dir().join(format!("deckle-pdf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("a.pdf");
    let c = cfg(254, Ecc::Q, 0.0);
    let enc = doc::encode(&c, &payload(20_000, 10)).unwrap();
    let geo = &enc.plan.geo;
    let imgs: Vec<_> = enc
        .pages
        .iter()
        .map(|p| raster::render(geo, &p.cells, &p.strip))
        .collect();
    assert!(imgs.len() >= 2, "want a multi-page PDF");
    deckle_core::pdf::write_pdf(&path, &imgs, geo.page_w_mm, geo.page_h_mm).unwrap();
    let bytes = std::fs::read(&path).unwrap();

    assert!(bytes.starts_with(b"%PDF-1.4"));
    assert!(bytes.ends_with(b"%%EOF\n"));

    let start = bytes.windows(9).rposition(|w| w == b"startxref").unwrap();
    let tail = String::from_utf8_lossy(&bytes[start + 9..]).to_string();
    let xref_at: usize = tail.trim().lines().next().unwrap().trim().parse().unwrap();
    assert!(
        bytes[xref_at..].starts_with(b"xref\n"),
        "startxref must point at the table"
    );

    let table = String::from_utf8_lossy(&bytes[xref_at..]).to_string();
    let mut lines = table.lines();
    lines.next(); // "xref"
    let header = lines.next().unwrap();
    let count: usize = header.split_whitespace().nth(1).unwrap().parse().unwrap();
    assert_eq!(
        count,
        2 + 3 * imgs.len() + 1,
        "one free entry plus every object"
    );

    let free = lines.next().unwrap();
    assert!(free.starts_with("0000000000 65535 f"));
    for id in 1..count {
        let entry = lines
            .next()
            .unwrap_or_else(|| panic!("missing entry for object {id}"));
        let off: usize = entry.split_whitespace().next().unwrap().parse().unwrap();
        let want = format!("{id} 0 obj");
        assert!(
            bytes[off..].starts_with(want.as_bytes()),
            "xref entry {id} points at offset {off}, which is not '{want}'"
        );
    }

    // Every page must reference a real image XObject at exact physical size.
    let text = String::from_utf8_lossy(&bytes);
    assert_eq!(text.matches("/Type/Page/Parent 2 0 R").count(), imgs.len());
    assert_eq!(text.matches("/ImageMask true").count(), imgs.len());
    let w_pt = geo.page_w_mm * 72.0 / 25.4;
    assert!(text.contains(&format!("/MediaBox[0 0 {w_pt:.4}")));

    // Stream lengths must be exact, or a strict parser stops at the first one.
    // Scan the raw bytes: a PDF is not valid UTF-8, so offsets taken from a
    // lossy string land in the wrong place once binary stream data is involved.
    let find = |hay: &[u8], needle: &[u8], from: usize| -> Option<usize> {
        hay[from..]
            .windows(needle.len())
            .position(|w| w == needle)
            .map(|i| i + from)
    };
    let mut at = 0usize;
    let mut checked = 0usize;
    while let Some(i) = find(&bytes, b"/Length ", at) {
        at = i + 8;
        let digits: String = bytes[at..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .map(|&b| b as char)
            .collect();
        let Ok(len) = digits.parse::<usize>() else {
            continue;
        };
        let Some(s_at) = find(&bytes, b"stream\n", at) else {
            continue;
        };
        if s_at - at > 400 {
            continue; // a chance byte sequence inside compressed data
        }
        let data = s_at + 7;
        assert!(
            bytes[data + len..].starts_with(b"\nendstream"),
            "/Length {len} does not reach exactly to endstream"
        );
        checked += 1;
        at = data + len;
    }
    assert_eq!(
        checked,
        2 * imgs.len(),
        "every image and content stream checked"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn black_is_the_default_and_colour_is_refused_clearly() {
    // PLAN.md 18.8: colour is never the default, at any tier, on any medium.
    assert_eq!(Config::default().ink_planes, InkPlanes::K);
    assert_eq!(InkPlanes::parse("k"), Some(InkPlanes::K));
    // CMYK is a common way to ask for it; accept the word, then explain.
    assert_eq!(InkPlanes::parse("cmyk"), Some(InkPlanes::Cmy));
    assert_eq!(InkPlanes::parse("sepia"), None);

    let mut c = cfg(254, Ecc::Q, 0.2);
    c.ink_planes = InkPlanes::Cmy;
    let files = payload(1_000, 21);
    // Every entry point must refuse, not silently print black.
    assert!(matches!(
        deckle_core::layout::PageGeometry::plan(&c),
        Err(LayoutError::ColourNotBuilt)
    ));
    assert!(matches!(
        doc::estimate(&c, &files),
        Err(LayoutError::ColourNotBuilt)
    ));
    assert!(matches!(
        doc::encode(&c, &files),
        Err(LayoutError::ColourNotBuilt)
    ));

    let msg = LayoutError::ColourNotBuilt.to_string();
    assert!(msg.contains("not implemented"), "must not imply it works");
    assert!(msg.contains("section 18"), "must say where it is designed");
    assert!(
        msg.contains("not CMYK"),
        "must explain why three planes, not four"
    );
}
