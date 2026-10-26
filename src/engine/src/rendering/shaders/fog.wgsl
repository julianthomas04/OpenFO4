const FOG_MIN_RANGE = 0.001;
const FOG_INV_FOUR_PI = 0.0795774715;

struct Fog {
    color: vec3<f32>,
    opacity: f32,
}

// How far `height` is up a height band given as its bottom and reciprocal span; a zero span
// holds it at 0.
fn fog_height_factor(height: f32, band: vec2<f32>) -> f32 {
    return saturate((height - band.x) * band.y);
}

// The normalised Henyey-Greenstein phase function; mirrors `henyey_greenstein` in
// lighting/god_rays.rs.
fn fog_henyey_greenstein(cos_theta: f32, asymmetry: f32) -> f32 {
    let g = asymmetry;
    let denominator = pow(1.0 + g * g - 2.0 * g * clamp(cos_theta, -1.0, 1.0), 1.5);
    return (1.0 - g * g) * FOG_INV_FOUR_PI / denominator;
}

// The sun's light the fog scatters towards the eye looking at `world_pos`: a forward and a
// backward lobe around the sun and the isotropic air; mirrors `SunInScatter::radiance` in
// lighting/god_rays.rs. Zero without god rays, at any distance.
fn fog_sun_in_scatter(world_pos: vec3<f32>) -> vec3<f32> {
    let offset = world_pos - frame.camera_pos.xyz;
    let cos_to_sun = dot(offset, frame.fog_to_sun.xyz) / max(length(offset), FOG_MIN_RANGE);
    return frame.fog_sun_forward.rgb * fog_henyey_greenstein(cos_to_sun, frame.fog_sun_forward.w)
        + frame.fog_sun_back.rgb * fog_henyey_greenstein(cos_to_sun, frame.fog_sun_back.w)
        + frame.fog_sun_air.rgb * FOG_INV_FOUR_PI;
}

// Bethesda-style parametric fog. The fog factor grows with distance by the fog power up to the
// maximum opacity and picks the colour between near and far. Each colour turns towards its high
// colour over its height band, and the opacity scales towards the high density scale over the far
// band. The sun's in-scatter adds to the colour, so the opacity weights it like the rest of the
// fog. A zero maximum opacity disables it.
fn fog_at(world_pos: vec3<f32>) -> Fog {
    let max_opacity = frame.fog_params.y;
    if (max_opacity <= 0.0) {
        return Fog(vec3<f32>(0.0), 0.0);
    }
    let near = frame.fog_near_color.w;
    let far = frame.fog_far_color.w;
    let power = frame.fog_params.x;
    let distance_to_camera = distance(frame.camera_pos.xyz, world_pos);
    let t = saturate((distance_to_camera - near) / max(far - near, FOG_MIN_RANGE));
    let factor = min(pow(t, power), max_opacity);
    let near_height = fog_height_factor(world_pos.y, frame.fog_heights.xy);
    let far_height = fog_height_factor(world_pos.y, frame.fog_heights.zw);
    let near_color = mix(frame.fog_near_color.rgb, frame.fog_near_high_color.rgb, near_height);
    let far_color = mix(frame.fog_far_color.rgb, frame.fog_far_high_color.rgb, far_height);
    let density = mix(1.0, frame.fog_params.z, far_height);
    let color = mix(near_color, far_color, factor) + fog_sun_in_scatter(world_pos);
    return Fog(color, factor * density);
}

fn apply_fog(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let fog = fog_at(world_pos);
    return mix(color, fog.color, fog.opacity);
}

// Share of a surface's own light that survives the fog.
fn fog_transmittance(world_pos: vec3<f32>) -> f32 {
    return 1.0 - fog_at(world_pos).opacity;
}
