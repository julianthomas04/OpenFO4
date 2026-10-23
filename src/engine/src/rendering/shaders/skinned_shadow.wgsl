// Skinned shadow casters; the alpha-test fragment shader is shared with shadow.wgsl.

@vertex
fn vs_shadow_skinned_opaque(
    vertex: VertexInput,
    skin: SkinInput,
    @builtin(instance_index) instance_id: u32,
) -> @builtin(position) vec4<f32> {
    return object_clip_position(skin_pose(vertex, skin, instance_id).position, instance_id);
}

@vertex
fn vs_shadow_skinned_alpha_test(
    vertex: VertexInput,
    skin: SkinInput,
    @builtin(instance_index) instance_id: u32,
) -> AlphaTestVaryings {
    var out: AlphaTestVaryings;
    out.clip_position = object_clip_position(skin_pose(vertex, skin, instance_id).position, instance_id);
    out.tex_coords = vertex.tex_coords;
    out.instance_id = instance_id;
    return out;
}
