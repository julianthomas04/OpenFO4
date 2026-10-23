//! The final pass of the bokeh depth of field: it selects between the sharp scene and
//! the blurred one and, while the camera's near plane is fully under water, applies the
//! underwater fog on top.

use super::dof_support::{
    BLURRED_BINDING, DEPTH_BINDING, PARAMS_BINDING, SHARP_BINDING, SceneInputs, compile_module,
    draw_fullscreen, fullscreen_pipeline, params_entry, unfilterable_float, view_entry,
};
use super::hdr_target::HDR_FORMAT;

/// Which final pass runs.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DofResolve {
    Plain,
    Fogged,
}

pub struct DofResolvePass {
    plain: wgpu::RenderPipeline,
    fogged: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl DofResolvePass {
    pub fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("dof_resolve_layout"),
            entries: &[
                params_entry(),
                unfilterable_float(SHARP_BINDING),
                unfilterable_float(BLURRED_BINDING),
                unfilterable_float(DEPTH_BINDING),
            ],
        });
        let module = compile_module(device);
        let pipeline =
            |label, entry| fullscreen_pipeline(device, label, &module, &layout, entry, HDR_FORMAT);
        Self {
            plain: pipeline("dof_resolve_plain", "fs_resolve_plain"),
            fogged: pipeline("dof_resolve_fogged", "fs_resolve_fogged"),
            layout,
        }
    }

    /// Redraws `target` from the scene copy and the blurred scene.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        blurred: &wgpu::TextureView,
        inputs: &SceneInputs<'_>,
        variant: DofResolve,
    ) {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("dof_resolve_bind_group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: PARAMS_BINDING,
                    resource: inputs.params.as_entire_binding(),
                },
                view_entry(SHARP_BINDING, inputs.sharp),
                view_entry(BLURRED_BINDING, blurred),
                view_entry(DEPTH_BINDING, inputs.merged_depth),
            ],
        });
        let (label, pipeline) = match variant {
            DofResolve::Plain => ("Depth Of Field Resolve Pass", &self.plain),
            DofResolve::Fogged => ("Underwater Fog Pass", &self.fogged),
        };
        draw_fullscreen(encoder, label, target, pipeline, &bind_group);
    }
}

#[cfg(test)]
mod tests;
