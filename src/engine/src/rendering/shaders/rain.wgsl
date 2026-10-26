// Rain: streaks that fall through a box around the camera, and splashes standing on the highest
// roof or ground of the occlusion map. Every drop and splash is worked out here from its
// instance index; the CPU hands over one uniform and no per drop data.

// Mirrors `RainUniform` in rain/uniform.rs.
struct RainUniform {
    // xyz: unit axis up a streak, w: the streak's length.
    along: vec4<f32>,
    // xyz: unit axis across a streak, w: the streak's width.
    across: vec4<f32>,
    // xyz: where the lattice of drops lies relative to the camera, w: the box edge.
    lattice: vec4<f32>,
    // rgb: the tint of the rain.
    tint: vec4<f32>,
    // x, y: columns and rows of streak subtextures, z: frames of a splash, w: the splash's edge.
    atlas: vec4<f32>,
    // xy: the corner of the occlusion map with the smallest x and z, z: its edge, w: 1 once it is drawn.
    window: vec4<f32>,
    // x: the height depth 0 of the map stands for, y: the span of heights its depth covers,
    // z: 1 where roofs shelter from the rain.
    heights: vec4<f32>,
    // x, y: the distances over which a drop near the camera fades in, z: the share of the box
    // edge a drop fades out from, w: the same for a splash.
    fades: vec4<f32>,
    // x: how opaque the alpha of the streak texture is drawn, y: the same for the splash atlas,
    // z: the least a drop's brightness varies to.
    looks: vec4<f32>,
    // x: how far through its cycle the splash phase is, y: the splash lattice edge, zw: where
    // that lattice lies relative to the camera (x and z).
    splash: vec4<f32>,
    // x: the splash cycles begun.
    splash_cycle: vec4<u32>,
}

@group(0) @binding(0) var<uniform> rain: RainUniform;
@group(0) @binding(1) var streak_texture: texture_2d<f32>;
@group(0) @binding(2) var rain_sampler: sampler;
@group(0) @binding(3) var splash_texture: texture_2d<f32>;
@group(0) @binding(4) var occlusion_map: texture_depth_2d;

// Streaks leaning flatter than this to the horizon count as this steep when roofs cut them.
const LEAST_UP = 0.001;
const NO_HEIGHT = -1.0e30;
// A position outside the clip volume: the drop or splash is not drawn.
const CULLED = vec4<f32>(0.0, 0.0, 2.0, 1.0);
// The random numbers each drop or splash draws.
const STREAMS = 8u;

fn pcg(value: u32) -> u32 {
    let state = value * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

fn unit_float(bits: u32) -> f32 {
    return f32(bits >> 8u) * (1.0 / 16777216.0);
}

// Number `stream` of the drop `id`, 0 up to 1.
fn random(id: u32, stream: u32) -> f32 {
    return unit_float(pcg(id * STREAMS + stream));
}

// The same, new in every cycle.
fn cycle_random(id: u32, cycle: u32, stream: u32) -> f32 {
    return unit_float(pcg(pcg(id * STREAMS + stream) + cycle * 2654435769u));
}

// Where the position `place` of a lattice falls relative to the camera: the lattice is shifted
// by `shift` and every cell repeats, so the nearest copy to the camera is the one drawn.
fn wrap_around_camera(place: vec3<f32>, shift: vec3<f32>, edge: f32) -> vec3<f32> {
    let shifted = place * edge + shift;
    return shifted - edge * floor(shifted / edge + 0.5);
}

// Mirrors `OcclusionWindow::uv`.
fn occlusion_uv(xz: vec2<f32>) -> vec2<f32> {
    return (xz - rain.window.xy) / rain.window.z;
}

// Mirrors `OcclusionWindow::height_from_depth`.
fn occlusion_height(depth: f32) -> f32 {
    return select(NO_HEIGHT, rain.heights.x + depth * rain.heights.y, depth > 0.0);
}

// The height of the highest caster over `xz`; NO_HEIGHT where there is none or the spot lies
// beyond the map.
fn highest_caster(xz: vec2<f32>) -> f32 {
    let uv = occlusion_uv(xz);
    if any(uv < vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0)) {
        return NO_HEIGHT;
    }
    let texel = vec2<i32>(uv * vec2<f32>(textureDimensions(occlusion_map)));
    return occlusion_height(textureLoad(occlusion_map, texel, 0));
}

// 1 at the middle of a box, 0 at its edge, where a drop leaves one side and enters the other.
fn box_fade(offset: vec3<f32>, edge: f32, fade_from: f32) -> f32 {
    let reach = max(max(abs(offset.x), abs(offset.y)), abs(offset.z)) / (0.5 * edge);
    return 1.0 - smoothstep(fade_from, 1.0, reach);
}

struct DropOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world: vec3<f32>,
    @location(2) fade: f32,
}

// The four corners of a strip quad: x runs 0 to 1 across, y 0 to 1 up.
fn corner_of(vertex: u32) -> vec2<f32> {
    return vec2<f32>(f32(vertex & 1u), f32(vertex >> 1u));
}

@vertex
fn vs_streak(@builtin(vertex_index) vertex: u32, @builtin(instance_index) drop: u32) -> DropOut {
    var out: DropOut;
    out.clip = CULLED;

    let edge = rain.lattice.w;
    let place = vec3<f32>(random(drop, 0u), random(drop, 1u), random(drop, 2u));
    let offset = wrap_around_camera(place, rain.lattice.xyz, edge);
    let centre = frame.camera_pos.xyz + offset;

    // The streak's lower end stops at the roof over it, and a streak wholly under one is gone.
    // Positions along the streak are in lengths from its middle.
    var lowest = -0.5;
    if rain.heights.z > 0.5 {
        let roof = highest_caster(centre.xz);
        let steep = max(rain.along.y, LEAST_UP) * rain.along.w;
        lowest = max(lowest, (roof - centre.y) / steep);
        if lowest >= 0.5 {
            return out;
        }
    }

    let corner = corner_of(vertex);
    let along = mix(lowest, 0.5, corner.y);
    let world = centre
        + rain.across.xyz * ((corner.x - 0.5) * rain.across.w)
        + rain.along.xyz * (along * rain.along.w);

    let columns = rain.atlas.x;
    let rows = rain.atlas.y;
    let subtexture = floor(random(drop, 3u) * columns * rows);
    let row = floor(subtexture / columns);
    let column = subtexture - row * columns;
    let across = select(corner.x, 1.0 - corner.x, random(drop, 4u) > 0.5);

    let near_fade = smoothstep(rain.fades.x, rain.fades.y, length(offset));
    let variation = mix(rain.looks.z, 1.0, random(drop, 5u));

    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.uv = (vec2<f32>(across, 0.5 - along) + vec2<f32>(column, row)) / vec2<f32>(columns, rows);
    out.world = world;
    out.fade = near_fade * variation * box_fade(offset, edge, rain.fades.z);
    return out;
}

@fragment
fn fs_streak(in: DropOut) -> @location(0) vec4<f32> {
    let texel = textureSample(streak_texture, rain_sampler, in.uv);
    let colour = apply_fog(texel.rgb * rain.tint.rgb, in.world);
    return vec4<f32>(colour, texel.a * rain.looks.x * in.fade);
}

@vertex
fn vs_splash(@builtin(vertex_index) vertex: u32, @builtin(instance_index) splash: u32) -> DropOut {
    var out: DropOut;
    out.clip = CULLED;
    if rain.window.w < 0.5 {
        return out;
    }

    // Each splash starts its cycle at a time of its own, and lands somewhere new in every cycle.
    let phase = rain.splash.x + random(splash, 0u);
    let cycle = rain.splash_cycle.x + u32(floor(phase));
    let place = vec3<f32>(cycle_random(splash, cycle, 1u), 0.0, cycle_random(splash, cycle, 2u));
    let edge = rain.splash.y;
    let offset = wrap_around_camera(place, vec3<f32>(rain.splash.z, 0.0, rain.splash.w), edge);
    let xz = frame.camera_pos.xz + offset.xz;
    let ground = highest_caster(xz);
    if ground < 0.5 * NO_HEIGHT {
        return out;
    }

    let base = vec3<f32>(xz.x, ground, xz.y);
    let to_camera = frame.camera_pos.xyz - base;
    let flat_to_camera = vec3<f32>(to_camera.x, 0.0, to_camera.z);
    let right = select(
        vec3<f32>(1.0, 0.0, 0.0),
        normalize(cross(vec3<f32>(0.0, 1.0, 0.0), flat_to_camera)),
        dot(flat_to_camera, flat_to_camera) > 1.0e-6,
    );
    let corner = corner_of(vertex);
    let size = rain.atlas.w;
    let world = base + right * ((corner.x - 0.5) * size) + vec3<f32>(0.0, corner.y * size, 0.0);

    let frames = rain.atlas.z;
    let frame_index = min(floor(fract(phase) * frames), frames - 1.0);

    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.uv = vec2<f32>((frame_index + corner.x) / frames, 1.0 - corner.y);
    out.world = world;
    out.fade = box_fade(vec3<f32>(offset.x, 0.0, offset.z), edge, rain.fades.w);
    return out;
}

@fragment
fn fs_splash(in: DropOut) -> @location(0) vec4<f32> {
    let texel = textureSample(splash_texture, rain_sampler, in.uv);
    let colour = apply_fog(texel.rgb * rain.tint.rgb, in.world);
    return vec4<f32>(colour, saturate(texel.a * rain.looks.y) * in.fade);
}
