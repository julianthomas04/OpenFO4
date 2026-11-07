// Mirrors `MoonUniform` in post/night_sky/moon.rs.
struct MoonUniform {
    // xyz: unit direction towards the moon, w: half edge of the quad on the tangent plane.
    direction_half_extent: vec4<f32>,
    // xyz: unit right axis of the quad, w: visibility from 0 to 1.
    right_visibility: vec4<f32>,
    // xyz: unit up axis of the quad, w: the sky radiance scale times the radiance of a lit moon texel.
    up_radiance_scale: vec4<f32>,
}

@group(3) @binding(0) var<uniform> moon: MoonUniform;
@group(3) @binding(1) var moon_texture: texture_2d<f32>;
@group(3) @binding(2) var moon_sampler: sampler;

struct MoonVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Triangle strip over the quad corners: (-1,-1), (1,-1), (-1,1), (1,1).
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> MoonVertexOutput {
    let corner = vec2<f32>(f32(vertex_index & 1u), f32(vertex_index >> 1u)) * 2.0 - 1.0;
    let direction = moon.direction_half_extent.xyz
        + (moon.right_visibility.xyz * corner.x + moon.up_radiance_scale.xyz * corner.y)
            * moon.direction_half_extent.w;
    var out: MoonVertexOutput;
    // w = 0 drops the camera translation, and the reverse-Z infinite projection puts the
    // point at depth 0, like the sky gradient.
    out.clip_position = frame.view_proj * vec4<f32>(direction, 0.0);
    out.uv = vec2<f32>(corner.x * 0.5 + 0.5, 0.5 - corner.y * 0.5);
    return out;
}

@fragment
fn fs_main(in: MoonVertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(moon_texture, moon_sampler, in.uv);
    let radiance = texel.rgb * moon.up_radiance_scale.w;
    return vec4<f32>(radiance, texel.a * moon.right_visibility.w);
}
