// Screen-space HUD overlay: solid colored triangles in clip space, multiplied
// by the global HUD tint. The CPU emits clip-space positions directly (no
// view-projection), so the vertex stage is pass-through. The tint reproduces
// FO4's white/greyscale art * fPipboyEffectColor multiply (Pip-Boy green).

struct HudTint {
    color: vec3<f32>,
    opacity: f32,
};

@group(0) @binding(0) var<uniform> tint: HudTint;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) color: vec4<f32>) -> VsOut {
    var out: VsOut;
    out.clip = vec4<f32>(pos, 0.0, 1.0);
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color.rgb * tint.color, in.color.a * tint.opacity);
}
