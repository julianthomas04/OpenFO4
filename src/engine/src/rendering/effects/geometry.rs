//! Effect geometry: the CPU mesh data the streamer hands over, the GPU pool that mirrors it while
//! it is resident, and the per-frame arena particle quads are written to.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Weak;
use std::sync::atomic::{AtomicU64, Ordering};

use glam::Mat4;
use wgpu::util::DeviceExt;

use super::particle_draw::ParticleDraw;
use super::shader::{EffectVertex, ParticleVertex};
use crate::assets::types::effect_material::{EffectMaterial, RefractionMaterial};
use crate::assets::types::mesh::CpuMesh;
use crate::rendering::frustum::Aabb;

/// Vertex colour of a mesh without a colour attribute: opaque white, the neutral multiplier.
const NEUTRAL_VERTEX_COLOUR: [u8; 4] = [255; 4];

static NEXT_MESH_ID: AtomicU64 = AtomicU64::new(1);

/// The vertices and indices of one effect (or refraction) submesh in the effect vertex layout.
/// Shared by every placement of the submesh; the GPU pool holds its buffers for as long as an
/// `Arc` to this data exists.
#[derive(Debug)]
pub struct EffectMeshData {
    id: u64,
    pub vertices: Vec<EffectVertex>,
    pub indices: Vec<u32>,
}

impl EffectMeshData {
    pub fn from_cpu_mesh(mesh: &CpuMesh) -> Self {
        let colours = mesh.vertex_colors.as_deref().unwrap_or(&[]);
        let vertices = mesh
            .vertices
            .iter()
            .enumerate()
            .map(|(index, vertex)| EffectVertex {
                position: vertex.position,
                uv: vertex.tex_coords,
                normal: vertex.normal,
                tangent: vertex.tangent,
                colour: colours.get(index).copied().unwrap_or(NEUTRAL_VERTEX_COLOUR),
            })
            .collect();
        Self::from_vertices(vertices, mesh.indices.clone())
    }

    pub fn from_vertices(vertices: Vec<EffectVertex>, indices: Vec<u32>) -> Self {
        Self {
            id: NEXT_MESH_ID.fetch_add(1, Ordering::Relaxed),
            vertices,
            indices,
        }
    }

    /// Identity of this data, unique for the process lifetime.
    pub fn id(&self) -> u64 {
        self.id
    }
}

/// One placement of an effect shape: where the stage draws a submesh and with which material.
#[derive(Debug, Clone)]
pub struct EffectInstance {
    pub mesh: Arc<EffectMeshData>,
    pub material: Arc<EffectMaterial>,
    /// Engine space <- mesh space.
    pub model: Mat4,
    /// Engine-space bounds of the placed mesh.
    pub bounds: Aabb,
}

/// One placement of a heat-haze shape, for the refraction stage.
#[derive(Debug, Clone)]
pub struct RefractionInstance {
    pub mesh: Arc<EffectMeshData>,
    pub material: Arc<RefractionMaterial>,
    pub model: Mat4,
    pub bounds: Aabb,
}

/// The GPU buffers of one effect mesh.
pub struct GpuEffectMesh {
    pub vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub index_count: u32,
}

struct PoolEntry {
    source: Weak<EffectMeshData>,
    mesh: GpuEffectMesh,
}

/// GPU buffers of the effect meshes in use, created on first use and released once the last
/// placement of a mesh is gone (its `EffectMeshData` dropped with the retired cell).
#[derive(Default)]
pub struct EffectGeometryPool {
    entries: HashMap<u64, PoolEntry>,
}

impl EffectGeometryPool {
    /// The buffers of `data`, uploaded if this is the first request for it.
    pub fn acquire(&mut self, device: &wgpu::Device, data: &Arc<EffectMeshData>) -> &GpuEffectMesh {
        &self
            .entries
            .entry(data.id())
            .or_insert_with(|| PoolEntry {
                source: Arc::downgrade(data),
                mesh: upload_mesh(device, data),
            })
            .mesh
    }

    pub fn get(&self, data: &EffectMeshData) -> Option<&GpuEffectMesh> {
        self.entries.get(&data.id()).map(|entry| &entry.mesh)
    }

    /// Drops the buffers of every mesh no placement refers to any more.
    pub fn release_unused(&mut self) {
        self.entries
            .retain(|_, entry| entry.source.strong_count() > 0);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn upload_mesh(device: &wgpu::Device, data: &EffectMeshData) -> GpuEffectMesh {
    GpuEffectMesh {
        vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("effect_mesh_vertices"),
            contents: bytemuck::cast_slice(&data.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        }),
        indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("effect_mesh_indices"),
            contents: bytemuck::cast_slice(&data.indices),
            usage: wgpu::BufferUsages::INDEX,
        }),
        index_count: data.indices.len() as u32,
    }
}

/// Where one particle draw's quads sit in the arena buffers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArenaRange {
    pub base_vertex: i32,
    pub first_index: u32,
    pub index_count: u32,
}

/// Lays the draws' geometry out back to back: each draw's indices stay local to its own
/// vertices and are rebased by `base_vertex` at draw time.
pub fn plan_arena(sizes: impl Iterator<Item = (usize, usize)>) -> Vec<ArenaRange> {
    let (mut vertices, mut indices) = (0usize, 0usize);
    sizes
        .map(|(vertex_count, index_count)| {
            let range = ArenaRange {
                base_vertex: vertices as i32,
                first_index: indices as u32,
                index_count: index_count as u32,
            };
            vertices += vertex_count;
            indices += index_count;
            range
        })
        .collect()
}

struct GrowingBuffer {
    buffer: wgpu::Buffer,
    capacity: u64,
    usage: wgpu::BufferUsages,
    label: &'static str,
}

impl GrowingBuffer {
    const INITIAL_CAPACITY: u64 = 64 * 1024;

    fn new(device: &wgpu::Device, label: &'static str, usage: wgpu::BufferUsages) -> Self {
        Self {
            buffer: Self::allocate(device, label, usage, Self::INITIAL_CAPACITY),
            capacity: Self::INITIAL_CAPACITY,
            usage,
            label,
        }
    }

    fn allocate(
        device: &wgpu::Device,
        label: &'static str,
        usage: wgpu::BufferUsages,
        capacity: u64,
    ) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: capacity,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8]) {
        let needed = bytes.len() as u64;
        if needed > self.capacity {
            self.capacity = needed.next_power_of_two();
            self.buffer = Self::allocate(device, self.label, self.usage, self.capacity);
        }
        if !bytes.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytes);
        }
    }
}

/// The per-frame vertex and index buffers of the simulated particle quads.
pub struct ParticleArena {
    vertices: GrowingBuffer,
    indices: GrowingBuffer,
    staged_vertices: Vec<ParticleVertex>,
    staged_indices: Vec<u32>,
}

impl ParticleArena {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            vertices: GrowingBuffer::new(
                device,
                "effect_particle_vertices",
                wgpu::BufferUsages::VERTEX,
            ),
            indices: GrowingBuffer::new(
                device,
                "effect_particle_indices",
                wgpu::BufferUsages::INDEX,
            ),
            staged_vertices: Vec::new(),
            staged_indices: Vec::new(),
        }
    }

    /// Writes the geometry of `draws` and returns each draw's range, in order.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        draws: &[ParticleDraw],
    ) -> Vec<ArenaRange> {
        let ranges = plan_arena(
            draws
                .iter()
                .map(|draw| (draw.vertices.len(), draw.indices.len())),
        );
        self.staged_vertices.clear();
        self.staged_indices.clear();
        for draw in draws {
            self.staged_vertices.extend_from_slice(&draw.vertices);
            self.staged_indices.extend_from_slice(&draw.indices);
        }
        self.vertices
            .write(device, queue, bytemuck::cast_slice(&self.staged_vertices));
        self.indices
            .write(device, queue, bytemuck::cast_slice(&self.staged_indices));
        ranges
    }

    pub fn vertex_buffer(&self) -> &wgpu::Buffer {
        &self.vertices.buffer
    }

    pub fn index_buffer(&self) -> &wgpu::Buffer {
        &self.indices.buffer
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals copied, not computed"
)]
mod tests {
    use super::*;
    use crate::rendering::vertex::Vertex;

    fn cpu_mesh(colours: Option<Vec<[u8; 4]>>) -> CpuMesh {
        let vertex = |x: f32| Vertex {
            position: [x, 0.0, 0.0],
            tex_coords: [x, 1.0],
            normal: [0.0, 1.0, 0.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
        };
        CpuMesh {
            vertices: vec![vertex(0.0), vertex(1.0), vertex(2.0)],
            topology: crate::assets::types::mesh::Topology::Triangles,
            indices: vec![0, 1, 2],
            texture_path: None,
            vertex_colors: colours,
            effect: None,
            refraction: None,
        }
    }

    #[test]
    fn mesh_data_carries_every_attribute_and_the_vertex_colour() {
        let data = EffectMeshData::from_cpu_mesh(&cpu_mesh(Some(vec![
            [1, 2, 3, 4],
            [5, 6, 7, 8],
            [9, 10, 11, 12],
        ])));

        assert_eq!(data.vertices[1].position, [1.0, 0.0, 0.0]);
        assert_eq!(data.vertices[1].uv, [1.0, 1.0]);
        assert_eq!(data.vertices[1].normal, [0.0, 1.0, 0.0]);
        assert_eq!(data.vertices[1].tangent, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(data.vertices[2].colour, [9, 10, 11, 12]);
        assert_eq!(data.indices, [0, 1, 2]);
    }

    #[test]
    fn a_mesh_without_colours_gets_opaque_white() {
        let data = EffectMeshData::from_cpu_mesh(&cpu_mesh(None));
        assert!(data.vertices.iter().all(|v| v.colour == [255; 4]));
    }

    #[test]
    fn every_mesh_data_has_its_own_id() {
        let mesh = cpu_mesh(None);
        let (first, second) = (
            EffectMeshData::from_cpu_mesh(&mesh),
            EffectMeshData::from_cpu_mesh(&mesh),
        );
        assert_ne!(first.id(), second.id());
    }

    #[test]
    fn the_arena_plan_places_draws_back_to_back_with_local_indices() {
        let ranges = plan_arena([(4, 6), (8, 12), (0, 0), (4, 6)].into_iter());

        assert_eq!(
            ranges,
            [
                ArenaRange {
                    base_vertex: 0,
                    first_index: 0,
                    index_count: 6
                },
                ArenaRange {
                    base_vertex: 4,
                    first_index: 6,
                    index_count: 12
                },
                ArenaRange {
                    base_vertex: 12,
                    first_index: 18,
                    index_count: 0
                },
                ArenaRange {
                    base_vertex: 12,
                    first_index: 18,
                    index_count: 6
                },
            ]
        );
    }

    #[test]
    fn the_pool_releases_a_mesh_once_its_data_is_dropped() {
        let Some((device, _queue)) = crate::rendering::gpu_test::test_device("effect pool") else {
            return;
        };
        let mut pool = EffectGeometryPool::default();
        let kept = Arc::new(EffectMeshData::from_cpu_mesh(&cpu_mesh(None)));
        let dropped = Arc::new(EffectMeshData::from_cpu_mesh(&cpu_mesh(None)));
        assert_eq!(pool.acquire(&device, &kept).index_count, 3);
        pool.acquire(&device, &dropped);
        assert_eq!(pool.len(), 2);

        drop(dropped);
        pool.release_unused();

        assert_eq!(pool.len(), 1);
        assert!(pool.get(&kept).is_some());
    }

    #[test]
    fn the_arena_grows_to_hold_a_frame_and_returns_one_range_per_draw() {
        let Some((device, queue)) = crate::rendering::gpu_test::test_device("effect arena") else {
            return;
        };
        let mut arena = ParticleArena::new(&device);
        let vertex = ParticleVertex {
            position: [0.0; 3],
            sub_texture: 0.0,
            uv: [0.0; 2],
            distortion: 1.0,
            colour: [255; 4],
        };
        let draw = |quads: usize| ParticleDraw {
            vertices: vec![vertex; quads * 4],
            indices: (0..quads as u32)
                .flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3])
                .collect(),
            material: Arc::new(crate::rendering::effects::test_fixtures::stub_material()),
            elapsed_seconds: 0.0,
            model: Mat4::IDENTITY,
            subtexture_rects: Vec::new(),
            bounds_centre: glam::Vec3::ZERO,
        };
        let big = draw(2000);
        let small = draw(1);

        let ranges = arena.upload(&device, &queue, &[big, small]);

        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[1].base_vertex, 8000);
        assert!(arena.vertex_buffer().size() >= 8004 * 32);
    }
}
