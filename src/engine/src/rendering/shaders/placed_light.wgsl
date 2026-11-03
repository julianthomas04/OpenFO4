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
    return falloff_of_loss(max(record_loss(normalised, attenuation), window_loss(normalised)));
}

// The same two terms as `placed_light_falloff`, multiplied instead of taking the larger loss. This
// is an interpretation confirmed against a vanilla capture of the loading menu's lights, not
// documented data, so only the loading art's scene lights use it.
fn loading_scene_light_falloff(radius: f32, attenuation: vec3<f32>, distance: f32) -> f32 {
    let normalised = distance / radius;
    return falloff_of_loss(record_loss(normalised, attenuation))
        * falloff_of_loss(window_loss(normalised));
}

fn record_loss(normalised: f32, attenuation: vec3<f32>) -> f32 {
    let distance_power = min(pow_or_zero(normalised, attenuation.z), MAX_DISTANCE_POWER);
    return saturate(attenuation.x + attenuation.y * distance_power);
}

fn window_loss(normalised: f32) -> f32 {
    return normalised * normalised;
}

fn falloff_of_loss(loss: f32) -> f32 {
    return pow_or_zero(1.0 - saturate(loss), LIGHT_FALLOFF_POWER);
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
