//! The box around a model's collision as its physics body is built, convex radius included.

use glam::Vec3;
use rapier3d::prelude::Pose;

use super::bake::merge_shapes_of_material;
use super::collision::{PartLocal, PartShape};

/// An axis-aligned box in model space, engine units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CollisionBox {
    pub min: Vec3,
    pub max: Vec3,
}

impl CollisionBox {
    /// The box around the colliders built from `shapes` at scale 1, posed by `part` inside the
    /// model as the body poses them; `None` when no collider is built.
    pub fn of_shapes(shapes: &[PartShape], part: Option<PartLocal>, path: &str) -> Option<Self> {
        let part_pose = part.map(|local| Pose::from_parts(local.translation, local.rotation));
        merge_shapes_of_material(shapes.iter().map(|part| &part.shape), 0, 1.0, path)
            .iter()
            .map(|cached| {
                let pose = part_pose.map_or(cached.local_pose, |part| part * cached.local_pose);
                let aabb = cached.shape.compute_aabb(&pose);
                Self {
                    min: aabb.mins,
                    max: aabb.maxs,
                }
            })
            .reduce(Self::union)
    }

    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    pub fn center(self) -> Vec3 {
        (self.min + self.max) / 2.0
    }

    pub fn height(self) -> f32 {
        self.max.y - self.min.y
    }

    pub fn half_diagonal(self) -> f32 {
        (self.max - self.min).length() / 2.0
    }

    /// Half the diagonal of the box seen from above: how far it reaches from its vertical axis
    /// whatever its yaw.
    pub fn horizontal_half_extent(self) -> f32 {
        let size = self.max - self.min;
        size.x.hypot(size.z) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use glam::Quat;

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

    fn assert_near(actual: Vec3, expected: Vec3) {
        assert!(
            (actual - expected).length() < 1e-4,
            "{actual:?} against {expected:?}"
        );
    }

    fn slab() -> PartShape {
        part(ColShape::Box {
            center: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            half_extents: Vec3::new(2.0, 0.5, 1.0),
        })
    }

    #[test]
    fn the_box_spans_every_shape_of_the_model() {
        let shapes = [
            part(ColShape::Box {
                center: Vec3::new(1.0, 0.5, 0.0),
                rotation: Quat::IDENTITY,
                half_extents: Vec3::new(1.0, 0.5, 2.0),
            }),
            part(ColShape::Sphere {
                center: Vec3::new(0.0, 3.0, 0.0),
                radius: 1.0,
            }),
        ];
        let bound = CollisionBox::of_shapes(&shapes, None, "test").unwrap();
        assert_near(bound.min, Vec3::new(-1.0, 0.0, -2.0));
        assert_near(bound.max, Vec3::new(2.0, 4.0, 2.0));
        assert_near(bound.center(), Vec3::new(0.5, 2.0, 0.0));
        assert!((bound.height() - 4.0).abs() < 1e-4);
        assert!((bound.half_diagonal() - 41.0_f32.sqrt() / 2.0).abs() < 1e-4);
        assert!((bound.horizontal_half_extent() - 5.0 / 2.0).abs() < 1e-4);
    }

    #[test]
    fn a_convex_shape_counts_with_its_convex_radius() {
        let cube = [-1.0, 1.0]
            .into_iter()
            .flat_map(|x| [-1.0, 1.0].map(move |y| (x, y)))
            .flat_map(|(x, y)| [-1.0, 1.0].map(move |z| Vec3::new(x, y, z)))
            .collect();
        let shapes = [part(ColShape::Convex {
            center: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            vertices: cube,
            radius: 0.25,
        })];
        let bound = CollisionBox::of_shapes(&shapes, None, "test").unwrap();
        assert_near(bound.min, Vec3::splat(-1.25));
        assert_near(bound.max, Vec3::splat(1.25));
    }

    #[test]
    fn a_model_without_shapes_has_no_box() {
        assert_eq!(CollisionBox::of_shapes(&[], None, "test"), None);
    }

    #[test]
    fn a_posed_part_bounds_its_turned_shapes_not_its_turned_box() {
        let ball = part(ColShape::Sphere {
            center: Vec3::ZERO,
            radius: 1.0,
        });
        let turned = CollisionBox::of_shapes(
            &[ball],
            Some(PartLocal {
                translation: Vec3::new(0.0, 5.0, 0.0),
                rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
            }),
            "test",
        )
        .unwrap();
        assert_near(turned.min, Vec3::new(-1.0, 4.0, -1.0));
        assert_near(turned.max, Vec3::new(1.0, 6.0, 1.0));
    }

    #[test]
    fn a_posed_part_is_turned_and_moved_inside_the_model() {
        let turned = CollisionBox::of_shapes(
            &[slab()],
            Some(PartLocal {
                translation: Vec3::new(3.0, 0.0, 0.0),
                rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            }),
            "test",
        )
        .unwrap();
        assert_near(turned.min, Vec3::new(2.0, -0.5, -2.0));
        assert_near(turned.max, Vec3::new(4.0, 0.5, 2.0));
    }

    #[test]
    #[ignore = "needs a converted installation (OPENFO4_INSTALL_DIR or OPENFO4_DB)"]
    fn the_installed_toaster_box_matches_the_measured_size() {
        use crate::assets::db::collision_layers::PlayerCollisionLayers;
        use crate::assets::types::glb_key::glb_key;
        use crate::assets::{AssetRootEnv, resolve_asset_root};
        use crate::bethesda_space::UNITS_TO_ENGINE;
        use crate::sim::collision::load_sidecar_file;

        let asset_root = resolve_asset_root(
            &AssetRootEnv::from_process(),
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")),
        )
        .expect("set OPENFO4_INSTALL_DIR to a converted installation");
        let items = crate::assets::db::test_support::installed_database()
            .get_items()
            .expect("the installed items read");
        let toaster = items
            .iter()
            .find(|item| item.editor_id.as_deref() == Some("Toaster_01_PostWar"))
            .expect("the post-war toaster is installed");
        let key = glb_key(
            toaster
                .model_path
                .as_deref()
                .expect("the toaster has a model"),
        );
        let stem = key.strip_suffix(".glb").expect("a model key ends in .glb");
        let sidecar = format!("{}/{stem}.col.json", asset_root.display());
        let data = load_sidecar_file(&sidecar, &PlayerCollisionLayers::all_solid())
            .unwrap_or_else(|| panic!("{sidecar} loads"));
        let bound = CollisionBox::of_shapes(&data.shapes, None, &key).expect("it has collision");
        let measured = (
            bound.half_diagonal() / UNITS_TO_ENGINE,
            bound.height() / UNITS_TO_ENGINE,
        );
        assert!(
            (measured.0 - 13.93).abs() < 0.05 && (measured.1 - 16.8).abs() < 0.05,
            "{measured:?}"
        );
    }
}
