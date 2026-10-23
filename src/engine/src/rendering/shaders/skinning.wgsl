// Linear blend skinning shared by the skinned object and skinned shadow vertex
// shaders. Group 2 holds the character instances (binding 0, scene_bindings.wgsl)
// next to the palette; instance_id indexes both `instances` and `skin_palette_base`.

struct SkinInput {
    @location(4) joints: vec4<u32>,
    @location(5) weights: vec4<f32>,
}

// Per character and skin: model-space joint matrix times inverse bind matrix.
@group(2) @binding(2) var<storage, read> skin_palette: array<mat4x4<f32>>;
// First palette matrix of each instance's skin.
@group(2) @binding(3) var<storage, read> skin_palette_base: array<u32>;

struct SkinnedPose {
    position: vec3<f32>,
    normal: vec3<f32>,
    tangent: vec3<f32>,
}

fn skin_matrix(skin: SkinInput, instance_id: u32) -> mat4x4<f32> {
    let base = skin_palette_base[instance_id];
    return skin_palette[base + skin.joints.x] * skin.weights.x
        + skin_palette[base + skin.joints.y] * skin.weights.y
        + skin_palette[base + skin.joints.z] * skin.weights.z
        + skin_palette[base + skin.joints.w] * skin.weights.w;
}

fn normalize_or_zero(v: vec3<f32>) -> vec3<f32> {
    let len = length(v);
    return select(vec3<f32>(0.0), v / len, len > 0.0);
}

fn skin_pose(vertex: VertexInput, skin: SkinInput, instance_id: u32) -> SkinnedPose {
    let m = skin_matrix(skin, instance_id);
    let m3 = mat3x3<f32>(m[0].xyz, m[1].xyz, m[2].xyz);
    return SkinnedPose(
        (m * vec4<f32>(vertex.position, 1.0)).xyz,
        normalize_or_zero(m3 * vertex.normal),
        normalize_or_zero(m3 * vertex.tangent.xyz),
    );
}
