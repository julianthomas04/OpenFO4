//! What the water draws while a water debug view is active.
//!
//! The translated water pixel shaders expose only their final colour, so the views here show
//! the engine-owned inputs and passes around them: a fragment entry of `water_wrapper.wgsl`
//! that replaces the translated pixel shader of a draw and only reads resources the water
//! passes already bind, or the translated depth-fog pass composited over black instead of over
//! the scene.
//! The translated shaders themselves receive and compute exactly what they do without a view.

use super::permutation::{SurfacePasses, Variant};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum WaterDebugOutput {
    /// A constant colour on every pixel a water draw covers.
    Coverage,
    /// The opaque scene copy the water shader refracts, at the pixel.
    SceneCopy,
    /// The depth-fog pass colour of the surface, weighted by the alpha the pass blends it with.
    DepthFog,
    /// The wading displacement field the ripple lookup reads.
    Displacement,
    /// The sharp screen-space reflection result.
    Ssr,
}

impl WaterDebugOutput {
    pub const ALL: [Self; 5] = [
        Self::Coverage,
        Self::SceneCopy,
        Self::DepthFog,
        Self::Displacement,
        Self::Ssr,
    ];

    /// The fragment entry of `water_wrapper.wgsl` that writes this output.
    pub fn fragment_entry(self) -> &'static str {
        match self {
            Self::Coverage => "fs_debug_coverage",
            Self::SceneCopy => "fs_debug_scene_copy",
            Self::DepthFog => "fs_depth_fog",
            Self::Displacement => "fs_debug_displacement",
            Self::Ssr => "fs_debug_ssr",
        }
    }

    /// Whether a surface with these passes draws this output: depth fog is a pass only some
    /// surfaces have, every other output covers all of them.
    pub fn applies_to(self, passes: &SurfacePasses) -> bool {
        self != Self::DepthFog || passes.depth_fog.is_some()
    }
}

/// The pipeline a water draw uses.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DrawPipeline {
    Shading(Variant),
    Debug(WaterDebugOutput, Variant),
}

impl DrawPipeline {
    /// The pipeline of a surface with `passes` while `debug` is selected; `None` when the
    /// selected output leaves the surface undrawn.
    pub fn select(debug: Option<WaterDebugOutput>, passes: &SurfacePasses) -> Option<Self> {
        match debug {
            None => Some(Self::Shading(passes.variant)),
            Some(output) => output
                .applies_to(passes)
                .then_some(Self::Debug(output, passes.variant)),
        }
    }
}

/// The selected water debug output.
#[derive(Debug, Default)]
pub struct DebugSelection {
    output: Option<WaterDebugOutput>,
}

impl DebugSelection {
    pub fn output(&self) -> Option<WaterDebugOutput> {
        self.output
    }

    /// Selects `output` (`None` for the normal shading). Returns the output when the selection
    /// changed to one, which is when its pipelines have to exist.
    pub fn select(&mut self, output: Option<WaterDebugOutput>) -> Option<WaterDebugOutput> {
        if self.output == output {
            return None;
        }
        self.output = output;
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::types::water_shader::water_flags;
    use crate::rendering::water::permutation::{PassContext, select};

    /// A surface of the depth shader with a depth-fog pass.
    fn surface_with_depth_fog() -> SurfacePasses {
        let passes = select(water_flags::DEPTH, PassContext::default());
        assert!(passes.depth_fog.is_some());
        passes
    }

    /// A surface of the refraction-only shader, which has no depth-fog pass.
    fn surface_without_depth_fog() -> SurfacePasses {
        let passes = select(0, PassContext::default());
        assert!(passes.depth_fog.is_none());
        assert_ne!(passes.variant, surface_with_depth_fog().variant);
        passes
    }

    #[test]
    fn depth_fog_applies_only_to_surfaces_with_a_depth_fog_pass() {
        assert!(WaterDebugOutput::DepthFog.applies_to(&surface_with_depth_fog()));
        assert!(!WaterDebugOutput::DepthFog.applies_to(&surface_without_depth_fog()));
    }

    #[test]
    fn every_other_output_applies_to_every_surface() {
        for output in WaterDebugOutput::ALL {
            if output != WaterDebugOutput::DepthFog {
                assert!(
                    output.applies_to(&surface_without_depth_fog()),
                    "{output:?}"
                );
                assert!(output.applies_to(&surface_with_depth_fog()), "{output:?}");
            }
        }
    }

    #[test]
    fn every_output_has_a_distinct_fragment_entry() {
        for (index, output) in WaterDebugOutput::ALL.into_iter().enumerate() {
            for other in &WaterDebugOutput::ALL[index + 1..] {
                assert_ne!(output.fragment_entry(), other.fragment_entry());
            }
        }
    }

    #[test]
    fn without_a_debug_output_every_surface_draws_its_shading() {
        for passes in [surface_with_depth_fog(), surface_without_depth_fog()] {
            assert_eq!(
                DrawPipeline::select(None, &passes),
                Some(DrawPipeline::Shading(passes.variant))
            );
        }
    }

    #[test]
    fn a_debug_output_draws_through_its_own_pipeline_of_the_surface_variant() {
        for output in WaterDebugOutput::ALL {
            let passes = surface_with_depth_fog();
            assert_eq!(
                DrawPipeline::select(Some(output), &passes),
                Some(DrawPipeline::Debug(output, passes.variant)),
                "{output:?}"
            );
        }
    }

    #[test]
    fn the_depth_fog_output_leaves_surfaces_without_the_pass_undrawn() {
        let passes = surface_without_depth_fog();
        assert_eq!(
            DrawPipeline::select(Some(WaterDebugOutput::DepthFog), &passes),
            None
        );
        assert_eq!(
            DrawPipeline::select(Some(WaterDebugOutput::Ssr), &passes),
            Some(DrawPipeline::Debug(WaterDebugOutput::Ssr, passes.variant))
        );
    }

    #[test]
    fn selecting_an_output_reports_the_change_and_deselecting_restores_the_shading() {
        let passes = surface_with_depth_fog();
        let mut selection = DebugSelection::default();
        assert_eq!(
            selection.select(None),
            None,
            "nothing changes from off to off"
        );

        assert_eq!(
            selection.select(Some(WaterDebugOutput::Ssr)),
            Some(WaterDebugOutput::Ssr)
        );
        assert_eq!(
            DrawPipeline::select(selection.output(), &passes),
            Some(DrawPipeline::Debug(WaterDebugOutput::Ssr, passes.variant))
        );
        assert_eq!(
            selection.select(Some(WaterDebugOutput::Ssr)),
            None,
            "the same output needs no new pipelines"
        );

        assert_eq!(selection.select(None), None);
        assert_eq!(
            DrawPipeline::select(selection.output(), &passes),
            Some(DrawPipeline::Shading(passes.variant))
        );
    }
}
