// ---------- Grass ----------
// Instanced blades of one grass model: one `GrassBlade` per instance (vertex
// buffer 1), placed on the terrain by the cell loader. The vertex stage adapts
// the engine's instance and vertex attributes to the register interface of the
// install-time translated instancing shader (`grass_vs`) and converts its
// outputs back; the fragment stage shades with the object path.
//
// Adaptors: engine space is Y up in units of 10 game units, the translated
// shader works in Bethesda space (Z up, game units). `ENGINE_TO_BETHESDA` is the
// change of axes, positions additionally scale by `GAME_UNITS_PER_ENGINE_UNIT`.
// The view rotation registers cb12[0..2] are the identity, so the shader's
// output rows o1..o3 hold the bitangent (x), tangent (y) and normal (z) columns
// of the leaned frame in Bethesda axes; the clip registers cb12[8..11] are the
// engine view-projection composed with the camera-relative Bethesda to engine
// map, so the translated clip output is the engine's clip position.

const GAME_UNITS_PER_ENGINE_UNIT = 10.0;
const ENGINE_UNITS_PER_GAME_UNIT = 0.1;
const ENGINE_TO_BETHESDA = mat3x3<f32>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, -1.0, 0.0));
const BETHESDA_TO_ENGINE = mat3x3<f32>(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, -1.0), vec3<f32>(0.0, 1.0, 0.0));
const MIN_NORMAL_LENGTH_SQ = 1.0e-12;

// Neutral values for the secondary wind inputs the engine has no data for yet, so only the
// primary sway acts. The wave time is already scaled per batch, hence the scale of one.
const FLUTTER_DISABLED = 0.0;
const BEND_TERM_DISABLED_DIVISOR = 1.0e12;
const WAVE_TIME_UNSCALED = 1.0;

const BATCH_UNIFORM_SCALING = 1u;
const BATCH_UNTINTED = 2u;

struct GrassFrame {
    // x: wind heading in the Bethesda XY plane (radians), y: sway magnitude
    // min, z: max (game units), w: wave frequency multiplier.
    wind: vec4<f32>,
    // x: fade start, y: fade range (engine units).
    fade: vec4<f32>,
}

struct GrassBatch {
    material_id: u32,
    // Includes the pixel shader's 4/255 alpha floor.
    alpha_cutoff: f32,
    // Radians per second of the wave at unit wind frequency (the GRAS wave
    // period times 2*pi/600).
    wave_rate: f32,
    flags: u32,
}

@group(3) @binding(0) var<uniform> grass_frame: GrassFrame;
@group(3) @binding(1) var<uniform> grass_batch: GrassBatch;

// Constant registers of the translated shader, filled per invocation.
var<private> grass_cb1: array<vec4<f32>, 1>;
var<private> grass_cb2: array<vec4<f32>, 18>;
var<private> grass_cb12: array<vec4<f32>, 37>;

struct GrassVertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) tangent: vec4<f32>,
    // rgb: tint, a: sway weight.
    @location(4) color: vec4<f32>,
    @location(5) blade_position: vec3<f32>,
    // Signed height-range factor k.
    @location(6) blade_height: f32,
    // The blade's local +X and +Y (Bethesda Z) axes in engine space.
    @location(7) blade_right: vec3<f32>,
    @location(8) blade_tint: f32,
    @location(9) blade_up: vec3<f32>,
}

struct GrassVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) world_tangent: vec3<f32>,
    @location(3) tangent_w: f32,
    @location(4) world_pos: vec3<f32>,
    @location(5) tint: vec3<f32>,
    @location(6) fade: f32,
}

fn unit_or(v: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    return select(v * inverseSqrt(dot(v, v)), fallback, dot(v, v) < MIN_NORMAL_LENGTH_SQ);
}

fn perpendicular_to(n: vec3<f32>) -> vec3<f32> {
    let axis = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.x) > 0.9);
    return normalize(cross(n, axis));
}

// Rows of the map from a camera-relative Bethesda position to engine clip space.
fn fill_clip_registers(camera: vec3<f32>) {
    let to_engine = mat4x4<f32>(
        vec4<f32>(ENGINE_UNITS_PER_GAME_UNIT, 0.0, 0.0, 0.0),
        vec4<f32>(0.0, 0.0, -ENGINE_UNITS_PER_GAME_UNIT, 0.0),
        vec4<f32>(0.0, ENGINE_UNITS_PER_GAME_UNIT, 0.0, 0.0),
        vec4<f32>(camera, 1.0),
    );
    let m = frame.view_proj * to_engine;
    for (var row = 0; row < 4; row++) {
        grass_cb12[8 + row] = vec4<f32>(m[0][row], m[1][row], m[2][row], m[3][row]);
    }
}

fn fill_grass_registers() {
    let camera = frame.camera_pos.xyz;
    let camera_bethesda = ENGINE_TO_BETHESDA * camera * GAME_UNITS_PER_ENGINE_UNIT;
    grass_cb12[0] = vec4<f32>(1.0, 0.0, 0.0, 0.0);
    grass_cb12[1] = vec4<f32>(0.0, 1.0, 0.0, 0.0);
    grass_cb12[2] = vec4<f32>(0.0, 0.0, 1.0, 0.0);
    grass_cb12[35] = vec4<f32>(camera_bethesda, 0.0);
    // Blades do not move between frames, so the previous-frame camera equals the current one.
    grass_cb12[36] = vec4<f32>(camera_bethesda, 0.0);
    fill_clip_registers(camera);

    grass_cb1[0] = vec4<f32>(0.0, 0.0, 1.0, 1.0);

    // Current and previous world matrices (rows, translation in w): identity.
    for (var row = 0; row < 3; row++) {
        var axis = vec4<f32>(0.0);
        axis[row] = 1.0;
        grass_cb2[row] = axis;
        grass_cb2[4 + row] = axis;
    }
    grass_cb2[3] = vec4<f32>(0.0, 0.0, 0.0, 1.0);

    // Without the uniform scaling flag only the blade's height axis scales.
    let uniform_scaling = (grass_batch.flags & BATCH_UNIFORM_SCALING) != 0u;
    grass_cb2[8] = vec4<f32>(select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0), uniform_scaling), 0.0);
    grass_cb2[9] = vec4<f32>(0.0, 0.0, grass_frame.fade.x * GAME_UNITS_PER_ENGINE_UNIT, grass_frame.fade.y * GAME_UNITS_PER_ENGINE_UNIT);
    let wave_time = frame.time_water.x * grass_batch.wave_rate;
    grass_cb2[11] = vec4<f32>(grass_frame.wind.x, 0.0, wave_time, wave_time);
    grass_cb2[12] = vec4<f32>(grass_frame.wind.y, grass_frame.wind.z, grass_frame.wind.w, 0.0);
    // cb2[13..16] stay zero: the engine has no data for displacement by the player and actors.
    grass_cb2[17] = vec4<f32>(BEND_TERM_DISABLED_DIVISOR, 0.0, FLUTTER_DISABLED, WAVE_TIME_UNSCALED);
}

@vertex
fn vs_grass(vertex: GrassVertexInput) -> GrassVertexOutput {
    fill_grass_registers();

    let has_tangent = vertex.tangent.w != 0.0;
    let normal = ENGINE_TO_BETHESDA * unit_or(vertex.normal, vec3<f32>(0.0, 1.0, 0.0));
    // A missing tangent (tangent_w == 0) gets a stand-in; the pixel stage ignores it.
    let tangent = ENGINE_TO_BETHESDA * unit_or(select(perpendicular_to(vertex.normal), vertex.tangent.xyz, has_tangent), vec3<f32>(1.0, 0.0, 0.0));
    let bitangent = cross(normal, tangent) * select(1.0, vertex.tangent.w, has_tangent);

    // The blade's rotation in Bethesda axes, rows packed into three registers.
    let right = vertex.blade_right;
    let up = vertex.blade_up;
    let rotation = ENGINE_TO_BETHESDA * mat3x3<f32>(right, up, cross(right, up)) * BETHESDA_TO_ENGINE;
    let row0 = vec3<f32>(rotation[0][0], rotation[1][0], rotation[2][0]);
    let row1 = vec3<f32>(rotation[0][1], rotation[1][1], rotation[2][1]);
    let row2 = vec3<f32>(rotation[0][2], rotation[1][2], rotation[2][2]);

    var input: grass_vs_in;
    input.v0 = vec4<f32>(ENGINE_TO_BETHESDA * vertex.position * GAME_UNITS_PER_ENGINE_UNIT, bitangent.x);
    input.v1 = vec4<f32>(vertex.tex_coords, 0.0, 0.0);
    input.v2 = vec4<f32>(normal * 0.5 + 0.5, bitangent.y * 0.5 + 0.5);
    input.v3 = vec4<f32>(tangent * 0.5 + 0.5, bitangent.z * 0.5 + 0.5);
    input.v4 = vertex.color;
    input.v5 = vec4<f32>(ENGINE_TO_BETHESDA * vertex.blade_position * GAME_UNITS_PER_ENGINE_UNIT, vertex.blade_tint);
    input.v6 = vec4<f32>(row0, row2.y);
    input.v7 = vec4<f32>(row1, row2.z);
    input.v8 = vec4<f32>(row2.x, vertex.blade_height, 0.0, 0.0);
    let result = grass_vs(input);

    var out: GrassVertexOutput;
    out.clip_position = result.o0;
    out.tex_coords = vec2<f32>(result.o4.w, result.o5.w);
    out.world_normal = safe_normalize(BETHESDA_TO_ENGINE * vec3<f32>(result.o1.z, result.o2.z, result.o3.z));
    out.world_tangent = safe_normalize(BETHESDA_TO_ENGINE * vec3<f32>(result.o1.y, result.o2.y, result.o3.y));
    out.tangent_w = vertex.tangent.w;
    out.world_pos = BETHESDA_TO_ENGINE * (result.o4.xyz + grass_cb12[35].xyz) * ENGINE_UNITS_PER_GAME_UNIT;
    // Colour-less grass variants (rubble, rocks) take neither the vertex
    // colour nor the terrain tint.
    out.tint = select(result.o6.xyz, vec3<f32>(1.0), (grass_batch.flags & BATCH_UNTINTED) != 0u);
    out.fade = result.o6.w;
    return out;
}

// Alpha test with the distance fade multiplied into the alpha, so grass thins
// out by erosion instead of blending. Every derivative (texture samples, the
// tangent frame, the mip view) precedes the discard.
@fragment
fn fs_grass(in: GrassVertexOutput, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let mat = materials[grass_batch.material_id];
    let albedo = textureSample(textures[mat.diffuse_index], material_sampler, in.tex_coords);
    let normal_sample = textureSample(textures[mat.normal_index], material_sampler, in.tex_coords);
    let specular_map = textureSample(textures[mat.specular_index], material_sampler, in.tex_coords);

    let normals = two_sided_normals(in.world_normal, in.world_tangent, in.tangent_w, in.world_pos, in.tex_coords, front_facing, normal_sample);

    let surface = surface_sample(mat, albedo.rgb * in.tint, normals, specular_map);
    let mip = texture_mip_info(mat.diffuse_index, in.tex_coords);

    if (albedo.a * in.fade < grass_batch.alpha_cutoff) {
        discard;
    }
    let point = ShadingPoint(in.world_pos, in.clip_position.xy, RECEIVER_OBJECT);
    return vec4<f32>(shade_surface(surface, point, vec3<f32>(0.0), mip), 1.0);
}
