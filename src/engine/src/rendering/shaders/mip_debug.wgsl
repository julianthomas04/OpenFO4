// Mip debug views: the diffuse texture's streaming state at a sampling site.
// Texture slots are bound as a binding_array, so everything here is queried
// per slot; `slot_mips` holds each slot's resident first file mip and the
// skip its held texture feedback calls for.

// Mirrors PLACEHOLDER_FIRST_MIP in gpu_driven.rs.
const PLACEHOLDER_FIRST_MIP = 0xffffffffu;
// Mirrors NO_FEEDBACK_SKIP in gpu_driven.rs.
const NO_FEEDBACK_SKIP = 0xffffffffu;
const NO_MIP = -1.0;
// Mirrors `mip_tint` in debug_view.rs. Red = mip 0 (most expensive), through
// yellow, to green from MIP_FULL_GREEN up; the tint is as strong as the
// shadow-cascade one (cascade_debug_tint).
const MIP_FULL_GREEN = 4.0;
const GREEN_HUE = 1.0 / 3.0;
const MIP_TINT_STRENGTH = 0.65;
// Outside the red-green ramp.
const PLACEHOLDER_MIP_TINT = vec3<f32>(1.0, 0.35, 1.0);
const MIN_ANISOTROPY_AXIS_SQ = 1e-12;
// Mip deficit view, on the same ramp: the resident mip may be this many mips
// finer than the one wanted and still count as tight.
const TIGHT_SLACK_MIPS = 1.0;
// Texture waste view: slots without feedback data; pure blue at the ramp's tint strength, outside every other ramp.
const NO_FEEDBACK_TINT = vec3<f32>(0.35, 0.35, 1.0);

// All values in full-resolution mip terms; NO_MIP while a stand-in texture is
// bound. `sampled` is what the resident texture can serve (clamped to its
// levels), `wanted` the mip the footprint calls for, never below 0: finer than
// `streamed` when the texture is under-resolved. It uses the same
// anisotropic LOD as the texture feedback. `feedback_skip` is the finest need
// any visible pixel had of the slot (the held feedback, in the units of
// `streamed`); NO_MIP when the slot has no feedback data.
struct MipInfo {
    streamed: f32,
    sampled: f32,
    wanted: f32,
    feedback_skip: f32,
}

fn mip_view_active() -> bool {
    let view = debug_view();
    return view == DEBUG_VIEW_STREAMED_MIP || view == DEBUG_VIEW_SAMPLED_MIP || view == DEBUG_VIEW_MIP_DEFICIT
        || view == DEBUG_VIEW_TEXTURE_WASTE;
}

// Standard anisotropic LOD: the footprint's major axis divided by the probe
// count min(ceil(major / minor), max anisotropy). An approximation of what the
// hardware selects, which is implementation defined.
fn anisotropic_lod(texel_dx: vec2<f32>, texel_dy: vec2<f32>) -> f32 {
    let dx_sq = dot(texel_dx, texel_dx);
    let dy_sq = dot(texel_dy, texel_dy);
    let major = sqrt(max(max(dx_sq, dy_sq), MIN_ANISOTROPY_AXIS_SQ));
    let minor = sqrt(max(min(dx_sq, dy_sq), MIN_ANISOTROPY_AXIS_SQ));
    let probes = min(ceil(major / minor), f32(max(frame.flags.w, 1u)));
    return log2(major / probes);
}

// The branch condition is a frame uniform, and the derivatives precede every
// per-pixel branch, so this is valid in uniform control flow.
fn texture_mip_info(slot: u32, uv: vec2<f32>) -> MipInfo {
    if (!mip_view_active()) {
        return MipInfo(NO_MIP, NO_MIP, NO_MIP, NO_MIP);
    }
    let resident_size = vec2<f32>(textureDimensions(textures[slot], 0));
    let lod = anisotropic_lod(dpdx(uv) * resident_size, dpdy(uv) * resident_size);
    let resident_lod = clamp(lod, 0.0, f32(textureNumLevels(textures[slot]) - 1u));
    let info = slot_mips[slot];
    let first_mip = info.first_mip;
    let streamed = f32(first_mip);
    let is_placeholder = first_mip == PLACEHOLDER_FIRST_MIP;
    return MipInfo(
        select(streamed, NO_MIP, is_placeholder),
        select(streamed + resident_lod, NO_MIP, is_placeholder),
        select(max(streamed + lod, 0.0), NO_MIP, is_placeholder),
        select(f32(info.feedback_skip), NO_MIP, info.feedback_skip == NO_FEEDBACK_SKIP),
    );
}

fn mip_tint(mip: f32) -> vec3<f32> {
    if (mip < 0.0) {
        return PLACEHOLDER_MIP_TINT;
    }
    let hue = saturate(mip / MIP_FULL_GREEN) * GREEN_HUE;
    let rgb = clamp(abs(fract(hue + vec3<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0) - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
    return mix(vec3<f32>(1.0), rgb, MIP_TINT_STRENGTH);
}

// Resident mips finer than `need` (both in `streamed` units): negative is too
// coarse, up to TIGHT_SLACK_MIPS is tight, more is waste.
fn slack_tint(slack: f32) -> vec3<f32> {
    if (slack < 0.0) {
        return mip_tint(0.0);
    }
    return mip_tint(select(MIP_FULL_GREEN * 0.5, MIP_FULL_GREEN, slack <= TIGHT_SLACK_MIPS));
}

// Per pixel: red where the footprint wants a finer mip than is resident (a
// streaming failure), green where the resident mip is at most TIGHT_SLACK_MIPS
// finer than wanted, yellow where it is more (this pixel needs less than the
// texture holds, which a nearer use may justify). iTexMipMapSkip is a
// deliberate bias, so the wanted mip includes it.
fn mip_deficit_tint(mip: MipInfo) -> vec3<f32> {
    if (mip.streamed < 0.0) {
        return mip_tint(NO_MIP);
    }
    return slack_tint(floor(mip.wanted) + f32(frame.texture_feedback.w) - mip.streamed);
}

// Per texture: the resident mip against the finest mip any visible pixel
// needed of the slot, so yellow is memory nothing on screen uses. Slots that
// do not report (alpha-test, decal, blend) or were not seen recently are
// NO_FEEDBACK_TINT, everything is while feedback is disabled.
fn texture_waste_tint(mip: MipInfo) -> vec3<f32> {
    if (mip.streamed < 0.0) {
        return mip_tint(NO_MIP);
    }
    if (mip.feedback_skip < 0.0) {
        return NO_FEEDBACK_TINT;
    }
    return slack_tint(mip.feedback_skip - mip.streamed);
}
