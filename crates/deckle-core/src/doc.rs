//! The document pipeline (PLAN.md section 11.1).
//!
//! compress -> chunk -> cross-block FEC -> frame -> lay out -> symbol encode,
//! and its inverse. `plan` is the capacity oracle; `estimate` is `plan` with the
//! compression measured but nothing rendered, which is what makes the estimator
//! and the encoder incapable of disagreeing.

use crate::block::{Block, FLAG_PARITY};
use crate::descriptor::*;
use crate::fec;
use crate::layout::*;
use crate::raster;
use crate::sha256::sha256;
use std::io::Write;

pub const DOC_MAGIC: &[u8; 4] = b"DKL1";

#[derive(Clone, Debug)]
pub struct FileEntry {
    pub name: String,
    pub data: Vec<u8>,
}

/// Serialise the manifest and file bodies into the pre-compression stream.
pub fn build_plain_stream(files: &[FileEntry]) -> (Vec<u8>, [u8; 16]) {
    let mut ident = Vec::new();
    for f in files {
        ident.extend_from_slice(f.name.as_bytes());
        ident.extend_from_slice(&sha256(&f.data));
    }
    let idh = sha256(&ident);
    let mut uuid = [0u8; 16];
    uuid.copy_from_slice(&idh[..16]);

    let mut s = Vec::new();
    s.extend_from_slice(DOC_MAGIC);
    s.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    s.extend_from_slice(&uuid);
    s.extend_from_slice(&0i64.to_le_bytes()); // creation time: zero keeps encodes reproducible
    s.extend_from_slice(&(files.len() as u16).to_le_bytes());
    for f in files {
        let nb = f.name.as_bytes();
        s.extend_from_slice(&(nb.len() as u16).to_le_bytes());
        s.extend_from_slice(nb);
        s.extend_from_slice(&(f.data.len() as u64).to_le_bytes());
        s.extend_from_slice(&sha256(&f.data));
    }
    for f in files {
        s.extend_from_slice(&f.data);
    }
    (s, uuid)
}

pub fn parse_plain_stream(s: &[u8]) -> Option<Vec<FileEntry>> {
    if s.len() < 32 || &s[0..4] != DOC_MAGIC {
        return None;
    }
    let mut o = 30;
    let n = u16::from_le_bytes(s[o..o + 2].try_into().ok()?) as usize;
    o += 2;
    let mut meta = Vec::new();
    for _ in 0..n {
        let nl = u16::from_le_bytes(s.get(o..o + 2)?.try_into().ok()?) as usize;
        o += 2;
        let name = String::from_utf8(s.get(o..o + nl)?.to_vec()).ok()?;
        o += nl;
        let size = u64::from_le_bytes(s.get(o..o + 8)?.try_into().ok()?) as usize;
        o += 8;
        let hash: [u8; 32] = s.get(o..o + 32)?.try_into().ok()?;
        o += 32;
        meta.push((name, size, hash));
    }
    let mut out = Vec::new();
    for (name, size, hash) in meta {
        let data = s.get(o..o + size)?.to_vec();
        o += size;
        if sha256(&data) != hash {
            return None;
        }
        out.push(FileEntry { name, data });
    }
    Some(out)
}

pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(data).expect("in-memory deflate");
    e.finish().expect("in-memory deflate")
}

pub fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut d = flate2::read::DeflateDecoder::new(data);
    let mut out = Vec::new();
    d.read_to_end(&mut out).ok()?;
    Some(out)
}

// ------------------------------------------------------------------ planning

#[derive(Clone, Debug)]
pub struct DocPlan {
    pub geo: PageGeometry,
    pub payload_len: usize,
    pub blocks_per_page: usize,
    pub payload_per_block: usize,
    pub data_blocks: usize,
    pub group_data: usize,
    pub group_parity: usize,
    pub groups: usize,
    pub parity_blocks: usize,
    pub total_blocks: usize,
    pub pages: usize,
    /// Blocks actually placed on each sheet.
    ///
    /// Not `blocks_per_page`, which is the sheet's capacity. Filling sheets
    /// greedily leaves the last one nearly empty, and then losing a *full* sheet
    /// costs far more than its share of the blocks - which quietly breaks the
    /// "any 1 in N sheets" promise the estimator prints. Spreading the blocks
    /// evenly makes that promise true.
    pub blocks_per_sheet: usize,
}

/// Plan the sheet layout for a payload of known length. This is the only place
/// sheet counts are computed.
pub fn plan(cfg: &Config, payload_len: usize) -> Result<DocPlan, LayoutError> {
    let geo = PageGeometry::plan(cfg)?;
    let per_block = geo.ecc.payload();
    let blocks_per_page = geo.codewords;
    let data_blocks = payload_len.div_ceil(per_block).max(1);

    let ratio = cfg.parity_ratio.clamp(0.0, 2.0);
    let planes = cfg.ink_planes.count();
    let (group_data, group_parity, groups, parity_blocks) = if ratio <= 0.0 {
        (data_blocks.max(1), 0, 1, 0)
    } else {
        // GF(256) caps a group at 255 blocks including parity, so split and stripe.
        let max_data = ((fec::MAX_GROUP as f64) / (1.0 + ratio)).floor() as usize;
        let max_data = max_data.clamp(1, fec::MAX_GROUP - 1);
        let mut groups = data_blocks.div_ceil(max_data);
        // Blocks stripe across the parity groups with period `groups`, and across
        // the ink planes with period `planes`. If those share a factor the two
        // interleavers alias: at nine groups and three planes, every group landed
        // wholly in one ink, so losing that ink destroyed three groups outright
        // rather than costing every group a recoverable third. Keeping the
        // periods coprime costs at most one extra group.
        while gcd(groups, planes) != 1 {
            groups += 1;
        }
        let gd = data_blocks.div_ceil(groups);
        let gp = ((gd as f64 * ratio).ceil() as usize).max(1);
        (gd, gp, groups, gp * groups)
    };

    let total_blocks = data_blocks + parity_blocks;
    let pages = total_blocks.div_ceil(blocks_per_page).max(1);
    let blocks_per_sheet = total_blocks.div_ceil(pages);
    Ok(DocPlan {
        geo,
        payload_len,
        blocks_per_page,
        payload_per_block: per_block,
        data_blocks,
        group_data,
        group_parity,
        groups,
        parity_blocks,
        total_blocks,
        pages,
        blocks_per_sheet,
    })
}

fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

/// Codeword slots of a page, ordered so consecutive blocks spread across the
/// ink planes and the interleave bands.
fn codeword_order(geo: &PageGeometry) -> Vec<usize> {
    let planes = geo.ink.count();
    let max_per = geo
        .bands
        .iter()
        .map(|b| b.codewords / planes)
        .max()
        .unwrap_or(0);
    let mut order = Vec::with_capacity(geo.codewords);
    for j in 0..max_per {
        for band in &geo.bands {
            let per = band.codewords / planes;
            if j < per {
                for p in 0..planes {
                    order.push(band.first_cw + p * per + j);
                }
            }
        }
    }
    debug_assert_eq!(order.len(), geo.codewords);
    order
}

#[derive(Clone, Debug)]
pub struct Estimate {
    pub input_bytes: usize,
    pub compressed_bytes: usize,
    pub compression: u8,
    pub usable_bytes_per_sheet: usize,
    pub cells_per_sheet: usize,
    pub structural_overhead: f64,
    pub plan: DocPlan,
    pub warnings: Vec<String>,
}

impl Estimate {
    pub fn data_sheets(&self) -> usize {
        self.plan.data_blocks.div_ceil(self.plan.blocks_per_page)
    }
    pub fn parity_sheets(&self) -> usize {
        self.plan.pages - self.data_sheets()
    }
}

/// Measure compression on the real input and plan the layout around it.
pub fn estimate(cfg: &Config, files: &[FileEntry]) -> Result<Estimate, LayoutError> {
    let (plainf, _) = build_plain_stream(files);
    let (payload, comp) = compress_stream(&plainf);
    let p = plan(cfg, payload.len())?;
    let total_cells = p.geo.cols * p.geo.rows;
    let mut warnings = Vec::new();
    if p.pages > 200 {
        warnings.push(format!(
            "{} sheets is impractical; a denser cell size or a lower parity ratio would \
             cut this substantially, or this file may not belong on paper",
            p.pages
        ));
    }
    if cfg.parity_ratio > 0.0 && p.pages < 2 {
        warnings.push(
            "the archive fits on one sheet, so cross-block parity protects against damage \
             but not against losing the sheet"
                .into(),
        );
    }
    Ok(Estimate {
        input_bytes: files.iter().map(|f| f.data.len()).sum(),
        compressed_bytes: payload.len(),
        compression: comp,
        usable_bytes_per_sheet: p.geo.payload_bytes_per_page(),
        cells_per_sheet: total_cells,
        structural_overhead: 1.0 - p.geo.usable_cells as f64 / total_cells as f64,
        plan: p,
        warnings,
    })
}

/// Compress unless it does not pay, which is what stops the estimator promising
/// a sheet count it cannot meet on already-compressed input.
pub fn compress_stream(plain: &[u8]) -> (Vec<u8>, u8) {
    let z = deflate(plain);
    if (z.len() as f64) < plain.len() as f64 * 0.97 {
        (z, COMPRESSION_DEFLATE)
    } else {
        (plain.to_vec(), COMPRESSION_NONE)
    }
}

// ------------------------------------------------------------------ encoding

pub struct EncodedPage {
    pub descriptor: Descriptor,
    /// One byte per cell: black-only pages use 0 or 1; colour pages carry the
    /// ink bits of `colour::INK_*`.
    pub cells: Vec<u8>,
    pub strip: Vec<bool>,
}

impl EncodedPage {
    /// Render at the profile's nominal dpi, in whichever ink mode it was built for.
    pub fn render(&self, geo: &PageGeometry) -> crate::bitmap::Scan {
        self.render_masked(geo).0
    }

    /// Render, and report where black ink went. The degradation harness needs
    /// that to model one ink fading without erasing the black structure too.
    pub fn render_masked(
        &self,
        geo: &PageGeometry,
    ) -> (crate::bitmap::Scan, Option<crate::bitmap::Gray>) {
        if geo.ink == InkPlanes::K {
            let mono: Vec<bool> = self.cells.iter().map(|&c| c != 0).collect();
            (
                crate::bitmap::Scan::grey(raster::render(geo, &mono, &self.strip)),
                None,
            )
        } else {
            let (rgb, k) = crate::colour::render_masked(geo, &self.cells, &self.strip);
            (crate::bitmap::Scan::colour(rgb), Some(k))
        }
    }
}

pub struct Encoded {
    pub plan: DocPlan,
    pub pages: Vec<EncodedPage>,
    pub doc_uuid: [u8; 16],
    pub plain_sha256: [u8; 32],
}

pub fn encode(cfg: &Config, files: &[FileEntry]) -> Result<Encoded, LayoutError> {
    let (plainf, uuid) = build_plain_stream(files);
    let plain_hash = sha256(&plainf);
    let (payload, comp) = compress_stream(&plainf);
    let p = plan(cfg, payload.len())?;
    let ecc = p.geo.ecc;
    let per = p.payload_per_block;

    // Data blocks.
    let mut data: Vec<Block> = Vec::with_capacity(p.data_blocks);
    for i in 0..p.data_blocks {
        let mut buf = vec![0u8; per];
        let s = i * per;
        let e = ((i + 1) * per).min(payload.len());
        if s < payload.len() {
            buf[..e - s].copy_from_slice(&payload[s..e]);
        }
        data.push(Block {
            index: i as u32,
            flags: 0,
            payload: buf,
        });
    }

    // Parity blocks, one group at a time.
    let mut parity: Vec<Block> = Vec::with_capacity(p.parity_blocks);
    if p.group_parity > 0 {
        for g in 0..p.groups {
            let s = g * p.group_data;
            let e = ((g + 1) * p.group_data).min(p.data_blocks);
            let payloads: Vec<Vec<u8>> = data[s..e].iter().map(|b| b.payload.clone()).collect();
            for (j, par) in fec::encode_group(&payloads, p.group_parity)
                .into_iter()
                .enumerate()
            {
                parity.push(Block {
                    index: (p.data_blocks + g * p.group_parity + j) as u32,
                    flags: FLAG_PARITY,
                    payload: par,
                });
            }
        }
    }

    // Printed order: stripe across groups so one lost sheet costs each group
    // an equal, recoverable share.
    let mut seq: Vec<&Block> = Vec::with_capacity(p.total_blocks);
    let per_group: Vec<(usize, usize)> = (0..p.groups)
        .map(|g| {
            let ds = g * p.group_data;
            let de = ((g + 1) * p.group_data).min(p.data_blocks);
            (ds, de)
        })
        .collect();
    let longest = per_group.iter().map(|(s, e)| e - s).max().unwrap_or(0) + p.group_parity;
    for t in 0..longest {
        for g in 0..p.groups {
            let (s, e) = per_group[g];
            let dn = e - s;
            if t < dn {
                seq.push(&data[s + t]);
            } else if t - dn < p.group_parity {
                let pi = g * p.group_parity + (t - dn);
                if pi < parity.len() {
                    seq.push(&parity[pi]);
                }
            }
        }
    }
    debug_assert_eq!(seq.len(), p.total_blocks);

    // Order the page's codeword slots so that consecutive blocks land in
    // different ink planes and different bands. Filling plane 0 first would put
    // a small archive entirely in one ink, and losing that ink would then lose
    // everything - exactly what giving each plane its own codewords is meant to
    // prevent.
    let order = codeword_order(&p.geo);
    let mut pages = Vec::with_capacity(p.pages);
    for pi in 0..p.pages {
        let s = (pi * p.blocks_per_sheet).min(seq.len());
        let e = ((pi + 1) * p.blocks_per_sheet).min(seq.len());
        let real = e - s;
        let mut cws: Vec<Vec<u8>> = (0..p.blocks_per_page)
            .map(|i| raster::filler_block(ecc, (pi as u64) << 32 | i as u64).to_codeword(ecc))
            .collect();
        for (i, blk) in seq[s..e].iter().enumerate() {
            cws[order[i]] = blk.to_codeword(ecc);
        }
        let seed = (pi as u64).wrapping_mul(2_654_435_761) as u32;
        let desc = Descriptor {
            format_version: if cfg.ink_planes == InkPlanes::K {
                FORMAT_VERSION
            } else {
                FORMAT_VERSION_COLOUR
            },
            symbology_id: SYMBOLOGY_RASTER_K,
            doc_uuid: uuid,
            plain_sha256_pre: plain_hash[..8].try_into().unwrap(),
            page_index: pi as u16,
            page_count: p.pages as u16,
            // Record the size actually printed, which follows the dot count.
            cell_um: (p.geo.cell_mm * 1000.0).round() as u16,
            grid_cols: p.geo.cols as u16,
            grid_rows: p.geo.rows as u16,
            sync_period: SYNC_PERIOD as u8,
            fid_cells: p.geo.fid_cells as u8,
            interleave_seed: seed,
            rs_n: RS_N as u8,
            rs_k: ecc.k() as u8,
            block_payload: per as u8,
            seq_start: s as u32,
            block_count: real as u16,
            compression: comp,
            encryption: 0,
            fec_scheme: if p.group_parity > 0 {
                FEC_RS8_CAUCHY
            } else {
                FEC_NONE
            },
            fec_data_blocks: p.group_data as u32,
            fec_parity_blocks: p.group_parity as u32,
            total_data_blocks: p.data_blocks as u32,
            total_blocks: p.total_blocks as u32,
            payload_len: payload.len() as u32,
            render_dpi: cfg.render_dpi as u16,
            provenance: PROVENANCE_BLIND,
            flags: 0,
            band_rows: p.geo.band_rows as u16,
            ink_planes: cfg.ink_planes.code(),
            cal_period: if cfg.ink_planes == InkPlanes::K {
                0
            } else {
                CAL_PERIOD as u8
            },
            cal_patch_cells: if cfg.ink_planes == InkPlanes::K {
                0
            } else {
                CAL_BLOCK as u8
            },
            plane_reg_spec: if cfg.ink_planes == InkPlanes::K { 0 } else { 1 },
        };
        let cells = if cfg.ink_planes == InkPlanes::K {
            raster::build_cells(&p.geo, &cws, pi as u16, seed)
                .into_iter()
                .map(|b| b as u8)
                .collect()
        } else {
            crate::colour::build_cells(&p.geo, &cws, pi as u16, seed)
        };
        pages.push(EncodedPage {
            cells,
            strip: raster::build_descriptor_strip(&desc),
            descriptor: desc,
        });
    }

    Ok(Encoded {
        plan: p,
        pages,
        doc_uuid: uuid,
        plain_sha256: plain_hash,
    })
}

// ------------------------------------------------------------------ recovery

#[derive(Debug, Default)]
pub struct Recovery {
    pub pages_read: usize,
    pub pages_failed: Vec<String>,
    pub blocks_recovered: usize,
    pub blocks_from_parity: usize,
    pub blocks_missing: usize,
    pub worst_margin: f64,
    pub mean_margin: f64,
    pub files: Vec<FileEntry>,
    pub hash_ok: bool,
    pub descriptor: Option<Descriptor>,
}

impl Recovery {
    pub fn margin_band(&self) -> &'static str {
        if self.blocks_from_parity > 0 || self.worst_margin > 0.75 {
            "recovered with difficulty"
        } else if self.worst_margin > 0.40 {
            "marginal"
        } else {
            "healthy"
        }
    }
}

/// Reassemble a document from decoded pages. Pages may arrive in any order and
/// the same page may be supplied more than once; the better read of each block wins.
pub fn reassemble(pages: Vec<crate::raster::PageDecode>) -> Result<Recovery, String> {
    let mut r = Recovery::default();
    if pages.is_empty() {
        return Err("no pages decoded".into());
    }
    let desc = pages[0].descriptor.clone();
    let mut best: std::collections::HashMap<u32, (Vec<u8>, f64)> = Default::default();
    let mut margins = Vec::new();

    for pd in &pages {
        if pd.descriptor.doc_uuid != desc.doc_uuid {
            return Err(format!(
                "page {} belongs to a different document ({}), not {}",
                pd.descriptor.page_index,
                crate::sha256::hex(&pd.descriptor.doc_uuid[..4]),
                crate::sha256::hex(&desc.doc_uuid[..4])
            ));
        }
        r.pages_read += 1;
        for b in &pd.blocks {
            margins.push(b.margin);
            let e = best
                .entry(b.block.index)
                .or_insert((b.block.payload.clone(), 1.1));
            if b.margin < e.1 {
                *e = (b.block.payload.clone(), b.margin);
            }
        }
    }
    r.worst_margin = margins.iter().cloned().fold(0.0, f64::max);
    r.mean_margin = if margins.is_empty() {
        0.0
    } else {
        margins.iter().sum::<f64>() / margins.len() as f64
    };

    let nd = desc.total_data_blocks as usize;
    let gd = desc.fec_data_blocks.max(1) as usize;
    let gp = desc.fec_parity_blocks as usize;
    let groups = if gd == 0 { 1 } else { nd.div_ceil(gd) };
    let mut data: Vec<Option<Vec<u8>>> = (0..nd)
        .map(|i| best.get(&(i as u32)).map(|(p, _)| p.clone()))
        .collect();
    r.blocks_recovered = data.iter().filter(|d| d.is_some()).count();

    if gp > 0 {
        for g in 0..groups {
            let s = g * gd;
            let e = ((g + 1) * gd).min(nd);
            if data[s..e].iter().all(|d| d.is_some()) {
                continue;
            }
            let parity: Vec<Option<Vec<u8>>> = (0..gp)
                .map(|j| {
                    best.get(&((nd + g * gp + j) as u32))
                        .map(|(p, _)| p.clone())
                })
                .collect();
            let mut slice: Vec<Option<Vec<u8>>> = data[s..e].to_vec();
            match fec::decode_group(&mut slice, &parity, desc.block_payload as usize) {
                Ok(n) => {
                    r.blocks_from_parity += n;
                    data[s..e].clone_from_slice(&slice);
                }
                Err(fec::FecError::InsufficientBlocks { have, need }) => {
                    return Err(format!(
                        "group {g} has {have} of {need} blocks; not enough parity survived. \
                         Rescan any pages that failed, or the archive is short {} blocks.",
                        need - have
                    ))
                }
                Err(e) => return Err(format!("group {g}: {e:?}")),
            }
        }
    }

    r.blocks_missing = data.iter().filter(|d| d.is_none()).count();
    if r.blocks_missing > 0 {
        return Err(format!(
            "{} of {} data blocks are unrecoverable; {} pages read",
            r.blocks_missing, nd, r.pages_read
        ));
    }

    let mut payload = Vec::with_capacity(nd * desc.block_payload as usize);
    for d in data {
        payload.extend_from_slice(&d.unwrap());
    }
    payload.truncate(desc.payload_len as usize);

    let plainf = match desc.compression {
        COMPRESSION_DEFLATE => inflate(&payload).ok_or("decompression failed")?,
        COMPRESSION_NONE => payload,
        other => return Err(format!("unknown compression id {other}")),
    };
    let h = sha256(&plainf);
    r.hash_ok = h[..8] == desc.plain_sha256_pre;
    r.files = parse_plain_stream(&plainf).ok_or("manifest did not parse or a file hash failed")?;
    r.descriptor = Some(desc);
    Ok(r)
}
