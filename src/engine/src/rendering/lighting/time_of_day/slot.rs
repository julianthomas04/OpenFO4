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

    pub const COUNT: usize = Self::ALL.len();

    /// The position in [`Self::ALL`].
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Runs `load` for every slot in the order of [`Self::index`], stopping at the first failure.
    pub fn try_map_all<T, E>(
        mut load: impl FnMut(Self) -> Result<T, E>,
    ) -> Result<[T; Self::COUNT], E> {
        Ok([
            load(Self::Sunrise)?,
            load(Self::Day)?,
            load(Self::Sunset)?,
            load(Self::Night)?,
            load(Self::EarlySunrise)?,
            load(Self::LateSunrise)?,
            load(Self::EarlySunset)?,
            load(Self::LateSunset)?,
        ])
    }

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
    fn the_index_is_the_position_in_record_order() {
        for (position, slot) in TimeSlot::ALL.into_iter().enumerate() {
            assert_eq!(slot.index(), position);
        }
    }

    #[test]
    fn every_slot_is_listed_once() {
        let distinct: HashSet<TimeSlot> = TimeSlot::ALL.into_iter().collect();
        assert_eq!(distinct.len(), 8);
    }

    #[test]
    fn every_slot_is_visited_in_index_order() {
        let visited = TimeSlot::try_map_all(Ok::<_, ()>).unwrap();
        assert_eq!(visited, TimeSlot::ALL);
        assert_eq!(visited.len(), TimeSlot::COUNT);
    }

    #[test]
    fn visiting_stops_at_the_first_failure() {
        let mut seen = Vec::new();
        let result = TimeSlot::try_map_all(|slot| {
            seen.push(slot);
            if slot == TimeSlot::Night {
                Err("night")
            } else {
                Ok(slot)
            }
        });
        assert_eq!(result, Err("night"));
        assert_eq!(
            seen,
            [
                TimeSlot::Sunrise,
                TimeSlot::Day,
                TimeSlot::Sunset,
                TimeSlot::Night
            ]
        );
    }
}
