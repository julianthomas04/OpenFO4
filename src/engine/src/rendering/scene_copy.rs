//! Copies of the opaque scene that translucent passes sample.
//!
//! A pass cannot sample its own attachments, so after the opaque world the
//! HDR colour and the depth buffer are copied into textures a later pass binds
//! (refraction reads the colour, depth fog the depth).
//!
//! Cost per frame at `WxH`: the colour copy reads and writes 8 B/px
//! (`Rgba16Float`), the depth copy 4 B/px (`Depth32Float`); 1920x1080 moves about
//! 25 MB each way, 3840x2160 about 100 MB. `copy_texture_to_texture` is a
//! bandwidth-bound blit with no shader work, the cheapest correct option: a
//! resolve needs multisampled sources and a fullscreen blit adds shader cost.

use std::cell::RefCell;

use super::post::hdr_target::HDR_FORMAT;
use super::texture::Texture;
use crate::utils::env_flag;

const FORCE_COPY_ENV: &str = "OPENFO4_WATER_SCENE_COPY";

const COLOR_BINDING: u32 = 0;
const DEPTH_BINDING: u32 = 1;
const SAMPLER_BINDING: u32 = 2;
const DEPTH_SAMPLER_BINDING: u32 = 3;

fn extent(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: width.max(1),
        height: height.max(1),
        depth_or_array_layers: 1,
    }
}

fn copy_target(
    device: &wgpu::Device,
    label: &'static str,
    size: wgpu::Extent3d,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn layout_entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty,
        count: None,
    }
}

fn texture_binding(sample_type: wgpu::TextureSampleType) -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type,
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    }
}

/// The bind group of the live depth buffer, kept while that buffer's view stays
/// the same.
struct LiveDepthGroup {
    depth_view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

struct CopyTargets {
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    depth: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

/// The colour copy's linear sampler and the point sampler of the depth reads.
struct CopySamplers {
    color: wgpu::Sampler,
    depth: wgpu::Sampler,
}

pub struct SceneCopy {
    targets: CopyTargets,
    size: wgpu::Extent3d,
    layout: wgpu::BindGroupLayout,
    samplers: CopySamplers,
    /// Rebuilt when the live depth view or the copy size changes, not per frame.
    live_depth_group: RefCell<Option<LiveDepthGroup>>,
    force: bool,
}

impl SceneCopy {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene_copy_layout"),
            entries: &[
                layout_entry(
                    COLOR_BINDING,
                    texture_binding(wgpu::TextureSampleType::Float { filterable: true }),
                ),
                layout_entry(
                    DEPTH_BINDING,
                    texture_binding(wgpu::TextureSampleType::Depth),
                ),
                layout_entry(
                    SAMPLER_BINDING,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                layout_entry(
                    DEPTH_SAMPLER_BINDING,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                ),
            ],
        });
        let color_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("scene_copy_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let depth_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("scene_copy_depth_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let samplers = CopySamplers {
            color: color_sampler,
            depth: depth_sampler,
        };
        let size = extent(width, height);
        let targets = Self::create_targets(device, &layout, &samplers, size);
        Self {
            targets,
            size,
            layout,
            samplers,
            live_depth_group: RefCell::new(None),
            force: env_flag(FORCE_COPY_ENV, false),
        }
    }

    fn create_targets(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        samplers: &CopySamplers,
        size: wgpu::Extent3d,
    ) -> CopyTargets {
        let color = copy_target(device, "scene_color", size, HDR_FORMAT);
        let depth = copy_target(device, "scene_depth", size, Texture::DEPTH_FORMAT);
        let color_view = color.create_view(&wgpu::TextureViewDescriptor::default());
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor {
            aspect: wgpu::TextureAspect::DepthOnly,
            ..Default::default()
        });
        let bind_group = Self::make_bind_group(
            device,
            layout,
            samplers,
            "scene_copy_bind_group",
            &color_view,
            &depth_view,
        );
        CopyTargets {
            color,
            color_view,
            depth,
            bind_group,
        }
    }

    fn make_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        samplers: &CopySamplers,
        label: &str,
        color_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: COLOR_BINDING,
                    resource: wgpu::BindingResource::TextureView(color_view),
                },
                wgpu::BindGroupEntry {
                    binding: DEPTH_BINDING,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: SAMPLER_BINDING,
                    resource: wgpu::BindingResource::Sampler(&samplers.color),
                },
                wgpu::BindGroupEntry {
                    binding: DEPTH_SAMPLER_BINDING,
                    resource: wgpu::BindingResource::Sampler(&samplers.depth),
                },
            ],
        })
    }

    /// The same layout with the live depth buffer in place of the depth copy,
    /// for a pass that holds the depth attachment read-only and samples it
    /// (the water depth-fog pass runs before the copy exists). The colour
    /// binding is this copy's colour, which that pass does not read. The bind
    /// group is cached until the live depth view or the copy size changes.
    pub fn bind_group_with_live_depth(
        &self,
        device: &wgpu::Device,
        live_depth: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let mut cache = self.live_depth_group.borrow_mut();
        if let Some(cached) = cache.as_ref()
            && cached.depth_view == *live_depth
        {
            return cached.bind_group.clone();
        }
        let bind_group = Self::make_bind_group(
            device,
            &self.layout,
            &self.samplers,
            "scene_copy_live_depth_bind_group",
            &self.targets.color_view,
            live_depth,
        );
        *cache = Some(LiveDepthGroup {
            depth_view: live_depth.clone(),
            bind_group: bind_group.clone(),
        });
        bind_group
    }

    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.size = extent(width, height);
        self.targets = Self::create_targets(device, &self.layout, &self.samplers, self.size);
        self.live_depth_group.replace(None);
    }

    /// Layout of [`Self::bind_group`]: scene colour (binding 0, filterable
    /// float), scene depth (binding 1, reverse-Z `texture_depth_2d`, read it
    /// with `textureLoad`) and a linear clamp sampler for the colour
    /// (binding 2).
    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.targets.bind_group
    }

    /// Whether to copy this frame: when a stage consumes the copy, or when
    /// `OPENFO4_WATER_SCENE_COPY` forces it to measure its cost.
    pub fn wanted(&self, stage_has_work: bool) -> bool {
        stage_has_work || self.force
    }

    /// The sources need `COPY_SRC` and this target's size; encode between the
    /// two world render passes.
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        hdr: &wgpu::Texture,
        depth: &wgpu::Texture,
    ) {
        encoder.copy_texture_to_texture(
            hdr.as_image_copy(),
            self.targets.color.as_image_copy(),
            self.size,
        );
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                aspect: wgpu::TextureAspect::DepthOnly,
                ..depth.as_image_copy()
            },
            wgpu::TexelCopyTextureInfo {
                aspect: wgpu::TextureAspect::DepthOnly,
                ..self.targets.depth.as_image_copy()
            },
            self.size,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::gpu_test::test_device;

    fn depth_view(device: &wgpu::Device) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("scene_copy_test_depth"),
                size: extent(4, 4),
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: Texture::DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    #[test]
    fn the_live_depth_bind_group_is_reused_until_the_view_or_the_size_changes() {
        let Some((device, _queue)) = test_device("scene copy live depth cache") else {
            return;
        };
        let mut scene = SceneCopy::new(&device, 4, 4);
        let (first_depth, second_depth) = (depth_view(&device), depth_view(&device));
        let first = scene.bind_group_with_live_depth(&device, &first_depth);
        assert_eq!(
            scene.bind_group_with_live_depth(&device, &first_depth),
            first
        );
        let second = scene.bind_group_with_live_depth(&device, &second_depth);
        assert_ne!(second, first, "another depth view needs another group");
        scene.resize(&device, 8, 8);
        assert_ne!(
            scene.bind_group_with_live_depth(&device, &second_depth),
            second
        );
    }
}
