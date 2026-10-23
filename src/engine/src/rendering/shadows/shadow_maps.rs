use super::settings::ShadowConfig;

pub const SHADOW_MAP_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// One reverse-Z depth layer per cascade, rendered by the shadow passes and
/// sampled by receivers through the frame bind group.
pub struct ShadowMaps {
    layer_views: Vec<wgpu::TextureView>,
    array_view: wgpu::TextureView,
    comparison_sampler: wgpu::Sampler,
}

impl ShadowMaps {
    pub fn new(device: &wgpu::Device, config: &ShadowConfig) -> Self {
        let layers = config.cascade_count as u32;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sun_shadow_maps"),
            size: wgpu::Extent3d {
                width: config.resolution,
                height: config.resolution,
                depth_or_array_layers: layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_MAP_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let layer_views = (0..layers)
            .map(|layer| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("sun_shadow_map_layer"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();

        let array_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("sun_shadow_map_array"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        // Shared by the sun maps and the lamp atlas. Reverse-Z: a receiver is
        // lit when it is at least as close to the light as the stored caster
        // depth.
        let comparison_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow_comparison_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::GreaterEqual),
            ..Default::default()
        });

        Self {
            layer_views,
            array_view,
            comparison_sampler,
        }
    }

    pub fn layer_view(&self, cascade: usize) -> &wgpu::TextureView {
        &self.layer_views[cascade]
    }

    pub fn array_view(&self) -> &wgpu::TextureView {
        &self.array_view
    }

    pub fn comparison_sampler(&self) -> &wgpu::Sampler {
        &self.comparison_sampler
    }
}
