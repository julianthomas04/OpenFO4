const DEBUG_VIEW_ALBEDO = 1u;
const DEBUG_VIEW_NORMALS = 2u;
const DEBUG_VIEW_LIGHTING_ONLY = 3u;
const DEBUG_VIEW_SPECULAR_ONLY = 4u;
const DEBUG_VIEW_SHADOW_CASCADES = 5u;
const DEBUG_VIEW_LIGHT_COUNT_HEATMAP = 6u;
const DEBUG_VIEW_LOCAL_LIGHTS_ONLY = 7u;
const DEBUG_VIEW_LAMP_SHADOW_SLOTS = 8u;
const DEBUG_VIEW_STREAMED_MIP = 9u;
const DEBUG_VIEW_SAMPLED_MIP = 10u;
const DEBUG_VIEW_MIP_DEFICIT = 11u;
const DEBUG_VIEW_TEXTURE_WASTE = 12u;
const DEBUG_VIEW_ENV_REFLECTION = 13u;
const DEBUG_VIEW_EFFECT_CLASS = 14u;

const CLASS_ALPHA_EFFECT = vec3<f32>(1.0, 0.0, 1.0);
const CLASS_ADDITIVE_EFFECT = vec3<f32>(1.0, 1.0, 0.0);
const CLASS_MULTIPLY_EFFECT = vec3<f32>(0.0, 1.0, 1.0);
const CLASS_OPAQUE_EFFECT = vec3<f32>(0.0, 1.0, 0.0);
const CLASS_LIT_ENV = vec3<f32>(0.0, 0.0, 1.0);
const CLASS_OTHER = vec3<f32>(0.08);

const TERMINATOR_SHARPNESS = 8.0;
const FOG_MIN_RANGE = 0.001;

const NO_CASCADE = 0xffffffffu;
const LAST_CASCADE_FADE_FRACTION = 0.1;
const MAX_SLOPE_BIAS_TANGENT = 10.0;
const MIN_NO_L = 1e-4;
// Widest sun penumbra, as a radius in texels; the blocker search covers it.
const SUN_MAX_PENUMBRA_TEXELS = 12.0;
const SUN_MIN_PENUMBRA_TEXELS = 1.0;
const SUN_BLOCKER_SEARCH_TAPS = 16u;
const MIN_PLANE_DETERMINANT = 1e-8;
const GOLDEN_ANGLE = 2.39996323;

// Light flags a receiver is not lit by.
const RECEIVER_OBJECT = 0u;
const RECEIVER_LANDSCAPE = LIGHT_FLAG_REF_DOES_NOT_LIGHT_LANDSCAPE;

const MIN_CLUSTER_DEPTH = 1e-3;
const MIN_LIGHT_DISTANCE = 1e-4;
const HEATMAP_FULL_SCALE = 32.0;
const HEATMAP_EMPTY_COLOR = vec3<f32>(0.0, 0.1, 0.6);
const HEATMAP_MID_COLOR = vec3<f32>(0.0, 1.0, 0.2);
const HEATMAP_FULL_COLOR = vec3<f32>(1.0, 0.1, 0.0);
const HEATMAP_OVERFLOW_COLOR = vec3<f32>(1.0, 0.0, 1.0);
const SLOT_VIEW_BACKGROUND = vec3<f32>(0.03);
const SLOT_VIEW_SHADOWED = 0.2;
const SLOT_PALETTE_SIZE = 8u;
const SLOT_PALETTE = array<vec3<f32>, 8>(
    vec3<f32>(1.0, 0.3, 0.3), vec3<f32>(0.3, 1.0, 0.3), vec3<f32>(0.3, 0.4, 1.0), vec3<f32>(1.0, 1.0, 0.3),
    vec3<f32>(1.0, 0.3, 1.0), vec3<f32>(0.3, 1.0, 1.0), vec3<f32>(1.0, 0.6, 0.2), vec3<f32>(0.7, 0.5, 1.0),
);

// A fragment being shaded.
struct ShadingPoint {
    world_pos: vec3<f32>,
    // Framebuffer position in pixels.
    frag_xy: vec2<f32>,
    excluded_light_flags: u32,
}

// Hides the shading-normal light leak past the geometric terminator.
fn terminator(ng: vec3<f32>, l: vec3<f32>) -> f32 {
    return saturate(dot(ng, l) * TERMINATOR_SHARPNESS);
}

fn cascade_containing(world_pos: vec3<f32>) -> u32 {
    for (var cascade = 0u; cascade < frame.flags.y; cascade = cascade + 1u) {
        let sphere = frame.cascade_spheres[cascade];
        if (distance(world_pos, sphere.xyz) < sphere.w) {
            return cascade;
        }
    }
    return NO_CASCADE;
}

// The `i`th of `taps` points spread evenly over the unit disc (a Vogel spiral).
fn disc_point(i: u32, taps: u32) -> vec2<f32> {
    let r = sqrt((f32(i) + 0.5) / f32(taps));
    let angle = f32(i) * GOLDEN_ANGLE;
    return vec2<f32>(cos(angle), sin(angle)) * r;
}

// The receiver's plane in a cascade's clip space: how its depth changes per unit of clip x and
// y. Lets taps far from the receiver compare against the plane instead of the receiver's own
// depth, so a wide filter does not shadow a surface with itself.
fn receiver_depth_slope(cascade: u32, ng: vec3<f32>) -> vec2<f32> {
    let m = frame.cascade_view_proj[cascade];
    let helper = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(ng.y) > 0.9);
    let t1 = normalize(cross(ng, helper));
    let t2 = cross(ng, t1);
    let a = (m * vec4<f32>(t1, 0.0)).xyz;
    let b = (m * vec4<f32>(t2, 0.0)).xyz;
    let det = a.x * b.y - a.y * b.x;
    if (abs(det) < MIN_PLANE_DETERMINANT) {
        return vec2<f32>(0.0);
    }
    let slope = vec2<f32>(a.z * b.y - a.y * b.z, a.x * b.z - a.z * b.x) / det;
    let radius = frame.cascade_spheres[cascade].w;
    let max_slope = MAX_SLOPE_BIAS_TANGENT * radius / cascade_depth_span(cascade);
    let length_slope = length(slope);
    return select(slope, slope * (max_slope / length_slope), length_slope > max_slope);
}

// World distance one unit of a cascade's clip depth stands for.
fn cascade_depth_span(cascade: u32) -> f32 {
    let m = frame.cascade_view_proj[cascade];
    return 1.0 / length(vec3<f32>(m[0].z, m[1].z, m[2].z));
}

fn plane_depth(depth: f32, slope: vec2<f32>, offset_uv: vec2<f32>) -> f32 {
    return depth + dot(slope, offset_uv * vec2<f32>(2.0, -2.0));
}

// Mean clip-depth lead of the casters in front of the receiver plane within the widest penumbra;
// negative when there are none.
fn mean_blocker_lead(uv: vec2<f32>, cascade: u32, depth: f32, slope: vec2<f32>) -> f32 {
    let size = vec2<f32>(textureDimensions(sun_shadow_maps));
    let radius_uv = SUN_MAX_PENUMBRA_TEXELS / size;
    var lead = 0.0;
    var blockers = 0.0;
    for (var i = 0u; i < SUN_BLOCKER_SEARCH_TAPS; i = i + 1u) {
        let offset = disc_point(i, SUN_BLOCKER_SEARCH_TAPS) * radius_uv;
        let texel = clamp(vec2<i32>((uv + offset) * size), vec2<i32>(0), vec2<i32>(size) - 1);
        let caster = textureLoad(sun_shadow_maps, texel, cascade, 0);
        let receiver = plane_depth(depth, slope, offset);
        if (caster > receiver) {
            lead += caster - receiver;
            blockers += 1.0;
        }
    }
    if (blockers == 0.0) {
        return -1.0;
    }
    return lead / blockers;
}

// Contact-hardening sun shadow: the sun is a disc, so an edge softens with the distance from its
// caster. Hardware-filtered compare taps over a disc whose radius is that penumbra.
fn filtered_shadow(uv: vec2<f32>, cascade: u32, depth: f32, slope: vec2<f32>) -> f32 {
    let taps = frame.shadow_filter.x;
    if (taps <= 1u) {
        return textureSampleCompareLevel(sun_shadow_maps, shadow_comparison_sampler, uv, cascade, depth);
    }
    let lead = mean_blocker_lead(uv, cascade, depth, slope);
    if (lead < 0.0) {
        return 1.0;
    }
    let cos_radius = frame.to_sun.w;
    let tan_radius = sqrt(1.0 - cos_radius * cos_radius) / cos_radius;
    let penumbra_world = lead * cascade_depth_span(cascade) * tan_radius;
    let penumbra_texels = clamp(
        penumbra_world / frame.cascade_texel_world[cascade],
        SUN_MIN_PENUMBRA_TEXELS,
        SUN_MAX_PENUMBRA_TEXELS,
    );
    let radius_uv = penumbra_texels / vec2<f32>(textureDimensions(sun_shadow_maps));
    var lit = 0.0;
    for (var i = 0u; i < taps; i = i + 1u) {
        let offset = disc_point(i, taps) * radius_uv;
        lit += textureSampleCompareLevel(
            sun_shadow_maps,
            shadow_comparison_sampler,
            uv + offset,
            cascade,
            plane_depth(depth, slope, offset),
        );
    }
    return lit / f32(taps);
}

// Receiver-side biases in texels of the shadow map at the receiver (x
// constant, y slope, z normal offset): constant and slope push the receiver
// towards the light, the normal offset along the geometric normal.
fn biased_receiver(world_pos: vec3<f32>, ng: vec3<f32>, l: vec3<f32>, texel: f32, bias: vec3<f32>) -> vec3<f32> {
    let n_dot_l = saturate(dot(ng, l));
    let slope = min(sqrt(1.0 - n_dot_l * n_dot_l) / max(n_dot_l, MIN_NO_L), MAX_SLOPE_BIAS_TANGENT);
    return world_pos
        + ng * (bias.z * texel * (1.0 - n_dot_l))
        + l * ((bias.x + bias.y * slope) * texel);
}

fn cascade_visibility(cascade: u32, world_pos: vec3<f32>, ng: vec3<f32>, l: vec3<f32>) -> f32 {
    let biased = biased_receiver(world_pos, ng, l, frame.cascade_texel_world[cascade], frame.shadow_params.xyz);
    let clip = frame.cascade_view_proj[cascade] * vec4<f32>(biased, 1.0);
    let uv = clip.xy * vec2<f32>(0.5, -0.5) + 0.5;
    return filtered_shadow(uv, cascade, clip.z, receiver_depth_slope(cascade, ng));
}

// 1 = fully lit. Blends into the next cascade across the split band and fades
// out over the outer part of the last cascade.
fn sun_shadow(world_pos: vec3<f32>, ng: vec3<f32>, l: vec3<f32>) -> f32 {
    if (frame.flags.x == 0u) {
        return 1.0;
    }
    let cascade = cascade_containing(world_pos);
    if (cascade == NO_CASCADE) {
        return 1.0;
    }
    let visibility = cascade_visibility(cascade, world_pos, ng, l);
    let sphere = frame.cascade_spheres[cascade];
    let to_edge = sphere.w - distance(world_pos, sphere.xyz);
    let next = cascade + 1u;
    if (next == frame.flags.y) {
        return mix(1.0, visibility, saturate(to_edge / (sphere.w * LAST_CASCADE_FADE_FRACTION)));
    }
    let blend_band = frame.shadow_params.w;
    let next_sphere = frame.cascade_spheres[next];
    if (to_edge >= blend_band || distance(world_pos, next_sphere.xyz) >= next_sphere.w) {
        return visibility;
    }
    return mix(cascade_visibility(next, world_pos, ng, l), visibility, to_edge / blend_band);
}

fn cascade_debug_tint(world_pos: vec3<f32>) -> vec3<f32> {
    if (frame.flags.x == 0u) {
        return vec3<f32>(1.0);
    }
    switch cascade_containing(world_pos) {
        case 0u: { return vec3<f32>(1.0, 0.35, 0.35); }
        case 1u: { return vec3<f32>(0.35, 1.0, 0.35); }
        case 2u: { return vec3<f32>(0.35, 0.35, 1.0); }
        case 3u: { return vec3<f32>(1.0, 1.0, 0.35); }
        default: { return vec3<f32>(1.0); }
    }
}

struct Fog {
    color: vec3<f32>,
    opacity: f32,
}

// Bethesda-style parametric fog. A zero maximum opacity disables it.
fn fog_at(world_pos: vec3<f32>) -> Fog {
    let near = frame.fog_near_color.w;
    let far = frame.fog_far_color.w;
    let power = frame.fog_params.x;
    let max_opacity = frame.fog_params.y;
    let distance_to_camera = distance(frame.camera_pos.xyz, world_pos);
    let t = saturate((distance_to_camera - near) / max(far - near, FOG_MIN_RANGE));
    return Fog(mix(frame.fog_near_color.rgb, frame.fog_far_color.rgb, t), min(pow(t, power), max_opacity));
}

fn apply_fog(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    let fog = fog_at(world_pos);
    return mix(color, fog.color, fog.opacity);
}

// Share of a surface's own light that survives the fog.
fn fog_transmittance(world_pos: vec3<f32>) -> f32 {
    return 1.0 - fog_at(world_pos).opacity;
}

fn light_cluster(p: ShadingPoint) -> u32 {
    let grid = frame.cluster_grid;
    let tile = min(vec2<u32>(p.frag_xy * frame.viewport.zw * vec2<f32>(grid.xy)), grid.xy - 1u);
    let depth = dot(p.world_pos - frame.camera_pos.xyz, frame.camera_forward.xyz);
    let slice = floor(log2(max(depth, MIN_CLUSTER_DEPTH)) * frame.cluster_z.z - frame.cluster_z.w) + 1.0;
    return cluster_index(tile, u32(clamp(slice, 0.0, f32(grid.z - 1u))), grid.x);
}

const LIGHT_FALLOFF_POWER = 2.2;
const MAX_DISTANCE_POWER = 1e30;
const MIN_CONE_COSINE_SPAN = 1e-6;
const MIN_CONE_SIN_SQ = 1e-12;
const MIN_CONE_TAN_DIVISOR = 1e-30;

// Distance falloff of a placed light, `distance` in engine units: the share of
// light lost, raised to a power. It is the larger of the light's own loss (its
// constant plus scalar times the normalised distance raised to its exponent) and
// the squared normalised distance, so the falloff never exceeds
// (1 - normalised^2)^power and reaches 0 with zero slope at the radius whatever
// the record says. Zero from the radius outwards.
fn local_light_attenuation(light: VisibleLight, distance: f32) -> f32 {
    let normalised = distance / light.radius;
    let window_loss = normalised * normalised;
    let distance_power = min(pow_or_zero(normalised, light.attenuation.z), MAX_DISTANCE_POWER);
    let record_loss = saturate(light.attenuation.x + light.attenuation.y * distance_power);
    return pow_or_zero(1.0 - saturate(max(record_loss, window_loss)), LIGHT_FALLOFF_POWER);
}

// Falloff of a spot light without a lamp shadow: the cosine of the angle off
// the axis rescaled to 1 on the axis and 0 at the cone edge, raised to the
// exponent. The rescaled cosine stays 0 past the edge, so the result is 0
// outside the cone whatever `cos_outer` is.
fn spot_cosine_falloff(cos_axis: f32, cos_outer: f32, exponent: f32) -> f32 {
    let inside = saturate(1.0 - (1.0 - cos_axis) / max(1.0 - cos_outer, MIN_CONE_COSINE_SPAN));
    return pow_or_zero(inside, max(exponent, 0.0));
}

// Falloff of a spot light with a lamp shadow: 1 - min(1, r^exponent), where r is
// the tangent of the angle off the axis over the tangent of the cone half-angle.
// 1 on the axis, 0 from the cone edge outwards and 0 at or behind the plane
// through the light perpendicular to its axis.
fn spot_radial_falloff(cos_axis: f32, cos_outer: f32, exponent: f32) -> f32 {
    if (cos_axis <= 0.0) {
        return 0.0;
    }
    let sin_sq_axis = saturate(1.0 - cos_axis * cos_axis);
    let sin_sq_outer = max(1.0 - cos_outer * cos_outer, MIN_CONE_SIN_SQ);
    let radial_sq = sin_sq_axis * cos_outer * cos_outer / max(cos_axis * cos_axis * sin_sq_outer, MIN_CONE_TAN_DIVISOR);
    if (radial_sq <= 0.0) {
        return 1.0;
    }
    let radial_power_log2 = min(0.5 * max(exponent, 0.0) * log2(radial_sq), 0.0);
    return 1.0 - exp2(radial_power_log2);
}

// Angular falloff of a placed light; `l` points from the receiver towards the light.
fn local_light_cone(light: VisibleLight, l: vec3<f32>) -> f32 {
    let cos_axis = -dot(l, light.direction);
    if (is_spot_light(light.flags)) {
        let has_lamp_shadow = (light.flags & LIGHT_FLAG_SHADOW_SPOT) != 0u && light.shadow_slot != SHADOW_SLOT_NONE;
        if (has_lamp_shadow) {
            return spot_radial_falloff(cos_axis, light.spot_cos_outer, light.spot_exponent);
        }
        return spot_cosine_falloff(cos_axis, light.spot_cos_outer, light.spot_exponent);
    }
    if (is_hemisphere_light(light.flags)) {
        return select(0.0, 1.0, cos_axis >= 0.0);
    }
    return 1.0;
}

fn reaches(light: VisibleLight, p: ShadingPoint, to_light: vec3<f32>) -> bool {
    return (light.flags & p.excluded_light_flags) == 0u && dot(to_light, to_light) < light.radius * light.radius;
}

fn local_light_shadow(light: VisibleLight, world_pos: vec3<f32>, ng: vec3<f32>, l: vec3<f32>) -> f32 {
    if (light.shadow_slot == SHADOW_SLOT_NONE) {
        return 1.0;
    }
    return lamp_shadow(light.shadow_slot, world_pos, ng, l);
}

fn local_lights(s: SurfaceSample, p: ShadingPoint, v: vec3<f32>) -> LightContribution {
    var total = LightContribution(vec3<f32>(0.0), vec3<f32>(0.0));
    if (!local_lights_enabled()) {
        return total;
    }
    let cluster = light_cluster(p);
    let count = cluster_light_counts[cluster];
    let list_start = cluster * MAX_LIGHTS_PER_CLUSTER;
    for (var i = 0u; i < count; i = i + 1u) {
        let light = visible_lights[cluster_light_indices[list_start + i]];
        let to_light = light.position - p.world_pos;
        if (!reaches(light, p, to_light)) {
            continue;
        }
        let distance = max(length(to_light), MIN_LIGHT_DISTANCE);
        let l = to_light / distance;
        var attenuation = local_light_attenuation(light, distance) * local_light_cone(light, l) * terminator(s.ng, l);
        if (attenuation > 0.0) {
            attenuation *= local_light_shadow(light, p.world_pos, s.ng, l);
        }
        let lit = brdf_local(s, l, v, light.color * attenuation, light);
        total.diffuse += lit.diffuse;
        total.specular += lit.specular;
    }
    return total;
}

fn sun_and_ambient(s: SurfaceSample, world_pos: vec3<f32>, v: vec3<f32>) -> LightContribution {
    let l = frame.to_sun.xyz;
    let shadow = sun_shadow(world_pos, s.ng, l);
    let direct = brdf_direct(s, l, v, frame.sun_illuminance.rgb * terminator(s.ng, l) * shadow);
    let ambient = brdf_ambient(s, v);
    return LightContribution(direct.diffuse + ambient.diffuse, direct.specular + ambient.specular);
}

fn view_direction(world_pos: vec3<f32>) -> vec3<f32> {
    return normalize(frame.camera_pos.xyz - world_pos);
}

// Clamped at zero so a negative-coloured light can only remove light.
fn surface_lighting(s: SurfaceSample, p: ShadingPoint) -> LightContribution {
    let v = view_direction(p.world_pos);
    let global = sun_and_ambient(s, p.world_pos, v);
    let local = local_lights(s, p, v);
    return LightContribution(
        max(global.diffuse + local.diffuse, vec3<f32>(0.0)),
        max(global.specular + local.specular, vec3<f32>(0.0)),
    );
}

// Blue (empty) through green to red at HEATMAP_FULL_SCALE lights; magenta
// where the cluster list is full.
fn light_count_heatmap(p: ShadingPoint) -> vec3<f32> {
    if (!local_lights_enabled()) {
        return vec3<f32>(0.0);
    }
    let count = cluster_light_counts[light_cluster(p)];
    if (count >= MAX_LIGHTS_PER_CLUSTER) {
        return HEATMAP_OVERFLOW_COLOR;
    }
    let t = saturate(f32(count) / HEATMAP_FULL_SCALE);
    let lower = mix(HEATMAP_EMPTY_COLOR, HEATMAP_MID_COLOR, saturate(t * 2.0));
    let upper = mix(HEATMAP_MID_COLOR, HEATMAP_FULL_COLOR, saturate(t * 2.0 - 1.0));
    return select(lower, upper, t > 0.5);
}

// Each shadowed lamp reaching the fragment in its slot's colour, darkened
// where its shadow falls.
fn lamp_shadow_slots(s: SurfaceSample, p: ShadingPoint) -> vec3<f32> {
    var color = SLOT_VIEW_BACKGROUND;
    if (!local_lights_enabled()) {
        return color;
    }
    let cluster = light_cluster(p);
    let count = cluster_light_counts[cluster];
    let list_start = cluster * MAX_LIGHTS_PER_CLUSTER;
    for (var i = 0u; i < count; i = i + 1u) {
        let light = visible_lights[cluster_light_indices[list_start + i]];
        let to_light = light.position - p.world_pos;
        if (light.shadow_slot == SHADOW_SLOT_NONE || !reaches(light, p, to_light)) {
            continue;
        }
        let l = to_light / max(length(to_light), MIN_LIGHT_DISTANCE);
        if (local_light_cone(light, l) <= 0.0) {
            continue;
        }
        let shadow = local_light_shadow(light, p.world_pos, s.ng, l);
        color += SLOT_PALETTE[light.shadow_slot % SLOT_PALETTE_SIZE] * mix(SLOT_VIEW_SHADOWED, 1.0, shadow);
    }
    return color;
}

// Normal lighting with a debug colour multiplied in, so surface detail stays visible.
fn tinted_lighting(s: SurfaceSample, p: ShadingPoint, tint: vec3<f32>) -> vec3<f32> {
    let lit = surface_lighting(s, p);
    return (lit.diffuse + lit.specular) * tint;
}

fn shade_surface(s: SurfaceSample, p: ShadingPoint, emissive: vec3<f32>, mip: MipInfo) -> vec3<f32> {
    let world_pos = p.world_pos;
    switch debug_view() {
        case DEBUG_VIEW_ALBEDO: {
            return s.albedo;
        }
        case DEBUG_VIEW_NORMALS: {
            return s.n * 0.5 + 0.5;
        }
        case DEBUG_VIEW_LIGHTING_ONLY: {
            var unlit_white = s;
            unlit_white.albedo = vec3<f32>(1.0);
            let lit = surface_lighting(unlit_white, p);
            return lit.diffuse + lit.specular;
        }
        case DEBUG_VIEW_SPECULAR_ONLY: {
            return surface_lighting(s, p).specular;
        }
        case DEBUG_VIEW_SHADOW_CASCADES: {
            return tinted_lighting(s, p, cascade_debug_tint(world_pos));
        }
        case DEBUG_VIEW_LIGHT_COUNT_HEATMAP: {
            return light_count_heatmap(p);
        }
        case DEBUG_VIEW_LAMP_SHADOW_SLOTS: {
            return lamp_shadow_slots(s, p);
        }
        case DEBUG_VIEW_STREAMED_MIP: {
            return tinted_lighting(s, p, mip_tint(mip.streamed));
        }
        case DEBUG_VIEW_SAMPLED_MIP: {
            return tinted_lighting(s, p, mip_tint(mip.sampled));
        }
        case DEBUG_VIEW_MIP_DEFICIT: {
            return tinted_lighting(s, p, mip_deficit_tint(mip));
        }
        case DEBUG_VIEW_TEXTURE_WASTE: {
            return tinted_lighting(s, p, texture_waste_tint(mip));
        }
        case DEBUG_VIEW_ENV_REFLECTION: {
            return vec3<f32>(0.0);
        }
        case DEBUG_VIEW_EFFECT_CLASS: {
            return CLASS_OTHER;
        }
        case DEBUG_VIEW_LOCAL_LIGHTS_ONLY: {
            var unlit_white = s;
            unlit_white.albedo = vec3<f32>(1.0);
            let lit = local_lights(unlit_white, p, view_direction(world_pos));
            return max(lit.diffuse + lit.specular, vec3<f32>(0.0));
        }
        default: {
            let lit = surface_lighting(s, p);
            return apply_fog(lit.diffuse + lit.specular + emissive, world_pos);
        }
    }
}
