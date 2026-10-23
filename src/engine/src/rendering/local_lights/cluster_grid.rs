use glam::{UVec4, Vec4};

use super::settings::LocalLightSettings;

// Mirrored by the constants in `shaders/light_data.wgsl`.
pub const CLUSTER_TILES_X: u32 = 32;
pub const CLUSTER_TILES_Y: u32 = 18;
pub const CLUSTER_SLICES: u32 = 32;
pub const CLUSTER_COUNT: u32 = CLUSTER_TILES_X * CLUSTER_TILES_Y * CLUSTER_SLICES;
pub const MAX_LIGHTS_PER_CLUSTER: u32 = 128;
pub const MAX_VISIBLE_LIGHTS: u32 = 4096;
/// Threads per cluster workgroup: one light per thread while compacting a
/// batch, one depth slice per thread while appending.
const CLUSTER_WORKGROUP_SIZE: u32 = 64;
const _: () = assert!(CLUSTER_SLICES <= CLUSTER_WORKGROUP_SIZE);

/// Far depth of the first slice, engine units (about 1.4 m).
const FIRST_SLICE_DEPTH: f32 = 10.0;
/// The exponential slices need a far depth past the first slice's.
const MIN_FAR_TO_FIRST_RATIO: f32 = 2.0;

/// Exponential depth slicing: slice 0 covers `[0, first)`, slices
/// `1..CLUSTER_SLICES` split `[first, far)` geometrically and the last one is
/// open-ended, so `slice = floor(log2(depth) * scale - bias) + 1`.
#[derive(Debug, Copy, Clone)]
pub struct ClusterDepth {
    first: f32,
    far: f32,
}

impl ClusterDepth {
    /// The far depth is where placed lights have faded out.
    pub fn for_settings(settings: &LocalLightSettings) -> Self {
        Self {
            first: FIRST_SLICE_DEPTH,
            far: settings
                .fade
                .end()
                .max(FIRST_SLICE_DEPTH * MIN_FAR_TO_FIRST_RATIO),
        }
    }

    /// x: first slice far depth, y: cluster far depth, z: log2 scale, w: log2 bias.
    pub fn uniform(&self) -> Vec4 {
        let scale = (CLUSTER_SLICES - 1) as f32 / (self.far / self.first).log2();
        Vec4::new(self.first, self.far, scale, self.first.log2() * scale)
    }
}

/// x, y: tiles, z: depth slices, w: 1 while local lights shade.
pub fn cluster_grid(settings: &LocalLightSettings) -> UVec4 {
    UVec4::new(
        CLUSTER_TILES_X,
        CLUSTER_TILES_Y,
        CLUSTER_SLICES,
        u32::from(settings.enabled),
    )
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact sentinels and copied values"
)]
mod tests {
    use super::*;
    use crate::rendering::local_lights::LightFade;

    fn settings(fade_start: f32, fade_range: f32) -> LocalLightSettings {
        LocalLightSettings::new(LightFade {
            start: fade_start,
            range: fade_range,
        })
    }

    /// The slice a depth lands in, by the formula the shader applies to the uniform.
    fn slice_of(uniform: Vec4, depth: f32) -> i32 {
        if depth < uniform.x {
            return 0;
        }
        ((depth.log2() * uniform.z - uniform.w).floor() as i32 + 1).min(CLUSTER_SLICES as i32 - 1)
    }

    #[test]
    fn the_first_slice_ends_at_the_first_depth() {
        let uniform = ClusterDepth::for_settings(&settings(300.0, 50.0)).uniform();
        assert_eq!(slice_of(uniform, 9.99), 0);
        assert_eq!(slice_of(uniform, 10.01), 1);
    }

    #[test]
    fn slices_split_the_range_geometrically_up_to_the_fade_end() {
        let uniform = ClusterDepth::for_settings(&settings(300.0, 50.0)).uniform();
        assert_eq!(uniform.y, 350.0);
        // The last geometric slice ends at the far depth.
        assert_eq!(slice_of(uniform, 349.0), CLUSTER_SLICES as i32 - 1);
        assert_eq!(slice_of(uniform, 1e6), CLUSTER_SLICES as i32 - 1);
        // Halfway in log space (sqrt(10 * 350)) is the middle of the geometric slices.
        let middle = (10.0f32 * 350.0).sqrt();
        let middle_slice = slice_of(uniform, middle);
        assert!((15..=17).contains(&middle_slice), "{middle_slice}");
    }

    #[test]
    fn slice_index_never_decreases_with_depth() {
        let uniform = ClusterDepth::for_settings(&settings(300.0, 50.0)).uniform();
        let depths = [
            1.0, 9.0, 10.0, 11.0, 25.0, 60.0, 120.0, 349.0, 350.0, 5000.0,
        ];
        let slices: Vec<i32> = depths.iter().map(|d| slice_of(uniform, *d)).collect();
        assert!(
            slices.windows(2).all(|pair| pair[0] <= pair[1]),
            "{slices:?}"
        );
    }

    #[test]
    fn a_short_fade_range_still_leaves_room_for_the_exponential_slices() {
        let uniform = ClusterDepth::for_settings(&settings(1.0, 1.0)).uniform();
        assert_eq!(uniform.y, 20.0);
        assert!(uniform.z.is_finite() && uniform.z > 0.0);
    }

    #[test]
    fn the_grid_uniform_reports_whether_local_lights_shade() {
        let mut local = settings(300.0, 50.0);
        assert_eq!(cluster_grid(&local).w, 1);
        local.enabled = false;
        assert_eq!(cluster_grid(&local).w, 0);
    }
}
