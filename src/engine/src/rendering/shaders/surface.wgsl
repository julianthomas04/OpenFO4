// Shading inputs every textured surface shader builds the same way.

struct SurfaceNormals {
    shading: vec3<f32>,
    geometric: vec3<f32>,
}

// Back faces of two-sided geometry are lit as their own side. Uses derivatives
// (tangent frame), so it must be called in uniform control flow.
fn two_sided_normals(
    world_normal: vec3<f32>,
    world_tangent: vec3<f32>,
    tangent_w: f32,
    world_pos: vec3<f32>,
    uv: vec2<f32>,
    front_facing: bool,
    normal_sample: vec4<f32>,
) -> SurfaceNormals {
    let ng = safe_normalize(world_normal);
    let tbn = tangent_frame(ng, world_tangent, tangent_w, world_pos, uv);
    let face_sign = select(-1.0, 1.0, front_facing);
    return SurfaceNormals(
        safe_normalize(tbn * decode_normal_map(normal_sample)) * face_sign,
        ng * face_sign,
    );
}

// `_s` maps: R = specular intensity, G = smoothness.
fn surface_sample(mat: Material, albedo: vec3<f32>, normals: SurfaceNormals, specular_map: vec4<f32>) -> SurfaceSample {
    return SurfaceSample(
        albedo,
        normals.shading,
        normals.geometric,
        mat.smoothness * specular_map.g,
        mat.specular_scale * specular_map.r,
    );
}
