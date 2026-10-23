use std::collections::HashMap;
use std::ops::Range;

use crate::rendering::free_list::{alloc_range, insert_free_range};
use crate::rendering::vertex::Vertex;

const INITIAL_VERTEX_CAPACITY: u32 = 512 * 1024;
const INITIAL_INDEX_CAPACITY: u32 = 2 * 1024 * 1024;

/// Each growth step multiplies the capacity by this percentage. Every step
/// allocates and copies the whole buffer inside one frame, so fewer, larger
/// steps trade spare VRAM for fewer frame spikes.
const GROWTH_PERCENT_ENV: &str = "OPENFO4_MEGA_GROWTH_PERCENT";
const DEFAULT_GROWTH_PERCENT: u64 = 200;

fn parse_growth_percent(raw: &str) -> Option<u64> {
    raw.trim()
        .parse::<u64>()
        .ok()
        .filter(|percent| (110..=400).contains(percent))
}

fn growth_percent_from_env() -> u64 {
    let Ok(raw) = std::env::var(GROWTH_PERCENT_ENV) else {
        return DEFAULT_GROWTH_PERCENT;
    };
    parse_growth_percent(&raw).unwrap_or_else(|| {
        tracing::warn!(
            "{GROWTH_PERCENT_ENV}={raw:?} is not a percentage in 110..=400; using {DEFAULT_GROWTH_PERCENT}"
        );
        DEFAULT_GROWTH_PERCENT
    })
}

const VERTEX_SIZE: u64 = std::mem::size_of::<Vertex>() as u64;
const INDEX_SIZE: u64 = std::mem::size_of::<u32>() as u64;

#[derive(Clone, Copy, Debug)]
pub struct MeshSlot {
    pub vertex_offset: u32,
    pub vertex_count: u32,
    pub index_offset: u32,
    pub index_count: u32,
}

/// Shared vertex/index storage with refcounted, keyed slots.
///
/// Streaming churn fragments the free lists, so the buffer supports
/// compaction: live slots are repacked tightly via GPU copies and `remapped`
/// is raised — the streamer must then re-patch the offsets baked into its
/// instances (see `WorldStreamer::patch_mega_offsets`). Growth is clamped to
/// the device's `max_buffer_size`; when even compaction can't make room,
/// `allocate` returns None instead of panicking.
pub struct MegaBuffer {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    label_prefix: String,
    vertex_capacity: u32,
    index_capacity: u32,
    max_vertex_capacity: u32,
    max_index_capacity: u32,
    vertex_cursor: u32,
    index_cursor: u32,
    free_ranges_v: Vec<Range<u32>>,
    free_ranges_i: Vec<Range<u32>>,
    slots: HashMap<String, (MeshSlot, u32)>,
    remapped: bool,
    growth_percent: u64,
}

impl MegaBuffer {
    pub fn new(device: &wgpu::Device, label_prefix: &str) -> Self {
        let max_bytes = device.limits().max_buffer_size;
        let max_vertex_capacity = (max_bytes / VERTEX_SIZE).min(u64::from(u32::MAX)) as u32;
        let max_index_capacity = (max_bytes / INDEX_SIZE).min(u64::from(u32::MAX)) as u32;

        let (vertex_buffer, index_buffer) = Self::create_buffers(
            device,
            label_prefix,
            INITIAL_VERTEX_CAPACITY,
            INITIAL_INDEX_CAPACITY,
        );

        Self {
            vertex_buffer,
            index_buffer,
            label_prefix: label_prefix.to_string(),
            vertex_capacity: INITIAL_VERTEX_CAPACITY,
            index_capacity: INITIAL_INDEX_CAPACITY,
            max_vertex_capacity,
            max_index_capacity,
            vertex_cursor: 0,
            index_cursor: 0,
            free_ranges_v: Vec::new(),
            free_ranges_i: Vec::new(),
            slots: HashMap::new(),
            remapped: false,
            growth_percent: growth_percent_from_env(),
        }
    }

    fn create_buffers(
        device: &wgpu::Device,
        label_prefix: &str,
        vertex_capacity: u32,
        index_capacity: u32,
    ) -> (wgpu::Buffer, wgpu::Buffer) {
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label_prefix}_mega_vertex")),
            size: u64::from(vertex_capacity) * VERTEX_SIZE,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label_prefix}_mega_index")),
            size: u64::from(index_capacity) * INDEX_SIZE,
            usage: wgpu::BufferUsages::INDEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        (vertex_buffer, index_buffer)
    }

    /// Returns the slot plus whether it was newly uploaded (false = refcount
    /// bump on an existing key). None when the geometry cannot fit even after
    /// growing to the device limit and compacting: the caller should skip the
    /// mesh.
    pub fn allocate(
        &mut self,
        key: &str,
        vertices: &[Vertex],
        indices: &[u32],
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<(MeshSlot, bool)> {
        if let Some((slot, refcount)) = self.slots.get_mut(key) {
            *refcount += 1;
            return Some((*slot, false));
        }

        let v_count = vertices.len() as u32;
        let i_count = indices.len() as u32;

        let pair = if let Some(p) = self.try_alloc_pair(v_count, i_count) {
            Some(p)
        } else {
            let mut p = None;
            // Mostly waste (churn fragmentation)? Compact instead of
            // growing — this also shrinks the buffer back toward live size.
            if self.is_mostly_waste(v_count, i_count)
                && self.compact(device, queue, v_count, i_count)
            {
                p = self.try_alloc_pair(v_count, i_count);
            }
            if p.is_none() && self.grow(device, queue, v_count, i_count) {
                p = self.try_alloc_pair(v_count, i_count);
            }
            // At the device size cap: compaction is the last resort.
            if p.is_none() && self.compact(device, queue, v_count, i_count) {
                p = self.try_alloc_pair(v_count, i_count);
            }
            p
        };

        let Some((v_offset, i_offset)) = pair else {
            tracing::warn!(
                "{} mega buffer exhausted: {} verts / {} indices don't fit (cap {} / {})",
                self.label_prefix,
                v_count,
                i_count,
                self.max_vertex_capacity,
                self.max_index_capacity,
            );
            return None;
        };

        queue.write_buffer(
            &self.vertex_buffer,
            u64::from(v_offset) * VERTEX_SIZE,
            bytemuck::cast_slice(vertices),
        );

        queue.write_buffer(
            &self.index_buffer,
            u64::from(i_offset) * INDEX_SIZE,
            bytemuck::cast_slice(indices),
        );

        let slot = MeshSlot {
            vertex_offset: v_offset,
            vertex_count: v_count,
            index_offset: i_offset,
            index_count: i_count,
        };

        self.slots.insert(key.to_string(), (slot, 1));
        Some((slot, true))
    }

    pub fn release(&mut self, key: &str) {
        let should_free = if let Some((_, refcount)) = self.slots.get_mut(key) {
            *refcount -= 1;
            *refcount == 0
        } else {
            false
        };

        if should_free && let Some((slot, _)) = self.slots.remove(key) {
            insert_free_range(
                &mut self.free_ranges_v,
                slot.vertex_offset,
                slot.vertex_count,
            );
            insert_free_range(&mut self.free_ranges_i, slot.index_offset, slot.index_count);
        }
    }

    pub fn slot(&self, key: &str) -> Option<MeshSlot> {
        self.slots.get(key).map(|(slot, _)| *slot)
    }

    /// True once after a compaction: every offset baked from an earlier
    /// `MeshSlot` is stale and must be re-read via `slot()`.
    pub fn take_remapped(&mut self) -> bool {
        std::mem::take(&mut self.remapped)
    }

    /// Allocate vertex+index ranges atomically: a partial success returns the
    /// carved vertex range to the free list (the old code leaked it).
    fn try_alloc_pair(&mut self, v_count: u32, i_count: u32) -> Option<(u32, u32)> {
        let v = alloc_range(
            &mut self.free_ranges_v,
            v_count,
            &mut self.vertex_cursor,
            self.vertex_capacity,
        )?;
        if let Some(i) = alloc_range(
            &mut self.free_ranges_i,
            i_count,
            &mut self.index_cursor,
            self.index_capacity,
        ) {
            Some((v, i))
        } else {
            insert_free_range(&mut self.free_ranges_v, v, v_count);
            None
        }
    }

    fn live_counts(&self) -> (u64, u64) {
        let mut v = 0u64;
        let mut i = 0u64;
        for (slot, _) in self.slots.values() {
            v += u64::from(slot.vertex_count);
            i += u64::from(slot.index_count);
        }
        (v, i)
    }

    /// At least half of both buffers would be reclaimed by compacting.
    fn is_mostly_waste(&self, needed_v: u32, needed_i: u32) -> bool {
        let (live_v, live_i) = self.live_counts();
        live_v + u64::from(needed_v) <= u64::from(self.vertex_capacity) / 2
            && live_i + u64::from(needed_i) <= u64::from(self.index_capacity) / 2
    }

    /// Grow toward the device size cap, preserving the existing layout.
    /// Returns false when the (clamped) target can't fit the pending bump
    /// allocation or nothing would actually grow.
    fn grow(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        needed_v: u32,
        needed_i: u32,
    ) -> bool {
        let req_v = u64::from(self.vertex_cursor) + u64::from(needed_v);
        let req_i = u64::from(self.index_cursor) + u64::from(needed_i);

        let want_v = req_v
            .max(u64::from(self.vertex_capacity) * self.growth_percent / 100)
            .min(u64::from(self.max_vertex_capacity)) as u32;
        let want_i = req_i
            .max(u64::from(self.index_capacity) * self.growth_percent / 100)
            .min(u64::from(self.max_index_capacity)) as u32;

        if u64::from(want_v) < req_v || u64::from(want_i) < req_i {
            return false;
        }
        if want_v == self.vertex_capacity && want_i == self.index_capacity {
            return false;
        }

        let (new_vb, new_ib) = Self::create_buffers(device, &self.label_prefix, want_v, want_i);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("mega_buffer_grow"),
        });

        let old_v_bytes = u64::from(self.vertex_cursor) * VERTEX_SIZE;
        let old_i_bytes = u64::from(self.index_cursor) * INDEX_SIZE;

        if old_v_bytes > 0 {
            encoder.copy_buffer_to_buffer(&self.vertex_buffer, 0, &new_vb, 0, old_v_bytes);
        }
        if old_i_bytes > 0 {
            encoder.copy_buffer_to_buffer(&self.index_buffer, 0, &new_ib, 0, old_i_bytes);
        }

        queue.submit(std::iter::once(encoder.finish()));

        self.vertex_buffer = new_vb;
        self.index_buffer = new_ib;
        self.vertex_capacity = want_v;
        self.index_capacity = want_i;
        true
    }

    /// Repack live slots tightly into fresh buffers via GPU copies, updating
    /// the slot table and raising `remapped`. Returns false when live data
    /// plus the pending allocation exceeds the device size cap.
    fn compact(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        needed_v: u32,
        needed_i: u32,
    ) -> bool {
        let (live_v, live_i) = self.live_counts();
        let req_v = live_v + u64::from(needed_v);
        let req_i = live_i + u64::from(needed_i);

        if req_v > u64::from(self.max_vertex_capacity) || req_i > u64::from(self.max_index_capacity)
        {
            return false;
        }

        // max-then-min, not clamp(): the floor may exceed a small device cap.
        let new_v_cap = (req_v * 3 / 2)
            .max(u64::from(INITIAL_VERTEX_CAPACITY))
            .min(u64::from(self.max_vertex_capacity)) as u32;
        let new_i_cap = (req_i * 3 / 2)
            .max(u64::from(INITIAL_INDEX_CAPACITY))
            .min(u64::from(self.max_index_capacity)) as u32;

        let (new_vb, new_ib) =
            Self::create_buffers(device, &self.label_prefix, new_v_cap, new_i_cap);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("mega_buffer_compact"),
        });

        let mut cursor_v = 0u32;
        let mut cursor_i = 0u32;
        for (slot, _) in self.slots.values_mut() {
            encoder.copy_buffer_to_buffer(
                &self.vertex_buffer,
                u64::from(slot.vertex_offset) * VERTEX_SIZE,
                &new_vb,
                u64::from(cursor_v) * VERTEX_SIZE,
                u64::from(slot.vertex_count) * VERTEX_SIZE,
            );
            encoder.copy_buffer_to_buffer(
                &self.index_buffer,
                u64::from(slot.index_offset) * INDEX_SIZE,
                &new_ib,
                u64::from(cursor_i) * INDEX_SIZE,
                u64::from(slot.index_count) * INDEX_SIZE,
            );
            slot.vertex_offset = cursor_v;
            slot.index_offset = cursor_i;
            cursor_v += slot.vertex_count;
            cursor_i += slot.index_count;
        }

        queue.submit(std::iter::once(encoder.finish()));

        tracing::info!(
            "{} mega buffer compacted: {} -> {} verts capacity, {} live slots",
            self.label_prefix,
            self.vertex_capacity,
            new_v_cap,
            self.slots.len(),
        );

        self.vertex_buffer = new_vb;
        self.index_buffer = new_ib;
        self.vertex_capacity = new_v_cap;
        self.index_capacity = new_i_cap;
        self.vertex_cursor = cursor_v;
        self.index_cursor = cursor_i;
        self.free_ranges_v.clear();
        self.free_ranges_i.clear();
        self.remapped = true;
        true
    }

    pub fn vertex_capacity(&self) -> u32 {
        self.vertex_capacity
    }

    pub fn index_capacity(&self) -> u32 {
        self.index_capacity
    }

    pub fn vertex_used(&self) -> u32 {
        self.vertex_cursor
    }

    pub fn index_used(&self) -> u32 {
        self.index_cursor
    }

    pub fn mesh_count(&self) -> usize {
        self.slots.len()
    }

    /// Test-only: simulate having grown to the device cap (capacity == cap),
    /// the state the original overflow crash occurred in.
    #[cfg(test)]
    fn set_max_capacities(&mut self, max_v: u32, max_i: u32) {
        self.max_vertex_capacity = max_v;
        self.max_index_capacity = max_i;
        self.vertex_capacity = self.vertex_capacity.min(max_v);
        self.index_capacity = self.index_capacity.min(max_i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::gpu_test::test_device;
    use bytemuck::Zeroable;

    fn verts(n: usize) -> Vec<Vertex> {
        vec![Vertex::zeroed(); n]
    }

    fn inds(n: usize) -> Vec<u32> {
        vec![0u32; n]
    }

    #[test]
    #[ignore = "needs a GPU adapter"]
    fn alloc_release_reuse_compact() {
        let (device, queue) =
            test_device("mega buffer alloc release reuse compact").expect("a GPU adapter");
        let mut mb = MegaBuffer::new(&device, "test");

        // Sequential bump allocation.
        let a = mb
            .allocate("a", &verts(100), &inds(300), &device, &queue)
            .unwrap()
            .0;
        let b = mb
            .allocate("b", &verts(200), &inds(600), &device, &queue)
            .unwrap()
            .0;
        let c = mb
            .allocate("c", &verts(50), &inds(150), &device, &queue)
            .unwrap()
            .0;
        assert_eq!(a.vertex_offset, 0);
        assert_eq!(b.vertex_offset, 100);
        assert_eq!(c.vertex_offset, 300);

        // Refcounting: a second allocate of "a" pins it across one release.
        let a2 = mb
            .allocate("a", &verts(100), &inds(300), &device, &queue)
            .unwrap()
            .0;
        assert_eq!(a2.vertex_offset, a.vertex_offset);
        mb.release("a");
        assert!(mb.slot("a").is_some());
        mb.release("a");
        assert!(mb.slot("a").is_none());

        // Free-range reuse: a's range (0..100) merged with b's (100..300) into
        // 0..300; the 150-vert allocation reuses it instead of bumping.
        mb.release("b");
        let d = mb
            .allocate("d", &verts(150), &inds(450), &device, &queue)
            .unwrap()
            .0;
        assert_eq!(d.vertex_offset, 0);
        assert_eq!(mb.vertex_used(), 350, "cursor must not advance on reuse");

        // Compaction at the cap: clamp the max so growth is impossible, leave
        // plenty of fragmentation waste, and allocate something that only fits
        // after repacking.
        mb.set_max_capacities(mb.vertex_used() + 200, mb.index_used() + 600);
        // live = d(150) + c(50) = 200 verts; cap = 550. A 300-vert alloc can't
        // bump (cursor 350 + 300 > 550 after grow is capped) -> compact.
        let e = mb.allocate("e", &verts(300), &inds(900), &device, &queue);
        assert!(e.is_some(), "compaction should make room");
        assert!(mb.take_remapped(), "compaction must raise the remap flag");
        assert!(!mb.take_remapped(), "flag is take-once");

        // After compaction live slots are tightly packed.
        let live: u32 = ["c", "d", "e"]
            .iter()
            .map(|k| mb.slot(k).unwrap().vertex_count)
            .sum();
        assert_eq!(mb.vertex_used(), live);

        // Exhaustion: beyond the cap returns None instead of panicking.
        let f = mb.allocate("f", &verts(10_000), &inds(30_000), &device, &queue);
        assert!(f.is_none());
    }

    #[test]
    fn growth_percent_accepts_the_documented_bounds() {
        assert_eq!(parse_growth_percent("110"), Some(110));
        assert_eq!(parse_growth_percent("400"), Some(400));
        assert_eq!(parse_growth_percent(" 250 "), Some(250));
    }

    #[test]
    fn growth_percent_rejects_values_outside_the_bounds_and_garbage() {
        for raw in ["109", "401", "0", "", "abc", "200%", "-200", "2.5e2"] {
            assert_eq!(parse_growth_percent(raw), None, "{raw:?}");
        }
    }
}
