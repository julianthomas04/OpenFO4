// Environment cube reflection shared by effect and lit materials.

const ENV_F0_DIELECTRIC = 0.04;
const ENV_EFFECT_ROUGHNESS = 0.5;
// Ambient luminance at which the daytime cubes reflect at full strength.
const ENV_AMBIENT_REFERENCE = 0.5;
const LUMINANCE_WEIGHTS = vec3<f32>(0.2126, 0.7152, 0.0722);
const NO_ENV_CUBE = 0u;

fn color_luminance(color: vec3<f32>) -> f32 {
    return dot(color, LUMINANCE_WEIGHTS);
}

fn srgb_color_to_linear(srgb: vec3<f32>) -> vec3<f32> {
    let low = srgb / 12.92;
    let high = pow((srgb + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, srgb <= vec3<f32>(0.04045));
}

// Cubes are authored in Bethesda space (Z up); engine space maps (x, y, z) to (x, z, -y).
fn env_cube_direction(engine_direction: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(engine_direction.x, -engine_direction.z, engine_direction.y);
}

// The shipped cubes are fixed daytime captures; the frame's ambient light scales them so
// reflections dim at night and indoors.
fn env_exposure(gradient: AmbientGradient, direction: vec3<f32>) -> f32 {
    return clamp(color_luminance(ambient_toward(gradient, direction)) / ENV_AMBIENT_REFERENCE, 0.0, 1.0);
}

fn env_lod(roughness_lod: f32, level_count: u32) -> f32 {
    let last_level = f32(max(level_count, 1u) - 1u);
    return clamp(roughness_lod * last_level, 0.0, last_level);
}

fn env_reflection_color(radiance: vec3<f32>, weight: f32, exposure: f32) -> vec3<f32> {
    return radiance * weight * exposure;
}

fn env_weight(f0: f32, n_dot_v: f32, mask: f32, mask_scale: f32) -> f32 {
    return schlick_reflectance(f0, n_dot_v) * mask * mask_scale;
}

fn env_enabled(mat: Material) -> bool {
    return (mat.effect_flags & EFFECT_FLAG_ENV) != 0u && mat.env_cube_index != NO_ENV_CUBE;
}

// `n` and `v` are unit vectors in world space, `v` towards the eye.
fn env_reflection(mat: Material, n: vec3<f32>, v: vec3<f32>, f0: f32, mask: f32, roughness_lod: f32) -> vec3<f32> {
    if (!env_enabled(mat)) {
        return vec3<f32>(0.0);
    }
    let direction = reflect(-v, n);
    let cube = env_cubes[mat.env_cube_index];
    let lod = env_lod(roughness_lod, textureNumLevels(cube));
    let radiance = textureSampleLevel(cube, material_sampler, env_cube_direction(direction), lod).rgb;
    let weight = env_weight(f0, dot(n, v), mask, mat.env_scale);
    return env_reflection_color(radiance, weight, env_exposure(ambient_gradient(), direction));
}

// Dielectric glass reflects 4% at normal incidence.
fn effect_env_reflection(mat: Material, n: vec3<f32>, v: vec3<f32>, mask: f32) -> vec3<f32> {
    return env_reflection(mat, n, v, ENV_F0_DIELECTRIC, mask, ENV_EFFECT_ROUGHNESS);
}

fn lit_env_f0(mat: Material, spec_intensity: f32) -> f32 {
    let specular_tint = srgb_color_to_linear(unpack4x8unorm(mat.specular_color).rgb);
    return max(ENV_F0_DIELECTRIC, color_luminance(specular_tint) * saturate(spec_intensity));
}

fn lit_env_reflection(mat: Material, s: SurfaceSample, v: vec3<f32>, mask: f32) -> vec3<f32> {
    return env_reflection(mat, s.n, v, lit_env_f0(mat, s.spec_intensity), mask, surface_roughness(s));
}
