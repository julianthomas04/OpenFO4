use glam::{Vec3, Vec4};

use super::source::LampCandidate;
use crate::rendering::local_lights::LightFade;
use crate::rendering::placed_light::{LIGHT_FLAG_REF_NEVER_FADES, SKIPPED_FLAGS};

/// Keeps the coverage term finite while the camera is inside a lamp's range.
const MIN_SURFACE_DISTANCE_FRACTION: f32 = 0.1;

/// What decides whether and how much a lamp's shadow can be seen this frame.
pub struct SelectionView<'a> {
    pub eye: Vec3,
    /// Camera clip planes; points inside satisfy `dot(n, p) + w >= 0`.
    pub planes: &'a [Vec4],
    pub light_fade: LightFade,
    /// Receiver distance from the camera at which lamp shadows are gone.
    pub shadow_fade_end: f32,
}

/// `None` when the lamp's shadow cannot show: a light the cull skips, a
/// light faded out by distance, a range sphere outside the camera frustum or
/// wholly beyond the shadow fade. Otherwise the brightness times the squared
/// angular size of the range sphere, clamped once the camera is close.
pub fn importance(candidate: &LampCandidate, view: &SelectionView) -> Option<f32> {
    let light = &candidate.light;
    if light.flags & SKIPPED_FLAGS != 0 {
        return None;
    }
    let position = Vec3::from(light.position);
    let distance = view.eye.distance(position);
    let fades = light.flags & LIGHT_FLAG_REF_NEVER_FADES == 0;
    if fades && distance >= view.light_fade.end() {
        return None;
    }
    if distance - light.radius >= view.shadow_fade_end {
        return None;
    }
    let outside = view
        .planes
        .iter()
        .any(|plane| plane.truncate().dot(position) + plane.w < -light.radius);
    if outside {
        return None;
    }
    let brightness = Vec3::from(light.color).abs().max_element();
    let surface_distance =
        (distance - light.radius).max(light.radius * MIN_SURFACE_DISTANCE_FRACTION);
    let coverage = (light.radius / surface_distance).powi(2).min(1.0);
    Some(brightness * coverage)
}

#[cfg(test)]
mod tests {
    use bytemuck::Zeroable;

    use super::*;
    use crate::rendering::placed_light::{
        GpuPlacedLight, LIGHT_FLAG_AMBIENT_ONLY, LIGHT_FLAG_OFF_BY_DEFAULT,
    };
    use crate::rendering::shadows::lamps::source::{LampKey, LampShadowSpec, ShadowType};

    const FADE: LightFade = LightFade {
        start: 100.0,
        range: 50.0,
    };

    fn lamp(position: [f32; 3], radius: f32, color: [f32; 3], flags: u32) -> LampCandidate {
        let mut light = GpuPlacedLight::zeroed();
        light.position = position;
        light.radius = radius;
        light.color = color;
        light.flags = flags;
        LampCandidate {
            key: LampKey {
                residency: 1,
                light_index: 0,
            },
            light,
            spec: LampShadowSpec {
                shadow_type: ShadowType::Omni,
                near_clip: 1.0,
                depth_bias: None,
            },
            slab_index: 0,
        }
    }

    fn view(planes: &[Vec4], shadow_fade_end: f32) -> SelectionView<'_> {
        SelectionView {
            eye: Vec3::ZERO,
            planes,
            light_fade: FADE,
            shadow_fade_end,
        }
    }

    #[test]
    fn importance_is_brightness_times_the_squared_angular_size() {
        // Surface at 40 - 10 = 30 from the eye: coverage (10 / 30)^2 = 1/9; brightest channel 2.
        let candidate = lamp([40.0, 0.0, 0.0], 10.0, [2.0, 0.5, 1.0], 0);
        let value = importance(&candidate, &view(&[], 1000.0)).unwrap();
        assert!((value - 2.0 / 9.0).abs() < 1e-5, "{value}");
    }

    #[test]
    fn importance_uses_the_magnitude_of_a_negative_colour() {
        let candidate = lamp([40.0, 0.0, 0.0], 10.0, [-3.0, 0.0, 0.0], 0);
        let value = importance(&candidate, &view(&[], 1000.0)).unwrap();
        assert!((value - 3.0 / 9.0).abs() < 1e-5, "{value}");
    }

    #[test]
    fn coverage_is_capped_at_one_next_to_the_lamp() {
        let candidate = lamp([12.0, 0.0, 0.0], 10.0, [1.0, 1.0, 1.0], 0);
        let value = importance(&candidate, &view(&[], 1000.0)).unwrap();
        assert!((value - 1.0).abs() < 1e-6, "{value}");
    }

    #[test]
    fn importance_stays_finite_with_the_camera_inside_the_range_sphere() {
        let candidate = lamp([0.0, 0.0, 0.0], 10.0, [1.0, 1.0, 1.0], 0);
        let value = importance(&candidate, &view(&[], 1000.0)).unwrap();
        assert!(value.is_finite() && value > 0.0);
    }

    #[test]
    fn lamps_the_cull_skips_are_never_selected() {
        for flags in [LIGHT_FLAG_OFF_BY_DEFAULT, LIGHT_FLAG_AMBIENT_ONLY] {
            let candidate = lamp([40.0, 0.0, 0.0], 10.0, [1.0, 1.0, 1.0], flags);
            assert_eq!(
                importance(&candidate, &view(&[], 1000.0)),
                None,
                "{flags:#x}"
            );
        }
    }

    #[test]
    fn a_lamp_faded_out_by_distance_has_no_shadow() {
        let at_fade_end = lamp([150.0, 0.0, 0.0], 10.0, [1.0, 1.0, 1.0], 0);
        let before = lamp([149.0, 0.0, 0.0], 10.0, [1.0, 1.0, 1.0], 0);
        assert_eq!(importance(&at_fade_end, &view(&[], 1000.0)), None);
        assert!(importance(&before, &view(&[], 1000.0)).is_some());
    }

    #[test]
    fn a_never_fading_lamp_ignores_the_light_fade() {
        let candidate = lamp(
            [500.0, 0.0, 0.0],
            10.0,
            [1.0, 1.0, 1.0],
            LIGHT_FLAG_REF_NEVER_FADES,
        );
        assert!(importance(&candidate, &view(&[], 1000.0)).is_some());
    }

    #[test]
    fn a_lamp_wholly_beyond_the_shadow_fade_has_no_shadow() {
        // Range sphere starts at 50 - 10 = 40 from the eye.
        let candidate = lamp([50.0, 0.0, 0.0], 10.0, [1.0, 1.0, 1.0], 0);
        assert_eq!(importance(&candidate, &view(&[], 40.0)), None);
        assert!(importance(&candidate, &view(&[], 40.1)).is_some());
    }

    #[test]
    fn a_range_sphere_outside_a_camera_plane_has_no_shadow() {
        // Plane x >= 0 keeps the positive half space.
        let planes = [Vec4::new(1.0, 0.0, 0.0, 0.0)];
        let outside = lamp([-10.0, 0.0, 0.0], 5.0, [1.0, 1.0, 1.0], 0);
        let straddling = lamp([-10.0, 0.0, 0.0], 15.0, [1.0, 1.0, 1.0], 0);
        assert_eq!(importance(&outside, &view(&planes, 1000.0)), None);
        assert!(importance(&straddling, &view(&planes, 1000.0)).is_some());
    }
}
