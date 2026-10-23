//! The random source of the audio decisions: a trait the tests can script and a seedable generator.

/// Steps of `AudioRng::unit`; a power of two no larger than the `f32` mantissa, so every step is
/// exactly representable and 1.0 is never reached.
const UNIT_STEPS: usize = 1 << 24;

pub trait AudioRng {
    /// A uniform value in `0..n`; `n` is never zero.
    fn below(&mut self, n: usize) -> usize;

    /// A uniform value in `[0, 1)`.
    fn unit(&mut self) -> f32 {
        self.below(UNIT_STEPS) as f32 / UNIT_STEPS as f32
    }

    /// A uniform value in `[-bound, bound)`; exactly 0 for a bound of 0.
    fn symmetric(&mut self, bound: f32) -> f32 {
        (2.0 * self.unit() - 1.0) * bound
    }
}

/// Small seedable generator; the same seed always yields the same sequence.
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
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
}

impl AudioRng for SplitMix64 {
    fn below(&mut self, n: usize) -> usize {
        ((u128::from(self.next_u64()) * n as u128) >> 64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_yields_the_same_sequence() {
        let mut a = SplitMix64::new(5);
        let mut b = SplitMix64::new(5);
        assert!((0..20).all(|_| a.below(1000) == b.below(1000)));
    }

    #[test]
    fn unit_stays_in_the_half_open_interval() {
        let mut rng = SplitMix64::new(1);
        assert!((0..10_000).all(|_| (0.0..1.0).contains(&rng.unit())));
    }

    #[test]
    fn symmetric_stays_within_the_bound_and_reaches_both_signs() {
        let mut rng = SplitMix64::new(2);
        let draws: Vec<f32> = (0..10_000).map(|_| rng.symmetric(3.0)).collect();
        assert!(draws.iter().all(|value| (-3.0..3.0).contains(value)));
        assert!(draws.iter().any(|&value| value < -2.5));
        assert!(draws.iter().any(|&value| value > 2.5));
    }

    #[test]
    fn symmetric_with_a_zero_bound_is_zero() {
        let mut rng = SplitMix64::new(3);
        assert!((0..100).all(|_| rng.symmetric(0.0).abs() == 0.0));
    }
}
