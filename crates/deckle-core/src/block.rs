//! Block framing (PLAN.md section 5.4).
//!
//! ```text
//!  byte  0..2   block_index   u24 little-endian, unique within the document
//!  byte  3      flags         bit0 = parity block, bit1 = page filler
//!  byte  4..7   crc32c        over bytes 0..3 and 8..k-1
//!  byte  8..k-1 payload
//!  byte  k..254 Reed-Solomon parity
//! ```
//!
//! The index and CRC inside every block are what make a partial read useful: a
//! recovered codeword identifies and validates itself without the page
//! descriptor and without any other block.

use crate::crc::crc32c;
use crate::gf256::{rs_decode, rs_encode_parity, RsError};
use crate::layout::{Ecc, BLOCK_HEADER, RS_N};

pub const FLAG_PARITY: u8 = 0x01;
pub const FLAG_FILLER: u8 = 0x02;
pub const FILLER_INDEX: u32 = 0x00FF_FFFF;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub index: u32,
    pub flags: u8,
    pub payload: Vec<u8>,
}

impl Block {
    pub fn is_parity(&self) -> bool {
        self.flags & FLAG_PARITY != 0
    }
    pub fn is_filler(&self) -> bool {
        self.flags & FLAG_FILLER != 0 || self.index == FILLER_INDEX
    }

    /// Frame and Reed-Solomon encode into a full 255-symbol codeword.
    pub fn to_codeword(&self, ecc: Ecc) -> Vec<u8> {
        let k = ecc.k();
        assert_eq!(self.payload.len(), ecc.payload(), "payload length mismatch");
        let mut msg = vec![0u8; k];
        msg[0..3].copy_from_slice(&self.index.to_le_bytes()[0..3]);
        msg[3] = self.flags;
        msg[BLOCK_HEADER..].copy_from_slice(&self.payload);
        let mut crc_input = Vec::with_capacity(4 + self.payload.len());
        crc_input.extend_from_slice(&msg[0..4]);
        crc_input.extend_from_slice(&self.payload);
        msg[4..8].copy_from_slice(&crc32c(&crc_input).to_le_bytes());
        let parity = rs_encode_parity(&msg, ecc.nsym());
        msg.extend(parity);
        debug_assert_eq!(msg.len(), RS_N);
        msg
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockError {
    Rs,
    Crc,
}

#[derive(Debug, Clone)]
pub struct BlockDecode {
    pub block: Block,
    /// Symbols altered by Reed-Solomon, as a fraction of correction capacity.
    pub margin: f64,
    pub corrected: usize,
}

/// Decode one 255-symbol codeword. `erasures` are symbol positions the sampler
/// flagged as unreliable; RS spends one parity symbol per erasure instead of two.
pub fn decode_codeword(cw: &[u8], ecc: Ecc, erasures: &[usize]) -> Result<BlockDecode, BlockError> {
    let mut buf = cw.to_vec();
    let nsym = ecc.nsym();
    let corrected = rs_decode(&mut buf, nsym, erasures).map_err(|e| match e {
        RsError::TooManyErrors => BlockError::Rs,
    })?;

    let k = ecc.k();
    let index = u32::from_le_bytes([buf[0], buf[1], buf[2], 0]);
    let flags = buf[3];
    let want = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let payload = buf[BLOCK_HEADER..k].to_vec();
    let mut crc_input = Vec::with_capacity(4 + payload.len());
    crc_input.extend_from_slice(&buf[0..4]);
    crc_input.extend_from_slice(&payload);
    if crc32c(&crc_input) != want {
        return Err(BlockError::Crc);
    }
    // Erasures cost one parity symbol each, errors two.
    let spent = (corrected * 2).max(erasures.len()) as f64;
    Ok(BlockDecode {
        block: Block {
            index,
            flags,
            payload,
        },
        margin: (spent / nsym as f64).min(1.0),
        corrected,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_and_recovers_a_block() {
        let ecc = Ecc::Q;
        let b = Block {
            index: 0x00AB_CDEF & 0x00FF_FFFF,
            flags: FLAG_PARITY,
            payload: (0..ecc.payload()).map(|i| (i * 31 % 251) as u8).collect(),
        };
        let mut cw = b.to_codeword(ecc);
        for i in 0..ecc.nsym() / 2 {
            cw[i * 3] ^= 0x5a;
        }
        let d = decode_codeword(&cw, ecc, &[]).unwrap();
        assert_eq!(d.block, b);
        assert!(d.margin > 0.9);
    }

    #[test]
    fn detects_corruption_beyond_capacity() {
        let ecc = Ecc::M;
        let b = Block {
            index: 42,
            flags: 0,
            payload: vec![0xAA; ecc.payload()],
        };
        let mut cw = b.to_codeword(ecc);
        for i in 0..ecc.nsym() {
            cw[i * 2] ^= 0xff;
        }
        assert!(decode_codeword(&cw, ecc, &[]).is_err());
    }
}
