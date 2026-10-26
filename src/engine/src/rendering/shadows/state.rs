use glam::{Mat4, UVec4, Vec3, Vec4};

use super::cascade_cache::{CascadeCache, ReasonCounts};
use super::cascades::{CascadeSet, GridAnchors};
use super::lamps::ShadowInvalidations;
use super::settings::{ShadowConfig, ShadowSettings};
use crate::rendering::camera::Camera;
use crate::rendering::lighting::frame_uniform::MAX_CASCADES;

/// The frame-uniform members that describe the sun shadow cascades.
pub struct ShadowUniformFields {
    pub cascade_view_proj: [Mat4; MAX_CASCADES],
    pub cascade_spheres: [Vec4; MAX_CASCADES],
    pub shadow_params: Vec4,
    pub cascade_texel_world: Vec4,
    pub shadow_filter: UVec4,
    pub cascade_count: u32,
}

/// CPU authority for sun shadows: the INI quality, the panel controls and the
/// cascades kept while frozen.
#[derive(Debug, Clone)]
pub struct ShadowState {
    pub config: ShadowConfig,
    pub settings: ShadowSettings,
    frozen: Option<CascadeSet>,
    cache: CascadeCache,
    anchors: GridAnchors,
}

impl ShadowState {
    pub fn new(config: ShadowConfig) -> Self {
        Self {
            cache: CascadeCache::new(config.cascade_count),
            config,
            settings: ShadowSettings::default(),
            frozen: None,
            anchors: GridAnchors::default(),
        }
    }

    /// Cumulative cascade renders, for the PERF line.
    pub fn cascade_refreshes(&self) -> ReasonCounts {
        self.cache.refreshes()
    }

    /// `current` is this frame's already chosen cascade set, if any.
    pub fn note_invalidations(
        &mut self,
        invalidations: &ShadowInvalidations,
        current: Option<&mut CascadeSet>,
    ) {
        self.cache.note_invalidations(invalidations, current);
    }

    /// Sun shadows are off: the cascade maps stop being maintained.
    pub fn discard_cached_cascades(&mut self) {
        self.cache.discard();
        self.frozen = None;
        self.anchors.reset();
    }

    /// Refits every frame unless frozen, in which case the cascades fitted when
    /// freezing was switched on are reused and only render until their map is
    /// committed. `rendered` are the cascades the renderer encoded since the
    /// previous call.
    pub fn cascades(&mut self, camera: &Camera, to_sun: Vec3, rendered: u32) -> CascadeSet {
        self.cache.commit(rendered);
        if let (true, Some(frozen)) = (self.settings.freeze_cascades, &self.frozen) {
            return self.cache.hold(frozen);
        }
        let fitted = CascadeSet::fit(
            camera,
            to_sun,
            &self.config,
            self.cache.settings.eye_centered_from,
            &mut self.anchors,
        );
        let resolved = self.cache.resolve(fitted);
        self.frozen = self.settings.freeze_cascades.then(|| resolved.clone());
        resolved
    }

    /// Biases leave here in texels, scaled by `fShadowBiasScale`; the shader
    /// converts them to world units with the receiving cascade's texel size.
    pub fn uniform_fields(&self, cascades: Option<&CascadeSet>) -> ShadowUniformFields {
        let mut fields = ShadowUniformFields {
            cascade_view_proj: [Mat4::ZERO; MAX_CASCADES],
            cascade_spheres: [Vec4::ZERO; MAX_CASCADES],
            shadow_params: Vec4::new(
                self.settings.constant_bias * self.config.bias_scale,
                self.settings.slope_bias * self.config.bias_scale,
                self.settings.normal_offset * self.config.bias_scale,
                self.config.split_blend,
            ),
            cascade_texel_world: Vec4::ZERO,
            shadow_filter: UVec4::new(self.config.filter.sun_disc_taps(), 0, 0, 0),
            cascade_count: 0,
        };
        let Some(cascades) = cascades else {
            return fields;
        };
        for (i, cascade) in cascades.cascades.iter().enumerate() {
            fields.cascade_view_proj[i] = cascade.view_proj;
            fields.cascade_spheres[i] = cascade.sphere;
            fields.cascade_texel_world[i] = cascade.texel_world;
        }
        fields.cascade_count = cascades.cascades.len() as u32;
        fields
    }
}
