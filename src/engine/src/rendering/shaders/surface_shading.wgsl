struct LightContribution {
    diffuse: vec3<f32>,
    specular: vec3<f32>,
}

const AMBIENT_REFLECTION_STRENGTH = 0.25;
const AMBIENT_REFLECTION_GRAZING_EXPONENT = 3.0;

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
