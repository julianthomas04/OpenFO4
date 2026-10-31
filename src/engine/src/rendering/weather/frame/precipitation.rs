use super::sides::{Side, Sides};
use crate::weather::{WeatherFlags, WeatherId};

/// The precipitation of a frame. Which weathers show it, and how much, is up to the fade ranges
/// of their precipitation, so it comes in and goes out as the weather changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrecipitationFrame {
    /// The weather whose precipitation falls: of the weathers that show some, the one that shows
    /// most. Its precipitation is what the library says of this weather.
    pub weather: WeatherId,
    /// `0..=1`, above 0: how dense the precipitation is. The weathers of a transition add up,
    /// as the drops of two weathers would fall together.
    pub intensity: f32,
    /// The flags of the weather whose precipitation falls.
    pub flags: WeatherFlags,
}

/// `None` while no weather shows precipitation. A weather without precipitation of its own
/// shows none whatever its fade range says. When two weathers show equally much, the one the
/// weather comes to is the one that falls.
pub(super) fn precipitation(sides: &Sides) -> Option<PrecipitationFrame> {
    let shown = || {
        sides.iter().filter_map(|side| {
            side.weather.precipitation.as_ref()?;
            let intensity = side.weather.precipitation_fade.intensity(side.role);
            (intensity > 0.0).then_some((side, intensity))
        })
    };
    let (dominant, _) = shown().reduce(|strongest, candidate| {
        if candidate.1 > strongest.1 {
            candidate
        } else {
            strongest
        }
    })?;
    Some(PrecipitationFrame {
        weather: dominant.id,
        intensity: shown()
            .map(|(_, intensity)| intensity)
            .sum::<f32>()
            .min(1.0),
        flags: dominant.weather.flags,
    })
}

/// How strong the thunder of the frame is, `0..=1`: the thunder of the weathers that have any,
/// as far as their fade ranges let them show it. The weathers of a transition add up.
pub(super) fn thunder_intensity(sides: &Sides) -> f32 {
    // Not `sum`: the sum of no thunder at all is `-0.0`, which a readout shows as "-0.00".
    sides
        .iter()
        .filter(|side| side.weather.has_thunder_sound())
        .map(thunder_of)
        .fold(0.0, |total, thunder| total + thunder)
        .min(1.0)
}

fn thunder_of(side: &Side) -> f32 {
    side.weather.lightning.fade.intensity(side.role)
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the fade ramps of the tests land on exact fractions"
)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::rendering::weather::WeatherLibrary;
    use crate::rendering::weather::definition::WeatherDef;
    use crate::rendering::weather::frame::fixtures::{
        A, B, dry, library, settled, transition, weather, with_precipitation_fade, with_thunder,
    };
    use crate::rendering::weather::library::fixtures::library_of;
    use crate::sim::WeatherSnapshot;
    use crate::weather::WeatherFlag;

    fn precipitation_at(
        library: &WeatherLibrary,
        snapshot: &WeatherSnapshot<'_>,
    ) -> Option<PrecipitationFrame> {
        precipitation(&Sides::of(library, snapshot).unwrap())
    }

    fn thunder_at(library: &WeatherLibrary, snapshot: &WeatherSnapshot<'_>) -> f32 {
        thunder_intensity(&Sides::of(library, snapshot).unwrap())
    }

    fn intensity_at(library: &WeatherLibrary, snapshot: &WeatherSnapshot<'_>) -> Option<f32> {
        precipitation_at(library, snapshot).map(|frame| frame.intensity)
    }

    fn with_flags(mut weather: WeatherDef, flags: &[WeatherFlag]) -> WeatherDef {
        let bits = flags
            .iter()
            .fold(0, |bits, flag| bits | u32::from(flag.bit()));
        weather.flags = WeatherFlags::from_bits(bits).unwrap();
        weather
    }

    #[test]
    fn a_settled_weather_with_precipitation_shows_it_fully() {
        let library = library();
        let frame = precipitation_at(&library, &settled(A)).unwrap();
        assert_eq!(frame.weather, A);
        assert_eq!(frame.intensity, 1.0);
        assert_eq!(frame.flags, library.get(A).unwrap().flags);
    }

    #[test]
    fn a_settled_weather_without_precipitation_shows_none_whatever_its_fade_range_says() {
        let library = library_of(vec![dry(weather("A"))]);
        assert_eq!(precipitation_at(&library, &settled(A)), None);
    }

    #[test]
    fn precipitation_comes_in_as_the_fade_range_of_the_weather_coming_in_says() {
        let library = library_of(vec![
            dry(weather("A")),
            with_precipitation_fade(weather("B"), 0.5, 1.0),
        ]);
        let at = |progress| intensity_at(&library, &transition(B, A, progress));
        assert_eq!(at(0.25), None);
        assert_eq!(at(0.5), None);
        assert_eq!(at(0.75), Some(0.5));
        assert_eq!(at(1.0), Some(1.0));
    }

    #[test]
    fn precipitation_goes_out_as_the_fade_range_of_the_weather_going_out_says() {
        let library = library_of(vec![
            with_precipitation_fade(weather("A"), 0.0, 0.5),
            dry(weather("B")),
        ]);
        let at = |progress| intensity_at(&library, &transition(B, A, progress));
        assert_eq!(at(0.0), Some(1.0));
        assert_eq!(at(0.25), Some(0.5));
        assert_eq!(at(0.5), None);
        assert_eq!(at(0.75), None);
    }

    #[test]
    fn the_flags_are_those_of_the_weather_whose_precipitation_falls() {
        let library = library_of(vec![
            with_flags(
                weather("A"),
                &[WeatherFlag::Rainy, WeatherFlag::RainOcclusion],
            ),
            with_flags(dry(weather("B")), &[WeatherFlag::Pleasant]),
        ]);
        let frame = precipitation_at(&library, &transition(B, A, 0.25)).unwrap();
        assert_eq!(frame.weather, A);
        assert!(frame.flags.contains(WeatherFlag::RainOcclusion));
        assert!(!frame.flags.contains(WeatherFlag::Pleasant));
    }

    fn two_downpours() -> WeatherLibrary {
        library_of(vec![
            with_precipitation_fade(weather("A"), 0.0, 1.0),
            with_precipitation_fade(weather("B"), 0.0, 1.0),
        ])
    }

    #[test]
    fn of_two_weathers_that_show_precipitation_the_one_that_shows_more_falls() {
        let library = two_downpours();
        let falling = |progress| {
            precipitation_at(&library, &transition(B, A, progress))
                .unwrap()
                .weather
        };
        assert_eq!(falling(0.25), A);
        assert_eq!(falling(0.75), B);
    }

    #[test]
    fn of_two_weathers_that_show_equally_much_the_one_coming_in_falls() {
        let frame = precipitation_at(&two_downpours(), &transition(B, A, 0.5)).unwrap();
        assert_eq!(frame.weather, B);
    }

    #[test]
    fn the_precipitation_of_two_weathers_adds_up() {
        let library = library_of(vec![
            with_precipitation_fade(weather("A"), 0.0, 1.0),
            with_precipitation_fade(weather("B"), 0.5, 1.0),
        ]);
        // A has 0.4 left and B has come a fifth of the way from its begin point.
        let frame = precipitation_at(&library, &transition(B, A, 0.6)).unwrap();
        assert!((frame.intensity - 0.6).abs() < 1e-6, "{}", frame.intensity);
        assert_eq!(frame.weather, A);
    }

    #[test]
    fn a_weather_without_precipitation_does_not_add_to_it() {
        let library = library_of(vec![
            with_precipitation_fade(weather("A"), 0.0, 1.0),
            dry(weather("B")),
        ]);
        assert_eq!(intensity_at(&library, &transition(B, A, 0.25)), Some(0.75));
    }

    #[test]
    fn a_weather_without_thunder_has_none_whatever_its_fade_range_says() {
        let thunder = thunder_at(&library(), &settled(A));
        assert_eq!(thunder, 0.0);
        assert!(
            thunder.is_sign_positive(),
            "a readout of -0 is no readout of none"
        );
    }

    #[test]
    fn a_settled_weather_with_thunder_has_it_fully() {
        let library = library_of(vec![with_thunder(weather("A"), 0.2, 0.8)]);
        assert_eq!(thunder_at(&library, &settled(A)), 1.0);
    }

    #[test]
    fn thunder_comes_in_and_goes_out_as_the_fade_ranges_of_the_weathers_say() {
        let library = library_of(vec![
            with_thunder(weather("A"), 0.5, 0.5),
            with_thunder(weather("B"), 0.5, 0.5),
        ]);
        let thunder = |progress| thunder_at(&library, &transition(B, A, progress));
        // Only the weather going out thunders until half way, only the one coming in from then.
        assert_eq!(thunder(0.0), 1.0);
        assert_eq!(thunder(0.25), 0.5);
        assert_eq!(thunder(0.5), 0.0);
        assert_eq!(thunder(0.75), 0.5);
        assert_eq!(thunder(1.0), 1.0);
    }

    #[test]
    fn only_the_weather_that_has_thunder_thunders_during_a_transition() {
        let library = library_of(vec![with_thunder(weather("A"), 0.0, 1.0), weather("B")]);
        assert_eq!(thunder_at(&library, &transition(B, A, 0.25)), 0.75);
    }

    fn fades() -> impl Strategy<Value = (f32, f32)> {
        (-0.5_f32..1.5, -0.5_f32..1.5)
    }

    proptest! {
        #[test]
        fn precipitation_and_thunder_never_pass_a_full_downpour_nor_go_below_none(
            (out_begin, out_end) in fades(),
            (in_begin, in_end) in fades(),
            progress in 0.0_f32..=1.0,
            leaving_is_dry: bool,
            coming_is_dry: bool,
        ) {
            let leaving = with_thunder(with_precipitation_fade(weather("A"), out_begin, out_end), out_begin, out_end);
            let coming = with_thunder(with_precipitation_fade(weather("B"), in_begin, in_end), in_begin, in_end);
            let library = library_of(vec![
                if leaving_is_dry { dry(leaving) } else { leaving },
                if coming_is_dry { dry(coming) } else { coming },
            ]);
            let snapshot = transition(B, A, progress);
            if let Some(frame) = precipitation_at(&library, &snapshot) {
                prop_assert!(frame.intensity > 0.0 && frame.intensity <= 1.0, "{}", frame.intensity);
            }
            let thunder = thunder_at(&library, &snapshot);
            prop_assert!((0.0..=1.0).contains(&thunder), "{thunder}");
        }
    }
}
