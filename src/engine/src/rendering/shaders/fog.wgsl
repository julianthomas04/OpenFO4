const FOG_MIN_RANGE = 0.001;

struct Fog {
    color: vec3<f32>,
    opacity: f32,
}

// Bethesda-style parametric fog. A zero maximum opacity disables it.
fn fog_at(world_pos: vec3<f32>) -> Fog {
    let near = frame.fog_near_color.w;
    let far = frame.fog_far_color.w;
    let power = frame.fog_params.x;
    let max_opacity = frame.fog_params.y;
    let distance_to_camera = distance(frame.camera_pos.xyz, world_pos);
    let t = saturate((distance_to_camera - near) / max(far - near, FOG_MIN_RANGE));
    return Fog(mix(frame.fog_near_color.rgb, frame.fog_far_color.rgb, t), min(pow(t, power), max_opacity));
}

fn apply_fog(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let fog = fog_at(world_pos);
    return mix(color, fog.color, fog.opacity);
}

// Share of a surface's own light that survives the fog.
fn fog_transmittance(world_pos: vec3<f32>) -> f32 {
    return 1.0 - fog_at(world_pos).opacity;
}
