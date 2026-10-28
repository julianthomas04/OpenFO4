use super::blend::{SlotBlend, slot_blend};
use super::climate_sky::ClimateSky;
use super::climate_times::ClimateTimes;
use super::day_time::DayTime;
use super::light::LightSource;
use super::moon::{MoonError, MoonKind, MoonPhase, moon_phase, moon_visibility};
use super::sky::{SkyGameSettings, SkyPosition};
use super::stars::{StarRotation, star_rotation};
use super::sun_disc::sun_position;
use super::sun_light::light_source;
use super::unmeasured::moon_position;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ClimateMoonError {
    #[error("the climate shows a moon but stores no moon phase length")]
    PhaseLengthMissing,
    #[error(transparent)]
    Moon(#[from] MoonError),
}

/// The one moon a climate shows and how long each of its phases lasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClimateMoon {
    kind: MoonKind,
    phase_length: u32,
}

impl ClimateMoon {
    pub fn new(kind: MoonKind, phase_length: u32) -> Result<Self, MoonError> {
        moon_phase(0, phase_length)?;
        Ok(Self { kind, phase_length })
    }

    /// `None` when the climate shows no moon, or when its database predates the moon export.
    /// Only Masser's path is modelled, so it represents a climate that shows both moons.
    pub fn from_climate(sky: &ClimateSky) -> Result<Option<Self>, ClimateMoonError> {
        let kind = match sky.moons {
            Some(moons) if moons.masser => MoonKind::Masser,
            Some(moons) if moons.secunda => MoonKind::Secunda,
            Some(_) | None => return Ok(None),
        };
        let phase_length = sky
            .moon_phase_length
            .ok_or(ClimateMoonError::PhaseLengthMissing)?;
        Ok(Some(Self::new(kind, phase_length)?))
    }

    pub fn kind(&self) -> MoonKind {
        self.kind
    }

    pub fn phase_length(&self) -> u32 {
        self.phase_length
    }
}

/// The moon as it stands in the sky at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoonSky {
    pub kind: MoonKind,
    pub phase: MoonPhase,
    pub position: SkyPosition,
    /// 0 while the moon is below the angle at which it starts to fade in, 1 once it has fully faded in.
    pub visibility: f32,
}

/// Everything the clock decides about the sky at one moment, independent of the weather.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkyMoment {
    pub blend: SlotBlend,
    pub sun: SkyPosition,
    pub moon: Option<MoonSky>,
    pub light: LightSource,
    pub star_rotation: StarRotation,
}

impl SkyMoment {
    pub fn at(
        time: DayTime,
        times: &ClimateTimes,
        moon: Option<&ClimateMoon>,
        settings: &SkyGameSettings,
    ) -> Self {
        let sun = sun_position(time, times, settings);
        let moon = moon
            .copied()
            .map(|moon| moon_sky(time, moon, times, settings));
        Self {
            blend: slot_blend(time, times),
            sun,
            moon,
            light: light_source(time, times),
            star_rotation: star_rotation(time, settings),
        }
    }
}

fn moon_sky(
    time: DayTime,
    moon: ClimateMoon,
    times: &ClimateTimes,
    settings: &SkyGameSettings,
) -> MoonSky {
    let (Ok(position), Ok(phase)) = (
        moon_position(time, moon.phase_length, times, settings),
        moon_phase(time.days_passed(), moon.phase_length),
    ) else {
        unreachable!("a ClimateMoon never has a zero phase length");
    };
    MoonSky {
        kind: moon.kind,
        phase,
        position,
        visibility: moon_visibility(position.elevation_deg, settings),
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact binary fractions or sentinels"
)]
mod tests {
    use super::super::climate_sky::Moons;
    use super::super::slot::TimeSlot;
    use super::super::test_support::{at, default_climate, vanilla_settings};
    use super::*;

    fn climate_with(moons: Option<Moons>, phase_length: Option<u32>) -> ClimateSky {
        ClimateSky {
            moon_phase_length: phase_length,
            moons,
            model_path: None,
        }
    }

    fn masser() -> ClimateMoon {
        ClimateMoon::new(MoonKind::Masser, 4).unwrap()
    }

    fn moment(days: u32, hour: f32, moon: Option<&ClimateMoon>) -> SkyMoment {
        SkyMoment::at(
            at(days, hour),
            &default_climate(),
            moon,
            &vanilla_settings(),
        )
    }

    #[test]
    fn a_climate_flagging_masser_gets_masser_with_its_phase_length() {
        let sky = climate_with(
            Some(Moons {
                masser: true,
                secunda: false,
            }),
            Some(4),
        );
        let moon = ClimateMoon::from_climate(&sky).unwrap().unwrap();
        assert_eq!((moon.kind(), moon.phase_length()), (MoonKind::Masser, 4));
    }

    #[test]
    fn a_climate_flagging_only_secunda_gets_secunda() {
        let sky = climate_with(
            Some(Moons {
                masser: false,
                secunda: true,
            }),
            Some(3),
        );
        let moon = ClimateMoon::from_climate(&sky).unwrap().unwrap();
        assert_eq!(moon.kind(), MoonKind::Secunda);
    }

    #[test]
    fn a_climate_flagging_both_moons_is_represented_by_masser() {
        let sky = climate_with(
            Some(Moons {
                masser: true,
                secunda: true,
            }),
            Some(4),
        );
        let moon = ClimateMoon::from_climate(&sky).unwrap().unwrap();
        assert_eq!(moon.kind(), MoonKind::Masser);
    }

    #[test]
    fn a_climate_without_moons_or_from_an_older_database_has_none() {
        let none = Moons {
            masser: false,
            secunda: false,
        };
        for sky in [
            climate_with(Some(none), Some(4)),
            climate_with(Some(none), None),
            climate_with(None, None),
        ] {
            assert_eq!(ClimateMoon::from_climate(&sky), Ok(None), "{sky:?}");
        }
    }

    #[test]
    fn a_flagged_moon_without_a_phase_length_is_an_error() {
        let sky = climate_with(
            Some(Moons {
                masser: true,
                secunda: false,
            }),
            None,
        );
        assert_eq!(
            ClimateMoon::from_climate(&sky),
            Err(ClimateMoonError::PhaseLengthMissing)
        );
    }

    #[test]
    fn a_zero_phase_length_is_rejected() {
        assert_eq!(
            ClimateMoon::new(MoonKind::Masser, 0),
            Err(MoonError::ZeroPhaseLength)
        );
        let sky = climate_with(
            Some(Moons {
                masser: true,
                secunda: false,
            }),
            Some(0),
        );
        assert_eq!(
            ClimateMoon::from_climate(&sky),
            Err(ClimateMoonError::Moon(MoonError::ZeroPhaseLength))
        );
    }

    #[test]
    fn at_noon_the_blend_holds_day_and_the_light_is_not_the_disc() {
        let moment = moment(0, 12.0, Some(&masser()));
        assert_eq!(
            (moment.blend.from, moment.blend.to),
            (TimeSlot::Day, TimeSlot::Day)
        );
        assert!(moment.sun.elevation_deg > 50.0);
        let gap = moment
            .light
            .direction
            .angle_between(moment.sun.direction())
            .to_degrees();
        assert!((15.0..25.0).contains(&gap), "{gap}");
    }

    #[test]
    fn the_light_is_the_same_with_or_without_a_moon() {
        for hour in [0.0, 6.0, 12.0, 21.0] {
            assert_eq!(
                moment(0, hour, Some(&masser())).light,
                moment(0, hour, None).light,
                "{hour}"
            );
        }
    }

    #[test]
    fn at_midnight_the_disc_is_down_but_the_light_still_shines_from_the_south() {
        let moment = moment(0, 0.0, None);
        assert!(moment.sun.elevation_deg < 0.0);
        assert!(moment.light.strength > 0.95);
        assert!(moment.light.direction.y > 0.5 && moment.light.direction.z > 0.7);
    }

    #[test]
    fn the_moon_phase_follows_the_days_passed() {
        let moon = masser();
        let phase = |days| moment(days, 12.0, Some(&moon)).moon.unwrap().phase;
        assert_eq!(phase(0), MoonPhase::Full);
        assert_eq!(phase(3), MoonPhase::Full);
        assert_eq!(phase(4), MoonPhase::ThreeWan);
        assert_eq!(phase(32), MoonPhase::Full);
    }

    #[test]
    fn the_moment_carries_the_climate_moon_kind() {
        let secunda = ClimateMoon::new(MoonKind::Secunda, 4).unwrap();
        assert_eq!(
            moment(0, 0.0, Some(&secunda)).moon.unwrap().kind,
            MoonKind::Secunda
        );
    }

    #[test]
    fn the_star_dome_turns_with_the_clock() {
        let early = moment(0, 1.0, None).star_rotation;
        let late = moment(0, 13.0, None).star_rotation;
        assert_eq!(early.axis, late.axis);
        assert_ne!(early.angle_rad, late.angle_rad);
    }
}
