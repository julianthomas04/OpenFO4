// The loading screen's logo: a premultiplied white-mask frame multiplied by the HUD colour and
// written as is, because the menu draws through the non-sRGB view of the swapchain. The quad
// covers the logo's rectangle.

struct IconParams {
    // Left, top, right, bottom of the logo in clip space.
    rect: vec4<f32>,
    // The tint's red, green and blue, and the opacity of the whole logo.
    colour: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: IconParams;
@group(0) @binding(1) var art: texture_2d<f32>;
@group(0) @binding(2) var art_sampler: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// A triangle strip of four vertices: (0,0) (1,0) (0,1) (1,1), v growing downwards.
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VsOut {
    let corner = vec2<f32>(f32(vertex_index & 1u), f32(vertex_index >> 1u));
    var out: VsOut;
    out.clip = vec4<f32>(
        mix(params.rect.x, params.rect.z, corner.x),
        mix(params.rect.y, params.rect.w, corner.y),
        0.0,
        1.0,
    );
    out.uv = corner;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let texel = textureSample(art, art_sampler, in.uv);
    return texel * vec4<f32>(params.colour.rgb, 1.0) * params.colour.a;
}
