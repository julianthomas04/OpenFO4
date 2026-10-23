// Skinned characters reuse the object fragment shaders; only the vertex stage differs.

@vertex
fn vs_skinned(
    vertex: VertexInput,
    skin: SkinInput,
    @builtin(instance_index) instance_id: u32,
) -> VertexOutput {
    let pose = skin_pose(vertex, skin, instance_id);
    return object_vertex_output(
        pose.position,
        pose.normal,
        vec4<f32>(pose.tangent, vertex.tangent.w),
        vertex.tex_coords,
        instance_id,
    );
}
