//! What the weather asks of the weather sounds: the sounds of every weather of the library, and
//! the weather the player hears each frame. Nothing here touches the audio device.

use super::frame::WeatherFrameError;
use super::library::WeatherLibrary;
use crate::audio::weather::{ThunderStrike, WeatherAudioInput, WeatherSide, WeatherSoundTable};
use crate::sim::WeatherSnapshot;

/// The sounds of every weather, in the order of the library.
pub fn sound_table(library: &WeatherLibrary) -> WeatherSoundTable {
    WeatherSoundTable::new(
        library
            .weathers()
            .iter()
            .map(|weather| weather.sounds.clone())
            .collect(),
    )
}

/// The weather the player hears at `snapshot`: the weathers that show with their precipitation and
/// wind, and the strikes of the step the snapshot is of. Take it once after each step of the
/// weather, as the strikes of a snapshot last one step.
pub fn audio_input(
    library: &WeatherLibrary,
    snapshot: &WeatherSnapshot<'_>,
    outdoor: bool,
) -> Result<WeatherAudioInput, WeatherFrameError> {
    let sides = snapshot
        .sides()
        .map(|(id, side)| {
            let weather = library.get(id).ok_or(WeatherFrameError::UnknownWeather {
                weather: id,
                count: library.weathers().len(),
            })?;
            Ok(WeatherSide::new(
                id,
                side,
                weather.precipitation_fade,
                weather.wind.speed,
            ))
        })
        .collect::<Result<_, WeatherFrameError>>()?;
    let strikes = snapshot
        .strikes
        .iter()
        .map(|strike| ThunderStrike {
            weather: strike.weather,
            delay_s: strike.thunder_delay_s,
            intensity: strike.flash_intensity,
        })
        .collect();
    Ok(WeatherAudioInput {
        sides,
        strikes,
        outdoor,
    })
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "the copied values of the tests are exact")]
mod tests {
    use super::*;
    use crate::rendering::weather::frame::fixtures::{
        A, B, library, settled, transition, weather, with_thunder,
    };
    use crate::rendering::weather::library::fixtures::library_of;
    use crate::sim::commands::WeatherEdit;
    use crate::sim::world_clock::WorldClock;
    use crate::sim::{ClockFrame, LightningStrike, PlayerMovement, SimCommand, SimWorld};
    use crate::weather::{SoundKind, WeatherId, WeatherSound};

    fn strike(weather: WeatherId) -> LightningStrike {
        LightningStrike {
            weather,
            distance_m: 900.0,
            thunder_delay_s: 2.6,
            flash_intensity: 0.7,
        }
    }

    #[test]
    fn the_table_holds_the_sounds_of_each_weather_by_its_id() {
        let mut quiet = weather("B");
        quiet.sounds.clear();
        let library = library_of(vec![with_thunder(weather("A"), 0.0, 1.0), quiet]);

        let table = sound_table(&library);

        assert_eq!(table.sounds(A).unwrap(), library.get(A).unwrap().sounds);
        assert!(table.sounds(A).unwrap().contains(&WeatherSound {
            kind: SoundKind::Thunder,
            descriptor: "00BB02:Fallout4.esm".to_owned(),
        }));
        assert_eq!(table.sounds(B).unwrap(), []);
        assert!(table.sounds(WeatherId::new(2)).is_err());
    }

    #[test]
    fn a_settled_weather_is_heard_whole_and_a_transition_as_the_two_weathers() {
        let library = library();

        let settled = audio_input(&library, &settled(A), true).unwrap();
        assert_eq!(settled.sides.len(), 1);
        assert_eq!(settled.sides[0].weather, A);
        assert_eq!(settled.sides[0].weight, 1.0);

        let changing = audio_input(&library, &transition(B, A, 0.25), true).unwrap();
        let weights: Vec<_> = changing
            .sides
            .iter()
            .map(|side| (side.weather, side.weight))
            .collect();
        assert_eq!(weights, [(B, 0.25), (A, 0.75)]);
    }

    #[test]
    fn the_wind_of_a_side_is_the_speed_of_its_weather() {
        let library = library();

        let input = audio_input(&library, &settled(A), true).unwrap();

        assert_eq!(input.sides[0].wind, library.get(A).unwrap().wind.speed);
    }

    #[test]
    fn the_player_hears_the_weather_outdoors_only() {
        let library = library();

        assert!(audio_input(&library, &settled(A), true).unwrap().outdoor);
        assert!(!audio_input(&library, &settled(A), false).unwrap().outdoor);
    }

    #[test]
    fn a_strike_brings_its_weather_its_delay_and_its_intensity() {
        let library = library();
        let strikes = [strike(B)];
        let snapshot = WeatherSnapshot {
            strikes: &strikes,
            ..settled(A)
        };

        let input = audio_input(&library, &snapshot, true).unwrap();

        assert_eq!(
            input.strikes,
            [ThunderStrike {
                weather: B,
                delay_s: 2.6,
                intensity: 0.7,
            }]
        );
    }

    #[test]
    fn a_weather_the_library_lacks_is_an_error() {
        let library = library();
        let unknown = WeatherId::new(7);

        assert_eq!(
            audio_input(&library, &settled(unknown), true),
            Err(WeatherFrameError::UnknownWeather {
                weather: unknown,
                count: 2
            })
        );
    }

    /// A world with the weather `A` of a library of a stormy weather.
    fn storm_world() -> SimWorld {
        let library = library_of(vec![with_thunder(weather("A"), 0.0, 1.0)]);
        let mut sim = SimWorld::new(PlayerMovement::default(), WorldClock::fixture());
        sim.install_weather(library.roll_table().unwrap(), 7);
        sim
    }

    /// One frame as the game runs it: the weather steps on, then the player hears it.
    fn heard_after_a_frame(sim: &mut SimWorld, library: &WeatherLibrary, in_game: bool) -> usize {
        sim.advance_world_clock(ClockFrame {
            in_game,
            world_dt: 0.25,
        });
        audio_input(library, &sim.weather_snapshot().unwrap(), true)
            .unwrap()
            .strikes
            .len()
    }

    #[test]
    fn a_strike_is_heard_in_the_frame_it_strikes_and_once() {
        let library = library_of(vec![with_thunder(weather("A"), 0.0, 1.0)]);
        let mut sim = storm_world();
        sim.apply(SimCommand::SetWeather(WeatherEdit::TriggerLightning));

        let heard: Vec<usize> = (0..4)
            .map(|_| heard_after_a_frame(&mut sim, &library, true))
            .collect();

        assert_eq!(heard, [1, 0, 0, 0]);
    }

    #[test]
    fn a_strike_is_not_heard_again_in_frames_out_of_gameplay() {
        let library = library_of(vec![with_thunder(weather("A"), 0.0, 1.0)]);
        let mut sim = storm_world();
        sim.apply(SimCommand::SetWeather(WeatherEdit::TriggerLightning));

        let heard: Vec<usize> = [true, false, false]
            .into_iter()
            .map(|in_game| heard_after_a_frame(&mut sim, &library, in_game))
            .collect();

        assert_eq!(heard, [1, 0, 0]);
    }
}
