//! Render a bootstrap page set and write each QR tile as its own PNG, plus a
//! manifest, so an external reader can be pointed at them one at a time.
use deckle_core::{bitmap::Gray, bootstrap, doc, layout::Config};
fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let cfg = Config::default();
    let files = vec![doc::FileEntry {
        name: "t".into(),
        data: vec![3u8; 5000],
    }];
    let enc = doc::encode(&cfg, &files).unwrap();
    let b = bootstrap::render(
        &enc.plan.geo,
        &enc.pages[0].descriptor,
        enc.pages.len(),
        &enc.plain_sha256,
        "test",
    );
    let mut man = String::new();
    for (i, t) in b.tiles.iter().enumerate() {
        let src = &b.sheets[t.sheet];
        let mut tile = Gray::new(t.px, t.px, 255);
        for y in 0..t.px {
            for x in 0..t.px {
                if t.x + x < src.w && t.y + y < src.h {
                    tile.set(x, y, src.get(t.x + x, t.y + y));
                }
            }
        }
        let p = out.join(format!("tile-{i:03}.png"));
        tile.write_png(&p).unwrap();
        man += &format!("{i}\t{}\t{}\t{}\n", t.program, t.index, t.count);
    }
    for (name, sha, text) in &b.programs {
        std::fs::write(out.join(format!("{name}.sha256")), sha).unwrap();
        std::fs::write(out.join(format!("{name}.base45")), text).unwrap();
    }
    std::fs::write(out.join("manifest.tsv"), man).unwrap();
    println!("{} sheets, {} tiles", b.sheets.len(), b.tiles.len());
}
