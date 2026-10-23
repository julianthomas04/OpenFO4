//! Placed water (a water activator reference): its plane and the volume that
//! decides which positions it covers.
//!
//! The plane is built from the geometry's world transform (normal is the
//! geometry's local Z axis, `d = dot(normal, world translation)`), not from the
//! vertices. A point is in the water when it lies inside the mesh's `.col.json`
//! collision sidecar under the reference transform.

use std::sync::Arc;

use glam::{Mat4, Vec3};

use crate::sim::bake::{CachedShape, merge_shapes_of_material};
use crate::sim::collision::PartShape;

/// A normal this close to horizontal has no water height at a point.
const MIN_UPWARD_NORMAL: f32 = 1e-4;

/// The water plane of a surface in engine space (Y up).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SurfacePlane {
    pub normal: Vec3,
    pub distance: f32,
}

impl SurfacePlane {
    /// The plane of a water geometry placed by `world`: the geometry's local
    /// up axis (Bethesda Z, engine Y) and the world translation.
    pub fn of_transform(world: Mat4) -> Self {
        let normal = world.transform_vector3(Vec3::Y).normalize();
        Self {
            normal,
            distance: normal.dot(world.w_axis.truncate()),
        }
    }

    /// A horizontal plane at `height`.
    pub fn horizontal(height: f32) -> Self {
        Self {
            normal: Vec3::Y,
            distance: height,
        }
    }

    /// The plane's height above the engine XZ position; `None` for a vertical
    /// plane.
    pub fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        (self.normal.y.abs() > MIN_UPWARD_NORMAL)
            .then(|| (self.distance - self.normal.x * x - self.normal.z * z) / self.normal.y)
    }

    /// Signed distance of `point` from the plane.
    pub fn signed_distance(&self, point: Vec3) -> f32 {
        self.normal.dot(point) - self.distance
    }
}

/// The collision of a water mesh in its local space: the volume the water
/// fills.
#[derive(Clone)]
pub struct VolumeShapes {
    shapes: Vec<CachedShape>,
}

impl VolumeShapes {
    /// From a mesh's sidecar shapes. Trimeshes are kept as the sidecar has
    /// them but enclose nothing, so a mesh whose collision is only trimesh
    /// covers no point. `None` when no shape survived.
    pub fn from_sidecar(shapes: &[PartShape], mesh: &str) -> Option<Self> {
        let shapes = merge_shapes_of_material(shapes.iter().map(|part| &part.shape), 0, 1.0, mesh);
        (!shapes.is_empty()).then_some(Self { shapes })
    }

    fn contains(&self, local_point: Vec3) -> bool {
        self.shapes
            .iter()
            .any(|part| part.shape.contains_point(&part.local_pose, local_point))
    }
}

impl std::fmt::Debug for VolumeShapes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VolumeShapes({} shapes)", self.shapes.len())
    }
}

/// One placed water instance.
#[derive(Debug, Clone)]
pub struct PlacedWaterVolume {
    /// The WATR of the placed activator (`WNAM`); `None` when it has none.
    pub water_type: Option<Arc<str>>,
    pub plane: SurfacePlane,
    world: Mat4,
    world_to_local: Mat4,
    /// `None`: the mesh has no collision, so the instance covers no position.
    shapes: Option<Arc<VolumeShapes>>,
}

impl PlacedWaterVolume {
    pub fn new(
        world: Mat4,
        water_type: Option<Arc<str>>,
        shapes: Option<Arc<VolumeShapes>>,
    ) -> Self {
        Self {
            water_type,
            plane: SurfacePlane::of_transform(world),
            world,
            world_to_local: world.inverse(),
            shapes,
        }
    }

    /// The instance origin, which decides the cell that owns the reference.
    pub fn origin(&self) -> Vec3 {
        self.world.w_axis.truncate()
    }

    /// Whether the water volume contains an engine-space point.
    pub fn contains(&self, point: Vec3) -> bool {
        self.shapes
            .as_ref()
            .is_some_and(|shapes| shapes.contains(self.world_to_local.transform_point3(point)))
    }
}

#[cfg(test)]
mod tests {
    use glam::{Quat, Vec3};

    use super::*;
    use crate::sim::collision::ColShape;

    fn part(shape: ColShape) -> PartShape {
        PartShape {
            shape,
            node: None,
            joint: None,
            material: 0,
        }
    }

    /// The Water1024 sidecar: a slab from the plane (local y 0) down 1024 game
    /// units, 1024 wide (engine units: 102.4).
    fn slab() -> Arc<VolumeShapes> {
        let shapes = [part(ColShape::Box {
            center: Vec3::new(0.0, -51.2, 0.0),
            rotation: Quat::IDENTITY,
            half_extents: Vec3::new(51.2, 51.2, 51.2),
        })];
        Arc::new(VolumeShapes::from_sidecar(&shapes, "slab").unwrap())
    }

    #[test]
    fn the_plane_of_an_untilted_placement_is_horizontal_at_the_translation() {
        let plane = SurfacePlane::of_transform(Mat4::from_translation(Vec3::new(5.0, 12.0, -3.0)));
        assert!(plane.normal.abs_diff_eq(Vec3::Y, 1e-6));
        assert!((plane.distance - 12.0).abs() < 1e-5);
        assert_eq!(plane.height_at(100.0, -50.0), Some(12.0));
    }

    #[test]
    fn the_plane_ignores_vertex_height_and_uses_the_world_translation() {
        // A mesh whose surface sits above its origin is still placed on the
        // plane through the origin (TroughBathWater).
        let world = Mat4::from_translation(Vec3::new(0.0, 4.0, 0.0));
        let plane = SurfacePlane::of_transform(world);
        assert_eq!(plane.height_at(0.0, 0.0), Some(4.0));
        assert!(plane.signed_distance(Vec3::new(0.0, 6.0, 0.0)) > 0.0);
    }

    #[test]
    fn a_tilted_placement_gives_a_sloped_plane_through_the_translation() {
        let world =
            Mat4::from_rotation_translation(Quat::from_rotation_z(0.5), Vec3::new(0.0, 10.0, 0.0));
        let plane = SurfacePlane::of_transform(world);
        let rise = plane.height_at(1.0, 0.0).unwrap() - plane.height_at(0.0, 0.0).unwrap();
        assert!((plane.height_at(0.0, 0.0).unwrap() - 10.0).abs() < 1e-4);
        assert!((rise - 0.5_f32.tan()).abs() < 1e-4, "{rise}");
    }

    #[test]
    fn a_scaled_placement_keeps_a_unit_normal() {
        let world = Mat4::from_scale_rotation_translation(
            Vec3::splat(3.0),
            Quat::IDENTITY,
            Vec3::new(0.0, 2.0, 0.0),
        );
        let plane = SurfacePlane::of_transform(world);
        assert!((plane.normal.length() - 1.0).abs() < 1e-6);
        assert!((plane.distance - 2.0).abs() < 1e-5);
    }

    #[test]
    fn a_vertical_plane_has_no_height() {
        let world = Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
        assert_eq!(SurfacePlane::of_transform(world).height_at(0.0, 0.0), None);
    }

    #[test]
    fn the_volume_contains_points_between_plane_and_floor_inside_the_footprint() {
        let volume = PlacedWaterVolume::new(
            Mat4::from_translation(Vec3::new(100.0, 10.0, 200.0)),
            None,
            Some(slab()),
        );
        assert!(volume.contains(Vec3::new(100.0, 5.0, 200.0)));
        assert!(volume.contains(Vec3::new(140.0, 9.9, 160.0)));
        assert!(!volume.contains(Vec3::new(100.0, 10.5, 200.0)), "above");
        assert!(!volume.contains(Vec3::new(100.0, -95.0, 200.0)), "below");
        assert!(!volume.contains(Vec3::new(160.0, 5.0, 200.0)), "outside");
    }

    #[test]
    fn the_volume_follows_rotation_and_scale_of_the_placement() {
        let world = Mat4::from_scale_rotation_translation(
            Vec3::splat(2.0),
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            Vec3::ZERO,
        );
        let volume = PlacedWaterVolume::new(world, None, Some(slab()));
        // Scaled by two, the slab is 204.8 wide and 204.8 deep.
        assert!(volume.contains(Vec3::new(100.0, -100.0, 0.0)));
        assert!(!volume.contains(Vec3::new(105.0, -100.0, 0.0)));
        assert!(volume.contains(Vec3::new(0.0, -200.0, 0.0)));
        assert!(!volume.contains(Vec3::new(0.0, -210.0, 0.0)));
    }

    #[test]
    fn a_mesh_without_collision_covers_nothing() {
        let volume = PlacedWaterVolume::new(Mat4::IDENTITY, None, None);
        assert!(!volume.contains(Vec3::new(0.0, -1.0, 0.0)));
    }

    #[test]
    fn a_convex_sidecar_shape_is_a_volume_too() {
        let hull = part(ColShape::Convex {
            center: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            vertices: vec![
                Vec3::new(-1.0, 0.0, -1.0),
                Vec3::new(1.0, 0.0, -1.0),
                Vec3::new(-1.0, 0.0, 1.0),
                Vec3::new(1.0, 0.0, 1.0),
                Vec3::new(-1.0, -2.0, -1.0),
                Vec3::new(1.0, -2.0, -1.0),
                Vec3::new(-1.0, -2.0, 1.0),
                Vec3::new(1.0, -2.0, 1.0),
            ],
        });
        let shapes = Arc::new(VolumeShapes::from_sidecar(&[hull], "hull").unwrap());
        let volume = PlacedWaterVolume::new(Mat4::IDENTITY, None, Some(shapes));
        assert!(volume.contains(Vec3::new(0.0, -1.0, 0.0)));
        assert!(!volume.contains(Vec3::new(0.0, 1.0, 0.0)));
    }

    #[test]
    fn no_sidecar_shapes_means_no_volume() {
        assert!(VolumeShapes::from_sidecar(&[], "empty").is_none());
    }
}
