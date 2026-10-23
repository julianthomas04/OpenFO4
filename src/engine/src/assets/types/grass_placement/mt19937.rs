//! MT19937, the 32-bit Mersenne Twister generator.

const STATE_LEN: usize = 624;
const SHIFT_LEN: usize = 397;
const MATRIX_A: u32 = 0x9908_b0df;
const UPPER_MASK: u32 = 0x8000_0000;
const LOWER_MASK: u32 = 0x7fff_ffff;

pub struct Mt19937 {
    state: [u32; STATE_LEN],
    next: usize,
}

impl Mt19937 {
    pub fn new(seed: u32) -> Self {
        let mut state = [0u32; STATE_LEN];
        state[0] = seed;
        for i in 1..STATE_LEN {
            let previous = state[i - 1];
            state[i] = 1_812_433_253u32
                .wrapping_mul(previous ^ (previous >> 30))
                .wrapping_add(i as u32);
        }
        Self {
            state,
            next: STATE_LEN,
        }
    }

    pub fn next_u32(&mut self) -> u32 {
        if self.next >= STATE_LEN {
            self.regenerate();
        }
        let mut y = self.state[self.next];
        self.next += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^ (y >> 18)
    }

    fn regenerate(&mut self) {
        for i in 0..STATE_LEN {
            let y = (self.state[i] & UPPER_MASK) | (self.state[(i + 1) % STATE_LEN] & LOWER_MASK);
            let mut next = self.state[(i + SHIFT_LEN) % STATE_LEN] ^ (y >> 1);
            if y & 1 != 0 {
                next ^= MATRIX_A;
            }
            self.state[i] = next;
        }
        self.next = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_seed_5489_gives_the_reference_sequence() {
        let mut generator = Mt19937::new(5489);
        assert_eq!(generator.next_u32(), 3_499_211_612);
        assert_eq!(generator.next_u32(), 581_869_302);
        assert_eq!(generator.next_u32(), 3_890_346_734);
    }

    #[test]
    fn seed_1_gives_the_reference_sequence() {
        let mut generator = Mt19937::new(1);
        assert_eq!(generator.next_u32(), 1_791_095_845);
        assert_eq!(generator.next_u32(), 4_282_876_139);
    }

    #[test]
    fn the_10000th_output_of_the_default_seed_matches_the_standard() {
        // The C++ standard requires the 10000th consecutive invocation of a
        // default-constructed mt19937 to produce 4123659995; this crosses
        // many state regenerations.
        let mut generator = Mt19937::new(5489);
        let tenthousandth = (0..10_000).map(|_| generator.next_u32()).last();
        assert_eq!(tenthousandth, Some(4_123_659_995));
    }
}
