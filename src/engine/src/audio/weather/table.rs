use std::collections::BTreeSet;

use super::error::WeatherAudioError;
use crate::weather::{SoundKind, WeatherId, WeatherSound};

/// The sounds of every weather of a climate, by weather.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WeatherSoundTable {
    weathers: Vec<Vec<WeatherSound>>,
}

impl WeatherSoundTable {
    /// `weathers` holds the sounds of each weather of the weather library in the library's order,
    /// which is what a `WeatherId` indexes.
    pub fn new(weathers: Vec<Vec<WeatherSound>>) -> Self {
        Self { weathers }
    }

    /// The sounds of `weather`; a weather the table does not know is an error.
    pub fn sounds(&self, weather: WeatherId) -> Result<&[WeatherSound], WeatherAudioError> {
        self.weathers
            .get(weather.index())
            .map(Vec::as_slice)
            .ok_or(WeatherAudioError::UnknownWeather(weather.index()))
    }

    /// The descriptors of every sound of every weather, each once.
    pub fn descriptors(&self) -> BTreeSet<&str> {
        self.descriptors_where(|_| true)
    }

    /// The descriptors of the sounds that play in a loop, each once.
    pub fn looping_descriptors(&self) -> BTreeSet<&str> {
        self.descriptors_where(plays_in_a_loop)
    }

    /// The descriptors of the thunder, each once.
    pub fn thunder_descriptors(&self) -> BTreeSet<&str> {
        self.descriptors_where(|kind| kind == SoundKind::Thunder)
    }

    fn descriptors_where(&self, wanted: impl Fn(SoundKind) -> bool) -> BTreeSet<&str> {
        self.weathers
            .iter()
            .flatten()
            .filter(|sound| wanted(sound.kind))
            .map(|sound| sound.descriptor.as_str())
            .collect()
    }
}

/// The default, precipitation and wind sounds are beds that run as long as their weather shows;
/// thunder is heard once per strike.
pub fn plays_in_a_loop(kind: SoundKind) -> bool {
    match kind {
        SoundKind::Default | SoundKind::Precipitation | SoundKind::Wind => true,
        SoundKind::Thunder => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sound(kind: SoundKind, descriptor: &str) -> WeatherSound {
        WeatherSound {
            kind,
            descriptor: descriptor.to_string(),
        }
    }

    fn table() -> WeatherSoundTable {
        WeatherSoundTable::new(vec![
            vec![sound(SoundKind::Default, "wind")],
            vec![
                sound(SoundKind::Default, "wind"),
                sound(SoundKind::Precipitation, "rain"),
                sound(SoundKind::Wind, "gust"),
                sound(SoundKind::Thunder, "thunder"),
            ],
            vec![],
        ])
    }

    #[test]
    fn sounds_are_found_by_the_position_of_the_weather() {
        let table = table();

        assert_eq!(
            table.sounds(WeatherId::new(0)).unwrap(),
            [sound(SoundKind::Default, "wind")]
        );
        assert_eq!(table.sounds(WeatherId::new(1)).unwrap().len(), 4);
        assert_eq!(table.sounds(WeatherId::new(2)).unwrap(), []);
    }

    #[test]
    fn a_weather_the_table_does_not_know_is_an_error_naming_it() {
        let error = table().sounds(WeatherId::new(3)).unwrap_err();

        assert!(
            matches!(error, WeatherAudioError::UnknownWeather(3)),
            "{error:?}"
        );
    }

    #[test]
    fn descriptors_are_listed_once_whatever_the_weather_and_split_by_how_they_play() {
        let table = table();

        assert_eq!(
            table.descriptors(),
            BTreeSet::from(["gust", "rain", "thunder", "wind"])
        );
        assert_eq!(
            table.looping_descriptors(),
            BTreeSet::from(["gust", "rain", "wind"])
        );
        assert_eq!(table.thunder_descriptors(), BTreeSet::from(["thunder"]));
    }
}
