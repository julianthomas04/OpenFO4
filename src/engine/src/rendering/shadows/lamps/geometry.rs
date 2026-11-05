use glam::{Mat4, Vec3};

use super::source::{LampCandidate, ShadowType};
use crate::rendering::shadows::views::{ShadowView, ViewCull};

pub const CUBE_FACES: usize = 6;
/// Faces in the lamp's basis: +forward, -forward, +up, -up, +side, -side.
const BACK_FACE: usize = 1;
pub const ALL_FACES: u8 = 0b11_1111;
const FRONT_FACES: u8 = ALL_FACES & !(1 << BACK_FACE);

/// Spots up to this full cone angle use one perspective tile; wider ones are
/// stored like hemisphere lights.
const MAX_SINGLE_TILE_SPOT_COS_HALF: f32 = 0.5;
/// Texels on each tile border outside the light's frustum, so filter taps of
/// receivers inside the frustum stay inside the tile.
pub const GUARD_TEXELS: u32 = 2;
/// Below this |forward.y| the basis keeps world Y as its up axis.
const VERTICAL_UP_SWITCH: f32 = 0.99;

/// How a lamp's depth is stored: one perspective tile along its axis, or the
/// faces of a cube in its basis.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum LampProjection {
    Spot { tan_half: f32 },
    Cube { faces: u8 },
}

impl LampProjection {
    /// Omni lamps store all six faces; hemisphere lamps and wide spots light
    /// only the half space in front of them and drop the back face.
    pub fn of(candidate: &LampCandidate) -> Self {
        match candidate.spec.shadow_type {
            ShadowType::Spot => Self::single_spot(candidate.light.spot_cos_outer)
                .unwrap_or(Self::Cube { faces: FRONT_FACES }),
            ShadowType::Hemisphere => Self::Cube { faces: FRONT_FACES },
            ShadowType::Omni => Self::Cube { faces: ALL_FACES },
        }
    }

    /// One perspective tile for a spot whose cone has the half-angle cosine `cos_half`; `None`
    /// for a cone too wide for one tile, which is stored like a hemisphere light.
    pub fn single_spot(cos_half: f32) -> Option<Self> {
        (cos_half >= MAX_SINGLE_TILE_SPOT_COS_HALF).then(|| Self::Spot {
            tan_half: (1.0 - cos_half * cos_half).sqrt() / cos_half,
        })
    }

    pub fn face_mask(self) -> u8 {
        match self {
            Self::Spot { .. } => 1,
            Self::Cube { faces } => faces,
        }
    }

    pub fn face_count(self) -> u32 {
        self.face_mask().count_ones()
    }

    fn tan_half(self) -> f32 {
        match self {
            Self::Spot { tan_half } => tan_half,
            Self::Cube { .. } => 1.0,
        }
    }
}

/// The lamp's orthonormal basis: forward is the light direction; the rest
/// only has to agree between rendering and sampling.
#[derive(Debug, Copy, Clone)]
pub struct LampBasis {
    pub forward: Vec3,
    pub up: Vec3,
    pub side: Vec3,
}

impl LampBasis {
    pub fn of(direction: Vec3) -> Self {
        let forward = direction.normalize();
        let reference = if forward.y.abs() > VERTICAL_UP_SWITCH {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let side = forward.cross(reference).normalize();
        let up = side.cross(forward);
        Self { forward, up, side }
    }

    /// Direction and up vector each face looks along; mirrored by the face
    /// order of `lamp_face` in `lamp_shadow.wgsl`.
    fn face(&self, face: usize) -> (Vec3, Vec3) {
        match face {
            0 => (self.forward, self.up),
            1 => (-self.forward, self.up),
            2 => (self.up, self.forward),
            3 => (-self.up, self.forward),
            4 => (self.side, self.up),
            _ => (-self.side, self.up),
        }
    }
}

/// The geometry of one lamp's shadow, shared by its rendered views and the
/// receivers' lookup.
#[derive(Debug, Copy, Clone)]
pub struct LampFrustums {
    pub basis: LampBasis,
    pub projection: LampProjection,
    pub view_projs: [Mat4; CUBE_FACES],
    /// World size of one texel per unit of distance along the face axis.
    pub texel_per_distance: f32,
}

impl LampFrustums {
    /// Each face's field of view is widened by `GUARD_TEXELS` on every side.
    pub fn new(candidate: &LampCandidate, projection: LampProjection, tile_size: u32) -> Self {
        Self::from_parts(
            Vec3::from(candidate.light.position),
            Vec3::from(candidate.light.direction),
            candidate.spec.near_clip,
            projection,
            tile_size,
        )
    }

    /// The frustums of a lamp at `position` shining along `direction`, for lights that are not
    /// placed lamps of a cell.
    pub fn from_parts(
        position: Vec3,
        direction: Vec3,
        near_clip: f32,
        projection: LampProjection,
        tile_size: u32,
    ) -> Self {
        let basis = LampBasis::of(direction);
        let tan_half =
            projection.tan_half() * tile_size as f32 / (tile_size - 2 * GUARD_TEXELS) as f32;
        let projection_matrix =
            Mat4::perspective_infinite_reverse_rh(2.0 * tan_half.atan(), 1.0, near_clip);
        let view_projs = std::array::from_fn(|face| {
            let (look, up) = basis.face(face);
            projection_matrix * Mat4::look_to_rh(position, look, up)
        });
        Self {
            basis,
            projection,
            view_projs,
            texel_per_distance: 2.0 * tan_half / tile_size as f32,
        }
    }

    pub fn view(&self, candidate: &LampCandidate, face: usize) -> ShadowView {
        ShadowView {
            view_proj: self.view_projs[face],
            cull: ViewCull::Perspective {
                light_sphere: Vec3::from(candidate.light.position).extend(candidate.light.radius),
                texel_per_distance: self.texel_per_distance,
                near: candidate.spec.near_clip,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_narrow_cone_gets_one_tile_whose_half_tangent_follows_the_half_angle() {
        let cos_half = 10.0_f32.to_radians().cos();

        let projection = LampProjection::single_spot(cos_half).unwrap();

        let LampProjection::Spot { tan_half } = projection else {
            panic!("{projection:?}");
        };
        assert!(
            (tan_half - 10.0_f32.to_radians().tan()).abs() < 1e-6,
            "{tan_half}"
        );
        assert_eq!(projection.face_count(), 1);
    }

    #[test]
    fn a_cone_wider_than_120_degrees_is_not_a_single_tile() {
        assert!(LampProjection::single_spot(0.499).is_none());
        assert!(LampProjection::single_spot(0.5).is_some());
    }

    #[test]
    fn a_lamp_looks_along_its_direction_from_its_position_with_its_near_clip() {
        let position = Vec3::new(1.0, 2.0, 3.0);
        let projection = LampProjection::single_spot(0.9).unwrap();

        let frustums =
            LampFrustums::from_parts(position, Vec3::new(0.0, 0.0, -1.0), 0.5, projection, 1024);

        let ahead = frustums.view_projs[0] * (position + Vec3::new(0.0, 0.0, -4.0)).extend(1.0);
        let ndc = ahead.truncate() / ahead.w;
        assert!(ndc.x.abs() < 1e-5 && ndc.y.abs() < 1e-5, "{ndc}");
        let at_near = frustums.view_projs[0] * (position + Vec3::new(0.0, 0.0, -0.5)).extend(1.0);
        assert!(
            (at_near.z / at_near.w - 1.0).abs() < 1e-5,
            "reverse-Z: the near plane is 1"
        );
        assert_eq!(frustums.basis.forward, Vec3::new(0.0, 0.0, -1.0));
    }

    #[test]
    fn the_guard_band_widens_the_field_so_a_texel_is_larger_than_the_bare_cone_gives() {
        let projection = LampProjection::single_spot(0.9).unwrap();
        let LampProjection::Spot { tan_half } = projection else {
            unreachable!()
        };

        let frustums = LampFrustums::from_parts(Vec3::ZERO, Vec3::X, 1.0, projection, 1024);

        assert!(frustums.texel_per_distance > 2.0 * tan_half / 1024.0);
    }
}
