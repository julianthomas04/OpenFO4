//! Effect materials for tests: the stub mesh permutation with a chosen blend.

use serde_json::{Value, json};

use super::shader::stubs;
use crate::assets::types::effect_material::EffectMaterial;

/// A material on the stub mesh shaders; `blend` is the JSON of the material's `blend` object.
pub fn stub_material_with_blend(blend: &Value) -> EffectMaterial {
    let mut document = json!({
        "v": 1, "vs": stubs::MESH_VS, "ps": stubs::MESH_PS,
        "flags1": 0, "flags2": 0, "blend": null,
        "z_test": true, "z_write": false, "double_sided": true,
        "clamp": "wrap_s_wrap_t", "textures": {},
        "base_color": [1.0, 1.0, 1.0, 1.0], "base_color_scale": 1.0,
        "falloff": { "start_angle": 1.0, "stop_angle": 1.0,
                     "start_opacity": 1.0, "stop_opacity": 1.0 },
        "soft_falloff_depth": 0.0, "uv_offset": [0.0, 0.0], "uv_scale": [1.0, 1.0],
        "lighting_influence": 0.0, "env_map_scale": 1.0,
        "emittance_color": [0.0, 0.0, 0.0], "refraction_power": 0.0, "alpha": 1.0
    });
    document["blend"] = blend.clone();
    serde_json::from_value(document).unwrap()
}

/// An unblended, depth-writing material on the stub mesh shaders.
pub fn stub_material() -> EffectMaterial {
    stub_material_with_blend(&json!({ "mode": "none" }))
}
