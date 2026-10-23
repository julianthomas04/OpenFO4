//! The depth the depth of field passes see: the nearer of the scene depth and the
//! underwater surface depth, in one colour texture the translated passes can load
//! (they cannot bind a depth texture). Both are reverse-Z, so the nearer one is the larger.

use super::dof_support::{
    SCENE_DEPTH_BINDING, SURFACE_DEPTH_BINDING, compile_module, depth_entry, draw_fullscreen,
    extent, fullscreen_pipeline, unfilterable_float, view_entry,
};
use crate::rendering::water::textures::SURFACE_DEPTH_FORMAT;

const MERGED_DEPTH_FORMAT: wgpu::TextureFormat = SURFACE_DEPTH_FORMAT;

struct MergedDepth {
    #[cfg(test)]
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl MergedDepth {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dof_merged_depth"),
            size: extent(width, height),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: MERGED_DEPTH_FORMAT,
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

pub struct DofDepthMerge {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    target: MergedDepth,
}

impl DofDepthMerge {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("dof_depth_merge_layout"),
            entries: &[
                depth_entry(SCENE_DEPTH_BINDING),
                unfilterable_float(SURFACE_DEPTH_BINDING),
            ],
        });
        let pipeline = fullscreen_pipeline(
            device,
            "dof_depth_merge",
            &compile_module(device),
            &layout,
            "fs_merge_depth",
            MERGED_DEPTH_FORMAT,
        );
        Self {
            pipeline,
            layout,
            target: MergedDepth::new(device, width, height),
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.target = MergedDepth::new(device, width, height);
    }

    /// The merged depth `encode` leaves behind.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.target.view
    }

    #[cfg(test)]
    fn texture(&self) -> &wgpu::Texture {
        &self.target.texture
    }

    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene_depth: &wgpu::TextureView,
        surface_depth: &wgpu::TextureView,
    ) {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dof_depth_merge_bind_group"),
            layout: &self.layout,
            entries: &[
                view_entry(SCENE_DEPTH_BINDING, scene_depth),
                view_entry(SURFACE_DEPTH_BINDING, surface_depth),
            ],
        });
        draw_fullscreen(
            encoder,
            "Depth Of Field Depth Merge Pass",
            &self.target.view,
            &self.pipeline,
            &bind_group,
        );
    }
}

#[cfg(test)]
mod tests;
