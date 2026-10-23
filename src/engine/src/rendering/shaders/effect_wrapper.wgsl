// Effect shader wrapper: the globals the install-time translated effect vertex and pixel
// functions (`effect_vs_<descriptor>`, `effect_ps_<descriptor>`) read their register arrays,
// textures and samplers through. The vertex entry, varyings and fragment entry that connect
// a vertex/pixel pair are generated per pair by `rendering/effects/shader.rs` from the
// shaders' interface metadata. `rendering/effects/constants.rs` fills the registers.
//
// Bind groups: 0 scene copy (only the depth copy is read), 1 per frame, 2 per material,
// 3 per draw (dynamic offset).

struct EffectFrame {
    cb12: array<vec4<f32>, 47>,
    vs_cb0: array<vec4<f32>, 1>,
    ps_cb0: array<vec4<f32>, 20>,
}

struct EffectDraw {
    vs_cb1: array<vec4<f32>, 3>,
    vs_cb2: array<vec4<f32>, 26>,
    vs_cb11: array<vec4<f32>, 128>,
    ps_cb1: array<vec4<f32>, 5>,
    ps_cb2: array<vec4<f32>, 17>,
}

// Declared so that skinned and vertex-pulling vertex shaders compose; no draw binds them.
struct EffectBones {
    cb10: array<vec4<f32>, 180>,
}

@group(0) @binding(1) var effect_t3: texture_depth_2d;

@group(1) @binding(0) var<uniform> effect_frame: EffectFrame;

@group(2) @binding(0) var effect_t0: texture_2d<f32>;
@group(2) @binding(1) var effect_t1: texture_2d<f32>;
@group(2) @binding(2) var effect_t2: texture_2d<f32>;
@group(2) @binding(4) var effect_t4: texture_2d<f32>;
@group(2) @binding(5) var effect_t5: texture_cube<f32>;
@group(2) @binding(6) var effect_t6: texture_2d<f32>;
@group(2) @binding(7) var effect_t7: texture_2d<f32>;
@group(2) @binding(8) var effect_t8: texture_2d<f32>;
@group(2) @binding(10) var effect_s0: sampler;
@group(2) @binding(11) var effect_s1: sampler;
@group(2) @binding(12) var effect_s2: sampler;
@group(2) @binding(13) var effect_s3: sampler;
@group(2) @binding(14) var effect_s4: sampler;
@group(2) @binding(15) var effect_s5: sampler;
@group(2) @binding(16) var effect_s6: sampler;
@group(2) @binding(17) var effect_s7: sampler;
@group(2) @binding(18) var effect_s8: sampler;

@group(3) @binding(0) var<uniform> effect_draw: EffectDraw;
@group(3) @binding(1) var<uniform> effect_bones: EffectBones;
@group(3) @binding(2) var<storage, read> effect_vtx_t8: array<u32>;
@group(3) @binding(3) var<storage, read> effect_vtx_t9: array<u32>;
@group(3) @binding(4) var<storage, read> effect_vtx_t10: array<u32>;
@group(3) @binding(5) var<storage, read> effect_vtx_t11: array<u32>;
