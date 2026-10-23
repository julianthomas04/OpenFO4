struct SurfaceSample {
    albedo: vec3<f32>,
    n: vec3<f32>,
    ng: vec3<f32>,
    smoothness: f32,
    spec_intensity: f32,
}

struct LightContribution {
    diffuse: vec3<f32>,
    specular: vec3<f32>,
}

// Ceiling of the specular intensity a surface can carry. Anything above
// SPECULAR_INTENSITY_LIMIT would push the specular peak (15 pi times the
// intensity) towards the half-float range.
override MAX_SPECULAR_INTENSITY: f32 = 1.0;
const SPECULAR_INTENSITY_LIMIT = 8.0;

const PI = 3.14159265;
const SPECULAR_LOBE_CAP = 15.0;
const SPECULAR_LOBE_SCALE = 0.25;
const MIN_HALF_VECTOR_LENGTH = 1e-4;
const MIN_COSINE_DIVISOR = 1e-12;

const OREN_NAYAR_FLATTEN_SCALE = 0.5;
const OREN_NAYAR_FLATTEN_OFFSET = 0.57;
const OREN_NAYAR_TILT_SCALE = 0.45;
const OREN_NAYAR_TILT_OFFSET = 0.09;

const RIM_GRAZING_EXPONENT = 0.01;
const FRESNEL_BASE = 0.2;
const SPECULAR_SHARPNESS_SLOPE = 10.0;
const SPECULAR_SHARPNESS_BASE = 1.0;

const AMBIENT_REFLECTION_STRENGTH = 0.25;
const AMBIENT_REFLECTION_GRAZING_EXPONENT = 3.0;

// Parts of the response a light can switch off; all false is the full response.
struct ResponseSwitches {
    // Drops the roughness term of the diffuse.
    flat_diffuse: bool,
    no_rim: bool,
    no_specular: bool,
}

const FULL_RESPONSE = ResponseSwitches();

// Scalar factors for a unit-coloured light; the caller applies colour and albedo.
struct SurfaceResponse {
    diffuse: f32,
    specular: f32,
}

fn surface_smoothness(s: SurfaceSample) -> f32 {
    return saturate(s.smoothness);
}

fn surface_roughness(s: SurfaceSample) -> f32 {
    return 1.0 - surface_smoothness(s);
}

fn specular_intensity_ceiling() -> f32 {
    return max(min(MAX_SPECULAR_INTENSITY, SPECULAR_INTENSITY_LIMIT), 0.0);
}

fn surface_specular_intensity(s: SurfaceSample) -> f32 {
    return min(max(s.spec_intensity, 0.0), specular_intensity_ceiling());
}

struct RoughnessCoefficients {
    // Diffuse at normal incidence relative to Lambert.
    flatten: f32,
    // Strength of the extra light towards the viewer-light overlap.
    tilt: f32,
}

fn roughness_coefficients(roughness: f32) -> RoughnessCoefficients {
    let roughness_sq = roughness * roughness;
    return RoughnessCoefficients(
        1.0 - OREN_NAYAR_FLATTEN_SCALE * roughness_sq / (roughness_sq + OREN_NAYAR_FLATTEN_OFFSET),
        OREN_NAYAR_TILT_SCALE * roughness_sq / (roughness_sq + OREN_NAYAR_TILT_OFFSET),
    );
}

// Product of the sines of two angles, from their cosines.
fn sine_product(cosine_a: f32, cosine_b: f32) -> f32 {
    return sqrt(saturate((1.0 - cosine_a * cosine_a) * (1.0 - cosine_b * cosine_b)));
}

// Overlap of the view and light directions projected into the tangent plane,
// left unnormalised on purpose, weighted by both sines over the larger cosine.
// `n_dot_l` > 0.
fn tangent_overlap(n_dot_l: f32, n_dot_v: f32, v_dot_l: f32) -> f32 {
    let overlap = max(v_dot_l - n_dot_v * n_dot_l, 0.0);
    let larger_cosine = max(max(n_dot_v, n_dot_l), MIN_COSINE_DIVISOR);
    return overlap * sine_product(n_dot_v, n_dot_l) / larger_cosine;
}

// Rough-surface diffuse times the cosine. `n_dot_l` > 0.
fn rough_diffuse(n_dot_l: f32, n_dot_v: f32, v_dot_l: f32, roughness: f32) -> f32 {
    let overlap = tangent_overlap(n_dot_l, n_dot_v, v_dot_l);
    let coefficients = roughness_coefficients(roughness);
    return (coefficients.flatten + coefficients.tilt * overlap) * n_dot_l;
}

// Light that wraps round the silhouette when light and viewer are on opposite sides.
fn rim_light(n_dot_l: f32, n_dot_v: f32, v_dot_l: f32, roughness: f32) -> f32 {
    return roughness * saturate(-v_dot_l) * pow_or_zero(1.0 - saturate(n_dot_v), RIM_GRAZING_EXPONENT) * saturate(n_dot_l);
}

fn schlick_reflectance(base_reflectance: f32, cosine: f32) -> f32 {
    return base_reflectance + (1.0 - base_reflectance) * pow5(1.0 - saturate(cosine));
}

// Microfacet specular with the cap applied to the lobe before intensity and the
// pi factor. Masking is divided by n.v analytically so it stays finite when the
// normal faces away from the viewer. `dot(s.n, l)` > 0.
fn specular_response(s: SurfaceSample, l: vec3<f32>, v: vec3<f32>) -> f32 {
    let bisector_sum = l + v;
    let bisector_length = length(bisector_sum);
    if (bisector_length < MIN_HALF_VECTOR_LENGTH) {
        return 0.0;
    }
    let h = bisector_sum / bisector_length;
    let n_dot_h = saturate(dot(s.n, h));
    let v_dot_h = saturate(dot(v, h));
    if (v_dot_h <= 0.0) {
        return 0.0;
    }
    let light_cosine = saturate(dot(s.n, l));
    let view_cosine = saturate(dot(s.n, v));
    let sharpness = exp2(SPECULAR_SHARPNESS_SLOPE * surface_smoothness(s) + SPECULAR_SHARPNESS_BASE);
    let facet_density = (sharpness + 2.0) / (2.0 * PI) * pow_or_zero(n_dot_h, sharpness);
    let masking_per_cosine = 2.0 * n_dot_h / v_dot_h;
    let light_masking = min(1.0, masking_per_cosine * light_cosine);
    let masking_per_view = min(light_masking / max(view_cosine, MIN_COSINE_DIVISOR), masking_per_cosine);
    let lobe = min(
        SPECULAR_LOBE_SCALE * facet_density * schlick_reflectance(FRESNEL_BASE, v_dot_h) * masking_per_view,
        SPECULAR_LOBE_CAP,
    );
    return PI * surface_specular_intensity(s) * light_cosine * lobe;
}

fn surface_response(s: SurfaceSample, l: vec3<f32>, v: vec3<f32>, switches: ResponseSwitches) -> SurfaceResponse {
    let n_dot_l = dot(s.n, l);
    if (n_dot_l <= 0.0) {
        return SurfaceResponse(0.0, 0.0);
    }
    let n_dot_v = dot(s.n, v);
    let v_dot_l = dot(v, l);
    let roughness = surface_roughness(s);
    let diffuse_roughness = select(roughness, 0.0, switches.flat_diffuse);
    var response = SurfaceResponse(rough_diffuse(n_dot_l, n_dot_v, v_dot_l, diffuse_roughness), 0.0);
    if (!switches.no_rim) {
        response.diffuse += rim_light(n_dot_l, n_dot_v, v_dot_l, roughness);
    }
    if (!switches.no_specular) {
        response.specular = specular_response(s, l, v);
    }
    return response;
}

fn response_contribution(s: SurfaceSample, light_color: vec3<f32>, response: SurfaceResponse) -> LightContribution {
    return LightContribution(s.albedo * light_color * response.diffuse, light_color * response.specular);
}

// The directional (sun) light. l points towards the light, v towards the viewer.
fn brdf_direct(s: SurfaceSample, l: vec3<f32>, v: vec3<f32>, light_color: vec3<f32>) -> LightContribution {
    return response_contribution(s, light_color, surface_response(s, l, v, FULL_RESPONSE));
}

fn local_response_switches(light: VisibleLight) -> ResponseSwitches {
    let flat_diffuse = (light.flags & LIGHT_FLAG_IGNORE_ROUGHNESS) != 0u;
    let no_rim = (light.flags & LIGHT_FLAG_NO_RIM_LIGHTING) != 0u
        || (flat_diffuse && is_directional_or_shadowed(light));
    let no_specular = (light.flags & LIGHT_FLAG_NON_SPECULAR) != 0u;
    return ResponseSwitches(flat_diffuse, no_rim, no_specular);
}

// A placed light. `light_color` already carries the attenuation; the light's
// flags choose the response.
fn brdf_local(s: SurfaceSample, l: vec3<f32>, v: vec3<f32>, light_color: vec3<f32>, light: VisibleLight) -> LightContribution {
    if ((light.flags & LIGHT_FLAG_ATTENUATION_ONLY) != 0u) {
        return LightContribution(s.albedo * light_color, vec3<f32>(0.0));
    }
    return response_contribution(s, light_color, surface_response(s, l, v, local_response_switches(light)));
}

fn ambient_diffuse_radiance(gradient: AmbientGradient, s: SurfaceSample) -> vec3<f32> {
    return s.albedo * ambient_toward(gradient, s.n);
}

// The environment seen in the mirror direction, weighted towards grazing views.
fn ambient_specular_radiance(gradient: AmbientGradient, s: SurfaceSample, v: vec3<f32>) -> vec3<f32> {
    let n_dot_v = dot(s.n, v);
    let reflected_view = 2.0 * n_dot_v * s.n - v;
    let grazing_weight = pow_or_zero(
        1.0 - saturate(n_dot_v),
        AMBIENT_REFLECTION_GRAZING_EXPONENT - surface_smoothness(s),
    );
    return surface_specular_intensity(s) * AMBIENT_REFLECTION_STRENGTH * grazing_weight * ambient_toward(gradient, reflected_view);
}

fn brdf_ambient(s: SurfaceSample, v: vec3<f32>) -> LightContribution {
    let gradient = ambient_gradient();
    return LightContribution(ambient_diffuse_radiance(gradient, s), ambient_specular_radiance(gradient, s, v));
}
