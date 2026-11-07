//! Dev overrides of what the world looks like at launch: where the camera looks, its field of
//! view, the time of day and the weather.

use crate::bethesda_space::yaw_from_rotation_z;
use crate::rendering::camera::parse_spawn_look;
use crate::rendering::weather::WeatherLibrary;
use crate::weather::WeatherId;

const SPAWN_LOOK_ENV: &str = "OPENFO4_SPAWN_LOOK";
const SPAWN_ANGLE_ENV: &str = "OPENFO4_SPAWN_ANGLE";
const FOV_ENV: &str = "OPENFO4_FOV";
const TIME_ENV: &str = "OPENFO4_TIME";
const WEATHER_ENV: &str = "OPENFO4_WEATHER";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SceneOverrideError {
    #[error("{SPAWN_LOOK_ENV}={0:?} is not `yaw_deg,pitch_deg` with finite numbers")]
    InvalidSpawnLook(String),
    #[error("{SPAWN_ANGLE_ENV}={0:?} is not `x,y,z` in degrees with finite numbers")]
    InvalidSpawnAngle(String),
    #[error("{SPAWN_ANGLE_ENV}={0:?} has a roll (y); the camera cannot roll")]
    SpawnAngleWithRoll(String),
    #[error("{SPAWN_LOOK_ENV} and {SPAWN_ANGLE_ENV} cannot be set together")]
    LookAndAngleBothSet,
    #[error("{FOV_ENV}={0:?} is not a number above 0 and below 180")]
    InvalidFov(String),
    #[error("{TIME_ENV}={0:?} is not an hour of the day like 12, 6.5 or 12:30")]
    InvalidTime(String),
    #[error("{WEATHER_ENV} is empty; expected a weather form key or editor id")]
    EmptyWeather,
    #[error("{WEATHER_ENV}={0:?} is set but the world has no weather cycle to force it in")]
    NoWeatherCycle(String),
    #[error("{WEATHER_ENV}={0:?} is not a weather of the climate of the worldspace")]
    UnknownWeather(String),
}

/// The look direction of the camera, in radians: engine yaw (0 faces +X/east) and pitch
/// (positive looks up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpawnLook {
    pub yaw: f32,
    pub pitch: f32,
}

/// A weather named by its form key (`1C3473:Fallout4.esm`) or its editor id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeatherSelector(String);

impl WeatherSelector {
    /// The index of the weather `self` names among `weathers`, given as (form key, editor id).
    /// Form keys match by their numeric id and their plugin, ignoring case and leading zeros;
    /// editor ids ignore case, as the game's do.
    pub fn find<'a>(
        &self,
        weathers: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
    ) -> Result<usize, SceneOverrideError> {
        let wanted = FormKey::parse(&self.0);
        weathers
            .into_iter()
            .position(|(form_key, editor_id)| match &wanted {
                Some(wanted) => FormKey::parse(form_key).as_ref() == Some(wanted),
                None => editor_id.is_some_and(|id| id.eq_ignore_ascii_case(&self.0)),
            })
            .ok_or_else(|| SceneOverrideError::UnknownWeather(self.0.clone()))
    }

    /// The weather of the climate's `library` that `self` names.
    pub fn resolve(
        &self,
        library: Option<&WeatherLibrary>,
    ) -> Result<WeatherId, SceneOverrideError> {
        let library = library.ok_or_else(|| SceneOverrideError::NoWeatherCycle(self.0.clone()))?;
        let weathers = library
            .weathers()
            .iter()
            .map(|weather| (weather.form_key.as_str(), weather.editor_id.as_deref()));
        self.find(weathers).map(WeatherId::new)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct FormKey {
    id: u32,
    plugin: String,
}

impl FormKey {
    fn parse(text: &str) -> Option<Self> {
        let (id, plugin) = text.trim().split_once(':')?;
        let id = id.trim();
        let id = id
            .strip_prefix("0x")
            .or_else(|| id.strip_prefix("0X"))
            .unwrap_or(id);
        Some(Self {
            id: u32::from_str_radix(id, 16).ok()?,
            plugin: plugin.trim().to_ascii_lowercase(),
        })
    }
}

/// The overrides as their environment variables give them.
#[derive(Debug, Default)]
struct OverrideEnv {
    spawn_look: Option<String>,
    spawn_angle: Option<String>,
    fov: Option<String>,
    time: Option<String>,
    weather: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SceneOverrides {
    pub look: Option<SpawnLook>,
    /// The world field of view in the convention of `fDefaultWorldFOV`, in place of the INI's.
    pub world_fov: Option<f32>,
    /// The hour of the day to start at with the world clock paused.
    pub hour: Option<f32>,
    pub weather: Option<WeatherSelector>,
}

impl SceneOverrides {
    pub fn from_env() -> Result<Self, SceneOverrideError> {
        let read = crate::utils::optional_env_var;
        Self::parse(OverrideEnv {
            spawn_look: read(SPAWN_LOOK_ENV),
            spawn_angle: read(SPAWN_ANGLE_ENV),
            fov: read(FOV_ENV),
            time: read(TIME_ENV),
            weather: read(WEATHER_ENV),
        })
    }

    fn parse(env: OverrideEnv) -> Result<Self, SceneOverrideError> {
        let look = match (env.spawn_look, env.spawn_angle) {
            (Some(_), Some(_)) => return Err(SceneOverrideError::LookAndAngleBothSet),
            (Some(raw), None) => Some(
                parse_spawn_look(&raw)
                    .map(|(yaw, pitch)| SpawnLook { yaw, pitch })
                    .ok_or(SceneOverrideError::InvalidSpawnLook(raw))?,
            ),
            (None, Some(raw)) => Some(look_from_spawn_angle(&raw)?),
            (None, None) => None,
        };
        Ok(Self {
            look,
            world_fov: env.fov.map(|raw| parse_fov(&raw)).transpose()?,
            hour: env.time.map(|raw| parse_hour(&raw)).transpose()?,
            weather: env.weather.as_deref().map(parse_weather).transpose()?,
        })
    }
}

/// `x,y,z` as the console's `player.getangle` prints them, in degrees: x pitches the view down,
/// z is the compass heading clockwise from north, y is a roll the camera does not have.
fn look_from_spawn_angle(raw: &str) -> Result<SpawnLook, SceneOverrideError> {
    let invalid = || SceneOverrideError::InvalidSpawnAngle(raw.to_owned());
    let angles = raw
        .split(',')
        .map(|part| part.trim().parse::<f32>().ok().filter(|v| v.is_finite()))
        .collect::<Option<Vec<f32>>>()
        .ok_or_else(invalid)?;
    let [x, y, z] = angles.as_slice() else {
        return Err(invalid());
    };
    if *y != 0.0 {
        return Err(SceneOverrideError::SpawnAngleWithRoll(raw.to_owned()));
    }
    Ok(SpawnLook {
        yaw: yaw_from_rotation_z(z.to_radians()),
        pitch: -x.to_radians(),
    })
}

fn parse_fov(raw: &str) -> Result<f32, SceneOverrideError> {
    raw.trim()
        .parse::<f32>()
        .ok()
        .filter(|fov| *fov > 0.0 && *fov < 180.0)
        .ok_or_else(|| SceneOverrideError::InvalidFov(raw.to_owned()))
}

fn parse_hour(raw: &str) -> Result<f32, SceneOverrideError> {
    let invalid = || SceneOverrideError::InvalidTime(raw.to_owned());
    let hour = match raw.trim().split_once(':') {
        Some((hours, minutes)) => {
            let hours: u8 = hours.trim().parse().map_err(|_| invalid())?;
            let minutes: u8 = minutes.trim().parse().map_err(|_| invalid())?;
            if hours > 23 || minutes > 59 {
                return Err(invalid());
            }
            f32::from(hours) + f32::from(minutes) / 60.0
        }
        None => raw.trim().parse::<f32>().map_err(|_| invalid())?,
    };
    if (0.0..24.0).contains(&hour) {
        Ok(hour)
    } else {
        Err(invalid())
    }
}

fn parse_weather(raw: &str) -> Result<WeatherSelector, SceneOverrideError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(SceneOverrideError::EmptyWeather);
    }
    Ok(WeatherSelector(trimmed.to_owned()))
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI};

    use super::*;

    fn parse(env: OverrideEnv) -> Result<SceneOverrides, SceneOverrideError> {
        SceneOverrides::parse(env)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn nothing_set_overrides_nothing() {
        assert_eq!(parse(OverrideEnv::default()), Ok(SceneOverrides::default()));
    }

    #[test]
    fn spawn_look_is_engine_yaw_and_pitch_in_degrees() {
        let overrides = parse(OverrideEnv {
            spawn_look: Some("90,-45".to_owned()),
            ..OverrideEnv::default()
        })
        .unwrap();
        let look = overrides.look.unwrap();
        assert!(close(look.yaw, FRAC_PI_2) && close(look.pitch, -FRAC_PI_4));
    }

    #[test]
    fn spawn_angle_turns_the_game_heading_into_engine_yaw_and_down_pitch_into_negative() {
        let look = look_from_spawn_angle("30, 0, 90").unwrap();
        assert!(close(look.yaw, 0.0), "heading 90 (east) is engine yaw 0");
        assert!(close(look.pitch, -30f32.to_radians()), "x 30 looks down");
        let north = look_from_spawn_angle("-10,0,0").unwrap();
        assert!(close(north.yaw, -FRAC_PI_2));
        assert!(close(north.pitch, 10f32.to_radians()));
        let south = look_from_spawn_angle("0,0,180").unwrap();
        assert!(close(south.yaw, FRAC_PI_2));
        assert!(close(look_from_spawn_angle("0,0,270").unwrap().yaw, PI));
    }

    #[test]
    fn spawn_angle_matches_the_door_heading_convention() {
        let look = look_from_spawn_angle("0,0,123.5").unwrap();
        assert!(close(look.yaw, yaw_from_rotation_z(123.5f32.to_radians())));
    }

    #[test]
    fn malformed_spawn_angles_are_rejected() {
        for raw in ["", "1,2", "1,2,3,4", "a,0,0", "inf,0,0", "0,0,NaN"] {
            assert_eq!(
                look_from_spawn_angle(raw),
                Err(SceneOverrideError::InvalidSpawnAngle(raw.to_owned())),
                "{raw:?}"
            );
        }
        assert_eq!(
            look_from_spawn_angle("0,5,0"),
            Err(SceneOverrideError::SpawnAngleWithRoll("0,5,0".to_owned()))
        );
    }

    #[test]
    fn look_and_angle_together_are_ambiguous() {
        let env = OverrideEnv {
            spawn_look: Some("0,0".to_owned()),
            spawn_angle: Some("0,0,0".to_owned()),
            ..OverrideEnv::default()
        };
        assert_eq!(parse(env), Err(SceneOverrideError::LookAndAngleBothSet));
    }

    #[test]
    fn an_invalid_spawn_look_is_an_error() {
        let env = OverrideEnv {
            spawn_look: Some("10".to_owned()),
            ..OverrideEnv::default()
        };
        assert_eq!(
            parse(env),
            Err(SceneOverrideError::InvalidSpawnLook("10".to_owned()))
        );
    }

    #[test]
    fn fov_must_lie_strictly_between_0_and_180() {
        assert_eq!(parse_fov(" 70 "), Ok(70.0));
        for raw in ["0", "180", "-5", "wide", "NaN", "inf"] {
            assert_eq!(
                parse_fov(raw),
                Err(SceneOverrideError::InvalidFov(raw.to_owned())),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn time_takes_hours_or_hours_and_minutes() {
        assert_eq!(parse_hour("12"), Ok(12.0));
        assert_eq!(parse_hour("6.5"), Ok(6.5));
        assert_eq!(parse_hour("12:30"), Ok(12.5));
        assert_eq!(parse_hour("0:00"), Ok(0.0));
        assert!(close(parse_hour("23:59").unwrap(), 23.0 + 59.0 / 60.0));
    }

    #[test]
    fn times_outside_the_day_are_rejected() {
        for raw in [
            "24", "-1", "24:00", "12:60", "noon", "12:", ":30", "NaN", "",
        ] {
            assert_eq!(
                parse_hour(raw),
                Err(SceneOverrideError::InvalidTime(raw.to_owned())),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn an_empty_weather_is_rejected() {
        assert_eq!(parse_weather("  "), Err(SceneOverrideError::EmptyWeather));
    }

    const WEATHERS: [(&str, Option<&str>); 3] = [
        ("01C3473:Fallout4.esm", Some("CommonwealthClear")),
        ("1C3474:Fallout4.esm", None),
        ("1C3473:DLCCoast.esm", Some("CoastClear")),
    ];

    fn find(selector: &str) -> Result<usize, SceneOverrideError> {
        parse_weather(selector).unwrap().find(WEATHERS)
    }

    #[test]
    fn a_weather_is_found_by_form_key_ignoring_case_and_leading_zeros() {
        assert_eq!(find("1C3473:Fallout4.esm"), Ok(0));
        assert_eq!(find("0x1c3473:FALLOUT4.ESM"), Ok(0));
        assert_eq!(find("1C3474:Fallout4.esm"), Ok(1));
        assert_eq!(find("1C3473:DLCCoast.esm"), Ok(2));
    }

    #[test]
    fn a_weather_is_found_by_editor_id_ignoring_case() {
        assert_eq!(find("commonwealthclear"), Ok(0));
        assert_eq!(find(" CoastClear "), Ok(2));
    }

    #[test]
    fn a_weather_of_no_entry_is_unknown() {
        for selector in ["1C3475:Fallout4.esm", "Nowhere", "zz:Fallout4.esm"] {
            assert_eq!(
                find(selector),
                Err(SceneOverrideError::UnknownWeather(selector.to_owned())),
                "{selector:?}"
            );
        }
    }
}
