// The 3D model of a large loading screen, lit by the game's three loading-menu scene lights:
// spotlights aimed at the model, fixed relative to the camera, each with the world's placed-light distance falloff
// (`placed_light.wgsl`) and the world's surface response (`surface_response.wgsl`), and no ambient
// term. The normal map and the specular map (R = intensity, G = smoothness) are read through the
// world's tangent frame (`normal_mapping.wgsl`). The output is linear radiance, unclamped, for the
// scene's float picture.

const SCENE_LIGHT_COUNT = 3;

struct SceneLight {
    // The position in engine space and the radius in engine units.
    position_radius: vec4<f32>,
    // The linear colour with the dimmer applied.
    colour: vec4<f32>,
    // The falloff's constant, scalar and exponent, as a placed light's record stores them.
    attenuation: vec4<f32>,
};

struct Scene {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    lights: array<SceneLight, SCENE_LIGHT_COUNT>,
    // xyz: the point the spotlights aim at, w: their falloff exponent.
    aim: vec4<f32>,
};

@group(0) @binding(0) var<uniform> scene: Scene;

// The shadows of the three lights, looked up by the world's `lamp_shadow` (`lamp_shadow.wgsl`)
// in the world's lamp-shadow table and atlas, slot `i` being light `i`.
@group(2) @binding(0) var<uniform> lamp_shadows: ShadowLampTable;
@group(2) @binding(1) var lamp_shadow_atlas: texture_depth_2d;
@group(2) @binding(2) var shadow_comparison_sampler: sampler_comparison;

// The world's lookup fades a shadow out with the receiver's distance from the camera; here the
// camera is the origin and the table's fade distance is out of reach.
struct ShadowFrame {
    camera_pos: vec4<f32>,
};
const frame = ShadowFrame(vec4<f32>(0.0));

// The view of the light whose depth the caster pass draws.
struct CasterView {
    view_proj: mat4x4<f32>,
};
@group(2) @binding(3) var<uniform> caster: CasterView;

@group(1) @binding(0) var diffuse: texture_2d<f32>;
@group(1) @binding(1) var diffuse_sampler: sampler;
@group(1) @binding(3) var normal_map: texture_2d<f32>;
@group(1) @binding(4) var specular_map: texture_2d<f32>;

struct Material {
    // The alpha below which a texel is cut out; 0 for a material that cuts nothing out.
    cutoff: vec4<f32>,
    // The material's smoothness and specular scale, which the specular map's G and R scale.
    surface: vec4<f32>,
};

@group(1) @binding(2) var<uniform> material: Material;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) tangent: vec4<f32>,
};

struct VsOut {
    // Invariant so that the depth prepass and the colour pass agree to the bit.
    @invariant @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) world_pos: vec3<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let model3 = mat3x3<f32>(
        scene.model[0].xyz,
        scene.model[1].xyz,
        scene.model[2].xyz,
    );
    let world = scene.model * vec4<f32>(in.position, 1.0);
    var out: VsOut;
    out.clip = scene.view_proj * world;
    out.uv = in.uv;
    out.normal = model3 * in.normal;
    out.tangent = vec4<f32>(model3 * in.tangent.xyz, in.tangent.w * select(1.0, -1.0, determinant(model3) < 0.0));
    out.world_pos = world.xyz;
    return out;
}

// Whether a texel of `alpha` is cut out (hair, grates, foliage): only a material that asks for
// an alpha test cuts anything out, like the world's alpha-test bucket.
fn is_cut_out(alpha: f32) -> bool {
    return material.cutoff.x > 0.0 && alpha < material.cutoff.x;
}

// The depth prepass: only the cut-out decides what is written.
@fragment
fn fs_depth(in: VsOut) -> @location(0) vec4<f32> {
    if is_cut_out(textureSample(diffuse, diffuse_sampler, in.uv).a) {
        discard;
    }
    return vec4<f32>(0.0);
}

struct ShadowVsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// The caster pass: the model as the light sees it.
@vertex
fn vs_shadow(in: VsIn) -> ShadowVsOut {
    var out: ShadowVsOut;
    out.clip = caster.view_proj * (scene.model * vec4<f32>(in.position, 1.0));
    out.uv = in.uv;
    return out;
}

// Cut-out texels cast nothing, as in the world's alpha-tested casters.
@fragment
fn fs_shadow(in: ShadowVsOut) {
    if is_cut_out(textureSample(diffuse, diffuse_sampler, in.uv).a) {
        discard;
    }
}

// The viewer is at the world's origin.
fn direction_to_viewer(world_pos: vec3<f32>) -> vec3<f32> {
    return safe_normalize(-world_pos);
}

// What light `index` adds to `surface` at `world_pos`: the world's local light, a spotlight with a
// lamp shadow aimed at `scene.aim` (`attenuation.w` is the cosine of its half-angle), so the full
// surface response. The response is zero where the light is behind the surface.
fn scene_light_radiance(index: u32, surface: SurfaceSample, light: SceneLight, world_pos: vec3<f32>) -> vec3<f32> {
    let to_light = light.position_radius.xyz - world_pos;
    let distance = max(length(to_light), MIN_LIGHT_DISTANCE);
    let to_light_unit = to_light / distance;
    let falloff = loading_scene_light_falloff(light.position_radius.w, light.attenuation.xyz, distance);
    let cos_axis = -dot(to_light_unit, normalize(scene.aim.xyz - light.position_radius.xyz));
    let cone = spot_cosine_falloff(cos_axis, light.attenuation.w, scene.aim.w);
    let shadow = lamp_shadow(index, world_pos, surface.ng, to_light_unit);
    let colour = light.colour.rgb * (falloff * cone * shadow * terminator(surface.ng, to_light_unit));
    let response = surface_response(surface, to_light_unit, direction_to_viewer(world_pos), FULL_RESPONSE);
    return surface.albedo * colour * response.diffuse + colour * response.specular;
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let texel = textureSample(diffuse, diffuse_sampler, in.uv);
    let normals = two_sided_normals(
        in.normal, in.tangent.xyz, in.tangent.w, in.world_pos, in.uv, front,
        textureSample(normal_map, diffuse_sampler, in.uv),
    );
    let specular_texel = textureSample(specular_map, diffuse_sampler, in.uv);
    if is_cut_out(texel.a) {
        discard;
    }
    let surface = SurfaceSample(
        texel.rgb,
        normals.shading,
        normals.geometric,
        material.surface.x * specular_texel.g,
        material.surface.y * specular_texel.r,
    );
    var lit = vec3<f32>(0.0);
    for (var index = 0; index < SCENE_LIGHT_COUNT; index = index + 1) {
        lit += scene_light_radiance(u32(index), surface, scene.lights[index], in.world_pos);
    }
    return vec4<f32>(lit, 1.0);
}
