use std::marker::PhantomData;
use std::ops::Range;

use bytemuck::Pod;

use crate::rendering::free_list;

/// Per-element-type sizing of a [`GpuSlab`].
pub trait SlabElement: Pod {
    /// Elements allocated before the first growth copy.
    const INITIAL_CAPACITY: u32;
    /// Holes tolerated before `needs_compaction` can fire, so a small world
    /// never repacks.
    const COMPACTION_FLOOR: u32;
}

/// Slab allocator over the element index space of one storage buffer: the
/// streamer allocates one contiguous range per cell and writes only that
/// range, instead of re-uploading every resident element on each membership
/// change (for object instances that was ~48 MB / 10-15 ms per integrate
/// frame).
///
/// Freed ranges are zero-filled on the GPU, so an all-zero element must be
/// inert to its consumer (a zeroed instance has `index_count == 0`, which the
/// cull shader skips). Holes then cost one wasted compute lane each and
/// critically no extra storage buffer: they live inside the existing buffer,
/// and the consuming bind group's storage-buffer count stays fixed. Dispatches
/// must cover `dispatch_count()` (the high-water cursor), not the live count.
/// Fragmentation is bounded by `needs_compaction` plus the owner's repack.
///
/// Zeroing uses `queue.write_buffer`, never `encoder.clear_buffer` in the
/// frame encoder: pending `write_buffer` staging flushes at the start of a
/// submit, so an encoder clear would land *after* same-frame reuse writes
/// and wipe them.
pub struct GpuSlab<T: SlabElement> {
    pub buffer: wgpu::Buffer,
    label: String,
    capacity: u32,
    max_capacity: u32,
    /// High-water mark; also the dispatch bound.
    cursor: u32,
    free_ranges: Vec<Range<u32>>,
    live: u32,
    /// Sticky until taken: the buffer handle changed, bind groups referencing
    /// it must be rebuilt.
    reallocated: bool,
    element: PhantomData<T>,
}

impl<T: SlabElement> GpuSlab<T> {
    const ELEMENT_SIZE: u64 = std::mem::size_of::<T>() as u64;

    pub fn new(device: &wgpu::Device, label: &str) -> Self {
        let max_capacity =
            (device.limits().max_buffer_size / Self::ELEMENT_SIZE).min(u64::from(u32::MAX)) as u32;
        let capacity = T::INITIAL_CAPACITY.min(max_capacity);
        let buffer = Self::create_buffer(device, label, capacity);

        Self {
            buffer,
            label: label.to_string(),
            capacity,
            max_capacity,
            cursor: 0,
            free_ranges: Vec::new(),
            live: 0,
            reallocated: false,
            element: PhantomData,
        }
    }

    fn create_buffer(device: &wgpu::Device, label: &str, capacity: u32) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: u64::from(capacity) * Self::ELEMENT_SIZE,
            // COPY_SRC: growth preserves contents via buffer-to-buffer copy.
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    /// Reserve a `count`-element range. Best-fit over freed ranges, then
    /// bump, then grow toward the device cap. None means even growth can't
    /// fit it — the caller should trigger a repack and retry, or degrade.
    pub fn allocate(
        &mut self,
        count: u32,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<u32> {
        if count == 0 {
            return None;
        }
        let offset = if let Some(o) = free_list::alloc_range(
            &mut self.free_ranges,
            count,
            &mut self.cursor,
            self.capacity,
        ) {
            o
        } else {
            // Bump space exhausted: grow (the free list was already
            // searched and can only have lost ranges to this call).
            let needed = self.cursor.checked_add(count)?;
            if !self.grow(device, queue, needed) {
                return None;
            }
            free_list::alloc_range(
                &mut self.free_ranges,
                count,
                &mut self.cursor,
                self.capacity,
            )?
        };
        self.live += count;
        Some(offset)
    }

    pub fn write(&self, offset: u32, elements: &[T], queue: &wgpu::Queue) {
        if elements.is_empty() {
            return;
        }
        queue.write_buffer(
            &self.buffer,
            u64::from(offset) * Self::ELEMENT_SIZE,
            bytemuck::cast_slice(elements),
        );
    }

    /// Return a range to the free list and zero it on the GPU so the stale
    /// elements stop being consumed.
    pub fn free(&mut self, offset: u32, count: u32, queue: &wgpu::Queue) {
        if count == 0 {
            return;
        }
        debug_assert!(offset + count <= self.cursor, "freeing past the cursor");
        free_list::insert_free_range(&mut self.free_ranges, offset, count);
        self.live -= count;
        let zeros = vec![0u8; count as usize * Self::ELEMENT_SIZE as usize];
        queue.write_buffer(&self.buffer, u64::from(offset) * Self::ELEMENT_SIZE, &zeros);
    }

    /// Compaction endpoint: replace the whole slab contents with a tightly
    /// packed array (the caller re-derives it from per-cell CPU copies and
    /// updates its range handles). Keeps the existing buffer when it fits —
    /// the stale tail `[len, old cursor)` is never scanned (dispatch stops at
    /// the cursor) and every future allocation is fully overwritten by its
    /// own write. Returns the uploaded count: ranges at or past it didn't fit
    /// even after growth (device cap) and must be dropped by the caller.
    pub fn reset_packed(
        &mut self,
        packed: &[T],
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> u32 {
        let mut len = packed.len() as u32;
        if len > self.capacity && !self.grow(device, queue, len) {
            tracing::warn!(
                "{} slab can't fit {} packed elements even at the device cap {}; \
                 dropping the tail",
                self.label,
                len,
                self.max_capacity,
            );
            len = self.capacity;
        }
        if len > 0 {
            queue.write_buffer(
                &self.buffer,
                0,
                bytemuck::cast_slice(&packed[..len as usize]),
            );
        }
        self.cursor = len;
        self.live = len;
        self.free_ranges.clear();
        len
    }

    /// Grow so total capacity ≥ `needed`, preserving contents. Mirrors
    /// `MegaBuffer::grow` (`write_buffer` is queue-ordered, so writes enqueued
    /// earlier this frame are visible to the copy).
    fn grow(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, needed: u32) -> bool {
        let want = u64::from(needed)
            .max(u64::from(self.capacity) * 3 / 2)
            .min(u64::from(self.max_capacity)) as u32;
        if want < needed || want == self.capacity {
            return false;
        }

        let new_buffer = Self::create_buffer(device, &self.label, want);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpu_slab_grow"),
        });
        if self.cursor > 0 {
            encoder.copy_buffer_to_buffer(
                &self.buffer,
                0,
                &new_buffer,
                0,
                u64::from(self.cursor) * Self::ELEMENT_SIZE,
            );
        }
        queue.submit(std::iter::once(encoder.finish()));

        self.buffer = new_buffer;
        self.capacity = want;
        self.reallocated = true;
        true
    }

    /// Dispatch bound: every slot below the high-water cursor, holes included
    /// (they self-skip because a zeroed element is inert).
    pub fn dispatch_count(&self) -> u32 {
        self.cursor
    }

    /// Currently allocated elements — the bound on emitted work; use for
    /// sizing and UI, never for the dispatch bound.
    pub fn live_count(&self) -> u32 {
        self.live
    }

    pub fn hole_count(&self) -> u32 {
        self.cursor - self.live
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Waste cap: repack once holes exceed a third of the live set (with the
    /// element type's floor so a small world never bothers). Keeps the
    /// dispatch's hole-scan overhead ≤ ~1.33x live.
    pub fn needs_compaction(&self) -> bool {
        self.hole_count() > (self.live / 3).max(T::COMPACTION_FLOOR)
    }

    pub fn take_reallocated(&mut self) -> bool {
        std::mem::take(&mut self.reallocated)
    }
}
