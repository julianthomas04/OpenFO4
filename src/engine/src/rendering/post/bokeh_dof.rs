//! The bokeh depth of field: the scene is copied aside, blurred by the bokeh passes and
//! redrawn from the sharp and the blurred copy by the final pass.

use super::bokeh_blur::BokehBlur;
use super::depth_of_field::DofRegisters;
use super::dof_depth_merge::DofDepthMerge;
use super::dof_resolve::{DofResolve, DofResolvePass};
use super::dof_support::{SceneInputs, extent};
use super::hdr_target::HDR_FORMAT;
use crate::rendering::water::textures::SURFACE_DEPTH_FORMAT;

/// A scene-sized texture the passes sample.
struct SampledTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl SampledTarget {
    fn new(
        device: &wgpu::Device,
        label: &str,
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
        width: u32,
        height: u32,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: extent(width, height),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: usage | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture, view }
    }

    /// The scene as the effect starts: the final pass cannot sample its own target.
    fn scene_copy(device: &wgpu::Device, width: u32, height: u32) -> Self {
        Self::new(
            device,
            "bokeh_dof_sharp",
            HDR_FORMAT,
            wgpu::TextureUsages::COPY_DST,
            width,
            height,
        )
    }

    /// Stays zero: the surface depth of a frame without an underwater surface.
    fn no_surface(device: &wgpu::Device, width: u32, height: u32) -> Self {
        Self::new(
            device,
            "bokeh_dof_no_surface",
            SURFACE_DEPTH_FORMAT,
            wgpu::TextureUsages::empty(),
            width,
            height,
        )
    }
}

pub struct BokehDepthOfField {
    params: wgpu::Buffer,
    sharp: SampledTarget,
    no_surface: SampledTarget,
    depth_merge: DofDepthMerge,
    blur: BokehBlur,
    resolve: DofResolvePass,
}

impl BokehDepthOfField {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bokeh_dof_params"),
            size: std::mem::size_of::<DofRegisters>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            params,
            sharp: SampledTarget::scene_copy(device, width, height),
            no_surface: SampledTarget::no_surface(device, width, height),
            depth_merge: DofDepthMerge::new(device, width, height),
            blur: BokehBlur::new(device, width, height),
            resolve: DofResolvePass::new(device),
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.sharp = SampledTarget::scene_copy(device, width, height);
        self.no_surface = SampledTarget::no_surface(device, width, height);
        self.depth_merge.resize(device, width, height);
        self.blur.resize(device, width, height);
    }

    /// Runs the effect over the finished scene `hdr`. `depth` is the live scene
    /// depth, `surface_depth` the underwater surface depth (`None` where there is none).
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        hdr: &wgpu::Texture,
        hdr_view: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        surface_depth: Option<&wgpu::TextureView>,
        registers: &DofRegisters,
        resolve: DofResolve,
    ) {
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(registers));
        encoder.copy_texture_to_texture(
            hdr.as_image_copy(),
            self.sharp.texture.as_image_copy(),
            self.sharp.texture.size(),
        );
        self.depth_merge.encode(
            device,
            encoder,
            depth,
            surface_depth.unwrap_or(&self.no_surface.view),
        );
        let inputs = SceneInputs {
            params: &self.params,
            sharp: &self.sharp.view,
            merged_depth: self.depth_merge.view(),
        };
        self.blur.encode(device, encoder, &inputs);
        self.resolve.encode(
            device,
            encoder,
            hdr_view,
            self.blur.blurred(),
            &inputs,
            resolve,
        );
    }
}

#[cfg(test)]
mod tests;
