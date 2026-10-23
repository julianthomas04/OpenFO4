// Placed-lamp shadows from the lamp atlas: one perspective tile along a
// spot's axis, or cube faces in the lamp basis. Mirrors shadows/lamps/geometry.rs.

const LAMP_SHADOW_FADE_POWER = 8.0;

// Face order of `LampBasis::face`: +forward, -forward, +up, -up, +side, -side.
fn lamp_face(lamp: ShadowLamp, to_receiver: vec3<f32>) -> u32 {
    if (lamp.params.z == 0.0) {
        return 0u;
    }
    let local = vec3<f32>(
        dot(to_receiver, lamp.axes[0].xyz),
        dot(to_receiver, lamp.axes[1].xyz),
        dot(to_receiver, lamp.axes[2].xyz),
    );
    let magnitude = abs(local);
    if (magnitude.x >= magnitude.y && magnitude.x >= magnitude.z) {
        return select(1u, 0u, local.x >= 0.0);
    }
    if (magnitude.y >= magnitude.z) {
        return select(3u, 2u, local.y >= 0.0);
    }
    return select(5u, 4u, local.z >= 0.0);
}

fn lamp_face_axis(lamp: ShadowLamp, face: u32) -> vec3<f32> {
    let sign = select(-1.0, 1.0, face % 2u == 0u);
    return lamp.axes[face / 2u].xyz * sign;
}

// Grid of hardware-filtered compare taps, kept inside the tile.
fn filtered_lamp_shadow(tile: vec4<f32>, uv: vec2<f32>, depth: f32) -> f32 {
    let taps = lamp_shadows.filter_taps.x;
    let texel_uv = 1.0 / vec2<f32>(textureDimensions(lamp_shadow_atlas));
    let lowest = tile.xy + texel_uv * 0.5;
    let highest = tile.xy + tile.z - texel_uv * 0.5;
    let center = f32(taps - 1u) * 0.5;
    var lit = 0.0;
    for (var y = 0u; y < taps; y = y + 1u) {
        for (var x = 0u; x < taps; x = x + 1u) {
            let offset = (vec2<f32>(f32(x), f32(y)) - center) * texel_uv;
            let tap = clamp(uv + offset, lowest, highest);
            lit += textureSampleCompareLevel(lamp_shadow_atlas, shadow_comparison_sampler, tap, depth);
        }
    }
    return lit / f32(taps * taps);
}

fn lamp_face_matrix(lamp: ShadowLamp, face: u32) -> mat4x4<f32> {
    switch face {
        case 0u: { return lamp.faces[0]; }
        case 1u: { return lamp.faces[1]; }
        case 2u: { return lamp.faces[2]; }
        case 3u: { return lamp.faces[3]; }
        case 4u: { return lamp.faces[4]; }
        default: { return lamp.faces[5]; }
    }
}

// 1 = fully lit. Receivers outside the stored faces are lit; the shadow fades
// out with the receiver's distance from the camera.
fn lamp_shadow(slot: u32, world_pos: vec3<f32>, ng: vec3<f32>, l: vec3<f32>) -> f32 {
    let lamp = lamp_shadows.lamps[slot];
    let to_receiver = world_pos - lamp.position.xyz;
    let face = lamp_face(lamp, to_receiver);
    let tile = lamp.tiles[face];
    let axis_distance = dot(to_receiver, lamp_face_axis(lamp, face));
    if (tile.z == 0.0 || axis_distance <= 0.0) {
        return 1.0;
    }
    let texel = axis_distance * lamp.params.y;
    let biased = biased_receiver(world_pos, ng, l, texel, lamp_shadows.bias.xyz);
    let clip = lamp_face_matrix(lamp, face) * vec4<f32>(biased, 1.0);
    if (clip.w <= 0.0) {
        return 1.0;
    }
    let ndc = clip.xyz / clip.w;
    let uv = ndc.xy * vec2<f32>(0.5, -0.5) + 0.5;
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))) {
        return 1.0;
    }
    let lit = filtered_lamp_shadow(tile, tile.xy + uv * tile.z, ndc.z);
    let from_camera = world_pos - frame.camera_pos.xyz;
    let fade = 1.0 - pow(saturate(dot(from_camera, from_camera) / lamp_shadows.bias.w), LAMP_SHADOW_FADE_POWER);
    return mix(1.0, lit, lamp.params.x * fade);
}
