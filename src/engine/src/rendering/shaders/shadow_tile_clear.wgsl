// Resets one lamp tile of the shadow atlas to the reverse-Z far depth; the
// viewport and scissor select the tile.
@vertex
fn vs_clear_tile(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(fullscreen_triangle_ndc(vertex_index), 0.0, 1.0);
}
