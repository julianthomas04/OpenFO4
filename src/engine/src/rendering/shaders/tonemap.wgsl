const AGX_MIN_EV = -12.47393;
const AGX_MAX_EV = 4.026069;
const DISPLAY_GAMMA = 2.2;
const DITHER_AMPLITUDE = 1.0 / 255.0;
const TONEMAP_FILMIC = 0u;
const TONEMAP_AGX = 1u;
const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);

// The filmic curve's fixed constants; its toe numerator is the image space's TonemapE.
const FILMIC_SHOULDER = 0.15;
const FILMIC_LINEAR_STRENGTH = 0.5;
const FILMIC_LINEAR_ANGLE = 0.1;
const FILMIC_TOE_STRENGTH = 0.2;
const FILMIC_TOE_DENOMINATOR = 0.3;
const FILMIC_WHITE = 11.2;
// Exposed scene light (a fog colour byte decoded with gamma 2.2 is its light) to the curve's input.
const FILMIC_INPUT_SCALE = 4.113;
// Contrast pivots about the display's middle: 0.5 once encoded.
const CONTRAST_PIVOT = 0.217638;

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
    let rec2020 = pow(max(display, vec3<f32>(0.0)), vec3<f32>(DISPLAY_GAMMA));
    return clamp(REC2020_TO_LINEAR_SRGB * rec2020, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn filmic_curve(x: vec3<f32>, toe: f32) -> vec3<f32> {
    let a = FILMIC_SHOULDER;
    let b = FILMIC_LINEAR_STRENGTH;
    let numerator = x * (a * x + FILMIC_LINEAR_ANGLE * b) + FILMIC_TOE_STRENGTH * toe;
    let denominator = x * (a * x + b) + FILMIC_TOE_STRENGTH * FILMIC_TOE_DENOMINATOR;
    return numerator / denominator - toe / FILMIC_TOE_DENOMINATOR;
}

// Per channel, linear display light from 0 to 1.
fn filmic(exposed: vec3<f32>, toe: f32) -> vec3<f32> {
    let white = filmic_curve(vec3<f32>(FILMIC_WHITE), toe);
    return clamp(filmic_curve(exposed * FILMIC_INPUT_SCALE, toe) / white, vec3<f32>(0.0), vec3<f32>(1.0));
}

// cinematic: x saturation, y brightness, z contrast, w tint amount; tint: linear colour.
fn apply_cinematic(picture: vec3<f32>, cinematic: vec4<f32>, tint: vec3<f32>) -> vec3<f32> {
    let luma = dot(picture, LUMA);
    let saturated = vec3<f32>(luma) + (picture - vec3<f32>(luma)) * cinematic.x;
    let tinted = mix(saturated, dot(saturated, LUMA) * tint, cinematic.w);
    let brightened = tinted * cinematic.y;
    let contrasted = vec3<f32>(CONTRAST_PIVOT) + (brightened - vec3<f32>(CONTRAST_PIVOT)) * cinematic.z;
    return clamp(contrasted, vec3<f32>(0.0), vec3<f32>(1.0));
}

fn tone_mapped(exposed: vec3<f32>) -> vec3<f32> {
    if (u32(frame.tonemap.x) == TONEMAP_AGX) {
        return display_to_linear_srgb(agx_display(exposed));
    }
    return filmic(exposed, frame.tonemap.y);
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
    let exposed = max(scene * exposure_state.exposure, vec3<f32>(0.0));
    let picture = apply_cinematic(tone_mapped(exposed), frame.cinematic, frame.tint.rgb);
    let dither = (interleaved_gradient_noise(in.clip_position.xy) - 0.5) * DITHER_AMPLITUDE;
    let display = pow(picture, vec3<f32>(1.0 / DISPLAY_GAMMA)) + dither;
    return vec4<f32>(srgb_to_linear(display), 1.0);
}
