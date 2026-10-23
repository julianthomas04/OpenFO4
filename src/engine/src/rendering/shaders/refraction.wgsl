// Heat-haze refraction: samples the opaque scene colour copy at a screen-space offset built
// from two scrolling normal maps. Mirrors `rendering/refraction/offset.rs`; every tuning value
// arrives through `Frame`.

struct Frame {
    view_proj: mat4x4<f32>,
    camera: vec4<f32>,
    viewport: vec4<f32>,       // xy pixels, zw reciprocal
    depth: vec4<f32>,          // near, reference depth, max gain, min view depth
    limits: vec4<f32>,         // max offset, soft fade, reject bias, replace offset
    far: vec4<f32>,            // x: sky view depth
}

struct Draw {
    model: mat4x4<f32>,
    normal_matrix: mat4x4<f32>,
    uv_a: vec4<f32>,           // offset.xy, scale.zw
    uv_b: vec4<f32>,
    params: vec4<f32>,         // strength, use vertex alpha
}

@group(0) @binding(0) var scene_colour: texture_2d<f32>;
@group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(0) @binding(2) var scene_sampler: sampler;

@group(1) @binding(0) var<uniform> frame: Frame;

@group(2) @binding(0) var normal_a: texture_2d<f32>;
@group(2) @binding(1) var normal_b: texture_2d<f32>;
@group(2) @binding(2) var normal_sampler: sampler;

@group(3) @binding(0) var<uniform> draw: Draw;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(4) colour: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world_position: vec3<f32>,
    @location(2) world_normal: vec3<f32>,
    @location(3) alpha: f32,
}

@vertex
fn vs_main(vin: VertexIn) -> VertexOut {
    let world = draw.model * vec4<f32>(vin.position, 1.0);
    var out: VertexOut;
    out.clip = frame.view_proj * world;
    out.uv = vin.uv;
    out.world_position = world.xyz;
    out.world_normal = (draw.normal_matrix * vec4<f32>(vin.normal, 0.0)).xyz;
    out.alpha = select(1.0, vin.colour.a, draw.params.y > 0.5);
    return out;
}

// Reverse-Z with an infinite far plane: depth = near / view depth.
fn view_depth_of(depth: f32) -> f32 {
    return select(frame.depth.x / depth, frame.far.x, depth <= 1.0e-9);
}

fn scene_view_depth_at(uv: vec2<f32>) -> f32 {
    let size = vec2<i32>(frame.viewport.xy);
    let texel = clamp(vec2<i32>(uv * frame.viewport.xy), vec2<i32>(0), size - vec2<i32>(1));
    return view_depth_of(textureLoad(scene_depth, texel, 0));
}

fn layer_normal_xy(tex: texture_2d<f32>, uv_transform: vec4<f32>, uv: vec2<f32>) -> vec2<f32> {
    let texel = textureSample(tex, normal_sampler, uv * uv_transform.zw + uv_transform.xy);
    return texel.rg * 2.0 - vec2<f32>(1.0);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Both layers are sampled before any selection so the derivatives stay uniform.
    let combined = (layer_normal_xy(normal_a, draw.uv_a, in.uv)
        + layer_normal_xy(normal_b, draw.uv_b, in.uv)) * 0.5;

    let card_depth = view_depth_of(in.clip.z);
    let pixel_uv = in.clip.xy * frame.viewport.zw;
    let scene_depth_here = scene_view_depth_at(pixel_uv);

    let to_camera = normalize(frame.camera.xyz - in.world_position);
    let facing = abs(dot(normalize(in.world_normal), to_camera));
    let gain = min(frame.depth.y / max(card_depth, frame.depth.w), frame.depth.z);
    let soft = clamp((scene_depth_here - card_depth) / frame.limits.y, 0.0, 1.0);
    let magnitude = draw.params.x * in.alpha * facing * gain * soft;
    var offset = vec2<f32>(combined.x, -combined.y) * magnitude;
    let length_sq = dot(offset, offset);
    if (length_sq > frame.limits.x * frame.limits.x) {
        offset = offset * (frame.limits.x * inverseSqrt(length_sq));
    }

    let displaced_uv = pixel_uv + offset;
    let in_front = scene_view_depth_at(displaced_uv) < card_depth - frame.limits.z;
    offset = select(offset, vec2<f32>(0.0), in_front);

    let colour = textureSampleLevel(scene_colour, scene_sampler, pixel_uv + offset, 0.0);
    let weight = clamp(length(offset) / frame.limits.w, 0.0, 1.0);
    return vec4<f32>(colour.rgb, weight);
}
