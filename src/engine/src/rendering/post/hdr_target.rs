pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

pub fn clear_color(linear_rgb: glam::Vec3) -> wgpu::Color {
    wgpu::Color {
        r: f64::from(linear_rgb.x),
        g: f64::from(linear_rgb.y),
        b: f64::from(linear_rgb.z),
        a: 1.0,
    }
}

pub struct HdrTarget {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

impl HdrTarget {
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr_target"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        Self { texture, view }
    }
}
