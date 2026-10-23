// Water shader wrapper: entry points, bind groups and varying plumbing around the
// install-time translated water vertex and pixel functions (`water_vs_*`, `water_ps_*`).
// The translated functions read the D3D register arrays below through the global names
// the shader manifest gives them. The camera-relative world is Bethesda-axes, Bethesda
// units; `rendering/water/constants.rs` fills the registers.

struct WaterFrame {
    cb0: array<vec4<f32>, 8>,
    cb12: array<vec4<f32>, 47>,
};

// Per-draw registers. Vertex and pixel stage have separate cb1/cb2 contents.
struct WaterGeometry {
    vs_cb2: array<vec4<f32>, 8>,
    ps_cb2: array<vec4<f32>, 5>,
    vs_cb1: array<vec4<f32>, 3>,
    ps_cb1: array<vec4<f32>, 14>,
    _padding: array<vec4<f32>, 2>,
};

@group(0) @binding(0) var scene_color: texture_2d<f32>;
@group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(0) @binding(2) var scene_sampler: sampler;
@group(0) @binding(3) var scene_depth_sampler: sampler;

@group(1) @binding(0) var<uniform> water_frame: WaterFrame;
@group(1) @binding(1) var ripple_texture: texture_2d<f32>;
@group(1) @binding(2) var ssr_sharp: texture_2d<f32>;
@group(1) @binding(3) var ssr_blurred: texture_2d<f32>;
@group(1) @binding(4) var linear_clamp: sampler;
@group(1) @binding(5) var water_mask: texture_2d<f32>;

@group(2) @binding(0) var normal_layer_1: texture_2d<f32>;
@group(2) @binding(1) var normal_layer_2: texture_2d<f32>;
@group(2) @binding(2) var normal_layer_3: texture_2d<f32>;
@group(2) @binding(3) var normal_sampler: sampler;

@group(3) @binding(0) var<uniform> geometry: WaterGeometry;

// `ps_cb2[0]` of the wading displacement quad draws (zero on every other draw): x is 1,
// yz the quad centre relative to the camera and w the reciprocal quad size. Such a draw
// only has pixels on the quad.
const DISPLACEMENT_QUAD_DRAW_REGISTER = 0u;
// The wading displacement quad's placement lives in this frame register.
const DISPLACEMENT_QUAD_FRAME_REGISTER = 36u;
const CAMERA_OFFSET_FRAME_REGISTER = 35u;

struct WaterVertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
}

// Every vertex entry fills registers 1..7 (unused ones stay zero); a fragment entry
// declares the ones its translated function reads.
struct WaterVarying {
    @builtin(position) clip_position: vec4<f32>,
    @location(1) v1: vec4<f32>,
    @location(2) v2: vec4<f32>,
    @location(3) v3: vec4<f32>,
    @location(4) v4: vec4<f32>,
    @location(5) v5: vec4<f32>,
    @location(6) v6: vec4<f32>,
    @location(7) v7: vec4<f32>,
}

fn displacement_uv(world_rel_xy: vec2<f32>) -> vec2<f32> {
    let quad = water_frame.cb12[DISPLACEMENT_QUAD_FRAME_REGISTER];
    let world_xy = world_rel_xy + water_frame.cb12[CAMERA_OFFSET_FRAME_REGISTER].xy;
    return vec2<f32>(
        (world_xy.x - quad.x) * quad.z + 0.5,
        0.5 - (world_xy.y - quad.y) * quad.z,
    );
}

// The pixel lies beyond the quad of a displacement quad draw.
fn outside_displacement_quad_draw(world_rel_xy: vec2<f32>) -> bool {
    let quad = geometry.ps_cb2[DISPLACEMENT_QUAD_DRAW_REGISTER];
    let offset = abs(world_rel_xy - quad.yz) * quad.w;
    return quad.x != 0.0 && any(offset > vec2<f32>(0.5));
}

fn position_input(position: vec3<f32>) -> vec4<f32> {
    return vec4<f32>(position, 0.0);
}

fn texcoord_input(tex_coords: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(tex_coords, 0.0, 0.0);
}

// The wading ripple lookup reads the displacement field over the quad, not the mesh uv.
fn with_displacement_uv(texcoord: vec4<f32>, world_rel_xy: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(displacement_uv(world_rel_xy), texcoord.zw);
}

@vertex
fn vs_basic(v: WaterVertexInput) -> WaterVarying {
    var i: water_vs_basic_in;
    i.v0 = position_input(v.position);
    let o = water_vs_basic(i);
    return WaterVarying(o.o0, o.o1, o.o2, o.o3, o.o4, o.o5, o.o6, vec4<f32>(0.0));
}

@vertex
fn vs_wading(v: WaterVertexInput) -> WaterVarying {
    var i: water_vs_wading_in;
    i.v0 = position_input(v.position);
    i.v1 = texcoord_input(v.tex_coords);
    let o = water_vs_wading(i);
    return WaterVarying(o.o0, o.o1, o.o2, o.o3, o.o4, o.o5, with_displacement_uv(o.o6, o.o1.xy), o.o7);
}

@vertex
fn vs_underwater(v: WaterVertexInput) -> WaterVarying {
    var i: water_vs_underwater_in;
    i.v0 = position_input(v.position);
    i.v1 = texcoord_input(v.tex_coords);
    let o = water_vs_underwater(i);
    return WaterVarying(o.o0, o.o1, o.o2, o.o3, o.o4, o.o5, with_displacement_uv(o.o6, o.o1.xy), o.o7);
}

@vertex
fn vs_lod(v: WaterVertexInput) -> WaterVarying {
    var i: water_vs_lod_in;
    i.v0 = position_input(v.position);
    let o = water_vs_lod(i);
    return WaterVarying(o.o0, o.o1, o.o2, o.o3, o.o4, vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
}

@vertex
fn vs_lod_ssr(v: WaterVertexInput) -> WaterVarying {
    var i: water_vs_lod_ssr_in;
    i.v0 = position_input(v.position);
    let o = water_vs_lod_ssr(i);
    return WaterVarying(o.o0, o.o1, o.o2, o.o3, o.o4, vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
}

@vertex
fn vs_position(v: WaterVertexInput) -> WaterVarying {
    var i: water_vs_position_in;
    i.v0 = position_input(v.position);
    let o = water_vs_position(i);
    return WaterVarying(o.o0, o.o1, o.o2, o.o3, vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
}

struct DepthInput {
    @builtin(position) pos: vec4<f32>,
    @location(1) v1: vec4<f32>,
    @location(4) v4: vec4<f32>,
    @location(5) v5: vec4<f32>,
    @location(6) v6: vec4<f32>,
    @location(7) v7: vec4<f32>,
}

fn visible_depth_colour(f: DepthInput, colour: vec4<f32>) -> vec4<f32> {
    if (outside_displacement_quad_draw(f.v1.xy)) {
        discard;
    }
    return colour;
}

@fragment
fn fs_main(f: DepthInput) -> @location(0) vec4<f32> {
    var i: water_ps_depth_in;
    i.v0 = f.pos;
    i.v1 = f.v1;
    i.v4 = f.v4;
    i.v5 = f.v5;
    i.v6 = f.v6;
    i.v7 = f.v7;
    return visible_depth_colour(f, water_ps_depth(i).o0);
}

// The interior (no direct sun) permutation of the same shader: descriptor bit 0x20.
@fragment
fn fs_main_interior(f: DepthInput) -> @location(0) vec4<f32> {
    var i: water_ps_depth_interior_in;
    i.v0 = f.pos;
    i.v1 = f.v1;
    i.v4 = f.v4;
    i.v5 = f.v5;
    i.v6 = f.v6;
    i.v7 = f.v7;
    return visible_depth_colour(f, water_ps_depth_interior(i).o0);
}

struct RefractionInput {
    @builtin(position) pos: vec4<f32>,
    @location(1) v1: vec4<f32>,
    @location(4) v4: vec4<f32>,
    @location(5) v5: vec4<f32>,
    @location(6) v6: vec4<f32>,
}

@fragment
fn fs_refraction_only(f: RefractionInput) -> @location(0) vec4<f32> {
    var i: water_ps_refraction_in;
    i.v0 = f.pos;
    i.v1 = f.v1;
    i.v4 = f.v4;
    i.v5 = f.v5;
    i.v6 = f.v6;
    return water_ps_refraction(i).o0;
}

struct UnderwaterInput {
    @builtin(position) pos: vec4<f32>,
    @location(1) v1: vec4<f32>,
    @location(4) v4: vec4<f32>,
    @location(5) v5: vec4<f32>,
    @location(6) v6: vec4<f32>,
}

@fragment
fn fs_underwater(f: UnderwaterInput) -> @location(0) vec4<f32> {
    var i: water_ps_underwater_in;
    i.v0 = f.pos;
    i.v1 = f.v1;
    i.v4 = f.v4;
    i.v5 = f.v5;
    i.v6 = f.v6;
    let o = water_ps_underwater(i);
    if (outside_displacement_quad_draw(f.v1.xy)) {
        discard;
    }
    return o.o0;
}

struct LodInput {
    @location(1) v1: vec4<f32>,
    @location(3) v3: vec4<f32>,
    @location(4) v4: vec4<f32>,
}

@fragment
fn fs_lod(f: LodInput) -> @location(0) vec4<f32> {
    var i: water_ps_lod_in;
    i.v1 = f.v1;
    i.v3 = f.v3;
    i.v4 = f.v4;
    return water_ps_lod(i).o0;
}

struct LodSsrInput {
    @builtin(position) pos: vec4<f32>,
    @location(1) v1: vec4<f32>,
    @location(3) v3: vec4<f32>,
    @location(4) v4: vec4<f32>,
}

@fragment
fn fs_lod_ssr(f: LodSsrInput) -> @location(0) vec4<f32> {
    var i: water_ps_lod_ssr_in;
    i.v0 = f.pos;
    i.v1 = f.v1;
    i.v3 = f.v3;
    i.v4 = f.v4;
    return water_ps_lod_ssr(i).o0;
}

struct PositionInput {
    @builtin(position) pos: vec4<f32>,
    @location(1) v1: vec4<f32>,
}

@fragment
fn fs_depth_fog(f: PositionInput) -> @location(0) vec4<f32> {
    var i: water_ps_depth_fog_in;
    i.v0 = f.pos;
    i.v1 = f.v1;
    return water_ps_depth_fog(i).o0;
}

// The mask target stores a single channel; the translated constant-colour pass writes it
// to alpha.
@fragment
fn fs_mask_surface() -> @location(0) vec4<f32> {
    var i: water_ps_mask_surface_in;
    return vec4<f32>(water_ps_mask_surface(i).o0.w, 0.0, 0.0, 0.0);
}

@fragment
fn fs_mask_wading(f: PositionInput) -> @location(0) vec4<f32> {
    var i: water_ps_mask_wading_in;
    let o = water_ps_mask_wading(i);
    if (outside_displacement_quad_draw(f.v1.xy)) {
        discard;
    }
    return vec4<f32>(o.o0.w, 0.0, 0.0, 0.0);
}

// The underwater surface depth: what the depth-writing underwater state leaves in the
// depth buffer, written to a texture of its own so the Hi-Z pyramid never sees it.
@fragment
fn fs_surface_depth(f: PositionInput) -> @location(0) f32 {
    return f.pos.z;
}

// Debug views (`rendering/water/debug_output.rs`): fragment entries that stand in for the
// translated pixel shader of a draw. They read only resources bound above and share the
// vertex entries of the shading draws, so they cover the same pixels; the translated shaders
// are not involved. The values reach the HDR target untonemapped, like every debug view.
struct DebugInput {
    @builtin(position) pos: vec4<f32>,
    @location(1) v1: vec4<f32>,
}

fn debug_screen_uv(f: DebugInput) -> vec2<f32> {
    return f.pos.xy * water_frame.cb0[0].xy;
}

// The wading quad draws only have pixels on the quad, as in the shading draws.
fn debug_discard_outside_quad(f: DebugInput) {
    if (outside_displacement_quad_draw(f.v1.xy)) {
        discard;
    }
}

@fragment
fn fs_debug_coverage(f: DebugInput) -> @location(0) vec4<f32> {
    debug_discard_outside_quad(f);
    return vec4<f32>(1.0, 0.0, 1.0, 0.0);
}

@fragment
fn fs_debug_scene_copy(f: DebugInput) -> @location(0) vec4<f32> {
    debug_discard_outside_quad(f);
    let scene = textureSampleLevel(scene_color, scene_sampler, debug_screen_uv(f), 0.0);
    return vec4<f32>(scene.rgb, 0.0);
}

// The sharp tap (`ssr_sharp`) weighted by its coverage alpha: black where the ray missed.
@fragment
fn fs_debug_ssr(f: DebugInput) -> @location(0) vec4<f32> {
    debug_discard_outside_quad(f);
    let tap = textureSampleLevel(ssr_sharp, linear_clamp, debug_screen_uv(f), 0.0);
    return vec4<f32>(tap.rgb * tap.a, 0.0);
}

// The field's R (height) and G (velocity) are biased by 0.5: red above rest, blue below,
// green the velocity magnitude. Black at rest and beyond the quad.
@fragment
fn fs_debug_displacement(f: DebugInput) -> @location(0) vec4<f32> {
    debug_discard_outside_quad(f);
    let uv = displacement_uv(f.v1.xy);
    let field = textureSampleLevel(ripple_texture, linear_clamp, uv, 0.0);
    let height = (field.x - 0.5) * 2.0;
    let velocity = abs(field.y - 0.5) * 2.0;
    let colour = vec3<f32>(max(height, 0.0), velocity, max(-height, 0.0));
    let on_quad = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
    return vec4<f32>(select(vec3<f32>(0.0), colour, on_quad), 0.0);
}
