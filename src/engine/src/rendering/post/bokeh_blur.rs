//! The blur passes of the bokeh depth of field, run on two scene-sized targets (A and B):
//!
//! 1. scene + depth -> A: `rgb` is the scene, `a` the blur radius;
//! 2. A -> B: a horizontal gather weighted by the blur radius in `a`;
//! 3. B -> A: two diagonal gathers, their minimum.
//!
//! A then holds the blurred scene the final pass selects from.

use super::dof_support::{
    BLUR_SAMPLER_BINDING, BLUR_SOURCE_BINDING, DEPTH_BINDING, PARAMS_BINDING, SHARP_BINDING,
    SceneInputs, compile_module, draw_fullscreen, extent, fullscreen_pipeline, params_entry,
    unfilterable_float, view_entry,
};
use super::hdr_target::HDR_FORMAT;

/// A scene-sized target the passes alternate between.
struct BlurTarget {
    #[cfg(test)]
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl BlurTarget {
    fn new(device: &wgpu::Device, label: &str, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: extent(width, height),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Self {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            #[cfg(test)]
            texture,
        }
    }
}

pub struct BokehBlur {
    circle_of_confusion: wgpu::RenderPipeline,
    horizontal: wgpu::RenderPipeline,
    diagonals: wgpu::RenderPipeline,
    scene_layout: wgpu::BindGroupLayout,
    blur_layout: wgpu::BindGroupLayout,
    /// Point sampling with clamped coordinates.
    sampler: wgpu::Sampler,
    a: BlurTarget,
    b: BlurTarget,
}

impl BokehBlur {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bokeh_scene_layout"),
            entries: &[
                params_entry(),
                unfilterable_float(SHARP_BINDING),
                unfilterable_float(DEPTH_BINDING),
            ],
        });
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bokeh_blur_layout"),
            entries: &[
                params_entry(),
                unfilterable_float(BLUR_SOURCE_BINDING),
                wgpu::BindGroupLayoutEntry {
                    binding: BLUR_SAMPLER_BINDING,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
        });
        let module = compile_module(device);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bokeh_blur_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        Self {
            circle_of_confusion: fullscreen_pipeline(
                device,
                "bokeh_circle_of_confusion",
                &module,
                &scene_layout,
                "fs_circle_of_confusion",
                HDR_FORMAT,
            ),
            horizontal: fullscreen_pipeline(
                device,
                "bokeh_blur_horizontal",
                &module,
                &blur_layout,
                "fs_blur_horizontal",
                HDR_FORMAT,
            ),
            diagonals: fullscreen_pipeline(
                device,
                "bokeh_blur_diagonals",
                &module,
                &blur_layout,
                "fs_blur_diagonals",
                HDR_FORMAT,
            ),
            scene_layout,
            blur_layout,
            sampler,
            a: BlurTarget::new(device, "bokeh_blur_a", width, height),
            b: BlurTarget::new(device, "bokeh_blur_b", width, height),
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.a = BlurTarget::new(device, "bokeh_blur_a", width, height);
        self.b = BlurTarget::new(device, "bokeh_blur_b", width, height);
    }

    /// The blurred scene `encode` leaves behind (`w` is the blur coverage).
    pub fn blurred(&self) -> &wgpu::TextureView {
        &self.a.view
    }

    #[cfg(test)]
    fn blurred_texture(&self) -> &wgpu::Texture {
        &self.a.texture
    }

    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        inputs: &SceneInputs<'_>,
    ) {
        let scene_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bokeh_scene_bind_group"),
            layout: &self.scene_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: PARAMS_BINDING,
                    resource: inputs.params.as_entire_binding(),
                },
                view_entry(SHARP_BINDING, inputs.sharp),
                view_entry(DEPTH_BINDING, inputs.merged_depth),
            ],
        });
        draw_fullscreen(
            encoder,
            "Bokeh Circle Of Confusion Pass",
            &self.a.view,
            &self.circle_of_confusion,
            &scene_group,
        );
        let blur_group = |source: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bokeh_blur_bind_group"),
                layout: &self.blur_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: PARAMS_BINDING,
                        resource: inputs.params.as_entire_binding(),
                    },
                    view_entry(BLUR_SOURCE_BINDING, source),
                    wgpu::BindGroupEntry {
                        binding: BLUR_SAMPLER_BINDING,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            })
        };
        draw_fullscreen(
            encoder,
            "Bokeh Horizontal Blur Pass",
            &self.b.view,
            &self.horizontal,
            &blur_group(&self.a.view),
        );
        draw_fullscreen(
            encoder,
            "Bokeh Diagonal Blur Pass",
            &self.a.view,
            &self.diagonals,
            &blur_group(&self.b.view),
        );
    }
}

#[cfg(test)]
mod tests;
