//! `material.extras.effect` and `material.extras.refraction`: the converter's
//! description of an effect-shader shape and of a heat-haze refraction shape.

use serde::{Deserialize, Serialize};

use crate::anim::scalar_track::Track;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    Add,
    Alpha,
    Mult,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendFactor {
    One,
    Zero,
    SrcColor,
    InvSrcColor,
    DstColor,
    InvDstColor,
    SrcAlpha,
    InvSrcAlpha,
    DstAlpha,
    InvDstAlpha,
    SrcAlphaSaturate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestFunc {
    Always,
    Less,
    Equal,
    Lequal,
    Greater,
    Notequal,
    Gequal,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClampMode {
    ClampSClampT,
    ClampSWrapT,
    WrapSClampT,
    WrapSWrapT,
}

/// Blend factors and alpha test of the shape's alpha property. `mode: none`
/// (no alpha property) leaves the factors and test unset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Blend {
    pub mode: BlendMode,
    #[serde(default)]
    pub src: Option<BlendFactor>,
    #[serde(default)]
    pub dst: Option<BlendFactor>,
    #[serde(default)]
    pub alpha_test: bool,
    #[serde(default = "default_test_func")]
    pub test_func: TestFunc,
    /// Threshold in 0..=1 (the NIF byte / 255).
    #[serde(default)]
    pub test_ref: f32,
}

fn default_test_func() -> TestFunc {
    TestFunc::Always
}

/// Deployed texture paths; an empty string means the slot is unused.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectTextures {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub greyscale: String,
    #[serde(default)]
    pub normal: String,
    #[serde(default)]
    pub env: String,
    #[serde(default)]
    pub env_mask: String,
    #[serde(default)]
    pub reflectance: String,
    #[serde(default)]
    pub lighting: String,
    #[serde(default)]
    pub emit_gradient: String,
}

impl EffectTextures {
    /// `None` for an unused slot.
    pub fn used(slot: &str) -> Option<&str> {
        (!slot.is_empty()).then_some(slot)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Falloff {
    pub start_angle: f32,
    pub stop_angle: f32,
    pub start_opacity: f32,
    pub stop_opacity: f32,
}

/// Effect-shader shape: the shader permutation to run and its parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectMaterial {
    pub v: u32,
    /// Translated shader names, e.g. `effect_vs_00a00015`.
    pub vs: String,
    pub ps: String,
    pub flags1: u32,
    pub flags2: u32,
    pub blend: Blend,
    pub z_test: bool,
    pub z_write: bool,
    pub double_sided: bool,
    pub clamp: ClampMode,
    pub textures: EffectTextures,
    pub base_color: [f32; 4],
    pub base_color_scale: f32,
    pub falloff: Falloff,
    /// Engine units.
    pub soft_falloff_depth: f32,
    pub uv_offset: [f32; 2],
    pub uv_scale: [f32; 2],
    pub lighting_influence: f32,
    pub env_map_scale: f32,
    pub emittance_color: [f32; 3],
    pub refraction_power: f32,
    pub alpha: f32,
    #[serde(default)]
    pub controllers: Vec<Track>,
}

/// Heat-haze shape (`token refr`): drawn by the refraction stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefractionMaterial {
    pub v: u32,
    pub strength: f32,
    pub normal_a: String,
    pub normal_b: String,
    pub uv_offset: [f32; 2],
    pub uv_scale: [f32; 2],
    pub vertex_colors: bool,
    pub double_sided: bool,
    #[serde(default)]
    pub controllers: Vec<Track>,
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals parsed or copied, not computed"
)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::assets::types::model_extras::{ExtrasError, parse_versioned};

    pub(crate) fn effect_fixture() -> Value {
        json!({
            "v": 1,
            "vs": "effect_vs_00a00015", "ps": "effect_ps_00a06015",
            "flags1": 2_147_483_664u32, "flags2": 48,
            "blend": { "mode": "add", "src": "src_alpha", "dst": "one",
                       "alpha_test": false, "test_func": "greater", "test_ref": 0.5 },
            "z_test": true, "z_write": false, "double_sided": true,
            "clamp": "wrap_s_wrap_t",
            "textures": { "source": "effects/embers_d.dds",
                          "greyscale": "effects/gradients/fireflame.dds",
                          "normal": "", "env": "", "env_mask": "", "reflectance": "",
                          "lighting": "", "emit_gradient": "" },
            "base_color": [0.64, 0.64, 0.64, 1.0], "base_color_scale": 2.64,
            "falloff": { "start_angle": 1.0, "stop_angle": 1.0,
                         "start_opacity": 0.0, "stop_opacity": 0.0 },
            "soft_falloff_depth": 10.0,
            "uv_offset": [0.0024, 0.0024], "uv_scale": [1.0, 1.0],
            "lighting_influence": 0.098, "env_map_scale": 1.0,
            "emittance_color": [0, 0, 0], "refraction_power": 0.0, "alpha": 1.0,
            "controllers": [
                { "target": "emissive_mult", "cycle": "loop", "start": 0.0, "stop": 50.0,
                  "frequency": 1.0, "phase": 0.0,
                  "times": [0.0, 0.0333], "values": [2.6, 2.7] },
                { "target": "emissive_color", "cycle": "clamp", "start": 0.0, "stop": 1.0,
                  "frequency": 1.0, "phase": 0.0,
                  "times": [0.0], "values": [[1.0, 0.5, 0.25]] }
            ]
        })
    }

    #[test]
    fn effect_fixture_parses_every_field() {
        let material: EffectMaterial = parse_versioned("effect", &effect_fixture()).unwrap();
        assert_eq!(material.ps, "effect_ps_00a06015");
        assert_eq!(material.flags1, 2_147_483_664);
        assert_eq!(material.blend.mode, BlendMode::Add);
        assert_eq!(material.blend.src, Some(BlendFactor::SrcAlpha));
        assert_eq!(material.blend.test_func, TestFunc::Greater);
        assert_eq!(material.clamp, ClampMode::WrapSWrapT);
        assert_eq!(
            EffectTextures::used(&material.textures.source),
            Some("effects/embers_d.dds")
        );
        assert_eq!(EffectTextures::used(&material.textures.normal), None);
        assert_eq!(material.base_color_scale, 2.64);
        assert_eq!(material.emittance_color, [0.0; 3]);
        assert_eq!(material.controllers.len(), 2);
    }

    #[test]
    fn effect_round_trips_through_json() {
        let material: EffectMaterial = parse_versioned("effect", &effect_fixture()).unwrap();
        let again: EffectMaterial =
            parse_versioned("effect", &serde_json::to_value(&material).unwrap()).unwrap();
        assert_eq!(material, again);
    }

    #[test]
    fn unknown_effect_version_is_rejected_with_its_number() {
        let mut fixture = effect_fixture();
        fixture["v"] = json!(2);
        let error = parse_versioned::<EffectMaterial>("effect", &fixture).unwrap_err();
        assert!(matches!(
            error,
            ExtrasError::UnsupportedVersion {
                kind: "effect",
                found: 2,
                ..
            }
        ));
    }

    #[test]
    fn missing_effect_field_is_an_invalid_error_naming_it() {
        let mut fixture = effect_fixture();
        fixture.as_object_mut().unwrap().remove("ps");
        let error = parse_versioned::<EffectMaterial>("effect", &fixture).unwrap_err();
        assert!(matches!(error, ExtrasError::Invalid { .. }));
        assert!(error.to_string().contains("ps"));
    }

    #[test]
    fn unknown_blend_factor_is_rejected() {
        let mut fixture = effect_fixture();
        fixture["blend"]["src"] = json!("half_alpha");
        assert!(matches!(
            parse_versioned::<EffectMaterial>("effect", &fixture),
            Err(ExtrasError::Invalid { .. })
        ));
    }

    #[test]
    fn blend_mode_none_needs_no_factors() {
        let mut fixture = effect_fixture();
        fixture["blend"] = json!({ "mode": "none" });
        let material: EffectMaterial = parse_versioned("effect", &fixture).unwrap();
        assert_eq!(material.blend.src, None);
        assert_eq!(material.blend.test_func, TestFunc::Always);
    }

    #[test]
    fn refraction_fixture_round_trips() {
        let fixture = json!({
            "v": 1, "strength": 0.04,
            "normal_a": "effects/smokevapor02tile_n.dds",
            "normal_b": "effects/smokevapor01tile_n.dds",
            "uv_offset": [0, 0], "uv_scale": [1, 1],
            "vertex_colors": true, "double_sided": false,
            "controllers": [
                { "target": "u_offset", "cycle": "loop", "start": 0.0, "stop": 4.0,
                  "times": [0.0, 4.0], "values": [0.0, 1.0] }
            ]
        });
        let material: RefractionMaterial = parse_versioned("refraction", &fixture).unwrap();
        assert_eq!(material.strength, 0.04);
        assert!(material.vertex_colors);
        let again: RefractionMaterial =
            parse_versioned("refraction", &serde_json::to_value(&material).unwrap()).unwrap();
        assert_eq!(material, again);
    }
}
