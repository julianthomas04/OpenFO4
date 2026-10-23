use super::atlas::Tile;
use crate::rendering::shaders;
use crate::rendering::shadows::shadow_maps::SHADOW_MAP_FORMAT;

/// The reverse-Z depth atlas every lamp view renders into, and the pipeline
/// that resets one tile before its view re-renders.
pub struct LampAtlasTexture {
    view: wgpu::TextureView,
    clear_pipeline: wgpu::RenderPipeline,
}

impl LampAtlasTexture {
    pub fn new(device: &wgpu::Device, resolution: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lamp_shadow_atlas"),
            size: wgpu::Extent3d {
                width: resolution,
                height: resolution,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_MAP_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("lamp_shadow_atlas_view"),
            ..Default::default()
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Lamp Shadow Tile Clear Layout"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let shader = shaders::SHADOW_TILE_CLEAR.compile(device);
        let clear_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Lamp Shadow Tile Clear Pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_clear_tile"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: None,
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_MAP_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            view,
            clear_pipeline,
        }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Confines the pass to `tile` and resets its depth; the caster draws
    /// that follow render into it.
    pub fn begin_tile(&self, pass: &mut wgpu::RenderPass<'_>, tile: Tile) {
        let size = tile.size as f32;
        pass.set_viewport(tile.x as f32, tile.y as f32, size, size, 0.0, 1.0);
        pass.set_scissor_rect(tile.x, tile.y, tile.size, tile.size);
        pass.set_pipeline(&self.clear_pipeline);
        pass.draw(0..3, 0..1);
    }
}
