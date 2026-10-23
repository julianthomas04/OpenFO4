// Texture feedback: a sparse sample of the main view's pixels reports, per
// texture slot, the finest mip their footprint calls for. Streaming reads the
// result back and only ever makes textures finer because of it.

// Mirrors FEEDBACK_ENCODE in texture_streaming/feedback.rs: a slot holds
// FEEDBACK_ENCODE - mip, so the finest mip is the largest value and a cleared
// buffer (zero) means "not sampled".
const FEEDBACK_ENCODE = 16u;
const FEEDBACK_MAX_MIP = 15.0;

// Own bind group, bound only for the main view's object and terrain pipelines:
// a writable binding in the shared material group would tax every pass.
@group(3) @binding(0) var<storage, read_write> texture_feedback_mips: array<atomic<u32>>;

// Pipeline-overridable so a disabled feedback compiles to nothing.
override TEXTURE_FEEDBACK: bool = false;

// Hash of the pixel and the frame, so each frame reports a different subset.
fn texture_feedback_pixel_reports(pixel: vec2<f32>) -> bool {
    let p = vec2<u32>(pixel);
    var h = (p.x * 0x9E3779B1u) ^ (p.y * 0x85EBCA77u) ^ (frame.texture_feedback.y * 0xC2B2AE3Du);
    h = (h ^ (h >> 15u)) * 0x2C1B3C6Du;
    h = h ^ (h >> 12u);
    return (h & frame.texture_feedback.z) == 0u;
}

// The footprint's mip in full-resolution terms; the same anisotropic LOD the
// mip debug views use.
fn feedback_wanted_mip(slot: u32, uv_dx: vec2<f32>, uv_dy: vec2<f32>) -> f32 {
    let resident_size = vec2<f32>(textureDimensions(textures[slot], 0));
    return f32(slot_mips[slot].first_mip) + anisotropic_lod(uv_dx * resident_size, uv_dy * resident_size);
}

// Report what the footprint at `uv` wants of each selected slot. Call from
// uniform control flow (it takes derivatives), from a fragment entry point that
// forces early depth tests, so only visible pixels report. Everything
// in `slots` is sampled with the same `uv`. Stand-in slots (a placeholder or
// a reserved view) have no first mip and are skipped.
fn texture_feedback(slots: vec4<u32>, selected: vec4<bool>, uv: vec2<f32>, pixel: vec2<f32>) {
    if (!TEXTURE_FEEDBACK) {
        return;
    }
    let uv_dx = dpdx(uv);
    let uv_dy = dpdy(uv);
    if (!texture_feedback_pixel_reports(pixel)) {
        return;
    }
    for (var i = 0u; i < 4u; i++) {
        let slot = slots[i];
        if (selected[i] && slot_mips[slot].first_mip != PLACEHOLDER_FIRST_MIP) {
            let wanted = u32(clamp(floor(feedback_wanted_mip(slot, uv_dx, uv_dy)), 0.0, FEEDBACK_MAX_MIP));
            let encoded = FEEDBACK_ENCODE - wanted;
            // The load keeps the many pixels of a big surface from all
            // contending on one address once its finest mip is recorded.
            if (atomicLoad(&texture_feedback_mips[slot]) < encoded) {
                atomicMax(&texture_feedback_mips[slot], encoded);
            }
        }
    }
}
