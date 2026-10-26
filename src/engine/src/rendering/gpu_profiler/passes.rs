const TIMESTAMPS_PER_PASS: u32 = 2;

macro_rules! gpu_passes {
    ($($name:ident => $label:literal),+ $(,)?) => {
        /// A profiled GPU pass, in frame order. Adding a pass is one line here
        /// plus the `GpuProfiler::*_writes` call where its pass is encoded.
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum GpuPass {
            $($name),+
        }

        impl GpuPass {
            pub const ALL: &'static [GpuPass] = &[$(GpuPass::$name),+];

            pub fn label(self) -> &'static str {
                match self {
                    $(GpuPass::$name => $label),+
                }
            }
        }
    };
}

gpu_passes! {
    ObjectCull => "object cull",
    TerrainCull => "terrain cull",
    LightCull => "light cull",
    LightCluster => "light cluster",
    ShadowCull => "shadow cull",
    ShadowCascade0 => "shadow cascade0",
    ShadowCascade1 => "shadow cascade1",
    ShadowCascade2 => "shadow cascade2",
    ShadowCascade3 => "shadow cascade3",
    LampShadows => "lamp shadows",
    RainOcclusion => "rain occlusion",
    World => "world",
    Grass => "grass",
    WaterDisplacement => "water displacement",
    WaterSsr => "water ssr",
    Water => "water",
    HizBuild => "hiz build",
    AutoExposure => "auto exposure",
    Tonemap => "tonemap",
    Hud => "hud",
    Egui => "egui",
}

const _: () = assert!(GpuPass::ALL.len() <= u32::BITS as usize);
const _: () =
    assert!(crate::rendering::lighting::frame_uniform::MAX_CASCADES <= GpuPass::CASCADES.len());

impl GpuPass {
    pub const COUNT: usize = Self::ALL.len();
    pub const QUERY_COUNT: u32 = Self::COUNT as u32 * TIMESTAMPS_PER_PASS;

    /// Passes skipped on frames where their cached output is still valid; an
    /// unrecorded frame counts as 0 ms so the smoothed time reflects the
    /// per-frame average rather than the last recorded frame.
    pub fn is_amortised(self) -> bool {
        matches!(
            self,
            GpuPass::ShadowCull | GpuPass::LampShadows | GpuPass::RainOcclusion
        ) || self.cascade_index().is_some()
    }

    const CASCADES: [GpuPass; 4] = [
        GpuPass::ShadowCascade0,
        GpuPass::ShadowCascade1,
        GpuPass::ShadowCascade2,
        GpuPass::ShadowCascade3,
    ];

    /// The span of one shadow cascade; at most `MAX_CASCADES` (4) exist.
    pub fn shadow_cascade(cascade: usize) -> GpuPass {
        Self::CASCADES[cascade]
    }

    pub fn cascade_index(self) -> Option<usize> {
        Self::CASCADES.iter().position(|pass| *pass == self)
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn bit(self) -> u32 {
        1 << self.index()
    }

    pub fn begin_query(self) -> u32 {
        self.index() as u32 * TIMESTAMPS_PER_PASS
    }

    pub fn end_query(self) -> u32 {
        self.begin_query() + 1
    }

    pub fn query_range(self) -> std::ops::Range<u32> {
        self.begin_query()..self.end_query() + 1
    }
}

/// Which timestamps a pass writes when one profiled entry spans several
/// passes (cascades, the `HiZ` mip chain): the first pass writes the begin
/// timestamp, the last the end timestamp, the ones between write nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpanPart {
    Whole,
    First,
    Inner,
    Last,
}

impl SpanPart {
    pub fn of(index: usize, count: usize) -> Self {
        match (index == 0, index + 1 == count) {
            (true, true) => SpanPart::Whole,
            (true, false) => SpanPart::First,
            (false, true) => SpanPart::Last,
            (false, false) => SpanPart::Inner,
        }
    }

    pub fn writes_begin(self) -> bool {
        matches!(self, SpanPart::Whole | SpanPart::First)
    }

    pub fn writes_end(self) -> bool {
        matches!(self, SpanPart::Whole | SpanPart::Last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grass_is_timed_right_after_the_world_pass_under_its_own_label() {
        assert_eq!(GpuPass::Grass.index(), GpuPass::World.index() + 1);
        assert_eq!(GpuPass::Grass.label(), "grass");
        assert!(!GpuPass::Grass.is_amortised());
    }

    #[test]
    fn the_rain_occlusion_map_is_timed_with_the_shadows_and_only_counts_on_frames_that_render_it() {
        assert_eq!(
            GpuPass::RainOcclusion.index(),
            GpuPass::LampShadows.index() + 1
        );
        assert!(GpuPass::RainOcclusion.is_amortised());
        assert_eq!(GpuPass::RainOcclusion.label(), "rain occlusion");
    }

    #[test]
    fn every_pass_owns_two_distinct_timestamp_slots() {
        let mut slots: Vec<u32> = GpuPass::ALL
            .iter()
            .flat_map(|pass| [pass.begin_query(), pass.end_query()])
            .collect();
        slots.sort_unstable();
        slots.dedup();
        assert_eq!(slots.len() as u32, GpuPass::QUERY_COUNT);
    }
}
