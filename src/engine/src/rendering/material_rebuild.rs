//! When the material bind group, which holds every texture view, is rebuilt. Rebuilds are
//! costly, so they are debounced while dirt accumulates; a few cases cannot wait.

use std::time::Duration;

/// The longest a dirty material bind group waits for its rebuild.
pub const MATERIAL_BG_DEBOUNCE: Duration = Duration::from_millis(150);

/// What changed since the last rebuild, and what is waiting on one.
#[derive(Debug, Clone, Copy, Default)]
pub struct MaterialDirt {
    /// Slots were recycled, vacated, swapped or resolved, or became alpha tested, or a cube map
    /// was added.
    pub slots_changed: bool,
    /// Some slot holds a view the bind group does not sample yet, a fresh slot included.
    pub views_unbound: bool,
    /// The material buffer was reallocated: the live bind group references a dead buffer.
    pub buffer_reallocated: bool,
    /// Slots past the bound part of the array are in use: indexing an unbound entry is UB.
    pub array_grew: bool,
    /// Staged cells wait for nothing but their views being bound, behind the loading screen.
    pub views_wanted_now: bool,
}

impl MaterialDirt {
    pub fn is_dirty(self) -> bool {
        self.slots_changed || self.views_unbound || self.buffer_reallocated || self.array_grew
    }

    fn cannot_wait(self) -> bool {
        self.buffer_reallocated || self.array_grew || (self.views_wanted_now && self.views_unbound)
    }
}

/// Whether to rebuild now, given whether the bind group is `dirty` (dirt accumulates across
/// frames until a rebuild) and how long ago it was last rebuilt.
pub fn rebuild_due(dirty: bool, dirt: MaterialDirt, since_rebuild: Duration) -> bool {
    dirty && (dirt.cannot_wait() || since_rebuild >= MATERIAL_BG_DEBOUNCE)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECENTLY: Duration = Duration::from_millis(10);

    #[test]
    fn a_fresh_unbound_slot_alone_makes_the_bind_group_dirty() {
        let dirt = MaterialDirt {
            views_unbound: true,
            ..MaterialDirt::default()
        };
        assert!(dirt.is_dirty());
        assert!(rebuild_due(dirt.is_dirty(), dirt, MATERIAL_BG_DEBOUNCE));
    }

    #[test]
    fn a_clean_bind_group_is_never_rebuilt() {
        assert!(!MaterialDirt::default().is_dirty());
        assert!(!rebuild_due(false, MaterialDirt::default(), Duration::MAX));
    }

    #[test]
    fn ordinary_dirt_waits_out_the_debounce() {
        let dirt = MaterialDirt {
            slots_changed: true,
            views_unbound: true,
            ..MaterialDirt::default()
        };
        assert!(!rebuild_due(true, dirt, RECENTLY));
        assert!(rebuild_due(true, dirt, MATERIAL_BG_DEBOUNCE));
    }

    #[test]
    fn views_wanted_behind_the_loading_screen_skip_the_debounce() {
        let dirt = MaterialDirt {
            views_unbound: true,
            views_wanted_now: true,
            ..MaterialDirt::default()
        };
        assert!(rebuild_due(true, dirt, RECENTLY));
    }

    #[test]
    fn views_wanted_with_every_view_bound_wait() {
        let dirt = MaterialDirt {
            slots_changed: true,
            views_wanted_now: true,
            ..MaterialDirt::default()
        };
        assert!(!rebuild_due(true, dirt, RECENTLY));
    }

    #[test]
    fn a_reallocated_buffer_or_a_grown_array_cannot_wait() {
        for dirt in [
            MaterialDirt {
                buffer_reallocated: true,
                ..MaterialDirt::default()
            },
            MaterialDirt {
                array_grew: true,
                ..MaterialDirt::default()
            },
        ] {
            assert!(rebuild_due(dirt.is_dirty(), dirt, RECENTLY));
        }
    }

    #[test]
    fn dirt_left_from_an_earlier_frame_still_rebuilds_after_the_debounce() {
        assert!(rebuild_due(
            true,
            MaterialDirt::default(),
            MATERIAL_BG_DEBOUNCE
        ));
    }
}
