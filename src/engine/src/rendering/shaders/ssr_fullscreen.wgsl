struct SsrFullscreenVertex {
    ndc: vec2<f32>,
    uv: vec2<f32>,
}

fn ssr_fullscreen_vertex(vertex_index: u32) -> SsrFullscreenVertex {
    let ndc = fullscreen_triangle_ndc(vertex_index);
    return SsrFullscreenVertex(ndc, fullscreen_uv(ndc));
}
