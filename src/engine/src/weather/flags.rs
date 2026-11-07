/// One flag of a weather's flag byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeatherFlag {
    Pleasant,
    Cloudy,
    Rainy,
    Snow,
    SkyStaticsAlwaysVisible,
    SkyStaticsFollowsSun,
    RainOcclusion,
    HudRainEffects,
}

impl WeatherFlag {
    pub const ALL: &'static [Self] = &[
        Self::Pleasant,
        Self::Cloudy,
        Self::Rainy,
        Self::Snow,
        Self::SkyStaticsAlwaysVisible,
        Self::SkyStaticsFollowsSun,
        Self::RainOcclusion,
        Self::HudRainEffects,
    ];

    pub const fn bit(self) -> u8 {
        match self {
            Self::Pleasant => 1,
            Self::Cloudy => 2,
            Self::Rainy => 4,
            Self::Snow => 8,
            Self::SkyStaticsAlwaysVisible => 16,
            Self::SkyStaticsFollowsSun => 32,
            Self::RainOcclusion => 64,
            Self::HudRainEffects => 128,
        }
    }
}

/// The weather's flag byte. The flags name every bit of it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WeatherFlags(u8);

impl WeatherFlags {
    /// `None` when `bits` does not fit the byte.
    pub fn from_bits(bits: u32) -> Option<Self> {
        u8::try_from(bits).ok().map(Self)
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, flag: WeatherFlag) -> bool {
        self.0 & flag.bit() != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_flag_has_its_own_bit_and_the_set_round_trips() {
        let bits: Vec<u8> = WeatherFlag::ALL.iter().map(|flag| flag.bit()).collect();
        assert_eq!(bits, [1, 2, 4, 8, 16, 32, 64, 128]);
        assert_eq!(WeatherFlags::from_bits(255).unwrap().bits(), 255);
    }

    #[test]
    fn a_set_contains_exactly_the_flags_of_its_bits() {
        let flags = WeatherFlags::from_bits(0b0000_0110).unwrap();
        let contained: Vec<WeatherFlag> = WeatherFlag::ALL
            .iter()
            .copied()
            .filter(|flag| flags.contains(*flag))
            .collect();
        assert_eq!(contained, [WeatherFlag::Cloudy, WeatherFlag::Rainy]);
    }

    #[test]
    fn bits_beyond_the_byte_are_rejected() {
        assert_eq!(WeatherFlags::from_bits(256), None);
        assert_eq!(WeatherFlags::from_bits(u32::MAX), None);
    }
}
