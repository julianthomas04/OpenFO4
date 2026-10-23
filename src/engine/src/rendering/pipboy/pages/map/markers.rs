//! Pure map geometry for markers and the player: the projection between engine positions and
//! the map's texels, the player arrow's heading and pulse.

use egui::Vec2 as EVec2;
use glam::{Vec2, Vec3};

use crate::rendering::pipboy::shapes::alpha_at_frame;
use crate::sim::pipboy::WorldMapProjection;
use crate::ui_layout::{
    ModuleKeys, PlacementKey, StageKey, UiLayout, UiLayoutError, key_check_fields,
};

/// The texel position of an engine position on a texture of `texture` texels.
pub fn engine_to_map(projection: &WorldMapProjection, texture: EVec2, position: Vec3) -> EVec2 {
    let uv = projection.engine_to_uv(position);
    EVec2::new(uv.x * texture.x, uv.y * texture.y)
}

/// The engine position (at height `y`) of a texel position on a texture of `texture` texels.
pub fn map_to_engine(projection: &WorldMapProjection, texture: EVec2, map: EVec2, y: f32) -> Vec3 {
    let uv = Vec2::new(map.x / texture.x, map.y / texture.y);
    Vec3::new(
        (uv.x - projection.origin_u) / projection.u_per_engine_x,
        y,
        (uv.y - projection.origin_v) / projection.v_per_engine_z,
    )
}

/// Clockwise rotation from "up" of an arrow that points where a camera of `yaw` looks, on a map
/// whose up is north (-Z).
pub fn arrow_rotation(yaw: f32) -> f32 {
    let (dy, dx) = yaw.sin_cos();
    dx.atan2(-dy)
}

const MOVIE: &str = "Pipboy_MapPage";

struct PulseKeys {
    stage: StageKey,
    arrow: PlacementKey,
}

static PULSE: PulseKeys = PulseKeys {
    stage: StageKey::of(MOVIE),
    arrow: PlacementKey::new(MOVIE, "[PlayerLocMarker]/@1"),
};

key_check_fields!(PulseKeys { stage, arrow });

fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    PlayerPulse::resolve(layout).map(drop)
}

/// The keys of the player arrow's pulse.
pub static PULSE_KEYS: ModuleKeys = ModuleKeys {
    keys: &[&PULSE],
    resolve: validate,
};

/// The player arrow's opacity on each frame of the marker clip's timeline, which loops at the
/// movie's frame rate.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerPulse {
    alphas: Vec<f32>,
    fps: f32,
}

impl PlayerPulse {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let arrow = layout.get(&PULSE.arrow)?;
        let fps = layout.get(&PULSE.stage)?.fps;
        let [first, last] = arrow.frames;
        let track = arrow.track();
        if track.is_empty() || first > last || !(fps.is_finite() && fps > 0.0) {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                PULSE.arrow.path,
                "the player marker has no animation",
            ));
        }
        let alphas = (first..=last)
            .map(|frame| alpha_at_frame(track, &arrow.local.color, frame))
            .collect();
        Ok(Self { alphas, fps })
    }

    /// The arrow's opacity `seconds` into its animation.
    pub fn alpha(&self, seconds: f64) -> f32 {
        let frame = (seconds.max(0.0) * f64::from(self.fps)) as usize % self.alphas.len();
        self.alphas[frame]
    }

    #[cfg(test)]
    pub fn frame_count(&self) -> usize {
        self.alphas.len()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::ui_layout::test_support::{assert_close, instance, synthesized_layout};

    const PROJECTION: WorldMapProjection = WorldMapProjection {
        origin_u: 0.5,
        origin_v: 0.25,
        u_per_engine_x: 1.0e-4,
        v_per_engine_z: 2.0e-4,
    };
    const TEXTURE: EVec2 = EVec2::new(400.0, 200.0);

    #[test]
    fn a_position_projects_to_texels_scaled_by_the_texture_size() {
        let map = engine_to_map(&PROJECTION, TEXTURE, Vec3::new(1000.0, 7.0, 500.0));
        assert!((map.x - 0.6 * 400.0).abs() < 1e-3);
        assert!((map.y - 0.35 * 200.0).abs() < 1e-3);
    }

    #[test]
    fn engine_and_map_positions_round_trip() {
        let position = Vec3::new(-1234.0, 12.0, 987.0);
        let map = engine_to_map(&PROJECTION, TEXTURE, position);
        let back = map_to_engine(&PROJECTION, TEXTURE, map, 12.0);
        assert!((back - position).abs().max_element() < 0.05);
    }

    #[test]
    fn the_arrow_points_up_for_north_and_right_for_east() {
        let north = -std::f32::consts::FRAC_PI_2;
        assert!(arrow_rotation(north).abs() < 1e-5);
        assert!((arrow_rotation(0.0) - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!(
            (arrow_rotation(std::f32::consts::FRAC_PI_2).abs() - std::f32::consts::PI).abs() < 1e-5
        );
    }

    /// The fixture movies run at this many frames per second.
    const FIXTURE_FPS: f64 = 30.0;

    /// The pulse in the middle of the zero-based `frame`.
    fn alpha_in_frame(pulse: &PlayerPulse, frame: f64) -> f32 {
        pulse.alpha((frame + 0.5) / FIXTURE_FPS)
    }

    fn pulse_layout(alphas: &[f32]) -> UiLayout {
        let keys = alphas
            .iter()
            .zip(1_u32..)
            .map(|(alpha, frame)| {
                json!({"frame": frame, "matrix": [1, 0, 0, 1, 0, 0],
                       "color": {"mult": [1, 1, 1, alpha], "add": [0, 0, 0, 0]},
                       "visible": true})
            })
            .collect::<Vec<_>>();
        let mut arrow = instance(1, "shape", 0.0, 0.0);
        arrow["frames"] = json!([1, alphas.len()]);
        arrow["track"] = json!(keys);
        synthesized_layout(
            &[&PULSE_KEYS],
            &[(MOVIE, json!({"instances": {"[PlayerLocMarker]/@1": arrow}}))],
        )
    }

    #[test]
    fn the_pulse_follows_the_clip_track_at_the_movie_frame_rate() {
        let pulse = PlayerPulse::resolve(&pulse_layout(&[0.9, 0.5, 0.1, 0.5])).unwrap();

        assert_eq!(pulse.frame_count(), 4);
        assert_close(alpha_in_frame(&pulse, 0.0), 0.9);
        assert_close(alpha_in_frame(&pulse, 1.0), 0.5);
        assert_close(alpha_in_frame(&pulse, 2.0), 0.1);
    }

    #[test]
    fn the_pulse_loops_over_its_frames() {
        let pulse = PlayerPulse::resolve(&pulse_layout(&[0.9, 0.5, 0.1, 0.5])).unwrap();

        assert_close(alpha_in_frame(&pulse, 4.0), alpha_in_frame(&pulse, 0.0));
        assert_close(alpha_in_frame(&pulse, 6.0), alpha_in_frame(&pulse, 2.0));
    }

    #[test]
    fn a_marker_without_a_track_is_a_layout_error() {
        let layout = synthesized_layout(&[&PULSE_KEYS], &[]);

        assert!(PlayerPulse::resolve(&layout).is_err());
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the installer's layout documents"]
    fn the_installed_marker_pulses_between_full_and_a_lower_opacity() {
        let installed = crate::ui_layout::test_support::installed_layout();
        let pulse = PlayerPulse::resolve(&installed).unwrap();
        let alphas: Vec<f32> = (0..pulse.frame_count())
            .map(|frame| pulse.alpha(frame as f64 / f64::from(pulse.fps)))
            .collect();
        assert!(pulse.frame_count() > 1);
        assert!(alphas.iter().any(|a| *a < 1.0) && alphas.iter().all(|a| *a > 0.0));
    }
}
