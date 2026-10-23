// Group 0: materials + textures (bindless) + terrain splatmaps
@group(0) @binding(0) var textures: binding_array<texture_2d<f32>>;
@group(0) @binding(1) var material_sampler: sampler;
@group(0) @binding(2) var<storage, read> materials: array<Material>;
@group(0) @binding(3) var terrain_sampler: sampler;
@group(0) @binding(4) var splatmaps: texture_2d_array<f32>;
// Trilinear without anisotropy: depth-only alpha cut-outs gain nothing from it.
@group(0) @binding(5) var shadow_sampler: sampler;
// Mirrors SlotMips in gpu_driven.rs.
struct SlotMips {
    // Resident first file mip, PLACEHOLDER_FIRST_MIP for a stand-in view.
    first_mip: u32,
    // Skip the held texture feedback calls for, NO_FEEDBACK_SKIP for none.
    feedback_skip: u32,
}
@group(0) @binding(6) var<storage, read> slot_mips: array<SlotMips>;

// Group 2: instances
@group(2) @binding(0) var<storage, read> instances: array<Instance>;
@group(2) @binding(1) var<storage, read> terrain_instances: array<TerrainInstance>;
