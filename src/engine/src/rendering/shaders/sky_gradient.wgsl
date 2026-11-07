// How much SkyUpper, SkyLower and Horizon each make of the sky at an elevation, fitted to the
// vanilla sky the probe measured from 1.5 to 89.5 degrees: the shares are the same at every
// azimuth and hour. Horizon fades out by 28 degrees, SkyUpper sets in at 8.25 degrees and takes
// over towards the zenith, and SkyLower is the rest, a band that peaks near 8 degrees.
const SKY_HORIZON_FADE_END_DEG = 28.0;
const SKY_HORIZON_FADE_POWER = 6.5;
const SKY_UPPER_ONSET_DEG = 8.25;
const SKY_UPPER_SPREAD_DEG = 11.0;
const SKY_UPPER_SHAPE = 0.85;

// x: SkyUpper, y: SkyLower, z: Horizon; they add up to 1.
fn sky_gradient_weights(sine_elevation: f32) -> vec3<f32> {
    let elevation = degrees(asin(clamp(sine_elevation, 0.0, 1.0)));
    let horizon = pow(1.0 - min(elevation / SKY_HORIZON_FADE_END_DEG, 1.0), SKY_HORIZON_FADE_POWER);
    let past_onset = max(elevation - SKY_UPPER_ONSET_DEG, 0.0) / SKY_UPPER_SPREAD_DEG;
    let upper = 1.0 - exp(-pow(past_onset, SKY_UPPER_SHAPE));
    let lower = max(1.0 - upper - horizon, 0.0);
    return vec3<f32>(upper, lower, horizon);
}
