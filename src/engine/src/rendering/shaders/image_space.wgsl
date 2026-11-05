// The colour stages of an image space, shared by the world's tonemap and the loading screen's
// colour pass.

const LUMA = vec3<f32>(0.2126, 0.7152, 0.0722);
// Contrast pivots about the display's middle: 0.5 once encoded.
const CONTRAST_PIVOT = 0.217638;

// cinematic: x saturation, y brightness, z contrast, w tint amount; tint: linear colour.
fn apply_cinematic(picture: vec3<f32>, cinematic: vec4<f32>, tint: vec3<f32>) -> vec3<f32> {
    let luma = dot(picture, LUMA);
    let saturated = vec3<f32>(luma) + (picture - vec3<f32>(luma)) * cinematic.x;
    let tinted = mix(saturated, dot(saturated, LUMA) * tint, cinematic.w);
    let brightened = tinted * cinematic.y;
    let contrasted = vec3<f32>(CONTRAST_PIVOT) + (brightened - vec3<f32>(CONTRAST_PIVOT)) * cinematic.z;
    return clamp(contrasted, vec3<f32>(0.0), vec3<f32>(1.0));
}

// Where an encoded display colour samples a colour LUT of `size` texels a side: grid point i of
// a side stands for display level i / (size - 1) and sits at its texel's centre.
fn colour_lut_coordinate(display: vec3<f32>, size: f32) -> vec3<f32> {
    return (clamp(display, vec3<f32>(0.0), vec3<f32>(1.0)) * (size - 1.0) + 0.5) / size;
}

fn srgb_to_linear(srgb: vec3<f32>) -> vec3<f32> {
    let c = clamp(srgb, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(linear: vec3<f32>) -> vec3<f32> {
    let c = clamp(linear, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3<f32>(0.0031308));
}
