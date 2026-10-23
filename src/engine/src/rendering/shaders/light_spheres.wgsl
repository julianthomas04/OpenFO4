// Light-sphere debug overlay: three great circles of each visible light's
// range, one instance per light. Depth-tested, never written (see
// debug_lines.wgsl). Mirrors SPHERE_VERTEX_COUNT in local_lights/light_spheres.rs.
const SPHERE_CIRCLE_SEGMENTS = 32u;
const SPHERE_CIRCLE_VERTICES = 64u;
const TAU = 6.28318531;
const OMNI_SPHERE_COLOR = vec3<f32>(1.0, 0.85, 0.2);
const DIRECTED_SPHERE_COLOR = vec3<f32>(0.2, 0.85, 1.0);
const SHADOWED_SPHERE_COLOR = vec3<f32>(1.0, 0.2, 1.0);

struct LightSphereVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) @interpolate(flat) color: vec3<f32>,
}

fn circle_point(circle: u32, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    switch circle {
        case 0u: { return vec3<f32>(c, s, 0.0); }
        case 1u: { return vec3<f32>(c, 0.0, s); }
        default: { return vec3<f32>(0.0, c, s); }
    }
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) light_index: u32) -> LightSphereVertexOutput {
    let light = visible_lights[light_index];
    let circle = vertex_index / SPHERE_CIRCLE_VERTICES;
    let line_vertex = vertex_index % SPHERE_CIRCLE_VERTICES;
    let point = line_vertex / 2u + line_vertex % 2u;
    let angle = f32(point) * TAU / f32(SPHERE_CIRCLE_SEGMENTS);
    let world = light.position + circle_point(circle, angle) * light.radius;

    var out: LightSphereVertexOutput;
    out.clip_position = frame.view_proj * vec4<f32>(world, 1.0);
    let directed = is_spot_light(light.flags) || is_hemisphere_light(light.flags);
    let unshadowed = select(OMNI_SPHERE_COLOR, DIRECTED_SPHERE_COLOR, directed);
    out.color = select(unshadowed, SHADOWED_SPHERE_COLOR, light.shadow_slot != SHADOW_SLOT_NONE);
    return out;
}

@fragment
fn fs_main(in: LightSphereVertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
