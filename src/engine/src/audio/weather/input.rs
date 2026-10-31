//! What the weather sounds need to know each frame, as plain values: nothing here knows the
//! database, the simulation or the renderer.

use crate::weather::{FadeRange, TransitionSide, WeatherId};

/// One of the weathers that show, and how strongly its sounds do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeatherSide {
    pub weather: WeatherId,
    /// The share of the weather in the picture, `0..=1`; see [`WeatherSide::new`].
    pub weight: f32,
    /// How strongly the weather's precipitation shows on this side, `0..=1`.
    pub precipitation: f32,
    /// The weather's wind speed, `0..=1`.
    pub wind: f32,
}

impl WeatherSide {
    /// The side `weather` plays in a transition. A settled weather has the whole picture; during a
    /// transition the weather it leads to has the progress and the weather it leaves the rest, so
    /// the two weights add up to 1.
    pub fn new(
        weather: WeatherId,
        side: TransitionSide,
        precipitation_fade: FadeRange,
        wind_speed: f32,
    ) -> Self {
        Self {
            weather,
            weight: side.weight(),
            precipitation: precipitation_fade.intensity(side),
            wind: wind_speed,
        }
    }
}

/// A strike of lightning whose thunder is on its way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThunderStrike {
    /// The weather whose thunder sounds the strike plays.
    pub weather: WeatherId,
    /// Real seconds from the flash to the thunder.
    pub delay_s: f32,
    /// `0..=1`; the lower, the quieter the thunder.
    pub intensity: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WeatherAudioInput {
    /// The weathers that show: one while the weather is settled, two during a transition.
    pub sides: Vec<WeatherSide>,
    /// The strikes that came in this frame.
    pub strikes: Vec<ThunderStrike>,
    /// Whether the player is outdoors. The weather is not heard indoors.
    pub outdoor: bool,
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the expected weights are exactly representable"
)]
mod tests {
    use super::*;

    const WEATHER: WeatherId = WeatherId::new(3);
    const FADE: FadeRange = FadeRange {
        begin_fade_in: 0.0,
        end_fade_out: 1.0,
    };

    fn side(side: TransitionSide) -> WeatherSide {
        WeatherSide::new(WEATHER, side, FADE, 0.4)
    }

    #[test]
    fn a_settled_weather_has_the_whole_picture() {
        let settled = side(TransitionSide::Settled);

        assert_eq!(settled.weather, WEATHER);
        assert_eq!(settled.weight, 1.0);
    }

    #[test]
    fn the_weights_of_a_transition_follow_its_progress_and_add_up_to_one() {
        for progress in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let incoming = side(TransitionSide::Incoming { progress });
            let outgoing = side(TransitionSide::Outgoing { progress });

            assert_eq!(incoming.weight, progress);
            assert_eq!(incoming.weight + outgoing.weight, 1.0, "{progress}");
        }
    }

    #[test]
    fn the_precipitation_follows_the_fade_of_the_weather_on_its_side() {
        let fade = FadeRange {
            begin_fade_in: 0.5,
            end_fade_out: 0.5,
        };
        let incoming = TransitionSide::Incoming { progress: 0.75 };
        let outgoing = TransitionSide::Outgoing { progress: 0.25 };

        assert_eq!(
            WeatherSide::new(WEATHER, incoming, fade, 0.0).precipitation,
            0.5
        );
        assert_eq!(
            WeatherSide::new(WEATHER, outgoing, fade, 0.0).precipitation,
            0.5
        );
        assert_eq!(
            WeatherSide::new(WEATHER, TransitionSide::Settled, fade, 0.0).precipitation,
            1.0
        );
    }

    #[test]
    fn the_wind_is_the_speed_of_the_weather() {
        assert_eq!(side(TransitionSide::Settled).wind, 0.4);
    }
}
