// Shadow casters of every shadow view (sun cascades and lamp views). Never
// binds the frame group: it samples the maps these passes render. Group 0
// (materials) and group 2 (instances) come from scene_bindings.wgsl.

struct ShadowViewUniform {
    view_proj: mat4x4<f32>,
};

@group(1) @binding(0) var<uniform> shadow_view: ShadowViewUniform;

struct AlphaTestVaryings {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) @interpolate(flat) instance_id: u32,
};

fn object_clip_position(position: vec3<f32>, instance_id: u32) -> vec4<f32> {
    return shadow_view.view_proj * (instances[instance_id].transform * vec4<f32>(position, 1.0));
}

@vertex
fn vs_shadow_opaque(vertex: VertexInput, @builtin(instance_index) instance_id: u32) -> @builtin(position) vec4<f32> {
    return object_clip_position(vertex.position, instance_id);
}

@vertex
fn vs_shadow_alpha_test(vertex: VertexInput, @builtin(instance_index) instance_id: u32) -> AlphaTestVaryings {
    var out: AlphaTestVaryings;
    out.clip_position = object_clip_position(vertex.position, instance_id);
    out.tex_coords = vertex.tex_coords;
    out.instance_id = instance_id;
    return out;
}

// Same cut-out rule as fs_alphatest in object.wgsl.
@fragment
fn fs_shadow_alpha_test(in: AlphaTestVaryings) {
    let inst = instances[in.instance_id];
    let mat = materials[inst.material_id];
    let alpha = textureSample(textures[mat.diffuse_index], shadow_sampler, in.tex_coords).a * inst.alpha_mult;
    if (alpha < inst.alpha_cutoff) {
        discard;
    }
}

@vertex
fn vs_shadow_terrain(vertex: VertexInput) -> @builtin(position) vec4<f32> {
    return shadow_view.view_proj * vec4<f32>(vertex.position, 1.0);
}
