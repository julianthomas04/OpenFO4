// Debug wireframe overlays (collision, navmesh): world-space line list with a per-vertex colour.
// Depth-tested against the scene (reverse-Z) but never written - the HiZ
// occlusion pyramid samples the depth buffer after the main pass.

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
) -> VertexOutput {
    var clip = frame.view_proj * vec4<f32>(position, 1.0);
    // Collision geometry often lies exactly on render surfaces (terrain
    // trimesh = render mesh). Reverse-Z: larger depth = closer, so a small
    // positive nudge pulls the lines in front instead of stippling. The
    // bias must be multiplicative: with the infinite reverse-Z projection
    // ndc_z = znear/d, so an additive NDC offset is a view-space pull of
    // ~2e-4*d^2 - lines x-ray through walls at range. Relative bias keeps
    // the pull at ~1e-4*d at every distance.
    clip.z *= 1.0001;
    return VertexOutput(clip, color);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
