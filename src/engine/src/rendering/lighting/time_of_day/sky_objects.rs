use std::sync::Arc;

use glam::Vec3;

use super::climate_sky::ClimateSunTextures;
use super::moment::MoonSky;
use super::moon::{MoonKind, MoonPhase};
use super::stars::StarRotation;

/// The moon the sky draws: plain data, no GPU types.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoonObject {
    pub kind: MoonKind,
    pub phase: MoonPhase,
    /// Unit vector from the viewer towards the moon in engine space.
    pub direction: Vec3,
    /// 0 to 1: how far the moon has faded in above the horizon.
    pub visibility: f32,
}

impl From<MoonSky> for MoonObject {
    fn from(moon: MoonSky) -> Self {
        Self {
            kind: moon.kind,
            phase: moon.phase,
            direction: moon.position.direction(),
            visibility: moon.visibility,
        }
    }
}

/// What the sky renderer needs each frame, from the clock and the weather.
#[derive(Debug, Clone, PartialEq)]
pub struct SkyObjects {
    /// Unit vector towards the sun disc in engine space. Always the sun's own place, never the
    /// moon's; it follows the manual sun while the lighting override is on.
    pub sun_disc_direction: Vec3,
    /// `None` where the climate shows no moon.
    pub moon: Option<MoonObject>,
    pub star_rotation: StarRotation,
    /// Opacity of the star layer, 0 to 1: the sampled weather's Stars colour.
    pub star_alpha: f32,
    /// The climate's sky model (`Sky\Stars.nif` in vanilla), when the database carries one.
    pub sky_model_path: Option<Arc<str>>,
    /// The climate's sun and sun glare textures.
    pub sun_textures: ClimateSunTextures,
}

#[cfg(test)]
mod tests {
    use super::super::sky::SkyPosition;
    use super::*;

    #[test]
    fn a_moon_object_points_where_the_moon_stands() {
        let position = SkyPosition {
            azimuth_deg: 90.0,
            elevation_deg: 0.0,
        };
        let moon = MoonObject::from(MoonSky {
            kind: MoonKind::Secunda,
            phase: MoonPhase::HalfWax,
            position,
            visibility: 0.25,
        });
        assert_eq!(moon.kind, MoonKind::Secunda);
        assert_eq!(moon.phase, MoonPhase::HalfWax);
        assert_eq!(moon.visibility, 0.25);
        assert!(moon.direction.abs_diff_eq(Vec3::X, 1e-6));
    }
}
