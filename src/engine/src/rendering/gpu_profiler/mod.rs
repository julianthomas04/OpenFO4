pub mod panel;
mod passes;

use std::cell::Cell;

pub use passes::{GpuPass, SpanPart};

use crate::rendering::device_loss::DeviceLossMonitor;
use crate::rendering::readback_ring::{ReadbackError, ReadbackRing, SLOTS};

const TIMESTAMP_BYTES: usize = std::mem::size_of::<u64>();
/// Each pass resolves into its own aligned slot, because only the passes that
/// actually ran this frame may be resolved: an unwritten query has no defined
/// result and the Vulkan resolve waits for it.
const RESOLVE_STRIDE: u64 = wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;
const RESOLVE_BYTES: u64 = RESOLVE_STRIDE * GpuPass::COUNT as u64;
const NANOSECONDS_PER_MILLISECOND: f64 = 1_000_000.0;
const SMOOTHING_WINDOW_FRAMES: f32 = 30.0;
const SMOOTHING_ALPHA: f32 = 2.0 / (SMOOTHING_WINDOW_FRAMES + 1.0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProfilerStatus {
    Unsupported,
    Active,
}

struct Recorder {
    query_set: wgpu::QuerySet,
    resolve_buffer: wgpu::Buffer,
    ring: ReadbackRing,
    milliseconds_per_tick: f64,
}

type TimestampEdges<'a> = (&'a wgpu::QuerySet, Option<u32>, Option<u32>);

fn fold_sample(smoothed: &mut Option<f32>, sample: f32) {
    *smoothed = Some(match *smoothed {
        Some(previous) => previous + (sample - previous) * SMOOTHING_ALPHA,
        None => sample,
    });
}

/// Per-pass GPU time from pass-level timestamp queries, read back through a
/// non-blocking ring (~2-3 frames stale) and smoothed with an EMA. Without
/// `TIMESTAMP_QUERY` every writes call returns `None` and nothing is recorded.
pub struct GpuProfiler {
    recorder: Option<Recorder>,
    frame_slot: Option<usize>,
    begun_passes: Cell<u32>,
    ended_passes: Cell<u32>,
    slot_recorded_passes: [u32; SLOTS],
    smoothed_ms: [Option<f32>; GpuPass::COUNT],
    skipped_frames: u64,
    invalid_samples: u64,
}

impl GpuProfiler {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let recorder = if device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            Some(Recorder::new(device, queue))
        } else {
            tracing::info!(
                "GPU timestamp queries unsupported by this adapter; GPU profiling disabled"
            );
            None
        };

        Self {
            recorder,
            frame_slot: None,
            begun_passes: Cell::new(0),
            ended_passes: Cell::new(0),
            slot_recorded_passes: [0; SLOTS],
            smoothed_ms: [None; GpuPass::COUNT],
            skipped_frames: 0,
            invalid_samples: 0,
        }
    }

    pub fn status(&self) -> ProfilerStatus {
        match self.recorder {
            Some(_) => ProfilerStatus::Active,
            None => ProfilerStatus::Unsupported,
        }
    }

    pub fn skipped_frames(&self) -> u64 {
        self.skipped_frames
    }

    pub fn invalid_samples(&self) -> u64 {
        self.invalid_samples
    }

    pub fn smoothed_ms(&self, pass: GpuPass) -> Option<f32> {
        self.smoothed_ms[pass.index()]
    }

    /// Sum of the smoothed pass times. Passes can overlap on the GPU, so this
    /// is a budget figure rather than the exact frame time.
    pub fn total_ms(&self) -> f32 {
        self.smoothed_ms.iter().flatten().sum()
    }

    /// Claims this frame's readback slot; a full ring skips profiling for the
    /// whole frame.
    pub fn begin_frame(&mut self) {
        let Some(recorder) = &mut self.recorder else {
            return;
        };
        self.begun_passes.set(0);
        self.ended_passes.set(0);
        self.frame_slot = recorder.ring.acquire();
        if self.frame_slot.is_none() {
            self.skipped_frames += 1;
        }
    }

    pub fn compute_writes(
        &self,
        pass: GpuPass,
        part: SpanPart,
    ) -> Option<wgpu::ComputePassTimestampWrites<'_>> {
        let (query_set, beginning_of_pass_write_index, end_of_pass_write_index) =
            self.claim(pass, part)?;
        Some(wgpu::ComputePassTimestampWrites {
            query_set,
            beginning_of_pass_write_index,
            end_of_pass_write_index,
        })
    }

    pub fn render_writes(
        &self,
        pass: GpuPass,
        part: SpanPart,
    ) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        let (query_set, beginning_of_pass_write_index, end_of_pass_write_index) =
            self.claim(pass, part)?;
        Some(wgpu::RenderPassTimestampWrites {
            query_set,
            beginning_of_pass_write_index,
            end_of_pass_write_index,
        })
    }

    /// Resolves the passes that ran this frame into the readback slot. Encode
    /// after the last profiled pass.
    pub fn end_frame(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let (Some(recorder), Some(slot)) = (&self.recorder, self.frame_slot) else {
            return;
        };
        let recorded = self.begun_passes.get() & self.ended_passes.get();
        for pass in GpuPass::ALL
            .iter()
            .filter(|pass| recorded & pass.bit() != 0)
        {
            encoder.resolve_query_set(
                &recorder.query_set,
                pass.query_range(),
                &recorder.resolve_buffer,
                pass.index() as u64 * RESOLVE_STRIDE,
            );
        }
        encoder.copy_buffer_to_buffer(
            &recorder.resolve_buffer,
            0,
            recorder.ring.buffer(slot),
            0,
            RESOLVE_BYTES,
        );
        self.slot_recorded_passes[slot] = recorded;
    }

    pub fn post_submit(&mut self) {
        if let Some(recorder) = &mut self.recorder {
            recorder.ring.post_submit();
        }
        self.frame_slot = None;
    }

    /// Non-blocking: folds any completed readbacks into the smoothed times.
    pub fn poll(
        &mut self,
        device: &wgpu::Device,
        device_loss: &DeviceLossMonitor,
    ) -> Result<(), ReadbackError> {
        let Some(recorder) = &mut self.recorder else {
            return Ok(());
        };
        let milliseconds_per_tick = recorder.milliseconds_per_tick;
        let slot_recorded_passes = &self.slot_recorded_passes;
        let smoothed_ms = &mut self.smoothed_ms;
        let invalid_samples = &mut self.invalid_samples;

        recorder.ring.poll(device, device_loss, |slot, data| {
            for pass in GpuPass::ALL {
                let smoothed = &mut smoothed_ms[pass.index()];
                if slot_recorded_passes[slot] & pass.bit() == 0 {
                    if pass.is_amortised() {
                        fold_sample(smoothed, 0.0);
                    } else {
                        *smoothed = None;
                    }
                    continue;
                }
                match pass_duration_ms(data, *pass, milliseconds_per_tick) {
                    Some(sample) => fold_sample(smoothed, sample),
                    None => *invalid_samples += 1,
                }
            }
        })
    }

    /// Records which edges of the pass get a timestamp this frame. `None` when
    /// profiling is off, the frame was skipped, or the part writes nothing.
    fn claim(&self, pass: GpuPass, part: SpanPart) -> Option<TimestampEdges<'_>> {
        let recorder = self.recorder.as_ref()?;
        self.frame_slot?;
        let begin = part.writes_begin().then(|| pass.begin_query());
        let end = part.writes_end().then(|| pass.end_query());
        if begin.is_none() && end.is_none() {
            return None;
        }
        if begin.is_some() {
            self.begun_passes.set(self.begun_passes.get() | pass.bit());
        }
        if end.is_some() {
            self.ended_passes.set(self.ended_passes.get() | pass.bit());
        }
        Some((&recorder.query_set, begin, end))
    }
}

impl Recorder {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self {
            query_set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("gpu_profiler_queries"),
                ty: wgpu::QueryType::Timestamp,
                count: GpuPass::QUERY_COUNT,
            }),
            resolve_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gpu_profiler_resolve"),
                size: RESOLVE_BYTES,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            ring: ReadbackRing::new(device, "gpu_profiler", RESOLVE_BYTES),
            milliseconds_per_tick: f64::from(queue.get_timestamp_period())
                / NANOSECONDS_PER_MILLISECOND,
        }
    }
}

/// `None` when the end timestamp precedes the begin one, which the GPU
/// reports after a clock disruption such as a power-state change.
fn pass_duration_ms(data: &[u8], pass: GpuPass, milliseconds_per_tick: f64) -> Option<f32> {
    let offset = pass.index() * RESOLVE_STRIDE as usize;
    let read = |at: usize| bytemuck::pod_read_unaligned::<u64>(&data[at..at + TIMESTAMP_BYTES]);
    let begin = read(offset);
    let end = read(offset + TIMESTAMP_BYTES);
    let ticks = end.checked_sub(begin)?;
    Some((ticks as f64 * milliseconds_per_tick) as f32)
}
