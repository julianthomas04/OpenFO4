//! The moon: a textured quad standing on the direction of the moon, drawn at infinite depth.
//! One bind group per phase holds that phase texture; a phase whose texture failed to load
//! or is the game stand-in for nothing has none and is not drawn.

use glam::Vec3;

use super::assets::{
    AssetState, PLACEHOLDER_MAX_EDGE, SkyAssetError, load_texture, moon_texture_key, report,
};
use super::layer::{self, LAYER_GROUP, LayerPipeline, finite_or_zero};
use crate::rendering::lighting::time_of_day::unmeasured::MOON_RADIANCE;
use crate::rendering::lighting::time_of_day::{MoonKind, MoonPhase};
use crate::rendering::shaders;

const MOON_VERTEX_COUNT: u32 = 4;
/// Where the up axis of the moon quad points while the moon stands near the zenith or nadir
/// and the world up axis cannot orient it.
const POLAR_UP_FALLBACK: Vec3 = Vec3::NEG_Z;
const POLAR_COS_LIMIT: f32 = 0.999;

pub(super) type PhaseTextures = [Result<wgpu::TextureView, SkyAssetError>; MoonPhase::ALL.len()];
pub(super) type PhaseStates = [AssetState; MoonPhase::ALL.len()];

/// The `iMasserSize` and `iSecundaSize` game settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoonSizes {
    pub masser: f32,
    pub secunda: f32,
}

impl MoonSizes {
    /// The values of the game settings.
    pub const VANILLA: Self = Self {
        masser: 90.0,
        secunda: 75.0,
    };

    pub fn of(&self, kind: MoonKind) -> f32 {
        match kind {
            MoonKind::Masser => self.masser,
            MoonKind::Secunda => self.secunda,
        }
    }
}

/// The unit right and up axes of the moon quad as seen from a camera looking along the unit
/// `direction`, with the world up axis pointing up the picture. Where the moon passes the
/// zenith or nadir the reference axis switches and the quad rolls by a sudden quarter turn.
pub(super) fn moon_basis(direction: Vec3) -> (Vec3, Vec3) {
    let reference = if direction.y.abs() > POLAR_COS_LIMIT {
        POLAR_UP_FALLBACK
    } else {
        Vec3::Y
    };
    let right = direction.cross(reference).normalize();
    (right, right.cross(direction))
}

/// Mirrors `MoonUniform` in `sky_moon.wgsl`.
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct MoonUniform {
    pub direction: [f32; 3],
    /// The half edge of the quad on the tangent plane of the unit sphere.
    pub half_extent: f32,
    pub right: [f32; 3],
    pub visibility: f32,
    pub up: [f32; 3],
    /// The sky radiance scale times the radiance of a lit moon texel.
    pub radiance_scale: f32,
}

impl MoonUniform {
    /// `None` when there is nothing to draw: the direction has no length, the disc has no size
    /// or the moon is invisible.
    pub(super) fn new(
        direction: Vec3,
        diameter_deg: f32,
        visibility: f32,
        radiance_scale: f32,
    ) -> Option<Self> {
        let direction = direction.try_normalize()?;
        let visibility = finite_or_zero(visibility).clamp(0.0, 1.0);
        let diameter_rad = finite_or_zero(diameter_deg).to_radians();
        if visibility <= 0.0 || diameter_rad <= 0.0 {
            return None;
        }
        let (right, up) = moon_basis(direction);
        Some(Self {
            direction: direction.to_array(),
            half_extent: (diameter_rad / 2.0).tan(),
            right: right.to_array(),
            visibility,
            up: up.to_array(),
            radiance_scale: finite_or_zero(radiance_scale).max(0.0) * MOON_RADIANCE,
        })
    }
}

pub(super) struct MoonLayer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
    kind: Option<MoonKind>,
    phases: [Option<wgpu::BindGroup>; MoonPhase::ALL.len()],
    visible_phase: Option<usize>,
}

impl MoonLayer {
    pub(super) fn new(device: &wgpu::Device, frame_layout: &wgpu::BindGroupLayout) -> Self {
        let bind_group_layout = layer::bind_group_layout(device, "night_sky_moon_bgl");
        let pipeline = layer::create_pipeline(
            device,
            &LayerPipeline {
                label: "Night Sky Moon Pipeline",
                shader: &shaders::SKY_MOON,
                frame_layout,
                layer_layout: &bind_group_layout,
                vertex_buffers: &[],
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                color_blend: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
            },
        );
        Self {
            pipeline,
            uniform: layer::uniform_buffer::<MoonUniform>(device, "night_sky_moon_uniform"),
            sampler: layer::sampler(
                device,
                "night_sky_moon_sampler",
                wgpu::AddressMode::ClampToEdge,
            ),
            bind_group_layout,
            kind: None,
            phases: std::array::from_fn(|_| None),
            visible_phase: None,
        }
    }

    /// The moon kind whose textures are installed.
    pub(super) fn kind(&self) -> Option<MoonKind> {
        self.kind
    }

    /// Loads the eight phase textures of `kind` and installs them.
    pub(super) fn load_kind(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        asset_root: &str,
        kind: MoonKind,
    ) -> PhaseStates {
        let textures = MoonPhase::ALL
            .map(|phase| load_texture(device, queue, asset_root, &moon_texture_key(kind, phase)));
        self.install(device, kind, textures)
    }

    /// Makes `textures` the phase textures of `kind`. A failure is logged and leaves its phase
    /// empty; so does a stand-in texture.
    pub(super) fn install(
        &mut self,
        device: &wgpu::Device,
        kind: MoonKind,
        textures: PhaseTextures,
    ) -> PhaseStates {
        self.kind = Some(kind);
        self.visible_phase = None;
        let mut states: PhaseStates = std::array::from_fn(|_| AssetState::NotRequested);
        for (index, texture) in textures.into_iter().enumerate() {
            let (state, bind_group) = match texture {
                Ok(view) if is_placeholder(&view) => (AssetState::Placeholder, None),
                Ok(view) => (AssetState::Loaded, Some(self.bind_group(device, &view))),
                Err(error) => {
                    report(&error);
                    (AssetState::failed(&error), None)
                }
            };
            states[index] = state;
            self.phases[index] = bind_group;
        }
        states
    }

    fn bind_group(&self, device: &wgpu::Device, view: &wgpu::TextureView) -> wgpu::BindGroup {
        layer::bind_group(
            device,
            "night_sky_moon_bind_group",
            &self.bind_group_layout,
            &self.uniform,
            view,
            &self.sampler,
        )
    }

    pub(super) fn hide(&mut self) {
        self.visible_phase = None;
    }

    /// Sets this frame moon: `None`, or a phase without a texture, draws nothing.
    pub(super) fn update(
        &mut self,
        queue: &wgpu::Queue,
        phase: MoonPhase,
        uniform: Option<MoonUniform>,
    ) {
        self.visible_phase = None;
        let Some(uniform) = uniform else { return };
        if self.phases[phase.index()].is_some() {
            queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
            self.visible_phase = Some(phase.index());
        }
    }

    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(bind_group) = self
            .visible_phase
            .and_then(|index| self.phases[index].as_ref())
        else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(LAYER_GROUP, bind_group, &[]);
        pass.draw(0..MOON_VERTEX_COUNT, 0..1);
    }
}

fn is_placeholder(view: &wgpu::TextureView) -> bool {
    let texture = view.texture();
    texture.width().max(texture.height()) < PLACEHOLDER_MAX_EDGE
}
