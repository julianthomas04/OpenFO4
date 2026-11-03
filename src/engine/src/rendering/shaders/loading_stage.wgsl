// The planes of the large loading screen's stage, each drawn as an effect-shader shape of its
// one source texture: the texel times the material's base colour, its alpha times the base
// colour's alpha and the material's alpha. Linear light; the textures decode sRGB.

struct StageView {
    view_proj: mat4x4<f32>,
};

struct StageMaterial {
    // rgb: the base colour times its scale; a: the base colour's alpha times the material's.
    colour: vec4<f32>,
};

@group(0) @binding(0) var<uniform> stage: StageView;
@group(1) @binding(0) var source: texture_2d<f32>;
@group(1) @binding(1) var source_sampler: sampler;
@group(1) @binding(2) var<uniform> material: StageMaterial;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip = stage.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(source, source_sampler, in.uv);
    return vec4<f32>(texel.rgb * material.colour.rgb, texel.a * material.colour.a);
}
