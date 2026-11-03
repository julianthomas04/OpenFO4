// The large loading screen's colour pass: the linear picture of its stage and model, through the
// cinematic stage of its image space, encoded to sRGB and looked up in its colour LUT. The result
// is premultiplied by the fade.

// The largest picture colour graded: a float picture holds infinities, and the saturation's
// luma difference turns one into NaN that spreads over the whole pixel. A NaN texel grades as 0.
const PICTURE_MAX = 65000.0;

struct GradeParams {
    // x: the fade, y: 1 when the target decodes sRGB itself, z and w: padding.
    look: vec4<f32>,
    // x saturation, y brightness, z contrast, w tint amount.
    cinematic: vec4<f32>,
    // rgb: the tint colour, linear.
    tint: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: GradeParams;
@group(0) @binding(1) var picture: texture_2d<f32>;
// Red along x, green along y, blue along z.
@group(0) @binding(2) var colour_lut: texture_3d<f32>;
@group(0) @binding(3) var colour_lut_sampler: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    var out: VsOut;
    out.clip = vec4<f32>(fullscreen_triangle_ndc(vertex_index), 0.0, 1.0);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let loaded = textureLoad(picture, vec2<i32>(in.clip.xy), 0).rgb;
    let linear = select(vec3<f32>(0.0), clamp(loaded, vec3<f32>(0.0), vec3<f32>(PICTURE_MAX)), loaded == loaded);
    let display = linear_to_srgb(apply_cinematic(linear, params.cinematic, params.tint.rgb));
    let coordinate = colour_lut_coordinate(display, f32(textureDimensions(colour_lut).x));
    let graded = textureSampleLevel(colour_lut, colour_lut_sampler, coordinate, 0.0).rgb;
    let written = select(graded, srgb_to_linear(graded), params.look.y > 0.5);
    let fade = params.look.x;
    return vec4<f32>(written * fade, fade);
}
