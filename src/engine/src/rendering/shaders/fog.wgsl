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

// The picture `t` of the way from `fog`'s to `other`'s.
fn fog_towards(fog: Fog, other: Fog, t: f32) -> Fog {
    let opacity = mix(fog.opacity, other.opacity, t);
    if (opacity <= 0.0) {
        return Fog(vec3<f32>(0.0), 0.0);
    }
    return Fog((fog.color * (fog.opacity * (1.0 - t)) + other.color * (other.opacity * t)) / opacity, opacity);
}

fn fog_share(height: f32, x: f32) -> f32 {
    return mix(fog_band_share(height, frame.fog_near_band), fog_band_share(height, frame.fog_far_band), x);
}

fn fog_density(share: f32) -> f32 {
    return mix(1.0, frame.fog_params.z, share);
}

// Beyond Far: the far colours at the far band's share, as dense as the height leaves the fog.
fn fog_fill(height: f32) -> Fog {
    let share = fog_share(height, 1.0);
    let color = mix(frame.fog_far_color.rgb, frame.fog_far_high_color.rgb, share);
    return Fog(color, fog_opacity(1.0, fog_density(share)));
}

// Normal below Max, held at Max and ramping towards the fill near Far from Max on, the fill
// beyond Far.
fn fog_at(world_pos: vec3<f32>) -> Fog {
    let far = frame.fog_far_color.w;
    if (far <= frame.fog_near_color.w) {
        return Fog(vec3<f32>(0.0), 0.0);
    }
    let distance_to_camera = distance(frame.camera_pos.xyz, world_pos);
    let height = world_pos.y;
    if (distance_to_camera >= far) {
        return fog_fill(height);
    }
    let x = fog_colour_weight(distance_to_camera);
    let share = fog_share(height, x);
    let near_color = mix(frame.fog_near_color.rgb, frame.fog_near_high_color.rgb, share);
    let far_color = mix(frame.fog_far_color.rgb, frame.fog_far_high_color.rgb, share);
    let density = fog_density(share);
    let max_opacity = frame.fog_params.y;
    if (x < max_opacity) {
        let color = mix(near_color, far_color, x) * frame.fog_near_high_color.w;
        return Fog(color, fog_opacity(x, density));
    }
    let cap = Fog(mix(near_color, far_color, max_opacity), fog_opacity(max_opacity, density));
    return fog_towards(cap, fog_fill(height), fog_ramp(distance_to_camera, frame.fog_params.w, far));
}

fn apply_fog(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let fog = fog_at(world_pos);
    return mix(color, fog.color, fog.opacity);
}

// Share of a surface's own light that survives the fog.
fn fog_transmittance(world_pos: vec3<f32>) -> f32 {
    return 1.0 - fog_at(world_pos).opacity;
}
