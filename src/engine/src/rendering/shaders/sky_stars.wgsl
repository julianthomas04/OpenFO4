const MIN_FADE_SINE = 1e-6;

// Mirrors `StarUniform` in post/night_sky/stars.rs.
struct StarUniform {
    rotation: mat4x4<f32>,
    // x: opacity of the stars, y: the sky radiance scale times the radiance of a star texel,
    // z: the sine of the elevation at which the stars are fully faded in.
    params: vec4<f32>,
}

@group(3) @binding(0) var<uniform> stars: StarUniform;
@group(3) @binding(1) var star_texture: texture_2d<f32>;
@group(3) @binding(2) var star_sampler: sampler;

struct StarVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) direction: vec3<f32>,
}

@vertex
fn vs_main(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>) -> StarVertexOutput {
    let direction = (stars.rotation * vec4<f32>(position, 0.0)).xyz;
    var out: StarVertexOutput;
    // w = 0 centres the dome on the camera and puts it at depth 0 under the reverse-Z
    // infinite projection, like the sky gradient.
    out.clip_position = frame.view_proj * vec4<f32>(direction, 0.0);
    out.uv = uv;
    out.direction = direction;
    return out;
}

// The star texture holds a flat grey in RGB and the stars in alpha.
@fragment
fn fs_main(in: StarVertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(star_texture, star_sampler, in.uv);
    let above_horizon = smoothstep(0.0, max(stars.params.z, MIN_FADE_SINE), normalize(in.direction).y);
    let radiance = texel.rgb * texel.a * stars.params.x * stars.params.y * above_horizon;
    return vec4<f32>(radiance, 0.0);
}
