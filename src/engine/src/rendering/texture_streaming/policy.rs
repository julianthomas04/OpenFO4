use super::settings::TextureStreamingConfig;
use crate::assets::types::texture::TextureLayout;

/// Smallest edge a streamed texture may be reduced to.
const MIN_STREAMED_EDGE: u32 = 64;

/// A texture is downgraded only once it wants this many more skipped mips than
/// it holds. Upgrades have no margin: a texture the GPU samples finer than it
/// holds is under-resolved, so it is upgraded as soon as that is true.
const DOWNGRADE_MARGIN: f32 = 1.5;

/// Decides how many top mips a texture should skip. Pure and stateless: the
/// caller owns the time-hold (require the target to persist for some time
/// before acting on it) that complements the downgrade margin here.
///
/// Two inputs ask for detail and the finer one wins:
/// - demand, the estimate from the camera and the geometry: the largest number
///   of screen pixels one UV unit of the texture covers (see
///   `demand::DemandView`), so the texture's sampling edge divided by it is the
///   texels per pixel and its `log2` the mip the GPU samples (see
///   `sampling_edge` for which edge);
/// - feedback, the finest mip the GPU was seen to sample (see `feedback`).
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ResidencyPolicy {
    /// `iTexMipMapSkip`: an additive quality bias on every texture.
    pub global_skip: u32,
    /// Sampler anisotropy clamp: the GPU samples an elongated footprint at the
    /// mip of its minor axis, down to this many probes.
    pub max_anisotropy: u32,
}

impl ResidencyPolicy {
    pub fn new(config: &TextureStreamingConfig) -> Self {
        Self {
            global_skip: config.mip_skip,
            max_anisotropy: u32::from(config.max_anisotropy).max(1),
        }
    }

    /// Highest skip a texture may take: the remaining top mip keeps an edge of
    /// at least `MIN_STREAMED_EDGE`, and textures at or below it never skip.
    pub fn max_skip(layout: &TextureLayout) -> u32 {
        let smaller_edge = layout.width.min(layout.height);
        if smaller_edge <= MIN_STREAMED_EDGE {
            return 0;
        }
        let by_size = (smaller_edge / MIN_STREAMED_EDGE).ilog2();
        by_size.min(layout.mip_count.saturating_sub(1))
    }

    /// The edge, in texels, whose texel-per-pixel ratio selects the sampled
    /// mip. A pixel covers the same UV span on both axes, so on a non-square
    /// texture the footprint is elongated by the aspect ratio; anisotropic
    /// filtering then samples the mip of the minor axis, unless the ratio
    /// exceeds the probe count, which leaves `longer / max_anisotropy`.
    fn sampling_edge(self, layout: &TextureLayout) -> f32 {
        let longer = layout.width.max(layout.height) as f32;
        let shorter = layout.width.min(layout.height) as f32;
        shorter.max(longer / self.max_anisotropy as f32)
    }

    /// Mips to skip according to demand, fractional, before clamping; infinite
    /// when demand has no opinion.
    fn demand_skip(self, demand: f32, layout: &TextureLayout) -> f32 {
        if demand.is_nan() || demand <= 0.0 {
            return f32::INFINITY;
        }
        self.global_skip as f32 + (self.sampling_edge(layout) / demand).log2()
    }

    /// Skip for a texture with no resident state to be stable against, e.g. its
    /// first load. No hysteresis; unusable demand yields the cheapest mip.
    pub fn initial_skip(&self, demand: f32, layout: &TextureLayout) -> u32 {
        let max_skip = Self::max_skip(layout);
        let floor_skip = self.global_skip.min(max_skip);
        let desired = self.demand_skip(demand, layout);
        if desired.is_infinite() && desired > 0.0 {
            return max_skip;
        }
        desired.floor().clamp(floor_skip as f32, max_skip as f32) as u32
    }

    /// Skip that serves a GPU-sampled `feedback_mip` (full-resolution terms)
    /// exactly: global skip added, clamped like every target.
    pub fn feedback_skip(&self, feedback_mip: u32, layout: &TextureLayout) -> u32 {
        let max_skip = Self::max_skip(layout);
        (feedback_mip + self.global_skip).clamp(self.global_skip.min(max_skip), max_skip)
    }

    /// Total top mips (global skip included) to drop, given the demand, the
    /// finest mip the GPU was seen to sample (`feedback_mip`, in full-resolution
    /// terms) and the skip currently resident. Hysteresis is relative to
    /// `current_skip`, so feeding back the previous result is stable under
    /// small changes. Without any opinion the current skip stays.
    pub fn target_skip(
        &self,
        demand: f32,
        feedback_mip: Option<u32>,
        current_skip: u32,
        layout: &TextureLayout,
    ) -> u32 {
        let max_skip = Self::max_skip(layout);
        let floor_skip = self.global_skip.min(max_skip);
        let current = current_skip.clamp(floor_skip, max_skip);

        let feedback_skip =
            feedback_mip.map_or(f32::INFINITY, |mip| (mip + self.global_skip) as f32);
        let desired = self.demand_skip(demand, layout).min(feedback_skip);
        // Only +inf means no opinion; -inf (infinite demand) wants full detail.
        if desired == f32::INFINITY {
            return current;
        }

        let wanted = desired.floor().clamp(floor_skip as f32, max_skip as f32) as u32;
        let downgrade_due = desired >= current as f32 + DOWNGRADE_MARGIN;
        if wanted < current || downgrade_due {
            wanted
        } else {
            current
        }
    }
}

#[cfg(test)]
pub(super) fn bc1_layout(width: u32, height: u32, mip_count: u32) -> TextureLayout {
    TextureLayout {
        format: wgpu::TextureFormat::Bc1RgbaUnorm,
        block_bytes: 8,
        block_dim: 4,
        width,
        height,
        mip_count,
        data_offset: 0,
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact sentinels and copied values"
)]
mod tests {
    use super::*;

    fn policy(global_skip: u32, max_anisotropy: u32) -> ResidencyPolicy {
        ResidencyPolicy {
            global_skip,
            max_anisotropy,
        }
    }

    #[test]
    fn max_skip_keeps_at_least_the_minimum_edge() {
        // (width, height, mips, expected): smaller edge / 64, rounded down to a power of two.
        let cases = [
            (64, 64, 7, 0),
            (128, 128, 8, 1),
            (1024, 1024, 11, 4),
            (2048, 256, 12, 2),
            (1024, 1024, 3, 2),
            (1024, 1024, 1, 0),
        ];
        for (width, height, mips, expected) in cases {
            let layout = bc1_layout(width, height, mips);
            assert_eq!(
                ResidencyPolicy::max_skip(&layout),
                expected,
                "{width}x{height} with {mips} mips"
            );
        }
    }

    #[test]
    fn initial_skip_follows_texels_per_pixel() {
        // 1024 texels covered by 256 pixels = 4 texels per pixel = mip 2.
        let layout = bc1_layout(1024, 1024, 11);
        assert_eq!(policy(0, 16).initial_skip(256.0, &layout), 2);
    }

    #[test]
    fn initial_skip_adds_the_global_skip() {
        let layout = bc1_layout(1024, 1024, 11);
        assert_eq!(policy(1, 16).initial_skip(256.0, &layout), 3);
    }

    #[test]
    fn initial_skip_is_clamped_to_max_skip() {
        let layout = bc1_layout(1024, 1024, 11);
        assert_eq!(policy(0, 16).initial_skip(1.0, &layout), 4);
    }

    #[test]
    fn initial_skip_with_unusable_demand_is_the_cheapest_mip() {
        let layout = bc1_layout(1024, 1024, 11);
        for demand in [0.0, -1.0, f32::NAN] {
            assert_eq!(policy(0, 16).initial_skip(demand, &layout), 4, "{demand}");
        }
    }

    #[test]
    fn initial_skip_with_full_detail_demand_keeps_every_mip() {
        let layout = bc1_layout(1024, 1024, 11);
        assert_eq!(policy(0, 16).initial_skip(f32::INFINITY, &layout), 0);
    }

    #[test]
    fn initial_skip_never_goes_below_the_global_skip() {
        let layout = bc1_layout(1024, 1024, 11);
        assert_eq!(policy(2, 16).initial_skip(f32::INFINITY, &layout), 2);
    }

    #[test]
    fn anisotropy_lets_an_elongated_texture_sample_a_finer_mip() {
        let layout = bc1_layout(2048, 256, 12);
        // 16 probes cover the 8:1 elongation, so the minor axis (256) selects the mip.
        assert_eq!(policy(0, 16).initial_skip(256.0, &layout), 0);
        // 4 probes leave 2048/4 = 512 texels, one mip coarser.
        assert_eq!(policy(0, 4).initial_skip(256.0, &layout), 1);
    }

    #[test]
    fn feedback_skip_adds_global_skip_and_clamps() {
        let layout = bc1_layout(1024, 1024, 11);
        assert_eq!(policy(1, 16).feedback_skip(2, &layout), 3);
        assert_eq!(policy(1, 16).feedback_skip(9, &layout), 4);
    }

    #[test]
    fn target_skip_without_any_opinion_keeps_the_current_skip() {
        let layout = bc1_layout(1024, 1024, 11);
        assert_eq!(policy(0, 16).target_skip(0.0, None, 3, &layout), 3);
    }

    #[test]
    fn target_skip_upgrades_without_margin() {
        // Demand 256 wants skip 2; the texture holds skip 3.
        let layout = bc1_layout(1024, 1024, 11);
        assert_eq!(policy(0, 16).target_skip(256.0, None, 3, &layout), 2);
    }

    #[test]
    fn target_skip_downgrade_needs_a_margin_over_the_current_skip() {
        let layout = bc1_layout(1024, 1024, 11);
        let policy = policy(0, 16);
        // Demand 128 wants skip 3, only 1 above the resident 2: below the 1.5 margin.
        assert_eq!(policy.target_skip(128.0, None, 2, &layout), 2);
        // Demand 64 wants skip 4, 2 above: past the margin.
        assert_eq!(policy.target_skip(64.0, None, 2, &layout), 4);
    }

    #[test]
    fn target_skip_feedback_overrides_coarser_demand() {
        let layout = bc1_layout(1024, 1024, 11);
        // Demand wants skip 4 but the GPU sampled mip 1.
        assert_eq!(policy(0, 16).target_skip(64.0, Some(1), 4, &layout), 1);
    }

    #[test]
    fn target_skip_feedback_alone_drives_a_texture_without_demand() {
        let layout = bc1_layout(1024, 1024, 11);
        assert_eq!(policy(1, 16).target_skip(0.0, Some(0), 4, &layout), 1);
    }

    #[test]
    fn target_skip_clamps_a_stale_current_skip_into_range() {
        let layout = bc1_layout(1024, 1024, 11);
        assert_eq!(policy(0, 16).target_skip(0.0, None, 99, &layout), 4);
    }

    #[test]
    fn target_skip_of_infinite_demand_wants_full_detail() {
        let layout = bc1_layout(1024, 1024, 11);
        assert_eq!(
            policy(0, 16).target_skip(f32::INFINITY, None, 3, &layout),
            0
        );
    }
}
