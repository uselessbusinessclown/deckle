//! Deckle: back up data to paper.
//!
//! The layout engine, encoder, decoder and software degradation loop. No
//! platform-specific code and no unsafe: the same crate builds and behaves
//! identically on macOS and Linux, and the software loop runs in CI on both.
//!
//! See `docs/PLAN.md` for the design this implements and `docs/PROTOTYPE.md`
//! for what is and is not built yet.

// In the coding and linear-algebra code the index *is* the mathematical object -
// a codeword symbol position, a matrix row, a histogram bucket - so range loops
// state the intent more clearly than iterator adapters over zipped slices.
#![allow(clippy::needless_range_loop)]

pub mod bitmap;
pub mod block;
pub mod crc;
pub mod degrade;
pub mod descriptor;
pub mod doc;
pub mod fec;
pub mod geom;
pub mod gf256;
pub mod layout;
pub mod pdf;
pub mod raster;
pub mod rng;
pub mod sha256;
