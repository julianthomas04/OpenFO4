// The part of a placed point light that depends only on where it is: its distance falloff and the
// terminator that hides the shading-normal leak. Needs only shading_math.wgsl. Shared by the
// world's lighting (lighting.wgsl) and the loading art's scene lights (loading_art.wgsl).

const TERMINATOR_SHARPNESS = 8.0;
const MIN_LIGHT_DISTANCE = 1e-4;
const LIGHT_FALLOFF_POWER = 2.2;
const MAX_DISTANCE_POWER = 1e30;

// Hides the shading-normal light leak past the geometric terminator.
fn terminator(ng: vec3<f32>, l: vec3<f32>) -> f32 {
    return saturate(dot(ng, l) * TERMINATOR_SHARPNESS);
}

// Distance falloff of a placed light of `radius` and record `attenuation` (constant, scalar,
// exponent), `distance` and `radius` in engine units: the share of light lost, raised to a power.
// It is the larger of the light's own loss (its constant plus scalar times the normalised distance
// raised to its exponent) and the squared normalised distance, so the falloff never exceeds
// (1 - normalised^2)^power and reaches 0 with zero slope at the radius whatever the record says.
// Zero from the radius outwards.
fn placed_light_falloff(radius: f32, attenuation: vec3<f32>, distance: f32) -> f32 {
    let normalised = distance / radius;
    let window_loss = normalised * normalised;
    let distance_power = min(pow_or_zero(normalised, attenuation.z), MAX_DISTANCE_POWER);
    let record_loss = saturate(attenuation.x + attenuation.y * distance_power);
    return pow_or_zero(1.0 - saturate(max(record_loss, window_loss)), LIGHT_FALLOFF_POWER);
}

const MIN_CONE_COSINE_SPAN = 1e-6;

// Falloff of a spot light without a lamp shadow: the cosine of the angle off
// the axis rescaled to 1 on the axis and 0 at the cone edge, raised to the
// exponent. The rescaled cosine stays 0 past the edge, so the result is 0
// outside the cone whatever `cos_outer` is.
fn spot_cosine_falloff(cos_axis: f32, cos_outer: f32, exponent: f32) -> f32 {
    let inside = saturate(1.0 - (1.0 - cos_axis) / max(1.0 - cos_outer, MIN_CONE_COSINE_SPAN));
    return pow_or_zero(inside, max(exponent, 0.0));
}
