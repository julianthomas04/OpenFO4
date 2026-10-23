use crate::rendering::device_loss::DeviceLossMonitor;
use crate::rendering::gpu_driven::DRAW_BUCKET_COUNT;
use crate::rendering::readback_ring::{ReadbackError, ReadbackRing};

/// Post-cull draw counts read back from the GPU, ~1-3 frames stale. The only
/// way to see whether GPU culling actually works.
#[derive(Default, Clone, Copy)]
pub struct CullStats {
    pub object_buckets: [u32; DRAW_BUCKET_COUNT],
    pub terrain: u32,
}

impl CullStats {
    pub fn objects_total(&self) -> u32 {
        self.object_buckets.iter().sum()
    }
}

const OBJECT_BYTES: u64 = (DRAW_BUCKET_COUNT * std::mem::size_of::<u32>()) as u64;
const SLOT_BYTES: u64 = OBJECT_BYTES + std::mem::size_of::<u32>() as u64;

/// Copies the cull count buffers back to the CPU through a non-blocking
/// readback ring.
pub struct CullStatsReadback {
    ring: ReadbackRing,
    pub latest: CullStats,
}

impl CullStatsReadback {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            ring: ReadbackRing::new(device, "cull_stats", SLOT_BYTES),
            latest: CullStats::default(),
        }
    }

    /// Encode count-buffer copies into the current slot. Must be encoded after
    /// the cull dispatches so the copies observe post-cull counts.
    pub fn encode_copy(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        object_counts: &wgpu::Buffer,
        terrain_count: &wgpu::Buffer,
    ) {
        let Some(slot) = self.ring.acquire() else {
            return;
        };
        let staging = self.ring.buffer(slot);
        encoder.copy_buffer_to_buffer(object_counts, 0, staging, 0, OBJECT_BYTES);
        encoder.copy_buffer_to_buffer(terrain_count, 0, staging, OBJECT_BYTES, 4);
    }

    pub fn post_submit(&mut self) {
        self.ring.post_submit();
    }

    /// Non-blocking: harvests any completed readbacks into `latest`.
    pub fn poll(
        &mut self,
        device: &wgpu::Device,
        device_loss: &DeviceLossMonitor,
    ) -> Result<(), ReadbackError> {
        let latest = &mut self.latest;
        self.ring.poll(device, device_loss, |_slot, data| {
            let counts: &[u32] = bytemuck::cast_slice(&data[..SLOT_BYTES as usize]);
            let mut stats = CullStats::default();
            stats
                .object_buckets
                .copy_from_slice(&counts[..DRAW_BUCKET_COUNT]);
            stats.terrain = counts[DRAW_BUCKET_COUNT];
            *latest = stats;
        })
    }
}
