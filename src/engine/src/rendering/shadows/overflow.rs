use super::lamps::atlas::Tile;
use super::shadow_cull::{CASTER_KINDS, CasterKind, ShadowDraws};
use super::views::MAX_SHADOW_VIEWS;
use crate::rendering::device_loss::DeviceLossMonitor;
use crate::rendering::lighting::frame_uniform::MAX_CASCADES;
use crate::rendering::readback_ring::{ReadbackError, ReadbackRing, SLOTS};

const COUNT_BYTES: u64 = (CASTER_KINDS * MAX_SHADOW_VIEWS * std::mem::size_of::<u32>()) as u64;

/// What the views of one readback frame were drawn with.
#[derive(Clone, Default)]
struct ViewFrame {
    lamp_tiles: Vec<Tile>,
    /// Bit `i`: cascade `i` rendered; the others kept an earlier map and
    /// their counts are zero.
    cascades_rendered: u32,
    cascade_capacities: [u32; CASTER_KINDS],
    lamp_capacities: [u32; CASTER_KINDS],
}

/// Lamp tiles to re-render and the draw count each kind needed. A tile is
/// listed when its view overflowed its draw region or when its draw counts
/// were never read back.
#[derive(Default)]
pub struct LampOverflow {
    pub tiles: Vec<Tile>,
    pub needed: [u32; CASTER_KINDS],
}

impl LampOverflow {
    pub fn overflowed(&self) -> bool {
        self.needed.iter().any(|&needed| needed > 0)
    }

    fn push_tile(&mut self, tile: Tile) {
        if !self.tiles.contains(&tile) {
            self.tiles.push(tile);
        }
    }
}

/// Draw counts of the latest read back frame. `cascade_overflow` is set when
/// any cascade needed more draws than its region holds.
#[derive(Default, Clone, Copy)]
pub struct ShadowDrawStats {
    pub cascade_draws: [[u32; CASTER_KINDS]; MAX_CASCADES],
    pub lamp_draws: [u32; CASTER_KINDS],
    pub cascade_overflow: bool,
}

/// Folds one read back frame's draw counts (`counts`, kind-major with a
/// `MAX_SHADOW_VIEWS` stride) into the draw statistics. A cascade that was not
/// rendered keeps the previous `latest` draws and overflow bit; lamp tiles
/// whose view needed more draws than its region holds are added to `overflow`.
fn evaluate_counts(
    frame: &ViewFrame,
    counts: &[u32],
    cascade_slots: usize,
    latest: &ShadowDrawStats,
    cascade_overflowed: &mut u32,
    overflow: &mut LampOverflow,
) -> ShadowDrawStats {
    let count = |kind: CasterKind, view: usize| counts[kind.index() * MAX_SHADOW_VIEWS + view];
    let mut stats = ShadowDrawStats {
        cascade_draws: latest.cascade_draws,
        ..Default::default()
    };
    for cascade in (0..cascade_slots).filter(|c| frame.cascades_rendered >> c & 1 == 1) {
        let mut overflowed = false;
        for kind in CasterKind::ALL {
            let drawn = count(kind, cascade);
            stats.cascade_draws[cascade][kind.index()] = drawn;
            overflowed |= drawn > frame.cascade_capacities[kind.index()];
        }
        *cascade_overflowed =
            *cascade_overflowed & !(1 << cascade) | u32::from(overflowed) << cascade;
    }
    stats.cascade_overflow = *cascade_overflowed != 0;
    for (lamp, tile) in frame.lamp_tiles.iter().enumerate() {
        let mut overflowed = false;
        for kind in CasterKind::ALL {
            let drawn = count(kind, cascade_slots + lamp);
            stats.lamp_draws[kind.index()] += drawn;
            if drawn > frame.lamp_capacities[kind.index()] {
                overflowed = true;
                let needed = &mut overflow.needed[kind.index()];
                *needed = (*needed).max(drawn);
            }
        }
        if overflowed {
            overflow.push_tile(*tile);
        }
    }
    stats
}

/// Reads the shadow cull's draw counts back to find views that overflowed
/// their draw region: the cull keeps counting past the capacity, so the count
/// is the draw count the view needed.
pub struct ShadowCountsReadback {
    ring: ReadbackRing,
    frames: [ViewFrame; SLOTS],
    cascade_slots: usize,
    /// Lamp tiles rendered in a frame the ring had no slot for.
    unchecked: Vec<Tile>,
    /// Bit `i`: the last read back render of cascade `i` overflowed its draw
    /// region; cleared when a render is encoded without a readback slot.
    cascade_overflowed: u32,
    pub latest: ShadowDrawStats,
}

impl ShadowCountsReadback {
    pub fn new(device: &wgpu::Device, cascade_slots: usize) -> Self {
        assert!(cascade_slots <= MAX_CASCADES);
        Self {
            ring: ReadbackRing::new(device, "shadow_draw_counts", COUNT_BYTES),
            frames: Default::default(),
            cascade_slots,
            unchecked: Vec::new(),
            cascade_overflowed: 0,
            latest: ShadowDrawStats::default(),
        }
    }

    /// Encode after the shadow cull dispatch so the copy sees this frame's
    /// counts. Without a free slot the lamp tiles of this frame are queued
    /// for the next poll to re-render.
    pub fn encode_copy(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        draws: &ShadowDraws,
        lamp_tiles: &[Tile],
        cascades_rendered: u32,
    ) {
        let Some(slot) = self.ring.acquire() else {
            self.unchecked.extend_from_slice(lamp_tiles);
            // The new render of these cascades is never read back, so the
            // overflow bits of their previous render no longer describe them.
            self.cascade_overflowed &= !cascades_rendered;
            self.latest.cascade_overflow = self.cascade_overflowed != 0;
            return;
        };
        encoder.copy_buffer_to_buffer(&draws.counts, 0, self.ring.buffer(slot), 0, COUNT_BYTES);
        self.frames[slot] = ViewFrame {
            lamp_tiles: lamp_tiles.to_vec(),
            cascades_rendered,
            cascade_capacities: draws.cascade_capacities(),
            lamp_capacities: draws.lamp_capacities(),
        };
    }

    pub fn post_submit(&mut self) {
        self.ring.post_submit();
    }

    pub fn poll(
        &mut self,
        device: &wgpu::Device,
        device_loss: &DeviceLossMonitor,
    ) -> Result<LampOverflow, ReadbackError> {
        let mut overflow = LampOverflow::default();
        let frames = &self.frames;
        let cascade_slots = self.cascade_slots;
        let latest = &mut self.latest;
        let cascade_overflowed = &mut self.cascade_overflowed;
        self.ring.poll(device, device_loss, |slot, data| {
            *latest = evaluate_counts(
                &frames[slot],
                bytemuck::cast_slice(data),
                cascade_slots,
                latest,
                cascade_overflowed,
                &mut overflow,
            );
        })?;
        for tile in std::mem::take(&mut self.unchecked) {
            overflow.push_tile(tile);
        }
        Ok(overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CASCADE_SLOTS: usize = 3;

    fn tile(index: u32) -> Tile {
        Tile {
            x: index * 128,
            y: 0,
            size: 128,
        }
    }

    /// Counts laid out kind-major with the `MAX_SHADOW_VIEWS` stride.
    fn counts(entries: &[(CasterKind, usize, u32)]) -> Vec<u32> {
        let mut counts = vec![0; CASTER_KINDS * MAX_SHADOW_VIEWS];
        for &(kind, view, count) in entries {
            counts[kind.index() * MAX_SHADOW_VIEWS + view] = count;
        }
        counts
    }

    fn frame(cascades_rendered: u32, lamp_tiles: Vec<Tile>) -> ViewFrame {
        ViewFrame {
            lamp_tiles,
            cascades_rendered,
            cascade_capacities: [100, 50, 10],
            lamp_capacities: [20, 10, 5],
        }
    }

    fn evaluate(
        frame: &ViewFrame,
        counts: &[u32],
        latest: &ShadowDrawStats,
        cascade_overflowed: &mut u32,
    ) -> (ShadowDrawStats, LampOverflow) {
        let mut overflow = LampOverflow::default();
        let stats = evaluate_counts(
            frame,
            counts,
            CASCADE_SLOTS,
            latest,
            cascade_overflowed,
            &mut overflow,
        );
        (stats, overflow)
    }

    #[test]
    fn draws_within_capacity_report_no_overflow() {
        let counts = counts(&[
            (CasterKind::Opaque, 0, 100),
            (CasterKind::AlphaTest, 1, 50),
            (CasterKind::Opaque, CASCADE_SLOTS, 20),
        ]);
        let mut flags = 0;
        let (stats, overflow) = evaluate(
            &frame(0b011, vec![tile(0)]),
            &counts,
            &ShadowDrawStats::default(),
            &mut flags,
        );
        assert!(!stats.cascade_overflow);
        assert!(!overflow.overflowed());
        assert_eq!(overflow.tiles, []);
        assert_eq!(stats.cascade_draws[0][CasterKind::Opaque.index()], 100);
    }

    #[test]
    fn a_cascade_over_its_region_sets_the_overflow_flag() {
        let counts = counts(&[(CasterKind::AlphaTest, 1, 51)]);
        let mut flags = 0;
        let (stats, _) = evaluate(
            &frame(0b010, vec![]),
            &counts,
            &ShadowDrawStats::default(),
            &mut flags,
        );
        assert!(stats.cascade_overflow);
        assert_eq!(flags, 0b010);
    }

    #[test]
    fn a_later_render_of_the_cascade_within_capacity_clears_its_overflow_bit() {
        let mut flags = 0b010;
        let (stats, _) = evaluate(
            &frame(0b010, vec![]),
            &counts(&[(CasterKind::AlphaTest, 1, 5)]),
            &ShadowDrawStats::default(),
            &mut flags,
        );
        assert!(!stats.cascade_overflow);
        assert_eq!(flags, 0);
    }

    #[test]
    fn a_cascade_that_was_not_rendered_keeps_its_previous_draws_and_overflow_bit() {
        let mut latest = ShadowDrawStats::default();
        latest.cascade_draws[2][CasterKind::Terrain.index()] = 7;
        let mut flags = 0b100;
        let (stats, _) = evaluate(&frame(0b001, vec![]), &counts(&[]), &latest, &mut flags);
        assert_eq!(stats.cascade_draws[2][CasterKind::Terrain.index()], 7);
        assert!(stats.cascade_overflow);
        assert_eq!(flags, 0b100);
    }

    #[test]
    fn a_lamp_tile_over_its_region_is_listed_with_the_draws_it_needed() {
        let counts = counts(&[
            (CasterKind::Opaque, CASCADE_SLOTS + 1, 300),
            (CasterKind::AlphaTest, CASCADE_SLOTS + 1, 11),
        ]);
        let mut flags = 0;
        let (_, overflow) = evaluate(
            &frame(0, vec![tile(0), tile(1)]),
            &counts,
            &ShadowDrawStats::default(),
            &mut flags,
        );
        assert_eq!(overflow.tiles, vec![tile(1)]);
        assert_eq!(overflow.needed, [300, 11, 0]);
        assert!(overflow.overflowed());
    }

    #[test]
    fn the_needed_draws_are_the_maximum_over_all_overflowing_lamps() {
        let counts = counts(&[
            (CasterKind::Opaque, CASCADE_SLOTS, 40),
            (CasterKind::Opaque, CASCADE_SLOTS + 1, 90),
            (CasterKind::Opaque, CASCADE_SLOTS + 2, 60),
        ]);
        let mut flags = 0;
        let (_, overflow) = evaluate(
            &frame(0, vec![tile(0), tile(1), tile(2)]),
            &counts,
            &ShadowDrawStats::default(),
            &mut flags,
        );
        assert_eq!(overflow.needed[CasterKind::Opaque.index()], 90);
        assert_eq!(overflow.tiles, vec![tile(0), tile(1), tile(2)]);
    }

    #[test]
    fn a_draw_count_equal_to_the_capacity_is_not_an_overflow() {
        let counts = counts(&[(CasterKind::Terrain, CASCADE_SLOTS, 5)]);
        let mut flags = 0;
        let (_, overflow) = evaluate(
            &frame(0, vec![tile(0)]),
            &counts,
            &ShadowDrawStats::default(),
            &mut flags,
        );
        assert!(!overflow.overflowed());
    }

    #[test]
    fn lamp_draws_are_summed_over_the_frames_lamp_views() {
        let counts = counts(&[
            (CasterKind::Opaque, CASCADE_SLOTS, 3),
            (CasterKind::Opaque, CASCADE_SLOTS + 1, 4),
        ]);
        let mut flags = 0;
        let (stats, _) = evaluate(
            &frame(0, vec![tile(0), tile(1)]),
            &counts,
            &ShadowDrawStats::default(),
            &mut flags,
        );
        assert_eq!(stats.lamp_draws[CasterKind::Opaque.index()], 7);
    }

    #[test]
    fn a_tile_that_overflows_in_several_kinds_is_listed_once() {
        let counts = counts(&[
            (CasterKind::Opaque, CASCADE_SLOTS, 999),
            (CasterKind::AlphaTest, CASCADE_SLOTS, 999),
        ]);
        let mut flags = 0;
        let (_, overflow) = evaluate(
            &frame(0, vec![tile(0)]),
            &counts,
            &ShadowDrawStats::default(),
            &mut flags,
        );
        assert_eq!(overflow.tiles, vec![tile(0)]);
    }

    #[test]
    fn push_tile_ignores_a_tile_already_listed() {
        let mut overflow = LampOverflow::default();
        overflow.push_tile(tile(3));
        overflow.push_tile(tile(3));
        overflow.push_tile(tile(4));
        assert_eq!(overflow.tiles, vec![tile(3), tile(4)]);
    }
}
