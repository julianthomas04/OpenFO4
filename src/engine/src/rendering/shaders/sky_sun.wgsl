const MIN_FADE_SINE = 1e-6;

// Mirrors `SunUniform` in post/night_sky/sun.rs.
struct SunUniform {
    // xyz: unit direction towards the sun, w: half edge of the quad on the tangent plane.
    direction_half_extent: vec4<f32>,
    // xyz: unit right axis of the quad, w: the sun's alpha from 0 to 1.
    right_alpha: vec4<f32>,
    // xyz: unit up axis of the quad, w: the sine of the elevation at which the sprite is fully faded in.
    up_horizon_fade: vec4<f32>,
    // rgb: radiance of a fully opaque texel (colour, radiance scale and sprite brightness).
    radiance: vec4<f32>,
}

@group(3) @binding(0) var<uniform> sun: SunUniform;
@group(3) @binding(1) var sun_texture: texture_2d<f32>;
@group(3) @binding(2) var sun_sampler: sampler;

struct SunVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) direction: vec3<f32>,
}

// Triangle strip over the quad corners: (-1,-1), (1,-1), (-1,1), (1,1).
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> SunVertexOutput {
    let corner = vec2<f32>(f32(vertex_index & 1u), f32(vertex_index >> 1u)) * 2.0 - 1.0;
    let direction = sun.direction_half_extent.xyz
        + (sun.right_alpha.xyz * corner.x + sun.up_horizon_fade.xyz * corner.y)
            * sun.direction_half_extent.w;
    var out: SunVertexOutput;
    // w = 0 drops the camera translation, and the reverse-Z infinite projection puts the
    // point at depth 0, like the sky gradient.
    out.clip_position = frame.view_proj * vec4<f32>(direction, 0.0);
    out.uv = vec2<f32>(corner.x * 0.5 + 0.5, 0.5 - corner.y * 0.5);
    out.direction = direction;
    return out;
}

// The sprite is added over the sky, weighted by the texture alpha: the textures hold a flat
// warm colour in RGB and the shape of the sun in alpha. The part below the horizon is cut.
@fragment
fn fs_main(in: SunVertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(sun_texture, sun_sampler, in.uv);
    let above_horizon = smoothstep(0.0, max(sun.up_horizon_fade.w, MIN_FADE_SINE), normalize(in.direction).y);
    let radiance = texel.rgb * texel.a * sun.radiance.rgb * sun.right_alpha.w * above_horizon;
    return vec4<f32>(radiance, 0.0);
}
