// Mirrors `lighting/fog/model.rs`; the uniform fields are documented on `FogUniformFields`.

const FOG_MIN_RANGE = 0.001;

// What the fog does to a surface behind it: `mix(surface, color, opacity)`.
struct Fog {
    color: vec3<f32>,
    opacity: f32,
}

// 0 up to `start`, 1 from `end` on, linear between.
fn fog_ramp(distance_to_camera: f32, start: f32, end: f32) -> f32 {
    return saturate((distance_to_camera - start) / max(end - start, FOG_MIN_RANGE));
}

// A high density scale above 1 makes the fog denser than full.
fn fog_opacity(weight: f32, density: f32) -> f32 {
    return min(weight * density, 1.0);
}

fn fog_band_share(height: f32, band: vec4<f32>) -> f32 {
    return band.z * saturate(height * band.x + band.y);
}

// `ramp^power`: 0 up to Near, 1 at Far.
fn fog_colour_weight(distance_to_camera: f32) -> f32 {
    let ramp = fog_ramp(distance_to_camera, frame.fog_near_color.w, frame.fog_far_color.w);
    return select(0.0, pow(ramp, frame.fog_params.x), ramp > 0.0);
}

// `fog` followed by a blend of `t` towards full fog of `fill`.
fn fog_towards(fog: Fog, fill: vec3<f32>, t: f32) -> Fog {
    let opacity = fog.opacity + (1.0 - fog.opacity) * t;
    if (opacity <= 0.0) {
        return fog;
    }
    return Fog((fog.color * (fog.opacity * (1.0 - t)) + fill * t) / opacity, opacity);
}

// Normal below Max, held at Max and ramping to full fog towards Far from Max on, full fog of the
// far colour beyond Far.
fn fog_at(world_pos: vec3<f32>) -> Fog {
    let far = frame.fog_far_color.w;
    if (far <= frame.fog_near_color.w) {
        return Fog(vec3<f32>(0.0), 0.0);
    }
    let distance_to_camera = distance(frame.camera_pos.xyz, world_pos);
    let beyond_far = distance_to_camera >= far;
    let x = select(fog_colour_weight(distance_to_camera), 1.0, beyond_far);
    let height = world_pos.y;
    let near_share = fog_band_share(height, frame.fog_near_band);
    let share = mix(near_share, fog_band_share(height, frame.fog_far_band), x);
    let near_color = mix(frame.fog_near_color.rgb, frame.fog_near_high_color.rgb, share);
    let far_color = mix(frame.fog_far_color.rgb, frame.fog_far_high_color.rgb, share);
    if (beyond_far) {
        return Fog(far_color, 1.0);
    }
    let density = mix(1.0, frame.fog_params.z, share);
    let max_opacity = frame.fog_params.y;
    if (x < max_opacity) {
        let color = mix(near_color, far_color, x) * frame.fog_sunlight.w + frame.fog_sunlight.rgb;
        return Fog(color, fog_opacity(x, density));
    }
    let cap = Fog(mix(near_color, far_color, max_opacity), fog_opacity(max_opacity, density));
    return fog_towards(cap, far_color, fog_ramp(distance_to_camera, frame.fog_params.w, far));
}

fn apply_fog(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let fog = fog_at(world_pos);
    return mix(color, fog.color, fog.opacity);
}

// Share of a surface's own light that survives the fog.
fn fog_transmittance(world_pos: vec3<f32>) -> f32 {
    return 1.0 - fog_at(world_pos).opacity;
}
