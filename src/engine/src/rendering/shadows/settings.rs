use crate::assets::ini::{GameIni, IniError, IniKey, capped_at_engine_limit};
use crate::rendering::lighting::frame_uniform::MAX_CASCADES;
use crate::rendering::world_streamer::SCALE;
use crate::utils::env_flag;

const RESOLUTION: IniKey = IniKey::new("Display", "iShadowMapResolution");
const DISTANCE: IniKey = IniKey::new("Display", "fDirShadowDistance");
const SPLITS: IniKey = IniKey::new("Display", "iDirShadowSplits");
const SPLIT_BLEND: IniKey = IniKey::new("Display", "fBlendSplitDirShadow");
const FILTER_QUALITY: IniKey = IniKey::new("Display", "uiOrthoShadowFilter");
const BIAS_SCALE: IniKey = IniKey::new("Display", "fShadowBiasScale");

/// Smallest caster, in shadow texels, each cascade still draws. Far cascades
/// cull more aggressively: a caster below this size is a sub-texel speck there.
const DEFAULT_MIN_CASTER_TEXELS: [f32; MAX_CASCADES] = [1.5, 3.0, 8.0, 8.0];
const MIN_TEXELS_ENV: &str = "OPENFO4_SHADOW_MIN_TEXELS";
const ALPHA_LAST_ENV: &str = "OPENFO4_SHADOW_ALPHA_LAST";

const MIN_RESOLUTION: u32 = 512;
/// The largest shadow map the engine allocates; a larger `iShadowMapResolution` is clamped to it.
const MAX_RESOLUTION: u32 = 8192;

/// PCF footprint selected by a shadow filter quality (`uiOrthoShadowFilter`, `uiShadowFilter`).
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ShadowFilter {
    SingleTap,
    Grid2x2,
    Grid3x3,
}

impl ShadowFilter {
    /// The highest quality with a footprint of its own; a higher setting is clamped to it.
    pub const MAX_QUALITY: u32 = 3;

    pub fn from_quality(quality: u32) -> Self {
        match quality {
            0 | 1 => Self::SingleTap,
            2 => Self::Grid2x2,
            _ => Self::Grid3x3,
        }
    }

    pub fn from_ini(ini: &GameIni, key: IniKey) -> Result<Self, IniError> {
        ini.require_capped(key, Self::MAX_QUALITY)
            .map(Self::from_quality)
    }

    pub fn taps_per_axis(self) -> u32 {
        match self {
            Self::SingleTap => 1,
            Self::Grid2x2 => 2,
            Self::Grid3x3 => 3,
        }
    }
}

/// Sun-shadow quality from the game's INI cascade; distances in engine units.
#[derive(Debug, Copy, Clone)]
pub struct ShadowConfig {
    pub resolution: u32,
    pub distance: f32,
    pub cascade_count: usize,
    pub split_blend: f32,
    pub filter: ShadowFilter,
    pub bias_scale: f32,
    pub cascade_min_texels: [f32; MAX_CASCADES],
    /// The last active cascade skips alpha-tested casters.
    pub skip_alpha_in_last_cascade: bool,
}

/// `raw` is a comma separated list overriding the leading cascades' minimum
/// caster sizes in texels; the remaining cascades keep their defaults. An empty
/// entry, a non-finite or negative value, or more values than cascades makes
/// the whole list invalid.
fn parse_min_texels(
    raw: &str,
    defaults: [f32; MAX_CASCADES],
) -> Result<[f32; MAX_CASCADES], String> {
    let mut texels = defaults;
    for (slot, part) in raw.split(',').enumerate() {
        let value: f32 = part
            .trim()
            .parse()
            .ok()
            .filter(|v: &f32| v.is_finite() && *v >= 0.0)
            .ok_or_else(|| format!("{part:?} is not a non-negative number"))?;
        *texels
            .get_mut(slot)
            .ok_or_else(|| format!("more than {MAX_CASCADES} values, one per cascade at most"))? =
            value;
    }
    Ok(texels)
}

fn min_texels_from_env() -> [f32; MAX_CASCADES] {
    let Ok(raw) = std::env::var(MIN_TEXELS_ENV) else {
        return DEFAULT_MIN_CASTER_TEXELS;
    };
    parse_min_texels(&raw, DEFAULT_MIN_CASTER_TEXELS).unwrap_or_else(|reason| {
        tracing::warn!("{MIN_TEXELS_ENV}={raw:?} ignored, using the defaults: {reason}");
        DEFAULT_MIN_CASTER_TEXELS
    })
}

impl ShadowConfig {
    pub fn from_ini(ini: &GameIni) -> Result<Self, IniError> {
        let resolution = capped_at_engine_limit(
            RESOLUTION,
            ini.require_valid::<u32>(
                RESOLUTION,
                &format!("a power of two, {MIN_RESOLUTION} or more"),
                |r| *r >= MIN_RESOLUTION && r.is_power_of_two(),
            )?,
            MAX_RESOLUTION,
        );
        let distance = ini.require_positive(DISTANCE)?;
        let cascade_count = capped_at_engine_limit(
            SPLITS,
            ini.require_valid::<usize>(SPLITS, "1 or more", |splits| *splits >= 1)?,
            MAX_CASCADES,
        );
        let split_blend = ini.require_non_negative(SPLIT_BLEND)?;
        let filter = ShadowFilter::from_ini(ini, FILTER_QUALITY)?;
        let bias_scale = ini.require_non_negative(BIAS_SCALE)?;

        let config = Self {
            resolution,
            distance: distance * SCALE,
            cascade_count,
            split_blend: split_blend * SCALE,
            filter,
            bias_scale,
            cascade_min_texels: min_texels_from_env(),
            skip_alpha_in_last_cascade: env_flag(ALPHA_LAST_ENV, true),
        };
        tracing::info!(
            "sun shadows: {} cascades of {}² to {distance} BU, split blend {split_blend} BU, {:?} filter, bias scale {bias_scale}",
            config.cascade_count,
            config.resolution,
            config.filter,
        );
        tracing::info!(
            "sun shadow caster culling: min texels {:?}, alpha-tested casters skipped in the last cascade: {}",
            &config.cascade_min_texels[..config.cascade_count],
            config.skip_alpha_in_last_cascade,
        );
        Ok(config)
    }
}

/// Live, non-persisted debug-panel controls. Biases are in shadow-map texels
/// of the receiving cascade.
#[derive(Debug, Copy, Clone)]
pub struct ShadowSettings {
    pub enabled: bool,
    pub constant_bias: f32,
    pub slope_bias: f32,
    pub normal_offset: f32,
    pub freeze_cascades: bool,
}

impl Default for ShadowSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            constant_bias: 1.0,
            slope_bias: 1.5,
            normal_offset: 1.5,
            freeze_cascades: false,
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact sentinels and copied values"
)]
mod tests {
    use super::*;
    use crate::rendering::test_support::ini_with;

    #[test]
    fn filter_quality_selects_the_pcf_footprint() {
        // (quality, expected taps per axis): 0 and 1 are single tap, 2 is 2x2, 3 and up 3x3.
        for (quality, taps) in [(0, 1), (1, 1), (2, 2), (3, 3), (9, 3)] {
            assert_eq!(
                ShadowFilter::from_quality(quality).taps_per_axis(),
                taps,
                "{quality}"
            );
        }
    }

    #[test]
    fn min_texels_override_the_leading_cascades_and_keep_the_rest() {
        let defaults = [1.5, 3.0, 8.0, 8.0];
        assert_eq!(
            parse_min_texels("2, 4.5", defaults),
            Ok([2.0, 4.5, 8.0, 8.0])
        );
        assert_eq!(
            parse_min_texels("0,0,0,0", defaults),
            Ok([0.0, 0.0, 0.0, 0.0])
        );
    }

    #[test]
    fn min_texels_reject_bad_entries_with_a_reason() {
        let defaults = [1.5, 3.0, 8.0, 8.0];
        for raw in ["", "1,,2", "abc", "-1", "NaN", "inf", "1,2,3,4,5"] {
            assert!(parse_min_texels(raw, defaults).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn too_many_min_texels_values_are_reported_as_such() {
        let error = parse_min_texels("1,2,3,4,5", [0.0; MAX_CASCADES]).unwrap_err();
        assert!(error.contains("more than"), "{error}");
    }

    #[test]
    fn ini_values_become_the_config_and_distances_engine_units() {
        let config = ShadowConfig::from_ini(&ini_with(
            "iShadowMapResolution=1024\nfDirShadowDistance=10000\niDirShadowSplits=4\n\
             fBlendSplitDirShadow=20\nuiOrthoShadowFilter=1\nfShadowBiasScale=0.5\n",
        ))
        .unwrap();
        assert_eq!(config.resolution, 1024);
        assert_eq!(config.cascade_count, 4);
        assert_eq!(config.filter, ShadowFilter::SingleTap);
        assert!((config.distance - 1000.0).abs() < 1e-3);
        assert!((config.split_blend - 2.0).abs() < 1e-4);
        assert_eq!(config.bias_scale, 0.5);
    }

    #[test]
    fn every_setting_comes_from_the_ini() {
        let config = ShadowConfig::from_ini(&ini_with("")).unwrap();
        assert_eq!(config.resolution, 2048);
        assert_eq!(config.cascade_count, 2);
        assert_eq!(config.filter, ShadowFilter::Grid2x2);
        assert!((config.distance - 1200.0).abs() < 1e-3);
        assert!((config.split_blend - 3.0).abs() < 1e-4);
        assert_eq!(config.bias_scale, 0.75);
    }

    #[test]
    fn invalid_ini_values_are_errors_naming_the_key() {
        // Not a power of two, too small, zero or negative, unparsable, or too large for the type.
        for (overrides, key) in [
            ("iShadowMapResolution=3000\n", "iShadowMapResolution"),
            ("iShadowMapResolution=256\n", "iShadowMapResolution"),
            ("iShadowMapResolution=0\n", "iShadowMapResolution"),
            ("iShadowMapResolution=-2048\n", "iShadowMapResolution"),
            ("iShadowMapResolution=10000\n", "iShadowMapResolution"),
            ("iShadowMapResolution=4294967296\n", "iShadowMapResolution"),
            ("iDirShadowSplits=0\n", "iDirShadowSplits"),
            ("iDirShadowSplits=-1\n", "iDirShadowSplits"),
            ("fDirShadowDistance=0\n", "fDirShadowDistance"),
            ("fDirShadowDistance=inf\n", "fDirShadowDistance"),
            ("fBlendSplitDirShadow=-5\n", "fBlendSplitDirShadow"),
            ("uiOrthoShadowFilter=-1\n", "uiOrthoShadowFilter"),
            ("uiOrthoShadowFilter=lots\n", "uiOrthoShadowFilter"),
            ("fShadowBiasScale=-0.1\n", "fShadowBiasScale"),
        ] {
            let error = ShadowConfig::from_ini(&ini_with(overrides)).unwrap_err();
            assert!(error.to_string().contains(key), "{overrides}: {error}");
        }
    }

    #[test]
    fn a_negative_whole_number_setting_is_not_reported_with_a_rust_type_name() {
        let error = ShadowConfig::from_ini(&ini_with("iDirShadowSplits=-1\n")).unwrap_err();
        let message = error.to_string();
        assert!(message.contains("a whole number, 0 or more"), "{message}");
        assert!(!message.contains("usize"), "{message}");
    }

    #[test]
    fn resolution_bounds_are_inclusive_and_a_larger_power_of_two_is_clamped() {
        // 1024, 2048 and 4096 are what the launcher's quality presets write.
        for (value, expected) in [
            (512, 512),
            (1024, 1024),
            (2048, 2048),
            (4096, 4096),
            (8192, 8192),
            (16384, 8192),
            (65536, 8192),
        ] {
            let ini = ini_with(&format!("iShadowMapResolution={value}\n"));
            assert_eq!(ShadowConfig::from_ini(&ini).unwrap().resolution, expected);
        }
    }

    #[test]
    fn cascade_count_bounds_are_inclusive_and_more_splits_than_cascades_are_clamped() {
        for (value, expected) in [
            (1, 1),
            (2, 2),
            (3, 3),
            (MAX_CASCADES, MAX_CASCADES),
            (MAX_CASCADES + 1, MAX_CASCADES),
            (99, MAX_CASCADES),
        ] {
            let ini = ini_with(&format!("iDirShadowSplits={value}\n"));
            assert_eq!(
                ShadowConfig::from_ini(&ini).unwrap().cascade_count,
                expected
            );
        }
    }

    #[test]
    fn the_filter_quality_accepts_zero_to_the_maximum_and_clamps_above() {
        for (quality, expected) in [
            (0, ShadowFilter::SingleTap),
            (1, ShadowFilter::SingleTap),
            (2, ShadowFilter::Grid2x2),
            (ShadowFilter::MAX_QUALITY, ShadowFilter::Grid3x3),
            (ShadowFilter::MAX_QUALITY + 1, ShadowFilter::Grid3x3),
        ] {
            let ini = ini_with(&format!("uiOrthoShadowFilter={quality}\n"));
            assert_eq!(
                ShadowConfig::from_ini(&ini).unwrap().filter,
                expected,
                "{quality}"
            );
        }
    }
}
