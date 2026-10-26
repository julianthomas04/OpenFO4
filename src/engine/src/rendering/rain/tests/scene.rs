//! A headless frame the rain is drawn in: a target and a depth to draw into, the real frame
//! binding, and the shadow rig's world, whose casters the occlusion map is drawn from.

use std::path::Path;

use bytemuck::Zeroable;
use glam::{Mat4, UVec2, Vec3, Vec4};

use super::{EYE, GENERATION};
use crate::rendering::gpu_test::{f16_bits_to_f32, read_mip};
use crate::rendering::lighting::frame_binding::FrameBinding;
use crate::rendering::lighting::frame_uniform::{FrameUniform, MAX_CASCADES};
use crate::rendering::local_lights::buffers::ClusterBuffers;
use crate::rendering::post::hdr_target::HDR_FORMAT;
use crate::rendering::rain::uniform::Draws;
use crate::rendering::rain::{RainGpu, RainInput, RainRenderer, RainView};
use crate::rendering::shadows::lamps::atlas_texture::LampAtlasTexture;
use crate::rendering::shadows::settings::{ShadowConfig, ShadowFilter};
use crate::rendering::shadows::shadow_maps::ShadowMaps;
use crate::rendering::shadows::test_rig::{ShadowRig, Slab, rig};
use crate::rendering::texture::Texture;

pub(super) const TARGET: u32 = 128;
pub(super) const BACKGROUND: f32 = 0.5;
/// How far a pixel must differ from the background to count as rain.
const LIT: f32 = 0.02;
const FOV_Y: f32 = std::f32::consts::FRAC_PI_3;
/// Where the splash atlas of the rain is looked for below an asset root.
pub(super) const SPLASH_ATLAS: &str = "textures/effects/rainscene/rain_rainsplash_a_16x1_d.dds";
/// Where the streak texture of [`super::downpour`] lies below an asset root: white until a test
/// writes another before its first frame.
pub(super) const STREAK_FILE: &str = "textures/effects/test_rain.dds";

#[derive(Clone, Copy)]
pub(super) struct Look {
    pub eye: Vec3,
    pub forward: Vec3,
}

impl Look {
    pub const AHEAD: Self = Self {
        eye: EYE,
        forward: Vec3::NEG_Z,
    };
}

#[derive(Clone, Copy)]
pub(super) struct Fog {
    pub colour: Vec3,
    pub opacity: f32,
}

pub(super) const NO_FOG: Fog = Fog {
    colour: Vec3::ZERO,
    opacity: 0.0,
};

/// A DDS file of `size` texels below `root`, each texel the `texel` of its (column, row).
pub(super) fn write_texture(
    root: &Path,
    relative: &str,
    size: UVec2,
    texel: impl Fn(UVec2) -> [u8; 4],
) {
    let mut dds = ddsfile::Dds::new_dxgi(ddsfile::NewDxgiParams {
        height: size.y,
        width: size.x,
        depth: None,
        format: ddsfile::DxgiFormat::R8G8B8A8_UNorm,
        mipmap_levels: Some(1),
        array_layers: None,
        caps2: None,
        is_cubemap: false,
        resource_dimension: ddsfile::D3D10ResourceDimension::Texture2D,
        alpha_mode: ddsfile::AlphaMode::Unknown,
    })
    .expect("a valid texture description");
    for (index, bytes) in dds.data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let index = index as u32;
        *bytes = texel(UVec2::new(index % size.x, index / size.x));
    }
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("a file below the root")).unwrap();
    dds.write(&mut std::fs::File::create(path).unwrap())
        .unwrap();
}

fn write_white_texture(root: &Path, relative: &str, size: UVec2) {
    write_texture(root, relative, size, |_| [255; 4]);
}

/// The renderer in a headless frame: a target to draw into, the real frame binding and the
/// world of a shadow rig for the occlusion map.
pub(super) struct Scene {
    pub rig: ShadowRig,
    pub rain: RainRenderer,
    /// The reverse-Z depth of the scene the rain is drawn over: 0 where nothing is.
    pub backdrop_depth: f32,
    pub projection: Mat4,
    /// The streamer generation the world is drawn in.
    pub generation: u64,
    pub underwater: bool,
    pub assets: tempfile::TempDir,
    frame: FrameBinding,
    target: wgpu::Texture,
    depth: wgpu::Texture,
}

fn target_texture(device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rain_test_target"),
        size: wgpu::Extent3d {
            width: TARGET,
            height: TARGET,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn frame_binding(device: &wgpu::Device) -> FrameBinding {
    let config = ShadowConfig {
        resolution: 4,
        distance: 100.0,
        cascade_count: 1,
        split_blend: 0.0,
        filter: ShadowFilter::SingleTap,
        bias_scale: 1.0,
        cascade_min_texels: [0.0; MAX_CASCADES],
        skip_alpha_in_last_cascade: false,
    };
    FrameBinding::new(
        device,
        &FrameUniform::zeroed(),
        &ShadowMaps::new(device, &config),
        LampAtlasTexture::new(device, 4).view(),
        &ClusterBuffers::new(device),
    )
}

impl Scene {
    /// A world of `roofs`, with a splash atlas of white frames.
    pub fn new(test: &str, roofs: &[Slab]) -> Option<Self> {
        Self::with_atlas(test, roofs, |root| {
            write_white_texture(root, SPLASH_ATLAS, UVec2::new(64, 4));
        })
    }

    /// The same with the splash atlas `write_atlas` puts below the asset root.
    pub fn with_atlas(test: &str, roofs: &[Slab], write_atlas: impl FnOnce(&Path)) -> Option<Self> {
        let mut rig = rig(test)?;
        let assets = tempfile::tempdir().unwrap();
        write_white_texture(assets.path(), STREAK_FILE, UVec2::new(4, 4));
        write_atlas(assets.path());
        rig.place(roofs, &[]);
        let frame = frame_binding(&rig.device);
        let rain = RainRenderer::new(
            &rig.device,
            &rig.queue,
            assets.path().to_str().unwrap(),
            &frame.layout,
        );
        Some(Self {
            frame,
            rain,
            target: target_texture(&rig.device, HDR_FORMAT),
            depth: target_texture(&rig.device, Texture::DEPTH_FORMAT),
            backdrop_depth: 0.0,
            projection: Mat4::perspective_infinite_reverse_rh(FOV_Y, 1.0, 0.5),
            generation: GENERATION,
            underwater: false,
            rig,
            assets,
        })
    }

    fn frame_uniform(&self, look: Look, fog: Fog) -> FrameUniform {
        let view = Mat4::look_to_rh(look.eye, look.forward, Vec3::Y);
        FrameUniform {
            view_proj: self.projection * view,
            camera_pos: look.eye.extend(1.0),
            camera_forward: look.forward.extend(0.0),
            fog_near_color: fog.colour.extend(0.0),
            fog_far_color: fog.colour.extend(1.0),
            fog_params: Vec4::new(1.0, fog.opacity, 0.0, 0.0),
            ..FrameUniform::zeroed()
        }
    }

    /// The pixels of a frame that works the rain of `input` out at `time` and draws it over a
    /// cleared scene.
    pub fn draw_at(&mut self, input: &RainInput, look: Look, fog: Fog, time: f64) -> Vec<Vec4> {
        self.frame(Some((input, time)), look, fog)
    }

    pub fn draw(&mut self, input: &RainInput, look: Look) -> Vec<Vec4> {
        self.draw_at(input, look, NO_FOG, 0.0)
    }

    /// The pixels of a frame that draws the rain without working any out.
    pub fn draw_unprepared(&mut self, look: Look) -> Vec<Vec4> {
        self.frame(None, look, NO_FOG)
    }

    /// What the rain has to draw once a frame has worked it out, without drawing it.
    pub fn prepared_draws(&mut self, input: &RainInput, look: Look) -> Draws {
        let mut encoder = self
            .rig
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.prepare(&mut encoder, input, look, 0.0);
        self.rig.queue.submit(std::iter::once(encoder.finish()));
        self.rain.draws
    }

    fn prepare(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        input: &RainInput,
        look: Look,
        time: f64,
    ) {
        let view = RainView {
            eye: look.eye,
            forward: look.forward,
            time,
            world_generation: self.generation,
            underwater: self.underwater,
        };
        self.rain.prepare(
            RainGpu {
                device: &self.rig.device,
                queue: &self.rig.queue,
                encoder,
                shadows: &self.rig.renderer,
                scene: &self.rig.world.scene(),
                profiler: &self.rig.profiler,
            },
            input,
            &view,
        );
    }

    fn frame(&mut self, rain: Option<(&RainInput, f64)>, look: Look, fog: Fog) -> Vec<Vec4> {
        let mut encoder = self
            .rig
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        if let Some((input, time)) = rain {
            self.prepare(&mut encoder, input, look, time);
        }
        let device = &self.rig.device;
        let queue = &self.rig.queue;
        self.frame.write(queue, &self.frame_uniform(look, fog));
        let target = self
            .target
            .create_view(&wgpu::TextureViewDescriptor::default());
        let depth = self
            .depth
            .create_view(&wgpu::TextureViewDescriptor::default());
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rain_test_clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(BACKGROUND),
                        g: f64::from(BACKGROUND),
                        b: f64::from(BACKGROUND),
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(self.backdrop_depth),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }));
        self.rain.encode(
            &mut encoder,
            &target,
            &depth,
            &self.frame,
            &self.rig.profiler,
        );
        queue.submit(std::iter::once(encoder.finish()));
        read_target(device, queue, &self.target)
    }
}

fn read_target(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<Vec4> {
    read_mip(device, queue, texture, 0, UVec2::splat(TARGET), 8)
        .as_chunks::<8>()
        .0
        .iter()
        .map(|texel| {
            let channel = |index: usize| {
                f16_bits_to_f32(u16::from_le_bytes([texel[index * 2], texel[index * 2 + 1]]))
            };
            Vec4::new(channel(0), channel(1), channel(2), channel(3))
        })
        .collect()
}

/// The (column, row) of every pixel the rain changed.
pub(super) fn lit(pixels: &[Vec4]) -> Vec<UVec2> {
    pixels
        .iter()
        .enumerate()
        .filter(|(_, pixel)| is_lit(**pixel))
        .map(|(index, _)| UVec2::new(index as u32 % TARGET, index as u32 / TARGET))
        .collect()
}

pub(super) fn is_lit(pixel: Vec4) -> bool {
    (pixel.truncate() - Vec3::splat(BACKGROUND))
        .abs()
        .max_element()
        > LIT
}

/// A roof `height_above` over `point`, wider than the cull's smallest caster.
pub(super) fn roof_over(point: Vec3, height_above: f32) -> Slab {
    Slab {
        centre: point + Vec3::new(0.0, height_above, 0.0),
        half_size: 100.0,
        alpha_tested: false,
    }
}

pub(super) fn roof(height_above_eye: f32) -> Slab {
    roof_over(EYE, height_above_eye)
}
