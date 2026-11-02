// The 3D model of a large loading screen: the diffuse texture under one directional light and a
// flat ambient term. The pass draws through the non-sRGB
// view of the swapchain like the rest of the menu, so the shader encodes the colour itself when
// `shading.z` is 1.

struct Scene {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    // Unit vector towards the light, in engine space.
    light_direction: vec4<f32>,
    // The ambient and the diffuse strength of the light, 1 when the shader has to encode to
    // sRGB, and 1 when the model is drawn in the HUD colour alone.
    shading: vec4<f32>,
    // The HUD colour in linear light.
    tint: vec4<f32>,
};

@group(0) @binding(0) var<uniform> scene: Scene;
@group(1) @binding(0) var diffuse: texture_2d<f32>;
@group(1) @binding(1) var diffuse_sampler: sampler;

struct Material {
    // The alpha below which a texel is cut out; 0 for a material that cuts nothing out.
    cutoff: vec4<f32>,
};

@group(1) @binding(2) var<uniform> material: Material;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
};

struct VsOut {
    // Invariant so that the depth prepass and the colour pass agree to the bit.
    @invariant @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) normal: vec3<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip = scene.view_proj * scene.model * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    out.normal = mat3x3<f32>(
        scene.model[0].xyz,
        scene.model[1].xyz,
        scene.model[2].xyz,
    ) * in.normal;
    return out;
}

// The weights of linear red, green and blue in perceived brightness (Rec. 709).
const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

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

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let facing = select(-in.normal, in.normal, front);
    let normal = normalize(facing);
    let light = max(dot(normal, scene.light_direction.xyz), 0.0);
    let texel = textureSample(diffuse, diffuse_sampler, in.uv);
    if is_cut_out(texel.a) {
        discard;
    }
    let brightness = dot(texel.rgb, LUMA);
    let albedo = select(texel.rgb, vec3<f32>(brightness) * scene.tint.rgb, scene.shading.w > 0.5);
    let lit = albedo * (scene.shading.x + scene.shading.y * light);
    let colour = select(lit, encode_srgb(lit), scene.shading.z > 0.5);
    return vec4<f32>(colour, 1.0);
}
