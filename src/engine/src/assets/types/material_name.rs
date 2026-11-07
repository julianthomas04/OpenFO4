use glam::Vec3;

use super::effect_params::{EffectBlend, EffectParams, EnvParams, Falloff, Palette};
use super::water_shader::WaterShaderToken;
use crate::assets::layout::texture_key;
use crate::rendering::gpu_driven::{LEGACY_SURFACE, SurfaceParams};
use crate::rendering::lighting::cell_lighting::srgb8_to_linear;

const DEFAULT_EMISSIVE_COLOR: [u8; 3] = [255, 255, 255];
const DEFAULT_EMISSIVE_MULT: f32 = 1.0;
const DEFAULT_EFFECT_COLOR: [u8; 3] = [255, 255, 255];

#[derive(Debug, thiserror::Error)]
pub enum MaterialNameError {
    #[error("alpha test token `test:{0}` is not an integer in 0..=255")]
    InvalidAlphaTest(String),
    #[error("token `{token}` carries `{value}`, which is not a finite number")]
    InvalidFloat { token: &'static str, value: String },
    #[error("token `{token}` carries `{value}`, which is not a six-digit hex colour")]
    InvalidColor { token: &'static str, value: String },
    #[error("token `eff:{0}` is not eight hex digits")]
    InvalidEffectKey(String),
    #[error("token `water:{0}` is not `WF:F1:F2:UOU,UOV:USU,USV`")]
    InvalidWater(String),
    #[error("token `bm:{0}` is not `add` or `mul`")]
    InvalidBlendMode(String),
    #[error("token `fo:{0}` is not `START,STOP,START_OPACITY,STOP_OPACITY,SCALES_COLOR`")]
    InvalidFalloff(String),
    #[error("token `pal:{0}` is not `FLAGS:PATH` with FLAGS a non-repeating subset of `c` and `a`")]
    InvalidPalette(String),
    #[error("token `{0}` requires an `env` token")]
    EnvTokenWithoutCube(&'static str),
    #[error("token `{0}` has an empty texture path")]
    EmptyTexturePath(&'static str),
    #[error("token `{0}` appears more than once")]
    DuplicateToken(&'static str),
    #[error("unknown token `{0}`")]
    UnknownToken(String),
}

/// Decoded GLB material name, the converter's
/// `diffuse|normal[|alpha][|fx][|refr][|eff:XXXXXXXX][|decal][|water:WF:F1:F2:UOU,UOV:USU,USV][|bm:M][|bc:RRGGBB][|bs:F][|fo:SA,EA,SO,EO,C][|li:F][|pal:FLAGS:PATH][|env:P][|envk:P][|envs:F][|spec:P][|glow:P][|sc:RRGGBB][|sm:F][|sg:F][|fp:F][|em:RRGGBB][|emm:F][|ts][|src:KEY]`
/// contract. Names predating the lighting tokens parse unchanged and yield
/// [`LEGACY_SURFACE`]. Texture paths are in cache-key form (`textures\...`).
#[derive(Debug, Clone)]
pub struct MaterialName {
    pub diffuse_path: Option<String>,
    pub normal_path: Option<String>,
    pub specular_path: Option<String>,
    /// Only kept while the material emits: a glow map without `em`/`emm` is
    /// ignored, like the BGSM `EmitEnabled` flag.
    pub glow_path: Option<String>,
    pub surface: SurfaceParams,
    pub alpha_cutoff: f32,
    pub alpha_mult: f32,
    pub is_blend: bool,
    pub is_effect: bool,
    /// `eff:XXXXXXXX`: hash of the shape's effect extras, so differing parameters never share a cache key.
    pub effect_key: Option<u32>,
    /// `refr`: a heat-haze shape drawn by the refraction stage, not the object buckets.
    pub is_refraction: bool,
    pub is_decal: bool,
    pub two_sided: bool,
    /// `BSWaterShaderProperty` fields of a water shape; such a shape also keeps
    /// `is_effect` but is drawn by the water stage, not as an effect.
    pub water: Option<WaterShaderToken>,
    pub source_material: Option<String>,
    /// Effect-shader parameters: `Some` for every effect shape (BGEM, a shape
    /// whose name carries effect tokens, or a textured `fx` shape), with
    /// [`EffectParams::LEGACY`] standing in when the name predates the tokens.
    pub effect: Option<EffectParams>,
    pub env: Option<EnvParams>,
}

impl MaterialName {
    /// A shape without any material name: no textures, matte, opaque.
    pub fn untextured() -> Self {
        Self {
            diffuse_path: None,
            normal_path: None,
            specular_path: None,
            glow_path: None,
            surface: LEGACY_SURFACE,
            alpha_cutoff: 0.0,
            alpha_mult: 1.0,
            is_blend: false,
            is_effect: false,
            effect_key: None,
            is_refraction: false,
            is_decal: false,
            two_sided: false,
            water: None,
            source_material: None,
            effect: None,
            env: None,
        }
    }

    /// The shape's material comes from a BGEM file.
    pub fn is_bgem(&self) -> bool {
        Self::is_bgem_source(self.source_material.as_deref())
    }

    pub fn is_bgem_source(source_material: Option<&str>) -> bool {
        source_material.is_some_and(|source| source.to_ascii_lowercase().ends_with(".bgem"))
    }

    /// An effect shape whose parameters are inline in the NIF (or a legacy
    /// textured effect name) rather than a BGEM file: hidden by default until
    /// effect animation and soft fading exist. BGEM effects (glass) always draw.
    /// Water shapes are never hidden.
    pub fn is_hidden_effect_shape(&self) -> bool {
        Self::hides_effect_shape_by_default(
            self.is_effect_shape(),
            self.is_water_shape(),
            self.source_material.as_deref(),
        )
    }

    /// Whether a part of a skinned or animated model stays behind the FX toggle. The effect
    /// stage cannot animate effect shapes with their model, so they draw through the
    /// object-shader effect path whatever the hidden-effect rule says; only a heat-haze shape
    /// no stage draws stays hidden.
    pub fn is_hidden_skinned_effect_shape(&self) -> bool {
        self.is_refraction
    }

    /// The single definition of the hidden-effect rule, for callers that hold
    /// the classification inputs rather than a `MaterialName`. Water shapes
    /// have their own renderer and are never hidden.
    pub fn hides_effect_shape_by_default(
        is_effect: bool,
        is_water: bool,
        source_material: Option<&str>,
    ) -> bool {
        is_effect && !is_water && !Self::is_bgem_source(source_material)
    }

    /// An untextured blend shape that carries no effect parameters: a CK
    /// helper volume, not geometry. Effect shapes (BGEM glass, inline effects
    /// with parameters) are never helpers.
    pub fn is_helper_shape(&self) -> bool {
        self.is_blend && self.diffuse_path.is_none() && self.effect.is_none()
    }

    /// Drawn through the effect shading path. Water shapes are effects by
    /// class but have their own renderer.
    pub fn is_effect_shape(&self) -> bool {
        self.effect.is_some()
    }

    pub fn is_water_shape(&self) -> bool {
        self.water.is_some()
    }

    pub fn parse(name: &str) -> Result<Self, MaterialNameError> {
        let segments: Vec<&str> = name.split('|').collect();
        let mut material = Self::untextured();
        material.diffuse_path = texture_path(segments[0]);

        let has_normal_segment = segments.get(1).is_some_and(|segment| !is_token(segment));
        let first_token = if has_normal_segment { 2 } else { 1 };
        let token_segments = segments.get(first_token..).unwrap_or_default();
        if has_normal_segment {
            material.normal_path = texture_path(segments[1]);
        } else if token_segments
            .iter()
            .all(|segment| *segment == "ts" || segment.starts_with("src:"))
        {
            material.normal_path = material.diffuse_path.as_deref().map(derive_normal_path);
        }

        let mut tokens = Tokens::default();
        let mut rest = token_segments.iter().peekable();
        while let Some(&segment) = rest.next() {
            match segment {
                "ts" => material.two_sided = true,
                "fx" => material.is_effect = true,
                "refr" => {
                    if std::mem::replace(&mut material.is_refraction, true) {
                        return Err(MaterialNameError::DuplicateToken("refr"));
                    }
                }
                "decal" => material.is_decal = true,
                "blend" => {
                    material.is_blend = true;
                    let multiplier = rest
                        .peek()
                        .and_then(|next| next.parse::<f32>().ok())
                        .filter(|value| value.is_finite());
                    if let Some(multiplier) = multiplier {
                        rest.next();
                        material.alpha_mult = multiplier;
                    }
                }
                _ => {
                    let (key, value) = segment
                        .split_once(':')
                        .ok_or_else(|| MaterialNameError::UnknownToken(segment.to_string()))?;
                    tokens.read(key, value, segment)?;
                }
            }
        }

        material.water = tokens.water;
        material.effect_key = tokens.effect_key;
        material.alpha_cutoff = tokens.alpha_test.map_or(0.0, |n| f32::from(n) / 255.0);
        material.surface = tokens.surface();
        material.source_material.clone_from(&tokens.source);
        material.effect = tokens.effect(
            material.is_bgem()
                || (material.is_effect
                    && material.diffuse_path.is_some()
                    && material.water.is_none()),
        );
        material.env = tokens.env()?;
        material.specular_path = tokens.specular.map(|p| texture_key(&p));
        material.glow_path = tokens
            .glow
            .filter(|_| material.surface.emissive != Vec3::ZERO)
            .map(|p| texture_key(&p));
        Ok(material)
    }
}

#[derive(Default)]
struct Tokens {
    alpha_test: Option<u8>,
    water: Option<WaterShaderToken>,
    effect_key: Option<u32>,
    source: Option<String>,
    specular: Option<String>,
    glow: Option<String>,
    specular_color: Option<[u8; 3]>,
    specular_mult: Option<f32>,
    smoothness: Option<f32>,
    fresnel_power: Option<f32>,
    emissive_color: Option<[u8; 3]>,
    emissive_mult: Option<f32>,
    blend_mode: Option<EffectBlend>,
    base_color: Option<[u8; 3]>,
    base_scale: Option<f32>,
    falloff: Option<Falloff>,
    lighting_influence: Option<f32>,
    palette: Option<Palette>,
    env_cube: Option<String>,
    env_mask: Option<String>,
    env_scale: Option<f32>,
}

impl Tokens {
    fn read(&mut self, key: &str, value: &str, segment: &str) -> Result<(), MaterialNameError> {
        match key {
            "test" => {
                let cutoff = value
                    .parse::<u8>()
                    .map_err(|_| MaterialNameError::InvalidAlphaTest(value.to_string()))?;
                once(&mut self.alpha_test, "test", cutoff)
            }
            "water" => {
                let water = WaterShaderToken::parse(value)
                    .ok_or_else(|| MaterialNameError::InvalidWater(value.to_string()))?;
                once(&mut self.water, "water", water)
            }
            "eff" => once(&mut self.effect_key, "eff", effect_key(value)?),
            "src" => once(&mut self.source, "src", value.to_string()),
            "spec" => once(&mut self.specular, "spec", non_empty_path("spec", value)?),
            "glow" => once(&mut self.glow, "glow", non_empty_path("glow", value)?),
            "sc" => once(&mut self.specular_color, "sc", color("sc", value)?),
            "sm" => once(&mut self.specular_mult, "sm", float("sm", value)?),
            "sg" => once(&mut self.smoothness, "sg", float("sg", value)?),
            "fp" => once(&mut self.fresnel_power, "fp", float("fp", value)?),
            "em" => once(&mut self.emissive_color, "em", color("em", value)?),
            "emm" => once(&mut self.emissive_mult, "emm", float("emm", value)?),
            "bm" => once(&mut self.blend_mode, "bm", blend_mode(value)?),
            "bc" => once(&mut self.base_color, "bc", color("bc", value)?),
            "bs" => once(&mut self.base_scale, "bs", float("bs", value)?),
            "fo" => once(&mut self.falloff, "fo", falloff(value)?),
            "li" => once(
                &mut self.lighting_influence,
                "li",
                float("li", value)?.clamp(0.0, 1.0),
            ),
            "pal" => once(&mut self.palette, "pal", palette(value)?),
            "env" => once(&mut self.env_cube, "env", non_empty_path("env", value)?),
            "envk" => once(&mut self.env_mask, "envk", non_empty_path("envk", value)?),
            "envs" => once(&mut self.env_scale, "envs", float("envs", value)?),
            _ => Err(MaterialNameError::UnknownToken(segment.to_string())),
        }
    }

    fn has_effect_tokens(&self) -> bool {
        self.blend_mode.is_some()
            || self.base_color.is_some()
            || self.base_scale.is_some()
            || self.falloff.is_some()
            || self.lighting_influence.is_some()
            || self.palette.is_some()
    }

    /// Effect parameters from the effect tokens; without any, `legacy_effect`
    /// shapes get [`EffectParams::LEGACY`] and everything else is no effect.
    /// An absent `li` means an unlit effect, as the encoder writes it.
    fn effect(&self, legacy_effect: bool) -> Option<EffectParams> {
        if !self.has_effect_tokens() {
            return legacy_effect.then_some(EffectParams::LEGACY);
        }
        Some(EffectParams {
            blend: self.blend_mode.unwrap_or(EffectBlend::Alpha),
            base_color_srgb: self.base_color.unwrap_or(DEFAULT_EFFECT_COLOR),
            base_color_scale: self.base_scale.unwrap_or(1.0),
            falloff: self.falloff,
            lighting_influence: self.lighting_influence,
            palette: self.palette.clone(),
        })
    }

    fn env(&self) -> Result<Option<EnvParams>, MaterialNameError> {
        let Some(cube) = &self.env_cube else {
            if self.env_mask.is_some() {
                return Err(MaterialNameError::EnvTokenWithoutCube("envk"));
            }
            if self.env_scale.is_some() {
                return Err(MaterialNameError::EnvTokenWithoutCube("envs"));
            }
            return Ok(None);
        };
        Ok(Some(EnvParams {
            cube_path: cube.clone(),
            mask_path: self.env_mask.as_deref().map(texture_key),
            mask_scale: self.env_scale.unwrap_or(1.0),
        }))
    }

    /// Specular is enabled when any of `sm`/`sg`/`fp` is present (the encoder
    /// writes all three or none); emission when `em` or `emm` is present.
    fn surface(&self) -> SurfaceParams {
        let specular_enabled = self.specular_mult.is_some()
            || self.smoothness.is_some()
            || self.fresnel_power.is_some();
        let emits = self.emissive_color.is_some() || self.emissive_mult.is_some();
        let emissive = if emits {
            srgb8_to_linear(self.emissive_color.unwrap_or(DEFAULT_EMISSIVE_COLOR))
                * self.emissive_mult.unwrap_or(DEFAULT_EMISSIVE_MULT)
        } else {
            Vec3::ZERO
        };
        SurfaceParams {
            specular_enabled,
            specular_color: self.specular_color.unwrap_or(LEGACY_SURFACE.specular_color),
            specular_mult: self.specular_mult.unwrap_or(LEGACY_SURFACE.specular_mult),
            smoothness: self.smoothness.unwrap_or(LEGACY_SURFACE.smoothness),
            fresnel_power: self.fresnel_power.unwrap_or(LEGACY_SURFACE.fresnel_power),
            emissive,
        }
    }
}

fn once<T>(slot: &mut Option<T>, token: &'static str, value: T) -> Result<(), MaterialNameError> {
    if slot.replace(value).is_some() {
        return Err(MaterialNameError::DuplicateToken(token));
    }
    Ok(())
}

fn is_token(segment: &str) -> bool {
    matches!(segment, "ts" | "fx" | "refr" | "decal" | "blend")
        || segment.split_once(':').is_some_and(|(key, _)| {
            matches!(
                key,
                "test"
                    | "water"
                    | "eff"
                    | "src"
                    | "spec"
                    | "glow"
                    | "sc"
                    | "sm"
                    | "sg"
                    | "fp"
                    | "em"
                    | "emm"
                    | "bm"
                    | "bc"
                    | "bs"
                    | "fo"
                    | "li"
                    | "pal"
                    | "env"
                    | "envk"
                    | "envs"
            )
        })
}

fn blend_mode(value: &str) -> Result<EffectBlend, MaterialNameError> {
    match value {
        "add" => Ok(EffectBlend::Additive),
        "mul" => Ok(EffectBlend::Multiply),
        _ => Err(MaterialNameError::InvalidBlendMode(value.to_string())),
    }
}

fn falloff(value: &str) -> Result<Falloff, MaterialNameError> {
    let invalid = || MaterialNameError::InvalidFalloff(value.to_string());
    let fields: Vec<&str> = value.split(',').collect();
    let [start, stop, start_opacity, stop_opacity, scales_color] = fields[..] else {
        return Err(invalid());
    };
    let number = |text: &str| {
        text.parse::<f32>()
            .ok()
            .filter(|number| number.is_finite())
            .ok_or_else(invalid)
    };
    Ok(Falloff {
        start_facing: number(start)?,
        stop_facing: number(stop)?,
        start_opacity: number(start_opacity)?,
        stop_opacity: number(stop_opacity)?,
        scales_color: match scales_color {
            "0" => false,
            "1" => true,
            _ => return Err(invalid()),
        },
    })
}

fn palette(value: &str) -> Result<Palette, MaterialNameError> {
    let invalid = || MaterialNameError::InvalidPalette(value.to_string());
    let (flags, path) = value.split_once(':').ok_or_else(invalid)?;
    let recolours_color = flags.contains('c');
    let drives_alpha = flags.contains('a');
    let canonical_len = usize::from(recolours_color) + usize::from(drives_alpha);
    if path.is_empty() || canonical_len == 0 || flags.len() != canonical_len {
        return Err(invalid());
    }
    Ok(Palette {
        path: texture_key(path),
        recolours_color,
        drives_alpha,
    })
}

fn texture_path(segment: &str) -> Option<String> {
    (!segment.is_empty()).then(|| texture_key(segment))
}

fn non_empty_path(token: &'static str, value: &str) -> Result<String, MaterialNameError> {
    if value.is_empty() {
        return Err(MaterialNameError::EmptyTexturePath(token));
    }
    Ok(value.to_string())
}

fn float(token: &'static str, value: &str) -> Result<f32, MaterialNameError> {
    value
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| MaterialNameError::InvalidFloat {
            token,
            value: value.to_string(),
        })
}

fn effect_key(value: &str) -> Result<u32, MaterialNameError> {
    if value.len() != 8 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(MaterialNameError::InvalidEffectKey(value.to_string()));
    }
    u32::from_str_radix(value, 16)
        .map_err(|_| MaterialNameError::InvalidEffectKey(value.to_string()))
}

fn color(token: &'static str, value: &str) -> Result<[u8; 3], MaterialNameError> {
    let invalid = || MaterialNameError::InvalidColor {
        token,
        value: value.to_string(),
    };
    if value.len() != 6 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    let packed = u32::from_str_radix(value, 16).map_err(|_| invalid())?;
    let [_, r, g, b] = packed.to_be_bytes();
    Ok([r, g, b])
}

/// FO4 convention: `foo_d.dds` / `foo.dds` diffuse -> `foo_n.dds` normal.
/// A plain `.dds` -> `_n.dds` replace would produce `foo_d_n.dds`.
/// Only used for legacy single-segment material names; current GLBs carry
/// the real normal-map path as the second `|`-separated segment.
fn derive_normal_path(diffuse: &str) -> String {
    let lower = diffuse.to_ascii_lowercase();
    if let Some(stem_len) = lower.strip_suffix("_d.dds").map(str::len) {
        format!("{}_n.dds", &diffuse[..stem_len])
    } else if let Some(stem_len) = lower.strip_suffix(".dds").map(str::len) {
        format!("{}_n.dds", &diffuse[..stem_len])
    } else {
        format!("{diffuse}_n.dds")
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals parsed or copied, not computed"
)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn parse(name: &str) -> MaterialName {
        MaterialName::parse(name).unwrap()
    }

    fn parse_err(name: &str) -> MaterialNameError {
        MaterialName::parse(name).unwrap_err()
    }

    #[test]
    fn diffuse_and_normal_segments_become_texture_root_paths() {
        let m = parse("wall_d.dds|wall_n.dds");
        assert_eq!(m.diffuse_path.as_deref(), Some("textures\\wall_d.dds"));
        assert_eq!(m.normal_path.as_deref(), Some("textures\\wall_n.dds"));
    }

    #[test]
    fn empty_diffuse_segment_means_no_diffuse_texture() {
        let m = parse("|");
        assert_eq!(m.diffuse_path, None);
        assert_eq!(m.normal_path, None);
    }

    #[test]
    fn legacy_name_without_lighting_tokens_is_matte_and_opaque() {
        let m = parse("a_d.dds|a_n.dds");
        assert!(!m.surface.specular_enabled);
        assert_eq!(m.surface.emissive, Vec3::ZERO);
        assert_eq!(m.alpha_cutoff, 0.0);
        assert_eq!(m.alpha_mult, 1.0);
        assert!(!m.is_blend && !m.is_effect && !m.is_decal && !m.two_sided);
    }

    #[test]
    fn single_segment_name_derives_normal_from_d_suffix() {
        let m = parse("rock_d.dds");
        assert_eq!(m.normal_path.as_deref(), Some("textures\\rock_n.dds"));
    }

    #[test]
    fn single_segment_name_without_d_suffix_derives_normal_from_stem() {
        let m = parse("rock.dds");
        assert_eq!(m.normal_path.as_deref(), Some("textures\\rock_n.dds"));
    }

    #[test]
    fn normal_derivation_ignores_suffix_case() {
        let m = parse("Rock_D.DDS");
        assert_eq!(m.normal_path.as_deref(), Some("textures\\Rock_n.dds"));
    }

    #[test]
    fn normal_is_derived_when_only_ts_and_src_tokens_follow() {
        let m = parse("rock_d.dds|ts|src:materials\\rock.bgsm");
        assert_eq!(m.normal_path.as_deref(), Some("textures\\rock_n.dds"));
        assert!(m.two_sided);
        assert_eq!(m.source_material.as_deref(), Some("materials\\rock.bgsm"));
    }

    #[test]
    fn normal_is_not_derived_when_other_tokens_follow_the_diffuse() {
        let m = parse("rock_d.dds|fx");
        assert_eq!(m.normal_path, None);
        assert!(m.is_effect);
    }

    #[test]
    fn alpha_test_token_scales_to_unit_cutoff() {
        let m = parse("a_d.dds|a_n.dds|test:255");
        assert_eq!(m.alpha_cutoff, 1.0);
        let m = parse("a_d.dds|a_n.dds|test:51");
        assert!((m.alpha_cutoff - 0.2).abs() < 1e-6);
    }

    #[test]
    fn alpha_test_above_255_is_rejected() {
        assert!(matches!(
            parse_err("a_d.dds|a_n.dds|test:256"),
            MaterialNameError::InvalidAlphaTest(v) if v == "256"
        ));
    }

    #[test]
    fn alpha_test_negative_or_text_is_rejected() {
        for bad in ["-1", "x", ""] {
            assert!(
                matches!(
                    parse_err(&format!("a_d.dds|a_n.dds|test:{bad}")),
                    MaterialNameError::InvalidAlphaTest(_)
                ),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn blend_without_multiplier_keeps_full_alpha() {
        let m = parse("a_d.dds|a_n.dds|blend");
        assert!(m.is_blend);
        assert_eq!(m.alpha_mult, 1.0);
    }

    #[test]
    fn blend_consumes_a_following_numeric_multiplier() {
        let m = parse("a_d.dds|a_n.dds|blend|0.5|ts");
        assert!(m.is_blend);
        assert_eq!(m.alpha_mult, 0.5);
        assert!(m.two_sided);
    }

    #[test]
    fn blend_followed_by_a_token_does_not_swallow_it() {
        let m = parse("a_d.dds|a_n.dds|blend|decal");
        assert!(m.is_blend && m.is_decal);
        assert_eq!(m.alpha_mult, 1.0);
    }

    #[test]
    fn untextured_blend_without_effect_data_is_a_helper_shape() {
        assert!(parse("||blend").is_helper_shape());
        assert!(!parse("a_d.dds|a_n.dds|blend").is_helper_shape());
    }

    #[test]
    fn classifier_truth_table() {
        struct Case {
            name: &'static str,
            helper: bool,
            effect: bool,
            bgem: bool,
            hidden: bool,
        }
        let cases = [
            Case {
                name: "||blend",
                helper: true,
                effect: false,
                bgem: false,
                hidden: false,
            },
            Case {
                name: "||blend|0.5|fx",
                helper: true,
                effect: false,
                bgem: false,
                hidden: false,
            },
            Case {
                name: "||blend|0.68|fx|bc:000000|bs:1|src:materials\\g.bgem",
                helper: false,
                effect: true,
                bgem: true,
                hidden: false,
            },
            Case {
                name: "||blend|src:materials\\g.BGEM",
                helper: false,
                effect: true,
                bgem: true,
                hidden: false,
            },
            Case {
                name: "||blend|fx|bc:FFFFFF|bs:1",
                helper: false,
                effect: true,
                bgem: false,
                hidden: true,
            },
            Case {
                name: "a_d.dds||blend|fx|src:m.bgem",
                helper: false,
                effect: true,
                bgem: true,
                hidden: false,
            },
            Case {
                name: "a_d.dds|a_n.dds|fx",
                helper: false,
                effect: true,
                bgem: false,
                hidden: true,
            },
            Case {
                name: "w_d.dds|w_n.dds|fx|water:196:80000008:00000001:0,0:1,1",
                helper: false,
                effect: false,
                bgem: false,
                hidden: false,
            },
            Case {
                name: "||fx",
                helper: false,
                effect: false,
                bgem: false,
                hidden: false,
            },
            Case {
                name: "a_d.dds|a_n.dds|blend",
                helper: false,
                effect: false,
                bgem: false,
                hidden: false,
            },
            Case {
                name: "a_d.dds|a_n.dds|src:materials\\w.bgsm",
                helper: false,
                effect: false,
                bgem: false,
                hidden: false,
            },
        ];
        for case in cases {
            let m = parse(case.name);
            assert_eq!(m.is_helper_shape(), case.helper, "helper: {}", case.name);
            assert_eq!(m.is_effect_shape(), case.effect, "effect: {}", case.name);
            assert_eq!(m.is_bgem(), case.bgem, "bgem: {}", case.name);
            assert_eq!(
                m.is_hidden_effect_shape(),
                case.hidden,
                "hidden: {}",
                case.name
            );
        }
    }

    #[test]
    fn hidden_effect_rule_truth_table() {
        let cases = [
            (true, true, None, false),
            (true, false, None, true),
            (true, false, Some("m.bgem"), false),
            (false, false, None, false),
        ];
        for (is_effect, is_water, source, expected) in cases {
            assert_eq!(
                MaterialName::hides_effect_shape_by_default(is_effect, is_water, source),
                expected,
                "effect={is_effect} water={is_water} source={source:?}"
            );
        }
    }

    #[test]
    fn bgem_name_without_effect_tokens_decodes_to_legacy_effect() {
        let m = parse("a_d.dds|a_n.dds|blend|0.5|fx|src:materials\\x.bgem");
        assert_eq!(m.effect, Some(EffectParams::LEGACY));
        assert_eq!(m.env, None);
    }

    #[test]
    fn untokenised_name_has_no_effect_or_env() {
        let m = parse("a_d.dds|a_n.dds|sm:1");
        assert_eq!((m.effect, m.env), (None, None));
    }

    #[test]
    fn water_shape_is_not_an_effect_shape() {
        let m = parse("w_d.dds|w_n.dds|fx|water:196:80000008:00000001:0,0:1,1");
        assert!(!m.is_effect_shape());
    }

    #[test]
    fn full_effect_token_set_decodes() {
        let m = parse(
            "g_d.dds||blend|1.01|fx|bm:add|bc:FF8000|bs:0.1|fo:1,0.5,0.2,0.9,1|li:0.98|pal:ca:shared\\grad_d.dds|env:shared\\cube_e.dds|envk:shared\\mask_g.dds|envs:0.2|ts|src:materials\\g.bgem",
        );
        let effect = m.effect.expect("effect");
        assert_eq!(effect.blend, EffectBlend::Additive);
        assert_eq!(effect.base_color_srgb, [255, 128, 0]);
        assert_eq!(effect.base_color_scale, 0.1);
        let falloff = effect.falloff.expect("falloff");
        assert_eq!(
            (
                falloff.start_facing,
                falloff.stop_facing,
                falloff.start_opacity,
                falloff.stop_opacity
            ),
            (1.0, 0.5, 0.2, 0.9)
        );
        assert!(falloff.scales_color);
        assert_eq!(effect.lighting_influence, Some(0.98));
        let palette = effect.palette.expect("palette");
        assert_eq!(palette.path, "textures\\shared\\grad_d.dds");
        assert!(palette.recolours_color && palette.drives_alpha);
        let env = m.env.expect("env");
        assert_eq!(env.cube_path, "shared\\cube_e.dds");
        assert_eq!(
            env.mask_path.as_deref(),
            Some("textures\\shared\\mask_g.dds")
        );
        assert_eq!(env.mask_scale, 0.2);
        assert_eq!(m.alpha_mult, 1.01);
    }

    #[test]
    fn effect_tokens_default_to_unlit_white_alpha() {
        let effect = parse("||blend|bc:FFFFFF").effect.expect("effect");
        assert_eq!(effect.blend, EffectBlend::Alpha);
        assert_eq!(effect.base_color_scale, 1.0);
        assert_eq!(effect.lighting_influence, None);
        assert_eq!(effect.falloff, None);
        assert_eq!(effect.palette, None);
    }

    #[test]
    fn multiply_blend_mode_decodes() {
        let effect = parse("||blend|bm:mul|bc:FFFFFF").effect.expect("effect");
        assert_eq!(effect.blend, EffectBlend::Multiply);
    }

    #[test]
    fn effect_token_first_after_an_empty_normal_is_not_taken_for_a_normal_path() {
        let m = parse("a_d.dds||bm:add|bc:FFFFFF");
        assert_eq!(m.normal_path, None);
        assert_eq!(m.effect.expect("effect").blend, EffectBlend::Additive);
        let m = parse("a_d.dds|bm:add");
        assert_eq!(m.normal_path, None);
        for token in [
            "bm:add",
            "bc:FFFFFF",
            "bs:1",
            "fo:1,0,0,1,0",
            "li:1",
            "pal:c:p.dds",
            "env:e.dds",
        ] {
            assert_eq!(parse(&format!("|{token}")).normal_path, None, "{token}");
        }
    }

    #[test]
    fn env_without_effect_decodes_for_lit_materials() {
        let m =
            parse("a_d.dds|a_n.dds|test:128|env:shared\\cube_e.dds|envs:0.5|src:materials\\w.bgsm");
        assert_eq!(m.effect, None);
        let env = m.env.expect("env");
        assert_eq!(env.mask_path, None);
        assert_eq!(env.mask_scale, 0.5);
    }

    #[test]
    fn env_scale_defaults_to_one() {
        assert_eq!(
            parse("a_d.dds|a_n.dds|env:e.dds").env.unwrap().mask_scale,
            1.0
        );
    }

    #[test]
    fn env_mask_or_scale_without_cube_is_rejected() {
        assert!(matches!(
            parse_err("a_d.dds|a_n.dds|envk:m.dds"),
            MaterialNameError::EnvTokenWithoutCube("envk")
        ));
        assert!(matches!(
            parse_err("a_d.dds|a_n.dds|envs:1"),
            MaterialNameError::EnvTokenWithoutCube("envs")
        ));
    }

    #[test]
    fn empty_env_paths_are_rejected() {
        for token in ["env", "envk"] {
            assert!(
                matches!(
                    parse_err(&format!("a_d.dds|a_n.dds|{token}:")),
                    MaterialNameError::EmptyTexturePath(t) if t == token
                ),
                "{token}"
            );
        }
        assert!(matches!(
            parse_err("a_d.dds|a_n.dds|pal:c:"),
            MaterialNameError::InvalidPalette(_)
        ));
    }

    #[test]
    fn malformed_effect_tokens_are_rejected() {
        assert!(
            matches!(parse_err("a_d.dds|a_n.dds|bm:x"), MaterialNameError::InvalidBlendMode(v) if v == "x")
        );
        assert!(matches!(
            parse_err("a_d.dds|a_n.dds|bm:"),
            MaterialNameError::InvalidBlendMode(_)
        ));
        for bad in [
            "1,0,0,1",
            "1,0,0,1,0,0",
            "1,0,0,1,2",
            "a,0,0,1,0",
            "1,0,0,nan,0",
            "",
        ] {
            assert!(
                matches!(
                    parse_err(&format!("a_d.dds|a_n.dds|fo:{bad}")),
                    MaterialNameError::InvalidFalloff(_)
                ),
                "{bad:?}"
            );
        }
        for bad in ["p.dds", "x:p.dds", "cc:p.dds", ":p.dds", "ca:", "c"] {
            assert!(
                matches!(
                    parse_err(&format!("a_d.dds|a_n.dds|pal:{bad}")),
                    MaterialNameError::InvalidPalette(_)
                ),
                "{bad:?}"
            );
        }
        assert!(matches!(
            parse_err("a_d.dds|a_n.dds|bc:12"),
            MaterialNameError::InvalidColor { token: "bc", .. }
        ));
        assert!(matches!(
            parse_err("a_d.dds|a_n.dds|bs:x"),
            MaterialNameError::InvalidFloat { token: "bs", .. }
        ));
    }

    #[test]
    fn duplicate_effect_tokens_are_rejected() {
        for token in [
            "bm:add",
            "bc:FFFFFF",
            "bs:1",
            "fo:1,0,0,1,0",
            "li:1",
            "pal:c:p.dds",
            "env:e.dds",
        ] {
            let key = token.split(':').next().unwrap();
            let name = format!("a_d.dds|a_n.dds|{token}|{token}");
            assert!(
                matches!(parse_err(&name), MaterialNameError::DuplicateToken(k) if k == key),
                "{token}"
            );
        }
    }

    #[test]
    fn palette_flags_pick_colour_and_alpha_independently() {
        let colour = parse("a_d.dds|a_n.dds|pal:c:p.dds")
            .effect
            .unwrap()
            .palette
            .unwrap();
        assert!(colour.recolours_color && !colour.drives_alpha);
        let alpha = parse("a_d.dds|a_n.dds|pal:a:p.dds")
            .effect
            .unwrap()
            .palette
            .unwrap();
        assert!(!alpha.recolours_color && alpha.drives_alpha);
    }

    #[test]
    fn palette_path_may_contain_colons() {
        let palette = parse("a_d.dds|a_n.dds|pal:c:dir:p.dds")
            .effect
            .unwrap()
            .palette
            .unwrap();
        assert_eq!(palette.path, "textures\\dir:p.dds");
    }

    #[test]
    fn eff_token_carries_the_effect_key_and_keeps_fx() {
        let m = parse("flame_d.dds||fx|eff:0A1b2C3d");
        assert_eq!(m.effect_key, Some(0x0A1B_2C3D));
        assert!(m.is_effect);
        assert_eq!(parse("a_d.dds|a_n.dds").effect_key, None);
    }

    #[test]
    fn eff_token_must_be_eight_hex_digits() {
        for bad in ["eff:1234567", "eff:123456789", "eff:0000000g", "eff:"] {
            let name = format!("a_d.dds|a_n.dds|{bad}");
            assert!(matches!(
                parse_err(&name),
                MaterialNameError::InvalidEffectKey(_)
            ));
        }
    }

    #[test]
    fn eff_token_after_a_normal_segment_is_not_mistaken_for_the_normal() {
        let m = parse("a_d.dds|a_n.dds|eff:00000001");
        assert_eq!(m.normal_path.as_deref(), Some("textures\\a_n.dds"));
        assert_eq!(m.effect_key, Some(1));
    }

    #[test]
    fn refr_token_marks_a_refraction_shape_and_keeps_both_texture_slots() {
        let m = parse("heat_d.dds|heat_n.dds|refr");
        assert!(m.is_refraction);
        assert_eq!(m.diffuse_path.as_deref(), Some("textures\\heat_d.dds"));
        assert_eq!(m.normal_path.as_deref(), Some("textures\\heat_n.dds"));
        assert!(!parse("a_d.dds|a_n.dds").is_refraction);
        assert!(!parse("a_d.dds|a_n.dds|fx").is_refraction);
    }

    #[test]
    fn refr_without_normal_segment_is_still_a_token() {
        let m = parse("heat_d.dds|refr");
        assert!(m.is_refraction);
        assert_eq!(m.normal_path, None);
    }

    #[test]
    fn repeated_effect_tokens_are_rejected() {
        assert!(matches!(
            parse_err("a_d.dds|a_n.dds|refr|refr"),
            MaterialNameError::DuplicateToken("refr")
        ));
        assert!(matches!(
            parse_err("a_d.dds|a_n.dds|eff:00000001|eff:00000002"),
            MaterialNameError::DuplicateToken("eff")
        ));
    }

    #[test]
    fn water_token_marks_a_water_shape_that_is_not_fx() {
        let m = parse(
            r"Water\DefaultWaterTile.dds|Water\WaterTileHumpy_n.dds|fx|water:196:80000008:00000001:0,0:1,1",
        );
        let water = m.water.expect("water token decoded");
        assert_eq!(water.water_flags, 196);
        assert!(m.is_water_shape());
        assert!(m.is_effect && !m.is_effect_shape());
    }

    #[test]
    fn water_token_value_keeps_its_colons() {
        let m = parse("|fx|water:0:00000000:00000000:0.5,0:1.5,1|ts");
        assert_eq!(m.water.unwrap().uv_scale, glam::Vec2::new(1.5, 1.0));
        assert!(m.two_sided);
    }

    #[test]
    fn malformed_or_repeated_water_token_is_rejected() {
        assert!(matches!(
            parse_err("a_d.dds|a_n.dds|water:1:2"),
            MaterialNameError::InvalidWater(v) if v == "1:2"
        ));
        let token = "water:0:00000000:00000000:0,0:1,1";
        assert!(matches!(
            parse_err(&format!("a_d.dds|a_n.dds|{token}|{token}")),
            MaterialNameError::DuplicateToken("water")
        ));
    }

    #[test]
    fn name_without_water_token_is_not_a_water_shape() {
        assert!(!parse("a_d.dds|a_n.dds|fx").is_water_shape());
    }

    #[test]
    fn any_of_the_three_specular_tokens_enables_specular() {
        for token in ["sm:1.5", "sg:0.4", "fp:3"] {
            let m = parse(&format!("a_d.dds|a_n.dds|{token}"));
            assert!(m.surface.specular_enabled, "{token}");
        }
    }

    #[test]
    fn specular_values_override_legacy_defaults() {
        let m = parse("a_d.dds|a_n.dds|sc:FF8000|sm:2|sg:0.75|fp:4");
        assert_eq!(m.surface.specular_color, [255, 128, 0]);
        assert_eq!(m.surface.specular_mult, 2.0);
        assert_eq!(m.surface.smoothness, 0.75);
        assert_eq!(m.surface.fresnel_power, 4.0);
    }

    #[test]
    fn emissive_multiplies_the_linear_colour() {
        let m = parse("a_d.dds|a_n.dds|em:FFFFFF|emm:2.5");
        assert!(m.surface.emissive.abs_diff_eq(Vec3::splat(2.5), 1e-5));
    }

    #[test]
    fn emissive_multiplier_alone_defaults_to_white() {
        let m = parse("a_d.dds|a_n.dds|emm:3");
        assert!(m.surface.emissive.abs_diff_eq(Vec3::splat(3.0), 1e-5));
    }

    #[test]
    fn emissive_colour_alone_defaults_to_multiplier_one() {
        let m = parse("a_d.dds|a_n.dds|em:FFFFFF");
        assert!(m.surface.emissive.abs_diff_eq(Vec3::ONE, 1e-5));
    }

    #[test]
    fn emissive_colour_is_converted_from_srgb_to_linear() {
        // sRGB 0x80 = 128/255 -> ((0.50196 + 0.055) / 1.055)^2.4 = 0.2159 (standard sRGB EOTF).
        let m = parse("a_d.dds|a_n.dds|em:808080");
        assert!((m.surface.emissive.x - 0.2159).abs() < 1e-3);
    }

    #[test]
    fn glow_map_is_kept_only_while_the_material_emits() {
        let emitting = parse("a_d.dds|a_n.dds|glow:a_g.dds|em:FFFFFF");
        assert_eq!(emitting.glow_path.as_deref(), Some("textures\\a_g.dds"));
        let dark = parse("a_d.dds|a_n.dds|glow:a_g.dds");
        assert_eq!(dark.glow_path, None);
        let black = parse("a_d.dds|a_n.dds|glow:a_g.dds|em:000000");
        assert_eq!(black.glow_path, None);
    }

    #[test]
    fn specular_map_path_gets_the_texture_root() {
        let m = parse("a_d.dds|a_n.dds|spec:a_s.dds");
        assert_eq!(m.specular_path.as_deref(), Some("textures\\a_s.dds"));
    }

    #[test]
    fn empty_spec_or_glow_path_is_rejected() {
        assert!(matches!(
            parse_err("a_d.dds|a_n.dds|spec:"),
            MaterialNameError::EmptyTexturePath("spec")
        ));
        assert!(matches!(
            parse_err("a_d.dds|a_n.dds|glow:"),
            MaterialNameError::EmptyTexturePath("glow")
        ));
    }

    #[test]
    fn duplicate_token_is_rejected() {
        assert!(matches!(
            parse_err("a_d.dds|a_n.dds|sm:1|sm:2"),
            MaterialNameError::DuplicateToken("sm")
        ));
    }

    #[test]
    fn unknown_token_is_rejected_with_its_text() {
        assert!(matches!(
            parse_err("a_d.dds|a_n.dds|ts|bogus"),
            MaterialNameError::UnknownToken(t) if t == "bogus"
        ));
        assert!(matches!(
            parse_err("a_d.dds|a_n.dds|zz:1"),
            MaterialNameError::UnknownToken(t) if t == "zz:1"
        ));
    }

    #[test]
    fn colour_must_be_exactly_six_hex_digits() {
        for bad in ["FFF", "FFFFFFF", "GGGGGG", "", "+FFFFF"] {
            assert!(
                matches!(
                    parse_err(&format!("a_d.dds|a_n.dds|sc:{bad}")),
                    MaterialNameError::InvalidColor { token: "sc", .. }
                ),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn colour_accepts_lowercase_hex() {
        let m = parse("a_d.dds|a_n.dds|sc:ff00aa|sm:1");
        assert_eq!(m.surface.specular_color, [255, 0, 170]);
    }

    #[test]
    fn non_finite_or_non_numeric_floats_are_rejected() {
        for bad in ["nan", "inf", "abc", ""] {
            assert!(
                matches!(
                    parse_err(&format!("a_d.dds|a_n.dds|sm:{bad}")),
                    MaterialNameError::InvalidFloat { token: "sm", .. }
                ),
                "{bad:?}"
            );
        }
    }

    proptest! {
        #[test]
        fn arbitrary_material_names_parse_or_error_without_panicking(name in "\\PC{0,80}") {
            let _ = MaterialName::parse(&name);
        }

        #[test]
        fn token_soup_never_panics(
            segments in proptest::collection::vec(
                prop_oneof![
                    Just("ts".to_string()), Just("fx".to_string()), Just("blend".to_string()),
                    Just("decal".to_string()), Just("0.5".to_string()), Just("test:".to_string()),
                    Just("sc:FF00FF".to_string()), Just("em:".to_string()), "[a-z:.0-9]{0,8}",
                ],
                0..8,
            ),
        ) {
            let _ = MaterialName::parse(&segments.join("|"));
        }
    }
}
