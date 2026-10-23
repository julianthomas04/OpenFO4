// Effect material shading and the premultiplied output of every blend class.

const EFFECT_SURFACE_SMOOTHNESS = 0.5;
const PALETTE_ROW = 0.5;
const PALETTE_LEVEL = 0.0;

fn has_flag(flags: u32, flag: u32) -> bool {
    return (flags & flag) != 0u;
}

// Colour before and after the view-dependent falloff, and the coverage after it.
struct EffectLayer {
    albedo: vec3<f32>,
    color: vec3<f32>,
    alpha: f32,
}

// Opacity along the facing ramp: `falloff` holds start and stop facing, then start and stop opacity.
// Equal thresholds make a step: facing at or above the threshold is the start opacity, as on a
// ramp whose facing meets its start.
fn falloff_opacity(falloff: vec4<f32>, facing: f32) -> f32 {
    let span = falloff.y - falloff.x;
    let is_step = span == 0.0;
    let ramp = saturate((facing - falloff.x) / select(span, 1.0, is_step));
    return mix(falloff.z, falloff.w, select(ramp, 1.0 - step(falloff.x, facing), is_step));
}

// The geometric normal is two-sided: a back face faces the viewer as much as a front face.
fn effect_facing(geometric_normal: vec3<f32>, view_direction: vec3<f32>) -> f32 {
    return abs(dot(geometric_normal, view_direction));
}

fn palette_uv(coordinate: f32) -> vec2<f32> {
    return vec2<f32>(saturate(coordinate), PALETTE_ROW);
}

// The palette recolours by the base luminance and drives the alpha by the base alpha.
fn palette_color_coordinate(base: vec4<f32>) -> f32 {
    return color_luminance(base.rgb);
}

fn palette_alpha_coordinate(base: vec4<f32>) -> f32 {
    return base.a;
}

fn palette_lookup(mat: Material, coordinate: f32) -> vec4<f32> {
    return textureSampleLevel(textures[mat.palette_index], material_sampler, palette_uv(coordinate), PALETTE_LEVEL);
}

fn apply_palette(flags: u32, base: vec4<f32>, by_luminance: vec4<f32>, by_alpha: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(
        select(base.rgb, by_luminance.rgb, has_flag(flags, EFFECT_FLAG_PALETTE_COLOR)),
        select(base.a, by_alpha.a, has_flag(flags, EFFECT_FLAG_PALETTE_ALPHA)),
    );
}

fn palette_layer(mat: Material, base: vec4<f32>) -> vec4<f32> {
    var by_luminance = base;
    var by_alpha = base;
    if (has_flag(mat.effect_flags, EFFECT_FLAG_PALETTE_COLOR)) {
        by_luminance = palette_lookup(mat, palette_color_coordinate(base));
    }
    if (has_flag(mat.effect_flags, EFFECT_FLAG_PALETTE_ALPHA)) {
        by_alpha = palette_lookup(mat, palette_alpha_coordinate(base));
    }
    return apply_palette(mat.effect_flags, base, by_luminance, by_alpha);
}

// Tint, alpha multiplier and the view-dependent falloff applied to the palette-resolved texel.
fn shape_effect_layer(flags: u32, falloff: vec4<f32>, tint: vec3<f32>, texel: vec4<f32>, alpha_mult: f32, facing: f32) -> EffectLayer {
    let albedo = texel.rgb * tint;
    var color = albedo;
    var alpha = texel.a * alpha_mult;
    if (has_flag(flags, EFFECT_FLAG_FALLOFF)) {
        let opacity = falloff_opacity(falloff, facing);
        alpha *= opacity;
        if (has_flag(flags, EFFECT_FLAG_FALLOFF_COLOR)) {
            color *= opacity;
        }
    }
    return EffectLayer(albedo, color, alpha);
}

fn effect_layer(mat: Material, alpha_mult: f32, base: vec4<f32>, facing: f32) -> EffectLayer {
    return shape_effect_layer(mat.effect_flags, mat.falloff, mat.effect_color.rgb, palette_layer(mat, base), alpha_mult, facing);
}

// Lighting influence 0 is the unlit colour, 1 the fully lit one; no specular term.
fn effect_body(mat: Material, color: vec3<f32>, normals: SurfaceNormals, point: ShadingPoint) -> vec3<f32> {
    let surface = SurfaceSample(color, normals.shading, normals.geometric, EFFECT_SURFACE_SMOOTHNESS, 0.0);
    let lit = surface_lighting(surface, point).diffuse;
    return mix(color, lit, saturate(mat.effect_color.w));
}

// Colour of a shaded fragment. `fogged_body` is for the classes that mix towards the fog
// colour, `body` for the ones that fade out with distance instead.
struct ShadedColor {
    body: vec3<f32>,
    fogged_body: vec3<f32>,
    reflection: vec3<f32>,
}

fn compose_opaque(c: ShadedColor, transmittance: f32) -> vec3<f32> {
    return c.fogged_body + c.reflection * transmittance;
}

// Premultiplied: the pipeline adds `rgb + dst * (1 - a)`. Additive writes a = 0.
fn compose_blend(flags: u32, c: ShadedColor, alpha: f32, transmittance: f32) -> vec4<f32> {
    if (has_flag(flags, EFFECT_FLAG_ADDITIVE)) {
        return vec4<f32>((c.body + c.reflection) * alpha * transmittance, 0.0);
    }
    let coverage = min(alpha, 1.0);
    return vec4<f32>(c.fogged_body * coverage + c.reflection * transmittance, coverage);
}

// The pipeline multiplies the destination by rgb.
fn compose_multiply(c: ShadedColor, alpha: f32, transmittance: f32) -> vec4<f32> {
    let strength = mix(vec3<f32>(1.0), c.body, saturate(alpha));
    return vec4<f32>(mix(vec3<f32>(1.0), strength, transmittance), 1.0);
}
