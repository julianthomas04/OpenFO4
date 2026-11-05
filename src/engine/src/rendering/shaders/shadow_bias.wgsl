// The receiver bias of the world's shadow maps, shared by the sun and lamp lookups
// (lighting.wgsl, lamp_shadow.wgsl) and the loading art's lamp shadows. Needs shading_math.wgsl.

const MAX_SLOPE_BIAS_TANGENT = 10.0;
const MIN_NO_L = 1e-4;

// Receiver-side biases in texels of the shadow map at the receiver (x
// constant, y slope, z normal offset): constant and slope push the receiver
// towards the light, the normal offset along the geometric normal.
fn biased_receiver(world_pos: vec3<f32>, ng: vec3<f32>, l: vec3<f32>, texel: f32, bias: vec3<f32>) -> vec3<f32> {
    let n_dot_l = saturate(dot(ng, l));
    let slope = min(sqrt(1.0 - n_dot_l * n_dot_l) / max(n_dot_l, MIN_NO_L), MAX_SLOPE_BIAS_TANGENT);
    return world_pos
        + ng * (bias.z * texel * (1.0 - n_dot_l))
        + l * ((bias.x + bias.y * slope) * texel);
}
