use super::buffers::COUNTER_BYTES;
use super::cluster_grid::MAX_VISIBLE_LIGHTS;
use crate::rendering::device_loss::DeviceLossMonitor;
use crate::rendering::readback_ring::{ReadbackError, ReadbackRing};

/// Light cull and cluster results, ~1-3 frames stale.
#[derive(Default, Clone, Copy)]
pub struct LocalLightCounters {
    pub visible: u32,
    /// Lights that passed the cull but did not fit `MAX_VISIBLE_LIGHTS`.
    pub visible_overflow: u32,
    /// Clusters whose list hit `MAX_LIGHTS_PER_CLUSTER` and dropped lights.
    pub overflowing_clusters: u32,
}

impl LocalLightCounters {
    fn from_raw(appended: u32, overflowing_clusters: u32) -> Self {
        let visible = appended.min(MAX_VISIBLE_LIGHTS);
        Self {
            visible,
            visible_overflow: appended - visible,
            overflowing_clusters,
        }
    }
}

/// Copies the light counters back through a non-blocking readback ring.
pub struct LocalLightStatsReadback {
    ring: ReadbackRing,
    pub latest: LocalLightCounters,
}

impl LocalLightStatsReadback {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            ring: ReadbackRing::new(device, "local_light_stats", COUNTER_BYTES),
            latest: LocalLightCounters::default(),
        }
    }

    /// Encode after the cluster build so the copy sees both counters.
    pub fn encode_copy(&mut self, encoder: &mut wgpu::CommandEncoder, counters: &wgpu::Buffer) {
        let Some(slot) = self.ring.acquire() else {
            return;
        };
        encoder.copy_buffer_to_buffer(counters, 0, self.ring.buffer(slot), 0, COUNTER_BYTES);
    }

    pub fn post_submit(&mut self) {
        self.ring.post_submit();
    }

    pub fn poll(
        &mut self,
        device: &wgpu::Device,
        device_loss: &DeviceLossMonitor,
    ) -> Result<(), ReadbackError> {
        let latest = &mut self.latest;
        self.ring.poll(device, device_loss, |_slot, data| {
            let counts: &[u32] = bytemuck::cast_slice(&data[..COUNTER_BYTES as usize]);
            *latest = LocalLightCounters::from_raw(counts[0], counts[1]);
        })
    }
}
