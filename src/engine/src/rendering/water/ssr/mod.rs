//! The water screen-space reflection chain ("SSLR"). The raytrace, blur
//! and Hi-Z shaders are translated from the installed game's shader archive at install
//! time (`ssr_*` entries of the shader manifest) and driven by the wrappers in
//! `shaders/ssr_*_wrapper.wgsl`; this module owns the targets, constants and passes.
//!
//! Per frame, when a visible water surface needs it:
//!
//! 1. Ray setup (descriptors `0x200` / `0x242` / `0x10200`): every SSR water
//!    surface writes its ray anchor uv and view z at half resolution.
//! 2. Hi-Z: the min pyramid of the view-space linear depth.
//! 3. Raytrace: the Hi-Z march over the scene colour copy -> sharp reflection.
//! 4. Blur horizontally into the scratch target, then vertically -> blurred reflection.
//!
//! The water shader then samples the sharp reflection as `t9` and the blurred one as `t10`.
//!
//! Scene colour: the engine's scene colour copy (`scene_copy.rs`) is the lit HDR scene
//! before the water (opaque world, sky, water depth fog), kept in `Rgba16Float`.
//!
//! Gap, logged once at the first run: surfaces other than water would need a normal
//! buffer and a reflection mask the engine does not render, so their ray-setup pixels
//! stay empty. The chain reflects for water only; the water shader never
//! samples them except through the blur's taps at a water edge.

pub mod constants;
pub mod pipelines;
pub mod targets;

pub use constants::SsrCamera;
pub use pipelines::RaySetupKind;

use std::cell::RefCell;

use glam::UVec2;

use self::{
    constants::{half_extent, linear_depth_constants, raytrace_constants},
    pipelines::{SsrPipelines, binding},
    targets::{SsrTargets, hiz_level_extent},
};
use super::{pipelines::WaterLayouts, textures::ReflectionViews};
use crate::rendering::{
    gpu_profiler::{GpuPass, GpuProfiler, SpanPart},
    scene_copy::SceneCopy,
};
use crate::utils::{env_flag, warn_once};

/// `OPENFO4_WATER_SSR=0` turns the chain off: the water shader then samples the
/// empty stand-ins and reflects the sky gradient only.
pub const SSR_ENV: &str = "OPENFO4_WATER_SSR";

/// Ray setup, two Hi-Z compute passes, raytrace, blur H, blur V.
const PASS_COUNT: usize = 6;
/// Threads per side of the Hi-Z compute groups.
const GROUP_EDGE: u32 = 8;
/// Fullscreen triangle.
const FULLSCREEN_VERTICES: u32 = 3;
/// The level of the Hi-Z pyramid each compute pass' dispatch is sized by:
/// the camera pass owns one thread per level-1 texel, the mip pass one per level-3.
const CAMERA_PASS_LEVEL: u32 = 1;
const MIPS_PASS_LEVEL: u32 = 3;

/// What decides whether a water surface sets up SSR rays.
#[derive(Debug, Copy, Clone)]
pub struct SurfaceSsr {
    pub lod: bool,
    pub wading: bool,
    /// The WATR's Screen Space Reflections flag.
    pub water_flag: bool,
    pub camera_underwater: bool,
    /// The surface's shading pipeline samples `t9` / `t10`; without taps a ray
    /// setup would be pure cost.
    pub shading_samples_ssr: bool,
}

/// The ray setup a surface draws: the water pass selection
/// adds it when the runtime toggle and the WATR flag are set and the camera is
/// not under water: descriptor `0x10200` for LOD water, `0x242` for wading
/// water, `0x200` otherwise. The runtime toggle is [`SSR_ENV`], applied by the
/// caller through [`SsrChain::enabled`].
pub fn ray_setup_kind(surface: &SurfaceSsr) -> Option<RaySetupKind> {
    if !surface.water_flag || surface.camera_underwater || !surface.shading_samples_ssr {
        return None;
    }
    Some(if surface.lod {
        RaySetupKind::Lod
    } else if surface.wading {
        RaySetupKind::Wading
    } else {
        RaySetupKind::Surface
    })
}

struct DepthGroup {
    depth_view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

struct Resources {
    targets: SsrTargets,
    hiz_mips: wgpu::BindGroup,
    raytrace: wgpu::BindGroup,
    blur_h: wgpu::BindGroup,
    blur_v: wgpu::BindGroup,
}

pub struct SsrChain {
    enabled: bool,
    pipelines: SsrPipelines,
    raytrace_cb0: wgpu::Buffer,
    linear_z: wgpu::Buffer,
    linear_clamp: wgpu::Sampler,
    point_clamp: wgpu::Sampler,
    resources: Option<Resources>,
    /// Rebuilt when the live depth view or the targets change, not per frame.
    hiz_camera_group: RefCell<Option<DepthGroup>>,
}

fn vec4_buffer(device: &wgpu::Device, label: &str) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: std::mem::size_of::<glam::Vec4>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn entry(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource }
}

fn view_entry(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    entry(binding, wgpu::BindingResource::TextureView(view))
}

impl SsrChain {
    pub fn new(
        device: &wgpu::Device,
        scene_layout: &wgpu::BindGroupLayout,
        water_layouts: &WaterLayouts,
    ) -> Self {
        let enabled = env_flag(SSR_ENV, true);
        if !enabled {
            tracing::info!("{SSR_ENV}=0: water screen-space reflections are off");
        }
        Self {
            enabled,
            pipelines: SsrPipelines::new(device, scene_layout, water_layouts),
            raytrace_cb0: vec4_buffer(device, "water_ssr_raytrace_cb0"),
            linear_z: vec4_buffer(device, "water_ssr_linear_z"),
            linear_clamp: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("water_ssr_linear_clamp"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            point_clamp: device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("water_ssr_point_clamp"),
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                ..Default::default()
            }),
            resources: None,
            hiz_camera_group: RefCell::new(None),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The targets of the current size; `None` before [`Self::prepare`] ran.
    pub fn targets(&self) -> Option<&SsrTargets> {
        self.resources.as_ref().map(|resources| &resources.targets)
    }

    pub fn ray_setup_pipeline(&self, kind: RaySetupKind) -> &wgpu::RenderPipeline {
        self.pipelines.ray_setup(kind)
    }

    /// Sizes the targets for a `viewport` of the scene, uploads this frame's
    /// constants and returns the textures the water shader samples.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        viewport: UVec2,
        camera: &SsrCamera,
    ) -> ReflectionViews {
        let half = half_extent(viewport.x, viewport.y);
        if self
            .resources
            .as_ref()
            .is_none_or(|r| r.targets.half() != half)
        {
            self.resources = Some(self.create_resources(device, half));
            self.hiz_camera_group.replace(None);
            warn_once("water-ssr-prepass-gap", || {
                "water SSR: ray setup for non-water surfaces needs a normal buffer and an SSR mask this engine does not render; only water surfaces set up rays".to_owned()
            });
            tracing::info!(
                "water SSR: half-resolution targets {}x{}, near {} far {} (Bethesda units; the far plane is the world camera's)",
                half.x,
                half.y,
                camera.near(),
                camera.far(),
            );
        }
        queue.write_buffer(
            &self.raytrace_cb0,
            0,
            bytemuck::bytes_of(&raytrace_constants(half, camera)),
        );
        queue.write_buffer(
            &self.linear_z,
            0,
            bytemuck::bytes_of(&linear_depth_constants(camera)),
        );
        self.resources
            .as_ref()
            .expect("created above")
            .targets
            .reflection_views()
    }

    fn create_resources(&self, device: &wgpu::Device, half: UVec2) -> Resources {
        let targets = SsrTargets::new(device, half);
        let layouts = &self.pipelines.layouts;
        let group = |label, layout, entries: &[wgpu::BindGroupEntry<'_>]| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries,
            })
        };
        let blur_group = |label, source: &wgpu::TextureView| {
            group(
                label,
                &layouts.blur,
                &[
                    entry(binding::BLUR_CB0, self.raytrace_cb0.as_entire_binding()),
                    view_entry(binding::BLUR_SOURCE, source),
                    entry(
                        binding::BLUR_SAMPLER,
                        wgpu::BindingResource::Sampler(&self.linear_clamp),
                    ),
                ],
            )
        };
        Resources {
            hiz_mips: group(
                "water_ssr_hiz_mips_group",
                &layouts.hiz_mips,
                &[
                    view_entry(binding::HIZ_MIP2_IN, &targets.hiz_levels[2]),
                    view_entry(binding::HIZ_MIP3, &targets.hiz_levels[3]),
                    view_entry(binding::HIZ_MIP4, &targets.hiz_levels[4]),
                ],
            ),
            raytrace: group(
                "water_ssr_raytrace_group",
                &layouts.raytrace,
                &[
                    entry(binding::RAYTRACE_CB0, self.raytrace_cb0.as_entire_binding()),
                    view_entry(binding::RAYTRACE_HIZ, &targets.hiz),
                    view_entry(binding::RAYTRACE_ANCHOR, &targets.anchor),
                    view_entry(binding::RAYTRACE_VIEW_Z, &targets.view_z),
                    entry(
                        binding::RAYTRACE_POINT_SAMPLER,
                        wgpu::BindingResource::Sampler(&self.point_clamp),
                    ),
                ],
            ),
            blur_h: blur_group("water_ssr_blur_h_group", &targets.sharp),
            blur_v: blur_group("water_ssr_blur_v_group", &targets.scratch),
            targets,
        }
    }

    fn hiz_camera_group(
        &self,
        device: &wgpu::Device,
        depth: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let resources = self.resources.as_ref().expect("prepare ran");
        let mut cache = self.hiz_camera_group.borrow_mut();
        if let Some(cached) = cache.as_ref()
            && cached.depth_view == *depth
        {
            return cached.bind_group.clone();
        }
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("water_ssr_hiz_camera_group"),
            layout: &self.pipelines.layouts.hiz_camera,
            entries: &[
                view_entry(binding::HIZ_DEPTH, depth),
                entry(binding::HIZ_LINEAR_Z, self.linear_z.as_entire_binding()),
                view_entry(binding::HIZ_MIP0, &resources.targets.hiz_levels[0]),
                view_entry(binding::HIZ_MIP1, &resources.targets.hiz_levels[1]),
                view_entry(binding::HIZ_MIP2, &resources.targets.hiz_levels[2]),
            ],
        });
        *cache = Some(DepthGroup {
            depth_view: depth.clone(),
            bind_group: bind_group.clone(),
        });
        bind_group
    }

    /// Starts the ray-setup pass over the anchor and view-z targets (cleared to 0); the caller draws
    /// the water surfaces into it. `None` before [`Self::prepare`] ran.
    pub fn begin_ray_setup<'e>(
        &self,
        encoder: &'e mut wgpu::CommandEncoder,
        profiler: &GpuProfiler,
    ) -> Option<wgpu::RenderPass<'e>> {
        let targets = &self.resources.as_ref()?.targets;
        let cleared = |view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        Some(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Water SSR Ray Setup"),
            color_attachments: &[cleared(&targets.anchor), cleared(&targets.view_z)],
            depth_stencil_attachment: None,
            timestamp_writes:
                profiler.render_writes(GpuPass::WaterSsr, SpanPart::of(0, PASS_COUNT)),
            occlusion_query_set: None,
            multiview_mask: None,
        }))
    }

    /// Everything after the ray setup: the Hi-Z pyramid of `depth`, the raytrace
    /// over the scene colour copy and the two blurs.
    pub fn encode_resolve(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene: &SceneCopy,
        depth: &wgpu::TextureView,
        profiler: &GpuProfiler,
    ) {
        let Some(resources) = &self.resources else {
            return;
        };
        let half = resources.targets.half();
        let span = |index| SpanPart::of(index, PASS_COUNT);

        let camera_groups = hiz_groups(hiz_level_extent(half, CAMERA_PASS_LEVEL));
        compute_pass(
            encoder,
            "Water SSR Hi-Z Camera",
            self.pipelines.hiz_camera(),
            &self.hiz_camera_group(device, depth),
            camera_groups,
            profiler.compute_writes(GpuPass::WaterSsr, span(1)),
        );
        let mip_groups = hiz_groups(hiz_level_extent(half, MIPS_PASS_LEVEL));
        compute_pass(
            encoder,
            "Water SSR Hi-Z Mips",
            self.pipelines.hiz_mips(),
            &resources.hiz_mips,
            mip_groups,
            profiler.compute_writes(GpuPass::WaterSsr, span(2)),
        );

        fullscreen_pass(
            encoder,
            "Water SSR Raytrace",
            &resources.targets.sharp,
            self.pipelines.raytrace(),
            &[scene.bind_group(), &resources.raytrace],
            profiler.render_writes(GpuPass::WaterSsr, span(3)),
        );
        fullscreen_pass(
            encoder,
            "Water SSR Blur H",
            &resources.targets.scratch,
            self.pipelines.blur_h(),
            &[&resources.blur_h],
            profiler.render_writes(GpuPass::WaterSsr, span(4)),
        );
        fullscreen_pass(
            encoder,
            "Water SSR Blur V",
            &resources.targets.blurred,
            self.pipelines.blur_v(),
            &[&resources.blur_v],
            profiler.render_writes(GpuPass::WaterSsr, span(5)),
        );
    }
}

fn compute_pass(
    encoder: &mut wgpu::CommandEncoder,
    label: &'static str,
    pipeline: &wgpu::ComputePipeline,
    group: &wgpu::BindGroup,
    groups: UVec2,
    timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
) {
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some(label),
        timestamp_writes,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.dispatch_workgroups(groups.x, groups.y, 1);
}

/// Workgroups covering one thread per texel of `extent`.
fn hiz_groups(extent: UVec2) -> UVec2 {
    extent.map(|texels| texels.div_ceil(GROUP_EDGE))
}

fn fullscreen_pass(
    encoder: &mut wgpu::CommandEncoder,
    label: &'static str,
    target: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    groups: &[&wgpu::BindGroup],
    timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    for (index, group) in groups.iter().enumerate() {
        pass.set_bind_group(index as u32, *group, &[]);
    }
    pass.draw(0..FULLSCREEN_VERTICES, 0..1);
}

#[cfg(test)]
pub(crate) mod tests;
