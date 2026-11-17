//! The first-person camera heights, read from the converted first-person skeleton and clips: the
//! standing height is the `Camera` bone's rest position above the feet, and sneaking shifts it by
//! the constant `Camera` translation of the additive sneak offset clip. Bone and clip identifiers
//! below are asset names, not game values.

use glam::Vec3;
use thiserror::Error;

use super::character_assets::{SkeletonLoadError, rest_model_pose};
use super::clip::Clip;
use super::clip_library::{ClipLibrary, ClipLibraryError};
use super::pose::PoseError;
use super::skeleton::{JointIndex, Skeleton};

/// The bone at the character's feet.
const ROOT_BONE: &str = "Root";
const CAMERA_BONE: &str = "Camera";
const SNEAK_OFFSET_CLIP: &str = "Common/SneakOffset";
/// Engine units the keys of the sneak offset may differ by and still be one offset.
const OFFSET_KEY_TOLERANCE: f32 = 1e-4;

/// Heights of the first-person camera above the feet (the `Root` bone's origin), along the up
/// axis of the character's model space, in engine units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FirstPersonCamera {
    /// In the rest pose.
    pub standing_height: f32,
    /// Added to the standing height while sneaking.
    pub sneak_shift: f32,
}

#[derive(Debug, Error)]
pub enum FirstPersonEyeError {
    #[error(transparent)]
    Skeleton(#[from] SkeletonLoadError),
    #[error("first-person skeleton '{path}' has no '{bone}' bone")]
    BoneMissing { path: String, bone: &'static str },
    #[error("first-person skeleton '{path}': '{CAMERA_BONE}' does not descend from '{ROOT_BONE}'")]
    CameraOutsideRoot { path: String },
    #[error("first-person skeleton '{path}': {source}")]
    RestPose { path: String, source: PoseError },
    #[error(transparent)]
    Clips(#[from] ClipLibraryError),
    #[error("clip '{clip}' does not translate the '{CAMERA_BONE}' bone")]
    CameraNotTranslated { clip: String },
    #[error("clip '{clip}' translates the '{CAMERA_BONE}' bone by more than one offset")]
    OffsetNotConstant { clip: String },
}

/// Reads the camera heights from the first-person character folder. A missing or malformed
/// skeleton, bone or clip is an error.
pub fn read_first_person_camera(
    library: &mut ClipLibrary,
) -> Result<FirstPersonCamera, FirstPersonEyeError> {
    let path = library.skeleton_path().to_owned();
    let skeleton = library.load_skeleton()?;
    let rest_model =
        rest_model_pose(&skeleton).map_err(|source| FirstPersonEyeError::RestPose {
            path: path.clone(),
            source,
        })?;
    let bone = |name: &'static str| {
        skeleton
            .index_of(name)
            .ok_or_else(|| FirstPersonEyeError::BoneMissing {
                path: path.clone(),
                bone: name,
            })
    };
    let root = bone(ROOT_BONE)?;
    let camera = bone(CAMERA_BONE)?;
    let camera_parent = skeleton.parents()[usize::from(camera)]
        .filter(|&parent| descends_from(&skeleton, parent, root))
        .ok_or_else(|| FirstPersonEyeError::CameraOutsideRoot { path: path.clone() })?;

    let model_position = |joint: JointIndex| rest_model[usize::from(joint)].w_axis.truncate();
    let standing_height = (model_position(camera) - model_position(root)).y;
    let offset = camera_offset(library.clip(SNEAK_OFFSET_CLIP)?, &skeleton, camera)?;
    let sneak_shift = rest_model[usize::from(camera_parent)]
        .transform_vector3(offset)
        .y;
    Ok(FirstPersonCamera {
        standing_height,
        sneak_shift,
    })
}

/// Whether `joint` is `ancestor` or lies below it.
fn descends_from(skeleton: &Skeleton, mut joint: JointIndex, ancestor: JointIndex) -> bool {
    loop {
        if joint == ancestor {
            return true;
        }
        match skeleton.parents()[usize::from(joint)] {
            Some(parent) => joint = parent,
            None => return false,
        }
    }
}

/// The one translation every key of the additive clip applies to the `camera` joint, in the
/// joint's parent space. The track binds to the joint by the skeleton's own name rule.
fn camera_offset(
    clip: &Clip,
    skeleton: &Skeleton,
    camera: JointIndex,
) -> Result<Vec3, FirstPersonEyeError> {
    let not_translated = || FirstPersonEyeError::CameraNotTranslated {
        clip: clip.name().to_owned(),
    };
    let keys = clip
        .tracks()
        .iter()
        .find(|track| skeleton.index_of(&track.node_name) == Some(camera))
        .and_then(|track| track.translation.as_ref())
        .ok_or_else(not_translated)?;
    let (&offset, rest) = keys.values().split_first().ok_or_else(not_translated)?;
    if rest
        .iter()
        .any(|key| !key.abs_diff_eq(offset, OFFSET_KEY_TOLERANCE))
    {
        return Err(FirstPersonEyeError::OffsetNotConstant {
            clip: clip.name().to_owned(),
        });
    }
    Ok(offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::character_assets::FIRST_PERSON_ASSETS_DIR;
    use crate::anim::character_assets::{ANIM_INDEX_FILE, AnimationFileError};
    use crate::anim::test_glb::{TranslationClip, glb_container, translation_pack_glb};

    const UNIT_DIR: &str = "meshes/Unit";
    /// Not the vanilla name, so the loader must take it from the index.
    const SKELETON_FILE: &str = "rig.glb";
    const PACK_FILE: &str = "Common.anims.glb";
    const PACK_ROOT_NODE: usize = 0;
    const PACK_CAMERA_NODE: usize = 1;

    #[derive(Clone, Copy)]
    struct Spec {
        skeleton_nodes: &'static str,
        pack_camera_name: &'static str,
        animated_node: usize,
        keys: [[f32; 3]; 2],
    }

    /// The vanilla layout: `Camera` 12 above an untransformed `Root`, turned so its own Y axis
    /// points along the model's Z axis, and a sneak offset of 4 down.
    const VANILLA: Spec = Spec {
        skeleton_nodes: r#"{"name":"skeleton.nif","children":[1]},{"name":"Root","children":[2]},{"name":"Camera","translation":[0,12,0],"rotation":[0.70710677,0,0,0.70710677]}"#,
        pack_camera_name: "Camera",
        animated_node: PACK_CAMERA_NODE,
        keys: [[0.0, -4.0, 0.0], [0.0, -4.0, 0.0]],
    };

    fn skeleton_glb(nodes: &str) -> Vec<u8> {
        glb_container(
            &format!(
                r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{nodes}]}}"#
            ),
            &[],
        )
    }

    /// A pack with the sneak offset clip translating one node over two keys.
    fn pack_glb(spec: Spec) -> Vec<u8> {
        translation_pack_glb(
            &format!(
                r#"{{"name":"Root","children":[1]}},{{"name":"{}"}}"#,
                spec.pack_camera_name
            ),
            &[TranslationClip {
                name: SNEAK_OFFSET_CLIP,
                extras: r#"{"cycle":"loop"}"#,
                node: spec.animated_node,
                times: [0.0, 0.033],
                keys: spec.keys,
            }],
        )
    }

    struct Fixture {
        root: tempfile::TempDir,
    }

    impl Fixture {
        fn new(spec: Spec) -> Self {
            let root = tempfile::tempdir().unwrap();
            let dir = root.path().join(UNIT_DIR);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(SKELETON_FILE), skeleton_glb(spec.skeleton_nodes)).unwrap();
            std::fs::write(dir.join(PACK_FILE), pack_glb(spec)).unwrap();
            std::fs::write(
                dir.join(ANIM_INDEX_FILE),
                format!(
                    r#"{{"skeleton":"{SKELETON_FILE}","packs":[{{"file":"{PACK_FILE}","clips":["{SNEAK_OFFSET_CLIP}"]}}]}}"#
                ),
            )
            .unwrap();
            Self { root }
        }

        fn unit(&self) -> std::path::PathBuf {
            self.root.path().join(UNIT_DIR)
        }

        fn read(&self) -> Result<FirstPersonCamera, FirstPersonEyeError> {
            let mut library =
                ClipLibrary::open(self.root.path().to_str().unwrap(), UNIT_DIR).unwrap();
            read_first_person_camera(&mut library)
        }
    }

    fn assert_camera(spec: Spec, standing_height: f32, sneak_shift: f32) {
        let camera = Fixture::new(spec).read().unwrap();
        assert!(
            (camera.standing_height - standing_height).abs() < 1e-5
                && (camera.sneak_shift - sneak_shift).abs() < 1e-5,
            "{camera:?}, wanted standing {standing_height} and shift {sneak_shift}"
        );
    }

    #[test]
    fn the_camera_rest_height_stands_and_the_sneak_offset_shifts_it() {
        assert_camera(VANILLA, 12.0, -4.0);
    }

    #[test]
    fn heights_are_measured_from_the_root_bone_not_the_model_origin() {
        let lifted_root = Spec {
            skeleton_nodes: r#"{"name":"skeleton.nif","children":[1]},{"name":"Root","translation":[0,2,0],"children":[2]},{"name":"Camera","translation":[0,12,0]}"#,
            ..VANILLA
        };
        assert_camera(lifted_root, 12.0, -4.0);
    }

    #[test]
    fn heights_are_read_along_the_model_up_axis_in_model_units() {
        let scaled_and_yawed_root = Spec {
            skeleton_nodes: r#"{"name":"skeleton.nif","children":[1]},{"name":"Root","translation":[0,1,0],"rotation":[0,0.70710677,0,0.70710677],"scale":[2,2,2],"children":[2]},{"name":"Camera","translation":[0,6,0]}"#,
            ..VANILLA
        };
        assert_camera(scaled_and_yawed_root, 12.0, -8.0);
    }

    #[test]
    fn the_sneak_offset_is_applied_in_the_space_of_the_camera_parent() {
        let halved_parent = Spec {
            skeleton_nodes: r#"{"name":"skeleton.nif","children":[1]},{"name":"Root","children":[2]},{"name":"Pivot","scale":[0.5,0.5,0.5],"children":[3]},{"name":"Camera","translation":[0,24,0]}"#,
            ..VANILLA
        };
        assert_camera(halved_parent, 12.0, -2.0);
    }

    #[test]
    fn the_offset_track_binds_to_the_camera_by_the_skeleton_name_rule() {
        let other_case = Spec {
            pack_camera_name: "CAMERA",
            ..VANILLA
        };
        assert_camera(other_case, 12.0, -4.0);
    }

    #[test]
    fn a_missing_skeleton_is_reported() {
        let fixture = Fixture::new(VANILLA);
        std::fs::remove_file(fixture.unit().join(SKELETON_FILE)).unwrap();
        assert!(matches!(
            fixture.read(),
            Err(FirstPersonEyeError::Skeleton(SkeletonLoadError::File(
                AnimationFileError::Missing { .. }
            )))
        ));
    }

    #[test]
    fn a_skeleton_that_is_no_gltf_is_reported() {
        let fixture = Fixture::new(VANILLA);
        std::fs::write(fixture.unit().join(SKELETON_FILE), b"not a glb").unwrap();
        assert!(matches!(
            fixture.read(),
            Err(FirstPersonEyeError::Skeleton(SkeletonLoadError::File(
                AnimationFileError::Import { .. }
            )))
        ));
    }

    #[test]
    fn a_skeleton_without_a_camera_bone_is_reported() {
        let no_camera = Spec {
            skeleton_nodes: r#"{"name":"skeleton.nif","children":[1]},{"name":"Root"}"#,
            ..VANILLA
        };
        assert!(matches!(
            Fixture::new(no_camera).read(),
            Err(FirstPersonEyeError::BoneMissing {
                bone: CAMERA_BONE,
                ..
            })
        ));
    }

    #[test]
    fn a_skeleton_without_a_root_bone_is_reported() {
        let no_root = Spec {
            skeleton_nodes: r#"{"name":"skeleton.nif","children":[1]},{"name":"Camera","translation":[0,12,0]}"#,
            ..VANILLA
        };
        assert!(matches!(
            Fixture::new(no_root).read(),
            Err(FirstPersonEyeError::BoneMissing {
                bone: ROOT_BONE,
                ..
            })
        ));
    }

    #[test]
    fn a_camera_beside_the_root_bone_is_reported() {
        let camera_beside_root = Spec {
            skeleton_nodes: r#"{"name":"skeleton.nif","children":[1,2]},{"name":"Root"},{"name":"Camera","translation":[0,12,0]}"#,
            ..VANILLA
        };
        assert!(matches!(
            Fixture::new(camera_beside_root).read(),
            Err(FirstPersonEyeError::CameraOutsideRoot { .. })
        ));
    }

    #[test]
    fn an_index_without_the_sneak_offset_clip_is_reported() {
        let fixture = Fixture::new(VANILLA);
        std::fs::write(
            fixture.unit().join(ANIM_INDEX_FILE),
            format!(r#"{{"skeleton":"{SKELETON_FILE}","packs":[]}}"#),
        )
        .unwrap();
        assert!(matches!(
            fixture.read(),
            Err(FirstPersonEyeError::Clips(ClipLibraryError::ClipNotIndexed { clip }))
                if clip == SNEAK_OFFSET_CLIP
        ));
    }

    #[test]
    fn a_sneak_offset_that_leaves_the_camera_alone_is_reported() {
        let root_moved = Spec {
            animated_node: PACK_ROOT_NODE,
            ..VANILLA
        };
        assert!(matches!(
            Fixture::new(root_moved).read(),
            Err(FirstPersonEyeError::CameraNotTranslated { .. })
        ));
    }

    fn keys_apart_by(difference: f32) -> Spec {
        Spec {
            keys: [[0.0, -4.0, 0.0], [0.0, -4.0 + difference, 0.0]],
            ..VANILLA
        }
    }

    #[test]
    fn sneak_offset_keys_within_the_tolerance_are_one_offset() {
        assert_camera(keys_apart_by(OFFSET_KEY_TOLERANCE * 0.5), 12.0, -4.0);
    }

    #[test]
    fn a_sneak_offset_whose_keys_differ_by_more_than_the_tolerance_is_reported() {
        for difference in [OFFSET_KEY_TOLERANCE * 1.5, 1.0] {
            assert!(
                matches!(
                    Fixture::new(keys_apart_by(difference)).read(),
                    Err(FirstPersonEyeError::OffsetNotConstant { .. })
                ),
                "{difference}"
            );
        }
    }

    /// Reads the converted first-person assets of an installed game; the expected values are the
    /// vanilla `Camera` rest height and `SneakOffset` translation.
    #[test]
    #[ignore = "needs the converted first-person skeleton and animations in the install"]
    fn the_real_installation_yields_the_vanilla_camera_heights() {
        let assets = crate::assets::resolve_asset_root(
            &crate::assets::AssetRootEnv::from_process(),
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
        )
        .expect("set OPENFO4_INSTALL_DIR to a converted installation");
        let mut library =
            ClipLibrary::open(assets.to_str().unwrap(), FIRST_PERSON_ASSETS_DIR).unwrap();
        let camera = read_first_person_camera(&mut library).unwrap();
        assert!(
            (camera.standing_height - 12.0483).abs() < 0.001,
            "{camera:?}"
        );
        assert!((camera.sneak_shift + 4.0).abs() < 0.001, "{camera:?}");
    }
}
