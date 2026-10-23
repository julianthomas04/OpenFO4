use crate::rendering::{gpu_driven::DRAW_BUCKET_COUNT, shaders, texture::Texture, vertex::Vertex};

pub struct ObjectPipelines {
    pub opaque_culled: wgpu::RenderPipeline,
    pub opaque_twosided: wgpu::RenderPipeline,
    pub alphatest_culled: wgpu::RenderPipeline,
    pub alphatest_twosided: wgpu::RenderPipeline,
    pub decal_alphatest_culled: wgpu::RenderPipeline,
    pub decal_alphatest_twosided: wgpu::RenderPipeline,
    pub blend_culled: wgpu::RenderPipeline,
    pub blend_twosided: wgpu::RenderPipeline,
    pub decal_blend_culled: wgpu::RenderPipeline,
    pub decal_blend_twosided: wgpu::RenderPipeline,
    pub multiply_culled: wgpu::RenderPipeline,
    pub multiply_twosided: wgpu::RenderPipeline,
}

/// One blend / cull / depth configuration of the object fragment shader.
#[derive(Clone, Copy)]
pub struct SurfaceVariant {
    pub label: &'static str,
    pub cull_mode: Option<wgpu::Face>,
    pub blend: wgpu::BlendState,
    pub depth_write: bool,
    pub fs_entry: &'static str,
    pub bias: wgpu::DepthBiasState,
}

impl SurfaceVariant {
    pub fn opaque(two_sided: bool) -> Self {
        Self {
            label: if two_sided {
                "Pipeline Opaque Two-Sided"
            } else {
                "Pipeline Opaque Culled"
            },
            cull_mode: cull_mode(two_sided),
            blend: wgpu::BlendState::REPLACE,
            depth_write: true,
            fs_entry: "fs_opaque",
            bias: wgpu::DepthBiasState::default(),
        }
    }

    pub fn alphatest(two_sided: bool) -> Self {
        Self {
            label: if two_sided {
                "Pipeline AlphaTest Two-Sided"
            } else {
                "Pipeline AlphaTest Culled"
            },
            fs_entry: "fs_alphatest",
            ..Self::opaque(two_sided)
        }
    }

    pub fn blend(two_sided: bool) -> Self {
        Self {
            label: if two_sided {
                "Pipeline Blend Two-Sided"
            } else {
                "Pipeline Blend Culled"
            },
            blend: PREMULTIPLIED_BLEND,
            depth_write: false,
            fs_entry: "fs_blend",
            ..Self::alphatest(two_sided)
        }
    }

    pub fn multiply(two_sided: bool) -> Self {
        Self {
            label: if two_sided {
                "Pipeline Multiply Two-Sided"
            } else {
                "Pipeline Multiply Culled"
            },
            blend: MULTIPLY_BLEND,
            fs_entry: "fs_multiply",
            ..Self::blend(two_sided)
        }
    }

    /// Decals gain a forward depth bias so they win their coplanar host. They keep the
    /// depth write of the surface they derive from: alpha-tested decals (rock trims
    /// without a host behind them) must be in the depth buffer or the water draws over
    /// them wherever its surface is nearer than the seabed; blended decals never write.
    fn decal(self, label: &'static str) -> Self {
        Self {
            label,
            bias: DECAL_BIAS,
            ..self
        }
    }
}

/// The fragment shader outputs colour already multiplied by alpha, so alpha, additive
/// (output alpha 0) and lit-plus-reflection surfaces all share one blend equation.
const PREMULTIPLIED_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
};

/// Destination colour scaled by the fragment colour; destination alpha is kept.
const MULTIPLY_BLEND: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Zero,
        dst_factor: wgpu::BlendFactor::Src,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::Zero,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
};

fn cull_mode(two_sided: bool) -> Option<wgpu::Face> {
    (!two_sided).then_some(wgpu::Face::Back)
}

// Reverse-Z: larger depth = closer, so a POSITIVE bias pulls decals toward
// the camera (the inverse of terrain's -2/-2.0 push-away), letting a decal
// coplanar with its host surface win the GreaterEqual depth test. Magnitude
// mirrors terrain's proven value: it's enough to clear a coplanar tie against
// a zero-biased surface, and the slope term stays small enough at grazing
// angles to avoid x-raying thin geometry in front of the decal.
const DECAL_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: 2,
    slope_scale: 2.0,
    clamp: 0.0,
};

/// What every object-shader pipeline shares, whatever its vertex source.
pub struct ObjectPipelineSpec<'a> {
    pub shader: &'a wgpu::ShaderModule,
    pub layout: &'a wgpu::PipelineLayout,
    pub vertex_entry: &'static str,
    pub vertex_buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    pub format: wgpu::TextureFormat,
    pub texture_feedback: bool,
}

pub fn create_object_pipeline(
    device: &wgpu::Device,
    spec: &ObjectPipelineSpec,
    variant: SurfaceVariant,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(variant.label),
        layout: Some(spec.layout),
        vertex: wgpu::VertexState {
            module: spec.shader,
            entry_point: Some(spec.vertex_entry),
            buffers: spec.vertex_buffers,
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: spec.shader,
            entry_point: Some(variant.fs_entry),
            targets: &[Some(wgpu::ColorTargetState {
                format: spec.format,
                blend: Some(variant.blend),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &[("TEXTURE_FEEDBACK", f64::from(spec.texture_feedback))],
                ..Default::default()
            },
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: variant.cull_mode,
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: Texture::DEPTH_FORMAT,
            depth_write_enabled: Some(variant.depth_write),
            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
            stencil: wgpu::StencilState::default(),
            bias: variant.bias,
        }),
        multisample: wgpu::MultisampleState {
            count: 1,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview_mask: None,
        cache: None,
    })
}

impl ObjectPipelines {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        layout: &wgpu::PipelineLayout,
        texture_feedback: bool,
    ) -> Self {
        let shader = shaders::OBJECT.compile(device);
        let spec = ObjectPipelineSpec {
            shader: &shader,
            layout,
            vertex_entry: "vs_main",
            vertex_buffers: &[Some(Vertex::desc())],
            format,
            texture_feedback,
        };
        let create = |variant| create_object_pipeline(device, &spec, variant);

        Self {
            opaque_culled: create(SurfaceVariant::opaque(false)),
            opaque_twosided: create(SurfaceVariant::opaque(true)),
            alphatest_culled: create(SurfaceVariant::alphatest(false)),
            alphatest_twosided: create(SurfaceVariant::alphatest(true)),
            decal_alphatest_culled: create(
                SurfaceVariant::alphatest(false).decal("Pipeline Decal AlphaTest Culled"),
            ),
            decal_alphatest_twosided: create(
                SurfaceVariant::alphatest(true).decal("Pipeline Decal AlphaTest Two-Sided"),
            ),
            blend_culled: create(SurfaceVariant::blend(false)),
            blend_twosided: create(SurfaceVariant::blend(true)),
            decal_blend_culled: create(
                SurfaceVariant::blend(false).decal("Pipeline Decal Blend Culled"),
            ),
            decal_blend_twosided: create(
                SurfaceVariant::blend(true).decal("Pipeline Decal Blend Two-Sided"),
            ),
            multiply_culled: create(SurfaceVariant::multiply(false)),
            multiply_twosided: create(SurfaceVariant::multiply(true)),
        }
    }

    /// Must match the bucket routing in `gpu_cull.wgsl` and the bucket constants
    /// in `gpu_driven.rs`.
    pub fn pipelines_in_bucket_order(&self) -> [&wgpu::RenderPipeline; DRAW_BUCKET_COUNT] {
        [
            &self.opaque_culled,
            &self.opaque_twosided,
            &self.alphatest_culled,
            &self.alphatest_twosided,
            &self.decal_alphatest_culled,
            &self.decal_alphatest_twosided,
            &self.blend_culled,
            &self.blend_twosided,
            &self.decal_blend_culled,
            &self.decal_blend_twosided,
            &self.multiply_culled,
            &self.multiply_twosided,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alphatest_decals_write_depth_so_water_cannot_cover_them() {
        for two_sided in [false, true] {
            let decal = SurfaceVariant::alphatest(two_sided).decal("decal");
            assert!(decal.depth_write);
            assert_eq!(decal.bias, DECAL_BIAS);
        }
    }

    #[test]
    fn blend_is_premultiplied_and_multiply_scales_the_destination() {
        for two_sided in [false, true] {
            let blend = SurfaceVariant::blend(two_sided);
            assert_eq!(blend.blend, PREMULTIPLIED_BLEND);
            assert_eq!(blend.blend.color.src_factor, wgpu::BlendFactor::One);
            assert_eq!(
                blend.blend.color.dst_factor,
                wgpu::BlendFactor::OneMinusSrcAlpha
            );
            assert_eq!(blend.fs_entry, "fs_blend");

            let multiply = SurfaceVariant::multiply(two_sided);
            assert_eq!(multiply.blend.color.src_factor, wgpu::BlendFactor::Zero);
            assert_eq!(multiply.blend.color.dst_factor, wgpu::BlendFactor::Src);
            assert_eq!(multiply.blend.alpha.src_factor, wgpu::BlendFactor::Zero);
            assert_eq!(multiply.blend.alpha.dst_factor, wgpu::BlendFactor::One);
            assert_eq!(multiply.fs_entry, "fs_multiply");
        }
    }

    #[test]
    fn blend_and_multiply_never_write_depth_and_keep_the_cull_mode() {
        for two_sided in [false, true] {
            for variant in [
                SurfaceVariant::blend(two_sided),
                SurfaceVariant::multiply(two_sided),
            ] {
                assert!(!variant.depth_write);
                assert_eq!(variant.cull_mode.is_none(), two_sided);
            }
        }
    }

    #[test]
    fn blended_decals_never_write_depth() {
        for two_sided in [false, true] {
            assert!(!SurfaceVariant::blend(two_sided).decal("decal").depth_write);
        }
    }
}
