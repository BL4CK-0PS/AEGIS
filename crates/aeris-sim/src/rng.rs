//! Deterministic random streams.
//!
//! Every stream is a ChaCha8 generator whose 256-bit seed is derived with
//! BLAKE3 in key-derivation mode from `(seed, label, index)`. Streams with
//! different labels or indices are independent; the same inputs always give
//! the same stream on every platform.

use rand::distributions::Standard;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

const CONTEXT: &str = "AERIS aeris-sim v1 random stream";

/// 256-bit seed for stream `(seed, label, index)`.
pub fn derive_seed(seed: u64, label: &str, index: u64) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key(CONTEXT);
    h.update(&seed.to_le_bytes());
    h.update(&(label.len() as u64).to_le_bytes());
    h.update(label.as_bytes());
    h.update(&index.to_le_bytes());
    *h.finalize().as_bytes()
}

/// ChaCha8 stream `(seed, label, index)`.
pub fn stream(seed: u64, label: &str, index: u64) -> ChaCha8Rng {
    ChaCha8Rng::from_seed(derive_seed(seed, label, index))
}

/// Uniform in `[0, 1)`.
pub fn unit(rng: &mut ChaCha8Rng) -> f64 {
    rng.sample(Standard)
}

/// Two independent standard normals (Box–Muller). Always draws exactly two
/// uniforms, which keeps common-random-number streams aligned.
pub fn normal_pair(rng: &mut ChaCha8Rng) -> (f64, f64) {
    let u1 = 1.0 - unit(rng); // (0, 1]
    let u2 = unit(rng);
    let r = (-2.0 * u1.ln()).sqrt();
    let th = std::f64::consts::TAU * u2;
    (r * th.cos(), r * th.sin())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_are_reproducible_and_separated() {
        let a: Vec<u64> = (0..4).map(|_| stream(7, "env", 0).r#gen()).collect();
        assert!(a.windows(2).all(|w| w[0] == w[1]));
        assert_ne!(derive_seed(7, "env", 0), derive_seed(7, "obs", 0));
        assert_ne!(derive_seed(7, "env", 0), derive_seed(8, "env", 0));
        assert_ne!(derive_seed(7, "env", 0), derive_seed(7, "env", 1));
        // Label length is part of the input: no ("ab", ..) vs ("a", "b..") collisions.
        assert_ne!(derive_seed(1, "ab", 0), derive_seed(1, "a", 0));
    }

    #[test]
    fn normal_pair_has_unit_moments() {
        let mut r = stream(1, "test", 0);
        let n = 100_000;
        let (mut s, mut s2) = (0.0, 0.0);
        for _ in 0..n / 2 {
            let (x, y) = normal_pair(&mut r);
            s += x + y;
            s2 += x * x + y * y;
        }
        let mean = s / n as f64;
        let var = s2 / n as f64 - mean * mean;
        assert!(mean.abs() < 0.02, "mean {mean}");
        assert!((var - 1.0).abs() < 0.02, "var {var}");
    }
}
