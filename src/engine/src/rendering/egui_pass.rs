//! The egui draw sequence shared by the developer menu and the Pip-Boy screen: tessellate, apply the
//! texture deltas, draw into a colour target with `LoadOp::Load`, free dropped textures.

use wgpu::{CommandEncoder, Device, Queue, TextureView};

use crate::rendering::gpu_profiler::{GpuPass, GpuProfiler, SpanPart};

/// Drains the pending uploads so the delta is empty by the time it drops;
/// epaint asserts on dropping unapplied deltas.
fn take_texture_updates(
    delta: &mut egui::TexturesDelta,
) -> Vec<(egui::TextureId, egui::epaint::ImageDelta)> {
    std::mem::take(&mut delta.set)
        .into_iter()
        .flat_map(|(id, deltas)| deltas.into_iter().map(move |delta| (id, delta)))
        .collect()
}

fn take_freed_textures(delta: &mut egui::TexturesDelta) -> Vec<egui::TextureId> {
    std::mem::take(&mut delta.free).into_iter().collect()
}

/// One egui context's renderer. egui-wgpu keys managed textures by id per context, so every
/// context needs a renderer of its own.
pub struct EguiPass {
    renderer: egui_wgpu::Renderer,
}

/// Where and how one frame of egui output is drawn.
pub struct EguiTarget<'a> {
    pub label: &'static str,
    pub view: &'a TextureView,
    pub size_in_pixels: [u32; 2],
    pub pixels_per_point: f32,
    /// The profiler span the pass is timed under.
    pub profiled_as: GpuPass,
}

impl EguiPass {
    pub fn new(device: &Device, surface_format: wgpu::TextureFormat) -> Self {
        let renderer = egui_wgpu::Renderer::new(
            device,
            surface_format,
            egui_wgpu::RendererOptions {
                msaa_samples: 1,
                ..Default::default()
            },
        );
        Self { renderer }
    }

    pub fn draw(
        &mut self,
        device: &Device,
        queue: &Queue,
        encoder: &mut CommandEncoder,
        profiler: &GpuProfiler,
        target: &EguiTarget<'_>,
        ctx: &egui::Context,
        full_output: egui::FullOutput,
    ) {
        let tris = ctx.tessellate(full_output.shapes, target.pixels_per_point);

        let mut textures_delta = full_output.textures_delta;
        for (id, image_delta) in take_texture_updates(&mut textures_delta) {
            self.renderer
                .update_texture(device, queue, id, &image_delta);
        }

        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: target.size_in_pixels,
            pixels_per_point: target.pixels_per_point,
        };

        self.renderer
            .update_buffers(device, queue, encoder, &tris, &screen_descriptor);

        {
            let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(target.label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: profiler.render_writes(target.profiled_as, SpanPart::Whole),
                multiview_mask: None,
            });

            self.renderer.render(
                &mut render_pass.forget_lifetime(),
                &tris,
                &screen_descriptor,
            );
        }

        for id in take_freed_textures(&mut textures_delta) {
            self.renderer.free_texture(&id);
        }
    }

    /// Stores a resource that paint callbacks of this pass read while drawing.
    pub fn insert_callback_resource<T: Send + Sync + 'static>(&mut self, resource: T) {
        self.renderer.callback_resources.insert(resource);
    }

    pub fn register_texture(&mut self, device: &Device, view: &TextureView) -> egui::TextureId {
        self.renderer
            .register_native_texture(device, view, wgpu::FilterMode::Linear)
    }

    /// Registers a texture that is drawn well below its native size: sampled trilinearly, so
    /// fine detail comes from the mip chain instead of aliasing.
    pub fn register_mipmapped_texture(
        &mut self,
        device: &Device,
        view: &TextureView,
    ) -> egui::TextureId {
        self.renderer.register_native_texture_with_sampler_options(
            device,
            view,
            wgpu::SamplerDescriptor {
                label: Some("egui_mipmapped_user_image"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::epaint::ImageDelta;

    fn one_pixel_delta() -> ImageDelta {
        ImageDelta::full(
            egui::ColorImage::filled([1, 1], egui::Color32::WHITE),
            egui::TextureOptions::default(),
        )
    }

    #[test]
    fn draining_applies_every_delta_and_leaves_the_delta_droppable() {
        let mut delta = egui::TexturesDelta::default();
        delta.push(egui::TextureId::Managed(0), one_pixel_delta());
        delta.push(egui::TextureId::Managed(1), one_pixel_delta());
        delta.free(egui::TextureId::Managed(2));

        let updates = take_texture_updates(&mut delta);
        let freed = take_freed_textures(&mut delta);

        assert_eq!(updates.len(), 2);
        assert_eq!(freed, vec![egui::TextureId::Managed(2)]);
        assert!(delta.is_empty());
        drop(delta);
    }
}
