use std::time::{Duration, Instant};

use glam::UVec4;

use super::residency::TextureResidency;
use crate::rendering::device_loss::DeviceLossMonitor;
use crate::rendering::gpu_driven::{MAX_TEXTURES, TextureArray};
use crate::rendering::readback_ring::{ReadbackError, ReadbackRing, SLOTS};
use crate::utils;

pub const FEEDBACK_ENV: &str = "OPENFO4_TEX_FEEDBACK";

/// How often the GPU's findings are read back.
const READBACK_INTERVAL: Duration = Duration::from_millis(100);
/// One pixel in `SAMPLE_MASK + 1` reports (a power of two minus one). Sparse
/// enough that the atomics stay cheap, dense enough that every texture on
/// screen is seen many times within one readback interval.
const SAMPLE_MASK: u32 = 31;
/// Mirrors `FEEDBACK_ENCODE` in `shaders/texture_feedback.wgsl`: a slot holds
/// `FEEDBACK_ENCODE - mip` so that zero (a cleared buffer) means "not sampled".
const FEEDBACK_ENCODE: u32 = 16;
const BUFFER_BYTES: u64 = MAX_TEXTURES as u64 * std::mem::size_of::<u32>() as u64;

/// What the GPU actually samples, as a safety net for the streaming estimate:
/// a sparse subset of the main view's pixels record, per texture slot, the
/// finest mip their footprint calls for (`atomicMax` into a buffer cleared at
/// every readback). The findings only ever make a texture finer.
pub struct TextureFeedback {
    enabled: bool,
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    ring: ReadbackRing,
    /// `recycle_epoch` when the accumulation window of each ring slot began.
    window_starts: [u64; SLOTS],
    /// `recycle_epoch` at the last clear, i.e. the start of the running window.
    window_start: u64,
    frame: u32,
    last_readback: Instant,
    last_report: Option<Instant>,
}

impl TextureFeedback {
    /// Enabled unless `OPENFO4_TEX_FEEDBACK=0`.
    pub fn new(device: &wgpu::Device, bind_group_layout: &wgpu::BindGroupLayout) -> Self {
        let enabled = utils::env_flag(FEEDBACK_ENV, true);
        tracing::info!("texture feedback {}", if enabled { "on" } else { "off" });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("texture_feedback"),
            size: BUFFER_BYTES,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("texture_feedback_bind_group"),
            layout: bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        Self {
            enabled,
            buffer,
            bind_group,
            ring: ReadbackRing::new(device, "texture_feedback", BUFFER_BYTES),
            window_starts: [0; SLOTS],
            window_start: 0,
            frame: 0,
            last_readback: Instant::now(),
            last_report: None,
        }
    }

    /// Fixed for the process lifetime: pipelines compile the feedback in or out.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    /// The frame uniform's `texture_feedback`: x enabled, y frame index (varies
    /// which pixels report), z sample mask, w `iTexMipMapSkip`. Call once per
    /// frame.
    pub fn frame_params(&mut self, global_skip: u32) -> UVec4 {
        self.frame = self.frame.wrapping_add(1);
        UVec4::new(
            u32::from(self.enabled),
            self.frame,
            SAMPLE_MASK,
            global_skip,
        )
    }

    /// Encode the copy of the findings into the readback ring, then clear them.
    /// Must be encoded after the world pass. A full ring skips the readback and
    /// keeps accumulating. `recycle_epoch` (the residency's) is stamped so
    /// findings for a texture that left its slot meanwhile can be dropped.
    pub fn encode_readback(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        now: Instant,
        recycle_epoch: u64,
    ) {
        if !self.enabled || now.duration_since(self.last_readback) < READBACK_INTERVAL {
            return;
        }
        let Some(slot) = self.ring.acquire() else {
            return;
        };
        encoder.copy_buffer_to_buffer(&self.buffer, 0, self.ring.buffer(slot), 0, BUFFER_BYTES);
        encoder.clear_buffer(&self.buffer, 0, None);
        self.window_starts[slot] = self.window_start;
        self.window_start = recycle_epoch;
        self.last_readback = now;
    }

    pub fn post_submit(&mut self) {
        self.ring.post_submit();
    }

    /// Non-blocking: hand every finished readback to the residency. With
    /// `publish_skips` (a mip view is shown) the held per-slot findings are then
    /// also published to `array` for the texture waste view.
    pub fn poll(
        &mut self,
        device: &wgpu::Device,
        device_loss: &DeviceLossMonitor,
        residency: &mut TextureResidency,
        array: &mut TextureArray,
        publish_skips: bool,
    ) -> Result<(), ReadbackError> {
        let last_report = &mut self.last_report;
        let window_starts = &self.window_starts;
        self.ring.poll(device, device_loss, |ring_slot, data| {
            let window_start = window_starts[ring_slot];
            let now = Instant::now();
            let findings: &[u32] = bytemuck::cast_slice(data);
            for (slot, &encoded) in findings.iter().enumerate() {
                let recycled = residency.recycled_since(slot as u32, window_start);
                if (1..=FEEDBACK_ENCODE).contains(&encoded) && !recycled {
                    residency.note_feedback(slot as u32, FEEDBACK_ENCODE - encoded, now);
                }
            }
            if publish_skips {
                residency.publish_feedback_skips(array, now);
            }
            *last_report = Some(now);
        })
    }

    /// Milliseconds since feedback was last processed; `None` when off or
    /// before the first report.
    pub fn age_ms(&self) -> Option<u32> {
        self.last_report
            .filter(|_| self.enabled)
            .map(|reported| reported.elapsed().as_millis().min(u128::from(u32::MAX)) as u32)
    }
}
