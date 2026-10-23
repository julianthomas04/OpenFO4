use glam::Vec3;

use winit::keyboard::KeyCode;

pub struct Camera {
    pub eye: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub aspect: f32,
    pub fovy: f32,
    pub znear: f32,
    pub zfar: f32,
}

impl Camera {
    pub fn aspect_of(width: u32, height: u32) -> f32 {
        width.max(1) as f32 / height.max(1) as f32
    }

    pub fn fit_viewport(&mut self, width: u32, height: u32) {
        self.aspect = Self::aspect_of(width, height);
    }

    pub fn view_matrix(&self) -> glam::Mat4 {
        glam::Mat4::look_at_rh(self.eye, self.target, self.up)
    }

    pub fn projection_matrix(&self) -> glam::Mat4 {
        glam::Mat4::perspective_infinite_reverse_rh(self.fovy.to_radians(), self.aspect, self.znear)
    }

    pub fn build_view_projection_matrix(&self) -> glam::Mat4 {
        self.projection_matrix() * self.view_matrix()
    }

    /// Unit view direction; view-space depth is the distance along it.
    pub fn forward(&self) -> Vec3 {
        (self.target - self.eye).normalize()
    }
}

pub struct CameraController {
    pub speed: f32,
    sensitivity: f32,
    yaw: f32,   // horizontal rotation (radians)
    pitch: f32, // vertical rotation (radians)
    forward: bool,
    backward: bool,
    left: bool,
    right: bool,
    up: bool,
    down: bool,
    sprint: bool,
}

impl CameraController {
    pub fn new(speed: f32, sensitivity: f32) -> Self {
        Self {
            speed,
            sensitivity,
            yaw: -std::f32::consts::FRAC_PI_2, // face -Z
            pitch: 0.0,
            forward: false,
            backward: false,
            left: false,
            right: false,
            up: false,
            down: false,
            sprint: false,
        }
    }

    pub fn handle_key(&mut self, code: KeyCode, is_pressed: bool) -> bool {
        match code {
            KeyCode::KeyW => {
                self.forward = is_pressed;
                true
            }
            KeyCode::KeyS => {
                self.backward = is_pressed;
                true
            }
            KeyCode::KeyA => {
                self.left = is_pressed;
                true
            }
            KeyCode::KeyD => {
                self.right = is_pressed;
                true
            }
            KeyCode::Space => {
                self.up = is_pressed;
                true
            }
            KeyCode::ShiftLeft => {
                self.down = is_pressed;
                true
            }
            KeyCode::ControlLeft => {
                self.sprint = is_pressed;
                true
            }
            _ => false,
        }
    }

    pub fn handle_mouse(&mut self, dx: f64, dy: f64) {
        self.yaw += dx as f32 * self.sensitivity;
        self.pitch -= dy as f32 * self.sensitivity;
        // Clamp pitch to prevent flipping
        self.pitch = self.pitch.clamp(
            -std::f32::consts::FRAC_PI_2 + 0.01,
            std::f32::consts::FRAC_PI_2 - 0.01,
        );
    }

    /// Dev override for the initial look direction (`OPENFO4_SPAWN_LOOK`). Yaw/pitch
    /// in radians; pitch is clamped to the same near-vertical bound as mouse look so
    /// "straight down" resolves to ~-89.4° rather than the degenerate gimbal pole.
    /// Non-finite inputs are rejected (a NaN/inf yaw would poison the view matrix
    /// irrecoverably, since mouse-look only ever adds to yaw).
    pub fn set_look(&mut self, yaw: f32, pitch: f32) {
        if !yaw.is_finite() || !pitch.is_finite() {
            return;
        }
        self.yaw = yaw;
        self.pitch = pitch.clamp(
            -std::f32::consts::FRAC_PI_2 + 0.01,
            std::f32::consts::FRAC_PI_2 - 0.01,
        );
    }

    pub fn yaw(&self) -> f32 {
        self.yaw
    }

    /// Unit look direction from yaw/pitch — shared by fly-mode movement and
    /// the walk-mode camera target.
    pub fn forward(&self) -> Vec3 {
        Vec3::new(
            self.yaw.cos() * self.pitch.cos(),
            self.pitch.sin(),
            self.yaw.sin() * self.pitch.cos(),
        )
        .normalize()
    }

    pub fn update_camera(&self, camera: &mut super::Camera, dt: f32) {
        let forward = self.forward();

        let right = forward.cross(Vec3::Y).normalize();

        let mut speed = self.speed;
        if self.sprint {
            speed *= 5.0;
        }

        let mut velocity = Vec3::ZERO;
        if self.forward {
            velocity += forward;
        }
        if self.backward {
            velocity -= forward;
        }
        if self.right {
            velocity += right;
        }
        if self.left {
            velocity -= right;
        }
        if self.up {
            velocity += Vec3::Y;
        }
        if self.down {
            velocity -= Vec3::Y;
        }

        if velocity.length_squared() > 0.0 {
            velocity = velocity.normalize() * speed * dt;
        }

        camera.eye += velocity;
        camera.target = camera.eye + forward;
    }
}

/// Parses `OPENFO4_SPAWN_LOOK` (`yaw_deg,pitch_deg`) into radians. Exactly two finite
/// numbers are required: each component is parsed (so `10,x,20` is rejected rather than
/// reduced to `10,20`) and `inf`/`nan` are refused since a non-finite angle would
/// poison the view matrix with no way to recover via mouse-look.
pub fn parse_spawn_look(raw: &str) -> Option<(f32, f32)> {
    let (yaw, pitch) = raw.split_once(',')?;
    let yaw: f32 = yaw.trim().parse().ok()?;
    let pitch: f32 = pitch.trim().parse().ok()?;
    (yaw.is_finite() && pitch.is_finite()).then_some((yaw.to_radians(), pitch.to_radians()))
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact sentinels and copied values"
)]
mod tests {
    use super::*;

    fn camera() -> Camera {
        Camera {
            eye: Vec3::ZERO,
            target: Vec3::new(0.0, 0.0, -1.0),
            up: Vec3::Y,
            aspect: 1.0,
            fovy: 60.0,
            znear: 0.1,
            zfar: 1000.0,
        }
    }

    #[test]
    fn fitting_the_viewport_sets_the_aspect_from_its_size() {
        let mut camera = camera();
        camera.fit_viewport(1920, 1080);
        assert!((camera.aspect - 16.0 / 9.0).abs() < 1e-6);
        camera.fit_viewport(800, 800);
        assert_eq!(camera.aspect, 1.0);
    }

    #[test]
    fn a_degenerate_viewport_never_gives_a_non_finite_aspect() {
        assert!(Camera::aspect_of(0, 0).is_finite());
        assert!(Camera::aspect_of(640, 0).is_finite());
        assert_eq!(Camera::aspect_of(0, 480), 1.0 / 480.0);
    }

    fn controller() -> CameraController {
        CameraController::new(10.0, 0.01)
    }

    #[test]
    fn camera_forward_is_the_unit_direction_to_the_target() {
        let mut camera = camera();
        camera.target = Vec3::new(3.0, 0.0, -4.0);
        assert!(
            camera
                .forward()
                .abs_diff_eq(Vec3::new(0.6, 0.0, -0.8), 1e-6)
        );
    }

    #[test]
    fn view_matrix_puts_the_eye_at_the_origin_looking_down_minus_z() {
        let mut camera = camera();
        camera.eye = Vec3::new(5.0, 2.0, 1.0);
        camera.target = camera.eye + Vec3::X;
        let ahead = camera
            .view_matrix()
            .transform_point3(camera.eye + Vec3::X * 7.0);
        assert!(
            ahead.abs_diff_eq(Vec3::new(0.0, 0.0, -7.0), 1e-5),
            "{ahead}"
        );
    }

    #[test]
    fn projection_is_reverse_z_so_near_geometry_has_the_larger_depth() {
        let camera = camera();
        let clip = camera.build_view_projection_matrix();
        let near = clip.project_point3(Vec3::new(0.0, 0.0, -0.2));
        let far = clip.project_point3(Vec3::new(0.0, 0.0, -500.0));
        assert!(near.z > far.z && far.z > 0.0, "{near} {far}");
    }

    #[test]
    fn set_look_points_the_controller_along_yaw_and_pitch() {
        let mut controller = controller();
        controller.set_look(0.0, 0.0);
        assert!(controller.forward().abs_diff_eq(Vec3::X, 1e-6));
        controller.set_look(std::f32::consts::FRAC_PI_2, 0.0);
        assert!(controller.forward().abs_diff_eq(Vec3::Z, 1e-6));
    }

    #[test]
    fn set_look_rejects_non_finite_angles_and_keeps_the_previous_look() {
        let mut controller = controller();
        controller.set_look(0.0, 0.0);
        controller.set_look(f32::NAN, 0.0);
        controller.set_look(0.0, f32::INFINITY);
        assert!(controller.forward().abs_diff_eq(Vec3::X, 1e-6));
        assert_eq!(controller.yaw(), 0.0);
    }

    #[test]
    fn set_look_clamps_pitch_short_of_the_pole() {
        let mut controller = controller();
        controller.set_look(0.0, -std::f32::consts::FRAC_PI_2);
        let forward = controller.forward();
        assert!(forward.y < -0.99 && forward.y > -1.0, "{forward}");
        assert!(forward.is_finite());
    }

    #[test]
    fn mouse_look_clamps_pitch_both_ways() {
        let mut controller = controller();
        controller.handle_mouse(0.0, -1e9);
        assert!(controller.forward().y < 1.0 && controller.forward().y > 0.99);
        controller.handle_mouse(0.0, 1e9);
        assert!(controller.forward().y > -1.0 && controller.forward().y < -0.99);
    }

    #[test]
    fn mouse_motion_turns_right_for_positive_dx_and_up_for_negative_dy() {
        let mut controller = controller();
        let before = controller.yaw();
        controller.handle_mouse(100.0, -100.0);
        assert!(controller.yaw() > before);
        assert!(controller.forward().y > 0.0);
    }

    #[test]
    fn holding_forward_moves_the_eye_along_the_look_direction_by_speed_times_dt() {
        let mut controller = controller();
        let mut camera = camera();
        controller.handle_key(KeyCode::KeyW, true);
        controller.update_camera(&mut camera, 0.5);
        assert!(
            camera.eye.abs_diff_eq(Vec3::new(0.0, 0.0, -5.0), 1e-4),
            "{}",
            camera.eye
        );
        assert!(
            camera
                .target
                .abs_diff_eq(camera.eye + controller.forward(), 1e-6)
        );
    }

    #[test]
    fn diagonal_movement_is_not_faster_than_straight_movement() {
        let mut controller = controller();
        let mut camera = camera();
        controller.handle_key(KeyCode::KeyW, true);
        controller.handle_key(KeyCode::KeyD, true);
        controller.update_camera(&mut camera, 1.0);
        assert!(
            (camera.eye.length() - 10.0).abs() < 1e-4,
            "{}",
            camera.eye.length()
        );
    }

    #[test]
    fn opposite_keys_cancel_out() {
        let mut controller = controller();
        let mut camera = camera();
        controller.handle_key(KeyCode::KeyW, true);
        controller.handle_key(KeyCode::KeyS, true);
        controller.update_camera(&mut camera, 1.0);
        assert_eq!(camera.eye, Vec3::ZERO);
    }

    #[test]
    fn sprint_multiplies_the_speed_by_five() {
        let mut controller = controller();
        let mut camera = camera();
        controller.handle_key(KeyCode::KeyW, true);
        controller.handle_key(KeyCode::ControlLeft, true);
        controller.update_camera(&mut camera, 1.0);
        assert!((camera.eye.length() - 50.0).abs() < 1e-3);
    }

    #[test]
    fn releasing_a_key_stops_the_motion() {
        let mut controller = controller();
        let mut camera = camera();
        controller.handle_key(KeyCode::KeyW, true);
        controller.handle_key(KeyCode::KeyW, false);
        controller.update_camera(&mut camera, 1.0);
        assert_eq!(camera.eye, Vec3::ZERO);
    }

    #[test]
    fn vertical_keys_move_along_world_up() {
        let mut controller = controller();
        let mut camera = camera();
        controller.handle_key(KeyCode::Space, true);
        controller.update_camera(&mut camera, 1.0);
        assert!(camera.eye.abs_diff_eq(Vec3::new(0.0, 10.0, 0.0), 1e-4));
    }

    #[test]
    fn spawn_look_parses_degrees_into_radians() {
        let (yaw, pitch) = parse_spawn_look(" 90 , -45 ").unwrap();
        assert!((yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        assert!((pitch + std::f32::consts::FRAC_PI_4).abs() < 1e-6);
    }

    #[test]
    fn spawn_look_rejects_malformed_and_non_finite_values() {
        for raw in ["", "10", "10,x", "10,20,30", "10,x,20", "inf,0", "0,NaN"] {
            assert!(parse_spawn_look(raw).is_none(), "{raw:?} must be rejected");
        }
    }
}
