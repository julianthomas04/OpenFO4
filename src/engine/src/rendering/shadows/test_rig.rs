//! A headless rig that draws hand-placed casters through the shadow cull and the caster
//! pipelines, for the tests of the passes built on them.

use std::path::Path;

use glam::{Mat4, Vec3};

use super::lamps::atlas_texture::LampAtlasTexture;
use super::settings::{ShadowConfig, ShadowFilter};
use super::shadow_maps::ShadowMaps;
use super::{ShadowRenderer, ShadowScene};
use crate::rendering::cube_array::CubeArray;
use crate::rendering::gpu_driven::{
    GpuInstance, GpuTerrainInstance, MaterialRegistry, NO_SPLAT_LAYER, TextureArray,
};
use crate::rendering::gpu_profiler::GpuProfiler;
use crate::rendering::gpu_test::{map_read, world_pass_device};
use crate::rendering::lighting::frame_uniform::MAX_CASCADES;
use crate::rendering::mega_buffer::{MegaBuffer, MeshSlot};
use crate::rendering::render_pipelines::world_pass_layout::WorldPassLayout;
use crate::rendering::skinning::bind_layout::SkinnedBindLayout;
use crate::rendering::skinning::scene::SkinnedScene;
use crate::rendering::vertex::Vertex;

const MAX_CASTERS: u64 = 16;
const SHADOW_MAP_SIZE: u32 = 64;
const SKINNED_BUFFER_BYTES: u64 = 256;
const SLAB_INDICES: [u32; 6] = [0, 1, 2, 0, 2, 3];
/// Below the alpha of the stand-in texture, so a cut-out slab is solid wherever it is drawn.
const ALPHA_CUTOFF: f32 = 0.5;

/// A flat square caster, parallel to the ground; `alpha_tested` makes an object a cut-out one.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Slab {
    pub centre: Vec3,
    pub half_size: f32,
    pub alpha_tested: bool,
}

impl Slab {
    fn corners(self) -> [Vec3; 4] {
        let reach = Vec3::new(self.half_size, 0.0, self.half_size);
        [
            self.centre + reach * Vec3::new(-1.0, 0.0, -1.0),
            self.centre + reach * Vec3::new(1.0, 0.0, -1.0),
            self.centre + reach * Vec3::new(1.0, 0.0, 1.0),
            self.centre + reach * Vec3::new(-1.0, 0.0, 1.0),
        ]
    }

    fn bounds(self) -> ([f32; 3], [f32; 3]) {
        let reach = Vec3::new(self.half_size, 0.0, self.half_size);
        (
            (self.centre - reach).to_array(),
            (self.centre + reach).to_array(),
        )
    }
}

fn slab_vertices(corners: [Vec3; 4]) -> Vec<Vertex> {
    corners
        .map(|corner| Vertex {
            position: corner.to_array(),
            tex_coords: [0.0; 2],
            normal: [0.0, 1.0, 0.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
        })
        .to_vec()
}

fn storage_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

pub(crate) struct ShadowRig {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub profiler: GpuProfiler,
    pub renderer: ShadowRenderer,
    pub world: RigWorld,
}

/// What the passes draw, apart from the renderer that draws it: borrowed on its own, so a
/// pass can take the renderer mutably.
pub(crate) struct RigWorld {
    object_instances: wgpu::Buffer,
    terrain_instances: wgpu::Buffer,
    object_mega: MegaBuffer,
    terrain_mega: MegaBuffer,
    object_slab: MeshSlot,
    material_bind_group: wgpu::BindGroup,
    instance_bind_group: wgpu::BindGroup,
    skinned_bind_group: wgpu::BindGroup,
    object_count: u32,
    terrain_count: u32,
}

pub(crate) fn rig(test: &str) -> Option<ShadowRig> {
    let (device, queue) = world_pass_device(test)?;
    let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("shadow_rig_frame_bgl"),
        entries: &[],
    });
    let world_pass_layout = WorldPassLayout::new(&device, &frame_layout);
    let skinned_layout = SkinnedBindLayout::new(&device);
    let config = ShadowConfig {
        resolution: SHADOW_MAP_SIZE,
        distance: 100.0,
        cascade_count: 1,
        split_blend: 0.0,
        filter: ShadowFilter::SingleTap,
        bias_scale: 1.0,
        cascade_min_texels: [0.0; MAX_CASCADES],
        skip_alpha_in_last_cascade: false,
    };
    let object_instances = storage_buffer(
        &device,
        "shadow_rig_object_instances",
        MAX_CASTERS * std::mem::size_of::<GpuInstance>() as u64,
    );
    let terrain_instances = storage_buffer(
        &device,
        "shadow_rig_terrain_instances",
        MAX_CASTERS * std::mem::size_of::<GpuTerrainInstance>() as u64,
    );
    let renderer = ShadowRenderer::new(
        &device,
        ShadowMaps::new(&device, &config),
        LampAtlasTexture::new(&device, SHADOW_MAP_SIZE),
        &config,
        &world_pass_layout,
        &skinned_layout,
        &object_instances,
        &terrain_instances,
    );
    let instance_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("shadow_rig_instance_bind_group"),
        layout: &world_pass_layout.instance_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: object_instances.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: terrain_instances.as_entire_binding(),
            },
        ],
    });
    let skinned_buffers = [0, 2, 3].map(|binding| {
        (
            binding,
            storage_buffer(&device, "shadow_rig_skinned", SKINNED_BUFFER_BYTES),
        )
    });
    let skinned_entries: Vec<_> = skinned_buffers
        .iter()
        .map(|(binding, buffer)| wgpu::BindGroupEntry {
            binding: *binding,
            resource: buffer.as_entire_binding(),
        })
        .collect();
    let skinned_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("shadow_rig_skinned_bind_group"),
        layout: skinned_layout.layout(),
        entries: &skinned_entries,
    });
    let material_bind_group = material_bind_group(&device, &queue, &world_pass_layout);
    let unit_slab = Slab {
        centre: Vec3::ZERO,
        half_size: 1.0,
        alpha_tested: false,
    };
    let mut object_mega = MegaBuffer::new(&device, "shadow_rig_object");
    let (object_slab, _) = object_mega
        .allocate(
            "unit slab",
            &slab_vertices(unit_slab.corners()),
            &SLAB_INDICES,
            &device,
            &queue,
        )
        .expect("the fresh mega buffer has room for a quad");
    let world = RigWorld {
        terrain_mega: MegaBuffer::new(&device, "shadow_rig_terrain"),
        object_instances,
        terrain_instances,
        object_mega,
        object_slab,
        material_bind_group,
        instance_bind_group,
        skinned_bind_group,
        object_count: 0,
        terrain_count: 0,
    };
    Some(ShadowRig {
        profiler: GpuProfiler::new(&device, &queue),
        device,
        queue,
        renderer,
        world,
    })
}

/// The material group of the world passes with nothing in it but the stand-in textures.
fn material_bind_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &WorldPassLayout,
) -> wgpu::BindGroup {
    let textures = TextureArray::new(device, queue);
    let cubes = CubeArray::new(device, queue, Path::new(""));
    let materials = MaterialRegistry::new(device);
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor::default());
    let splatmaps = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow_rig_splatmaps"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("shadow_rig_material_bind_group"),
        layout: &layout.material_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureViewArray(
                    &textures.build_bind_group_entries(),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: materials.buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&splatmaps),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: textures.slot_mips_buffer().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureViewArray(&cubes.views()),
            },
        ],
    })
}

impl ShadowRig {
    /// Fills the scene with `objects` (opaque statics) and `terrain` (landscape pieces). A rig
    /// is placed once.
    pub fn place(&mut self, objects: &[Slab], terrain: &[Slab]) {
        assert!(
            (objects.len().max(terrain.len()) as u64) <= MAX_CASTERS,
            "the rig holds {MAX_CASTERS} casters of a kind"
        );
        let world = &mut self.world;
        assert_eq!(world.object_count + world.terrain_count, 0, "placed twice");
        let object_slab = world.object_slab;
        let instances: Vec<GpuInstance> = objects
            .iter()
            .map(|slab| {
                let (aabb_min, aabb_max) = slab.bounds();
                GpuInstance {
                    transform: Mat4::from_translation(slab.centre)
                        .mul_mat4(&Mat4::from_scale(Vec3::splat(slab.half_size)))
                        .to_cols_array_2d(),
                    aabb_min,
                    _pad0: 0.0,
                    aabb_max,
                    _pad1: 0.0,
                    index_offset: object_slab.index_offset,
                    index_count: object_slab.index_count,
                    vertex_offset: object_slab.vertex_offset,
                    material_id: 0,
                    alpha_cutoff: if slab.alpha_tested { ALPHA_CUTOFF } else { 0.0 },
                    alpha_mult: 1.0,
                    flags: 0,
                    _pad2: 0,
                }
            })
            .collect();
        let pieces: Vec<GpuTerrainInstance> = terrain
            .iter()
            .enumerate()
            .map(|(piece, slab)| {
                let (mesh, _) = world
                    .terrain_mega
                    .allocate(
                        &format!("terrain piece {piece}"),
                        &slab_vertices(slab.corners()),
                        &SLAB_INDICES,
                        &self.device,
                        &self.queue,
                    )
                    .expect("the fresh mega buffer has room for a quad");
                let (aabb_min, aabb_max) = slab.bounds();
                GpuTerrainInstance {
                    aabb_min,
                    _pad0: 0.0,
                    aabb_max,
                    _pad1: 0.0,
                    index_offset: mesh.index_offset,
                    index_count: mesh.index_count,
                    vertex_offset: mesh.vertex_offset,
                    splat_layer: 0,
                    layer_indices: [0; 4],
                    splat_layer_hi: NO_SPLAT_LAYER,
                    _pad2: 0,
                    _pad3: 0,
                    _pad4: 0,
                    layer_indices_hi: [0; 4],
                    flags: [0; 4],
                }
            })
            .collect();
        self.queue
            .write_buffer(&world.object_instances, 0, bytemuck::cast_slice(&instances));
        self.queue
            .write_buffer(&world.terrain_instances, 0, bytemuck::cast_slice(&pieces));
        world.object_count = instances.len() as u32;
        world.terrain_count = pieces.len() as u32;
        self.renderer
            .resize_draws(&self.device, [world.object_count, 0, world.terrain_count]);
        self.renderer.rebind(
            &self.device,
            &world.object_instances,
            &world.terrain_instances,
        );
    }
}

impl RigWorld {
    pub fn scene(&self) -> ShadowScene<'_> {
        ShadowScene {
            material_bind_group: &self.material_bind_group,
            instance_bind_group: &self.instance_bind_group,
            object_mega: &self.object_mega,
            terrain_mega: &self.terrain_mega,
            object_count: self.object_count,
            terrain_count: self.terrain_count,
            skinned: SkinnedScene {
                bind_group: &self.skinned_bind_group,
                characters: &[],
                draws: &[],
                visible: &[],
                world_bounds: &[],
            },
        }
    }
}

/// `count` words of `buffer`, read back.
pub(crate) fn read_u32s(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    buffer: &wgpu::Buffer,
    count: usize,
) -> Vec<u32> {
    let size = (count * std::mem::size_of::<u32>()) as u64;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shadow_rig_readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, size);
    queue.submit(std::iter::once(encoder.finish()));
    bytemuck::cast_slice(&map_read(device, &staging)).to_vec()
}
