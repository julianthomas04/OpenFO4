//! A loading screen's model in two steps that can both run off the frame thread: reading and
//! flattening the file ([`CpuArt::load`]), then putting its geometry and textures on the GPU
//! ([`LoadedArt::upload`]; `wgpu`'s device and queue are `Send + Sync`).

use std::collections::HashMap;
use std::io;
use std::ops::Range;
use std::path::PathBuf;

use wgpu::util::DeviceExt;

use super::art::LoadingArt;
use super::camera::ArtCamera;
use crate::assets::types::material_name::MaterialName;
use crate::assets::types::mesh::{CpuMesh, Topology};
use crate::rendering::texture::load_texture_asset_srgb;
use crate::rendering::vertex::Vertex;

#[derive(Debug, thiserror::Error)]
pub enum LoadingArtError {
    #[error("the loading screen model '{}' does not exist", path.display())]
    ModelMissing { path: PathBuf },
    #[error("the loading screen model '{}' cannot be read: {source}", path.display())]
    ModelUnreadable {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the loading screen model '{}' is not a usable model: {source}", path.display())]
    ModelInvalid {
        path: PathBuf,
        #[source]
        source: anyhow::Error,
    },
    #[error("the loading screen model '{}' has nothing to draw", path.display())]
    NothingToDraw { path: PathBuf },
    #[error("the loading screen model '{}' is too large to draw", path.display())]
    TooLarge { path: PathBuf },
}

/// One drawable part of the model: its triangles and the diffuse texture it asks for.
struct CpuSubmesh {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    diffuse_key: Option<String>,
}

/// The model read from disk, ready to be uploaded.
pub struct CpuArt {
    model_path: PathBuf,
    submeshes: Vec<CpuSubmesh>,
    asset_root: PathBuf,
    camera: ArtCamera,
}

impl CpuArt {
    /// Reads the art's model. Only triangle parts that a viewer sees are kept.
    pub fn load(art: &LoadingArt) -> Result<Self, LoadingArtError> {
        let path = &art.model_path;
        match std::fs::metadata(path) {
            Ok(_) => {}
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Err(LoadingArtError::ModelMissing { path: path.clone() });
            }
            Err(source) => {
                return Err(LoadingArtError::ModelUnreadable {
                    path: path.clone(),
                    source,
                });
            }
        }
        let model = CpuMesh::load_model(&path.to_string_lossy()).map_err(|source| {
            LoadingArtError::ModelInvalid {
                path: path.clone(),
                source,
            }
        })?;
        let submeshes: Vec<_> = model
            .meshes
            .into_iter()
            .filter(|mesh| mesh.topology == Topology::Triangles && !mesh.indices.is_empty())
            .filter_map(|mesh| submesh(mesh, path))
            .collect();
        if submeshes.is_empty() {
            return Err(LoadingArtError::NothingToDraw { path: path.clone() });
        }
        Ok(Self {
            model_path: path.clone(),
            submeshes,
            asset_root: art.asset_root.clone(),
            camera: ArtCamera::new(art),
        })
    }
}

/// `None` for a helper volume, which is geometry for the editor and never seen.
fn submesh(mesh: CpuMesh, model: &std::path::Path) -> Option<CpuSubmesh> {
    let diffuse_key = match mesh.texture_path.as_deref().map(MaterialName::parse) {
        None => None,
        Some(Ok(material)) if material.is_helper_shape() => return None,
        Some(Ok(material)) => material.diffuse_path,
        Some(Err(error)) => {
            let name = mesh.texture_path.as_deref().unwrap_or_default();
            crate::utils::warn_once(
                &format!("loading-art-material:{}:{name}", model.display()),
                || {
                    format!(
                        "{}: material {name:?} is not understood ({error}); drawing the part untextured",
                        model.display()
                    )
                },
            );
            None
        }
    };
    Some(CpuSubmesh {
        vertices: mesh.vertices,
        indices: mesh.indices,
        diffuse_key,
    })
}

/// A part of the model's shared buffers drawn with one texture.
pub(super) struct Draw {
    pub indices: Range<u32>,
    pub base_vertex: i32,
    pub diffuse: wgpu::TextureView,
}

/// A model on the GPU with the camera that views it.
pub struct LoadedArt {
    pub(super) vertices: wgpu::Buffer,
    pub(super) indices: wgpu::Buffer,
    pub(super) draws: Vec<Draw>,
    pub(super) camera: ArtCamera,
}

impl LoadedArt {
    /// Uploads `cpu`'s geometry and loads its textures. A texture that is missing or unusable is
    /// warned about once and drawn white.
    pub fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        cpu: CpuArt,
    ) -> Result<Self, LoadingArtError> {
        let mut vertices: Vec<Vertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut ranges = Vec::with_capacity(cpu.submeshes.len());
        let mut textures = TextureCache::new(device, queue, &cpu.asset_root);
        for part in &cpu.submeshes {
            let start = u32::try_from(indices.len()).map_err(|_| too_large(&cpu.model_path))?;
            let base_vertex =
                i32::try_from(vertices.len()).map_err(|_| too_large(&cpu.model_path))?;
            vertices.extend_from_slice(&part.vertices);
            indices.extend_from_slice(&part.indices);
            let end = u32::try_from(indices.len()).map_err(|_| too_large(&cpu.model_path))?;
            ranges.push(Draw {
                indices: start..end,
                base_vertex,
                diffuse: textures.get(part.diffuse_key.as_deref()),
            });
        }
        Ok(Self {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("loading_art_vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("loading_art_indices"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            draws: ranges,
            camera: cpu.camera,
        })
    }
}

fn too_large(path: &std::path::Path) -> LoadingArtError {
    LoadingArtError::TooLarge {
        path: path.to_path_buf(),
    }
}

/// The views of the textures a model asks for, each loaded once.
struct TextureCache<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    asset_root: &'a std::path::Path,
    loaded: HashMap<Option<String>, wgpu::TextureView>,
}

impl<'a> TextureCache<'a> {
    fn new(
        device: &'a wgpu::Device,
        queue: &'a wgpu::Queue,
        asset_root: &'a std::path::Path,
    ) -> Self {
        Self {
            device,
            queue,
            asset_root,
            loaded: HashMap::new(),
        }
    }

    fn get(&mut self, key: Option<&str>) -> wgpu::TextureView {
        let key = key.map(str::to_owned);
        if let Some(view) = self.loaded.get(&key) {
            return view.clone();
        }
        let view = key
            .as_deref()
            .and_then(|key| self.load(key))
            .unwrap_or_else(|| white_texel(self.device, self.queue));
        self.loaded.insert(key, view.clone());
        view
    }

    fn load(&self, key: &str) -> Option<wgpu::TextureView> {
        match load_texture_asset_srgb(
            self.device,
            self.queue,
            &self.asset_root.to_string_lossy(),
            key,
        ) {
            Ok(view) => Some(view),
            Err(error) => {
                crate::utils::warn_once(&format!("loading-art-texture:{key}"), || {
                    format!("{error}; the loading screen draws that part white")
                });
                None
            }
        }
    }
}

fn white_texel(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("loading_art_white"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &[255; 4],
        )
        .create_view(&wgpu::TextureViewDescriptor::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::loading_screen::test_support::cube_art;

    #[test]
    fn a_model_that_does_not_exist_is_reported_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mut art = cube_art(dir.path(), "");
        art.model_path = dir.path().join("absent.glb");

        let error = CpuArt::load(&art).err().expect("there is no file");

        assert!(
            matches!(&error, LoadingArtError::ModelMissing { path } if *path == art.model_path),
            "{error}"
        );
    }

    #[test]
    fn a_file_that_is_not_a_model_is_reported_as_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let mut art = cube_art(dir.path(), "");
        art.model_path = dir.path().join("junk.glb");
        std::fs::write(&art.model_path, b"not a glb").unwrap();

        let error = CpuArt::load(&art).err().expect("the file is junk");

        assert!(
            matches!(error, LoadingArtError::ModelInvalid { .. }),
            "{error}"
        );
    }

    #[test]
    fn the_cube_loads_as_one_part_asking_for_its_material_texture() {
        let dir = tempfile::tempdir().unwrap();
        let art = cube_art(dir.path(), "props\\crate_d.dds");

        let cpu = CpuArt::load(&art).unwrap();

        assert_eq!(cpu.submeshes.len(), 1);
        assert_eq!(cpu.submeshes[0].vertices.len(), 24);
        assert_eq!(cpu.submeshes[0].indices.len(), 36);
        assert_eq!(
            cpu.submeshes[0].diffuse_key.as_deref(),
            Some("textures\\props\\crate_d.dds")
        );
    }
}
