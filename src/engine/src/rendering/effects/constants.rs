//! The one place that fills the binding registers of the translated effect shaders: the effect
//! material, its controller tracks evaluated at the draw's time, the draw's model matrix, the
//! frame's camera, the picked lights and the ambient colour in; `vs_cb1`, `vs_cb2`, `vs_cb11`,
//! `ps_cb1`, `ps_cb2` (per draw) and `cb12`, `ps_cb0`, `vs_cb0` (per frame) out.
//!
//! Spaces and units:
//! - "Model space" is the space of the POSITION attribute: for meshes the GLB mesh space
//!   (engine axes, engine units), for particles the simulation space. The draw's model matrix
//!   maps it to engine space.
//! - Engine units (eu) = Bethesda units (BU) x 0.1.
//! - The camera-relative frame (`vs_cb2[4..=6]`, `cb12`) is Bethesda axes in BU, see
//!   [`crate::rendering::game_frame::cb12`].
//! - Clip space is the engine's reverse-Z with an infinite far plane.
//!
//! Registers the engine has no source for hold the documented neutral values.

use glam::{Mat4, Vec2, Vec3, Vec4};

use crate::anim::scalar_track::{TrackTarget, TrackValue};
use crate::assets::types::effect_material::EffectMaterial;
use crate::rendering::game_frame::cb12::{CB12_LEN, camera_relative_bethesda, frame_cb12};
use crate::rendering::lighting::frame_uniform::FrameUniform;
use crate::rendering::placed_light::{GpuPlacedLight, LIGHT_FLAG_SPOT_MASK};

/// Register counts of the binding slots the translated effect shaders read.
pub const VS_CB0_LEN: usize = 1;
pub const VS_CB1_LEN: usize = 3;
pub const VS_CB2_LEN: usize = 26;
pub const VS_CB11_LEN: usize = 128;
pub const PS_CB0_LEN: usize = 20;
pub const PS_CB1_LEN: usize = 5;
pub const PS_CB2_LEN: usize = 17;

/// Lights one lit effect draw receives.
pub const LIGHT_COUNT: usize = 4;

/// Gamma the effect shaders expect authored colours and alpha to be raised to.
const COLOUR_GAMMA: f32 = 2.2;
/// Exponent of the light attenuation `(1 - saturate(d / r)^2)^2.2`.
const ATTENUATION_EXPONENT: f32 = 2.2;

/// `vs_cb1[1].x` lower bound: the soft fade divides by it.
const MIN_SOFT_FADE_DISTANCE_EU: f32 = 0.1;
/// Depth the soft-fade near reference is expressed in: 1 BU.
const DEPTH_REFERENCE_EU: f32 = 0.1;
/// Reference used when the near plane equals [`DEPTH_REFERENCE_EU`] (the linearisation factor
/// would be 0).
const FALLBACK_DEPTH_REFERENCE_EU: f32 = 0.2;
/// Offset that keeps the linearised depth of the sky (raw depth 0) finite.
const DEPTH_LINEARISATION_EPSILON: f32 = 1e-6;
/// Smallest distance between the falloff start and stop cosines (the shader divides by it).
const MIN_FALLOFF_SPAN: f32 = 1e-5;
/// Largest spot cone cosine (the shader divides by `1 - cos`).
const MAX_SPOT_CONE_COS: f32 = 1.0 - 1e-5;
/// Model-space coordinate of an unused light slot: far away, with zero colour.
const UNUSED_LIGHT_POSITION: f32 = 1.0e7;

/// Registers by name.
const VS_CB1_UV_TRANSFORM: usize = 0;
const VS_CB1_SOFT_FADE: usize = 1;
const VS_CB1_FALLOFF: usize = 2;
const VS_CB2_CLIP_ROWS: usize = 0;
const VS_CB2_CAMERA_RELATIVE_ROWS: usize = 4;
const VS_CB2_MODEL_CAMERA: usize = 13;
const PS_CB0_DEPTH_LINEARISATION: usize = 0;
const PS_CB1_BASE_COLOUR: usize = 0;
const PS_CB1_PALETTE_SCALE: usize = 1;
const PS_CB1_LIGHTING: usize = 2;
const PS_CB1_ENVIRONMENT: usize = 3;
const PS_CB2_LIGHT_POSITION: usize = 0;
const PS_CB2_SPOT_DIRECTION: usize = 3;
const PS_CB2_SPOT_EXPONENT: usize = 6;
const PS_CB2_SPOT_CONE_COS: usize = 7;
const PS_CB2_INVERSE_RADIUS: usize = 8;
const PS_CB2_LIGHT_COLOUR: usize = 9;
const PS_CB2_AMBIENT: usize = 12;
const PS_CB2_LIGHT_INFLUENCE_AND_ALPHA: usize = 13;
const PS_CB2_ALPHA_TEST: usize = 14;

/// `ps_cb2[14].y`: texture-alpha cut; 1 disables it (no engine source).
const NO_TEXTURE_ALPHA_CUT: f32 = 1.0;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ConstantsError {
    #[error("effect shader name `{0}` does not end in an 8-digit hex descriptor")]
    MalformedShaderName(String),
    #[error("{count} lights exceed the {LIGHT_COUNT} light registers")]
    TooManyLights { count: usize },
    #[error("{count} sub-texture rectangles exceed the {VS_CB11_LEN} registers")]
    TooManySubtextures { count: usize },
    #[error("track target `{target}` does not drive an effect material")]
    NotAnEffectTarget { target: &'static str },
    #[error("track target `{target}` produced a value of the wrong kind")]
    TrackValueKind { target: &'static str },
    #[error("the model matrix is singular or not finite")]
    SingularModel,
}

/// Descriptor of an effect shader permutation: the 8 hex digits ending its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Descriptor(u32);

impl Descriptor {
    pub const TEXTURE_COORDINATES: u32 = 0x0000_0004;
    pub const SUBTEXTURES: u32 = 0x0000_0008;
    pub const FALLOFF: u32 = 0x0000_0010;
    pub const LIGHTING: u32 = 0x0000_0400;
    pub const SOFT: u32 = 0x0000_1000;
    pub const PALETTE_COLOUR: u32 = 0x0000_2000;
    pub const ENVIRONMENT: u32 = 0x0008_0000;
    pub const COLOUR_FALLOFF: u32 = 0x0020_0000;
    pub const DISTORTION: u32 = 0x0040_0000;

    pub const fn new(bits: u32) -> Self {
        Self(bits)
    }

    /// `effect_ps_00a06015` -> `0x00a06015`.
    pub fn of_shader_name(name: &str) -> Result<Self, ConstantsError> {
        name.rsplit_once('_')
            .and_then(|(_, digits)| parse_descriptor_digits(digits))
            .map(Self)
            .ok_or_else(|| ConstantsError::MalformedShaderName(name.to_owned()))
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    const fn has_any(self, bits: u32) -> bool {
        self.0 & bits != 0
    }
}

/// The 8 hex digits ending an effect shader name; the one parser of that format.
pub fn parse_descriptor_digits(digits: &str) -> Option<u32> {
    if digits.len() != 8 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

/// `cb12`, `ps_cb0` and `vs_cb0`: constants shared by every effect draw of a frame.
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct EffectFrameConstants {
    pub cb12: [Vec4; CB12_LEN],
    pub vs_cb0: [Vec4; VS_CB0_LEN],
    pub ps_cb0: [Vec4; PS_CB0_LEN],
}

const _: () = assert!(std::mem::size_of::<EffectFrameConstants>() as u64 == FRAME_UNIFORM_SIZE);

/// Per-draw registers. `vs_cb11` only carries data for particle draws.
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct EffectDrawConstants {
    pub vs_cb1: [Vec4; VS_CB1_LEN],
    pub vs_cb2: [Vec4; VS_CB2_LEN],
    pub vs_cb11: [Vec4; VS_CB11_LEN],
    pub ps_cb1: [Vec4; PS_CB1_LEN],
    pub ps_cb2: [Vec4; PS_CB2_LEN],
    pub padding: [Vec4; DRAW_PADDING_LEN],
}

/// Vec4 rows of the per-draw uniform (`vs_cb1`, `vs_cb2`, `vs_cb11`, `ps_cb1`, `ps_cb2`).
pub const DRAW_ROWS: usize = VS_CB1_LEN + VS_CB2_LEN + VS_CB11_LEN + PS_CB1_LEN + PS_CB2_LEN;
/// Vec4 rows of the per-frame uniform (`cb12`, `vs_cb0`, `ps_cb0`).
pub const FRAME_ROWS: usize = CB12_LEN + VS_CB0_LEN + PS_CB0_LEN;
pub const FRAME_UNIFORM_SIZE: u64 = FRAME_ROWS as u64 * 16;
pub const DRAW_UNIFORM_SIZE: u64 = DRAW_ROWS as u64 * 16;
/// WebGPU's minimum uniform offset alignment, which every desktop backend satisfies.
const UNIFORM_OFFSET_ALIGNMENT: u64 = 256;
/// Dynamic-offset stride of one draw's constants.
pub const DRAW_CONSTANTS_STRIDE: u64 =
    DRAW_UNIFORM_SIZE.div_ceil(UNIFORM_OFFSET_ALIGNMENT) * UNIFORM_OFFSET_ALIGNMENT;
const DRAW_PADDING_LEN: usize = DRAW_CONSTANTS_STRIDE as usize / 16 - DRAW_ROWS;
const _: () = assert!(std::mem::size_of::<EffectDrawConstants>() as u64 == DRAW_CONSTANTS_STRIDE);

/// The material values the controller tracks can override, at one point in time. Colours and
/// alpha are as authored (before the shader's gamma).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimatedParameters {
    /// rgb: `base_color.rgb`, or the `emissive_color` track; a: `base_color.a`.
    pub base_color: Vec4,
    /// `base_color_scale`, or the `emissive_mult` track.
    pub base_color_scale: f32,
    /// `alpha`, or the `alpha` track.
    pub alpha: f32,
    /// `uv_offset`, or the `u_offset` / `v_offset` tracks.
    pub uv_offset: Vec2,
    /// `uv_scale`, or the `u_scale` / `v_scale` tracks.
    pub uv_scale: Vec2,
}

impl AnimatedParameters {
    /// The material's own values, no track applied.
    pub fn at_rest(material: &EffectMaterial) -> Self {
        Self {
            base_color: Vec4::from(material.base_color),
            base_color_scale: material.base_color_scale,
            alpha: material.alpha,
            uv_offset: Vec2::from(material.uv_offset),
            uv_scale: Vec2::from(material.uv_scale),
        }
    }

    /// The values `elapsed` seconds after the instance started. When several tracks drive the
    /// same target, the first one in the material's list wins.
    pub fn at(material: &EffectMaterial, elapsed: f32) -> Result<Self, ConstantsError> {
        let mut parameters = Self::at_rest(material);
        for track in material.controllers.iter().rev() {
            parameters.apply(track.target, track.evaluate(elapsed))?;
        }
        Ok(parameters)
    }

    fn apply(&mut self, target: TrackTarget, value: TrackValue) -> Result<(), ConstantsError> {
        let kind_error = || ConstantsError::TrackValueKind {
            target: target.name(),
        };
        let scalar = || match value {
            TrackValue::Scalar(scalar) => Ok(scalar),
            TrackValue::Vec3(_) => Err(kind_error()),
        };
        match target {
            TrackTarget::EmissiveMult => self.base_color_scale = scalar()?,
            TrackTarget::Alpha => self.alpha = scalar()?,
            TrackTarget::UOffset => self.uv_offset.x = scalar()?,
            TrackTarget::VOffset => self.uv_offset.y = scalar()?,
            TrackTarget::UScale => self.uv_scale.x = scalar()?,
            TrackTarget::VScale => self.uv_scale.y = scalar()?,
            TrackTarget::EmissiveColor => match value {
                TrackValue::Vec3(rgb) => self.base_color = rgb.extend(self.base_color.w),
                TrackValue::Scalar(_) => return Err(kind_error()),
            },
            TrackTarget::Dimmer
            | TrackTarget::LightColor
            | TrackTarget::Offset
            | TrackTarget::RefractionStrength => {
                return Err(ConstantsError::NotAnEffectTarget {
                    target: target.name(),
                });
            }
        }
        Ok(())
    }
}

/// Spot cone of an [`EffectLight`], engine space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpotCone {
    /// Unit vector the cone points along.
    pub direction: Vec3,
    /// Cosine of the cone half angle.
    pub cos_half_angle: f32,
    pub exponent: f32,
}

/// One local light fed to an effect draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectLight {
    /// Engine space.
    pub position: Vec3,
    /// Engine units (LIGH radius BU x 0.1, times the reference's scale).
    pub radius: f32,
    /// Linear colour on the scale where 1.0 leaves the texture colour unchanged: the LIGH
    /// colour `pow(rgb / 255, 2.2)` times its fade and the current dimmer.
    pub color: Vec3,
    pub spot: Option<SpotCone>,
}

impl EffectLight {
    /// A placed light as the GPU slab holds it (pre-animation colour).
    pub fn from_placed(light: &GpuPlacedLight) -> Self {
        let spot = (light.flags & LIGHT_FLAG_SPOT_MASK != 0).then(|| SpotCone {
            direction: Vec3::from(light.direction),
            cos_half_angle: light.spot_cos_outer,
            exponent: light.spot_exponent,
        });
        Self {
            position: Vec3::from(light.position),
            radius: light.radius,
            color: Vec3::from(light.color),
            spot,
        }
    }

    /// `(1 - saturate(d / r)^2)^2.2`, times `saturate((cos - cos_half) / (1 - cos_half))^exponent`
    /// inside a spot cone.
    pub fn attenuation_at(&self, point: Vec3) -> f32 {
        if self.radius <= 0.0 {
            return 0.0;
        }
        let offset = point - self.position;
        let ratio = (offset.length() / self.radius).clamp(0.0, 1.0);
        let distance_term = (1.0 - ratio * ratio).powf(ATTENUATION_EXPONENT);
        let cone_term = match (self.spot, offset.try_normalize()) {
            (Some(spot), Some(to_point)) => {
                let cos_half = spot.cos_half_angle.min(MAX_SPOT_CONE_COS);
                let inside = (spot.direction.dot(to_point) - cos_half) / (1.0 - cos_half);
                inside.clamp(0.0, 1.0).powf(spot.exponent)
            }
            // A point light, or the point at the light itself: no cone term.
            _ => 1.0,
        };
        distance_term * cone_term
    }
}

/// The light environment of one draw. With `LIGHT_COUNT` lights or fewer; unused slots get
/// zero-colour lights.
#[derive(Debug, Clone, Copy)]
pub struct EffectLighting<'a> {
    pub lights: &'a [EffectLight],
    /// Linear cell ambient colour at the draw.
    pub ambient: Vec3,
    /// Engine-space point the light sum for the lighting-influence tint is evaluated at (the
    /// draw's bounding-box centre).
    pub centre: Vec3,
}

impl EffectLighting<'_> {
    /// `ps_cb2[13].xyz`: the ambient plus the sum of the lights evaluated at the centre.
    pub fn light_at_centre(&self) -> Vec3 {
        self.lights
            .iter()
            .map(|light| light.color * light.attenuation_at(self.centre))
            .fold(self.ambient, |sum, light| sum + light)
    }
}

/// The frame's camera as the effect registers need it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectView {
    /// Engine reverse-Z view-projection.
    pub view_projection: Mat4,
    /// Engine space.
    pub camera: Vec3,
    /// Camera near plane, eu.
    pub near_plane: f32,
}

impl EffectView {
    pub fn of_frame(frame: &FrameUniform, near_plane: f32) -> Self {
        Self {
            view_projection: frame.view_proj,
            camera: frame.camera_pos.truncate(),
            near_plane,
        }
    }
}

/// Everything one draw's registers depend on.
#[derive(Debug, Clone, Copy)]
pub struct EffectDrawInputs<'a> {
    pub material: &'a EffectMaterial,
    /// Descriptor of the draw's pixel shader.
    pub descriptor: Descriptor,
    pub animated: &'a AnimatedParameters,
    /// Engine space <- model space.
    pub model: Mat4,
    pub view: &'a EffectView,
    pub lighting: &'a EffectLighting<'a>,
    /// Particle sub-texture rectangles `(u offset, u scale, v offset, v scale)` as stored;
    /// empty for meshes.
    pub subtexture_rects: &'a [[f32; 4]],
}

/// `cb12`, `ps_cb0` and `vs_cb0` for this frame's engine state.
pub fn frame_constants(frame: &FrameUniform, near_plane: f32) -> EffectFrameConstants {
    let mut ps_cb0 = [Vec4::ZERO; PS_CB0_LEN];
    ps_cb0[PS_CB0_DEPTH_LINEARISATION] = DepthLinearisation::new(near_plane).ps_cb0_row();
    EffectFrameConstants {
        cb12: frame_cb12(frame),
        vs_cb0: [Vec4::ZERO; VS_CB0_LEN],
        ps_cb0,
    }
}

pub fn draw_constants(inputs: &EffectDrawInputs) -> Result<EffectDrawConstants, ConstantsError> {
    let model_space = ModelSpace::of(inputs.model)?;
    Ok(EffectDrawConstants {
        vs_cb1: vertex_cb1(inputs),
        vs_cb2: vertex_cb2(inputs, &model_space),
        vs_cb11: vertex_cb11(inputs)?,
        ps_cb1: pixel_cb1(inputs),
        ps_cb2: pixel_cb2(inputs, &model_space)?,
        padding: [Vec4::ZERO; DRAW_PADDING_LEN],
    })
}

/// The inverse of the model matrix and its uniform scale (model units per engine unit
/// inverted: one model unit is `uniform_scale` eu).
struct ModelSpace {
    from_engine: Mat4,
    uniform_scale: f32,
}

impl ModelSpace {
    fn of(model: Mat4) -> Result<Self, ConstantsError> {
        let determinant = model.determinant();
        if !(determinant.is_finite() && determinant.abs() > f32::MIN_POSITIVE) {
            return Err(ConstantsError::SingularModel);
        }
        Ok(Self {
            from_engine: model.inverse(),
            uniform_scale: determinant.abs().cbrt(),
        })
    }
}

/// The scene-depth linearisation of the soft fade:
/// `scene_depth / s = Y / ((1 - depth) * Z + Yc)` with `Yc = k (1 + e)`, `Z = -k`,
/// `Y = k n / s`, `k = 1 - n / reference` (reverse-Z, infinite far plane, `n` the near plane).
struct DepthLinearisation {
    k: f32,
    near_plane: f32,
}

impl DepthLinearisation {
    fn new(near_plane: f32) -> Self {
        let k = 1.0 - near_plane / DEPTH_REFERENCE_EU;
        let k = if k.abs() < DEPTH_LINEARISATION_EPSILON {
            1.0 - near_plane / FALLBACK_DEPTH_REFERENCE_EU
        } else {
            k
        };
        Self { k, near_plane }
    }

    /// `ps_cb0[0]`: `(unused, Yc, Z, unused)`.
    fn ps_cb0_row(&self) -> Vec4 {
        Vec4::new(
            0.0,
            self.k * (1.0 + DEPTH_LINEARISATION_EPSILON),
            -self.k,
            0.0,
        )
    }

    /// `ps_cb1[2].y`: `Y` for the soft fade distance `s` (eu).
    fn numerator(&self, soft_fade_distance: f32) -> f32 {
        self.k * self.near_plane / soft_fade_distance
    }
}

fn soft_fade_distance(material: &EffectMaterial) -> f32 {
    material.soft_falloff_depth.max(MIN_SOFT_FADE_DISTANCE_EU)
}

fn gamma(value: f32) -> f32 {
    value.max(0.0).powf(COLOUR_GAMMA)
}

/// `vs_cb1`: uv transform, soft fade distance, falloff.
fn vertex_cb1(inputs: &EffectDrawInputs) -> [Vec4; VS_CB1_LEN] {
    let mut cb1 = [Vec4::ZERO; VS_CB1_LEN];
    let animated = inputs.animated;
    cb1[VS_CB1_UV_TRANSFORM] = Vec4::new(
        animated.uv_offset.x,
        animated.uv_offset.y,
        animated.uv_scale.x,
        animated.uv_scale.y,
    );
    if inputs.descriptor.has_any(Descriptor::SOFT) {
        cb1[VS_CB1_SOFT_FADE].x = soft_fade_distance(inputs.material);
    }
    cb1[VS_CB1_FALLOFF] = if inputs
        .descriptor
        .has_any(Descriptor::FALLOFF | Descriptor::COLOUR_FALLOFF)
    {
        let falloff = &inputs.material.falloff;
        let stop = if (falloff.stop_angle - falloff.start_angle).abs() < MIN_FALLOFF_SPAN {
            falloff.start_angle + MIN_FALLOFF_SPAN
        } else {
            falloff.stop_angle
        };
        Vec4::new(
            falloff.start_angle,
            stop,
            falloff.start_opacity,
            falloff.stop_opacity,
        )
    } else {
        Vec4::new(1.0, 1.0 + MIN_FALLOFF_SPAN, 1.0, 1.0)
    };
    cb1
}

/// `vs_cb2`: clip rows, camera-relative Bethesda rows, camera in model space.
fn vertex_cb2(inputs: &EffectDrawInputs, model_space: &ModelSpace) -> [Vec4; VS_CB2_LEN] {
    let mut cb2 = [Vec4::ZERO; VS_CB2_LEN];
    let clip = inputs.view.view_projection * inputs.model;
    for row in 0..4 {
        cb2[VS_CB2_CLIP_ROWS + row] = clip.row(row);
    }
    let camera_relative = camera_relative_bethesda(inputs.view.camera) * inputs.model;
    for row in 0..3 {
        cb2[VS_CB2_CAMERA_RELATIVE_ROWS + row] = camera_relative.row(row);
    }
    cb2[VS_CB2_MODEL_CAMERA] = model_space
        .from_engine
        .transform_point3(inputs.view.camera)
        .extend(1.0);
    cb2
}

/// `vs_cb11`: particle sub-texture rectangles, copied as stored.
fn vertex_cb11(inputs: &EffectDrawInputs) -> Result<[Vec4; VS_CB11_LEN], ConstantsError> {
    let rects = inputs.subtexture_rects;
    if rects.len() > VS_CB11_LEN {
        return Err(ConstantsError::TooManySubtextures { count: rects.len() });
    }
    let mut cb11 = [Vec4::ZERO; VS_CB11_LEN];
    if inputs.descriptor.has_any(Descriptor::SUBTEXTURES) {
        for (register, rect) in cb11.iter_mut().zip(rects) {
            *register = Vec4::from(*rect);
        }
    }
    Ok(cb11)
}

/// `ps_cb1`: base colour, palette scale, lighting influence and soft-fade numerator,
/// environment / distortion strength.
fn pixel_cb1(inputs: &EffectDrawInputs) -> [Vec4; PS_CB1_LEN] {
    let mut cb1 = [Vec4::ZERO; PS_CB1_LEN];
    let animated = inputs.animated;
    let descriptor = inputs.descriptor;
    let palette = descriptor.has_any(Descriptor::PALETTE_COLOUR);

    let colour = animated.base_color.to_array().map(gamma);
    let rgb_scale = if palette {
        1.0
    } else {
        animated.base_color_scale
    };
    cb1[PS_CB1_BASE_COLOUR] = Vec4::new(
        colour[0] * rgb_scale,
        colour[1] * rgb_scale,
        colour[2] * rgb_scale,
        colour[3],
    );
    cb1[PS_CB1_PALETTE_SCALE].x = if palette {
        animated.base_color_scale
    } else {
        1.0
    };

    cb1[PS_CB1_LIGHTING].x = inputs.material.lighting_influence;
    if descriptor.has_any(Descriptor::SOFT) {
        cb1[PS_CB1_LIGHTING].y = DepthLinearisation::new(inputs.view.near_plane)
            .numerator(soft_fade_distance(inputs.material));
    }
    if descriptor.has_any(Descriptor::ENVIRONMENT | Descriptor::DISTORTION) {
        cb1[PS_CB1_ENVIRONMENT].x = inputs.material.env_map_scale;
    }
    cb1
}

/// `ps_cb2`: the light registers (model space), ambient, lighting-influence tint, alpha and
/// alpha test.
fn pixel_cb2(
    inputs: &EffectDrawInputs,
    model_space: &ModelSpace,
) -> Result<[Vec4; PS_CB2_LEN], ConstantsError> {
    let lighting = inputs.lighting;
    if lighting.lights.len() > LIGHT_COUNT {
        return Err(ConstantsError::TooManyLights {
            count: lighting.lights.len(),
        });
    }
    let mut cb2 = [Vec4::ZERO; PS_CB2_LEN];
    if inputs.descriptor.has_any(Descriptor::LIGHTING) {
        for slot in 0..LIGHT_COUNT {
            write_light(&mut cb2, slot, lighting.lights.get(slot), model_space);
        }
    }
    cb2[PS_CB2_AMBIENT] = lighting.ambient.extend(0.0);
    cb2[PS_CB2_LIGHT_INFLUENCE_AND_ALPHA] = lighting
        .light_at_centre()
        .extend(gamma(inputs.animated.alpha));
    let blend = &inputs.material.blend;
    let alpha_threshold = if blend.alpha_test {
        blend.test_ref
    } else {
        0.0
    };
    cb2[PS_CB2_ALPHA_TEST] = Vec4::new(alpha_threshold, NO_TEXTURE_ALPHA_CUT, 0.0, 0.0);
    Ok(cb2)
}

/// Component `slot` of rows `0..=11`: one light, model space and model units.
fn write_light(
    cb2: &mut [Vec4; PS_CB2_LEN],
    slot: usize,
    light: Option<&EffectLight>,
    model_space: &ModelSpace,
) {
    let Some(light) = light else {
        for axis in 0..3 {
            cb2[PS_CB2_LIGHT_POSITION + axis][slot] = UNUSED_LIGHT_POSITION;
        }
        return;
    };
    let position = model_space.from_engine.transform_point3(light.position);
    let radius_model = light.radius / model_space.uniform_scale;
    let inverse_radius = if radius_model > 0.0 {
        radius_model.recip()
    } else {
        0.0
    };
    let color = if radius_model > 0.0 {
        light.color
    } else {
        Vec3::ZERO
    };
    let (direction, exponent, cone_cos) = light.spot.map_or((Vec3::ZERO, 0.0, 0.0), |spot| {
        let direction = model_space
            .from_engine
            .transform_vector3(spot.direction)
            .normalize_or_zero();
        (
            direction,
            spot.exponent,
            spot.cos_half_angle.min(MAX_SPOT_CONE_COS),
        )
    });
    for axis in 0..3 {
        cb2[PS_CB2_LIGHT_POSITION + axis][slot] = position[axis];
        cb2[PS_CB2_SPOT_DIRECTION + axis][slot] = direction[axis];
        cb2[PS_CB2_LIGHT_COLOUR + axis][slot] = color[axis];
    }
    cb2[PS_CB2_SPOT_EXPONENT][slot] = exponent;
    cb2[PS_CB2_SPOT_CONE_COS][slot] = cone_cos;
    cb2[PS_CB2_INVERSE_RADIUS][slot] = inverse_radius;
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "registers are copies or exact products of fixture values"
)]
mod tests {
    use super::*;
    use crate::bethesda_space::position_from_engine;
    use crate::rendering::placed_light::LIGHT_FLAG_NON_SHADOW_SPOTLIGHT;
    use glam::Quat;
    use serde_json::json;

    /// Engine camera near plane, eu.
    const NEAR_PLANE: f32 = 0.5;

    /// The barrel flame card: palette colour and alpha, a base colour scale driven by an
    /// emissive track, uv offset, falloff, plus soft depth and additive blending so every
    /// register those bits feed is exercised.
    const FLAME_PS: &str = "effect_ps_00a07035";

    fn flame_json() -> serde_json::Value {
        json!({
            "v": 1,
            "vs": "effect_vs_00a01035", "ps": FLAME_PS,
            "flags1": 2_147_483_664u32, "flags2": 48,
            "blend": { "mode": "add", "src": "src_alpha", "dst": "one",
                       "alpha_test": true, "test_func": "greater", "test_ref": 0.5 },
            "z_test": true, "z_write": false, "double_sided": true,
            "clamp": "wrap_s_wrap_t",
            "textures": { "source": "effects/embers_d.dds",
                          "greyscale": "effects/gradients/fireflame.dds" },
            "base_color": [0.64, 0.5, 0.25, 0.8], "base_color_scale": 2.64,
            "falloff": { "start_angle": 0.9, "stop_angle": 0.3,
                         "start_opacity": 0.2, "stop_opacity": 1.0 },
            "soft_falloff_depth": 10.0,
            "uv_offset": [0.0024, 0.0024], "uv_scale": [1.0, 2.0],
            "lighting_influence": 0.098, "env_map_scale": 0.35,
            "emittance_color": [0, 0, 0], "refraction_power": 0.0, "alpha": 0.9,
            "controllers": [
                { "target": "emissive_mult", "cycle": "loop", "start": 0.0, "stop": 0.04,
                  "frequency": 1.0, "phase": 0.0,
                  "times": [0.0, 0.04], "values": [2.6, 2.7] }
            ]
        })
    }

    fn material_from(value: serde_json::Value) -> EffectMaterial {
        serde_json::from_value(value).expect("the fixture is a valid effect material")
    }

    fn flame() -> EffectMaterial {
        material_from(flame_json())
    }

    fn flame_with_tracks(tracks: serde_json::Value) -> EffectMaterial {
        let mut value = flame_json();
        value["controllers"] = tracks;
        material_from(value)
    }

    fn view() -> EffectView {
        let eye = Vec3::new(120.0, 35.0, -48.0);
        let look = Mat4::look_to_rh(eye, Vec3::new(0.3, -0.2, -1.0).normalize(), Vec3::Y);
        let projection = Mat4::perspective_infinite_reverse_rh(1.2, 16.0 / 9.0, NEAR_PLANE);
        EffectView {
            view_projection: projection * look,
            camera: eye,
            near_plane: NEAR_PLANE,
        }
    }

    /// Uniform scale 2, rotated, placed near the camera.
    fn model() -> Mat4 {
        Mat4::from_scale_rotation_translation(
            Vec3::splat(2.0),
            Quat::from_rotation_y(0.7),
            Vec3::new(125.0, 30.0, -60.0),
        )
    }

    struct Draw {
        material: EffectMaterial,
        descriptor: Descriptor,
        animated: AnimatedParameters,
        model: Mat4,
        view: EffectView,
        lights: Vec<EffectLight>,
        ambient: Vec3,
        centre: Vec3,
        rects: Vec<[f32; 4]>,
    }

    impl Draw {
        fn of(material: EffectMaterial, elapsed: f32) -> Self {
            Self {
                descriptor: Descriptor::of_shader_name(&material.ps).unwrap(),
                animated: AnimatedParameters::at(&material, elapsed).unwrap(),
                material,
                model: model(),
                view: view(),
                lights: Vec::new(),
                ambient: Vec3::ZERO,
                centre: Vec3::new(125.0, 30.0, -60.0),
                rects: Vec::new(),
            }
        }

        fn flame() -> Self {
            Self::of(flame(), 0.0)
        }

        fn with_bits(mut self, bits: u32) -> Self {
            self.descriptor = Descriptor::new(bits);
            self
        }

        fn constants(&self) -> Result<EffectDrawConstants, ConstantsError> {
            let lighting = EffectLighting {
                lights: &self.lights,
                ambient: self.ambient,
                centre: self.centre,
            };
            draw_constants(&EffectDrawInputs {
                material: &self.material,
                descriptor: self.descriptor,
                animated: &self.animated,
                model: self.model,
                view: &self.view,
                lighting: &lighting,
                subtexture_rects: &self.rects,
            })
        }

        fn registers(&self) -> EffectDrawConstants {
            self.constants().expect("the fixture draw is valid")
        }
    }

    fn gamma_of(value: f32) -> f32 {
        value.powf(2.2)
    }

    fn assert_near(actual: f32, expected: f32, tolerance: f32, context: &str) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{context}: {actual} vs {expected}"
        );
    }

    fn dot_rows(rows: &[Vec4], point: Vec4) -> Vec4 {
        Vec4::new(
            rows[0].dot(point),
            rows[1].dot(point),
            rows[2].dot(point),
            rows.get(3).map_or(0.0, |row| row.dot(point)),
        )
    }

    // --- descriptor -----------------------------------------

    #[test]
    fn descriptor_is_read_from_the_last_eight_hex_digits_of_the_name() {
        assert_eq!(
            Descriptor::of_shader_name("effect_ps_00a07035")
                .unwrap()
                .bits(),
            0x00a0_7035
        );
    }

    #[test]
    fn a_name_without_an_eight_digit_hex_suffix_is_rejected() {
        for name in ["effect_ps_00a0703", "effect_ps_00a0703g", "effect", ""] {
            assert_eq!(
                Descriptor::of_shader_name(name),
                Err(ConstantsError::MalformedShaderName(name.to_owned())),
                "{name}"
            );
        }
    }

    // --- animated parameters --------------------------------

    #[test]
    fn at_rest_parameters_are_the_material_values() {
        let animated = AnimatedParameters::at_rest(&flame());
        assert_eq!(animated.base_color, Vec4::new(0.64, 0.5, 0.25, 0.8));
        assert_eq!(animated.base_color_scale, 2.64);
        assert_eq!(animated.alpha, 0.9);
        assert_eq!(animated.uv_offset, Vec2::splat(0.0024));
        assert_eq!(animated.uv_scale, Vec2::new(1.0, 2.0));
    }

    #[test]
    fn the_emissive_track_overrides_the_base_colour_scale_and_loops() {
        let material = flame();
        let at = |t| {
            AnimatedParameters::at(&material, t)
                .unwrap()
                .base_color_scale
        };
        assert_near(at(0.02), 2.65, 1e-5, "mid key");
        assert_near(at(0.06), 2.65, 1e-5, "one loop later");
    }

    #[test]
    fn the_first_track_of_a_target_wins() {
        let material = flame_with_tracks(json!([
            { "target": "alpha", "cycle": "clamp", "start": 0.0, "stop": 1.0,
              "times": [0.0], "values": [0.25] },
            { "target": "alpha", "cycle": "clamp", "start": 0.0, "stop": 1.0,
              "times": [0.0], "values": [0.75] }
        ]));
        assert_eq!(AnimatedParameters::at(&material, 0.0).unwrap().alpha, 0.25);
    }

    #[test]
    fn a_light_only_target_is_an_error() {
        let material = flame_with_tracks(json!([
            { "target": "dimmer", "cycle": "loop", "start": 0.0, "stop": 1.0,
              "times": [0.0], "values": [1.0] }
        ]));
        assert_eq!(
            AnimatedParameters::at(&material, 0.0),
            Err(ConstantsError::NotAnEffectTarget { target: "dimmer" })
        );
    }

    // --- frame: cb12, ps_cb0, vs_cb0 ------------------------

    fn frame_uniform() -> FrameUniform {
        let mut frame: FrameUniform = bytemuck::Zeroable::zeroed();
        let view = view();
        frame.view_proj = view.view_projection;
        frame.inv_view_proj = view.view_projection.inverse();
        frame.camera_pos = view.camera.extend(1.0);
        frame.fog_near_color = Vec4::new(0.5, 0.5, 0.6, 10.0);
        frame.fog_far_color = Vec4::new(0.7, 0.7, 0.8, 110.0);
        frame.fog_params = Vec4::new(2.0, 0.9, 0.0, 0.0);
        frame
    }

    #[test]
    fn the_view_is_the_frames_camera() {
        let frame = frame_uniform();
        assert_eq!(EffectView::of_frame(&frame, NEAR_PLANE), view());
    }

    #[test]
    fn cb12_is_the_shared_game_frame_block() {
        let frame = frame_uniform();
        assert_eq!(frame_constants(&frame, NEAR_PLANE).cb12, frame_cb12(&frame));
    }

    #[test]
    fn ps_cb0_0_holds_the_depth_linearisation_for_the_engine_near_plane() {
        let constants = frame_constants(&frame_uniform(), NEAR_PLANE);
        // k = 1 - 0.5 / 0.1 = -4.
        assert_eq!(
            constants.ps_cb0[0],
            Vec4::new(0.0, -4.0 * (1.0 + 1e-6), 4.0, 0.0)
        );
        assert!(constants.ps_cb0[1..].iter().all(|row| *row == Vec4::ZERO));
        assert_eq!(constants.vs_cb0, [Vec4::ZERO]);
    }

    #[test]
    fn a_near_plane_of_one_bethesda_unit_switches_to_the_two_unit_reference() {
        let row = frame_constants(&frame_uniform(), 0.1).ps_cb0[0];
        let k = 1.0 - 0.1 / 0.2;
        assert_eq!(row, Vec4::new(0.0, k * (1.0 + 1e-6), -0.5, 0.0));
    }

    /// `scene_depth / s = Y / ((1 - depth) * Z + Yc)` as the pixel stage evaluates it.
    fn linearised(ps_cb0: Vec4, y: f32, depth: f32) -> f32 {
        y / ((1.0 - depth) * ps_cb0.z + ps_cb0.y)
    }

    #[test]
    fn soft_fade_linearisation_yields_view_depth_over_the_fade_distance() {
        let y = Draw::flame().registers().ps_cb1[2].y;
        let ps_cb0 = frame_constants(&frame_uniform(), NEAR_PLANE).ps_cb0[0];
        for view_depth in [0.6, 25.0, 900.0] {
            let depth = NEAR_PLANE / view_depth;
            // The sky offset e = 1e-6 bends far depths slightly: n / (s (depth + e)).
            let expected = NEAR_PLANE / (10.0 * (depth + 1e-6));
            assert_near(
                linearised(ps_cb0, y, depth),
                expected,
                1e-3 * expected,
                &format!("view depth {view_depth}"),
            );
            assert_near(
                expected,
                view_depth / 10.0,
                2e-3 * expected,
                "close to depth / s",
            );
        }
        assert!(linearised(ps_cb0, y, 0.0) > 1e4, "the sky is far");
        // Near-fade reference: 1 BU (0.1 eu) over s.
        assert_near(
            y / (ps_cb0.y * ps_cb0.z + ps_cb0.y),
            0.1 / 10.0,
            1e-6,
            "near reference",
        );
    }

    // --- vs_cb2 ---------------------------------------------

    #[test]
    fn vs_cb2_0_to_3_map_a_model_vertex_to_reverse_z_clip_space() {
        let draw = Draw::flame();
        let vs_cb2 = draw.registers().vs_cb2;
        let local = Vec4::new(0.3, 1.2, -0.4, 1.0);
        let expected = draw.view.view_projection * draw.model * local;
        assert!(dot_rows(&vs_cb2[0..4], local).abs_diff_eq(expected, 1e-3));
    }

    #[test]
    fn vs_cb2_4_to_6_map_a_model_vertex_to_camera_relative_bethesda_units() {
        let draw = Draw::flame();
        let vs_cb2 = draw.registers().vs_cb2;
        let local = Vec3::new(0.3, 1.2, -0.4);
        let world = draw.model.transform_point3(local);
        let expected = position_from_engine(world - draw.view.camera);
        let actual = dot_rows(&vs_cb2[4..7], local.extend(1.0)).truncate();
        assert!(actual.abs_diff_eq(expected, 1e-3), "{actual} vs {expected}");
    }

    #[test]
    fn vs_cb2_13_is_the_camera_in_model_space() {
        let draw = Draw::flame();
        let camera_model = draw.registers().vs_cb2[13];
        assert_eq!(camera_model.w, 1.0);
        assert!(
            draw.model
                .transform_point3(camera_model.truncate())
                .abs_diff_eq(draw.view.camera, 1e-3)
        );
    }

    #[test]
    fn unused_vs_cb2_rows_are_zero() {
        let vs_cb2 = Draw::flame().registers().vs_cb2;
        for row in (7..13).chain(14..VS_CB2_LEN) {
            assert_eq!(vs_cb2[row], Vec4::ZERO, "vs_cb2[{row}]");
        }
    }

    #[test]
    fn a_singular_model_matrix_is_an_error() {
        let mut draw = Draw::flame();
        draw.model = Mat4::from_scale(Vec3::new(1.0, 0.0, 1.0));
        assert_eq!(draw.constants().unwrap_err(), ConstantsError::SingularModel);
    }

    // --- vs_cb1 ---------------------------------------------

    #[test]
    fn vs_cb1_0_is_uv_offset_then_uv_scale() {
        assert_eq!(
            Draw::flame().registers().vs_cb1[0],
            Vec4::new(0.0024, 0.0024, 1.0, 2.0)
        );
    }

    #[test]
    fn uv_tracks_override_the_uv_transform() {
        let material = flame_with_tracks(json!([
            { "target": "u_offset", "cycle": "loop", "start": 0.0, "stop": 2.0,
              "times": [0.0, 2.0], "values": [0.0, 1.0] },
            { "target": "v_scale", "cycle": "clamp", "start": 0.0, "stop": 1.0,
              "times": [0.0], "values": [3.0] }
        ]));
        let draw = Draw::of(material, 0.5);
        assert_eq!(
            draw.registers().vs_cb1[0],
            Vec4::new(0.25, 0.0024, 1.0, 3.0)
        );
    }

    #[test]
    fn vs_cb1_1_is_the_soft_fade_distance_in_engine_units() {
        assert_eq!(
            Draw::flame().registers().vs_cb1[1],
            Vec4::new(10.0, 0.0, 0.0, 0.0)
        );
    }

    #[test]
    fn a_zero_soft_fade_distance_is_raised_to_one_bethesda_unit() {
        let mut draw = Draw::flame();
        draw.material.soft_falloff_depth = 0.0;
        let registers = draw.registers();
        assert_eq!(registers.vs_cb1[1].x, 0.1);
        assert!(registers.ps_cb1[2].y.is_finite());
    }

    #[test]
    fn without_the_soft_bit_the_soft_registers_are_zero() {
        let registers = Draw::flame().with_bits(0x00a0_6035).registers();
        assert_eq!(registers.vs_cb1[1], Vec4::ZERO);
        assert_eq!(registers.ps_cb1[2].y, 0.0);
    }

    #[test]
    fn vs_cb1_2_is_the_falloff_cosines_and_opacities() {
        assert_eq!(
            Draw::flame().registers().vs_cb1[2],
            Vec4::new(0.9, 0.3, 0.2, 1.0)
        );
    }

    #[test]
    fn equal_falloff_cosines_are_separated_so_the_shader_cannot_divide_by_zero() {
        let mut draw = Draw::flame();
        draw.material.falloff.start_angle = 1.0;
        draw.material.falloff.stop_angle = 1.0;
        let falloff = draw.registers().vs_cb1[2];
        assert_eq!(falloff.x, 1.0);
        assert!(falloff.y > falloff.x);
        assert!(((0.5 - falloff.x) / (falloff.y - falloff.x)).is_finite());
    }

    #[test]
    fn without_a_falloff_bit_the_falloff_register_is_neutral() {
        let falloff = Draw::flame().with_bits(0x0000_7025).registers().vs_cb1[2];
        assert_eq!(falloff, Vec4::new(1.0, 1.0 + 1e-5, 1.0, 1.0));
    }

    #[test]
    fn the_colour_falloff_bit_alone_also_fills_the_falloff_register() {
        let falloff = Draw::flame().with_bits(0x00a0_7025).registers().vs_cb1[2];
        assert_eq!(falloff, Vec4::new(0.9, 0.3, 0.2, 1.0));
    }

    // --- vs_cb11 --------------------------------------------

    /// Particle with sub-textures, uv, vertex colours, additive, soft, palette.
    const PARTICLE_BITS: u32 = 0x0000_70ad;

    #[test]
    fn vs_cb11_holds_the_subtexture_rectangles_as_stored() {
        let mut draw = Draw::flame().with_bits(PARTICLE_BITS);
        draw.rects = vec![[0.0, 0.5, 0.0, 0.5], [0.5, 0.5, 0.0, 0.5]];
        let cb11 = draw.registers().vs_cb11;
        assert_eq!(cb11[0], Vec4::new(0.0, 0.5, 0.0, 0.5));
        assert_eq!(cb11[1], Vec4::new(0.5, 0.5, 0.0, 0.5));
        assert!(cb11[2..].iter().all(|row| *row == Vec4::ZERO));
    }

    #[test]
    fn vs_cb11_is_zero_without_the_subtexture_bit() {
        let mut draw = Draw::flame();
        draw.rects = vec![[0.0, 0.5, 0.0, 0.5]];
        assert!(
            draw.registers()
                .vs_cb11
                .iter()
                .all(|row| *row == Vec4::ZERO)
        );
    }

    #[test]
    fn more_subtextures_than_registers_is_an_error() {
        let mut draw = Draw::flame().with_bits(PARTICLE_BITS);
        draw.rects = vec![[0.0, 1.0, 0.0, 1.0]; VS_CB11_LEN + 1];
        assert_eq!(
            draw.constants().unwrap_err(),
            ConstantsError::TooManySubtextures { count: 129 }
        );
    }

    // --- ps_cb1 ---------------------------------------------

    #[test]
    fn palette_permutations_keep_the_base_colour_unscaled_and_put_the_scale_in_ps_cb1_1() {
        let ps_cb1 = Draw::of(flame(), 0.02).registers().ps_cb1;
        let expected = Vec4::new(gamma_of(0.64), gamma_of(0.5), gamma_of(0.25), gamma_of(0.8));
        assert!(ps_cb1[0].abs_diff_eq(expected, 1e-6), "{}", ps_cb1[0]);
        assert_near(ps_cb1[1].x, 2.65, 1e-5, "emissive track value");
    }

    #[test]
    fn other_permutations_fold_the_base_colour_scale_into_the_rgb() {
        let mut draw = Draw::of(flame_with_tracks(json!([])), 0.0).with_bits(0x0080_1035);
        draw.animated.base_color_scale = 2.64;
        let ps_cb1 = draw.registers().ps_cb1;
        let expected = Vec4::new(
            gamma_of(0.64) * 2.64,
            gamma_of(0.5) * 2.64,
            gamma_of(0.25) * 2.64,
            gamma_of(0.8),
        );
        assert!(ps_cb1[0].abs_diff_eq(expected, 1e-5), "{}", ps_cb1[0]);
        assert_eq!(ps_cb1[1].x, 1.0);
    }

    #[test]
    fn the_emissive_colour_track_replaces_the_base_rgb_but_not_its_alpha() {
        let material = flame_with_tracks(json!([
            { "target": "emissive_color", "cycle": "clamp", "start": 0.0, "stop": 1.0,
              "times": [0.0], "values": [[1.0, 0.5, 0.0]] }
        ]));
        let base = Draw::of(material, 0.0).registers().ps_cb1[0];
        assert!(
            base.abs_diff_eq(Vec4::new(1.0, gamma_of(0.5), 0.0, gamma_of(0.8)), 1e-6),
            "{base}"
        );
    }

    #[test]
    fn ps_cb1_2_is_the_lighting_influence_and_the_soft_fade_numerator() {
        let row = Draw::flame().registers().ps_cb1[2];
        assert_eq!(row.x, 0.098);
        // Y = k n / s = -4 * 0.5 / 10.
        assert_near(row.y, -0.2, 1e-6, "numerator");
    }

    #[test]
    fn ps_cb1_3_is_the_environment_scale_only_for_environment_or_distortion() {
        assert_eq!(Draw::flame().registers().ps_cb1[3], Vec4::ZERO);
        for bits in [0x0088_0005, 0x0040_00ad] {
            assert_eq!(
                Draw::flame().with_bits(bits).registers().ps_cb1[3].x,
                0.35,
                "{bits:#x}"
            );
        }
    }

    #[test]
    fn ps_cb1_4_is_zero() {
        assert_eq!(Draw::flame().registers().ps_cb1[4], Vec4::ZERO);
    }

    // --- ps_cb2: lights -------------------------------------

    const LIT_FLAME_BITS: u32 = 0x00a0_7435;
    const AMBIENT: Vec3 = Vec3::new(0.05, 0.06, 0.07);

    fn point_light(position: Vec3, radius: f32, color: Vec3) -> EffectLight {
        EffectLight {
            position,
            radius,
            color,
            spot: None,
        }
    }

    fn lit_draw() -> Draw {
        let mut draw = Draw::flame().with_bits(LIT_FLAME_BITS);
        draw.lights = vec![
            point_light(
                Vec3::new(125.0, 33.0, -60.0),
                20.0,
                Vec3::new(1.0, 0.6, 0.2),
            ),
            point_light(
                Vec3::new(140.0, 30.0, -60.0),
                10.0,
                Vec3::new(0.5, 0.5, 0.5),
            ),
            point_light(
                Vec3::new(100.0, 40.0, -70.0),
                50.0,
                Vec3::new(0.2, 0.3, 0.9),
            ),
            EffectLight {
                position: Vec3::new(125.0, 50.0, -60.0),
                radius: 40.0,
                color: Vec3::splat(2.0),
                spot: Some(SpotCone {
                    direction: Vec3::NEG_Y,
                    cos_half_angle: 0.8,
                    exponent: 3.0,
                }),
            },
        ];
        draw.ambient = AMBIENT;
        draw
    }

    fn light_column(ps_cb2: &[Vec4; PS_CB2_LEN], first: usize, slot: usize) -> Vec3 {
        Vec3::new(
            ps_cb2[first][slot],
            ps_cb2[first + 1][slot],
            ps_cb2[first + 2][slot],
        )
    }

    #[test]
    fn light_positions_are_in_model_space() {
        let draw = lit_draw();
        let ps_cb2 = draw.registers().ps_cb2;
        for (slot, light) in draw.lights.iter().enumerate() {
            let model_position = light_column(&ps_cb2, 0, slot);
            assert!(
                draw.model
                    .transform_point3(model_position)
                    .abs_diff_eq(light.position, 1e-3),
                "light {slot}"
            );
        }
    }

    #[test]
    fn inverse_radius_is_in_model_units() {
        let ps_cb2 = lit_draw().registers().ps_cb2;
        // The model scales by 2: a 20 eu radius is 10 model units.
        assert_near(ps_cb2[8].x, 1.0 / 10.0, 1e-6, "light 0");
        assert_near(ps_cb2[8].y, 1.0 / 5.0, 1e-6, "light 1");
        assert_near(ps_cb2[8].z, 1.0 / 25.0, 1e-6, "light 2");
        assert_near(ps_cb2[8].w, 1.0 / 20.0, 1e-6, "light 3");
    }

    #[test]
    fn light_colours_fill_rows_9_to_11() {
        let draw = lit_draw();
        let ps_cb2 = draw.registers().ps_cb2;
        for (slot, light) in draw.lights.iter().enumerate() {
            assert_eq!(light_column(&ps_cb2, 9, slot), light.color, "light {slot}");
        }
    }

    #[test]
    fn spot_rows_hold_the_model_space_direction_exponent_and_cone_cosine() {
        let draw = lit_draw();
        let ps_cb2 = draw.registers().ps_cb2;
        let direction = light_column(&ps_cb2, 3, 3);
        let expected = draw
            .model
            .inverse()
            .transform_vector3(Vec3::NEG_Y)
            .normalize();
        assert!(direction.abs_diff_eq(expected, 1e-5), "{direction}");
        assert_eq!(ps_cb2[6].w, 3.0);
        assert_eq!(ps_cb2[7].w, 0.8);
        for slot in 0..3 {
            assert_eq!(light_column(&ps_cb2, 3, slot), Vec3::ZERO, "light {slot}");
            assert_eq!(ps_cb2[6][slot], 0.0, "point light {slot} has no cone");
            assert_eq!(ps_cb2[7][slot], 0.0);
        }
    }

    #[test]
    fn a_full_cone_cosine_is_kept_below_one() {
        let mut draw = lit_draw();
        draw.lights[3].spot = Some(SpotCone {
            direction: Vec3::NEG_Y,
            cos_half_angle: 1.0,
            exponent: 1.0,
        });
        assert!(draw.registers().ps_cb2[7].w < 1.0);
    }

    #[test]
    fn unused_light_slots_are_far_away_and_black() {
        let mut draw = lit_draw();
        draw.lights.truncate(2);
        let ps_cb2 = draw.registers().ps_cb2;
        for slot in 2..4 {
            assert_eq!(light_column(&ps_cb2, 9, slot), Vec3::ZERO);
            assert_eq!(ps_cb2[8][slot], 0.0);
            assert!(light_column(&ps_cb2, 0, slot).length() > 1e6);
            assert_eq!(ps_cb2[7][slot], 0.0);
        }
    }

    #[test]
    fn a_light_with_no_radius_is_black() {
        let mut draw = lit_draw();
        draw.lights[0].radius = 0.0;
        let ps_cb2 = draw.registers().ps_cb2;
        assert_eq!(light_column(&ps_cb2, 9, 0), Vec3::ZERO);
        assert_eq!(ps_cb2[8].x, 0.0);
    }

    /// The pixel stage's point-light sum at a model-space point.
    fn shader_point_light_sum(ps_cb2: &[Vec4; PS_CB2_LEN], point: Vec3) -> Vec3 {
        (0..LIGHT_COUNT)
            .map(|slot| {
                let distance = (point - light_column(ps_cb2, 0, slot)).length();
                let ratio = (distance * ps_cb2[8][slot]).clamp(0.0, 1.0);
                light_column(ps_cb2, 9, slot) * (1.0 - ratio * ratio).powf(2.2)
            })
            .fold(ps_cb2[12].truncate(), |sum, light| sum + light)
    }

    #[test]
    fn a_unit_light_at_the_vertex_gives_one_and_none_at_its_radius() {
        let mut draw = Draw::flame().with_bits(LIT_FLAME_BITS);
        let light_position = Vec3::new(125.0, 30.0, -60.0);
        draw.lights = vec![point_light(light_position, 20.0, Vec3::ONE)];
        let ps_cb2 = draw.registers().ps_cb2;
        let to_model = draw.model.inverse();
        let at = |engine: Vec3| shader_point_light_sum(&ps_cb2, to_model.transform_point3(engine));
        assert!(at(light_position).abs_diff_eq(Vec3::ONE, 1e-5));
        assert!(at(light_position + Vec3::new(20.0, 0.0, 0.0)).abs_diff_eq(Vec3::ZERO, 1e-6));
        assert!(at(light_position + Vec3::new(0.0, 0.0, 35.0)).abs_diff_eq(Vec3::ZERO, 1e-6));
    }

    #[test]
    fn more_than_four_lights_is_an_error() {
        let mut draw = lit_draw();
        draw.lights.push(draw.lights[0]);
        assert_eq!(
            draw.constants().unwrap_err(),
            ConstantsError::TooManyLights { count: 5 }
        );
    }

    // --- ps_cb2: ambient, tint, alpha, alpha test -----------

    #[test]
    fn ps_cb2_12_is_the_ambient_colour() {
        assert_eq!(lit_draw().registers().ps_cb2[12], AMBIENT.extend(0.0));
    }

    #[test]
    fn ps_cb2_13_rgb_is_the_ambient_plus_the_lights_at_the_effect_centre() {
        let tint = lit_draw().registers().ps_cb2[13].truncate();
        // Light 0: 3 eu of 20; light 1: 15 eu of 10 (out of range); light 2: sqrt(825) eu of
        // 50; light 3: 20 eu of 40 straight along its cone axis (full cone term).
        let falloff = |distance: f32, radius: f32| (1.0 - (distance / radius).powi(2)).powf(2.2);
        let expected = AMBIENT
            + Vec3::new(1.0, 0.6, 0.2) * falloff(3.0, 20.0)
            + Vec3::new(0.2, 0.3, 0.9) * falloff(825.0_f32.sqrt(), 50.0)
            + Vec3::splat(2.0) * falloff(20.0, 40.0);
        assert!(tint.abs_diff_eq(expected, 1e-5), "{tint} vs {expected}");
    }

    #[test]
    fn a_spot_light_pointing_away_does_not_tint() {
        let mut draw = lit_draw();
        let mut spot = draw.lights[3];
        spot.spot = Some(SpotCone {
            direction: Vec3::Y,
            cos_half_angle: 0.8,
            exponent: 3.0,
        });
        draw.lights = vec![spot];
        assert_eq!(draw.registers().ps_cb2[13].truncate(), AMBIENT);
    }

    #[test]
    fn without_lights_the_tint_is_the_ambient() {
        let mut draw = Draw::flame();
        draw.ambient = Vec3::new(0.3, 0.2, 0.1);
        assert_eq!(
            draw.registers().ps_cb2[13].truncate(),
            Vec3::new(0.3, 0.2, 0.1)
        );
    }

    #[test]
    fn ps_cb2_13_w_is_the_gamma_material_alpha_or_its_track() {
        assert_near(
            Draw::flame().registers().ps_cb2[13].w,
            gamma_of(0.9),
            1e-6,
            "material alpha",
        );
        let material = flame_with_tracks(json!([
            { "target": "alpha", "cycle": "clamp", "start": 0.0, "stop": 1.0,
              "times": [0.0, 1.0], "values": [0.0, 1.0] }
        ]));
        assert_near(
            Draw::of(material, 0.5).registers().ps_cb2[13].w,
            gamma_of(0.5),
            1e-6,
            "track",
        );
    }

    #[test]
    fn ps_cb2_14_is_the_alpha_test_threshold_and_an_open_texture_alpha_cut() {
        assert_eq!(
            Draw::flame().registers().ps_cb2[14],
            Vec4::new(0.5, 1.0, 0.0, 0.0)
        );
        let mut draw = Draw::flame();
        draw.material.blend.alpha_test = false;
        assert_eq!(draw.registers().ps_cb2[14], Vec4::new(0.0, 1.0, 0.0, 0.0));
    }

    #[test]
    fn rim_rows_are_zero() {
        let ps_cb2 = lit_draw().registers().ps_cb2;
        assert_eq!(ps_cb2[15], Vec4::ZERO);
        assert_eq!(ps_cb2[16], Vec4::ZERO);
    }

    #[test]
    fn a_draw_without_lights_has_black_light_slots() {
        let ps_cb2 = Draw::flame().registers().ps_cb2;
        assert!(ps_cb2[9..12].iter().all(|row| *row == Vec4::ZERO));
    }

    // --- layout against the wrapper -------------------------

    fn wrapper_member_offsets(struct_name: &str) -> Vec<(String, u32)> {
        let module =
            naga::front::wgsl::parse_str(crate::rendering::effects::shader::EFFECT_WRAPPER)
                .unwrap();
        module
            .types
            .iter()
            .find_map(|(_, ty)| match (&ty.name, &ty.inner) {
                (Some(name), naga::TypeInner::Struct { members, .. }) if name == struct_name => {
                    Some(
                        members
                            .iter()
                            .map(|m| (m.name.clone().unwrap(), m.offset))
                            .collect(),
                    )
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{struct_name} is not declared in the wrapper"))
    }

    #[test]
    fn the_frame_constants_have_the_wrappers_field_offsets() {
        let rust = [
            ("cb12", std::mem::offset_of!(EffectFrameConstants, cb12)),
            ("vs_cb0", std::mem::offset_of!(EffectFrameConstants, vs_cb0)),
            ("ps_cb0", std::mem::offset_of!(EffectFrameConstants, ps_cb0)),
        ];
        let wrapper = wrapper_member_offsets("EffectFrame");
        let rust: Vec<_> = rust
            .iter()
            .map(|(n, o)| ((*n).to_owned(), *o as u32))
            .collect();
        assert_eq!(rust, wrapper);
    }

    #[test]
    fn the_draw_constants_have_the_wrappers_field_offsets_and_pad_to_the_stride() {
        let rust = [
            ("vs_cb1", std::mem::offset_of!(EffectDrawConstants, vs_cb1)),
            ("vs_cb2", std::mem::offset_of!(EffectDrawConstants, vs_cb2)),
            (
                "vs_cb11",
                std::mem::offset_of!(EffectDrawConstants, vs_cb11),
            ),
            ("ps_cb1", std::mem::offset_of!(EffectDrawConstants, ps_cb1)),
            ("ps_cb2", std::mem::offset_of!(EffectDrawConstants, ps_cb2)),
        ];
        let wrapper = wrapper_member_offsets("EffectDraw");
        let rust: Vec<_> = rust
            .iter()
            .map(|(n, o)| ((*n).to_owned(), *o as u32))
            .collect();
        assert_eq!(rust, wrapper);
        assert_eq!(
            std::mem::offset_of!(EffectDrawConstants, padding) as u64,
            crate::rendering::effects::shader::bindings::DRAW_UNIFORM_SIZE
        );
        assert_eq!(
            DRAW_CONSTANTS_STRIDE,
            crate::rendering::effects::shader::bindings::DRAW_CONSTANTS_STRIDE
        );
    }

    // --- placed lights --------------------------------------

    fn placed(flags: u32) -> GpuPlacedLight {
        let mut light: GpuPlacedLight = bytemuck::Zeroable::zeroed();
        light.position = [1.0, 2.0, 3.0];
        light.radius = 25.0;
        light.color = [0.8, 0.4, 0.1];
        light.flags = flags;
        light.direction = [0.0, -1.0, 0.0];
        light.spot_exponent = 2.0;
        light.spot_cos_outer = 0.7;
        light
    }

    #[test]
    fn a_placed_omni_light_has_no_cone() {
        let light = EffectLight::from_placed(&placed(0));
        assert_eq!(light.position, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(light.radius, 25.0);
        assert_eq!(light.color, Vec3::new(0.8, 0.4, 0.1));
        assert_eq!(light.spot, None);
    }

    #[test]
    fn a_placed_spot_light_keeps_its_cone() {
        let light = EffectLight::from_placed(&placed(LIGHT_FLAG_NON_SHADOW_SPOTLIGHT));
        assert_eq!(
            light.spot,
            Some(SpotCone {
                direction: Vec3::NEG_Y,
                cos_half_angle: 0.7,
                exponent: 2.0,
            })
        );
    }

    #[test]
    fn the_uniform_layout_is_derived_from_the_register_counts() {
        assert_eq!(
            FRAME_UNIFORM_SIZE,
            (CB12_LEN + VS_CB0_LEN + PS_CB0_LEN) as u64 * 16
        );
        assert_eq!(
            DRAW_UNIFORM_SIZE,
            (VS_CB1_LEN + VS_CB2_LEN + VS_CB11_LEN + PS_CB1_LEN + PS_CB2_LEN) as u64 * 16
        );
        assert_eq!(DRAW_CONSTANTS_STRIDE, 3072);
        assert_eq!(
            crate::rendering::effects::shader::bindings::DRAW_CONSTANTS_STRIDE,
            DRAW_CONSTANTS_STRIDE,
            "the shader bindings re-export the one definition"
        );
    }

    #[test]
    fn both_descriptor_parsers_agree_on_every_name() {
        use crate::rendering::effects::shader::EffectDescriptor;
        for (vs, ps) in [
            ("effect_vs_00a00015", "effect_ps_10a06015"),
            ("effect_vs_0000000", "effect_ps_00000005"),
            ("effect_vs_00a0001g", "effect_ps_00000005"),
            ("water_vs_00000001", "effect_ps_00000005"),
            ("effect_vs_00000001", "effect_ps_"),
        ] {
            let stage = EffectDescriptor::from_names(vs, ps);
            let constants = (
                Descriptor::of_shader_name(vs).map(Descriptor::bits),
                Descriptor::of_shader_name(ps).map(Descriptor::bits),
            );
            match (stage, constants) {
                (Ok(pair), (Ok(vs_bits), Ok(ps_bits))) => {
                    assert_eq!((pair.vs, pair.ps), (vs_bits, ps_bits));
                }
                (Err(_), (Err(_), _) | (_, Err(_))) => {}
                (Err(_), _) if !vs.starts_with("effect_vs_") => {}
                (stage, constants) => panic!("{vs} {ps}: {stage:?} vs {constants:?}"),
            }
        }
    }
}
