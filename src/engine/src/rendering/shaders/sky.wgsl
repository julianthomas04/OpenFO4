const SUN_DISC_RADIANCE_SCALE = 60.0;
const SUN_GLOW_EXPONENT = 64.0;
const SUN_GLOW_STRENGTH = 0.25;
const GROUND_BLEND_SINE = 0.15;
const SUN_HORIZON_FADE_SINE = 0.02;
const MIN_SUN_EDGE_WIDTH = 1e-6;

struct SkyVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> SkyVertexOutput {
    let ndc = fullscreen_triangle_ndc(vertex_index);
    var out: SkyVertexOutput;
    out.clip_position = vec4<f32>(ndc, 0.0, 1.0);
    out.ndc = ndc;
    return out;
}

// Infinite reverse-Z puts NDC z = 0 at infinity, so the unprojected point has
// w = 0 and its xyz is the view direction, free of camera-position cancellation.
fn view_ray(ndc: vec2<f32>) -> vec3<f32> {
    return normalize((frame.inv_view_proj * vec4<f32>(ndc, 0.0, 1.0)).xyz);
}

fn sky_gradient(sine_elevation: f32) -> vec3<f32> {
    let above = mix(frame.sky_horizon.rgb, frame.sky_zenith.rgb, sqrt(max(sine_elevation, 0.0)));
    let below = mix(frame.sky_horizon.rgb, frame.sky_ground.rgb, smoothstep(0.0, GROUND_BLEND_SINE, -sine_elevation));
    return select(below, above, sine_elevation >= 0.0);
}

@fragment
fn fs_main(in: SkyVertexOutput) -> @location(0) vec4<f32> {
    let dir = view_ray(in.ndc);
    let cos_sun = dot(dir, frame.to_sun.xyz);
    let edge_width = max(fwidth(cos_sun), MIN_SUN_EDGE_WIDTH);
    let disc = smoothstep(frame.to_sun.w - edge_width, frame.to_sun.w + edge_width, cos_sun);
    let glow = pow(max(cos_sun, 0.0), SUN_GLOW_EXPONENT);
    let sun_visibility = smoothstep(0.0, SUN_HORIZON_FADE_SINE, dir.y);
    let sun = frame.sun_disc_illuminance.rgb * (disc * SUN_DISC_RADIANCE_SCALE + glow * SUN_GLOW_STRENGTH) * sun_visibility;
    return vec4<f32>(sky_gradient(dir.y) + sun, 1.0);
}
