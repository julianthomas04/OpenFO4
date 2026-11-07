//! Carrying a loose dynamic body in front of the camera, matched to measurements of vanilla
//! Fallout 4. The rules here are pure; `physics::carried` applies them to the rapier body.
//! Distances are measured in Fallout 4 units and converted once to engine units.

use glam::{Quat, Vec3};

use super::commands::look_direction;
use crate::bethesda_space::UNITS_TO_ENGINE;

/// Havok mass above which a grab is refused. Measured: mass 100 lifts, 120 and 150 are refused.
pub(super) const GRAB_MASS_LIMIT: f32 = 110.0;
/// Havok mass from which carrying forces walking speed. Measured: 70 runs, 80 walks.
pub(super) const WALK_MASS_THRESHOLD: f32 = 75.0;
/// How much closer than at grab time the body is held. Measured: about 9 FO4 units.
const HOLD_INSET: f32 = 9.0 * UNITS_TO_ENGINE;
/// Separation from the hold point that strains the grip. Measured: between 150 and 200 FO4 units.
const BREAK_SEPARATION: f32 = 175.0 * UNITS_TO_ENGINE;
/// How long the strain must last before the body is torn away. Measured: between 0.6 and 1.1 s.
const BREAK_GRACE_SECONDS: f32 = 0.8;
/// How fast the carried body keeps pressing into an obstacle between it and its hold point:
/// enough to stay in contact, too little to pound it into contact chatter. An engine
/// tolerance, not a vanilla value.
const OBSTACLE_PRESS_SPEED: f32 = 1.0;
/// Spin rate while a mouse button is held. Measured: about 172 deg/s for both buttons.
const SPIN_RATE: f32 = 172.0 * std::f32::consts::PI / 180.0;
/// Throw speed of a tap. Measured: least-squares intercept 220 FO4 units/s.
const THROW_BASE_SPEED: f32 = 220.0 * UNITS_TO_ENGINE;
/// Throw speed gained per second of charge. Measured: least-squares slope 783 FO4 units/s².
const THROW_SPEED_PER_SECOND: f32 = 783.0 * UNITS_TO_ENGINE;
/// Fastest throw, reached from about 1.6 s of charge. Measured: plateau about 1470 FO4 units/s.
pub(super) const THROW_MAX_SPEED: f32 = 1470.0 * UNITS_TO_ENGINE;

/// Why a carried body was let go.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ReleaseReason {
    Dropped,
    Thrown,
    /// The body stayed too far from the hold point for too long.
    TornAway,
    /// The cell owning the body was unloaded.
    CellUnloaded,
    /// The player was placed somewhere else (spawn, teleport, load).
    Teleported,
    /// The Pip-Boy opened.
    MenuOpened,
    /// The body left the world without its cell being unloaded (a physics quarantine).
    BodyLost,
    /// The hold point could not be computed from the eye and the view (non-finite input).
    InvalidHold,
}

/// Why a grab was refused.
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum GrabRefusal {
    AlreadyCarrying,
    /// The entity is not a loose body simulating inside the physics radius.
    NotDynamic,
    TooHeavy {
        mass: f32,
    },
    /// The player has not been placed in the world yet.
    PlayerNotPlaced,
    /// The simulation is not stepping (Pip-Boy open or a load holding it), so the body
    /// could not be held.
    SimulationPaused,
}

/// The body being carried, as the render side sees it.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct CarriedInfo {
    pub entity: super::EntityId,
    pub mass: f32,
}

/// The first-person camera's look direction, in the `CameraController` convention.
#[derive(Copy, Clone, PartialEq, Debug)]
pub(super) struct View {
    pub yaw: f32,
    pub pitch: f32,
}

impl View {
    pub fn forward(self) -> Vec3 {
        look_direction(self.yaw, self.pitch)
    }

    pub fn right(self) -> Vec3 {
        self.rotation() * Vec3::Z
    }

    /// Maps the camera frame (x forward along [`Self::forward`], y up, z right) onto the world.
    pub fn rotation(self) -> Quat {
        Quat::from_rotation_y(-self.yaw) * Quat::from_rotation_z(self.pitch)
    }
}

pub(super) fn can_grab(mass: f32) -> bool {
    mass <= GRAB_MASS_LIMIT
}

pub(super) fn forces_walk(mass: f32) -> bool {
    mass >= WALK_MASS_THRESHOLD
}

/// Eye-to-body distance kept while carrying, from the distance at grab time.
pub(super) fn hold_distance(grab_distance: f32) -> f32 {
    (grab_distance - HOLD_INSET).max(0.0)
}

/// Where the carried body's origin is held.
pub(super) fn carry_target(eye: Vec3, view: View, hold_distance: f32) -> Vec3 {
    eye + view.forward() * hold_distance
}

/// The body-local axis, signed to point along the camera's right, that lies closest to it:
/// the axis the mouse buttons spin the body about. A provisional rule, not a measured one:
/// it fits the upright toaster, a teddy lying on its back may contradict it, and which axis
/// vanilla picks is still open (`physics-grab-observations.md`).
pub(super) fn spin_axis(body_rotation: Quat, camera_right: Vec3) -> Vec3 {
    let local_right = body_rotation.inverse() * camera_right;
    let axis = [Vec3::X, Vec3::Y, Vec3::Z]
        .into_iter()
        .max_by(|a, b| {
            a.dot(local_right)
                .abs()
                .total_cmp(&b.dot(local_right).abs())
        })
        .unwrap_or(Vec3::X);
    axis * axis.dot(local_right).signum()
}

/// Body-local rotation of one step of spin. `spin` +1 (left button) tips the top of the body
/// away from the player, -1 (right button) towards; any other value spins by its sign.
pub(super) fn spin_step(axis: Vec3, spin: i8, dt: f32) -> Quat {
    Quat::from_axis_angle(axis, -f32::from(spin.signum()) * SPIN_RATE * dt)
}

/// Linear velocity that brings `current` onto `target` within one step of `dt`. A dead-beat
/// proportional servo: the closed loop has its pole at zero, so it never overshoots.
pub(super) fn servo_linvel(current: Vec3, target: Vec3, dt: f32) -> Vec3 {
    (target - current) / dt
}

/// `servo` with its push into each of `obstacles` (unit normals pointing out of the body into
/// what it touches and cannot move) cut to `OBSTACLE_PRESS_SPEED`. The body keeps following
/// its hold point along the free directions and rests against what blocks it instead of being
/// driven into it at full speed every step, which makes the contact chatter.
pub(super) fn limit_push(servo: Vec3, obstacles: &[Vec3]) -> Vec3 {
    obstacles.iter().fold(servo, |velocity, normal| {
        let excess = velocity.dot(*normal) - OBSTACLE_PRESS_SPEED;
        if excess > 0.0 {
            velocity - *normal * excess
        } else {
            velocity
        }
    })
}

/// Angular velocity that turns `current` onto `target` within one step of `dt`, the short way.
pub(super) fn servo_angvel(current: Quat, target: Quat, dt: f32) -> Vec3 {
    let error = (target * current.inverse()).normalize();
    let error = if error.w < 0.0 { -error } else { error };
    let (axis, angle) = error.to_axis_angle();
    axis * angle / dt
}

/// How long the body has been held too far from its hold point.
#[derive(Copy, Clone, Default, Debug)]
pub(super) struct Strain {
    seconds: f32,
}

impl Strain {
    /// Accounts one step of `dt` at `separation`; true once the grip breaks.
    pub fn update(&mut self, separation: f32, dt: f32) -> bool {
        if separation > BREAK_SEPARATION {
            self.seconds += dt;
        } else {
            self.seconds = 0.0;
        }
        self.seconds >= BREAK_GRACE_SECONDS
    }
}

/// Launch speed of a throw charged for `charge_secs`, the same for every mass. A negative or
/// NaN charge is a tap.
pub(super) fn throw_speed(charge_secs: f32) -> f32 {
    (THROW_BASE_SPEED + THROW_SPEED_PER_SECOND * charge_secs.max(0.0)).min(THROW_MAX_SPEED)
}

#[cfg(test)]
mod tests {
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

    use super::*;
    use crate::sim::FIXED_DT;

    const EPSILON: f32 = 1e-4;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPSILON
    }

    #[test]
    fn a_tap_throws_at_the_base_speed() {
        assert!(close(throw_speed(0.0), 22.0));
        assert!(close(throw_speed(-1.0), 22.0));
        assert!(close(throw_speed(f32::NAN), 22.0));
    }

    #[test]
    fn the_throw_speed_ramps_with_the_charge() {
        assert!(close(throw_speed(0.4), 22.0 + 78.3 * 0.4));
        assert!(close(throw_speed(1.2), 22.0 + 78.3 * 1.2));
        assert!(throw_speed(0.8) > throw_speed(0.4));
    }

    #[test]
    fn the_throw_speed_plateaus_from_one_point_six_seconds() {
        for charge in [1.6, 2.4, 3.2, 100.0, f32::INFINITY] {
            assert!(close(throw_speed(charge), 147.0), "{charge}");
        }
        assert!(throw_speed(1.5) < THROW_MAX_SPEED);
    }

    #[test]
    fn mass_seventy_keeps_running_and_mass_eighty_walks() {
        assert!(!forces_walk(70.0));
        assert!(forces_walk(80.0));
        assert!(forces_walk(WALK_MASS_THRESHOLD));
    }

    #[test]
    fn mass_one_hundred_is_grabbed_and_mass_one_hundred_twenty_refused() {
        assert!(can_grab(100.0));
        assert!(!can_grab(120.0));
        assert!(can_grab(GRAB_MASS_LIMIT));
    }

    fn strained_seconds(separation_fo4: f32, seconds: f32) -> Option<f32> {
        let mut strain = Strain::default();
        let ticks = (seconds / FIXED_DT).round() as u32;
        (1..=ticks)
            .find(|_| strain.update(separation_fo4 * UNITS_TO_ENGINE, FIXED_DT))
            .map(|tick| tick as f32 * FIXED_DT)
    }

    #[test]
    fn a_separation_of_one_hundred_fifty_never_breaks_the_grip() {
        assert_eq!(strained_seconds(150.0, 600.0), None);
    }

    #[test]
    fn a_separation_of_two_hundred_breaks_the_grip_within_one_point_one_seconds() {
        let broke = strained_seconds(200.0, 1.1).expect("torn away");
        assert!(broke > 0.6 && broke <= 1.1, "{broke}");
    }

    #[test]
    fn closing_back_in_resets_the_strain() {
        let mut strain = Strain::default();
        for _ in 0..40 {
            assert!(!strain.update(BREAK_SEPARATION * 2.0, FIXED_DT));
        }
        assert!(!strain.update(0.0, FIXED_DT));
        for _ in 0..40 {
            assert!(!strain.update(BREAK_SEPARATION * 2.0, FIXED_DT));
        }
    }

    #[test]
    fn an_upright_box_seen_along_a_horizontal_direction_spins_about_its_local_x() {
        let facing_minus_z = View {
            yaw: -FRAC_PI_2,
            pitch: 0.0,
        };
        assert!(spin_axis(Quat::IDENTITY, facing_minus_z.right()).abs_diff_eq(Vec3::X, EPSILON));
        let facing_plus_z = View {
            yaw: FRAC_PI_2,
            pitch: 0.3,
        };
        assert!(
            spin_axis(Quat::IDENTITY, facing_plus_z.right()).abs_diff_eq(Vec3::NEG_X, EPSILON),
            "signed to point along the camera's right"
        );
    }

    #[test]
    fn a_turned_body_spins_about_the_local_axis_nearest_the_camera_right() {
        let lying = Quat::from_rotation_z(FRAC_PI_2);
        let right = View {
            yaw: -FRAC_PI_2,
            pitch: 0.0,
        }
        .right();
        let axis = spin_axis(lying, right);
        assert!((lying * axis).abs_diff_eq(right, EPSILON), "{axis}");
    }

    #[test]
    fn the_hold_distance_is_nine_units_short_of_the_grab_distance() {
        assert!(close(hold_distance(10.0), 9.1));
        assert_eq!(hold_distance(0.5), 0.0);
    }

    #[test]
    fn the_carry_target_lies_the_hold_distance_along_the_view() {
        let eye = Vec3::new(1.0, 2.0, 3.0);
        let view = View {
            yaw: 0.0,
            pitch: FRAC_PI_4,
        };
        let target = carry_target(eye, view, 4.0);
        assert!(close(target.distance(eye), 4.0));
        assert!(target.y > eye.y, "positive pitch looks up");
        assert!(close(target.z, eye.z), "yaw 0 looks along +X");
    }

    #[test]
    fn the_view_frame_follows_the_look_direction() {
        for (yaw, pitch) in [
            (0.0, 0.0),
            (0.7, -0.4),
            (-2.5, 1.2),
            (FRAC_PI_2, -FRAC_PI_4),
        ] {
            let view = View { yaw, pitch };
            assert!(
                (view.rotation() * Vec3::X).abs_diff_eq(view.forward(), EPSILON),
                "{view:?}"
            );
            assert!(close(view.forward().length(), 1.0));
            assert!(close(view.right().dot(view.forward()), 0.0));
            assert!(close(view.right().y, 0.0), "the right axis stays level");
        }
    }

    #[test]
    fn yaw_zero_looks_along_plus_x_and_minus_half_pi_along_minus_z() {
        assert!(look_direction(0.0, 0.0).abs_diff_eq(Vec3::X, EPSILON));
        assert!(look_direction(-FRAC_PI_2, 0.0).abs_diff_eq(Vec3::NEG_Z, EPSILON));
        assert!(look_direction(0.0, FRAC_PI_2).abs_diff_eq(Vec3::Y, EPSILON));
    }

    #[test]
    fn one_second_of_spin_turns_one_hundred_seventy_two_degrees() {
        let mut rotation = Quat::IDENTITY;
        for _ in 0..60 {
            rotation *= spin_step(Vec3::X, 1, FIXED_DT);
        }
        let (axis, angle) = rotation.to_axis_angle();
        assert!((angle.to_degrees() - 172.0).abs() < 0.01, "{angle}");
        assert!(axis.abs_diff_eq(Vec3::NEG_X, EPSILON));
    }

    #[test]
    fn the_buttons_spin_opposite_ways_and_no_button_holds_still() {
        let left = spin_step(Vec3::X, 1, FIXED_DT);
        let right = spin_step(Vec3::X, -1, FIXED_DT);
        assert!((left * right).abs_diff_eq(Quat::IDENTITY, EPSILON));
        assert_eq!(spin_step(Vec3::X, 0, FIXED_DT), Quat::IDENTITY);
    }

    #[test]
    fn the_left_button_tips_the_top_away_from_the_player() {
        let view = View {
            yaw: -FRAC_PI_2,
            pitch: 0.0,
        };
        let axis = spin_axis(Quat::IDENTITY, view.right());
        let top = spin_step(axis, 1, FIXED_DT * 10.0) * Vec3::Y;
        assert!(top.dot(view.forward()) > 0.0, "{top}");
    }

    #[test]
    fn the_linear_servo_closes_the_gap_in_one_step() {
        let current = Vec3::new(1.0, 2.0, 3.0);
        let target = Vec3::new(2.0, 0.0, 3.5);
        let velocity = servo_linvel(current, target, FIXED_DT);
        assert!((current + velocity * FIXED_DT).abs_diff_eq(target, EPSILON));
    }

    #[test]
    fn a_body_against_an_obstacle_presses_on_gently_and_keeps_following_along_it() {
        let servo = Vec3::new(140.0, 3.0, -2.0);
        let wall = [Vec3::X];
        assert!(
            limit_push(servo, &wall)
                .abs_diff_eq(Vec3::new(OBSTACLE_PRESS_SPEED, 3.0, -2.0), EPSILON)
        );
        let backing_off = Vec3::new(-50.0, 3.0, 0.0);
        assert_eq!(limit_push(backing_off, &wall), backing_off);
    }

    #[test]
    fn a_body_in_a_corner_presses_gently_into_both_faces() {
        let pressed = limit_push(Vec3::new(140.0, -80.0, 5.0), &[Vec3::X, Vec3::NEG_Y]);
        assert!(pressed.abs_diff_eq(
            Vec3::new(OBSTACLE_PRESS_SPEED, -OBSTACLE_PRESS_SPEED, 5.0),
            EPSILON
        ));
    }

    #[test]
    fn a_body_touching_nothing_is_servoed_at_full_speed() {
        let servo = Vec3::new(140.0, 3.0, -2.0);
        assert_eq!(limit_push(servo, &[]), servo);
    }

    #[test]
    fn the_angular_servo_turns_the_short_way_in_one_step() {
        let current = Quat::from_rotation_y(0.2);
        let target = Quat::from_rotation_y(-0.3);
        let velocity = servo_angvel(current, target, FIXED_DT);
        assert!(velocity.abs_diff_eq(Vec3::new(0.0, -0.5 / FIXED_DT, 0.0), 1e-2));
        let flipped = servo_angvel(current, -target, FIXED_DT);
        assert!(
            flipped.abs_diff_eq(velocity, 1e-2),
            "q and -q are one rotation"
        );
        assert_eq!(servo_angvel(current, current, FIXED_DT), Vec3::ZERO);
    }
}
