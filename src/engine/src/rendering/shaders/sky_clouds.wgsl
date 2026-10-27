const MIN_HAZE_SINE = 1e-6;

// Mirrors `CloudUniform` in post/night_sky/clouds.rs.
struct CloudUniform {
    // rgb: the radiance of a fully lit texel, w: the opacity of the layer.
    radiance_opacity: vec4<f32>,
    // xy: the texture coordinate offset of the scroll, z: the sine of the elevation at which the
    // horizon haze has cleared.
    offset_haze: vec4<f32>,
}

@group(3) @binding(0) var<uniform> cloud: CloudUniform;
@group(3) @binding(1) var cloud_texture: texture_2d<f32>;
@group(3) @binding(2) var cloud_sampler: sampler;

struct CloudVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) direction: vec3<f32>,
    @location(2) edge_opacity: f32,
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
) -> CloudVertexOutput {
    var out: CloudVertexOutput;
    // w = 0 centres the model on the camera and puts it at depth 0 under the reverse-Z
    // infinite projection, like the sky gradient and the stars.
    out.clip_position = frame.view_proj * vec4<f32>(position, 0.0);
    out.uv = uv + cloud.offset_haze.xy;
    out.direction = position;
    out.edge_opacity = color.a;
    return out;
}

// The cloud textures hold the shading of the cloud in RGB and its opacity in alpha; the alpha of
// the vertex colour fades a mesh out towards its edges. Towards the horizon the cloud takes the
// colour of the sky there, and below it nothing but that colour is left.
@fragment
fn fs_main(in: CloudVertexOutput) -> @location(0) vec4<f32> {
    let texel = textureSample(cloud_texture, cloud_sampler, in.uv);
    let haze = 1.0 - smoothstep(0.0, max(cloud.offset_haze.z, MIN_HAZE_SINE), normalize(in.direction).y);
    let radiance = mix(texel.rgb * cloud.radiance_opacity.rgb, frame.sky_horizon.rgb, haze);
    return vec4<f32>(radiance, texel.a * cloud.radiance_opacity.w * in.edge_opacity);
}
