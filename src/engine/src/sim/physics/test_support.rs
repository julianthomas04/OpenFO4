//! Builders for sim-level tests. Everything is the real thing: a real rapier
//! world, the real shape bake and a real terrain mesh; only the content is
//! synthetic and tiny.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{Quat, Vec3};

use super::{ActivationMode, PhysicsContext};
use crate::sim::bake::{BakedCellColliders, ShapeBakeCache};
use crate::sim::collision::{
    CellColliderBatch, ColShape, ObjectColliderGroup, PartLocal, PartShape, Placement,
    TerrainColliderSpec,
};
use crate::sim::{CELL_SIZE_WORLD, CellGrid};

/// The cell holding the ground quad and, in most tests, the objects.
pub const GROUND_CELL: CellGrid = (0, 0);
/// Cell below `GROUND_CELL` on the grid's Y axis (engine -Z), for objects that must outlive the ground cell.
pub const NEIGHBOUR_CELL: CellGrid = (0, 1);
const GROUND_WIDTH: f32 = 400.0;

/// A flat terrain quad at height 0 covering both `GROUND_CELL` and `NEIGHBOUR_CELL`, normals up.
pub fn ground_batch() -> CellColliderBatch {
    let depth = CELL_SIZE_WORLD * 2.0 - 20.0;
    let vertices = vec![
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(GROUND_WIDTH, 0.0, 0.0),
        Vec3::new(GROUND_WIDTH, 0.0, -depth),
        Vec3::new(0.0, 0.0, -depth),
    ];
    CellColliderBatch {
        terrain: Some(TerrainColliderSpec {
            vertices,
            indices: vec![[0, 1, 2], [0, 2, 3]],
        }),
        objects: vec![],
    }
}

pub fn placement(ref_key: &str, at: Vec3) -> Placement {
    Placement {
        translation: at,
        rotation: Quat::IDENTITY,
        scale: 1.0,
        ref_key: Some(ref_key.to_owned()),
        ragdoll_pose: None,
        part_local: None,
    }
}

/// A part of the weapon REFR `ref_key` at `at`, offset by `local` inside it.
pub fn part_placement(ref_key: &str, at: Vec3, local: Vec3) -> Placement {
    Placement {
        part_local: Some(PartLocal {
            translation: local,
            rotation: Quat::IDENTITY,
        }),
        ..placement(ref_key, at)
    }
}

/// A part of the weapon REFR `ref_key` at `at`, posed by `local` inside it.
pub fn part_placement_posed(ref_key: &str, at: Vec3, local: PartLocal) -> Placement {
    Placement {
        part_local: Some(local),
        ..placement(ref_key, at)
    }
}

pub fn box_shape(half_extent: f32) -> PartShape {
    box_shape_posed(half_extent, Vec3::ZERO, Quat::IDENTITY)
}

/// A box whose own pose inside its mesh is `center` and `rotation`.
pub fn box_shape_posed(half_extent: f32, center: Vec3, rotation: Quat) -> PartShape {
    PartShape {
        shape: ColShape::Box {
            center,
            rotation,
            half_extents: Vec3::splat(half_extent),
        },
        node: None,
        joint: None,
        material: 0,
    }
}

pub fn object_group(
    path: &str,
    placements: Vec<Placement>,
    dynamic: bool,
    mass: Option<f32>,
) -> ObjectColliderGroup {
    ObjectColliderGroup {
        path: path.to_owned(),
        placements,
        dynamic,
        mass,
        ragdoll_sidecar: None,
        water: None,
    }
}

/// Bakes `batch` through the real worker path; `shapes` answers the lazy shape loader by mesh path.
pub fn bake(batch: &CellColliderBatch, shapes: &[(&str, Vec<PartShape>)]) -> BakedCellColliders {
    let cache_dir = tempfile::tempdir().unwrap();
    ShapeBakeCache::new(cache_dir.path().to_owned()).bake_cell(
        batch,
        |path| {
            let found = shapes.iter().find(|(known, _)| *known == path);
            Arc::new(found.map(|(_, parts)| parts.clone()).unwrap_or_default())
        },
        |_| None,
        |_| None,
    )
}

/// A context in which activated bodies are simulated at once, as `OPENFO4_PHYSICS_ACTIVATION=wake`.
pub fn wake_context() -> PhysicsContext {
    let mut context = PhysicsContext::new();
    context.activation = ActivationMode::Wake;
    context
}

/// Runs fixed ticks the way `SimWorld::tick` does: the pipeline only steps when something needs it.
pub fn run_ticks(context: &mut PhysicsContext, ticks: u32) {
    for _ in 0..ticks {
        if context.needs_step() {
            context.step();
        }
    }
}

/// A minimal valid `.col.json` body.
pub const SIDECAR: &[u8] = br#"{"shapes":[{"type":"sphere","radius":1.0}]}"#;

/// Writes `contents` to `dir/name` and returns the path.
pub fn write_file(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, contents).unwrap();
    path
}

/// A real `.col.json` file in its own directory, standing in for the installer's output.
pub fn sidecar_file(contents: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = write_file(dir.path(), "crate.col.json", contents);
    (dir, path)
}

/// Sets the file's modification time to `seconds` after the Unix epoch.
pub fn set_mtime(path: &Path, seconds: u64) {
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
        .unwrap();
}
