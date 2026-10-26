//! The cloud model: the meshes of the vanilla `Sky\Clouds.nif`, one for each cloud layer, in one
//! vertex list and one index list.

use std::ops::Range;

use super::assets::{SkyAssetError, first_vertex, load_model_meshes, triangle_indices};
use crate::assets::types::mesh::CpuMesh;

/// The cloud model of a weather that names none.
pub(super) const DEFAULT_MODEL: &str = "Sky\\Clouds.nif";

const OPAQUE_WHITE: [u8; 4] = [255; 4];

/// Mirrors the vertex inputs of `sky_clouds.wgsl`.
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct CloudVertex {
    /// A direction from the camera: the model is drawn at infinite depth.
    pub position: [f32; 3],
    pub uv: [f32; 2],
    /// `COLOR_0`; its alpha fades a mesh out towards its edges.
    pub color: [u8; 4],
}

impl CloudVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Unorm8x4];

    pub(super) fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

/// The triangles of the cloud model's meshes. The mesh at position `i` of the model file is
/// the shape of cloud layer `i`, judging by their names.
#[derive(Debug, Clone)]
pub(super) struct CloudGeometry {
    pub vertices: Vec<CloudVertex>,
    pub indices: Vec<u32>,
    /// The part of `indices` of each mesh, in the order of the file; empty for a mesh that is
    /// not drawn as triangles.
    pub ranges: Vec<Range<u32>>,
}

impl CloudGeometry {
    pub(super) fn from_meshes(key: &str, meshes: Vec<CpuMesh>) -> Result<Self, SkyAssetError> {
        let mut vertices = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut ranges = Vec::with_capacity(meshes.len());
        for mesh in meshes {
            let begin = index_position(key, indices.len())?;
            if let Some(mesh_indices) = triangle_indices(key, &mesh)? {
                let first = first_vertex(key, vertices.len(), mesh.vertices.len())?;
                indices.extend(mesh_indices.iter().map(|index| index + first));
                vertices.extend(cloud_vertices(&mesh));
            }
            ranges.push(begin..index_position(key, indices.len())?);
        }
        if indices.is_empty() {
            return Err(SkyAssetError::NoTriangles {
                key: key.to_owned(),
            });
        }
        Ok(Self {
            vertices,
            indices,
            ranges,
        })
    }
}

/// A mesh without vertex colours, or a vertex without one, is opaque.
fn cloud_vertices(mesh: &CpuMesh) -> impl Iterator<Item = CloudVertex> + '_ {
    let colors = mesh.vertex_colors.as_deref().unwrap_or_default();
    mesh.vertices
        .iter()
        .enumerate()
        .map(move |(index, vertex)| CloudVertex {
            position: vertex.position,
            uv: vertex.tex_coords,
            color: colors.get(index).copied().unwrap_or(OPAQUE_WHITE),
        })
}

fn index_position(key: &str, count: usize) -> Result<u32, SkyAssetError> {
    u32::try_from(count).map_err(|_| SkyAssetError::Mesh {
        key: key.to_owned(),
        reason: "more indices than a 32 bit position addresses".to_owned(),
    })
}

/// The cloud model `model_path`, a `.nif` path converted to its GLB.
pub(super) fn load_cloud_geometry(
    asset_root: &str,
    model_path: &str,
) -> Result<CloudGeometry, SkyAssetError> {
    let (key, meshes) = load_model_meshes(asset_root, model_path)?;
    CloudGeometry::from_meshes(&key, meshes)
}
