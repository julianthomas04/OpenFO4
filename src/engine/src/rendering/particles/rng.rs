//! Small deterministic random generator (`SplitMix64`); the same seed always
//! yields the same particle stream.

#[derive(Debug, Clone)]
pub struct ParticleRng {
    state: u64,
}

impl ParticleRng {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in `[-1, 1)`.
    pub fn signed(&mut self) -> f32 {
        self.unit().mul_add(2.0, -1.0)
    }

    /// `centre` plus a symmetric deviation of up to `variation`.
    pub fn spread(&mut self, centre: f32, variation: f32) -> f32 {
        self.signed().mul_add(variation, centre)
    }

    /// `true` with the given probability (values outside `0..=1` saturate).
    pub fn chance(&mut self, probability: f32) -> bool {
        self.unit() < probability
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_gives_same_sequence() {
        let mut a = ParticleRng::new(7);
        let mut b = ParticleRng::new(7);
        for _ in 0..16 {
            assert_eq!(a.unit().to_bits(), b.unit().to_bits());
        }
    }

    #[test]
    fn unit_and_signed_stay_in_range() {
        let mut rng = ParticleRng::new(1);
        for _ in 0..10_000 {
            assert!((0.0..1.0).contains(&rng.unit()));
            assert!((-1.0..1.0).contains(&rng.signed()));
        }
    }

    #[test]
    fn spread_is_bounded_by_variation() {
        let mut rng = ParticleRng::new(3);
        for _ in 0..1000 {
            assert!((rng.spread(5.0, 2.0) - 5.0).abs() <= 2.0);
        }
    }
}
