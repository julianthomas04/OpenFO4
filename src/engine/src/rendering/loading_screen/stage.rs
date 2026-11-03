//! The stage behind a large loading screen's model: the game's `LoadScreen02` mesh, a background
//! plane and an additive overlay plane, each an effect-shader shape. The game ships it in three
//! widths of the same height; the one whose shape is nearest the screen's is drawn. Its planes
//! also fix the camera (see [`super::stage_framing`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::Vec3;

use super::stage_framing::{StageFraming, StageFramingError, StagePlane};
use crate::assets::types::effect_material::{BlendMode, EffectMaterial};
use crate::assets::types::glb_key::glb_key;
use crate::assets::types::mesh::{CpuMesh, Topology};
use crate::rendering::vertex::Vertex;

/// Below the meshes folder: the stage in each width the game ships, 16:9 first, then the
/// ultrawide and the super-ultrawide one.
pub const STAGE_MODELS: [&str; 3] = [
    r"Interface\Objects\LoadScreen\LoadScreen02.nif",
    r"Interface\Objects\LoadScreen\LoadScreen02UW.nif",
    r"Interface\Objects\LoadScreen\LoadScreen02SUW.nif",
];

/// The converted file of each of [`STAGE_MODELS`], below the asset root.
pub fn stage_paths(asset_root: &Path) -> Vec<PathBuf> {
    STAGE_MODELS
        .iter()
        .map(|model| asset_root.join(glb_key(model)))
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum StageError {
    #[error("the loading screen stage '{}' is not installed", path.display())]
    Missing { path: PathBuf },
    #[error("the loading screen stage '{}' is not a usable model: {source}", path.display())]
    Invalid {
        path: PathBuf,
        #[source]
        source: anyhow::Error,
    },
    #[error(
        "the loading screen stage '{}' has {count} effect-shader planes facing the camera; it needs at least 2",
        path.display()
    )]
    TooFewPlanes { path: PathBuf, count: usize },
    #[error("the loading screen stage '{}' has a shape that is not an effect-shader plane facing the camera", path.display())]
    NotAnEffectPlane { path: PathBuf },
    #[error("the loading screen stage '{}' frames no camera: {source}", path.display())]
    Framing {
        path: PathBuf,
        #[source]
        source: StageFramingError,
    },
    #[error("no loading screen stage is usable: {0}")]
    NoneUsable(String),
}

/// One plane of the stage and the effect material that draws it.
#[derive(Debug, Clone)]
pub struct StageLayer {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub plane: StagePlane,
    pub material: Arc<EffectMaterial>,
}

impl StageLayer {
    pub fn is_opaque(&self) -> bool {
        self.material.blend.mode == BlendMode::None
    }
}

/// One width of the stage, read from disk.
#[derive(Debug, Clone)]
pub struct CpuStage {
    /// In the order they are drawn: the opaque ones, then the blended ones, each in file order.
    pub layers: Vec<StageLayer>,
    pub framing: StageFraming,
    /// Width over height of the farthest plane, the one that has to cover the screen.
    pub aspect: f32,
}

impl CpuStage {
    pub fn load(path: &Path) -> Result<Self, StageError> {
        if !path.is_file() {
            return Err(StageError::Missing {
                path: path.to_path_buf(),
            });
        }
        let model =
            CpuMesh::load_model(&path.to_string_lossy()).map_err(|source| StageError::Invalid {
                path: path.to_path_buf(),
                source,
            })?;
        Self::from_meshes(model.meshes, path)
    }

    fn from_meshes(meshes: Vec<CpuMesh>, path: &Path) -> Result<Self, StageError> {
        let not_a_plane = || StageError::NotAnEffectPlane {
            path: path.to_path_buf(),
        };
        let mut layers = meshes
            .into_iter()
            .filter(|mesh| mesh.topology == Topology::Triangles && !mesh.indices.is_empty())
            .map(|mesh| {
                let material = mesh.effect.clone().ok_or_else(not_a_plane)?;
                let plane = StagePlane::from_positions(
                    mesh.vertices
                        .iter()
                        .map(|vertex| Vec3::from(vertex.position)),
                )
                .ok_or_else(not_a_plane)?;
                Ok(StageLayer {
                    vertices: mesh.vertices,
                    indices: mesh.indices,
                    plane,
                    material,
                })
            })
            .collect::<Result<Vec<_>, StageError>>()?;
        let Some((nearest, farthest)) = nearest_and_farthest(&layers) else {
            return Err(StageError::TooFewPlanes {
                path: path.to_path_buf(),
                count: layers.len(),
            });
        };
        let framing =
            StageFraming::of_planes(nearest, farthest).map_err(|source| StageError::Framing {
                path: path.to_path_buf(),
                source,
            })?;
        layers.sort_by_key(|layer| !layer.is_opaque());
        Ok(Self {
            layers,
            framing,
            aspect: farthest.aspect(),
        })
    }
}

/// The planes of the layers nearest and farthest from the camera; `None` with fewer than two.
fn nearest_and_farthest(layers: &[StageLayer]) -> Option<(StagePlane, StagePlane)> {
    if layers.len() < 2 {
        return None;
    }
    let planes = || layers.iter().map(|layer| layer.plane);
    let nearest = planes().max_by(|a, b| a.depth.total_cmp(&b.depth))?;
    let farthest = planes().min_by(|a, b| a.depth.total_cmp(&b.depth))?;
    Some((nearest, farthest))
}

/// The index of the shape in `aspects` nearest `screen_aspect`, both width over height, compared
/// by ratio, so a stage twice as wide as the screen is as far from it as one half as wide.
/// `None` without shapes or for a screen without a usable shape.
pub fn nearest_aspect(aspects: &[f32], screen_aspect: f32) -> Option<usize> {
    if !(screen_aspect.is_finite() && screen_aspect > 0.0) {
        return None;
    }
    let distance = |aspect: f32| (aspect / screen_aspect).ln().abs();
    aspects
        .iter()
        .enumerate()
        .filter(|(_, aspect)| aspect.is_finite() && **aspect > 0.0)
        .min_by(|(_, a), (_, b)| distance(**a).total_cmp(&distance(**b)))
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::loading_screen::test_support::{TestStageLayer, write_stage};

    const SIXTEEN_NINE: f32 = 37.3 / 21.0;
    const ULTRAWIDE: f32 = 48.4 / 21.0;
    const SUPER_ULTRAWIDE: f32 = 74.0 / 21.0;
    const GAME_STAGES: [f32; 3] = [SIXTEEN_NINE, ULTRAWIDE, SUPER_ULTRAWIDE];

    #[test]
    fn each_common_screen_gets_the_stage_of_its_shape() {
        let pick = |width: f32, height: f32| nearest_aspect(&GAME_STAGES, width / height);
        assert_eq!(pick(2560.0, 1440.0), Some(0));
        assert_eq!(pick(1920.0, 1200.0), Some(0), "16:10 is nearest 16:9");
        assert_eq!(pick(1024.0, 768.0), Some(0));
        assert_eq!(
            pick(3440.0, 1440.0),
            Some(1),
            "21:9 gets the ultrawide stage"
        );
        assert_eq!(pick(2560.0, 1080.0), Some(1));
        assert_eq!(
            pick(5120.0, 1440.0),
            Some(2),
            "32:9 gets the super-ultrawide stage"
        );
        assert_eq!(
            pick(11520.0, 2160.0),
            Some(2),
            "wider than any stage gets the widest"
        );
    }

    #[test]
    fn shapes_are_compared_by_ratio() {
        let between = (SIXTEEN_NINE * ULTRAWIDE).sqrt();
        assert_eq!(nearest_aspect(&GAME_STAGES, between * 0.999), Some(0));
        assert_eq!(nearest_aspect(&GAME_STAGES, between * 1.001), Some(1));
    }

    #[test]
    fn no_shape_or_no_screen_shape_picks_nothing() {
        assert_eq!(nearest_aspect(&[], 1.5), None);
        assert_eq!(nearest_aspect(&GAME_STAGES, 0.0), None);
        assert_eq!(nearest_aspect(&GAME_STAGES, f32::NAN), None);
        assert_eq!(nearest_aspect(&[f32::NAN, 2.0], 1.0), Some(1));
    }

    #[test]
    fn the_stage_paths_name_the_converted_models_below_the_meshes_folder() {
        let paths = stage_paths(Path::new("root"));
        assert_eq!(paths.len(), 3);
        assert!(
            paths[1]
                .to_string_lossy()
                .ends_with(r"Interface\Objects\LoadScreen\LoadScreen02UW.glb"),
            "{:?}",
            paths[1]
        );
    }

    #[test]
    fn a_stage_frames_its_camera_and_draws_opaque_planes_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_stage(dir.path(), "stage.glb", &TestStageLayer::vanilla());

        let stage = CpuStage::load(&path).expect("the stage loads");

        assert!((stage.framing.vertical_fov_radians.to_degrees() - 23.98).abs() < 0.05);
        assert!((stage.aspect - 16.0 / 9.0).abs() < 0.01, "{}", stage.aspect);
        let opaque: Vec<bool> = stage.layers.iter().map(StageLayer::is_opaque).collect();
        assert_eq!(opaque, [true, false], "the overlay comes first in the file");
        assert!((stage.layers[1].material.base_color[3] - 0.6).abs() < 1e-6);
    }

    #[test]
    fn a_missing_stage_is_reported_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let error = CpuStage::load(&dir.path().join("none.glb")).unwrap_err();
        assert!(matches!(error, StageError::Missing { .. }), "{error}");
    }

    #[test]
    fn a_stage_of_one_plane_frames_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let [overlay, _] = TestStageLayer::vanilla();
        let path = write_stage(dir.path(), "stage.glb", &[overlay]);
        let error = CpuStage::load(&path).unwrap_err();
        assert!(
            matches!(error, StageError::TooFewPlanes { count: 1, .. }),
            "{error}"
        );
    }

    #[test]
    fn a_shape_without_an_effect_material_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let [overlay, mut background] = TestStageLayer::vanilla();
        background.effect = None;
        let path = write_stage(dir.path(), "stage.glb", &[overlay, background]);
        let error = CpuStage::load(&path).unwrap_err();
        assert!(
            matches!(error, StageError::NotAnEffectPlane { .. }),
            "{error}"
        );
    }
}
