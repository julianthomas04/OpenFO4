use super::flag_set::flag_set;

flag_set! {
    /// The weather's flag byte.
    WeatherFlags of WeatherFlag {
        Pleasant = 1,
        Cloudy = 2,
        Rainy = 4,
        Snow = 8,
        SkyStaticsAlwaysVisible = 16,
        SkyStaticsFollowsSun = 32,
        RainOcclusion = 64,
        HudRainEffects = 128,
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
    fn bits_no_flag_names_are_rejected() {
        assert_eq!(WeatherFlags::from_bits(256), None);
        assert_eq!(WeatherFlags::from_bits(u32::MAX), None);
    }
}
