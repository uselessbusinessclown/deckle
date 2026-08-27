//! Cross-block erasure coding (PLAN.md section 5.7).
//!
//! Systematic Reed-Solomon over GF(2^8) using a Cauchy generator matrix: every
//! square submatrix of a Cauchy matrix is invertible, so *any* D surviving
//! blocks of a group reconstruct the group. Data blocks are printed verbatim, so
//! an undamaged archive never invokes this decoder at all.
//!
//! PROTOTYPE DEVIATION: the specification calls for GF(2^16) with groups up to
//! 32,768 blocks. GF(2^8) caps a group at 255 blocks, so this prototype splits
//! large documents into several groups and stripes pages across them, which
//! spreads a lost sheet evenly. The interface is unchanged by the upgrade.

use crate::gf256::{inv, mul};

pub const MAX_GROUP: usize = 255;

/// Cauchy matrix row `i`, column `j`: 1 / (x_i + y_j) with disjoint x and y sets.
fn cauchy(i: usize, j: usize, p: usize) -> u8 {
    let x = i as u8;
    let y = (p + j) as u8;
    inv(x ^ y)
}

/// Encode `p` parity blocks from `d` data blocks, all of equal length.
pub fn encode_group(data: &[Vec<u8>], p: usize) -> Vec<Vec<u8>> {
    let d = data.len();
    assert!(
        d + p <= MAX_GROUP,
        "group of {d}+{p} exceeds GF(256) capacity"
    );
    if p == 0 || d == 0 {
        return Vec::new();
    }
    let len = data[0].len();
    let mut out = vec![vec![0u8; len]; p];
    for i in 0..p {
        for (j, blk) in data.iter().enumerate() {
            debug_assert_eq!(blk.len(), len);
            let c = cauchy(i, j, p);
            if c == 0 {
                continue;
            }
            let row = &mut out[i];
            for (o, &b) in row.iter_mut().zip(blk.iter()) {
                *o ^= mul(c, b);
            }
        }
    }
    out
}

#[derive(Debug, PartialEq, Eq)]
pub enum FecError {
    /// Fewer than `d` blocks of the group survived.
    InsufficientBlocks {
        have: usize,
        need: usize,
    },
    Singular,
}

/// Reconstruct the `d` data blocks of a group.
///
/// `data` and `parity` hold `Some(block)` for survivors and `None` for losses.
/// On success every `data` slot is filled.
pub fn decode_group(
    data: &mut [Option<Vec<u8>>],
    parity: &[Option<Vec<u8>>],
    len: usize,
) -> Result<usize, FecError> {
    let d = data.len();
    let p = parity.len();
    let missing: Vec<usize> = (0..d).filter(|&i| data[i].is_none()).collect();
    if missing.is_empty() {
        return Ok(0);
    }
    let have_parity: Vec<usize> = (0..p).filter(|&i| parity[i].is_some()).collect();
    let have_data = d - missing.len();
    if have_data + have_parity.len() < d {
        return Err(FecError::InsufficientBlocks {
            have: have_data + have_parity.len(),
            need: d,
        });
    }

    // Build a d x d system from the surviving rows of E = [I; A]: the surviving
    // data rows are unit vectors, so only the missing columns are unknown.
    let m = missing.len();
    let rows: Vec<usize> = have_parity.into_iter().take(m).collect();
    if rows.len() < m {
        return Err(FecError::InsufficientBlocks {
            have: have_data + rows.len(),
            need: d,
        });
    }

    // Reduce: parity_i = sum_j A[i][j] * data_j. Move known data terms to the
    // right-hand side, leaving an m x m system in the missing blocks.
    let mut mat = vec![vec![0u8; m]; m];
    let mut rhs = vec![vec![0u8; len]; m];
    for (r, &pi) in rows.iter().enumerate() {
        let mut acc = parity[pi].clone().unwrap();
        for j in 0..d {
            let c = cauchy(pi, j, p);
            if let Some(known) = &data[j] {
                if c != 0 {
                    for (a, &b) in acc.iter_mut().zip(known.iter()) {
                        *a ^= mul(c, b);
                    }
                }
            }
        }
        rhs[r] = acc;
        for (cidx, &j) in missing.iter().enumerate() {
            mat[r][cidx] = cauchy(pi, j, p);
        }
    }

    // Gauss-Jordan over GF(256), applying the same operations to the RHS blocks.
    for col in 0..m {
        let piv = (col..m)
            .find(|&r| mat[r][col] != 0)
            .ok_or(FecError::Singular)?;
        mat.swap(col, piv);
        rhs.swap(col, piv);
        let d0 = inv(mat[col][col]);
        for c in col..m {
            mat[col][c] = mul(mat[col][c], d0);
        }
        for b in rhs[col].iter_mut() {
            *b = mul(*b, d0);
        }
        for r in 0..m {
            if r != col && mat[r][col] != 0 {
                let f = mat[r][col];
                for c in col..m {
                    mat[r][c] ^= mul(f, mat[col][c]);
                }
                let (src, dst) = if r < col {
                    let (a, b) = rhs.split_at_mut(col);
                    (&b[0], &mut a[r])
                } else {
                    let (a, b) = rhs.split_at_mut(r);
                    (&a[col], &mut b[0])
                };
                for (x, &y) in dst.iter_mut().zip(src.iter()) {
                    *x ^= mul(f, y);
                }
            }
        }
    }

    for (cidx, &j) in missing.iter().enumerate() {
        data[j] = Some(rhs[cidx].clone());
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    fn group(d: usize, len: usize, seed: u64) -> Vec<Vec<u8>> {
        let mut r = Rng::new(seed);
        (0..d)
            .map(|_| (0..len).map(|_| r.next_u32() as u8).collect())
            .collect()
    }

    #[test]
    fn recovers_exactly_p_losses() {
        let (d, p, len) = (60usize, 15usize, 183usize);
        let orig = group(d, len, 5);
        let par = encode_group(&orig, p);
        let mut data: Vec<Option<Vec<u8>>> = orig.iter().cloned().map(Some).collect();
        for i in 0..p {
            data[i * 4] = None; // lose exactly p data blocks
        }
        let parity: Vec<Option<Vec<u8>>> = par.into_iter().map(Some).collect();
        decode_group(&mut data, &parity, len).unwrap();
        for i in 0..d {
            assert_eq!(data[i].as_ref().unwrap(), &orig[i], "block {i}");
        }
    }

    #[test]
    fn recovers_when_parity_is_also_lost() {
        let (d, p, len) = (40usize, 20usize, 64usize);
        let orig = group(d, len, 9);
        let par = encode_group(&orig, p);
        let mut data: Vec<Option<Vec<u8>>> = orig.iter().cloned().map(Some).collect();
        let mut parity: Vec<Option<Vec<u8>>> = par.into_iter().map(Some).collect();
        for i in 0..12 {
            data[i * 3] = None;
        }
        for i in 0..8 {
            parity[i] = None; // 12 lost data, 12 surviving parity: exactly enough
        }
        decode_group(&mut data, &parity, len).unwrap();
        for i in 0..d {
            assert_eq!(data[i].as_ref().unwrap(), &orig[i]);
        }
    }

    #[test]
    fn reports_insufficient_parity() {
        let (d, p, len) = (30usize, 5usize, 32usize);
        let orig = group(d, len, 3);
        let par = encode_group(&orig, p);
        let mut data: Vec<Option<Vec<u8>>> = orig.iter().cloned().map(Some).collect();
        for i in 0..6 {
            data[i] = None;
        }
        let parity: Vec<Option<Vec<u8>>> = par.into_iter().map(Some).collect();
        assert!(matches!(
            decode_group(&mut data, &parity, len),
            Err(FecError::InsufficientBlocks { .. })
        ));
    }
}
