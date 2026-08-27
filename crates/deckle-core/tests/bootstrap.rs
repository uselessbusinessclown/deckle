//! The bootstrap page is the archival promise (PLAN.md 12.5, 15.2).
//!
//! The claim it makes is precise: someone with a commodity QR reader and a
//! Python interpreter can recover the decoder from the paper, check it against
//! a printed hash, and read the archive. These tests check that claim with an
//! independent QR decoder rather than with Deckle's own code.

use deckle_core::bitmap::Gray;
use deckle_core::bootstrap::{self, DKL_FEC, DKL_REF};
use deckle_core::doc::{self, FileEntry};
use deckle_core::layout::{Config, Ecc, Paper};
use deckle_core::sha256::{hex, sha256};
use deckle_core::{base45, raster};

fn archive(parity: f64) -> (deckle_core::doc::Encoded, Config) {
    let cfg = Config {
        paper: Paper::parse("105x148").unwrap(),
        parity_ratio: parity,
        ecc: Ecc::Q,
        ..Config::default()
    };
    let files = vec![FileEntry {
        name: "payload.bin".into(),
        data: (0..4000u32).map(|i| (i * 37 % 251) as u8).collect(),
    }];
    let enc = doc::encode(&cfg, &files).unwrap();
    (enc, cfg)
}

fn crop(sheet: &Gray, x: usize, y: usize, n: usize) -> Gray {
    let mut t = Gray::new(n, n, 255);
    for yy in 0..n {
        for xx in 0..n {
            if x + xx < sheet.w && y + yy < sheet.h {
                t.set(xx, yy, sheet.get(x + xx, y + yy));
            }
        }
    }
    t
}

/// Read one QR symbol with rqrr, which knows nothing about Deckle.
fn read_qr(tile: &Gray) -> String {
    let img: Vec<u8> = tile.px.clone();
    let mut prep =
        rqrr::PreparedImage::prepare_from_greyscale(tile.w, tile.h, |x, y| img[y * tile.w + x]);
    let grids = prep.detect_grids();
    assert_eq!(grids.len(), 1, "expected exactly one QR symbol in the tile");
    let (_meta, content) = grids[0].decode().expect("QR symbol did not decode");
    content
}

#[test]
fn a_commodity_qr_reader_recovers_the_programs() {
    let (enc, _) = archive(0.3);
    let b = bootstrap::render(
        &enc.plan.geo,
        &enc.pages[0].descriptor,
        enc.pages.len(),
        &enc.plain_sha256,
        "test",
    );
    assert_eq!(b.programs.len(), 2, "parity archive prints both programs");

    for (file, printed_sha, payload) in &b.programs {
        // Read this program's tiles, in printed order, with an outside decoder.
        let mut tiles: Vec<_> = b.tiles.iter().filter(|t| t.program == *file).collect();
        tiles.sort_by_key(|t| t.index);
        assert!(!tiles.is_empty());
        let mut seen = String::new();
        for t in &tiles {
            let text = read_qr(&crop(&b.sheets[t.sheet], t.x, t.y, t.px));
            assert_eq!(
                &text, &t.text,
                "{file} tile {} carried the wrong text",
                t.index
            );
            seen.push_str(&text);
        }
        assert_eq!(
            &seen, payload,
            "{file}: tiles do not concatenate to the payload"
        );

        // The printed procedure: Base45 decode, then inflate.
        let blob = base45::decode(&seen).expect("Base45 from the printed alphabet");
        let source = inflate_raw(&blob);
        let expect: &str = if file.starts_with("dkl_ref") {
            DKL_REF
        } else {
            DKL_FEC
        };
        assert_eq!(
            source,
            expect.as_bytes(),
            "{file} did not survive the round trip"
        );
        assert_eq!(
            &hex(&sha256(&source)),
            printed_sha,
            "{file}: recovered source does not match the SHA-256 printed beside it"
        );
    }
}

fn inflate_raw(data: &[u8]) -> Vec<u8> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::DeflateDecoder::new(data)
        .read_to_end(&mut out)
        .expect("deflate stream");
    out
}

#[test]
fn the_printed_programs_are_the_files_in_the_repository() {
    // include_str! makes this true at compile time; the test states it so that
    // moving the sources cannot quietly break the promise.
    let on_disk = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/dkl_ref.py"),
    )
    .expect("reference/dkl_ref.py");
    assert_eq!(on_disk, DKL_REF);
    assert!(
        DKL_REF.contains("standard library"),
        "the promise is in the docstring"
    );
    assert!(DKL_FEC.contains("Cauchy"));
}

#[test]
fn an_archive_without_parity_does_not_print_the_parity_tool() {
    let (enc, _) = archive(0.0);
    let b = bootstrap::render(
        &enc.plan.geo,
        &enc.pages[0].descriptor,
        enc.pages.len(),
        &enc.plain_sha256,
        "test",
    );
    assert_eq!(b.programs.len(), 1);
    assert!(b.tiles.iter().all(|t| t.program == "dkl_ref.py"));
}

#[test]
fn bootstrap_sheets_are_not_mistaken_for_data_sheets() {
    // A bootstrap page has no cell grid, so the decoder must reject it clearly
    // rather than returning nonsense.
    let (enc, _) = archive(0.2);
    let b = bootstrap::render(
        &enc.plan.geo,
        &enc.pages[0].descriptor,
        enc.pages.len(),
        &enc.plain_sha256,
        "test",
    );
    for sheet in &b.sheets {
        assert!(
            raster::decode_page(sheet).is_err(),
            "a bootstrap sheet must not decode as a data page"
        );
    }
}
