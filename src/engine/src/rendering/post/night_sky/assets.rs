//! The files of the sky objects: the eight textures of a moon, the star texture, the star dome
//! mesh and the two textures of the sun. A file that is missing or unusable is a typed error, logged once; the object it belongs
//! to is then not drawn, and [`SkyAssetStatus`] says why.

use std::io;
use std::path::Path;

use crate::assets::layout::texture_key;
use crate::assets::resolve::resolve_below;
use crate::assets::types::glb_key::glb_key;
use crate::assets::types::mesh::{CpuMesh, Topology};
use crate::rendering::lighting::time_of_day::{MoonKind, MoonPhase, texture_stem};
use crate::rendering::texture::{TextureAssetError, load_texture_asset_srgb};
use crate::rendering::vertex::Vertex;

const SKY_TEXTURE_FOLDER: &str = "sky";
const DDS_EXTENSION: &str = "dds";
const STAR_TEXTURE_STEM: &str = "skystars";

/// A moon texture whose longer edge is below this is the game's stand-in for a phase with
/// nothing to see (the new moon is an 8x8 opaque black square).
pub(super) const PLACEHOLDER_MAX_EDGE: u32 = 16;

#[derive(Debug, thiserror::Error)]
pub enum SkyAssetError {
    #[error("sky mesh '{key}' is not in the install: {source}")]
    NotFound {
        key: String,
        #[source]
        source: io::Error,
    },
    #[error("sky mesh '{key}' cannot be read: {source}")]
    Unreadable {
        key: String,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Texture(#[from] TextureAssetError),
    #[error("sky mesh '{key}' is unusable: {reason}")]
    Mesh { key: String, reason: String },
    #[error("sky mesh '{key}' has no triangles")]
    NoTriangles { key: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetFailureKind {
    NotFound,
    Unreadable,
    Texture,
    Mesh,
    NoTriangles,
}

/// Why an asset is not in use: the kind of the [`SkyAssetError`] and its message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetFailure {
    pub kind: AssetFailureKind,
    pub message: String,
}

impl SkyAssetError {
    pub fn kind(&self) -> AssetFailureKind {
        match self {
            Self::NotFound { .. } | Self::Texture(TextureAssetError::NotFound { .. }) => {
                AssetFailureKind::NotFound
            }
            Self::Unreadable { .. } | Self::Texture(TextureAssetError::Unreadable { .. }) => {
                AssetFailureKind::Unreadable
            }
            Self::Texture(TextureAssetError::Unusable { .. }) => AssetFailureKind::Texture,
            Self::Mesh { .. } => AssetFailureKind::Mesh,
            Self::NoTriangles { .. } => AssetFailureKind::NoTriangles,
        }
    }
}

/// Whether one asset of the night sky is in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetState {
    NotRequested,
    Loaded,
    /// Loaded, but a stand-in with nothing to draw.
    Placeholder,
    Failed(AssetFailure),
}

impl AssetState {
    pub(super) fn failed(error: &SkyAssetError) -> Self {
        Self::Failed(AssetFailure {
            kind: error.kind(),
            message: error.to_string(),
        })
    }
}

/// What the night sky has loaded, for the dev menu. `moon` is indexed by [`MoonPhase::index`]
/// and describes the moon kind of the latest update. The cloud textures load on demand, a
/// weather at a time; one that fails is only logged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkyAssetStatus {
    pub moon: [AssetState; MoonPhase::ALL.len()],
    pub star_texture: AssetState,
    pub star_mesh: AssetState,
    pub sun_texture: AssetState,
    pub sun_glare_texture: AssetState,
    pub cloud_mesh: AssetState,
}

impl Default for SkyAssetStatus {
    fn default() -> Self {
        Self {
            moon: std::array::from_fn(|_| AssetState::NotRequested),
            star_texture: AssetState::NotRequested,
            star_mesh: AssetState::NotRequested,
            sun_texture: AssetState::NotRequested,
            sun_glare_texture: AssetState::NotRequested,
            cloud_mesh: AssetState::NotRequested,
        }
    }
}

pub(super) fn moon_texture_key(kind: MoonKind, phase: MoonPhase) -> String {
    sky_texture_key(texture_stem(kind, phase))
}

pub(super) fn star_texture_key() -> String {
    sky_texture_key(STAR_TEXTURE_STEM)
}

fn sky_texture_key(stem: &str) -> String {
    texture_key(&format!(
        "{SKY_TEXTURE_FOLDER}\\{}.{DDS_EXTENSION}",
        stem.to_ascii_lowercase()
    ))
}

/// The path below `asset_root` that the mesh `key` names. A file that is not there is told apart
/// from one that cannot be looked up (permissions, a broken disk).
fn resolve(asset_root: &str, key: &str) -> Result<String, SkyAssetError> {
    resolve_below(Path::new(asset_root), key).map_err(|source| {
        let key = key.to_owned();
        if source.kind() == io::ErrorKind::NotFound {
            SkyAssetError::NotFound { key, source }
        } else {
            SkyAssetError::Unreadable { key, source }
        }
    })
}

/// The sRGB-decoding view of the texture `key` below `asset_root`.
pub(super) fn load_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    asset_root: &str,
    key: &str,
) -> Result<wgpu::TextureView, SkyAssetError> {
    Ok(load_texture_asset_srgb(device, queue, asset_root, key)?)
}

/// The indices of `mesh` when it is a triangle list, which must stay within its vertices; `None`
/// for a mesh that is not drawn as triangles.
pub(super) fn triangle_indices<'a>(
    key: &str,
    mesh: &'a CpuMesh,
) -> Result<Option<&'a [u32]>, SkyAssetError> {
    if mesh.topology != Topology::Triangles {
        return Ok(None);
    }
    if mesh
        .indices
        .iter()
        .any(|&index| index as usize >= mesh.vertices.len())
    {
        return Err(SkyAssetError::Mesh {
            key: key.to_owned(),
            reason: "an index points past the vertices".to_owned(),
        });
    }
    Ok(Some(&mesh.indices))
}

/// The index of the first vertex of a mesh of `added` vertices that is appended to a list of
/// `existing` ones, which all 32 bit indices must be able to address.
pub(super) fn first_vertex(key: &str, existing: usize, added: usize) -> Result<u32, SkyAssetError> {
    let addressable = existing
        .checked_add(added)
        .is_some_and(|total| u32::try_from(total).is_ok());
    if !addressable {
        return Err(SkyAssetError::Mesh {
            key: key.to_owned(),
            reason: "more vertices than a 32 bit index addresses".to_owned(),
        });
    }
    Ok(existing as u32)
}

/// The star dome's triangles with every part of the model in one vertex and index list.
#[derive(Debug, Clone)]
pub(super) struct StarGeometry {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl StarGeometry {
    pub(super) fn from_meshes(key: &str, meshes: Vec<CpuMesh>) -> Result<Self, SkyAssetError> {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for mesh in meshes {
            let Some(mesh_indices) = triangle_indices(key, &mesh)? else {
                continue;
            };
            let first = first_vertex(key, vertices.len(), mesh.vertices.len())?;
            indices.extend(mesh_indices.iter().map(|index| index + first));
            vertices.extend(mesh.vertices);
        }
        if indices.is_empty() {
            return Err(SkyAssetError::NoTriangles {
                key: key.to_owned(),
            });
        }
        Ok(Self { vertices, indices })
    }
}

/// The meshes of the sky model `model_path`, a `.nif` path converted to its GLB, in the
/// order of the file, and the key of the file.
pub(super) fn load_model_meshes(
    asset_root: &str,
    model_path: &str,
) -> Result<(String, Vec<CpuMesh>), SkyAssetError> {
    let key = glb_key(model_path);
    let relative = resolve(asset_root, &key)?;
    let model = CpuMesh::load_model(&format!("{asset_root}/{relative}")).map_err(|error| {
        SkyAssetError::Mesh {
            key: key.clone(),
            reason: format!("{error:#}"),
        }
    })?;
    Ok((key, model.meshes))
}

/// The star dome of the climate's sky model.
pub(super) fn load_star_geometry(
    asset_root: &str,
    model_path: &str,
) -> Result<StarGeometry, SkyAssetError> {
    let (key, meshes) = load_model_meshes(asset_root, model_path)?;
    StarGeometry::from_meshes(&key, meshes)
}

/// Logs a failed load once per file, so a missing texture does not repeat every climate change.
pub(super) fn report(error: &SkyAssetError) {
    crate::utils::warn_once(&format!("night-sky:{error}"), || {
        format!("{error}; the object is not drawn")
    });
}
