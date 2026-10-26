use glam::{Vec3, Vec4};

pub const AMBIENT_CUBE_FACES: usize = 6;

/// Fill calibration of the exterior ambient (+0.58 stop): the six linear face
/// colours of the cube used for exterior shading are multiplied by it before
/// upload. An engine tuning value, independent of where the cube comes from.
pub const EXTERIOR_AMBIENT_FILL: f32 = 1.5;

/// The six directional ambient colours, linear, faces in engine order
/// +X, -X, +Y, -Y, +Z, -Z.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct AmbientCube(pub [Vec3; AMBIENT_CUBE_FACES]);

impl AmbientCube {
    /// The faces as the frame uniform carries them (w unused).
    pub fn uniform_faces(&self) -> [Vec4; AMBIENT_CUBE_FACES] {
        self.0.map(|face| face.extend(0.0))
    }

    /// Every face colour multiplied by `factor`.
    #[must_use]
    pub fn scaled(&self, factor: f32) -> Self {
        Self(self.0.map(|face| face * factor))
    }

    /// `colour` added to every face colour.
    #[must_use]
    pub fn added(&self, colour: Vec3) -> Self {
        Self(self.0.map(|face| face + colour))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_faces_keep_the_face_order_and_colours() {
        let cube = AmbientCube([
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(3.0, 0.0, 0.0),
            Vec3::new(4.0, 0.0, 0.0),
            Vec3::new(5.0, 0.0, 0.0),
            Vec3::new(6.0, 0.0, 0.0),
        ]);
        let reds: Vec<f32> = cube.uniform_faces().iter().map(|face| face.x).collect();
        assert_eq!(reds, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn scaling_multiplies_every_face_and_keeps_the_face_order() {
        let cube = AmbientCube([
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(4.0, 5.0, 6.0),
            Vec3::new(7.0, 8.0, 9.0),
            Vec3::new(10.0, 11.0, 12.0),
            Vec3::new(13.0, 14.0, 15.0),
            Vec3::new(16.0, 17.0, 18.0),
        ]);
        let scaled = cube.scaled(0.5);
        for (face, original) in scaled.0.iter().zip(cube.0) {
            assert_eq!(*face, original * 0.5);
        }
    }

    #[test]
    fn adding_a_colour_lifts_every_face_by_it_and_keeps_the_face_order() {
        let cube = AmbientCube(std::array::from_fn(|face| Vec3::splat(face as f32)));
        let lifted = cube.added(Vec3::new(0.5, 0.25, 0.125));
        for (face, (lifted, original)) in lifted.0.iter().zip(cube.0).enumerate() {
            assert_eq!(
                *lifted,
                original + Vec3::new(0.5, 0.25, 0.125),
                "face {face}"
            );
        }
    }

    #[test]
    fn adding_no_light_leaves_the_cube_as_it_is() {
        let cube = AmbientCube(std::array::from_fn(|face| {
            Vec3::new(0.1 + face as f32, 0.0, 0.3)
        }));
        assert_eq!(cube.added(Vec3::ZERO), cube);
    }

    #[test]
    fn uniform_faces_leave_the_fourth_component_zero() {
        let cube = AmbientCube([Vec3::new(0.1, 0.2, 0.3); AMBIENT_CUBE_FACES]);
        for face in cube.uniform_faces() {
            assert_eq!(face, Vec4::new(0.1, 0.2, 0.3, 0.0));
        }
    }
}
