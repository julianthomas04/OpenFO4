use glam::Vec3;

use super::ambient::AmbientCube;
use super::cell_lighting::StoredColourEncoding;

/// Linear interpolation of a value that has a meaningful midpoint: `t` 0 gives `self`, 1 gives
/// `other`. Colours are blended in the linear space they are stored in.
pub trait Lerp: Sized {
    #[must_use]
    fn lerp(&self, other: &Self, t: f32) -> Self;
}

/// Implements [`Lerp`] for a struct by blending every field. The struct literal names each
/// field, so a field added later without being listed here does not compile.
macro_rules! lerp_fields {
    ($ty:ty { $($field:ident),+ $(,)? }) => {
        impl $crate::rendering::lighting::lerp::Lerp for $ty {
            fn lerp(&self, other: &Self, t: f32) -> Self {
                Self {
                    $($field: $crate::rendering::lighting::lerp::Lerp::lerp(&self.$field, &other.$field, t)),+
                }
            }
        }
    };
}
pub(crate) use lerp_fields;

impl Lerp for f32 {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        self + (other - self) * t
    }
}

impl Lerp for Vec3 {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        Vec3::lerp(*self, *other, t)
    }
}

impl<T: Lerp, const N: usize> Lerp for [T; N] {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        std::array::from_fn(|index| self[index].lerp(&other[index], t))
    }
}

impl Lerp for AmbientCube {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        Self(self.0.lerp(&other.0, t))
    }
}

/// Two linear colours blended as the game blends weather colours: as the stored 0 to 1 values
/// they were decoded from with `encoding`, and decoded again.
pub(crate) fn lerp_stored(from: Vec3, to: Vec3, t: f32, encoding: StoredColourEncoding) -> Vec3 {
    encoding.decode(encoding.encode(from).lerp(encoding.encode(to), t))
}

/// The end a value that has no midpoint snaps to: `from` up to and including exactly one half of
/// the way, `to` beyond it.
pub(crate) fn snap_to_nearer<'a, T: ?Sized>(from: &'a T, to: &'a T, t: f32) -> &'a T {
    if t > 0.5 { to } else { from }
}

/// An absent value cannot be blended with a present one, so the nearer end wins and the result
/// snaps at the midpoint (`self` at exactly one half).
impl<T: Lerp + Clone> Lerp for Option<T> {
    fn lerp(&self, other: &Self, t: f32) -> Self {
        match (self, other) {
            (Some(from), Some(to)) => Some(from.lerp(to, t)),
            _ => snap_to_nearer(self, other, t).clone(),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the blend at the ends and at one half is exact"
)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Pair {
        scale: f32,
        tint: Vec3,
    }
    lerp_fields!(Pair { scale, tint });

    fn pair(scale: f32, tint: f32) -> Pair {
        Pair {
            scale,
            tint: Vec3::splat(tint),
        }
    }

    #[test]
    fn scalars_and_vectors_blend_linearly_with_exact_ends() {
        assert_eq!(2.0_f32.lerp(&6.0, 0.0), 2.0);
        assert_eq!(2.0_f32.lerp(&6.0, 1.0), 6.0);
        assert_eq!(2.0_f32.lerp(&6.0, 0.25), 3.0);
        assert_eq!(
            Lerp::lerp(&Vec3::ZERO, &Vec3::splat(4.0), 0.5),
            Vec3::splat(2.0)
        );
    }

    /// Bytes 48 and 144 halfway are byte 96; each decoded by hand.
    #[test]
    fn a_stored_blend_of_srgb_colours_lands_on_the_blended_byte() {
        let byte_48 = ((48.0 / 255.0 + 0.055) / 1.055_f32).powf(2.4);
        let byte_96 = ((96.0 / 255.0 + 0.055) / 1.055_f32).powf(2.4);
        let byte_144 = ((144.0 / 255.0 + 0.055) / 1.055_f32).powf(2.4);
        let halfway = lerp_stored(
            Vec3::splat(byte_48),
            Vec3::splat(byte_144),
            0.5,
            StoredColourEncoding::Srgb,
        );
        assert!(halfway.abs_diff_eq(Vec3::splat(byte_96), 1e-6), "{halfway}");
    }

    #[test]
    fn a_stored_blend_of_gamma_colours_lands_on_the_blended_byte() {
        let byte = |value: f32| (value / 255.0).powf(2.2);
        let quarter = lerp_stored(
            Vec3::new(byte(0.0), byte(200.0), byte(40.0)),
            Vec3::new(byte(100.0), byte(0.0), byte(40.0)),
            0.25,
            StoredColourEncoding::Gamma22,
        );
        let expected = Vec3::new(byte(25.0), byte(150.0), byte(40.0));
        assert!(quarter.abs_diff_eq(expected, 1e-6), "{quarter}");
    }

    #[test]
    fn a_stored_blend_keeps_its_ends() {
        let (from, to) = (Vec3::new(0.0, 0.002, 0.3), Vec3::new(0.9, 0.5, 0.001));
        for encoding in [
            StoredColourEncoding::Srgb,
            StoredColourEncoding::Gamma22,
            StoredColourEncoding::Linear,
        ] {
            assert!(lerp_stored(from, to, 0.0, encoding).abs_diff_eq(from, 1e-6));
            assert!(lerp_stored(from, to, 1.0, encoding).abs_diff_eq(to, 1e-6));
        }
    }

    #[test]
    fn arrays_blend_element_wise() {
        assert_eq!([0.0_f32, 10.0].lerp(&[4.0, 20.0], 0.5), [2.0, 15.0]);
    }

    #[test]
    fn an_ambient_cube_blends_every_face() {
        let dark = AmbientCube([Vec3::ZERO; 6]);
        let bright = AmbientCube(std::array::from_fn(|face| Vec3::splat(2.0 * face as f32)));
        let halfway = dark.lerp(&bright, 0.5);
        for (face, colour) in halfway.0.iter().enumerate() {
            assert_eq!(*colour, Vec3::splat(face as f32), "face {face}");
        }
    }

    #[test]
    fn a_struct_blends_every_listed_field() {
        assert_eq!(pair(0.0, 0.0).lerp(&pair(2.0, 4.0), 0.5), pair(1.0, 2.0));
    }

    #[test]
    fn a_value_without_a_midpoint_snaps_to_the_nearer_end_and_the_first_at_exactly_one_half() {
        let ends = [
            (0.0, "from"),
            (0.4, "from"),
            (0.5, "from"),
            (0.6, "to"),
            (1.0, "to"),
        ];
        for (t, nearer) in ends {
            assert_eq!(snap_to_nearer("from", "to", t), nearer, "{t}");
        }
    }

    #[test]
    fn two_present_options_blend() {
        assert_eq!(Some(0.0_f32).lerp(&Some(4.0), 0.25), Some(1.0));
    }

    #[test]
    fn a_missing_option_snaps_to_the_nearer_end() {
        let present = Some(4.0_f32);
        assert_eq!(None.lerp(&present, 0.4), None);
        assert_eq!(None.lerp(&present, 0.6), present);
        assert_eq!(present.lerp(&None, 0.4), present);
        assert_eq!(present.lerp(&None, 0.6), None);
    }

    #[test]
    fn a_missing_option_snaps_to_the_first_value_at_exactly_one_half() {
        let present = Some(4.0_f32);
        assert_eq!(present.lerp(&None, 0.5), present);
        assert_eq!(None.lerp(&present, 0.5), None);
    }
}
