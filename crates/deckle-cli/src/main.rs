//! deckle - back up data to paper.
//!
//! Subcommands: estimate, encode, decode, inspect, simulate.
//! Every subcommand accepts --json and prints the same structure the GUI and the
//! tests consume, so there is only ever one code path (PLAN.md section 10).

use deckle_core::bitmap::Scan;
use deckle_core::bootstrap;
use deckle_core::degrade::{apply_scan_masked, Degradation};
use deckle_core::doc::{self, Estimate, FileEntry};
use deckle_core::layout::{Config, Ecc, InkPlanes, Paper};
use deckle_core::pdf;
use deckle_core::raster;
use deckle_core::sha256::hex;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
deckle - back up data to paper

USAGE
  deckle estimate  <file>... [options]
  deckle encode    <file>... --out <dir> [options]
  deckle decode    <page.png>... --out <dir> [--json]
  deckle inspect   <page.png> [--json] [--verbose]
  deckle simulate  <file>... [--degrade SPEC] [options]

OPTIONS
  --paper NAME     A4 | Letter | Legal | A3 | WxH in mm      (default A4)
  --landscape
  --margin MM      margin on all four edges                  (default 12.7)
  --cell UM        cell size in micrometres                  (default 254)
  --dpi N          render resolution; cell must be a whole
                   number of dots at this resolution         (default 600)
  --ecc L|M|Q|H    symbol error correction                   (default Q)
  --parity F       cross-block parity ratio, 0 to disable    (default 0.20)
  --ink k|cm|cmy   ink planes carrying payload               (default k)
                   k   black only. The only mode rated for long-term
                       storage, and what you want on laser toner
                   cm  cyan + magenta, 2 bits per cell, about 2x.
                       Leaves out yellow, which fades first and is
                       read in the noisiest scanner channel
                   cmy all three, 3 bits per cell, about 3x. The most
                       capacity and the least durable
  --format png|pdf|both                                      (default both)
  --no-bootstrap   omit the decoding documentation            (default: on)
                   The bootstrap page carries the format, the procedure
                   and the reference decoder as QR. Without it the sheets
                   can only be read by deckle itself. Reasonable to drop
                   when the pages join an archive that already has one, or
                   when paper is scarcer than the tool.
  --out DIR        output directory
  --degrade SPEC   simulate only; comma-separated, e.g.
                   blur=0.6,noise=8,rotate=1.5,dotgain=0.15,
                   illum=0.3,blobs=200,folds=2,stain=0.05,
                   missing=0.05,scale=0.01,perspective=0.004,
                   quarters=1,mirror,invert,seed=7
  --json           machine-readable output
  --verbose        inspect only; dump decoder geometry for diagnosing
                   a page that will not read

DEGRADE SPEC UNITS
  blur and dotgain are in cell widths, so they mean the same
  thing at every density. noise is in grey levels.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("deckle: {e}");
            ExitCode::FAILURE
        }
    }
}

struct Opts {
    cfg: Config,
    out: Option<PathBuf>,
    json: bool,
    format: String,
    degrade: String,
    verbose: bool,
    bootstrap: bool,
    inputs: Vec<PathBuf>,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        cfg: Config::default(),
        out: None,
        json: false,
        format: "both".into(),
        degrade: String::new(),
        verbose: false,
        bootstrap: true,
        inputs: Vec::new(),
    };
    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        let val = |i: &mut usize| -> Result<String, String> {
            *i += 1;
            args.get(*i)
                .cloned()
                .ok_or_else(|| format!("{a} needs a value"))
        };
        match a.as_str() {
            "--paper" => {
                let v = val(&mut i)?;
                o.cfg.paper = Paper::parse(&v).ok_or(format!("unknown paper size '{v}'"))?;
            }
            "--landscape" => o.cfg.landscape = true,
            "--margin" => o.cfg.margin_mm = val(&mut i)?.parse().map_err(|_| "bad --margin")?,
            "--cell" => o.cfg.cell_um = val(&mut i)?.parse().map_err(|_| "bad --cell")?,
            "--dpi" => o.cfg.render_dpi = val(&mut i)?.parse().map_err(|_| "bad --dpi")?,
            "--ecc" => {
                let v = val(&mut i)?;
                o.cfg.ecc = Ecc::parse(&v).ok_or(format!("bad --ecc '{v}', use L M Q or H"))?;
            }
            "--parity" => o.cfg.parity_ratio = val(&mut i)?.parse().map_err(|_| "bad --parity")?,
            "--ink" => {
                let v = val(&mut i)?;
                o.cfg.ink_planes =
                    InkPlanes::parse(&v).ok_or(format!("bad --ink '{v}', use k or cmy"))?;
            }
            "--format" => o.format = val(&mut i)?,
            "--out" => o.out = Some(PathBuf::from(val(&mut i)?)),
            "--degrade" => o.degrade = val(&mut i)?,
            "--json" => o.json = true,
            "--verbose" => o.verbose = true,
            "--no-bootstrap" => o.bootstrap = false,
            s if s.starts_with('-') => return Err(format!("unknown option '{s}'")),
            s => o.inputs.push(PathBuf::from(s)),
        }
        i += 1;
    }
    if o.inputs.is_empty() {
        return Err("no input files".into());
    }
    Ok(o)
}

fn read_files(paths: &[PathBuf]) -> Result<Vec<FileEntry>, String> {
    paths
        .iter()
        .map(|p| {
            Ok(FileEntry {
                name: p
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "unnamed".into()),
                data: std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?,
            })
        })
        .collect()
}

fn run(args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    match args[0].as_str() {
        "estimate" => cmd_estimate(&o),
        "encode" => cmd_encode(&o),
        "decode" => cmd_decode(&o),
        "inspect" => cmd_inspect(&o),
        "simulate" => cmd_simulate(&o),
        other => Err(format!("unknown command '{other}'; try --help")),
    }
}

fn est_json(e: &Estimate) -> String {
    let p = &e.plan;
    format!(
        "{{\"input_bytes\":{},\"compressed_bytes\":{},\"compression\":\"{}\",\
\"usable_bytes_per_sheet\":{},\"cells_per_sheet\":{},\"structural_overhead\":{:.5},\
\"blocks_per_sheet\":{},\"payload_per_block\":{},\"data_blocks\":{},\"parity_blocks\":{},\
\"fec_groups\":{},\"group_data\":{},\"group_parity\":{},\"data_sheets\":{},\
\"parity_sheets\":{},\"total_sheets\":{},\"grid\":\"{}x{}\",\"cell_um\":{},\"ecc\":\"{}\",\
\"provenance\":\"blind\",\"warnings\":[{}]}}",
        e.input_bytes,
        e.compressed_bytes,
        if e.compression == 1 {
            "deflate"
        } else {
            "none"
        },
        e.usable_bytes_per_sheet,
        e.cells_per_sheet,
        e.structural_overhead,
        p.blocks_per_page,
        p.payload_per_block,
        p.data_blocks,
        p.parity_blocks,
        p.groups,
        p.group_data,
        p.group_parity,
        e.data_sheets(),
        e.parity_sheets(),
        p.pages,
        p.geo.cols,
        p.geo.rows,
        p.geo.cell_mm * 1000.0,
        p.geo.ecc,
        e.warnings
            .iter()
            .map(|w| format!("\"{}\"", w.replace('"', "'")))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn print_estimate(e: &Estimate, json: bool) {
    if json {
        println!("{}", est_json(e));
        return;
    }
    let p = &e.plan;
    let ratio = if e.input_bytes > 0 {
        e.compressed_bytes as f64 / e.input_bytes as f64
    } else {
        1.0
    };
    println!("Input              {} bytes", e.input_bytes);
    println!(
        "Compressed         {} bytes  ({:.1}% of input, {})",
        e.compressed_bytes,
        ratio * 100.0,
        if e.compression == 1 {
            "deflate"
        } else {
            "stored"
        }
    );
    println!(
        "Grid               {} x {} cells at {:.0} um  ({:.2}% structure)",
        p.geo.cols,
        p.geo.rows,
        p.geo.cell_mm * 1000.0,
        e.structural_overhead * 100.0
    );
    println!(
        "Per sheet          {} bytes  ({} blocks of {} at ECC {})",
        e.usable_bytes_per_sheet, p.blocks_per_page, p.payload_per_block, p.geo.ecc
    );
    println!(
        "Blocks             {} data + {} parity in {} group(s) of {}+{}",
        p.data_blocks, p.parity_blocks, p.groups, p.group_data, p.group_parity
    );
    println!(
        "Sheets             {} data + {} parity = {} total",
        e.data_sheets(),
        e.parity_sheets(),
        p.pages
    );
    if p.parity_blocks > 0 && p.pages > 1 {
        let tol = (p.pages as f64 * p.group_parity as f64 / (p.group_data + p.group_parity) as f64)
            .floor() as usize;
        println!(
            "Loss tolerance     any {} of {} sheets may be destroyed or missing",
            tol.max(0),
            p.pages
        );
    }
    println!("Ink                {}", e.plan.geo.ink.label());
    println!("Density provenance UNVERIFIED (chosen blind)");
    for w in &e.warnings {
        println!("Warning            {w}");
    }
}

fn cmd_estimate(o: &Opts) -> Result<(), String> {
    let files = read_files(&o.inputs)?;
    let e = doc::estimate(&o.cfg, &files).map_err(|e| e.to_string())?;
    print_estimate(&e, o.json);
    Ok(())
}

fn cmd_encode(o: &Opts) -> Result<(), String> {
    let out = o.out.clone().ok_or("encode needs --out DIR")?;
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let files = read_files(&o.inputs)?;
    let enc = doc::encode(&o.cfg, &files).map_err(|e| e.to_string())?;
    let geo = &enc.plan.geo;

    let mut rendered = Vec::with_capacity(enc.pages.len());
    let mut for_pdf: Vec<pdf::Page> = Vec::with_capacity(enc.pages.len());
    for p in &enc.pages {
        let (scan, black) = p.render_masked(geo);
        for_pdf.push(match (&scan.rgb, &black) {
            (Some(rgb), Some(k)) => pdf::Page::indexed_cmyk(rgb, k),
            _ => pdf::Page::Mono(scan.luma.clone()),
        });
        rendered.push(scan);
    }
    let boot = if o.bootstrap {
        bootstrap::render_sheets(
            geo,
            &enc.pages[0].descriptor,
            enc.pages.len(),
            &enc.plain_sha256,
            env!("CARGO_PKG_VERSION"),
        )
    } else {
        Vec::new()
    };

    if o.format != "pdf" {
        for (i, s) in rendered.iter().enumerate() {
            let path = out.join(format!("page-{:03}.png", i + 1));
            match &s.rgb {
                Some(c) => c.write_png(&path),
                None => s.luma.write_png(&path),
            }
            .map_err(|e| format!("{}: {e}", path.display()))?;
        }
        for (i, img) in boot.iter().enumerate() {
            let path = out.join(format!("bootstrap-{:03}.png", i + 1));
            img.write_png(&path)
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    if o.format != "png" {
        // The bootstrap sheets go last, so they end up on top when the stack is
        // turned face up.
        let mut all = for_pdf;
        all.extend(boot.iter().cloned().map(pdf::Page::Mono));
        let path = out.join("archive.pdf");
        pdf::write_pages(&path, &all, geo.page_w_mm, geo.page_h_mm)
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }

    let e = doc::estimate(&o.cfg, &files).map_err(|e| e.to_string())?;
    if o.json {
        println!(
            "{{\"pages\":{},\"bootstrap_pages\":{},\"doc_uuid\":\"{}\",\
\"plain_sha256\":\"{}\",\"out\":\"{}\",\"estimate\":{}}}",
            enc.pages.len(),
            boot.len(),
            hex(&enc.doc_uuid),
            hex(&enc.plain_sha256),
            out.display(),
            est_json(&e)
        );
    } else {
        print_estimate(&e, false);
        println!();
        println!("Document           {}", hex(&enc.doc_uuid[..8]));
        println!("Plaintext SHA-256  {}", hex(&enc.plain_sha256));
        println!(
            "Wrote              {} data sheet(s) + {} bootstrap sheet(s) to {}",
            enc.pages.len(),
            boot.len(),
            out.display()
        );
        if boot.is_empty() {
            println!();
            println!("WARNING: no bootstrap page. Without it these sheets can only be read");
            println!("         by deckle itself. Drop --no-bootstrap for a real archive.");
        }
    }
    Ok(())
}

fn decode_pages(paths: &[PathBuf]) -> (Vec<raster::PageDecode>, Vec<String>) {
    let mut ok = Vec::new();
    let mut bad = Vec::new();
    for p in paths {
        match Scan::read_png(p) {
            Err(e) => bad.push(format!("{}: {e}", p.display())),
            Ok(img) => match raster::decode_scan(&img) {
                Ok(d) => ok.push(d),
                Err(e) => bad.push(format!("{}: {e}", p.display())),
            },
        }
    }
    (ok, bad)
}

fn cmd_decode(o: &Opts) -> Result<(), String> {
    let out = o.out.clone().ok_or("decode needs --out DIR")?;
    let (pages, failed) = decode_pages(&o.inputs);
    if pages.is_empty() {
        return Err(format!("no pages decoded.\n  {}", failed.join("\n  ")));
    }
    let mut rec = doc::reassemble(pages).map_err(|e| {
        if failed.is_empty() {
            e
        } else {
            format!(
                "{e}\nPages that would not decode:\n  {}",
                failed.join("\n  ")
            )
        }
    })?;
    rec.pages_failed = failed;
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    for f in &rec.files {
        let name = Path::new(&f.name)
            .file_name()
            .map(|s| s.to_owned())
            .ok_or_else(|| format!("refusing to write suspicious name '{}'", f.name))?;
        let path = out.join(name);
        std::fs::write(&path, &f.data).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    if o.json {
        println!(
            "{{\"pages_read\":{},\"pages_failed\":{},\"blocks_recovered\":{},\
\"blocks_from_parity\":{},\"worst_margin\":{:.4},\"mean_margin\":{:.4},\"band\":\"{}\",\
\"hash_ok\":{},\"files\":[{}]}}",
            rec.pages_read,
            rec.pages_failed.len(),
            rec.blocks_recovered,
            rec.blocks_from_parity,
            rec.worst_margin,
            rec.mean_margin,
            rec.margin_band(),
            rec.hash_ok,
            rec.files
                .iter()
                .map(|f| format!("{{\"name\":\"{}\",\"bytes\":{}}}", f.name, f.data.len()))
                .collect::<Vec<_>>()
                .join(",")
        );
    } else {
        println!("Pages read         {}", rec.pages_read);
        if !rec.pages_failed.is_empty() {
            println!("Pages failed       {}", rec.pages_failed.len());
            for f in &rec.pages_failed {
                println!("                   {f}");
            }
        }
        println!("Blocks recovered   {}", rec.blocks_recovered);
        if rec.blocks_from_parity > 0 {
            println!("Rebuilt by parity  {}", rec.blocks_from_parity);
        }
        println!(
            "Correction margin  worst {:.0}%, mean {:.0}% of capacity - {}",
            rec.worst_margin * 100.0,
            rec.mean_margin * 100.0,
            rec.margin_band()
        );
        println!(
            "Plaintext hash     {}",
            if rec.hash_ok { "verified" } else { "MISMATCH" }
        );
        for f in &rec.files {
            println!("Wrote              {} ({} bytes)", f.name, f.data.len());
        }
    }
    if !rec.hash_ok {
        return Err("recovered data did not match the document hash".into());
    }
    Ok(())
}

fn cmd_inspect(o: &Opts) -> Result<(), String> {
    let scan = Scan::read_png(&o.inputs[0]).map_err(|e| e.to_string())?;
    let img = &scan.luma;
    if o.verbose {
        let p = raster::probe(img);
        println!("Image              {} x {} px", img.w, img.h);
        println!("Global threshold   {}", p.otsu);
        println!("Finder candidates  {}", p.finders.len());
        for (pt, u) in &p.finders {
            println!(
                "                   ({:8.1}, {:8.1})  unit {:.2} px",
                pt.x, pt.y, u
            );
        }
        match p.corners {
            None => println!("Corners            not resolved"),
            Some((tl, tr, bl, br)) => {
                println!(
                    "Corners            TL ({:.1},{:.1})  TR ({:.1},{:.1})",
                    tl.x, tl.y, tr.x, tr.y
                );
                println!(
                    "                   BL ({:.1},{:.1})  BR ({:.1},{:.1})",
                    bl.x, bl.y, br.x, br.y
                );
                println!("Fiducial aspect    {:.5}", p.aspect);
            }
        }
        for i in 0..p.desc_predicted.len() {
            println!(
                "Strip marker {i}     predicted ({:8.1},{:8.1})  found ({:8.1},{:8.1})",
                p.desc_predicted[i].x,
                p.desc_predicted[i].y,
                p.desc_refined[i].x,
                p.desc_refined[i].y
            );
        }
        println!(
            "Descriptor         {}",
            if p.desc_ok { "read" } else { "UNREADABLE" }
        );
        println!();
    }
    let d = raster::decode_scan(&scan).map_err(|e| e.to_string())?;
    let de = &d.descriptor;
    if o.json {
        println!(
            "{{\"doc_uuid\":\"{}\",\"page\":{},\"of\":{},\"cell_um\":{},\"grid\":\"{}x{}\",\
\"ecc_k\":{},\"blocks\":{},\"erased\":{},\"mirrored\":{},\"worst_margin\":{:.4},\
\"geometry_residual\":{:.4}}}",
            hex(&de.doc_uuid),
            de.page_index + 1,
            de.page_count,
            de.cell_um,
            de.grid_cols,
            de.grid_rows,
            de.rs_k,
            d.blocks.len(),
            d.erased,
            d.mirrored,
            d.worst_margin,
            d.geometry_residual
        );
    } else {
        println!("Document           {}", hex(&de.doc_uuid[..8]));
        println!(
            "Page               {} of {}",
            de.page_index + 1,
            de.page_count
        );
        println!(
            "Grid               {} x {} at {} um",
            de.grid_cols, de.grid_rows, de.cell_um
        );
        println!(
            "Coding             RS(255,{}), {} byte blocks",
            de.rs_k, de.block_payload
        );
        println!(
            "Blocks on page     {} read, {} unreadable",
            d.blocks.len(),
            d.erased
        );
        println!(
            "Worst margin       {:.0}% of capacity",
            d.worst_margin * 100.0
        );
        println!("Geometry residual  {:.3} cells", d.geometry_residual);
        if let Some(reg) = d.plane_registration {
            let m = d.plane_margin.unwrap_or([0.0; 3]);
            let dead = d.dead_planes.unwrap_or([false; 3]);
            let names = de
                .ink()
                .map(|i| i.plane_names())
                .unwrap_or(&["cyan", "magenta", "yellow"]);
            println!("Ink planes         {}", names.join(", "));
            for (i, name) in names.iter().enumerate() {
                println!(
                    "  {name:<16} registration {:.2} cells, margin {:.0}% of capacity{}",
                    reg[i],
                    m[i] * 100.0,
                    if dead[i] { "  - FADED BEYOND USE" } else { "" }
                );
            }
        }
        if d.mirrored {
            println!("Orientation        page was mirrored; decoded flipped");
        }
    }
    Ok(())
}

fn cmd_simulate(o: &Opts) -> Result<(), String> {
    let files = read_files(&o.inputs)?;
    let enc = doc::encode(&o.cfg, &files).map_err(|e| e.to_string())?;
    let geo = &enc.plan.geo;
    let deg = Degradation::parse(&o.degrade)?;
    let cell_px = geo.cell_dots as f64;

    let mut decoded = Vec::new();
    let mut failed = Vec::new();
    for (i, p) in enc.pages.iter().enumerate() {
        let (clean, black) = p.render_masked(geo);
        let dirty = apply_scan_masked(&clean, black.as_ref(), &deg, cell_px);
        match raster::decode_scan(&dirty) {
            Ok(d) => decoded.push(d),
            Err(e) => failed.push(format!("page {}: {e}", i + 1)),
        }
    }
    let n_pages = enc.pages.len();
    let result = if decoded.is_empty() {
        Err("no pages decoded".to_string())
    } else {
        doc::reassemble(decoded)
    };

    match result {
        Ok(rec) => {
            let ok = rec.hash_ok
                && rec.files.len() == files.len()
                && rec.files.iter().zip(&files).all(|(a, b)| a.data == b.data);
            if o.json {
                println!(
                    "{{\"ok\":{},\"pages\":{},\"pages_failed\":{},\"blocks_from_parity\":{},\
\"worst_margin\":{:.4},\"mean_margin\":{:.4},\"band\":\"{}\"}}",
                    ok,
                    n_pages,
                    failed.len(),
                    rec.blocks_from_parity,
                    rec.worst_margin,
                    rec.mean_margin,
                    rec.margin_band()
                );
            } else {
                println!(
                    "Pages              {} rendered, {} failed to decode",
                    n_pages,
                    failed.len()
                );
                for f in &failed {
                    println!("                   {f}");
                }
                if rec.blocks_from_parity > 0 {
                    println!("Rebuilt by parity  {} blocks", rec.blocks_from_parity);
                }
                println!(
                    "Correction margin  worst {:.0}%, mean {:.0}% - {}",
                    rec.worst_margin * 100.0,
                    rec.mean_margin * 100.0,
                    rec.margin_band()
                );
                println!(
                    "Round trip         {}",
                    if ok { "EXACT" } else { "MISMATCH" }
                );
            }
            if !ok {
                return Err("round trip did not reproduce the input".into());
            }
            Ok(())
        }
        Err(e) => {
            if o.json {
                println!(
                    "{{\"ok\":false,\"pages\":{},\"pages_failed\":{},\"error\":\"{}\"}}",
                    n_pages,
                    failed.len(),
                    e.replace('"', "'").replace('\n', " ")
                );
            }
            for f in &failed {
                eprintln!("  {f}");
            }
            Err(e)
        }
    }
}
