// The 3D model of a large loading screen: the diffuse texture under one directional, tinted light, a
// near-zero ambient term and the world's specular lobe. The normal map and the specular map
// (R = intensity, G = smoothness) are read through the world's tangent frame and surface
// response (`normal_mapping.wgsl`, `surface_response.wgsl`). The pass draws through the non-sRGB
// view of the swapchain like the rest of the menu, so the shader encodes the colour itself when
// `shading.z` is 1.

struct Scene {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    // Unit vector towards the light, in engine space.
    light_direction: vec4<f32>,
    // The ambient and the diffuse strength of the light, 1 when the shader has to encode to sRGB,
    // and the strength of the specular lobe.
    shading: vec4<f32>,
    // The linear colour of the light, applied to its diffuse and specular terms.
    light_colour: vec4<f32>,
};

@group(0) @binding(0) var<uniform> scene: Scene;
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

fn encode_srgb(linear: vec3<f32>) -> vec3<f32> {
    let low = linear * 12.92;
    let high = 1.055 * pow(linear, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, linear <= vec3<f32>(0.0031308));
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

// The viewer is at the world's origin.
fn direction_to_viewer(world_pos: vec3<f32>) -> vec3<f32> {
    return safe_normalize(-world_pos);
}

// The specular lobe of the key light on `surface`; none where the light does not reach it.
fn key_specular(surface: SurfaceSample, light: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    if dot(surface.n, light) <= 0.0 {
        return vec3<f32>(0.0);
    }
    return scene.light_colour.rgb * scene.shading.w
        * specular_response(surface, light, direction_to_viewer(world_pos));
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
    let light = scene.light_direction.xyz;
    let surface = SurfaceSample(
        texel.rgb,
        normals.shading,
        normals.geometric,
        material.surface.x * specular_texel.g,
        material.surface.y * specular_texel.r,
    );
    let key = scene.light_colour.rgb * (scene.shading.y * max(dot(surface.n, light), 0.0));
    let lit = texel.rgb * (vec3<f32>(scene.shading.x) + key) + key_specular(surface, light, in.world_pos);
    let colour = select(lit, encode_srgb(lit), scene.shading.z > 0.5);
    return vec4<f32>(colour, 1.0);
}
