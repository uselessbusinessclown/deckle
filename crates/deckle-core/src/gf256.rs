//! GF(2^8) arithmetic and Reed-Solomon coding over the primitive polynomial
//! x^8 + x^4 + x^3 + x^2 + 1 (0x11d), generator alpha = 2, first consecutive
//! root alpha^0.
//!
//! Systematic RS(n, k) with n <= 255. Decoding handles errors, erasures, and
//! any mixture satisfying `2*errors + erasures <= n - k` (PLAN.md section 5.5).
//! Codewords are stored most-significant-symbol first: `cw[0]` is the
//! coefficient of x^(n-1).

pub const PRIM: u16 = 0x11d;

pub struct Tables {
    pub exp: [u8; 512],
    pub log: [u8; 256],
}

impl Tables {
    pub const fn new() -> Self {
        let mut exp = [0u8; 512];
        let mut log = [0u8; 256];
        let mut x: u16 = 1;
        let mut i = 0;
        while i < 255 {
            exp[i] = x as u8;
            log[x as usize] = i as u8;
            x <<= 1;
            if x & 0x100 != 0 {
                x ^= PRIM;
            }
            i += 1;
        }
        let mut j = 255;
        while j < 512 {
            exp[j] = exp[j - 255];
            j += 1;
        }
        Tables { exp, log }
    }
}

impl Default for Tables {
    fn default() -> Self {
        Tables::new()
    }
}

pub static T: Tables = Tables::new();

#[inline]
pub fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        0
    } else {
        T.exp[T.log[a as usize] as usize + T.log[b as usize] as usize]
    }
}

#[inline]
pub fn div(a: u8, b: u8) -> u8 {
    assert!(b != 0, "GF(256) division by zero");
    if a == 0 {
        0
    } else {
        T.exp[(T.log[a as usize] as usize + 255 - T.log[b as usize] as usize) % 255]
    }
}

#[inline]
pub fn inv(a: u8) -> u8 {
    assert!(a != 0, "GF(256) inverse of zero");
    T.exp[255 - T.log[a as usize] as usize]
}

#[inline]
pub fn pow(a: u8, e: i32) -> u8 {
    if a == 0 {
        return 0;
    }
    let l = T.log[a as usize] as i32;
    let idx = ((l * e) % 255 + 255) % 255;
    T.exp[idx as usize]
}

/// Polynomials here are little-endian in degree: `p[0]` is the constant term.
fn poly_mul(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; a.len() + b.len() - 1];
    for (i, &x) in a.iter().enumerate() {
        if x == 0 {
            continue;
        }
        for (j, &y) in b.iter().enumerate() {
            out[i + j] ^= mul(x, y);
        }
    }
    out
}

fn poly_eval(p: &[u8], x: u8) -> u8 {
    let mut acc = 0u8;
    for &c in p.iter().rev() {
        acc = mul(acc, x) ^ c;
    }
    acc
}

fn poly_scale(p: &[u8], s: u8) -> Vec<u8> {
    p.iter().map(|&c| mul(c, s)).collect()
}

fn poly_add_into(dst: &mut Vec<u8>, src: &[u8]) {
    if src.len() > dst.len() {
        dst.resize(src.len(), 0);
    }
    for (d, &s) in dst.iter_mut().zip(src.iter()) {
        *d ^= s;
    }
}

/// Generator polynomial for `nsym` parity symbols: prod (x + alpha^i), i in 0..nsym.
/// Returned little-endian; the leading coefficient `g[nsym]` is 1.
pub fn generator(nsym: usize) -> Vec<u8> {
    let mut g = vec![1u8];
    for i in 0..nsym {
        g = poly_mul(&g, &[pow(2, i as i32), 1]);
    }
    g
}

/// Systematic encode: returns the `nsym` parity symbols for `data`,
/// most-significant first, to be appended to `data`.
pub fn rs_encode_parity(data: &[u8], nsym: usize) -> Vec<u8> {
    let mut gen = generator(nsym);
    gen.reverse(); // big-endian: gen[0] == 1
    let mut out = vec![0u8; data.len() + nsym];
    out[..data.len()].copy_from_slice(data);
    for i in 0..data.len() {
        let coef = out[i];
        if coef != 0 {
            for j in 1..gen.len() {
                out[i + j] ^= mul(gen[j], coef);
            }
        }
    }
    out.split_off(data.len())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RsError {
    /// More corruption than the code can correct, or a decode that failed to verify.
    TooManyErrors,
}

fn syndromes(cw: &[u8], nsym: usize) -> Vec<u8> {
    (0..nsym)
        .map(|i| {
            let x = pow(2, i as i32);
            let mut acc = 0u8;
            for &c in cw.iter() {
                acc = mul(acc, x) ^ c;
            }
            acc
        })
        .collect()
}

/// Decode a full codeword in place.
///
/// `erasures` lists positions (0 = first symbol of the codeword) known to be
/// unreliable. Returns the number of symbols actually altered.
pub fn rs_decode(cw: &mut [u8], nsym: usize, erasures: &[usize]) -> Result<usize, RsError> {
    let n = cw.len();
    if erasures.len() > nsym || nsym == 0 || n > 255 {
        return Err(RsError::TooManyErrors);
    }
    let mut erasures: Vec<usize> = erasures.to_vec();
    erasures.sort_unstable();
    erasures.dedup();
    if erasures.iter().any(|&p| p >= n) {
        return Err(RsError::TooManyErrors);
    }

    let synd = syndromes(cw, nsym);
    if synd.iter().all(|&s| s == 0) {
        return Ok(0);
    }

    // Forney syndromes: fold the known erasure locations out of the syndrome
    // sequence so Berlekamp-Massey only has to find the unknown errors.
    let mut fsynd = synd.clone();
    for &pos in &erasures {
        let x = pow(2, (n - 1 - pos) as i32);
        for i in 0..fsynd.len() - 1 {
            fsynd[i] = mul(fsynd[i], x) ^ fsynd[i + 1];
        }
        fsynd.pop();
    }

    // Berlekamp-Massey.
    let mut err_loc = vec![1u8];
    let mut old_loc = vec![1u8];
    for i in 0..fsynd.len() {
        let mut delta = fsynd[i];
        for j in 1..err_loc.len() {
            if j <= i {
                delta ^= mul(err_loc[j], fsynd[i - j]);
            }
        }
        old_loc.insert(0, 0); // multiply by x
        if delta != 0 {
            if old_loc.len() > err_loc.len() {
                let new_loc = poly_scale(&old_loc, delta);
                old_loc = poly_scale(&err_loc, inv(delta));
                err_loc = new_loc;
            }
            let scaled = poly_scale(&old_loc, delta);
            poly_add_into(&mut err_loc, &scaled);
        }
    }
    while err_loc.len() > 1 && *err_loc.last().unwrap() == 0 {
        err_loc.pop();
    }
    let nerr = err_loc.len() - 1;
    if 2 * nerr + erasures.len() > nsym {
        return Err(RsError::TooManyErrors);
    }

    // Chien search. An error at array index `pos` has locator X = alpha^(n-1-pos),
    // and Lambda(x) = prod (1 - X_i x) vanishes at x = X_i^-1. Test that directly
    // rather than relying on an index identity.
    let mut err_pos: Vec<usize> = Vec::new();
    for pos in 0..n {
        let x_inv = inv(pow(2, (n - 1 - pos) as i32));
        if poly_eval(&err_loc, x_inv) == 0 {
            err_pos.push(pos);
        }
    }
    if err_pos.len() != nerr {
        return Err(RsError::TooManyErrors);
    }

    let mut all_pos = erasures;
    all_pos.extend_from_slice(&err_pos);
    all_pos.sort_unstable();
    all_pos.dedup();
    if all_pos.len() > nsym {
        return Err(RsError::TooManyErrors);
    }

    // Combined locator over erasures and errors.
    let mut lambda = vec![1u8];
    let xs: Vec<u8> = all_pos
        .iter()
        .map(|&p| pow(2, (n - 1 - p) as i32))
        .collect();
    for &x in &xs {
        lambda = poly_mul(&lambda, &[1, x]);
    }

    // Error evaluator Omega(x) = S(x) * Lambda(x) mod x^nsym.
    let mut omega = poly_mul(&synd, &lambda);
    omega.truncate(nsym);

    // Forney, with the locator derivative in product form (unambiguous in GF(2^m)).
    let mut altered = 0usize;
    for (i, &pos) in all_pos.iter().enumerate() {
        let xi_inv = inv(xs[i]);
        let mut prime = 1u8;
        for (j, &xj) in xs.iter().enumerate() {
            if j != i {
                prime = mul(prime, 1 ^ mul(xi_inv, xj));
            }
        }
        if prime == 0 {
            return Err(RsError::TooManyErrors);
        }
        // Omega(Xi^-1) = Y_i * prod_{j!=i}(1 - Xj*Xi^-1), so the magnitude is
        // the evaluator over that product directly; with first root alpha^0
        // there is no additional X_i factor.
        let mag = div(poly_eval(&omega, xi_inv), prime);
        if mag != 0 {
            cw[pos] ^= mag;
            altered += 1;
        }
    }

    // A correct decode leaves every syndrome zero. This check is what stops a
    // silent mis-correction from reaching the caller (PLAN.md R6).
    if syndromes(cw, nsym).iter().any(|&s| s != 0) {
        return Err(RsError::TooManyErrors);
    }
    Ok(altered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codeword(k: usize, nsym: usize, seed: u64) -> Vec<u8> {
        let mut s = seed;
        let data: Vec<u8> = (0..k)
            .map(|_| {
                s = s
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (s >> 33) as u8
            })
            .collect();
        let mut cw = data.clone();
        cw.extend(rs_encode_parity(&data, nsym));
        cw
    }

    #[test]
    fn clean_codeword_decodes_untouched() {
        let cw = codeword(191, 64, 1);
        let mut c = cw.clone();
        assert_eq!(rs_decode(&mut c, 64, &[]).unwrap(), 0);
        assert_eq!(c, cw);
    }

    #[test]
    fn corrects_up_to_t_errors() {
        for nsym in [16usize, 32, 64, 96] {
            let k = 255 - nsym;
            let t = nsym / 2;
            let cw = codeword(k, nsym, nsym as u64);
            let mut c = cw.clone();
            for i in 0..t {
                c[(i * 7 + 3) % 255] ^= 0xa5;
            }
            let fixed = rs_decode(&mut c, nsym, &[]).expect("t errors must correct");
            assert!(fixed <= t);
            assert_eq!(c, cw, "nsym={nsym}");
        }
    }

    #[test]
    fn corrects_up_to_nsym_erasures() {
        let nsym = 64;
        let cw = codeword(191, nsym, 7);
        let mut c = cw.clone();
        let pos: Vec<usize> = (0..nsym).map(|i| i * 3 + 1).collect();
        for &p in &pos {
            c[p] = 0x00;
        }
        rs_decode(&mut c, nsym, &pos).expect("nsym erasures must correct");
        assert_eq!(c, cw);
    }

    #[test]
    fn mixed_errors_and_erasures() {
        let nsym = 64;
        let cw = codeword(191, nsym, 11);
        let mut c = cw.clone();
        let er: Vec<usize> = (0..30).map(|i| i * 5).collect();
        for &p in &er {
            c[p] ^= 0x3c;
        }
        for i in 0..17 {
            c[200 + i] ^= 0x9b; // 2*17 + 30 = 64 = nsym
        }
        rs_decode(&mut c, nsym, &er).expect("2e+f == nsym must correct");
        assert_eq!(c, cw);
    }

    #[test]
    fn reports_failure_beyond_capacity() {
        let nsym = 32;
        let cw = codeword(223, nsym, 13);
        let mut c = cw.clone();
        for i in 0..40 {
            c[i * 5] ^= 0x77;
        }
        assert_eq!(rs_decode(&mut c, nsym, &[]), Err(RsError::TooManyErrors));
    }
}
