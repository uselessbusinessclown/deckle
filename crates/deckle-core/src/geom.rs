//! Planar homography fitting and application.
//!
//! The decoder maps scan pixels into a normalised "fiducial frame" whose corners
//! are the four corner-marker centres (PLAN.md section 5.8, step 3). Everything
//! downstream is expressed in that frame, which is why the decoder never needs
//! to know the paper size, the scan resolution, or the page orientation.

#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }
    pub fn dist(self, o: Point) -> f64 {
        ((self.x - o.x).powi(2) + (self.y - o.y).powi(2)).sqrt()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Homography(pub [f64; 9]);

impl Homography {
    pub fn apply(&self, p: Point) -> Point {
        let m = &self.0;
        let d = m[6] * p.x + m[7] * p.y + m[8];
        Point {
            x: (m[0] * p.x + m[1] * p.y + m[2]) / d,
            y: (m[3] * p.x + m[4] * p.y + m[5]) / d,
        }
    }

    /// Homography taking `src[i]` to `dst[i]` for four correspondences.
    pub fn from_four(src: &[Point; 4], dst: &[Point; 4]) -> Option<Homography> {
        // Direct linear transform with h22 fixed to 1: eight unknowns, eight rows.
        let mut a = [[0.0f64; 9]; 8];
        for i in 0..4 {
            let (x, y) = (src[i].x, src[i].y);
            let (u, v) = (dst[i].x, dst[i].y);
            a[i * 2] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
            a[i * 2 + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
        }
        let sol = solve8(&mut a)?;
        Some(Homography([
            sol[0], sol[1], sol[2], sol[3], sol[4], sol[5], sol[6], sol[7], 1.0,
        ]))
    }

    pub fn invert(&self) -> Option<Homography> {
        let m = &self.0;
        let c = |a: usize, b: usize, c2: usize, d: usize| m[a] * m[b] - m[c2] * m[d];
        let inv = [
            c(4, 8, 5, 7),
            c(2, 7, 1, 8),
            c(1, 5, 2, 4),
            c(5, 6, 3, 8),
            c(0, 8, 2, 6),
            c(2, 3, 0, 5),
            c(3, 7, 4, 6),
            c(1, 6, 0, 7),
            c(0, 4, 1, 3),
        ];
        let det = m[0] * inv[0] + m[1] * inv[3] + m[2] * inv[6];
        if det.abs() < 1e-12 {
            return None;
        }
        let mut out = [0.0; 9];
        for i in 0..9 {
            out[i] = inv[i] / det;
        }
        Some(Homography(out))
    }
}

/// Gaussian elimination with partial pivoting on an 8x9 augmented system.
fn solve8(a: &mut [[f64; 9]; 8]) -> Option<[f64; 8]> {
    for col in 0..8 {
        let mut piv = col;
        for r in col + 1..8 {
            if a[r][col].abs() > a[piv][col].abs() {
                piv = r;
            }
        }
        if a[piv][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, piv);
        let d = a[col][col];
        for c in col..9 {
            a[col][c] /= d;
        }
        for r in 0..8 {
            if r != col && a[r][col] != 0.0 {
                let f = a[r][col];
                for c in col..9 {
                    a[r][c] -= f * a[col][c];
                }
            }
        }
    }
    let mut out = [0.0; 8];
    for i in 0..8 {
        out[i] = a[i][8];
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_known_transform() {
        let src = [
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(0.0, 1.0),
            Point::new(1.0, 1.0),
        ];
        let dst = [
            Point::new(100.0, 210.0),
            Point::new(900.0, 190.0),
            Point::new(120.0, 1180.0),
            Point::new(940.0, 1210.0),
        ];
        let h = Homography::from_four(&src, &dst).unwrap();
        for i in 0..4 {
            let p = h.apply(src[i]);
            assert!((p.x - dst[i].x).abs() < 1e-6 && (p.y - dst[i].y).abs() < 1e-6);
        }
        let hi = h.invert().unwrap();
        let mid = h.apply(Point::new(0.37, 0.62));
        let back = hi.apply(mid);
        assert!((back.x - 0.37).abs() < 1e-9 && (back.y - 0.62).abs() < 1e-9);
    }
}
