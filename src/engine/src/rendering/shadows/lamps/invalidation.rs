use glam::Vec3;

use crate::rendering::frustum::Aabb;

/// Where shadow casters changed since the lamp shadows last looked: cells
/// that integrated or retired, dynamic bodies that moved. Cached lamp tiles
/// whose range sphere touches a region must re-render.
#[derive(Debug, Default)]
pub struct ShadowInvalidations {
    regions: Vec<Aabb>,
    everything: bool,
}

impl ShadowInvalidations {
    pub fn region(&mut self, aabb: Aabb) {
        if !self.everything {
            self.regions.push(aabb);
        }
    }

    /// The whole resident world changed (worldspace switch, interior load).
    pub fn everything(&mut self) {
        self.everything = true;
        self.regions.clear();
    }

    #[must_use]
    pub fn take(&mut self) -> Self {
        std::mem::take(self)
    }

    pub fn is_everything(&self) -> bool {
        self.everything
    }

    pub fn touches_sphere(&self, center: Vec3, radius: f32) -> bool {
        self.everything
            || self.regions.iter().any(|aabb| {
                let nearest = center.clamp(aabb.min, aabb.max);
                nearest.distance_squared(center) <= radius * radius
            })
    }

    /// Whether a region may shadow into the light-aligned cylinder around
    /// `center` along the unit `axis` (towards the light), judged on each
    /// region's bounding sphere. Regions whose bounding diameter is below
    /// `min_extent` are skipped: casters that small are culled from the view.
    pub fn touches_cylinder(&self, center: Vec3, axis: Vec3, radius: f32, min_extent: f32) -> bool {
        self.everything
            || self.regions.iter().any(|aabb| {
                let region_center = (aabb.min + aabb.max) * 0.5;
                let extent = (aabb.max - aabb.min).length();
                if extent < min_extent {
                    return false;
                }
                let reach = radius + extent * 0.5;
                let offset = region_center - center;
                let along = offset.dot(axis);
                let radial = (offset - axis * along).length();
                radial <= reach && along >= -reach
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn region(min: Vec3, max: Vec3) -> Aabb {
        Aabb { min, max }
    }

    fn with_region(aabb: Aabb) -> ShadowInvalidations {
        let mut invalidations = ShadowInvalidations::default();
        invalidations.region(aabb);
        invalidations
    }

    #[test]
    fn nothing_changed_touches_nothing() {
        let none = ShadowInvalidations::default();
        assert!(!none.touches_sphere(Vec3::ZERO, 1000.0));
        assert!(!none.touches_cylinder(Vec3::ZERO, Vec3::Y, 1000.0, 0.0));
        assert!(!none.is_everything());
    }

    #[test]
    fn a_sphere_touches_a_region_it_overlaps() {
        let invalidations = with_region(region(Vec3::splat(10.0), Vec3::splat(12.0)));
        assert!(invalidations.touches_sphere(Vec3::splat(11.0), 0.1));
    }

    #[test]
    fn a_sphere_touches_a_region_exactly_at_its_radius() {
        let invalidations = with_region(region(
            Vec3::new(10.0, -1.0, -1.0),
            Vec3::new(12.0, 1.0, 1.0),
        ));
        assert!(invalidations.touches_sphere(Vec3::ZERO, 10.0));
        assert!(!invalidations.touches_sphere(Vec3::ZERO, 9.99));
    }

    #[test]
    fn a_sphere_measures_to_the_nearest_point_of_the_box_not_its_centre() {
        let invalidations =
            with_region(region(Vec3::new(0.0, 0.0, 0.0), Vec3::new(100.0, 1.0, 1.0)));
        assert!(invalidations.touches_sphere(Vec3::new(50.0, 3.0, 0.0), 2.5));
    }

    #[test]
    fn everything_touches_every_sphere_and_cylinder() {
        let mut invalidations = ShadowInvalidations::default();
        invalidations.everything();
        assert!(invalidations.is_everything());
        assert!(invalidations.touches_sphere(Vec3::splat(1e6), 0.0));
        assert!(invalidations.touches_cylinder(Vec3::splat(1e6), Vec3::Y, 0.0, 1e9));
    }

    #[test]
    fn regions_added_after_everything_are_not_kept() {
        let mut invalidations = ShadowInvalidations::default();
        invalidations.everything();
        invalidations.region(region(Vec3::ZERO, Vec3::ONE));
        assert!(invalidations.regions.is_empty());
    }

    #[test]
    fn everything_replaces_the_regions_collected_so_far() {
        let mut invalidations = with_region(region(Vec3::ZERO, Vec3::ONE));
        invalidations.everything();
        assert!(invalidations.regions.is_empty());
    }

    #[test]
    fn take_hands_over_the_changes_and_resets() {
        let mut invalidations = with_region(region(Vec3::ZERO, Vec3::ONE));
        let taken = invalidations.take();
        assert!(taken.touches_sphere(Vec3::ZERO, 0.1));
        assert!(!invalidations.touches_sphere(Vec3::ZERO, 0.1));
    }

    #[test]
    fn a_region_inside_the_light_cylinder_touches_it() {
        let invalidations = with_region(region(
            Vec3::new(4.0, 20.0, -1.0),
            Vec3::new(6.0, 22.0, 1.0),
        ));
        assert!(invalidations.touches_cylinder(Vec3::ZERO, Vec3::Y, 10.0, 0.0));
    }

    #[test]
    fn a_region_beside_the_light_cylinder_does_not_touch_it() {
        let invalidations = with_region(region(
            Vec3::new(40.0, 0.0, -1.0),
            Vec3::new(42.0, 2.0, 1.0),
        ));
        assert!(!invalidations.touches_cylinder(Vec3::ZERO, Vec3::Y, 10.0, 0.0));
    }

    #[test]
    fn a_region_towards_the_light_shadows_into_the_cylinder() {
        let invalidations = with_region(region(
            Vec3::new(-1.0, 5000.0, -1.0),
            Vec3::new(1.0, 5002.0, 1.0),
        ));
        assert!(invalidations.touches_cylinder(Vec3::ZERO, Vec3::Y, 10.0, 0.0));
    }

    #[test]
    fn a_region_far_behind_the_cylinder_away_from_the_light_cannot_shadow_into_it() {
        let invalidations = with_region(region(
            Vec3::new(-1.0, -5002.0, -1.0),
            Vec3::new(1.0, -5000.0, 1.0),
        ));
        assert!(!invalidations.touches_cylinder(Vec3::ZERO, Vec3::Y, 10.0, 0.0));
    }

    #[test]
    fn a_region_smaller_than_the_drawn_caster_size_is_skipped() {
        let invalidations = with_region(region(Vec3::ZERO, Vec3::splat(0.1)));
        assert!(invalidations.touches_cylinder(Vec3::ZERO, Vec3::Y, 10.0, 0.1));
        assert!(!invalidations.touches_cylinder(Vec3::ZERO, Vec3::Y, 10.0, 0.5));
    }
}
