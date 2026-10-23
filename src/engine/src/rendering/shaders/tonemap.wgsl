const AGX_MIN_EV = -12.47393;
const AGX_MAX_EV = 4.026069;
const AGX_DISPLAY_GAMMA = 2.2;
const DITHER_AMPLITUDE = 1.0 / 255.0;
const TONEMAP_REINHARD = 0u;
const TONEMAP_AGX = 1u;
const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);
const MIN_LUMA = 1e-6;

const LINEAR_SRGB_TO_REC2020 = mat3x3<f32>(
    vec3<f32>(0.6274, 0.0691, 0.0164),
    vec3<f32>(0.3293, 0.9195, 0.0880),
    vec3<f32>(0.0433, 0.0113, 0.8956),
);
const REC2020_TO_LINEAR_SRGB = mat3x3<f32>(
    vec3<f32>(1.6605, -0.1246, -0.0182),
    vec3<f32>(-0.5876, 1.1329, -0.1006),
    vec3<f32>(-0.0728, -0.0083, 1.1187),
);
const AGX_INSET = mat3x3<f32>(
    vec3<f32>(0.856627153315983, 0.137318972929847, 0.11189821299995),
    vec3<f32>(0.0951212405381588, 0.761241990602591, 0.0767994186031903),
    vec3<f32>(0.0482516061458583, 0.101439036467562, 0.811302368396859),
);
const AGX_OUTSET = mat3x3<f32>(
    vec3<f32>(1.1271005818144368, -0.1413297634984383, -0.14132976349843826),
    vec3<f32>(-0.11060664309660323, 1.157823702216272, -0.11060664309660294),
    vec3<f32>(-0.016493938717834573, -0.016493938717834257, 1.2519364065950405),
);

struct ExposureState {
    adapted_luminance: f32,
    exposure: f32,
}

@group(0) @binding(0) var hdr_scene: texture_2d<f32>;
@group(0) @binding(1) var<storage, read> exposure_state: ExposureState;

struct TonemapVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> TonemapVertexOutput {
    var out: TonemapVertexOutput;
    out.clip_position = vec4<f32>(fullscreen_triangle_ndc(vertex_index), 0.0, 1.0);
    return out;
}

fn agx_default_contrast(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

// Returns the display-encoded (gamma 2.2) AgX image with the default look.
fn agx_display(scene_linear: vec3<f32>) -> vec3<f32> {
    var c = AGX_INSET * (LINEAR_SRGB_TO_REC2020 * scene_linear);
    c = log2(max(c, vec3<f32>(1e-10)));
    c = clamp((c - AGX_MIN_EV) / (AGX_MAX_EV - AGX_MIN_EV), vec3<f32>(0.0), vec3<f32>(1.0));
    return AGX_OUTSET * agx_default_contrast(c);
}

fn display_to_linear_srgb(display: vec3<f32>) -> vec3<f32> {
    let rec2020 = pow(max(display, vec3<f32>(0.0)), vec3<f32>(AGX_DISPLAY_GAMMA));
    return clamp(REC2020_TO_LINEAR_SRGB * rec2020, vec3<f32>(0.0), vec3<f32>(1.0));
}

// Extended Reinhard on luminance, T(L) = L (p L + 1) / (L + 1) with p = 1 / white^2;
// chroma is kept by scaling the colour with T(L) / L.
fn reinhard_linear(scene_linear: vec3<f32>, white_point: f32) -> vec3<f32> {
    let luma = max(dot(scene_linear, LUMA), MIN_LUMA);
    let p = 1.0 / (white_point * white_point);
    let mapped = luma * (p * luma + 1.0) / (luma + 1.0);
    return scene_linear * (mapped / luma);
}

fn saturate_from_luma(color: vec3<f32>, saturation: f32) -> vec3<f32> {
    let luma = dot(color, LUMA);
    return max(vec3<f32>(luma) + (color - vec3<f32>(luma)) * saturation, vec3<f32>(0.0));
}

fn linear_to_srgb(linear: vec3<f32>) -> vec3<f32> {
    let c = clamp(linear, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3<f32>(0.0031308));
}

fn srgb_to_linear(srgb: vec3<f32>) -> vec3<f32> {
    let c = clamp(srgb, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

fn interleaved_gradient_noise(pixel: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
}

@fragment
fn fs_main(in: TonemapVertexOutput) -> @location(0) vec4<f32> {
    let scene = textureLoad(hdr_scene, vec2<i32>(in.clip_position.xy), 0).rgb;
    if (debug_view() != 0u) {
        return vec4<f32>(scene, 1.0);
    }
    let dither = (interleaved_gradient_noise(in.clip_position.xy) - 0.5) * DITHER_AMPLITUDE;
    let exposed = scene * exposure_state.exposure;
    if (u32(frame.tonemap.x) == TONEMAP_AGX) {
        let display = agx_display(exposed) + dither;
        return vec4<f32>(display_to_linear_srgb(display), 1.0);
    }
    let mapped = saturate_from_luma(reinhard_linear(exposed, frame.tonemap.y), frame.tonemap.z);
    return vec4<f32>(srgb_to_linear(linear_to_srgb(mapped) + dither), 1.0);
}
