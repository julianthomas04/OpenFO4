// Skeleton overlay: world-space coloured line list drawn over the finished
// world pass. A fragment is hidden when the scene is nearer to the camera than
// the fragment by more than the reach of its joint (the distance from the joint
// to the mesh it drives), so a skeleton shows through its own mesh but not
// through other geometry standing in front of it.

// Surfaces the object rests on or is mounted to (floor, wall) sit just in front
// of joints near them; this slack keeps those joints from being hidden.
const CONTACT_TOLERANCE_MIN: f32 = 1.0;
const CONTACT_TOLERANCE_PER_DEPTH: f32 = 0.02;

@group(0) @binding(0) var scene_depth: texture_depth_2d;

struct OverlayOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) view_depth: f32,
    @location(2) reach: f32,
}

fn view_depth(world: vec3<f32>) -> f32 {
    return dot(world - frame.camera_pos.xyz, frame.camera_forward.xyz);
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) reach: f32,
) -> OverlayOut {
    var out: OverlayOut;
    out.clip = frame.view_proj * vec4<f32>(position, 1.0);
    out.color = color;
    out.view_depth = view_depth(position);
    out.reach = reach;
    return out;
}

// Reverse-Z: the cleared depth 0 means no geometry (sky), larger is nearer.
fn occluder_view_depth(pixel: vec2<f32>, depth: f32) -> f32 {
    let uv = pixel * frame.viewport.zw;
    let ndc = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let world = frame.inv_view_proj * ndc;
    return view_depth(world.xyz / world.w);
}

@fragment
fn fs_main(in: OverlayOut) -> @location(0) vec4<f32> {
    let depth = textureLoad(scene_depth, vec2<i32>(in.clip.xy), 0);
    let tolerance = in.reach + max(CONTACT_TOLERANCE_MIN, in.view_depth * CONTACT_TOLERANCE_PER_DEPTH);
    if depth > 0.0 && occluder_view_depth(in.clip.xy, depth) < in.view_depth - tolerance {
        discard;
    }
    return in.color;
}
