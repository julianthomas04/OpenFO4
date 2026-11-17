//! Clips of a character folder read by name, each pack imported once.

use std::collections::HashMap;

use thiserror::Error;

use super::character_assets::{
    AnimationFileError, CharacterAssets, CharacterAssetsError, SkeletonLoadError,
    load_animation_file,
};
use super::clip::Clip;
use super::skeleton::Skeleton;

#[derive(Debug, Error)]
pub enum ClipLibraryError {
    #[error("clip '{clip}' is not listed in the animation index")]
    ClipNotIndexed { clip: String },
    #[error("clip pack {0}")]
    Pack(#[from] AnimationFileError),
    #[error("clip pack '{path}' does not contain clip '{clip}'")]
    ClipNotInPack { path: String, clip: String },
}

/// Reads clips by name through the converter's index, importing each pack once.
pub struct ClipLibrary {
    assets: CharacterAssets,
    loaded: HashMap<String, Vec<Clip>>,
}

impl ClipLibrary {
    /// Opens the character folder `assets_dir` below `asset_root`.
    pub fn open(asset_root: &str, assets_dir: &str) -> Result<Self, CharacterAssetsError> {
        Ok(Self {
            assets: CharacterAssets::open(asset_root, assets_dir)?,
            loaded: HashMap::new(),
        })
    }

    /// Full path of the skeleton the index names.
    pub fn skeleton_path(&self) -> &str {
        &self.assets.skeleton_path
    }

    pub fn load_skeleton(&self) -> Result<Skeleton, SkeletonLoadError> {
        self.assets.load_skeleton()
    }

    pub fn clip(&mut self, name: &str) -> Result<&Clip, ClipLibraryError> {
        let file = self
            .assets
            .packs
            .iter()
            .find(|pack| pack.clips.iter().any(|clip| clip == name))
            .map(|pack| pack.file.clone())
            .ok_or_else(|| ClipLibraryError::ClipNotIndexed {
                clip: name.to_owned(),
            })?;
        let path = format!("{}/{file}", self.assets.dir);
        if !self.loaded.contains_key(&file) {
            let clips = load_animation_file(&path)?.clips;
            self.loaded.insert(file.clone(), clips);
        }
        self.loaded[&file]
            .iter()
            .find(|clip| clip.name() == name)
            .ok_or(ClipLibraryError::ClipNotInPack {
                path,
                clip: name.to_owned(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::character_assets::ANIM_INDEX_FILE;
    use crate::anim::test_glb::{TranslationClip, translation_pack_glb};

    const UNIT_DIR: &str = "meshes/Unit";
    const PACK_FILE: &str = "Pack.anims.glb";
    const PACKED_CLIP: &str = "Packed";
    const UNPACKED_CLIP: &str = "Unpacked";

    /// A pack holding one clip named `PACKED_CLIP` that moves one node over two keys.
    fn pack_glb() -> Vec<u8> {
        translation_pack_glb(
            r#"{"name":"Root"}"#,
            &[TranslationClip {
                name: PACKED_CLIP,
                extras: r#"{"cycle":"loop"}"#,
                node: 0,
                times: [0.0, 0.5],
                keys: [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            }],
        )
    }

    /// A character folder whose index lists both clips in the one pack, which holds only
    /// `PACKED_CLIP`.
    struct Fixture {
        root: tempfile::TempDir,
    }

    impl Fixture {
        fn new(pack: &[u8]) -> Self {
            let root = tempfile::tempdir().unwrap();
            let dir = root.path().join(UNIT_DIR);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(PACK_FILE), pack).unwrap();
            std::fs::write(
                dir.join(ANIM_INDEX_FILE),
                format!(
                    r#"{{"skeleton":"rig.glb","packs":[{{"file":"{PACK_FILE}","clips":["{PACKED_CLIP}","{UNPACKED_CLIP}"]}}]}}"#
                ),
            )
            .unwrap();
            Self { root }
        }

        fn library(&self) -> ClipLibrary {
            ClipLibrary::open(self.root.path().to_str().unwrap(), UNIT_DIR).unwrap()
        }

        fn remove_pack(&self) {
            std::fs::remove_file(self.root.path().join(UNIT_DIR).join(PACK_FILE)).unwrap();
        }
    }

    #[test]
    fn a_listed_clip_is_read_from_its_pack() {
        let fixture = Fixture::new(&pack_glb());
        let mut library = fixture.library();
        assert_eq!(library.clip(PACKED_CLIP).unwrap().name(), PACKED_CLIP);
    }

    #[test]
    fn a_clip_listed_for_a_pack_that_lacks_it_is_reported() {
        let fixture = Fixture::new(&pack_glb());
        assert!(matches!(
            fixture.library().clip(UNPACKED_CLIP),
            Err(ClipLibraryError::ClipNotInPack { clip, .. }) if clip == UNPACKED_CLIP
        ));
    }

    #[test]
    fn a_pack_that_is_no_gltf_is_reported() {
        let fixture = Fixture::new(b"not a glb");
        assert!(matches!(
            fixture.library().clip(PACKED_CLIP),
            Err(ClipLibraryError::Pack(AnimationFileError::Import { .. }))
        ));
    }

    #[test]
    fn a_pack_is_imported_once_for_all_its_clips() {
        let fixture = Fixture::new(&pack_glb());
        let mut library = fixture.library();
        library.clip(PACKED_CLIP).unwrap();
        fixture.remove_pack();
        assert_eq!(library.clip(PACKED_CLIP).unwrap().name(), PACKED_CLIP);
        assert!(matches!(
            library.clip(UNPACKED_CLIP),
            Err(ClipLibraryError::ClipNotInPack { .. })
        ));
    }
}
