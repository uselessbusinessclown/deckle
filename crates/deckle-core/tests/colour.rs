//! Colour mode (PLAN.md section 18, v1.1 "Chroma").
//!
//! The properties worth asserting are the ones the design turns on: three bits
//! per cell, a plane owning its codewords so a faded ink degrades instead of
//! destroying the page, structure staying black, and a greyscale scan being
//! refused rather than guessed at.

use deckle_core::degrade::{apply_scan_masked, Degradation};
use deckle_core::doc::{self, FileEntry};
use deckle_core::layout::{Config, Ecc, InkPlanes, PageGeometry, Paper};
use deckle_core::raster;
use deckle_core::rng::Rng;

fn cfg(ink: InkPlanes, parity: f64) -> Config {
    Config {
        paper: Paper::parse("105x148").unwrap(),
        cell_um: 254,
        ecc: Ecc::Q,
        parity_ratio: parity,
        ink_planes: ink,
        ..Config::default()
    }
}

fn payload(n: usize, seed: u64) -> Vec<FileEntry> {
    let mut r = Rng::new(seed);
    vec![FileEntry {
        name: "payload.bin".into(),
        data: (0..n).map(|_| r.next_u32() as u8).collect(),
    }]
}

fn round_trip(c: &Config, files: &[FileEntry], deg: &Degradation) -> String {
    let enc = doc::encode(c, files).expect("encode");
    let geo = &enc.plan.geo;
    let mut decoded = Vec::new();
    let mut failed = Vec::new();
    for (i, p) in enc.pages.iter().enumerate() {
        let (clean, black) = p.render_masked(geo);
        let dirty = apply_scan_masked(&clean, black.as_ref(), deg, geo.cell_dots as f64);
        match raster::decode_scan(&dirty) {
            Ok(d) => decoded.push(d),
            Err(e) => failed.push(format!("page {}: {e}", i + 1)),
        }
    }
    if decoded.is_empty() {
        return format!("no pages decoded ({})", failed.join("; "));
    }
    match doc::reassemble(decoded) {
        Err(e) => format!("{e} [{}]", failed.join("; ")),
        Ok(r) if !r.hash_ok => "hash mismatch".into(),
        Ok(r) => {
            for (a, b) in r.files.iter().zip(files) {
                if a.data != b.data {
                    return "content differs".into();
                }
            }
            String::new()
        }
    }
}

#[test]
fn colour_carries_three_bits_per_cell() {
    let mono = PageGeometry::plan(&cfg(InkPlanes::K, 0.2)).unwrap();
    let col = PageGeometry::plan(&cfg(InkPlanes::Cmy, 0.2)).unwrap();
    assert_eq!(col.ink, InkPlanes::Cmy);
    assert_eq!(InkPlanes::Cmy.bits_per_cell(), 3);
    // Three bits a cell. The structure colour adds - registration strips and the
    // calibration lattice - costs about 0.5% of cells, which the per-band
    // rounding mostly absorbs, so the ratio lands at 2.9 to 3.0 in practice.
    // It cannot exceed 3: that is the ceiling an RGB scanner imposes.
    let ratio = col.payload_bytes_per_page() as f64 / mono.payload_bytes_per_page() as f64;
    assert!(
        (2.85..=3.0).contains(&ratio),
        "colour should be just under 3x mono, got {ratio:.3}"
    );
    // Codewords divide evenly across the planes, in every band.
    assert!(!col.bands.is_empty());
    for b in &col.bands {
        assert_eq!(b.codewords % 3, 0, "a plane must own whole codewords");
    }
    assert!(
        !col.cal_patches.is_empty(),
        "colour needs calibration patches"
    );
}

#[test]
fn cyan_magenta_gives_two_bits_and_no_yellow() {
    // Yellow is the weak link twice over: least lightfast, and read in the
    // noisiest scanner channel. --ink cm drops it for two thirds of the gain.
    let mono = PageGeometry::plan(&cfg(InkPlanes::K, 0.2)).unwrap();
    let cm = PageGeometry::plan(&cfg(InkPlanes::Cm, 0.2)).unwrap();
    assert_eq!(InkPlanes::Cm.bits_per_cell(), 2);
    assert_eq!(InkPlanes::parse("cm"), Some(InkPlanes::Cm));
    let ratio = cm.payload_bytes_per_page() as f64 / mono.payload_bytes_per_page() as f64;
    assert!(
        (1.9..=2.0).contains(&ratio),
        "cm should be just under 2x, got {ratio:.3}"
    );
    for b in &cm.bands {
        assert_eq!(
            b.codewords % 2,
            0,
            "each of the two planes owns whole codewords"
        );
    }

    // No cell may carry yellow.
    let enc = doc::encode(&cfg(InkPlanes::Cm, 0.2), &payload(6_000, 20)).unwrap();
    let y = deckle_core::colour::INK_Y;
    let k = deckle_core::colour::INK_K;
    assert!(
        enc.pages[0].cells.iter().all(|c| c & k != 0 || c & y == 0),
        "a cyan/magenta page must lay down no yellow ink"
    );
    assert_eq!(enc.pages[0].descriptor.ink_planes, 0b011);
    assert_eq!(
        round_trip(
            &cfg(InkPlanes::Cm, 0.2),
            &payload(25_000, 21),
            &Degradation::default()
        ),
        ""
    );
}

#[test]
fn cyan_magenta_ignores_everything_that_happens_to_yellow() {
    // The point of leaving the plane out: nothing that goes wrong with yellow,
    // or with the blue channel it is read in, can touch the archive.
    let c = cfg(InkPlanes::Cm, 0.3);
    let files = payload(15_000, 22);
    for spec in ["fadey=1.0", "bluenoise=120", "cast=0.5", "crosstalk=0.5"] {
        assert_eq!(
            round_trip(&c, &files, &Degradation::parse(spec).unwrap()),
            "",
            "cyan/magenta should be untouched by {spec}"
        );
    }
}

#[test]
fn a_lost_plane_is_rebuilt_in_cyan_magenta_too() {
    let c = cfg(InkPlanes::Cm, 1.2);
    let files = payload(15_000, 23);
    for spec in ["fadec=1.0", "fadem=1.0"] {
        assert_eq!(
            round_trip(&c, &files, &Degradation::parse(spec).unwrap()),
            "",
            "losing a plane entirely should still recover: {spec}"
        );
    }
}

#[test]
fn colour_round_trips_exactly() {
    assert_eq!(
        round_trip(
            &cfg(InkPlanes::Cmy, 0.2),
            &payload(30_000, 1),
            &Degradation::default()
        ),
        ""
    );
}

#[test]
fn colour_survives_its_own_failure_modes() {
    let c = cfg(InkPlanes::Cmy, 0.4);
    let files = payload(20_000, 2);
    let cases = [
        "blur=0.25",
        "noise=30",
        "crosstalk=0.3", // real inks are not ideal
        "regc=0.25",     // one plane lands off-register
        "reg=0.2",       // all three do
        "cast=0.2",      // scanner lamp ageing
        "bluenoise=25",  // blue is the noisiest channel in practice
        "illum=0.4",
        "fadey=0.6", // yellow fades first
        "quarters=2",
        "mirror",
    ];
    let mut bad = Vec::new();
    for spec in cases {
        let e = round_trip(&c, &files, &Degradation::parse(spec).unwrap());
        if !e.is_empty() {
            bad.push(format!("{spec}: {e}"));
        }
    }
    assert!(
        bad.is_empty(),
        "colour degradations failed:\n  {}",
        bad.join("\n  ")
    );
}

#[test]
fn a_lost_ink_plane_is_rebuilt_from_parity() {
    // The point of giving each plane its own codewords (PLAN.md 18.3): losing an
    // ink erases a third of the blocks instead of putting a wrong bit in all of
    // them. At this parity ratio that third is recoverable.
    let c = cfg(InkPlanes::Cmy, 1.2);
    let files = payload(20_000, 3);
    for (plane, spec) in [(0, "fadec=1.0"), (1, "fadem=1.0"), (2, "fadey=1.0")] {
        let e = round_trip(&c, &files, &Degradation::parse(spec).unwrap());
        assert_eq!(e, "", "losing plane {plane} entirely should still recover");
    }
}

#[test]
fn a_colour_archive_scanned_in_greyscale_is_refused() {
    // Silently decoding this would be the worst outcome: the three planes are
    // summed into one channel and cannot be pulled apart again.
    let enc = doc::encode(&cfg(InkPlanes::Cmy, 0.2), &payload(5_000, 4)).unwrap();
    let geo = &enc.plan.geo;
    let (clean, black) = enc.pages[0].render_masked(geo);
    let flat = apply_scan_masked(
        &clean,
        black.as_ref(),
        &Degradation::parse("greyscale").unwrap(),
        geo.cell_dots as f64,
    );
    assert!(
        flat.rgb.is_none(),
        "the degradation must actually flatten it"
    );
    match raster::decode_scan(&flat) {
        Err(e) => {
            let m = e.to_string();
            assert!(m.contains("greyscale"), "must name the problem: {m}");
            assert!(m.contains("Rescan"), "must say what to do: {m}");
        }
        Ok(_) => panic!("a greyscale scan of a colour page must not decode"),
    }
}

#[test]
fn structure_stays_black_on_a_colour_page() {
    // Corner markers, sync marks and the descriptor must be readable before any
    // colour calibration exists (PLAN.md 18.7), so none of them may carry ink.
    let enc = doc::encode(&cfg(InkPlanes::Cmy, 0.2), &payload(5_000, 5)).unwrap();
    let geo = &enc.plan.geo;
    let cells = &enc.pages[0].cells;
    let k = deckle_core::colour::INK_K;
    let mut structural = 0;
    for &(bx, by) in &geo.sync_marks {
        for dy in 1..3 {
            for dx in 1..3 {
                let v = cells[(by + dy) * geo.cols + bx + dx];
                assert_eq!(v, k, "a sync mark carried ink: {v:#b}");
                structural += 1;
            }
        }
    }
    assert!(structural > 100);
    // And the page must still locate and identify itself.
    let d = raster::decode_scan(&enc.pages[0].render(geo)).unwrap();
    assert_eq!(d.descriptor.ink_planes, 0b111);
    assert_eq!(d.descriptor.format_version, 0x0110);
    assert_eq!(d.dead_planes, Some([false; 3]));
    let reg = d
        .plane_registration
        .expect("per-plane registration reported");
    for r in reg {
        assert!(
            r < 0.3,
            "a clean render should register to well under a cell: {r}"
        );
    }
}

#[test]
fn a_faded_plane_is_reported_before_it_becomes_loss() {
    // PLAN.md 18.6: per-plane margin is what tells someone their yellow is going
    // while there is still time to reprint.
    let enc = doc::encode(&cfg(InkPlanes::Cmy, 0.5), &payload(20_000, 6)).unwrap();
    let geo = &enc.plan.geo;
    let (clean, black) = enc.pages[0].render_masked(geo);
    let dirty = apply_scan_masked(
        &clean,
        black.as_ref(),
        &Degradation::parse("fadey=1.0").unwrap(),
        geo.cell_dots as f64,
    );
    let d = raster::decode_scan(&dirty).expect("the black structure still reads");
    assert_eq!(d.dead_planes, Some([false, false, true]), "yellow is gone");
    let m = d.plane_margin.unwrap();
    assert!(
        m[2] > m[0] && m[2] > m[1],
        "yellow must report the worst margin: {m:?}"
    );
}

#[test]
fn black_and_colour_pages_do_not_decode_as_each_other() {
    let mono = doc::encode(&cfg(InkPlanes::K, 0.2), &payload(4_000, 7)).unwrap();
    let col = doc::encode(&cfg(InkPlanes::Cmy, 0.2), &payload(4_000, 7)).unwrap();
    assert_eq!(mono.pages[0].descriptor.format_version, 0x0100);
    assert_eq!(col.pages[0].descriptor.format_version, 0x0110);
    // A colour page rendered to greyscale is not a mono page.
    let flat = col.pages[0].render(&col.plan.geo).luma;
    assert!(raster::decode_page(&flat).is_err());
}
