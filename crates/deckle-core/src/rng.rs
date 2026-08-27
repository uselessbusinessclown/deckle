//! Deterministic PRNG. Two uses, both requiring reproducibility rather than
//! cryptographic quality: the cell whitener (so an all-zero payload still
//! produces a balanced, Sauvola-friendly page) and the degradation harness.

#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }
    /// Uniform in [0, 1).
    #[inline]
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Approximately standard normal (Irwin-Hall, twelve samples).
    pub fn next_normal(&mut self) -> f64 {
        let mut s = 0.0;
        for _ in 0..12 {
            s += self.next_f64();
        }
        s - 6.0
    }
    pub fn next_bool(&mut self) -> bool {
        self.next_u64() & 1 != 0
    }
}

/// Keystream for the cell whitener: one bit per cell, indexed by raster position.
pub struct Whitener {
    rng: Rng,
    buf: u64,
    left: u32,
}

impl Whitener {
    pub fn new(seed: u64) -> Self {
        Whitener {
            rng: Rng::new(seed),
            buf: 0,
            left: 0,
        }
    }
    #[inline]
    pub fn next_bit(&mut self) -> bool {
        if self.left == 0 {
            self.buf = self.rng.next_u64();
            self.left = 64;
        }
        let b = self.buf & 1 != 0;
        self.buf >>= 1;
        self.left -= 1;
        b
    }
}
