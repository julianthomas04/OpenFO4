// ---------- Object vertex shader ----------

fn handedness(m: mat3x3<f32>) -> f32 {
    return select(1.0, -1.0, determinant(m) < 0.0);
}

// Transpose of the inverse scaled by |det|: correct under non-uniform scale.
fn cofactor_normal_matrix(m: mat3x3<f32>) -> mat3x3<f32> {
    return mat3x3<f32>(cross(m[1], m[2]), cross(m[2], m[0]), cross(m[0], m[1])) * handedness(m);
}

// Shared by every object vertex entry point; the inputs are in the instance's model space.
fn object_vertex_output(
    position: vec3<f32>,
    normal: vec3<f32>,
    tangent: vec4<f32>,
    tex_coords: vec2<f32>,
    instance_id: u32,
) -> VertexOutput {
    let model = instances[instance_id].transform;
    let model3 = mat3x3<f32>(model[0].xyz, model[1].xyz, model[2].xyz);
    let world_pos = model * vec4<f32>(position, 1.0);

    var out: VertexOutput;
    out.tex_coords = tex_coords;
    out.clip_position = frame.view_proj * world_pos;
    out.world_pos = world_pos.xyz;
    out.world_normal = cofactor_normal_matrix(model3) * normal;
    out.world_tangent = model3 * tangent.xyz;
    out.tangent_w = tangent.w * handedness(model3);
    out.instance_id = instance_id;
    return out;
}

@vertex
fn vs_main(
    vertex: VertexInput,
    @builtin(instance_index) instance_id: u32,
) -> VertexOutput {
    return object_vertex_output(vertex.position, vertex.normal, vertex.tangent, vertex.tex_coords, instance_id);
}

// ---------- Object fragment shaders ----------

fn surface_normals(in: VertexOutput, front_facing: bool, normal_sample: vec4<f32>) -> SurfaceNormals {
    return two_sided_normals(in.world_normal, in.world_tangent, in.tangent_w, in.world_pos, in.tex_coords, front_facing, normal_sample);
}

const VIEW_OFF = 0u;

// What the surface shaders of this file shade one fragment from.
struct ObjectInputs {
    mat: Material,
    alpha_mult: f32,
    albedo: vec4<f32>,
    normals: SurfaceNormals,
    specular_map: vec4<f32>,
    emissive: vec3<f32>,
    mip: MipInfo,
    point: ShadingPoint,
    uv: vec2<f32>,
    uv_dx: vec2<f32>,
    uv_dy: vec2<f32>,
    // The entry point draws into a blend bucket.
    blended: bool,
}

struct ShadedObject {
    color: ShadedColor,
    // Coverage after falloff, unclamped.
    alpha: f32,
    // Texture alpha times the instance multiplier: what the cut-out test reads.
    cutout_alpha: f32,
    effect_flags: u32,
    transmittance: f32,
    // rgb: the colour a debug view draws instead; w: 1 while one replaces the shading.
    debug: vec4<f32>,
}

fn object_class_color(flags: u32, blended: bool) -> vec3<f32> {
    if (has_flag(flags, EFFECT_FLAG_EFFECT)) {
        if (has_flag(flags, EFFECT_FLAG_ADDITIVE)) {
            return CLASS_ADDITIVE_EFFECT;
        }
        if (has_flag(flags, EFFECT_FLAG_MULTIPLY)) {
            return CLASS_MULTIPLY_EFFECT;
        }
        return select(CLASS_OPAQUE_EFFECT, CLASS_ALPHA_EFFECT, blended);
    }
    return select(CLASS_OTHER, CLASS_LIT_ENV, has_flag(flags, EFFECT_FLAG_ENV));
}

// w = 0 when no debug view replaces the shading.
fn object_debug_color(o: ObjectInputs, surface: SurfaceSample, reflection: vec3<f32>) -> vec4<f32> {
    switch debug_view() {
        case VIEW_OFF: {
            return vec4<f32>(0.0);
        }
        case DEBUG_VIEW_ENV_REFLECTION: {
            return vec4<f32>(reflection, 1.0);
        }
        case DEBUG_VIEW_EFFECT_CLASS: {
            return vec4<f32>(object_class_color(o.mat.effect_flags, o.blended), 1.0);
        }
        default: {
            return vec4<f32>(shade_surface(surface, o.point, o.emissive, o.mip), 1.0);
        }
    }
}

fn effect_env_mask(o: ObjectInputs) -> f32 {
    if (!env_enabled(o.mat)) {
        return 1.0;
    }
    return textureSampleGrad(textures[o.mat.env_mask_index], material_sampler, o.uv, o.uv_dx, o.uv_dy).r;
}

fn shade_effect_object(o: ObjectInputs) -> ShadedObject {
    let v = view_direction(o.point.world_pos);
    let facing = effect_facing(o.normals.geometric, v);
    let layer = effect_layer(o.mat, o.alpha_mult, o.albedo, facing);
    let reflection = effect_env_reflection(o.mat, o.normals.shading, v, effect_env_mask(o));
    let surface = SurfaceSample(layer.albedo, o.normals.shading, o.normals.geometric, EFFECT_SURFACE_SMOOTHNESS, 0.0);
    let fog = fog_at(o.point.world_pos);
    let body = effect_body(o.mat, layer.color, o.normals, o.point);
    return ShadedObject(
        ShadedColor(body, mix(body, fog.color, fog.opacity), reflection),
        layer.alpha,
        o.albedo.a * o.alpha_mult,
        o.mat.effect_flags,
        1.0 - fog.opacity,
        object_debug_color(o, surface, reflection),
    );
}

fn shade_lit_object(o: ObjectInputs) -> ShadedObject {
    let v = view_direction(o.point.world_pos);
    let surface = surface_sample(o.mat, o.albedo.rgb, o.normals, o.specular_map);
    let reflection = lit_env_reflection(o.mat, surface, v, o.specular_map.r);
    let color = shade_surface(surface, o.point, o.emissive, o.mip);
    let alpha = o.albedo.a * o.alpha_mult;
    // Only the reflection is weighted by transmittance, so materials without one skip the fog lookup.
    var transmittance = 1.0;
    if (env_enabled(o.mat)) {
        transmittance = fog_transmittance(o.point.world_pos);
    }
    return ShadedObject(
        ShadedColor(color, color, reflection),
        alpha,
        alpha,
        o.mat.effect_flags,
        transmittance,
        object_debug_color(o, surface, reflection),
    );
}

// `report_feedback` and `blended` are constants per entry point: only fragments that cannot
// be discarded may report (see texture_feedback).
fn shade_object(in: VertexOutput, front_facing: bool, report_feedback: bool, blended: bool) -> ShadedObject {
    let inst = instances[in.instance_id];
    let mat = materials[inst.material_id];

    let albedo = textureSample(textures[mat.diffuse_index], material_sampler, in.tex_coords);
    let normals = surface_normals(in, front_facing, textureSample(textures[mat.normal_index], material_sampler, in.tex_coords));
    let specular_map = textureSample(textures[mat.specular_index], material_sampler, in.tex_coords);
    let glow = textureSample(textures[mat.glow_index], material_sampler, in.tex_coords).rgb;
    let uv_dx = dpdx(in.tex_coords);
    let uv_dy = dpdy(in.tex_coords);

    if (report_feedback) {
        texture_feedback(
            vec4<u32>(mat.diffuse_index, mat.normal_index, mat.specular_index, mat.glow_index),
            vec4<bool>(true),
            in.tex_coords,
            in.clip_position.xy,
        );
    }

    let inputs = ObjectInputs(
        mat,
        inst.alpha_mult,
        albedo,
        normals,
        specular_map,
        mat.emissive.rgb * glow,
        texture_mip_info(mat.diffuse_index, in.tex_coords),
        ShadingPoint(in.world_pos, in.clip_position.xy, RECEIVER_OBJECT),
        in.tex_coords,
        uv_dx,
        uv_dy,
        blended,
    );
    if (has_flag(mat.effect_flags, EFFECT_FLAG_EFFECT)) {
        return shade_effect_object(inputs);
    }
    return shade_lit_object(inputs);
}

fn has_debug_color(o: ShadedObject) -> bool {
    return o.debug.w > 0.0;
}

fn opaque_rgb(o: ShadedObject) -> vec3<f32> {
    return select(compose_opaque(o.color, o.transmittance), o.debug.rgb, has_debug_color(o));
}

// Opaque buckets only: no discard, so early depth tests can be forced, which
// keeps the feedback's storage writes from moving the depth test after shading.
@early_depth_test(force)
@fragment
fn fs_opaque(in: VertexOutput, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    return vec4<f32>(opaque_rgb(shade_object(in, front_facing, true, false)), 1.0);
}

// Alpha-test buckets: they discard, so no forced early depth and no feedback
// (the streaming estimate covers these surfaces).
@fragment
fn fs_alphatest(in: VertexOutput, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let inst = instances[in.instance_id];
    let o = shade_object(in, front_facing, false, false);
    if (o.cutout_alpha < inst.alpha_cutoff) {
        discard;
    }
    return vec4<f32>(opaque_rgb(o), o.alpha);
}

// Blend buckets, premultiplied output: alpha and additive effects and lit blended
// surfaces. A debug view draws the surface opaque.
@fragment
fn fs_blend(in: VertexOutput, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let inst = instances[in.instance_id];
    let o = shade_object(in, front_facing, false, true);
    if (o.cutout_alpha < inst.alpha_cutoff) {
        discard;
    }
    if (has_debug_color(o)) {
        return vec4<f32>(o.debug.rgb, 1.0);
    }
    return compose_blend(o.effect_flags, o.color, o.alpha, o.transmittance);
}

// Multiplicative buckets: the pipeline multiplies the destination by the output.
@fragment
fn fs_multiply(in: VertexOutput, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let inst = instances[in.instance_id];
    let o = shade_object(in, front_facing, false, true);
    if (o.cutout_alpha < inst.alpha_cutoff) {
        discard;
    }
    if (has_debug_color(o)) {
        return vec4<f32>(o.debug.rgb, 1.0);
    }
    return compose_multiply(o.color, o.alpha, o.transmittance);
}
