/// One of the eight times of day a weather carries values for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeSlot {
    Sunrise,
    Day,
    Sunset,
    Night,
    EarlySunrise,
    LateSunrise,
    EarlySunset,
    LateSunset,
}

impl TimeSlot {
    /// The game's record order.
    pub const ALL: [Self; 8] = [
        Self::Sunrise,
        Self::Day,
        Self::Sunset,
        Self::Night,
        Self::EarlySunrise,
        Self::LateSunrise,
        Self::EarlySunset,
        Self::LateSunset,
    ];

    /// The key of this slot in the database's per-slot JSON objects.
    pub const fn json_key(self) -> &'static str {
        match self {
            Self::Sunrise => "Sunrise",
            Self::Day => "Day",
            Self::Sunset => "Sunset",
            Self::Night => "Night",
            Self::EarlySunrise => "EarlySunrise",
            Self::LateSunrise => "LateSunrise",
            Self::EarlySunset => "EarlySunset",
            Self::LateSunset => "LateSunset",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn json_keys_match_the_database_spelling_in_record_order() {
        let keys: Vec<&str> = TimeSlot::ALL.iter().map(|slot| slot.json_key()).collect();
        assert_eq!(
            keys,
            [
                "Sunrise",
                "Day",
                "Sunset",
                "Night",
                "EarlySunrise",
                "LateSunrise",
                "EarlySunset",
                "LateSunset"
            ]
        );
    }

    #[test]
    fn every_slot_is_listed_once() {
        let distinct: HashSet<TimeSlot> = TimeSlot::ALL.into_iter().collect();
        assert_eq!(distinct.len(), 8);
    }
}
