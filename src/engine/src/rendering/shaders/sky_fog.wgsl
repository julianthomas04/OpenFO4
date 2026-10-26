// Sine of the elevation at which the sky has cleared of the fog. A heuristic: no public source
// says how far above the horizon the fog reaches into the sky.
const SKY_FOG_CLEAR_SINE = 0.2;

// The share of the fog the sky takes at `sine_elevation`: all of it at and below the horizon,
// none from SKY_FOG_CLEAR_SINE up.
fn sky_fog_weight(sine_elevation: f32) -> f32 {
    return 1.0 - smoothstep(0.0, SKY_FOG_CLEAR_SINE, sine_elevation);
}

// `radiance` of the sky in the unit `direction`, turned towards the fog of a point at the fog's
// far distance that way, so the horizon meets the fogged terrain without a seam.
fn apply_sky_fog(radiance: vec3<f32>, direction: vec3<f32>) -> vec3<f32> {
    let fog = fog_at(frame.camera_pos.xyz + direction * frame.fog_far_color.w);
    return mix(radiance, fog.color, fog.opacity * sky_fog_weight(direction.y));
}
