//! The sun: two textured quads standing on the direction of the sun, drawn at infinite depth and
//! added over the sky gradient, the disc sprite and the larger glare sprite behind it. Each has
//! its own texture, size and colour; one without a texture is not drawn.

use glam::Vec3;

use super::assets::{AssetState, SkyAssetError, report};
use super::layer::{self, LAYER_GROUP, LayerPipeline, WorldGroupLayouts};
use super::quad::quad_basis;
use crate::assets::ini::{GameIni, IniError, IniKey};
use crate::rendering::lighting::sky_measured::{SUN_DISC_BRIGHTNESS, sun_disc_edge_deg};
use crate::rendering::lighting::time_of_day::unmeasured::{
    SUN_GLARE_RADIANCE, SUN_HORIZON_FADE_SINE, sun_glare_edge_deg,
};
use crate::rendering::shaders;
use crate::utils::finite_or_zero;

const SUN_VERTEX_COUNT: u32 = 4;

const BASE_SIZE: IniKey = IniKey::new("Weather", "fSunBaseSize");
const GLARE_SIZE: IniKey = IniKey::new("Weather", "fSunGlareSize");

/// The `fSunBaseSize` and `fSunGlareSize` game settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SunSizes {
    pub base: f32,
    pub glare: f32,
}

impl SunSizes {
    /// The values the game uses when its INIs leave the settings out, as they do in vanilla.
    pub const VANILLA: Self = Self {
        base: 425.0,
        glare: 600.0,
    };

    /// A setting no INI sets takes its [`Self::VANILLA`] value, with a note in the log; one that
    /// is set must be a finite number above zero.
    pub fn from_ini(ini: &GameIni) -> Result<Self, IniError> {
        Ok(Self {
            base: size_or(ini, BASE_SIZE, Self::VANILLA.base)?,
            glare: size_or(ini, GLARE_SIZE, Self::VANILLA.glare)?,
        })
    }
}

fn size_or(ini: &GameIni, key: IniKey, vanilla: f32) -> Result<f32, IniError> {
    Ok(ini.optional_positive(key)?.unwrap_or_else(|| {
        tracing::info!(
            "{} is not set in any game INI; the sun sprite takes the vanilla default {vanilla}",
            key.name
        );
        vanilla
    }))
}

/// The sun of the frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SunInput {
    /// Unit vector in engine space, towards the sun.
    pub direction: Vec3,
    /// 0 to 1: the sun's alpha around sunrise and sunset.
    pub alpha: f32,
    /// The weather's Sun colour, linear.
    pub disc_color: Vec3,
    /// The weather's `SunGlare` colour, linear.
    pub glare_color: Vec3,
}

/// Mirrors `SunUniform` in `sky_sun.wgsl`.
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct SunUniform {
    pub direction: [f32; 3],
    /// The half edge of the quad on the tangent plane of the unit sphere.
    pub half_extent: f32,
    pub right: [f32; 3],
    pub alpha: f32,
    pub up: [f32; 3],
    /// The sine of the elevation at which the sprite is fully faded in.
    pub horizon_fade_sine: f32,
    /// The radiance of a fully opaque texel.
    pub radiance: [f32; 3],
    pub padding: f32,
}

impl SunUniform {
    /// `None` when there is nothing to draw: the direction has no length, the sprite has no
    /// size, the sun has no alpha or the radiance is zero.
    pub(super) fn new(direction: Vec3, edge_deg: f32, alpha: f32, radiance: Vec3) -> Option<Self> {
        let direction = direction.try_normalize()?;
        let alpha = finite_or_zero(alpha).clamp(0.0, 1.0);
        let edge_rad = finite_or_zero(edge_deg).to_radians();
        let radiance = radiance
            .to_array()
            .map(|part| finite_or_zero(part).max(0.0));
        if alpha <= 0.0 || edge_rad <= 0.0 || radiance.iter().all(|&part| part <= 0.0) {
            return None;
        }
        let (right, up) = quad_basis(direction);
        Some(Self {
            direction: direction.to_array(),
            half_extent: (edge_rad / 2.0).tan(),
            right: right.to_array(),
            alpha,
            up: up.to_array(),
            horizon_fade_sine: SUN_HORIZON_FADE_SINE,
            radiance,
            padding: 0.0,
        })
    }
}

/// What each sprite of the sun draws this frame; `None` draws nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct SunUniforms {
    pub disc: Option<SunUniform>,
    pub glare: Option<SunUniform>,
}

impl SunUniforms {
    /// `radiance_scale` is the sky radiance scale the sky colours are multiplied by.
    pub(super) fn new(sun: &SunInput, sizes: SunSizes, radiance_scale: f32) -> Self {
        let scale = finite_or_zero(radiance_scale).max(0.0);
        let sprite = |edge_deg: f32, color: Vec3, brightness: f32| {
            SunUniform::new(
                sun.direction,
                edge_deg,
                sun.alpha,
                color * (scale * brightness),
            )
        };
        Self {
            disc: sprite(
                sun_disc_edge_deg(sizes.base),
                sun.disc_color,
                SUN_DISC_BRIGHTNESS,
            ),
            glare: sprite(
                sun_glare_edge_deg(sizes.glare),
                sun.glare_color,
                SUN_GLARE_RADIANCE,
            ),
        }
    }
}

struct SunSprite {
    uniform: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
    visible: bool,
}

impl SunSprite {
    fn new(device: &wgpu::Device, label: &str) -> Self {
        Self {
            uniform: layer::uniform_buffer::<SunUniform>(device, label),
            bind_group: None,
            visible: false,
        }
    }

    /// `None` is a climate that names no texture: nothing to draw, nothing failed.
    fn install(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        texture: Option<Result<wgpu::TextureView, SkyAssetError>>,
    ) -> AssetState {
        self.visible = false;
        match texture {
            None => {
                self.bind_group = None;
                AssetState::NotRequested
            }
            Some(Ok(view)) => {
                self.bind_group = Some(layer::bind_group(
                    device,
                    "night_sky_sun_bind_group",
                    layout,
                    &self.uniform,
                    &view,
                    sampler,
                ));
                AssetState::Loaded
            }
            Some(Err(error)) => {
                report(&error);
                self.bind_group = None;
                AssetState::failed(&error)
            }
        }
    }

    fn update(&mut self, queue: &wgpu::Queue, uniform: Option<SunUniform>) {
        self.visible = false;
        let Some(uniform) = uniform else { return };
        if self.bind_group.is_some() {
            queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
            self.visible = true;
        }
    }

    fn bind_group(&self) -> Option<&wgpu::BindGroup> {
        self.bind_group.as_ref().filter(|_| self.visible)
    }
}

pub(super) struct SunLayer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    disc: SunSprite,
    glare: SunSprite,
}

impl SunLayer {
    pub(super) fn new(device: &wgpu::Device, world_groups: WorldGroupLayouts<'_>) -> Self {
        let bind_group_layout = layer::bind_group_layout(device, "night_sky_sun_bgl");
        let pipeline = layer::create_pipeline(
            device,
            &LayerPipeline {
                label: "Night Sky Sun Pipeline",
                shader: &shaders::SKY_SUN,
                world_groups,
                layer_layout: &bind_group_layout,
                vertex_buffers: &[],
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                color_blend: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            },
        );
        Self {
            pipeline,
            sampler: layer::sampler(
                device,
                "night_sky_sun_sampler",
                wgpu::AddressMode::ClampToEdge,
            ),
            bind_group_layout,
            disc: SunSprite::new(device, "night_sky_sun_disc_uniform"),
            glare: SunSprite::new(device, "night_sky_sun_glare_uniform"),
        }
    }

    /// Makes the textures the sun's: the disc texture's state, then the glare texture's.
    pub(super) fn install(
        &mut self,
        device: &wgpu::Device,
        disc: Option<Result<wgpu::TextureView, SkyAssetError>>,
        glare: Option<Result<wgpu::TextureView, SkyAssetError>>,
    ) -> [AssetState; 2] {
        let (layout, sampler) = (&self.bind_group_layout, &self.sampler);
        [
            self.disc.install(device, layout, sampler, disc),
            self.glare.install(device, layout, sampler, glare),
        ]
    }

    pub(super) fn update(&mut self, queue: &wgpu::Queue, uniforms: &SunUniforms) {
        self.disc.update(queue, uniforms.disc);
        self.glare.update(queue, uniforms.glare);
    }

    /// The glare first, then the disc; both add to the sky, so the order is not visible.
    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let sprites = [self.glare.bind_group(), self.disc.bind_group()];
        if sprites.iter().all(Option::is_none) {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        for bind_group in sprites.into_iter().flatten() {
            pass.set_bind_group(LAYER_GROUP, bind_group, &[]);
            pass.draw(0..SUN_VERTEX_COUNT, 0..1);
        }
    }
}
