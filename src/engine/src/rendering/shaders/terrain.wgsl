// Landscape carries no specular data: fully matte.
const TERRAIN_SPEC_INTENSITY = 0.0;
const TERRAIN_SMOOTHNESS = 0.0;
// Layers blended in below this weight do not report their mip.
const TERRAIN_FEEDBACK_MIN_WEIGHT = 0.1;

// ---------- Terrain vertex shader ----------

@vertex
fn vs_terrain(
    vertex: VertexInput,
    @builtin(instance_index) instance_id: u32,
) -> VertexOutput {
    var out: VertexOutput;
    out.tex_coords = vertex.tex_coords;
    out.clip_position = frame.view_proj * vec4<f32>(vertex.position, 1.0);
    out.world_normal = vertex.normal;
    out.world_tangent = vertex.tangent.xyz;
    out.tangent_w = vertex.tangent.w;
    out.instance_id = instance_id;
    out.world_pos = vertex.position;
    return out;
}

// ---------- Terrain fragment shader ----------

fn dominant_layer(weights: vec4<f32>) -> u32 {
    var layer = 0u;
    for (var i = 1u; i < 4u; i++) {
        layer = select(layer, i, weights[i] > weights[layer]);
    }
    return layer;
}

@early_depth_test(force)
@fragment
fn fs_terrain(in: VertexOutput) -> @location(0) vec4<f32> {
    let inst = terrain_instances[in.instance_id];
    let baked = (inst.flags.x & 1u) != 0u;

    // Splatmap: clamp sampler (terrain_sampler) — Repeat would bleed opposite
    // edges across cell borders. Weights normalized: overlapping layer alphas
    // can sum past 1 and would render overbright.
    let w_raw = textureSample(splatmaps, terrain_sampler, in.tex_coords, inst.splat_layer);
    let w_sum = w_raw.r + w_raw.g + w_raw.b + w_raw.a;
    var w = select(w_raw / w_sum, vec4<f32>(1.0, 0.0, 0.0, 0.0), w_sum < 1e-4);
    // Baked tile: all weight on layer x; the splat sample above ran with
    // tile-space UVs (garbage by design) and is fully discarded here. select()
    // keeps every textureSample in uniform control flow.
    w = select(w, vec4<f32>(1.0, 0.0, 0.0, 0.0), baked);

    // No fract(): Repeat addressing wraps identically, but fract() makes the
    // UV derivatives explode at tile seams (forces the smallest mip -> lines).
    // Splat layers tile 32x per cell; a baked tile maps 1:1 to its UV sub-rect.
    let t = select(in.tex_coords * 32.0, in.tex_coords, baked);

    let color = textureSample(textures[inst.layer_indices.x], material_sampler, t).rgb * w.r
              + textureSample(textures[inst.layer_indices.y], material_sampler, t).rgb * w.g
              + textureSample(textures[inst.layer_indices.z], material_sampler, t).rgb * w.b
              + textureSample(textures[inst.layer_indices.w], material_sampler, t).rgb * w.a;

    // The baked tile has all its weight on its one layer.
    texture_feedback(inst.layer_indices, w > vec4<f32>(TERRAIN_FEEDBACK_MIN_WEIGHT), t, in.clip_position.xy);

    let mip = texture_mip_info(inst.layer_indices[dominant_layer(w)], t);

    let n = normalize(in.world_normal);
    let surface = SurfaceSample(color, n, n, TERRAIN_SMOOTHNESS, TERRAIN_SPEC_INTENSITY);
    let point = ShadingPoint(in.world_pos, in.clip_position.xy, RECEIVER_LANDSCAPE);
    return vec4<f32>(shade_surface(surface, point, vec3<f32>(0.0), mip), 1.0);
}
