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
            ShadowType::Spot if candidate.light.spot_cos_outer >= MAX_SINGLE_TILE_SPOT_COS_HALF => {
                let cos_half = candidate.light.spot_cos_outer;
                Self::Spot {
                    tan_half: (1.0 - cos_half * cos_half).sqrt() / cos_half,
                }
            }
            ShadowType::Spot | ShadowType::Hemisphere => Self::Cube { faces: FRONT_FACES },
            ShadowType::Omni => Self::Cube { faces: ALL_FACES },
        }
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
        let position = Vec3::from(candidate.light.position);
        let basis = LampBasis::of(Vec3::from(candidate.light.direction));
        let tan_half =
            projection.tan_half() * tile_size as f32 / (tile_size - 2 * GUARD_TEXELS) as f32;
        let projection_matrix = Mat4::perspective_infinite_reverse_rh(
            2.0 * tan_half.atan(),
            1.0,
            candidate.spec.near_clip,
        );
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
