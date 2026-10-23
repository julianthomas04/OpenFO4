use anyhow::Context;

use crate::rendering::{
    cube_array::CUBE_SLOTS,
    gpu_driven::{DRAW_BUCKET_COUNT, MAX_TEXTURES},
};

/// The object-cull compute pass binds 1 instance buffer + one draw buffer per
/// bucket + 1 counts buffer as storage buffers in a single shader stage, the
/// most of any pipeline. Decal buckets push this past wgpu's conservative
/// default of 8.
pub const STORAGE_BUFFERS_PER_STAGE: u32 = DRAW_BUCKET_COUNT as u32 + 2;

/// Device features the renderer cannot run without; startup shader validation derives its
/// capabilities from this set.
pub const REQUIRED_FEATURES: wgpu::Features = wgpu::Features::TEXTURE_COMPRESSION_BC
    .union(wgpu::Features::MULTI_DRAW_INDIRECT_COUNT)
    .union(wgpu::Features::INDIRECT_FIRST_INSTANCE)
    .union(wgpu::Features::TEXTURE_BINDING_ARRAY)
    .union(wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING)
    .union(wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY)
    .union(wgpu::Features::SHADER_EARLY_DEPTH_TEST)
    .union(crate::rendering::water::displacement::REQUIRED_FORMAT_FEATURES);

pub async fn request_device(
    adapter: &wgpu::Adapter,
) -> anyhow::Result<(wgpu::Device, wgpu::Queue)> {
    let optional_features = adapter.features() & wgpu::Features::TIMESTAMP_QUERY;
    let required_features = REQUIRED_FEATURES | optional_features;

    let missing_features = required_features - adapter.features();
    if !missing_features.is_empty() {
        anyhow::bail!(
            "GPU lacks required features: {missing_features:?}; this renderer needs BC texture              compression, multi-draw-indirect-count, indirect-first-instance, texture binding              arrays with non-uniform indexing and partial binding, early depth test, and 16-bit UNORM render targets (TEXTURE_FORMAT_16BIT_NORM with TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES, used by the water displacement field)",
        );
    }

    // Request exactly what the object cull needs and fail loudly if the adapter
    // can't satisfy it.
    let needed_storage_buffers = STORAGE_BUFFERS_PER_STAGE;
    let available_storage_buffers = adapter.limits().max_storage_buffers_per_shader_stage;
    if available_storage_buffers < needed_storage_buffers {
        anyhow::bail!(
            "GPU exposes only {available_storage_buffers} storage buffers per shader stage; \
             the object-cull pass needs {needed_storage_buffers} \
             (1 instances + {DRAW_BUCKET_COUNT} draw buckets + 1 counts)",
        );
    }
    tracing::info!(
        "storage buffers per shader stage: requesting {needed_storage_buffers} \
         (adapter max {available_storage_buffers})",
    );

    let needed_bindless_textures = MAX_TEXTURES + CUBE_SLOTS;
    let adapter_limits = adapter.limits();
    for (name, available) in [
        (
            "sampled textures per shader stage",
            adapter_limits.max_sampled_textures_per_shader_stage,
        ),
        (
            "binding array elements per shader stage",
            adapter_limits.max_binding_array_elements_per_shader_stage,
        ),
    ] {
        if available < needed_bindless_textures {
            anyhow::bail!(
                "GPU exposes only {available} {name}; the material bind group needs {needed_bindless_textures} ({MAX_TEXTURES} 2D textures + {CUBE_SLOTS} cube maps)",
            );
        }
    }

    let mut limits = wgpu::Limits::default();
    limits.max_storage_buffers_per_shader_stage = needed_storage_buffers;
    limits.max_sampled_textures_per_shader_stage = needed_bindless_textures;
    limits.max_storage_buffer_binding_size = 256 * 1024 * 1024;
    // Mega buffers clamp their growth to this and compact when fragmented;
    // request whatever the adapter offers up to 2 GiB so dense regions fit.
    limits.max_buffer_size = adapter.limits().max_buffer_size.min(2 * 1024 * 1024 * 1024);
    limits.max_binding_array_elements_per_shader_stage = needed_bindless_textures;
    // Splatmap array: one 128x128 layer per resident landscape cell (D3D12 cap).
    limits.max_texture_array_layers = 2048;

    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: None,
            required_features,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            required_limits: limits,
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::Off,
        })
        .await
        .context(
            "request_device failed — this renderer requires BC texture compression, \
             multi-draw-indirect-count, indirect-first-instance, and an 8192-entry \
             texture binding array; try updating GPU drivers",
        )
}
