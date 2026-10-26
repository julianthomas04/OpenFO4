use std::iter;

use super::WeatherFrameError;
use crate::rendering::lighting::lerp::Lerp;
use crate::rendering::weather::definition::WeatherDef;
use crate::rendering::weather::library::WeatherLibrary;
use crate::sim::WeatherSnapshot;
use crate::weather::{TransitionSide, WeatherId};

/// One of the weathers a frame shows.
#[derive(Debug, Clone, Copy)]
pub(super) struct Side<'a> {
    pub id: WeatherId,
    pub weather: &'a WeatherDef,
    pub role: TransitionSide,
}

/// The weathers of a frame: the one the weather has come to and, while it changes, the one it
/// leaves.
#[derive(Debug, Clone, Copy)]
pub(super) struct Sides<'a> {
    pub incoming: Side<'a>,
    pub outgoing: Option<Side<'a>>,
}

impl<'a> Sides<'a> {
    pub fn of(
        library: &'a WeatherLibrary,
        snapshot: &WeatherSnapshot<'_>,
    ) -> Result<Self, WeatherFrameError> {
        let side = |id, role| -> Result<Side<'a>, WeatherFrameError> {
            Ok(Side {
                id,
                weather: weather_of(library, id)?,
                role,
            })
        };
        let Some(previous) = snapshot.previous else {
            return Ok(Self {
                incoming: side(snapshot.current, TransitionSide::Settled)?,
                outgoing: None,
            });
        };
        let progress = snapshot.progress;
        if !(0.0..=1.0).contains(&progress) {
            return Err(WeatherFrameError::Progress { progress });
        }
        Ok(Self {
            incoming: side(snapshot.current, TransitionSide::Incoming { progress })?,
            outgoing: Some(side(previous, TransitionSide::Outgoing { progress })?),
        })
    }

    /// The weathers, the incoming one first.
    pub fn iter(&self) -> impl Iterator<Item = &Side<'a>> {
        iter::once(&self.incoming).chain(&self.outgoing)
    }

    /// The weathers in the order they are drawn over one another, the one that goes out first.
    pub fn back_to_front(&self) -> impl Iterator<Item = &Side<'a>> {
        self.outgoing.iter().chain(iter::once(&self.incoming))
    }

    /// What `value` makes of each weather, blended by the progress of the transition: the value
    /// of the weather being left at its start and that of the weather it comes to at its end.
    pub fn blend<T: Lerp>(&self, value: impl Fn(&WeatherDef) -> T) -> T {
        match &self.outgoing {
            Some(outgoing) => value(outgoing.weather)
                .lerp(&value(self.incoming.weather), self.incoming.role.weight()),
            None => value(self.incoming.weather),
        }
    }
}

pub(super) fn weather_of(
    library: &WeatherLibrary,
    id: WeatherId,
) -> Result<&WeatherDef, WeatherFrameError> {
    library.get(id).ok_or(WeatherFrameError::UnknownWeather {
        weather: id,
        count: library.weathers().len(),
    })
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the weights and blends at the ends and at the quarter marks are exact"
)]
mod tests {
    use super::*;
    use crate::rendering::weather::frame::fixtures::{A, B, library, settled, transition, weather};
    use crate::rendering::weather::library::fixtures::library_of;

    fn library_with_speeds(speed_of_a: f32, speed_of_b: f32) -> WeatherLibrary {
        let mut a = weather("A");
        a.wind.speed = speed_of_a;
        let mut b = weather("B");
        b.wind.speed = speed_of_b;
        library_of(vec![a, b])
    }

    fn speed(weather: &WeatherDef) -> f32 {
        weather.wind.speed
    }

    #[test]
    fn a_settled_weather_is_all_of_the_frame() {
        let library = library();
        let sides = Sides::of(&library, &settled(B)).unwrap();
        assert_eq!(sides.incoming.id, B);
        assert_eq!(sides.incoming.role, TransitionSide::Settled);
        assert_eq!(sides.incoming.role.weight(), 1.0);
        assert!(sides.outgoing.is_none());
    }

    #[test]
    fn a_transition_shares_the_frame_between_its_weathers_by_its_progress() {
        let library = library();
        let sides = Sides::of(&library, &transition(B, A, 0.25)).unwrap();
        let outgoing = sides.outgoing.unwrap();
        assert_eq!((sides.incoming.id, outgoing.id), (B, A));
        assert_eq!(sides.incoming.role.weight(), 0.25);
        assert_eq!(outgoing.role.weight(), 0.75);
    }

    #[test]
    fn the_ends_of_a_transition_are_the_whole_frame_for_one_weather() {
        let library = library();
        let start = Sides::of(&library, &transition(B, A, 0.0)).unwrap();
        assert_eq!(start.incoming.role.weight(), 0.0);
        assert_eq!(start.outgoing.unwrap().role.weight(), 1.0);
        let end = Sides::of(&library, &transition(B, A, 1.0)).unwrap();
        assert_eq!(end.incoming.role.weight(), 1.0);
        assert_eq!(end.outgoing.unwrap().role.weight(), 0.0);
    }

    #[test]
    fn the_roles_are_those_the_simulation_gives_its_weathers() {
        let library = library();
        for snapshot in [settled(B), transition(B, A, 0.3)] {
            let sides = Sides::of(&library, &snapshot).unwrap();
            let ours: Vec<_> = sides.iter().map(|side| (side.id, side.role)).collect();
            assert_eq!(ours, snapshot.sides().collect::<Vec<_>>());
        }
    }

    #[test]
    fn the_weathers_are_drawn_over_one_another_from_the_one_that_goes_out_to_the_one_that_comes_in()
    {
        let library = library();
        let sides = Sides::of(&library, &transition(B, A, 0.5)).unwrap();
        let drawn: Vec<WeatherId> = sides.back_to_front().map(|side| side.id).collect();
        assert_eq!(drawn, [A, B]);
        let settled = Sides::of(&library, &settled(B)).unwrap();
        let drawn: Vec<WeatherId> = settled.back_to_front().map(|side| side.id).collect();
        assert_eq!(drawn, [B]);
    }

    #[test]
    fn a_settled_weather_blends_to_its_own_value() {
        let library = library_with_speeds(0.2, 0.6);
        let sides = Sides::of(&library, &settled(B)).unwrap();
        assert_eq!(sides.blend(speed), 0.6);
    }

    #[test]
    fn a_transition_blends_from_the_weather_it_leaves_to_the_weather_it_comes_to() {
        let library = library_with_speeds(0.25, 0.75);
        let at = |progress| {
            Sides::of(&library, &transition(B, A, progress))
                .unwrap()
                .blend(speed)
        };
        assert_eq!(at(0.0), 0.25);
        assert_eq!(at(0.5), 0.5);
        assert_eq!(at(1.0), 0.75);
    }

    #[test]
    fn a_weather_the_library_does_not_have_is_an_error_whichever_side_it_is_on() {
        let library = library();
        let unknown = WeatherId::new(7);
        for snapshot in [
            settled(unknown),
            transition(unknown, A, 0.5),
            transition(A, unknown, 0.5),
        ] {
            assert_eq!(
                Sides::of(&library, &snapshot).unwrap_err(),
                WeatherFrameError::UnknownWeather {
                    weather: unknown,
                    count: 2
                }
            );
        }
    }

    #[test]
    fn a_progress_that_is_no_fraction_is_an_error_during_a_transition() {
        let library = library();
        for progress in [-0.1, 1.5, f32::NAN, f32::INFINITY] {
            let error = Sides::of(&library, &transition(B, A, progress)).unwrap_err();
            assert!(
                matches!(error, WeatherFrameError::Progress { progress: shown } if shown.is_nan() == progress.is_nan()),
                "{progress}: {error:?}"
            );
        }
    }

    #[test]
    fn a_settled_weather_has_no_use_for_the_progress() {
        let library = library();
        let snapshot = WeatherSnapshot {
            progress: f32::NAN,
            ..settled(B)
        };
        assert!(Sides::of(&library, &snapshot).is_ok());
    }
}
