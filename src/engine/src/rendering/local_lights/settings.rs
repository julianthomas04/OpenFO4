use crate::assets::ini::{GameIni, IniError, IniKey};
use crate::rendering::world_streamer::SCALE;

const FADE_START: IniKey = IniKey::new("Display", "fLightLODMaxStartFade");
/// Fixed guess: no INI key is known for the fade range; calibrate.
const FADE_RANGE_BETH: f32 = 500.0;

/// Distance fade of placed lights by their distance to the camera, engine
/// units: full strength up to `start`, a smoothstep down to zero at
/// `start + range`. `NeverFades` refs ignore it.
#[derive(Debug, Copy, Clone)]
pub struct LightFade {
    pub start: f32,
    pub range: f32,
}

impl LightFade {
    pub fn end(&self) -> f32 {
        self.start + self.range
    }
}

impl LightFade {
    pub fn from_ini(ini: &GameIni) -> Result<Self, IniError> {
        let start = ini.require_non_negative(FADE_START)?;
        tracing::info!("placed-light fade: full to {start} BU, gone {FADE_RANGE_BETH} BU further");
        Ok(Self {
            start: start * SCALE,
            range: FADE_RANGE_BETH * SCALE,
        })
    }
}

/// Panel controls of the placed-light shading; not persisted.
#[derive(Debug, Copy, Clone)]
pub struct LocalLightSettings {
    pub enabled: bool,
    /// Multiplies every placed light's colour, for calibration.
    pub intensity: f32,
    pub show_spheres: bool,
    pub fade: LightFade,
}

impl LocalLightSettings {
    pub fn new(fade: LightFade) -> Self {
        Self {
            enabled: true,
            intensity: 1.0,
            show_spheres: false,
            fade,
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals parsed or multiplied by zero"
)]
mod tests {
    use super::*;
    use crate::rendering::test_support::ini_with;

    #[test]
    fn the_fade_start_comes_from_the_ini_in_engine_units() {
        let fade = LightFade::from_ini(&ini_with("")).unwrap();
        assert!((fade.start - 400.0).abs() < 1e-3);
        assert!((fade.range - FADE_RANGE_BETH * SCALE).abs() < 1e-3);
        assert!((fade.end() - 450.0).abs() < 1e-3);
    }

    #[test]
    fn a_zero_start_is_accepted() {
        let fade = LightFade::from_ini(&ini_with("fLightLODMaxStartFade=0\n")).unwrap();
        assert_eq!(fade.start, 0.0);
    }

    #[test]
    fn a_negative_or_unparsable_start_is_an_error_naming_the_key() {
        for text in ["fLightLODMaxStartFade=-1\n", "fLightLODMaxStartFade=far\n"] {
            let error = LightFade::from_ini(&ini_with(text)).unwrap_err();
            assert!(
                error.to_string().contains("fLightLODMaxStartFade"),
                "{text}"
            );
        }
    }
}
