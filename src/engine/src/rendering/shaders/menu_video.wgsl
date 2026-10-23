// The main menu background video: three 8-bit planes (luma, Cb, Cr) converted to the video's
// gamma-encoded R'G'B' and written as is, because the menu draws through the non-sRGB view of the
// swapchain. The quad covers the fitted rectangle; the rest of the target stays black.

struct VideoParams {
    // Left, top, right, bottom of the fitted rectangle in clip space.
    rect: vec4<f32>,
    // Subtracted from (Y, Cb, Cr) before the matrix.
    bias: vec4<f32>,
    row_r: vec4<f32>,
    row_g: vec4<f32>,
    row_b: vec4<f32>,
    // Added to the chroma texture coordinates: the half-size planes are sampled as if their
    // samples were centre-sited, and a left-sited stream is half a luma pixel off that.
    chroma_shift: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: VideoParams;
@group(0) @binding(1) var luma_plane: texture_2d<f32>;
@group(0) @binding(2) var cb_plane: texture_2d<f32>;
@group(0) @binding(3) var cr_plane: texture_2d<f32>;
@group(0) @binding(4) var plane_sampler: sampler;

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
    let chroma_uv = in.uv + params.chroma_shift.xy;
    let ycbcr = vec3<f32>(
        textureSample(luma_plane, plane_sampler, in.uv).r,
        textureSample(cb_plane, plane_sampler, chroma_uv).r,
        textureSample(cr_plane, plane_sampler, chroma_uv).r,
    ) - params.bias.xyz;
    let rgb = vec3<f32>(
        dot(params.row_r.xyz, ycbcr),
        dot(params.row_g.xyz, ycbcr),
        dot(params.row_b.xyz, ycbcr),
    );
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
