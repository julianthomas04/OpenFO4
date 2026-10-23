//! Which per-cell grass draws a frame submits: a draw-distance test and a
//! frustum test against the bounds of each draw's blades.

use glam::{Vec3, Vec4};

use super::fade::distance_to_aabb;
use crate::assets::types::grass::GrassBlade;
use crate::rendering::frustum::Aabb;

pub struct GrassView {
    pub eye: Vec3,
    pub planes: [Vec4; 6],
}

impl GrassView {
    /// Blades beyond the draw distance have faded out completely, so a draw
    /// whose nearest blade could still show is the only one worth submitting.
    pub fn sees(&self, bounds: &Aabb, draw_distance: f32) -> bool {
        distance_to_aabb(self.eye, bounds) < draw_distance && !bounds.is_outside(&self.planes)
    }
}

/// Bounds of `blades`, each grown by `reach` (how far a blade's geometry can lie
/// from its position).
pub fn blades_bounds(blades: &[GrassBlade], reach: f32) -> Option<Aabb> {
    let reach = Vec3::splat(reach);
    Aabb::from_points(blades.iter().map(|blade| Vec3::from(blade.position))).map(|bounds| Aabb {
        min: bounds.min - reach,
        max: bounds.max + reach,
    })
}

/// Farthest a blade's vertices can lie from its position: the model's radius at
/// the largest height scale `1 + |k|`, plus the wind's largest lean.
pub fn blade_reach(model_radius: f32, blades: &[GrassBlade], max_lean: f32) -> f32 {
    let max_scale = blades
        .iter()
        .map(|blade| 1.0 + blade.height_k.abs())
        .fold(1.0, f32::max);
    model_radius * max_scale + max_lean
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals parsed or copied, not computed"
)]
mod tests {
    use super::*;
    use crate::rendering::frustum::clip_planes;
    use glam::Mat4;

    fn blade(position: [f32; 3], height_k: f32) -> GrassBlade {
        GrassBlade::new(position, height_k, [1.0, 0.0, 0.0], 1.0, [0.0, 1.0, 0.0])
    }

    #[test]
    fn bounds_span_every_blade_grown_by_the_reach() {
        let blades = [blade([1.0, 2.0, 3.0], 0.0), blade([-4.0, 5.0, 0.5], 0.0)];
        let bounds = blades_bounds(&blades, 0.5).unwrap();
        assert_eq!(bounds.min, Vec3::new(-4.5, 1.5, 0.0));
        assert_eq!(bounds.max, Vec3::new(1.5, 5.5, 3.5));
    }

    #[test]
    fn no_blades_have_no_bounds() {
        assert!(blades_bounds(&[], 1.0).is_none());
    }

    #[test]
    fn reach_scales_the_model_by_the_tallest_blade_and_adds_the_lean() {
        let blades = [blade([0.0; 3], -0.25), blade([0.0; 3], 0.5)];
        assert_eq!(blade_reach(4.0, &blades, 1.0), 4.0 * 1.5 + 1.0);
    }

    #[test]
    fn reach_counts_a_shrunken_blade_as_scaled_by_its_magnitude() {
        let blades = [blade([0.0; 3], -0.75)];
        assert_eq!(blade_reach(4.0, &blades, 0.0), 4.0 * 1.75);
    }

    #[test]
    fn reach_never_shrinks_the_model_below_its_own_radius() {
        assert_eq!(blade_reach(4.0, &[], 0.0), 4.0);
    }

    fn view(eye: Vec3) -> GrassView {
        let view_proj = Mat4::perspective_rh(1.0, 1.0, 0.1, 10_000.0)
            * Mat4::look_to_rh(eye, Vec3::NEG_Z, Vec3::Y);
        GrassView {
            eye,
            planes: clip_planes(&view_proj),
        }
    }

    const DRAW_DISTANCE: f32 = 450.0;

    fn cube_at(center: Vec3, half: f32) -> Aabb {
        Aabb {
            min: center - Vec3::splat(half),
            max: center + Vec3::splat(half),
        }
    }

    #[test]
    fn bounds_ahead_and_within_the_draw_distance_are_seen() {
        let view = view(Vec3::ZERO);
        assert!(view.sees(&cube_at(Vec3::new(0.0, 0.0, -100.0), 5.0), DRAW_DISTANCE));
    }

    #[test]
    fn bounds_beyond_the_draw_distance_are_culled() {
        let view = view(Vec3::ZERO);
        assert!(!view.sees(&cube_at(Vec3::new(0.0, 0.0, -600.0), 5.0), DRAW_DISTANCE));
    }

    #[test]
    fn a_draw_whose_nearest_edge_is_inside_the_distance_is_kept() {
        let view = view(Vec3::ZERO);
        assert!(view.sees(&cube_at(Vec3::new(0.0, 0.0, -454.0), 10.0), DRAW_DISTANCE));
    }

    #[test]
    fn bounds_behind_the_camera_are_culled_by_the_frustum() {
        let view = view(Vec3::ZERO);
        assert!(!view.sees(&cube_at(Vec3::new(0.0, 0.0, 100.0), 5.0), DRAW_DISTANCE));
    }
}
