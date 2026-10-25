use serde::Deserialize;

use super::wrap::HOURS_PER_DAY;

/// The two twilight windows of a climate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClimateWindow {
    Sunrise,
    Sunset,
}

#[derive(Debug, thiserror::Error)]
pub enum ClimateTimesError {
    #[error("malformed climate JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{field} is not an HH:MM:SS clock time: {value:?}")]
    MalformedClock { field: &'static str, value: String },
    #[error("{field} {value:?} has an hour, minute or second out of range")]
    ClockOutOfRange { field: &'static str, value: String },
    #[error("{field} is {hours} h, outside [0, 24)")]
    HourOutOfRange { field: &'static str, hours: f32 },
    #[error("{window:?} window does not increase (begin {begin} h, end {end} h)")]
    WindowNotIncreasing {
        window: ClimateWindow,
        begin: f32,
        end: f32,
    },
    #[error("sunrise ends at {sunrise_end} h, after sunset begins at {sunset_begin} h")]
    WindowsOverlap { sunrise_end: f32, sunset_begin: f32 },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawClimateTimes {
    sunrise_begin: String,
    sunrise_end: String,
    sunset_begin: String,
    sunset_end: String,
}

/// The sunrise and sunset windows of a climate in hours of the day, validated
/// so that `0 <= sunrise_begin < sunrise_end <= sunset_begin < sunset_end < 24`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClimateTimes {
    sunrise_begin: f32,
    sunrise_end: f32,
    sunset_begin: f32,
    sunset_end: f32,
}

impl ClimateTimes {
    pub fn new(
        sunrise_begin: f32,
        sunrise_end: f32,
        sunset_begin: f32,
        sunset_end: f32,
    ) -> Result<Self, ClimateTimesError> {
        let bounds = [
            ("sunrise begin", sunrise_begin),
            ("sunrise end", sunrise_end),
            ("sunset begin", sunset_begin),
            ("sunset end", sunset_end),
        ];
        for (field, hours) in bounds {
            if !(0.0..HOURS_PER_DAY).contains(&hours) {
                return Err(ClimateTimesError::HourOutOfRange { field, hours });
            }
        }
        for (window, begin, end) in [
            (ClimateWindow::Sunrise, sunrise_begin, sunrise_end),
            (ClimateWindow::Sunset, sunset_begin, sunset_end),
        ] {
            if begin >= end {
                return Err(ClimateTimesError::WindowNotIncreasing { window, begin, end });
            }
        }
        if sunrise_end > sunset_begin {
            return Err(ClimateTimesError::WindowsOverlap {
                sunrise_end,
                sunset_begin,
            });
        }
        Ok(Self {
            sunrise_begin,
            sunrise_end,
            sunset_begin,
            sunset_end,
        })
    }

    /// From the four `"HH:MM:SS"` strings the database stores for a climate.
    pub fn from_clock_strings(
        sunrise_begin: &str,
        sunrise_end: &str,
        sunset_begin: &str,
        sunset_end: &str,
    ) -> Result<Self, ClimateTimesError> {
        Self::new(
            parse_clock("SunriseBegin", sunrise_begin)?,
            parse_clock("SunriseEnd", sunrise_end)?,
            parse_clock("SunsetBegin", sunset_begin)?,
            parse_clock("SunsetEnd", sunset_end)?,
        )
    }

    /// From a climate's `data` JSON; keys other than the four windows are ignored.
    pub fn parse(climate_json: &str) -> Result<Self, ClimateTimesError> {
        let raw: RawClimateTimes = serde_json::from_str(climate_json)?;
        Self::from_clock_strings(
            &raw.sunrise_begin,
            &raw.sunrise_end,
            &raw.sunset_begin,
            &raw.sunset_end,
        )
    }

    pub fn sunrise_begin(&self) -> f32 {
        self.sunrise_begin
    }

    pub fn sunrise_end(&self) -> f32 {
        self.sunrise_end
    }

    pub fn sunset_begin(&self) -> f32 {
        self.sunset_begin
    }

    pub fn sunset_end(&self) -> f32 {
        self.sunset_end
    }

    pub fn window(&self, window: ClimateWindow) -> (f32, f32) {
        match window {
            ClimateWindow::Sunrise => (self.sunrise_begin, self.sunrise_end),
            ClimateWindow::Sunset => (self.sunset_begin, self.sunset_end),
        }
    }

    pub fn sunrise_midpoint(&self) -> f32 {
        f32::midpoint(self.sunrise_begin, self.sunrise_end)
    }

    pub fn sunset_midpoint(&self) -> f32 {
        f32::midpoint(self.sunset_begin, self.sunset_end)
    }
}

fn parse_clock(field: &'static str, text: &str) -> Result<f32, ClimateTimesError> {
    let malformed = || ClimateTimesError::MalformedClock {
        field,
        value: text.to_owned(),
    };
    let mut parts = text.split(':');
    let (Some(hours), Some(minutes), Some(seconds), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(malformed());
    };
    let number = |part: &str| {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        part.parse::<u32>().ok()
    };
    let (Some(hours), Some(minutes), Some(seconds)) =
        (number(hours), number(minutes), number(seconds))
    else {
        return Err(malformed());
    };
    if hours >= 24 || minutes >= 60 || seconds >= 60 {
        return Err(ClimateTimesError::ClockOutOfRange {
            field,
            value: text.to_owned(),
        });
    }
    Ok(hours as f32 + minutes as f32 / 60.0 + seconds as f32 / 3600.0)
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact binary fractions of an hour"
)]
mod tests {
    use super::*;

    const DEFAULT_CLIMATE_JSON: &str = r#"{"Weathers":[{"Weather":"02B52A:Fallout4.esm","Chance":51}],"SunriseBegin":"05:00:00","SunriseEnd":"09:00:00","SunsetBegin":"17:00:00","SunsetEnd":"21:00:00"}"#;

    fn times_of(json: &str) -> ClimateTimes {
        ClimateTimes::parse(json).unwrap()
    }

    fn json(
        sunrise_begin: &str,
        sunrise_end: &str,
        sunset_begin: &str,
        sunset_end: &str,
    ) -> String {
        format!(
            r#"{{"SunriseBegin":"{sunrise_begin}","SunriseEnd":"{sunrise_end}","SunsetBegin":"{sunset_begin}","SunsetEnd":"{sunset_end}"}}"#
        )
    }

    fn hours(times: &ClimateTimes) -> [f32; 4] {
        [
            times.sunrise_begin(),
            times.sunrise_end(),
            times.sunset_begin(),
            times.sunset_end(),
        ]
    }

    #[test]
    fn the_three_vanilla_climates_parse_to_their_hours() {
        assert_eq!(
            hours(&times_of(DEFAULT_CLIMATE_JSON)),
            [5.0, 9.0, 17.0, 21.0]
        );
        assert_eq!(
            hours(&times_of(&json(
                "05:30:00", "08:00:00", "16:00:00", "20:30:00"
            ))),
            [5.5, 8.0, 16.0, 20.5]
        );
        assert_eq!(
            hours(&times_of(&json(
                "04:00:00", "10:00:00", "16:00:00", "22:00:00"
            ))),
            [4.0, 10.0, 16.0, 22.0]
        );
    }

    #[test]
    fn minutes_and_seconds_contribute_fractions_of_an_hour() {
        let times = times_of(&json("05:15:36", "09:00:00", "17:00:00", "23:59:59"));
        assert!((times.sunrise_begin() - (5.0 + 15.0 / 60.0 + 36.0 / 3600.0)).abs() < 1e-6);
        assert!((times.sunset_end() - (24.0 - 1.0 / 3600.0)).abs() < 1e-4);
    }

    #[test]
    fn midpoints_are_the_centres_of_both_windows() {
        let times = times_of(DEFAULT_CLIMATE_JSON);
        assert_eq!(times.sunrise_midpoint(), 7.0);
        assert_eq!(times.sunset_midpoint(), 19.0);
        assert_eq!(times.window(ClimateWindow::Sunrise), (5.0, 9.0));
        assert_eq!(times.window(ClimateWindow::Sunset), (17.0, 21.0));
    }

    #[test]
    fn a_sunrise_that_ends_exactly_when_sunset_begins_is_allowed() {
        let times = times_of(&json("05:00:00", "12:00:00", "12:00:00", "21:00:00"));
        assert_eq!(times.sunrise_end(), times.sunset_begin());
    }

    #[test]
    fn malformed_clock_strings_name_the_field_and_value() {
        for bad in [
            "",
            "5",
            "05:00",
            "05:00:00:00",
            "aa:00:00",
            "05:0x:00",
            "+5:00:00",
            "-1:00:00",
            "05:00:00.5",
            "1.05:00:00",
            " 05:00:00",
            "99999999999:00:00",
            "::",
        ] {
            let result = ClimateTimes::parse(&json(bad, "09:00:00", "17:00:00", "21:00:00"));
            assert!(
                matches!(&result, Err(ClimateTimesError::MalformedClock { field: "SunriseBegin", value }) if value == bad),
                "{bad:?}: {result:?}"
            );
        }
    }

    #[test]
    fn every_field_is_checked() {
        let fields = ["SunriseBegin", "SunriseEnd", "SunsetBegin", "SunsetEnd"];
        for (index, expected) in fields.into_iter().enumerate() {
            let mut texts = ["05:00:00", "09:00:00", "17:00:00", "21:00:00"];
            texts[index] = "xx";
            let result = ClimateTimes::from_clock_strings(texts[0], texts[1], texts[2], texts[3]);
            assert!(
                matches!(&result, Err(ClimateTimesError::MalformedClock { field, .. }) if *field == expected),
                "{expected}: {result:?}"
            );
        }
    }

    #[test]
    fn clock_components_out_of_range_are_rejected() {
        for bad in ["24:00:00", "12:60:00", "12:00:60", "99:99:99"] {
            let result = ClimateTimes::parse(&json("05:00:00", "09:00:00", "17:00:00", bad));
            assert!(
                matches!(&result, Err(ClimateTimesError::ClockOutOfRange { field: "SunsetEnd", value }) if value == bad),
                "{bad:?}: {result:?}"
            );
        }
    }

    #[test]
    fn a_window_that_does_not_increase_is_rejected() {
        for (begin, end) in [("09:00:00", "09:00:00"), ("10:00:00", "09:00:00")] {
            let result = ClimateTimes::parse(&json(begin, end, "17:00:00", "21:00:00"));
            assert!(
                matches!(
                    result,
                    Err(ClimateTimesError::WindowNotIncreasing {
                        window: ClimateWindow::Sunrise,
                        ..
                    })
                ),
                "{begin}..{end}"
            );
        }
        let result = ClimateTimes::parse(&json("05:00:00", "09:00:00", "21:00:00", "17:00:00"));
        assert!(matches!(
            result,
            Err(ClimateTimesError::WindowNotIncreasing {
                window: ClimateWindow::Sunset,
                ..
            })
        ));
    }

    #[test]
    fn sunrise_ending_after_sunset_begins_is_rejected() {
        let result = ClimateTimes::parse(&json("05:00:00", "18:00:00", "17:00:00", "21:00:00"));
        assert!(matches!(
            result,
            Err(ClimateTimesError::WindowsOverlap { .. })
        ));
    }

    #[test]
    fn missing_keys_and_broken_json_are_json_errors() {
        for bad in [r#"{"SunriseBegin":"05:00:00"}"#, "{ not json", ""] {
            assert!(
                matches!(ClimateTimes::parse(bad), Err(ClimateTimesError::Json(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn numeric_hours_outside_a_day_or_not_finite_are_rejected() {
        for bad in [-0.5, 24.0, f32::NAN, f32::INFINITY] {
            let result = ClimateTimes::new(bad, 9.0, 17.0, 21.0);
            assert!(
                matches!(
                    result,
                    Err(ClimateTimesError::HourOutOfRange {
                        field: "sunrise begin",
                        ..
                    })
                ),
                "{bad}"
            );
        }
    }
}
