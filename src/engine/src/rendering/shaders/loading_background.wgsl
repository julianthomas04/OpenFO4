// The large loading screen's background: the interface image and, added over it, the grid
// overlay, both sampled at the same coordinates, then multiplied per channel and lifted. All in
// gamma-encoded values, because the menu draws through the non-sRGB view of the swapchain. The
// result is premultiplied by the fade.

struct BackgroundParams {
    // The right end of the u range, the factor of the overlay's alpha, the fade, unused.
    look: vec4<f32>,
    tint: vec4<f32>,
    lift: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: BackgroundParams;
@group(0) @binding(1) var background: texture_2d<f32>;
@group(0) @binding(2) var overlay: texture_2d<f32>;
@group(0) @binding(3) var layer_sampler: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// A triangle strip of four vertices over the whole target, v growing downwards.
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    let corner = vec2<f32>(f32(vertex_index & 1u), f32(vertex_index >> 1u));
    var out: VsOut;
    out.clip = vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(corner.x * params.look.x, corner.y);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let base = textureSample(background, layer_sampler, in.uv);
    let grid = textureSample(overlay, layer_sampler, in.uv);
    let graded = params.tint.rgb * (base.rgb + params.look.y * grid.a * grid.rgb) + params.lift.rgb;
    let fade = params.look.z;
    return vec4<f32>(graded * fade, fade);
}
