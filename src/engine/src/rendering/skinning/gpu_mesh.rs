use glam::Mat4;
use wgpu::util::DeviceExt;

use super::materials::{CharacterGpuContext, PartPolicy, ResolvedMaterial, resolve_material};
use crate::assets::types::skinned_mesh::{SkinnedMesh, SkinnedMeshError};
use crate::rendering::gpu_driven::TextureHandle;

pub struct GpuSkinnedPart {
    pub vertex_buffer: wgpu::Buffer,
    pub skin_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub material: ResolvedMaterial,
    /// Drawn only while the debug "Show FX" toggle is on.
    pub hidden_by_default: bool,
    /// Index into `GpuSkinnedMesh::inverse_binds`.
    pub skin: usize,
}

pub struct GpuSkinnedMesh {
    pub parts: Vec<GpuSkinnedPart>,
    pub inverse_binds: Vec<Vec<Mat4>>,
    /// See `SkinnedMesh::joint_reaches`.
    pub joint_reaches: Vec<f32>,
}

impl GpuSkinnedMesh {
    /// Fails before any texture is admitted, so an error leaves no references behind.
    pub fn upload(
        mesh: &SkinnedMesh,
        policy: PartPolicy,
        gpu: &mut CharacterGpuContext,
    ) -> Result<Self, SkinnedMeshError> {
        mesh.bind_bounds().ok_or(SkinnedMeshError::NoVertices)?;
        let joint_reaches = mesh.joint_reaches();
        let parts = mesh
            .parts
            .iter()
            .filter_map(|part| {
                let material_name = policy.material_name(part.material_name.as_deref())?;
                let buffer = |label, contents: &[u8], usage| {
                    gpu.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(label),
                            contents,
                            usage,
                        })
                };
                Some(GpuSkinnedPart {
                    vertex_buffer: buffer(
                        "skinned_vertices",
                        bytemuck::cast_slice(&part.vertices),
                        wgpu::BufferUsages::VERTEX,
                    ),
                    skin_buffer: buffer(
                        "skin_vertices",
                        bytemuck::cast_slice(&part.skin_vertices),
                        wgpu::BufferUsages::VERTEX,
                    ),
                    index_buffer: buffer(
                        "skinned_indices",
                        bytemuck::cast_slice(&part.indices),
                        wgpu::BufferUsages::INDEX,
                    ),
                    index_count: part.indices.len() as u32,
                    material: resolve_material(material_name, gpu),
                    hidden_by_default: policy.hides_part_by_default(part.material_name.as_deref()),
                    skin: part.skin,
                })
            })
            .collect();
        Ok(Self {
            parts,
            inverse_binds: mesh.inverse_binds.clone(),
            joint_reaches,
        })
    }

    pub fn texture_refs(&self) -> impl Iterator<Item = TextureHandle> + '_ {
        self.parts
            .iter()
            .flat_map(|part| part.material.texture_refs.iter().copied())
    }
}
