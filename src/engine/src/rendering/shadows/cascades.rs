use glam::{Mat4, Vec3, Vec4};

use super::settings::ShadowConfig;
use super::views::{ShadowView, ViewCull};
use crate::rendering::camera::Camera;

const PRACTICAL_SPLIT_LAMBDA: f32 = 0.75;
/// Sphere radii snap up to this step (engine units) so the texel size only
/// changes in discrete jumps, e.g. while the sprint FOV ramps.
const RADIUS_QUANTUM: f32 = 1.0;
/// Below this |`to_sun.y`| the light basis keeps world Y as its up axis.
const ZENITH_UP_SWITCH: f32 = 0.99;

#[derive(Debug, Copy, Clone)]
pub struct Cascade {
    pub view_proj: Mat4,
    /// xyz: world-space centre, w: radius.
    pub sphere: Vec4,
    pub texel_world: f32,
    pub min_texels: f32,
    pub skip_alpha: bool,
    /// False when the cascade map still holds an earlier render and this
    /// cascade carries the matrices that render used.
    pub refresh: bool,
}

/// Cascades fitted for one frame, nearest first.
#[derive(Debug, Clone)]
pub struct CascadeSet {
    pub cascades: Vec<Cascade>,
    pub to_sun: Vec3,
}

impl CascadeSet {
    /// Cascades from `eye_centered_from` on are centred on the eye and cover
    /// the whole circle around it, so turning the camera does not move them.
    /// `anchors` pin each cascade's texel grid and follow the camera across fits.
    pub fn fit(
        camera: &Camera,
        to_sun: Vec3,
        config: &ShadowConfig,
        eye_centered_from: usize,
        anchors: &mut GridAnchors,
    ) -> Self {
        let light_rotation = light_rotation(to_sun);
        let corner_slope_sq = corner_slope_squared(camera);
        let forward = (camera.target - camera.eye).normalize();
        let splits = practical_splits(camera.znear, config.distance, config.cascade_count);
        let last = config.cascade_count - 1;

        let cascades = (0..config.cascade_count)
            .map(|i| {
                let blend = if i == last { 0.0 } else { config.split_blend };
                let far = splits[i + 1] + blend;
                let (center, radius) = if i >= eye_centered_from {
                    (camera.eye, far * (1.0 + corner_slope_sq).sqrt())
                } else {
                    let (center_depth, radius) =
                        slice_bounding_sphere(splits[i], far, corner_slope_sq);
                    (camera.eye + forward * center_depth, radius)
                };
                let radius = (radius / RADIUS_QUANTUM).ceil() * RADIUS_QUANTUM;
                let texel_world = texel_world(radius, config);
                let anchor = anchors.follow(i, center, radius, texel_world, light_rotation);
                let mut cascade = fit_cascade(center, radius, light_rotation, anchor, config);
                cascade.min_texels = config.cascade_min_texels[i];
                cascade.skip_alpha = i == last && config.skip_alpha_in_last_cascade;
                cascade
            })
            .collect();

        Self { cascades, to_sun }
    }

    /// `None` for cascades whose map is kept from an earlier frame.
    pub fn views(&self) -> Vec<Option<ShadowView>> {
        self.cascades
            .iter()
            .map(|cascade| {
                cascade.refresh.then_some(ShadowView {
                    view_proj: cascade.view_proj,
                    cull: ViewCull::Ortho {
                        sphere: cascade.sphere,
                        to_light: self.to_sun,
                        texel_world: cascade.texel_world,
                        min_texels: cascade.min_texels,
                        skip_alpha: cascade.skip_alpha,
                    },
                })
            })
            .collect()
    }
}

/// Where each cascade's light-space texel grid is pinned. Snapping to the grid keeps shadows still
/// while the camera moves, but a turning sun swings every point about the pin, by its distance
/// from the pin times the angle: pinned at the world origin, a point thousands of units away
/// crosses many texels per frame and every shadow edge shimmers. So each pin stays within a
/// cascade radius of its cascade centre, and moves only by whole texels across the light, which
/// leaves the grid where it was.
#[derive(Debug, Clone, Default)]
pub struct GridAnchors {
    anchors: Vec<Option<Vec3>>,
}

impl GridAnchors {
    /// Forgets every pin; the next fit pins each grid on its cascade centre.
    pub fn reset(&mut self) {
        self.anchors.clear();
    }

    fn follow(
        &mut self,
        cascade: usize,
        center: Vec3,
        radius: f32,
        texel_world: f32,
        light_rotation: Mat4,
    ) -> Vec3 {
        if self.anchors.len() <= cascade {
            self.anchors.resize(cascade + 1, None);
        }
        let anchor = match self.anchors[cascade] {
            Some(anchor) if anchor.distance(center) <= radius => anchor,
            Some(anchor) => {
                let offset = light_rotation.transform_vector3(center - anchor);
                let whole_texels = Vec3::new(
                    snap_to_texel(offset.x, texel_world),
                    snap_to_texel(offset.y, texel_world),
                    offset.z,
                );
                anchor + light_rotation.inverse().transform_vector3(whole_texels)
            }
            None => center,
        };
        self.anchors[cascade] = Some(anchor);
        anchor
    }
}

fn snap_to_texel(light_space: f32, texel_world: f32) -> f32 {
    (light_space / texel_world).round() * texel_world
}

fn texel_world(radius: f32, config: &ShadowConfig) -> f32 {
    2.0 * radius / config.resolution as f32
}

/// Rotation into a light space looking along -`to_sun`; depends on the sun only,
/// so texel snapping in it stays stable while the camera turns.
fn light_rotation(to_sun: Vec3) -> Mat4 {
    let up = if to_sun.y.abs() > ZENITH_UP_SWITCH {
        Vec3::Z
    } else {
        Vec3::Y
    };
    Mat4::look_to_rh(Vec3::ZERO, -to_sun, up)
}

/// tan² of the angle between the view axis and a frustum corner ray.
fn corner_slope_squared(camera: &Camera) -> f32 {
    let tan_y = (camera.fovy.to_radians() * 0.5).tan();
    let tan_x = tan_y * camera.aspect;
    tan_x * tan_x + tan_y * tan_y
}

/// Practical split scheme: a blend of logarithmic and uniform splits.
fn practical_splits(near: f32, far: f32, count: usize) -> Vec<f32> {
    (0..=count)
        .map(|i| {
            let t = i as f32 / count as f32;
            let logarithmic = near * (far / near).powf(t);
            let uniform = near + (far - near) * t;
            PRACTICAL_SPLIT_LAMBDA * logarithmic + (1.0 - PRACTICAL_SPLIT_LAMBDA) * uniform
        })
        .collect()
}

/// Smallest sphere centred on the view axis enclosing the frustum slice
/// [near, far]. It depends only on the slice and FOV, never on the camera
/// orientation, so turning does not resize it. Returns (centre depth, radius).
fn slice_bounding_sphere(near: f32, far: f32, corner_slope_sq: f32) -> (f32, f32) {
    let center = (far + near) * (1.0 + corner_slope_sq) * 0.5;
    if center >= far {
        (far, far * corner_slope_sq.sqrt())
    } else {
        let to_far_plane = far - center;
        (
            center,
            (to_far_plane * to_far_plane + far * far * corner_slope_sq).sqrt(),
        )
    }
}

/// Reverse-Z orthographic cascade around a sphere centre snapped to the texel grid pinned at
/// `anchor`, its depth range extended towards the sun by the caster reach.
fn fit_cascade(
    center: Vec3,
    radius: f32,
    light_rotation: Mat4,
    anchor: Vec3,
    config: &ShadowConfig,
) -> Cascade {
    let texel_world = texel_world(radius, config);
    let light_center = light_rotation.transform_vector3(center - anchor);
    let snapped = Vec3::new(
        snap_to_texel(light_center.x, texel_world),
        snap_to_texel(light_center.y, texel_world),
        light_center.z,
    );
    let view = Mat4::from_translation(-snapped) * light_rotation * Mat4::from_translation(-anchor);
    let caster_reach = config.distance;
    // Near and far are swapped for reverse-Z: the side facing the sun maps to 1.
    let projection = Mat4::orthographic_rh(
        -radius,
        radius,
        -radius,
        radius,
        radius,
        -(radius + caster_reach),
    );
    let snapped_center = anchor + light_rotation.inverse().transform_vector3(snapped);

    Cascade {
        view_proj: projection * view,
        sphere: snapped_center.extend(radius),
        texel_world,
        min_texels: 0.0,
        skip_alpha: false,
        refresh: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::shadows::settings::ShadowFilter;

    const TO_SUN: Vec3 = Vec3::new(0.3, 0.9, 0.3);

    fn config(cascade_count: usize) -> ShadowConfig {
        ShadowConfig {
            resolution: 2048,
            distance: 200.0,
            cascade_count,
            split_blend: 2.0,
            filter: ShadowFilter::Grid3x3,
            bias_scale: 1.0,
            cascade_min_texels: [1.5, 3.0, 8.0, 8.0],
            skip_alpha_in_last_cascade: true,
        }
    }

    fn camera_looking(direction: Vec3) -> Camera {
        camera_at(Vec3::new(100.0, 50.0, -20.0), direction)
    }

    fn camera_at(eye: Vec3, direction: Vec3) -> Camera {
        Camera {
            eye,
            target: eye + direction,
            up: Vec3::Y,
            aspect: 16.0 / 9.0,
            fovy: 60.0,
            znear: 0.1,
            zfar: 1000.0,
        }
    }

    fn tan_half_fov(fovy_degrees: f32, aspect: f32) -> (f32, f32) {
        let tan_y = (fovy_degrees.to_radians() / 2.0).tan();
        (tan_y * aspect, tan_y)
    }

    /// Where `point` falls on the cascade's shadow map, in texels.
    fn texel_position(cascade: &Cascade, point: Vec3, resolution: u32) -> glam::Vec2 {
        let clip = cascade.view_proj.project_point3(point);
        (clip.truncate() * 0.5 + 0.5) * resolution as f32
    }

    /// How far `to` lies from `from` within a texel, ignoring whole-texel shifts of the map.
    fn sub_texel_change(from: glam::Vec2, to: glam::Vec2) -> f32 {
        let change = to - from;
        (change - change.round()).abs().max_element()
    }

    /// One frame of the sun at the vanilla timescale: about 0.0014 degrees at 60 frames a second.
    const SUN_DEGREES_PER_FRAME: f32 = 0.0014;

    #[test]
    fn a_turning_sun_moves_shadows_far_from_the_world_origin_by_a_fraction_of_a_texel() {
        let config = config(3);
        let camera = camera_at(Vec3::new(2_000.0, 50.0, -1_500.0), -Vec3::Z);
        let point = camera.eye - Vec3::Z * 3.0;
        let axis = Vec3::new(0.2, 0.3, 0.9).normalize();
        let mut anchors = GridAnchors::default();
        let mut to_sun = TO_SUN.normalize();
        let mut previous = None;
        for frame in 0..60 {
            let set = CascadeSet::fit(&camera, to_sun, &config, 3, &mut anchors);
            let position = texel_position(&set.cascades[0], point, config.resolution);
            if let Some(previous) = previous {
                let change = sub_texel_change(previous, position);
                assert!(change < 0.1, "frame {frame}: {change} texels");
            }
            previous = Some(position);
            to_sun = glam::Quat::from_axis_angle(axis, SUN_DEGREES_PER_FRAME.to_radians()) * to_sun;
        }
    }

    #[test]
    fn walking_under_a_still_sun_keeps_every_point_on_the_same_spot_of_the_texel_grid() {
        let config = config(3);
        let start = camera_looking(-Vec3::Z);
        let point = start.eye - Vec3::Z * 3.0;
        let mut anchors = GridAnchors::default();
        let first = CascadeSet::fit(&start, TO_SUN, &config, 3, &mut anchors);
        let origin = texel_position(&first.cascades[0], point, config.resolution);
        let mut eye = start.eye;
        for step in 0..2000 {
            eye += Vec3::new(0.37, 0.0, -0.11);
            let set = CascadeSet::fit(&camera_at(eye, -Vec3::Z), TO_SUN, &config, 3, &mut anchors);
            let position = texel_position(&set.cascades[0], point, config.resolution);
            let change = sub_texel_change(origin, position);
            assert!(change < 0.01, "step {step}: {change} texels");
        }
        let walked = first.cascades[0].sphere.truncate().distance(eye);
        assert!(
            walked > 3.0 * first.cascades[0].sphere.w,
            "the walk crosses many anchor moves"
        );
    }

    #[test]
    fn a_reset_pins_the_grids_on_the_next_cascade_centres() {
        let config = config(3);
        let mut anchors = GridAnchors::default();
        CascadeSet::fit(&camera_looking(-Vec3::Z), TO_SUN, &config, 3, &mut anchors);
        anchors.reset();
        let far = camera_at(Vec3::new(5_000.0, 50.0, 3_000.0), -Vec3::Z);
        let mut fresh = GridAnchors::default();
        let after_reset = CascadeSet::fit(&far, TO_SUN, &config, 3, &mut anchors);
        let from_scratch = CascadeSet::fit(&far, TO_SUN, &config, 3, &mut fresh);
        for (reset, scratch) in after_reset.cascades.iter().zip(&from_scratch.cascades) {
            assert!(reset.view_proj.abs_diff_eq(scratch.view_proj, 1e-6));
        }
    }

    #[test]
    fn practical_splits_span_near_to_far() {
        let splits = practical_splits(0.5, 400.0, 4);
        assert_eq!(splits.len(), 5);
        assert!((splits[0] - 0.5).abs() < 1e-5);
        assert!((splits[4] - 400.0).abs() < 1e-3);
    }

    #[test]
    fn practical_splits_increase_strictly() {
        let splits = practical_splits(0.1, 300.0, 4);
        assert!(
            splits.windows(2).all(|pair| pair[0] < pair[1]),
            "{splits:?}"
        );
    }

    #[test]
    fn practical_splits_blend_logarithmic_and_uniform_by_the_lambda() {
        // near 1, far 100, two cascades: at t = 0.5 logarithmic = 10, uniform = 50.5,
        // so 0.75 * 10 + 0.25 * 50.5 = 20.125.
        let splits = practical_splits(1.0, 100.0, 2);
        assert!((splits[1] - 20.125).abs() < 1e-3, "{}", splits[1]);
    }

    #[test]
    fn slice_sphere_contains_every_corner_of_a_wide_slice_and_touches_the_far_corners() {
        let (tan_x, tan_y) = tan_half_fov(60.0, 16.0 / 9.0);
        let (near, far) = (10.0, 50.0);
        let (center, radius) = slice_bounding_sphere(near, far, tan_x * tan_x + tan_y * tan_y);
        let corner_distance = |depth: f32, sx: f32, sy: f32| {
            Vec3::new(sx * depth * tan_x, sy * depth * tan_y, depth).distance(Vec3::Z * center)
        };
        for depth in [near, far] {
            for (sx, sy) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
                assert!(corner_distance(depth, sx, sy) <= radius + 1e-3);
            }
        }
        assert!((corner_distance(far, 1.0, 1.0) - radius).abs() < 1e-3);
    }

    #[test]
    fn slice_sphere_of_a_narrow_frustum_touches_near_and_far_corners() {
        let (tan_x, tan_y) = tan_half_fov(20.0, 1.0);
        let (near, far) = (10.0, 50.0);
        let (center, radius) = slice_bounding_sphere(near, far, tan_x * tan_x + tan_y * tan_y);
        let corner_distance =
            |depth: f32| Vec3::new(depth * tan_x, depth * tan_y, depth).distance(Vec3::Z * center);
        assert!((corner_distance(near) - radius).abs() < 1e-3);
        assert!((corner_distance(far) - radius).abs() < 1e-3);
        assert!(center > near && center < far);
    }

    #[test]
    fn fit_produces_the_configured_cascades_in_order_of_distance() {
        let set = CascadeSet::fit(
            &camera_looking(-Vec3::Z),
            TO_SUN,
            &config(3),
            3,
            &mut GridAnchors::default(),
        );
        assert_eq!(set.cascades.len(), 3);
        let radii: Vec<f32> = set.cascades.iter().map(|c| c.sphere.w).collect();
        assert!(radii.windows(2).all(|pair| pair[0] < pair[1]), "{radii:?}");
    }

    #[test]
    fn fit_cascades_enclose_their_frustum_slices() {
        let camera = camera_looking(-Vec3::Z);
        let config = config(3);
        let set = CascadeSet::fit(&camera, TO_SUN, &config, 3, &mut GridAnchors::default());
        let splits = practical_splits(camera.znear, config.distance, 3);
        let (tan_x, tan_y) = tan_half_fov(camera.fovy, camera.aspect);
        for (i, cascade) in set.cascades.iter().enumerate() {
            let blend = if i == 2 { 0.0 } else { config.split_blend };
            for depth in [splits[i], splits[i + 1] + blend] {
                for (sx, sy) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
                    // The camera looks down -Z with +Y up, so +X is to its right.
                    let corner =
                        camera.eye + Vec3::new(sx * depth * tan_x, sy * depth * tan_y, -depth);
                    let distance = corner.distance(cascade.sphere.truncate());
                    assert!(
                        distance <= cascade.sphere.w + cascade.texel_world,
                        "cascade {i} corner at depth {depth}: {distance} > {}",
                        cascade.sphere.w
                    );
                }
            }
        }
    }

    #[test]
    fn fit_applies_the_per_cascade_minimum_texels_and_skips_alpha_only_in_the_last() {
        let set = CascadeSet::fit(
            &camera_looking(-Vec3::Z),
            TO_SUN,
            &config(3),
            3,
            &mut GridAnchors::default(),
        );
        let min_texels: Vec<f32> = set.cascades.iter().map(|c| c.min_texels).collect();
        let skip_alpha: Vec<bool> = set.cascades.iter().map(|c| c.skip_alpha).collect();
        assert_eq!(min_texels, vec![1.5, 3.0, 8.0]);
        assert_eq!(skip_alpha, vec![false, false, true]);
    }

    #[test]
    fn fit_keeps_alpha_casters_when_the_config_says_so() {
        let mut config = config(2);
        config.skip_alpha_in_last_cascade = false;
        let set = CascadeSet::fit(
            &camera_looking(-Vec3::Z),
            TO_SUN,
            &config,
            2,
            &mut GridAnchors::default(),
        );
        assert!(set.cascades.iter().all(|c| !c.skip_alpha));
    }

    #[test]
    fn eye_centered_cascade_does_not_move_when_the_camera_turns() {
        let config = config(3);
        let ahead = CascadeSet::fit(
            &camera_looking(-Vec3::Z),
            TO_SUN,
            &config,
            2,
            &mut GridAnchors::default(),
        );
        let turned = CascadeSet::fit(
            &camera_looking(Vec3::X),
            TO_SUN,
            &config,
            2,
            &mut GridAnchors::default(),
        );
        assert!(
            ahead.cascades[2]
                .view_proj
                .abs_diff_eq(turned.cascades[2].view_proj, 1e-6)
        );
        assert!(
            !ahead.cascades[0]
                .view_proj
                .abs_diff_eq(turned.cascades[0].view_proj, 1e-3)
        );
    }

    #[test]
    fn eye_centered_cascades_are_centred_on_the_eye() {
        let camera = camera_looking(-Vec3::Z);
        let set = CascadeSet::fit(&camera, TO_SUN, &config(3), 2, &mut GridAnchors::default());
        let last = set.cascades[2];
        assert!(last.sphere.truncate().distance(camera.eye) <= last.texel_world);
        assert!(set.cascades[1].sphere.truncate().distance(camera.eye) > last.texel_world);
    }

    #[test]
    fn views_leave_out_cascades_that_keep_their_map() {
        let mut set = CascadeSet::fit(
            &camera_looking(-Vec3::Z),
            TO_SUN,
            &config(3),
            3,
            &mut GridAnchors::default(),
        );
        set.cascades[1].refresh = false;
        let views = set.views();
        assert_eq!(
            views.iter().map(Option::is_some).collect::<Vec<_>>(),
            vec![true, false, true]
        );
    }

    #[test]
    fn light_rotation_looks_along_minus_the_sun_direction() {
        let rotation = light_rotation(TO_SUN.normalize());
        let forward = rotation.transform_vector3(-TO_SUN.normalize());
        assert!(forward.abs_diff_eq(-Vec3::Z, 1e-5), "{forward}");
    }

    #[test]
    fn light_rotation_stays_finite_with_the_sun_at_the_zenith() {
        for to_sun in [Vec3::Y, -Vec3::Y] {
            let rotation = light_rotation(to_sun);
            assert!(rotation.is_finite(), "{to_sun}");
            assert!(
                rotation
                    .transform_vector3(-to_sun)
                    .abs_diff_eq(-Vec3::Z, 1e-5)
            );
        }
    }

    fn fitted_at_light_space(light_x: f32, light_y: f32, radius: f32) -> Cascade {
        let rotation = light_rotation(TO_SUN.normalize());
        let center = rotation
            .inverse()
            .transform_point3(Vec3::new(light_x, light_y, -30.0));
        fit_cascade(center, radius, rotation, Vec3::ZERO, &config(3))
    }

    #[test]
    fn fit_cascade_snaps_to_whole_texels_so_sub_texel_motion_changes_nothing() {
        let radius = 64.0;
        let texel = 2.0 * radius / 2048.0;
        let base = fitted_at_light_space(10.1 * texel, 20.2 * texel, radius);
        let nudged = fitted_at_light_space(10.3 * texel, 20.4 * texel, radius);
        assert!(base.view_proj.abs_diff_eq(nudged.view_proj, 1e-4));
        assert!(base.sphere.abs_diff_eq(nudged.sphere, 1e-4));
    }

    #[test]
    fn fit_cascade_moves_by_whole_texels() {
        let radius = 64.0;
        let texel = 2.0 * radius / 2048.0;
        let base = fitted_at_light_space(10.1 * texel, 20.2 * texel, radius);
        let one_texel = fitted_at_light_space(11.1 * texel, 20.2 * texel, radius);
        let moved = base.sphere.truncate().distance(one_texel.sphere.truncate());
        assert!((moved - texel).abs() < 1e-4, "{moved} vs {texel}");
    }

    #[test]
    fn fit_cascade_maps_the_sphere_edges_to_the_clip_square_and_depth_to_reverse_z() {
        let radius = 64.0;
        let cascade = fitted_at_light_space(0.0, 0.0, radius);
        let to_sun = TO_SUN.normalize();
        let center = cascade.sphere.truncate();
        let clip = |point: Vec3| cascade.view_proj.project_point3(point);

        let sun_side = clip(center + to_sun * (radius + 200.0));
        let far_side = clip(center - to_sun * radius);
        assert!((sun_side.z - 1.0).abs() < 1e-4, "{sun_side}");
        assert!(far_side.z.abs() < 1e-4, "{far_side}");

        let rotation = light_rotation(to_sun);
        let light_x = rotation.inverse().transform_vector3(Vec3::X);
        assert!((clip(center + light_x * radius).x - 1.0).abs() < 1e-4);
        assert!((clip(center - light_x * radius).x + 1.0).abs() < 1e-4);
        assert!(clip(center).truncate().abs_diff_eq(glam::Vec2::ZERO, 1e-4));
    }
}
