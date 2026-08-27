//! The page descriptor: everything a decoder needs, carried on the page itself.
//!
//! PLAN.md fixed decision 3 requires that the decoder never asks the user for
//! configuration. This 96-byte record is how that promise is kept.
//!
//! PROTOTYPE DEVIATION: the specification calls for the descriptor to travel in
//! a standard QR symbol so a commodity reader can extract it. This prototype
//! carries it in a low-density K-only raster strip (see `raster::descriptor`)
//! protected by RS(255,127). The payload layout below is what a QR would carry,
//! so swapping the carrier later does not change this module.

use crate::crc::crc32c;
use crate::layout::{Ecc, RS_N};

pub const MAGIC: &[u8; 4] = b"DKLP";
pub const FORMAT_VERSION: u16 = 0x0100;
pub const SYMBOLOGY_RASTER_K: u16 = 1;
pub const DESC_LEN: usize = 96;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub format_version: u16,
    pub symbology_id: u16,
    pub doc_uuid: [u8; 16],
    pub plain_sha256_pre: [u8; 8],
    pub page_index: u16,
    pub page_count: u16,
    pub cell_um: u16,
    pub grid_cols: u16,
    pub grid_rows: u16,
    pub sync_period: u8,
    pub fid_cells: u8,
    /// Page-level interleave seed. Per-band coefficients are derived from each
    /// band's own cell count, so only this seed has to travel.
    pub interleave_seed: u32,
    pub rs_n: u8,
    pub rs_k: u8,
    pub block_payload: u8,
    /// Offset of this page's first block within the printed block sequence.
    /// Blocks are self-identifying, so this is reporting detail, not a decode input.
    pub seq_start: u32,
    pub block_count: u16,
    pub compression: u8,
    pub encryption: u8,
    pub fec_scheme: u8,
    pub fec_data_blocks: u32,
    pub fec_parity_blocks: u32,
    pub total_data_blocks: u32,
    pub total_blocks: u32,
    pub payload_len: u64,
    pub render_dpi: u16,
    pub provenance: u8,
    pub flags: u8,
    pub band_rows: u16,
}

pub const COMPRESSION_NONE: u8 = 0;
pub const COMPRESSION_DEFLATE: u8 = 1;
pub const FEC_NONE: u8 = 0;
pub const FEC_RS8_CAUCHY: u8 = 1;
pub const PROVENANCE_BLIND: u8 = 0;

macro_rules! put {
    ($b:expr, $off:expr, $v:expr) => {{
        let bytes = $v.to_le_bytes();
        $b[$off..$off + bytes.len()].copy_from_slice(&bytes);
    }};
}
macro_rules! get {
    ($b:expr, $off:expr, $t:ty) => {{
        const N: usize = std::mem::size_of::<$t>();
        let mut a = [0u8; N];
        a.copy_from_slice(&$b[$off..$off + N]);
        <$t>::from_le_bytes(a)
    }};
}

impl Descriptor {
    pub fn encode(&self) -> [u8; DESC_LEN] {
        let mut b = [0u8; DESC_LEN];
        b[0..4].copy_from_slice(MAGIC);
        put!(b, 4, self.format_version);
        put!(b, 6, self.symbology_id);
        b[8..24].copy_from_slice(&self.doc_uuid);
        b[24..32].copy_from_slice(&self.plain_sha256_pre);
        put!(b, 32, self.page_index);
        put!(b, 34, self.page_count);
        put!(b, 36, self.cell_um);
        put!(b, 38, self.grid_cols);
        put!(b, 40, self.grid_rows);
        b[42] = self.sync_period;
        b[43] = self.fid_cells;
        put!(b, 44, self.interleave_seed);
        // 48..52 reserved, zero.
        b[52] = self.rs_n;
        b[53] = self.rs_k;
        b[54] = self.block_payload;
        put!(b, 55, self.seq_start);
        put!(b, 59, self.block_count);
        b[61] = self.compression;
        b[62] = self.encryption;
        b[63] = self.fec_scheme;
        put!(b, 64, self.fec_data_blocks);
        put!(b, 68, self.fec_parity_blocks);
        put!(b, 72, self.total_data_blocks);
        put!(b, 76, self.total_blocks);
        put!(b, 80, self.payload_len);
        put!(b, 88, self.render_dpi);
        b[90] = self.provenance;
        b[91] = self.flags;
        put!(b, 92, self.band_rows);
        // 94..96 reserved, zero.
        b
    }

    pub fn decode(b: &[u8]) -> Option<Descriptor> {
        if b.len() < DESC_LEN || &b[0..4] != MAGIC {
            return None;
        }
        Some(Descriptor {
            format_version: get!(b, 4, u16),
            symbology_id: get!(b, 6, u16),
            doc_uuid: b[8..24].try_into().ok()?,
            plain_sha256_pre: b[24..32].try_into().ok()?,
            page_index: get!(b, 32, u16),
            page_count: get!(b, 34, u16),
            cell_um: get!(b, 36, u16),
            grid_cols: get!(b, 38, u16),
            grid_rows: get!(b, 40, u16),
            sync_period: b[42],
            fid_cells: b[43],
            interleave_seed: get!(b, 44, u32),
            rs_n: b[52],
            rs_k: b[53],
            block_payload: b[54],
            seq_start: get!(b, 55, u32),
            block_count: get!(b, 59, u16),
            compression: b[61],
            encryption: b[62],
            fec_scheme: b[63],
            fec_data_blocks: get!(b, 64, u32),
            fec_parity_blocks: get!(b, 68, u32),
            total_data_blocks: get!(b, 72, u32),
            total_blocks: get!(b, 76, u32),
            payload_len: get!(b, 80, u64),
            render_dpi: get!(b, 88, u16),
            provenance: b[90],
            flags: b[91],
            band_rows: get!(b, 92, u16),
        })
    }

    pub fn ecc(&self) -> Option<Ecc> {
        if self.rs_n as usize != RS_N {
            return None;
        }
        Ecc::from_k(self.rs_k as usize)
    }

    /// Descriptor plus CRC-32C, padded to the RS(255,127) message length.
    pub fn to_message(&self) -> Vec<u8> {
        let d = self.encode();
        let mut m = vec![0u8; crate::layout::DESC_RS_K];
        m[..DESC_LEN].copy_from_slice(&d);
        let c = crc32c(&d);
        m[DESC_LEN..DESC_LEN + 4].copy_from_slice(&c.to_le_bytes());
        m
    }

    pub fn from_message(m: &[u8]) -> Option<Descriptor> {
        if m.len() < DESC_LEN + 4 {
            return None;
        }
        let want = u32::from_le_bytes(m[DESC_LEN..DESC_LEN + 4].try_into().ok()?);
        if crc32c(&m[..DESC_LEN]) != want {
            return None;
        }
        Descriptor::decode(&m[..DESC_LEN])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Descriptor {
        Descriptor {
            format_version: FORMAT_VERSION,
            symbology_id: SYMBOLOGY_RASTER_K,
            doc_uuid: [7u8; 16],
            plain_sha256_pre: [9u8; 8],
            page_index: 3,
            page_count: 11,
            cell_um: 254,
            grid_cols: 726,
            grid_rows: 970,
            sync_period: 32,
            fid_cells: 27,
            interleave_seed: 435_437,
            rs_n: 255,
            rs_k: 191,
            block_payload: 183,
            seq_start: 1_234,
            block_count: 336,
            compression: COMPRESSION_DEFLATE,
            encryption: 0,
            fec_scheme: FEC_RS8_CAUCHY,
            fec_data_blocks: 200,
            fec_parity_blocks: 40,
            total_data_blocks: 2_000,
            total_blocks: 2_400,
            payload_len: 1_234_567,
            render_dpi: 600,
            provenance: PROVENANCE_BLIND,
            flags: 0,
            band_rows: 128,
        }
    }

    #[test]
    fn round_trips_through_the_message_frame() {
        let d = sample();
        let m = d.to_message();
        assert_eq!(Descriptor::from_message(&m).unwrap(), d);
    }

    #[test]
    fn rejects_a_corrupted_message() {
        let d = sample();
        let mut m = d.to_message();
        m[40] ^= 0x01;
        assert!(Descriptor::from_message(&m).is_none());
    }
}
