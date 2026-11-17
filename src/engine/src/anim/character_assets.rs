//! A converted character asset folder: the converter's animation index, the clip packs it lists
//! and the skeleton it names.

use std::io::ErrorKind;
use std::path::Path;

use glam::Mat4;
use serde::Deserialize;
use thiserror::Error;

use super::gltf_import::{AnimationAsset, ImportError};
use super::pose::{PoseError, local_to_model};
use super::skeleton::Skeleton;
use crate::assets::layout::MESHES_DIR;

pub const ANIM_INDEX_FILE: &str = "skeleton.anims.json";
/// The first-person character folder below the asset root.
pub const FIRST_PERSON_ASSETS_DIR: &str = "meshes/Actors/Character/_1stPerson/CharacterAssets";

#[derive(Debug, Error)]
pub enum AnimIndexError {
    #[error("animation index '{path}' does not exist; run the installer's animation conversion")]
    Missing { path: String },
    #[error("animation index '{path}' cannot be read: {source}")]
    Unreadable {
        path: String,
        source: std::io::Error,
    },
    #[error("animation index '{path}' is malformed: {source}")]
    Malformed {
        path: String,
        source: serde_json::Error,
    },
}

#[derive(Debug, Error)]
pub enum SkeletonPathError {
    #[error("animation index '{path}' names skeleton '{name}', which is not a relative path")]
    NotRelative { path: String, name: String },
    #[error(
        "animation index '{path}' names skeleton '{name}', which leaves the '{MESHES_DIR}' folder"
    )]
    EscapesMeshes { path: String, name: String },
}

#[derive(Debug, Error)]
pub enum CharacterAssetsError {
    #[error(transparent)]
    Index(#[from] AnimIndexError),
    #[error(transparent)]
    SkeletonPath(#[from] SkeletonPathError),
}

/// A converted animation file (a skeleton or a clip pack) that could not be read.
#[derive(Debug, Error)]
pub enum AnimationFileError {
    #[error("'{path}' does not exist")]
    Missing { path: String },
    #[error("'{path}': {source}")]
    Import { path: String, source: ImportError },
}

#[derive(Debug, Error)]
pub enum SkeletonLoadError {
    #[error("skeleton {0}")]
    File(#[from] AnimationFileError),
    #[error("skeleton '{0}' has no joints")]
    Empty(String),
}

/// One clip pack as listed by the converter's index: the clip names are known
/// without opening the (possibly huge) pack file.
#[derive(Debug, Deserialize)]
pub struct ClipPackEntry {
    pub file: String,
    pub clips: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct AnimIndex {
    /// The converted NIF skeleton: the full render hierarchy the bodies are
    /// skinned to. The clips (Havok, a subset of its bones) bind to it by name.
    skeleton: String,
    packs: Vec<ClipPackEntry>,
}

/// A character folder as its index describes it.
pub struct CharacterAssets {
    /// The folder holding the index and the packs.
    pub dir: String,
    /// Full path of the skeleton the index names.
    pub skeleton_path: String,
    pub packs: Vec<ClipPackEntry>,
}

impl CharacterAssets {
    /// Reads the index of `assets_dir` (relative to `asset_root`) and resolves its skeleton.
    pub fn open(asset_root: &str, assets_dir: &str) -> Result<Self, CharacterAssetsError> {
        let dir = format!("{asset_root}/{assets_dir}");
        let index_path = format!("{dir}/{ANIM_INDEX_FILE}");
        let index = read_index(&index_path)?;
        let skeleton = resolve_skeleton_path(&index_path, assets_dir, &index.skeleton)?;
        Ok(Self {
            dir,
            skeleton_path: format!("{asset_root}/{skeleton}"),
            packs: index.packs,
        })
    }

    pub fn load_skeleton(&self) -> Result<Skeleton, SkeletonLoadError> {
        load_skeleton(&self.skeleton_path)
    }
}

fn read_index(path: &str) -> Result<AnimIndex, AnimIndexError> {
    let text = std::fs::read_to_string(path).map_err(|source| match source.kind() {
        ErrorKind::NotFound => AnimIndexError::Missing {
            path: path.to_owned(),
        },
        _ => AnimIndexError::Unreadable {
            path: path.to_owned(),
            source,
        },
    })?;
    serde_json::from_str(&text).map_err(|source| AnimIndexError::Malformed {
        path: path.to_owned(),
        source,
    })
}

/// Resolves the index's `skeleton` against the index folder (both relative to
/// the asset root) into a normalized asset-root-relative path, lexically: `..`
/// may climb out of the unit but never out of the meshes folder.
fn resolve_skeleton_path(
    index_path: &str,
    assets_dir: &str,
    skeleton: &str,
) -> Result<String, SkeletonPathError> {
    let not_relative = || SkeletonPathError::NotRelative {
        path: index_path.to_owned(),
        name: skeleton.to_owned(),
    };
    let escapes = || SkeletonPathError::EscapesMeshes {
        path: index_path.to_owned(),
        name: skeleton.to_owned(),
    };
    let name = skeleton.replace('\\', "/");
    if name.is_empty() || name.starts_with('/') || name.contains(':') {
        return Err(not_relative());
    }
    let mut resolved: Vec<&str> = assets_dir
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    for part in name.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                resolved.pop().ok_or_else(escapes)?;
            }
            normal => resolved.push(normal),
        }
    }
    if resolved.len() < 2 || resolved[0] != MESHES_DIR {
        return Err(escapes());
    }
    Ok(resolved.join("/"))
}

/// Imports the converted animation file at `path`, telling a missing file from a broken one.
pub fn load_animation_file(path: &str) -> Result<AnimationAsset, AnimationFileError> {
    if !Path::new(path).is_file() {
        return Err(AnimationFileError::Missing {
            path: path.to_owned(),
        });
    }
    AnimationAsset::load(path).map_err(|source| AnimationFileError::Import {
        path: path.to_owned(),
        source,
    })
}

pub fn load_skeleton(path: &str) -> Result<Skeleton, SkeletonLoadError> {
    let skeleton = load_animation_file(path)?.skeleton;
    if skeleton.is_empty() {
        return Err(SkeletonLoadError::Empty(path.to_owned()));
    }
    Ok(skeleton)
}

/// Model-space transform of every joint in the rest pose.
pub fn rest_model_pose(skeleton: &Skeleton) -> Result<Vec<Mat4>, PoseError> {
    let mut pose = vec![Mat4::IDENTITY; skeleton.len()];
    local_to_model(skeleton, skeleton.rest_pose(), &mut pose)?;
    Ok(pose)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::test_glb::glb_container;

    fn resolved(assets_dir: &str, skeleton: &str) -> Option<String> {
        resolve_skeleton_path("index", assets_dir, skeleton).ok()
    }

    #[test]
    fn a_skeleton_beside_the_index_resolves_below_the_meshes_folder() {
        assert_eq!(
            resolved("meshes/Actors/Dog/CharacterAssets", "skeleton.glb").as_deref(),
            Some("meshes/Actors/Dog/CharacterAssets/skeleton.glb")
        );
    }

    #[test]
    fn a_skeleton_may_climb_out_of_its_unit_but_not_out_of_the_meshes_folder() {
        assert_eq!(
            resolved(
                "meshes/Actors/Turret/CharacterAssets/Mounted",
                "..\\TurretMountedSkeleton.glb"
            )
            .as_deref(),
            Some("meshes/Actors/Turret/CharacterAssets/TurretMountedSkeleton.glb")
        );
        assert!(matches!(
            resolve_skeleton_path("index", "meshes/Actors/Dog", "../../../skeleton.glb"),
            Err(SkeletonPathError::EscapesMeshes { .. })
        ));
    }

    #[test]
    fn a_unit_outside_the_meshes_folder_of_the_layout_is_rejected() {
        assert!(resolved("textures/Actors/Dog", "skeleton.glb").is_none());
    }

    #[test]
    fn an_absolute_skeleton_path_is_not_relative() {
        assert!(matches!(
            resolve_skeleton_path("index", "meshes/Actors/Dog", "C:/skeleton.glb"),
            Err(SkeletonPathError::NotRelative { .. })
        ));
    }

    #[test]
    fn a_folder_without_an_index_is_reported() {
        let root = tempfile::tempdir().unwrap();
        assert!(matches!(
            CharacterAssets::open(root.path().to_str().unwrap(), "meshes/Unit"),
            Err(CharacterAssetsError::Index(AnimIndexError::Missing { .. }))
        ));
    }

    #[test]
    fn an_index_that_cannot_be_read_is_reported() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("meshes/Unit").join(ANIM_INDEX_FILE)).unwrap();
        assert!(matches!(
            CharacterAssets::open(root.path().to_str().unwrap(), "meshes/Unit"),
            Err(CharacterAssetsError::Index(
                AnimIndexError::Unreadable { .. }
            ))
        ));
    }

    #[test]
    fn a_skeleton_without_joints_is_reported() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rig.glb");
        std::fs::write(
            &path,
            glb_container(
                r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[]}],"nodes":[]}"#,
                &[],
            ),
        )
        .unwrap();
        assert!(matches!(
            load_skeleton(path.to_str().unwrap()),
            Err(SkeletonLoadError::Empty(_))
        ));
    }

    #[test]
    fn a_malformed_index_is_reported() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("meshes/Unit");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(ANIM_INDEX_FILE), r#"{"packs":[]}"#).unwrap();
        assert!(matches!(
            CharacterAssets::open(root.path().to_str().unwrap(), "meshes/Unit"),
            Err(CharacterAssetsError::Index(
                AnimIndexError::Malformed { .. }
            ))
        ));
    }

    #[test]
    fn the_skeleton_named_by_the_index_resolves_against_the_asset_root() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("meshes/Unit");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(ANIM_INDEX_FILE),
            r#"{"skeleton":"rig.glb","packs":[]}"#,
        )
        .unwrap();
        let root = root.path().to_str().unwrap();
        let assets = CharacterAssets::open(root, "meshes/Unit").unwrap();
        assert_eq!(assets.skeleton_path, format!("{root}/meshes/Unit/rig.glb"));
        assert!(matches!(
            assets.load_skeleton(),
            Err(SkeletonLoadError::File(AnimationFileError::Missing { .. }))
        ));
    }
}
