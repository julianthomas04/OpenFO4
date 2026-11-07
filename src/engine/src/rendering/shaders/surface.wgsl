// Shading inputs every textured surface shader builds the same way.

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
