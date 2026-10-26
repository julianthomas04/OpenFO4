struct TestCase {
    albedo_smoothness: vec4<f32>,
    normal_intensity: vec4<f32>,
    to_light: vec4<f32>,
    to_viewer: vec4<f32>,
    light_color: vec4<f32>,
    light_axis_cos_outer: vec4<f32>,
    attenuation_exponent: vec4<f32>,
    position_radius: vec4<f32>,
    ids: vec4<u32>,
}

struct TestResult {
    first: vec4<f32>,
    second: vec4<f32>,
}

const KIND_DIRECT = 0u;
const KIND_LOCAL = 1u;
const KIND_AMBIENT = 2u;
const KIND_ATTENUATION = 3u;
const KIND_CONE = 4u;
const KIND_AMBIENT_TOWARD = 5u;
const KIND_FALLOFF = 6u;
const KIND_COMPOSE_BLEND = 7u;
const KIND_COMPOSE_MULTIPLY = 8u;
const KIND_ENV_TERM = 9u;
const KIND_ENV_EXPOSURE = 10u;
const KIND_ENV_ENABLED = 11u;
const KIND_ENV_DIRECTION = 12u;
const KIND_PALETTE_COORDINATES = 13u;
const KIND_APPLY_PALETTE = 14u;
const KIND_EFFECT_LAYER = 15u;
const KIND_EFFECT_FACING = 16u;
const KIND_LIT_ENV_F0 = 17u;
const KIND_FOG = 18u;
const KIND_SKY_FOG = 19u;

@group(3) @binding(0) var<storage, read> test_cases: array<TestCase>;
@group(3) @binding(1) var<storage, read_write> test_results: array<TestResult>;

fn contribution_result(lit: LightContribution) -> TestResult {
    return TestResult(vec4<f32>(lit.diffuse, 0.0), vec4<f32>(lit.specular, 0.0));
}

fn scalar_result(value: f32) -> TestResult {
    return TestResult(vec4<f32>(value, 0.0, 0.0, 0.0), vec4<f32>(0.0));
}

fn color_alpha_result(color: vec4<f32>) -> TestResult {
    return TestResult(vec4<f32>(color.rgb, 0.0), vec4<f32>(color.a, 0.0, 0.0, 0.0));
}

fn shaded_color(c: TestCase) -> ShadedColor {
    return ShadedColor(c.albedo_smoothness.xyz, c.normal_intensity.xyz, c.to_light.xyz);
}

fn material_with(flags: u32, env_cube_index: u32) -> Material {
    return Material(
        0u, 0u, 0u, 0u, 0u, 0.0, 0.0, 0.0,
        vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0),
        env_cube_index, 0u, 1.0, flags, 0u, 0u, 0u, 0u,
    );
}

fn evaluate(c: TestCase) -> TestResult {
    let normal = c.normal_intensity.xyz;
    let sample = SurfaceSample(c.albedo_smoothness.xyz, normal, normal, c.albedo_smoothness.w, c.normal_intensity.w);
    let l = c.to_light.xyz;
    let v = c.to_viewer.xyz;
    let light = VisibleLight(
        c.position_radius.xyz,
        c.position_radius.w,
        c.light_color.xyz,
        c.ids.y,
        c.attenuation_exponent.xyz,
        c.attenuation_exponent.w,
        c.light_axis_cos_outer.xyz,
        c.light_axis_cos_outer.w,
        c.ids.z,
    );
    switch c.ids.x {
        case KIND_DIRECT: {
            return contribution_result(brdf_direct(sample, l, v, c.light_color.xyz));
        }
        case KIND_LOCAL: {
            return contribution_result(brdf_local(sample, l, v, c.light_color.xyz, light));
        }
        case KIND_AMBIENT: {
            return contribution_result(brdf_ambient(sample, v));
        }
        case KIND_ATTENUATION: {
            return scalar_result(local_light_attenuation(light, c.to_light.w));
        }
        case KIND_CONE: {
            return scalar_result(local_light_cone(light, l));
        }
        case KIND_FALLOFF: {
            return scalar_result(falloff_opacity(c.attenuation_exponent, c.to_light.w));
        }
        case KIND_COMPOSE_BLEND: {
            return color_alpha_result(compose_blend(c.ids.y, shaded_color(c), c.to_viewer.x, c.to_viewer.y));
        }
        case KIND_COMPOSE_MULTIPLY: {
            return color_alpha_result(compose_multiply(shaded_color(c), c.to_viewer.x, c.to_viewer.y));
        }
        case KIND_ENV_TERM: {
            let weight = env_weight(c.to_viewer.x, c.to_viewer.y, c.to_viewer.z, c.to_viewer.w);
            return TestResult(vec4<f32>(env_reflection_color(c.light_color.xyz, weight, c.position_radius.x), 0.0), vec4<f32>(weight, env_lod(c.position_radius.y, c.ids.w), 0.0, 0.0));
        }
        case KIND_ENV_EXPOSURE: {
            return scalar_result(env_exposure(ambient_gradient(), c.to_light.xyz));
        }
        case KIND_ENV_ENABLED: {
            return scalar_result(select(0.0, 1.0, env_enabled(material_with(c.ids.y, c.ids.z))));
        }
        case KIND_ENV_DIRECTION: {
            return TestResult(vec4<f32>(env_cube_direction(c.to_light.xyz), 0.0), vec4<f32>(0.0));
        }
        case KIND_PALETTE_COORDINATES: {
            let base = c.albedo_smoothness;
            let uv = palette_uv(c.to_light.x);
            return TestResult(
                vec4<f32>(palette_color_coordinate(base), palette_alpha_coordinate(base), uv.x, 0.0),
                vec4<f32>(uv.y, 0.0, 0.0, 0.0),
            );
        }
        case KIND_APPLY_PALETTE: {
            return color_alpha_result(apply_palette(c.ids.y, c.albedo_smoothness, c.normal_intensity, c.to_light));
        }
        case KIND_EFFECT_LAYER: {
            let layer = shape_effect_layer(c.ids.y, c.attenuation_exponent, c.albedo_smoothness.xyz, c.normal_intensity, c.to_viewer.x, c.to_viewer.y);
            return TestResult(vec4<f32>(layer.color, 0.0), vec4<f32>(layer.alpha, layer.albedo.xy, 0.0));
        }
        case KIND_EFFECT_FACING: {
            return scalar_result(effect_facing(c.to_light.xyz, c.to_viewer.xyz));
        }
        case KIND_LIT_ENV_F0: {
            let mat = Material(
                0u, 0u, 0u, 0u, c.ids.y, 0.0, 0.0, 0.0,
                vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0),
                0u, 0u, 1.0, 0u, 0u, 0u, 0u, 0u,
            );
            return scalar_result(lit_env_f0(mat, c.to_light.x));
        }
        case KIND_FOG: {
            let fog = fog_at(c.to_light.xyz);
            return TestResult(vec4<f32>(fog.color, 0.0), vec4<f32>(fog.opacity, 0.0, 0.0, 0.0));
        }
        case KIND_SKY_FOG: {
            return TestResult(
                vec4<f32>(apply_sky_fog(c.light_color.xyz, c.to_light.xyz), 0.0),
                vec4<f32>(sky_fog_weight(c.to_light.y), 0.0, 0.0, 0.0),
            );
        }
        case KIND_AMBIENT_TOWARD, default: {
            return TestResult(vec4<f32>(ambient_toward(ambient_gradient(), l), 0.0), vec4<f32>(0.0));
        }
    }
}

@compute @workgroup_size(64)
fn lighting_test_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x < arrayLength(&test_cases)) {
        test_results[id.x] = evaluate(test_cases[id.x]);
    }
}
