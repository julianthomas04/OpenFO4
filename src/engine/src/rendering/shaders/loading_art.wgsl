// The 3D model of a large loading screen: the diffuse texture under one directional light and a
// flat ambient term, faded by the screen's reveal alpha. The pass draws through the non-sRGB
// view of the swapchain like the rest of the menu, so the shader encodes the colour itself when
// `shading.w` is 1.

struct Scene {
    view_proj: mat4x4<f32>,
    model: mat4x4<f32>,
    // Unit vector towards the light, in engine space.
    light_direction: vec4<f32>,
    // Opacity of the whole model, the ambient and the diffuse strength of the light, and 1 when
    // the shader has to encode to sRGB.
    shading: vec4<f32>,
};

@group(0) @binding(0) var<uniform> scene: Scene;
@group(1) @binding(0) var diffuse: texture_2d<f32>;
@group(1) @binding(1) var diffuse_sampler: sampler;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
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

fn encode_srgb(linear: vec3<f32>) -> vec3<f32> {
    let low = linear * 12.92;
    let high = 1.055 * pow(linear, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, linear <= vec3<f32>(0.0031308));
}

@fragment
fn fs_main(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let facing = select(-in.normal, in.normal, front);
    let normal = normalize(facing);
    let light = max(dot(normal, scene.light_direction.xyz), 0.0);
    let albedo = textureSample(diffuse, diffuse_sampler, in.uv).rgb;
    let lit = albedo * (scene.shading.y + scene.shading.z * light);
    let colour = select(lit, encode_srgb(lit), scene.shading.w > 0.5);
    return vec4<f32>(colour, scene.shading.x);
}
