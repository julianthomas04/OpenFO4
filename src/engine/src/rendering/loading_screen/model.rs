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
use super::pan_window::{bounding_sphere_radius, pan_limit};
use crate::assets::types::material_name::MaterialName;
use crate::assets::types::mesh::{CpuMesh, Topology};
use crate::rendering::texture::{load_texture_asset, load_texture_asset_srgb};
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

/// One drawable part of the model: its triangles, the textures it asks for, the alpha below
/// which the diffuse texture is cut out (0 when the material has no alpha test) and the
/// material's specular parameters.
struct CpuSubmesh {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    diffuse_key: Option<String>,
    normal_key: Option<String>,
    specular_key: Option<String>,
    alpha_cutoff: f32,
    smoothness: f32,
    specular_scale: f32,
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
        let radius = bounding_sphere_radius(
            submeshes
                .iter()
                .flat_map(|part| part.vertices.iter().map(|vertex| vertex.position)),
        );
        Ok(Self {
            model_path: path.clone(),
            submeshes,
            asset_root: art.asset_root.clone(),
            camera: ArtCamera::new(art, pan_limit(radius, art.transform.scale)),
        })
    }
}

/// `None` for a helper volume, which is geometry for the editor and never seen.
fn submesh(mesh: CpuMesh, model: &std::path::Path) -> Option<CpuSubmesh> {
    let material = match mesh.texture_path.as_deref().map(MaterialName::parse) {
        None => MaterialName::untextured(),
        Some(Ok(material)) if material.is_helper_shape() => return None,
        Some(Ok(material)) => material,
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
            MaterialName::untextured()
        }
    };
    Some(CpuSubmesh {
        vertices: mesh.vertices,
        indices: mesh.indices,
        diffuse_key: material.diffuse_path,
        normal_key: material.normal_path,
        specular_key: material.specular_path,
        alpha_cutoff: material.alpha_cutoff,
        smoothness: material.surface.smoothness,
        specular_scale: material.surface.specular_scale(),
    })
}

/// A part of the model's shared buffers drawn with one material.
pub(super) struct Draw {
    pub indices: Range<u32>,
    pub base_vertex: i32,
    pub diffuse: wgpu::TextureView,
    pub normal: wgpu::TextureView,
    pub specular: wgpu::TextureView,
    pub alpha_cutoff: f32,
    pub smoothness: f32,
    pub specular_scale: f32,
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
    /// warned about once and replaced by a neutral one: white for the diffuse, a flat normal
    /// for the normal map and no specular for the specular map.
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
                diffuse: textures.get(TextureRole::Diffuse, part.diffuse_key.as_deref()),
                normal: textures.get(TextureRole::Normal, part.normal_key.as_deref()),
                specular: textures.get(TextureRole::Specular, part.specular_key.as_deref()),
                alpha_cutoff: part.alpha_cutoff,
                smoothness: part.smoothness,
                specular_scale: part.specular_scale,
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

/// What a texture of a material is used for: fixes its colour space, what a part without that
/// texture shows and what it shows when the texture cannot be loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TextureRole {
    Diffuse,
    Normal,
    Specular,
}

impl TextureRole {
    fn is_srgb(self) -> bool {
        self == Self::Diffuse
    }

    /// A material that names no texture for the role: white diffuse, flat normal and, like the
    /// world, a white specular map that leaves the material's own parameters in charge.
    fn absent_texel(self) -> [u8; 4] {
        match self {
            Self::Diffuse | Self::Specular => [255; 4],
            Self::Normal => [128, 128, 255, 255],
        }
    }

    /// A texture that is named but cannot be loaded: the same, except that the specular map
    /// then switches the specular off.
    fn unavailable_texel(self) -> [u8; 4] {
        match self {
            Self::Specular => [0, 0, 0, 255],
            Self::Diffuse | Self::Normal => self.absent_texel(),
        }
    }

    fn unavailable_effect(self) -> &'static str {
        match self {
            Self::Diffuse => "draws that part white",
            Self::Normal => "draws that part with a flat normal",
            Self::Specular => "draws that part without specular",
        }
    }
}

/// The views of the textures a model asks for, each loaded once per role.
struct TextureCache<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    asset_root: &'a std::path::Path,
    loaded: HashMap<(TextureRole, Option<String>), wgpu::TextureView>,
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

    fn get(&mut self, role: TextureRole, key: Option<&str>) -> wgpu::TextureView {
        let key = (role, key.map(str::to_owned));
        if let Some(view) = self.loaded.get(&key) {
            return view.clone();
        }
        let view = match key.1.as_deref() {
            None => texel(self.device, self.queue, role, role.absent_texel()),
            Some(path) => self
                .load(role, path)
                .unwrap_or_else(|| texel(self.device, self.queue, role, role.unavailable_texel())),
        };
        self.loaded.insert(key, view.clone());
        view
    }

    fn load(&self, role: TextureRole, key: &str) -> Option<wgpu::TextureView> {
        let load = if role.is_srgb() {
            load_texture_asset_srgb
        } else {
            load_texture_asset
        };
        match load(
            self.device,
            self.queue,
            &self.asset_root.to_string_lossy(),
            key,
        ) {
            Ok(view) => Some(view),
            Err(error) => {
                crate::utils::warn_once(&format!("loading-art-texture:{key}"), || {
                    format!("{error}; the loading screen {}", role.unavailable_effect())
                });
                None
            }
        }
    }
}

fn texel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    role: TextureRole,
    rgba: [u8; 4],
) -> wgpu::TextureView {
    let format = if role.is_srgb() {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    device
        .create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("loading_art_texel"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &rgba,
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
        assert!(cpu.submeshes[0].alpha_cutoff.abs() < 1e-6);
    }

    #[test]
    fn a_part_takes_its_alpha_cutoff_from_the_material_test_token() {
        let dir = tempfile::tempdir().unwrap();
        let art = cube_art(dir.path(), "props\\leaf_d.dds|test:51");

        let cpu = CpuArt::load(&art).unwrap();

        assert!((cpu.submeshes[0].alpha_cutoff - 0.2).abs() < 1e-6);
    }

    #[test]
    fn a_part_takes_its_normal_map_specular_map_and_specular_parameters_from_its_material() {
        let dir = tempfile::tempdir().unwrap();
        let art = cube_art(
            dir.path(),
            "props\\armor_d.dds|props\\armor_n.dds|spec:props\\armor_s.dds|sm:1.5|sg:0.4|fp:5",
        );

        let part = &CpuArt::load(&art).unwrap().submeshes[0];

        assert_eq!(
            part.normal_key.as_deref(),
            Some("textures\\props\\armor_n.dds")
        );
        assert_eq!(
            part.specular_key.as_deref(),
            Some("textures\\props\\armor_s.dds")
        );
        assert!((part.specular_scale - 1.5).abs() < 1e-6);
        assert!((part.smoothness - 0.4).abs() < 1e-6);
    }

    #[test]
    fn a_part_whose_material_has_no_specular_parameters_is_matte() {
        let dir = tempfile::tempdir().unwrap();
        let art = cube_art(dir.path(), "props\\crate_d.dds|spec:props\\crate_s.dds");

        let part = &CpuArt::load(&art).unwrap().submeshes[0];

        assert!(part.specular_scale.abs() < 1e-6);
    }

    #[test]
    fn an_unavailable_specular_map_is_black_while_the_other_neutral_texels_are_not() {
        assert_eq!(TextureRole::Specular.absent_texel(), [255; 4]);
        assert_eq!(TextureRole::Specular.unavailable_texel(), [0, 0, 0, 255]);
        assert_eq!(
            TextureRole::Normal.unavailable_texel(),
            [128, 128, 255, 255]
        );
        assert_eq!(TextureRole::Diffuse.unavailable_texel(), [255; 4]);
    }

    #[test]
    fn only_the_diffuse_map_is_decoded_as_colour() {
        assert!(TextureRole::Diffuse.is_srgb());
        assert!(!TextureRole::Normal.is_srgb());
        assert!(!TextureRole::Specular.is_srgb());
    }
}
