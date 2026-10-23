use std::time::{Duration, Instant};

use super::gpu_profiler::{GpuPass, GpuProfiler};
use super::grass::GrassStats;
use super::lighting::frame_uniform::MAX_CASCADES;
use super::lighting::state::ExposureReadout;
use super::local_lights::stats::LocalLightCounters;
use super::shadows::ShadowDrawStats;
use super::shadows::cascade_cache::{ReasonCounts, RefreshReason};
use super::shadows::lamps::cache::LampShadowStats;
use super::shadows::shadow_cull::CasterKind;
use super::texture_streaming::TextureStats;
use crate::navmesh::NavStats;

const LINE_INTERVAL: Duration = Duration::from_secs(1);
/// `generate_allocator_report` walks every live allocation and hitches the
/// frame, so VRAM is sampled this rarely and the last sample repeats on the
/// lines in between.
const VRAM_SAMPLE_INTERVAL: Duration = Duration::from_secs(10);

pub struct PerfLog {
    last_line: Instant,
    last_frame_end: Option<Instant>,
    frame_ms: Vec<f32>,
    update_ms_sum: f32,
    render_ms_sum: f32,
    last_textures: TextureStats,
    last_cascade_refreshes: ReasonCounts,
    vram: Option<(Instant, [String; 2])>,
}

/// CPU timing of the frames since the previous PERF line.
struct IntervalTimings {
    frame_p50: f32,
    frame_p99: f32,
    frame_max: f32,
    update_mean: f32,
    render_mean: f32,
}

pub struct PerfFrame<'a> {
    pub fps: f64,
    pub profiler: &'a GpuProfiler,
    pub lights_loaded: u32,
    pub local_lights: LocalLightCounters,
    pub lamp_shadows: &'a LampShadowStats,
    pub shadow_draws: &'a ShadowDrawStats,
    /// Cumulative cascade renders.
    pub cascade_refreshes: ReasonCounts,
    pub exposure: ExposureReadout,
    pub textures: TextureStats,
    pub world_draws: u32,
    pub object_slots: u32,
    pub object_live: u32,
    pub grass: GrassStats,
    pub navmesh: NavStats,
    pub device: &'a wgpu::Device,
}

const BYTES_PER_MB: f64 = 1024.0 * 1024.0;

impl Default for PerfLog {
    fn default() -> Self {
        Self::new()
    }
}

impl PerfLog {
    pub fn new() -> Self {
        Self {
            last_line: Instant::now(),
            last_frame_end: None,
            frame_ms: Vec::new(),
            update_ms_sum: 0.0,
            render_ms_sum: 0.0,
            last_textures: TextureStats::default(),
            last_cascade_refreshes: Default::default(),
            vram: None,
        }
    }

    /// Called once per frame after update and render; the frame time is the
    /// wall time between consecutive calls, so it includes present/vsync waits.
    pub fn record_frame(&mut self, update: Duration, render: Duration) {
        let now = Instant::now();
        if let Some(previous) = self.last_frame_end.replace(now) {
            self.frame_ms
                .push(now.duration_since(previous).as_secs_f32() * 1000.0);
        }
        self.update_ms_sum += update.as_secs_f32() * 1000.0;
        self.render_ms_sum += render.as_secs_f32() * 1000.0;
    }

    fn drain_timings(&mut self) -> Option<IntervalTimings> {
        if self.frame_ms.is_empty() {
            return None;
        }
        self.frame_ms.sort_by(f32::total_cmp);
        let count = self.frame_ms.len();
        let nearest_rank = |quantile: f32| {
            self.frame_ms[((count as f32 * quantile).ceil() as usize).clamp(1, count) - 1]
        };
        let timings = IntervalTimings {
            frame_p50: nearest_rank(0.50),
            frame_p99: nearest_rank(0.99),
            frame_max: self.frame_ms[count - 1],
            update_mean: self.update_ms_sum / count as f32,
            render_mean: self.render_ms_sum / count as f32,
        };
        self.frame_ms.clear();
        self.update_ms_sum = 0.0;
        self.render_ms_sum = 0.0;
        Some(timings)
    }

    /// Whether `emit_if_due` would print now, so callers can skip gathering a frame's stats.
    pub fn is_due(&self) -> bool {
        self.last_line.elapsed() >= LINE_INTERVAL
    }

    pub fn emit_if_due(&mut self, frame: &PerfFrame) {
        let now = Instant::now();
        if now.duration_since(self.last_line) < LINE_INTERVAL {
            return;
        }
        self.last_line = now;
        let timings = self.drain_timings();
        let mut interval_fields = self.texture_fields(&frame.textures);
        interval_fields.extend(self.cascade_refresh_fields(&frame.cascade_refreshes));
        let vram = self.vram_fields(frame.device, now);
        tracing::info!(
            "{}",
            format_line(frame, timings.as_ref(), &interval_fields, &vram)
        );
    }

    /// Per cascade: renders since the previous line, then their non-zero
    /// reasons (`-` when there were none).
    fn cascade_refresh_fields(&mut self, now: &ReasonCounts) -> Vec<String> {
        let before = std::mem::replace(&mut self.last_cascade_refreshes, *now);
        (0..MAX_CASCADES)
            .flat_map(|cascade| {
                let deltas = RefreshReason::ALL.map(|reason| {
                    now[cascade][reason as usize].saturating_sub(before[cascade][reason as usize])
                });
                let reasons: Vec<String> = RefreshReason::ALL
                    .iter()
                    .zip(deltas)
                    .filter(|&(_, count)| count > 0)
                    .map(|(reason, count)| format!("{}:{count}", reason.label()))
                    .collect();
                [
                    format!("cascade{cascade}_refreshes={}", deltas.iter().sum::<u64>()),
                    format!(
                        "cascade{cascade}_refresh_reasons={}",
                        if reasons.is_empty() {
                            "-".to_owned()
                        } else {
                            reasons.join(",")
                        }
                    ),
                ]
            })
            .collect()
    }

    /// The last allocator sample, refreshed at most every `VRAM_SAMPLE_INTERVAL`.
    fn vram_fields(&mut self, device: &wgpu::Device, now: Instant) -> [String; 2] {
        let due = self
            .vram
            .as_ref()
            .is_none_or(|(sampled, _)| now.duration_since(*sampled) >= VRAM_SAMPLE_INTERVAL);
        if due {
            self.vram = Some((now, sample_vram(device)));
        }
        self.vram
            .as_ref()
            .map(|(_, fields)| fields.clone())
            .unwrap_or_default()
    }

    /// Gauges as-is, counters as the delta since the previous PERF line.
    fn texture_fields(&mut self, now: &TextureStats) -> Vec<String> {
        let before = std::mem::replace(&mut self.last_textures, *now);
        let megabytes = |bytes: u64| bytes as f64 / BYTES_PER_MB;
        vec![
            format!("tex_resident_mb={:.0}", megabytes(now.resident_bytes)),
            format!("tex_live_slots={}", now.live_slots),
            format!("tex_pending={}", now.pending),
            format!("tex_up={}", now.upgrades.saturating_sub(before.upgrades)),
            format!(
                "tex_down={}",
                now.downgrades.saturating_sub(before.downgrades)
            ),
            format!(
                "tex_upload_mb={:.1}",
                megabytes(now.uploaded_bytes.saturating_sub(before.uploaded_bytes))
            ),
            format!(
                "tex_placeholder={}",
                now.placeholders.saturating_sub(before.placeholders)
            ),
            format!(
                "tex_slots_recycled={}",
                now.slots_recycled.saturating_sub(before.slots_recycled)
            ),
            format!(
                "tex_bg_rebuilds={}",
                now.bind_group_rebuilds
                    .saturating_sub(before.bind_group_rebuilds)
            ),
            format!(
                "tex_stale_dropped={}",
                now.stale_dropped.saturating_sub(before.stale_dropped)
            ),
            format!(
                "tex_feedback_upgrades={}",
                now.feedback_upgrades
                    .saturating_sub(before.feedback_upgrades)
            ),
            format!("tex_stuck={}", now.stuck),
            format!("tex_undemanded={}", now.undemanded),
            format!(
                "tex_feedback_age_ms={}",
                now.feedback_age_ms
                    .map_or_else(|| "-".to_owned(), |ms| ms.to_string())
            ),
        ]
    }
}

fn format_line(
    frame: &PerfFrame,
    timings: Option<&IntervalTimings>,
    interval_fields: &[String],
    vram_fields: &[String],
) -> String {
    let passes = GpuPass::ALL.iter().map(|pass| {
        let ms = frame
            .profiler
            .smoothed_ms(*pass)
            .map_or_else(|| String::from("-"), |ms| format!("{ms:.3}"));
        format!("{}={ms}", pass.label().replace(' ', "_"))
    });
    let timing_fields = timings.into_iter().flat_map(|t| {
        [
            format!("frame_ms_p50={:.2}", t.frame_p50),
            format!("frame_ms_p99={:.2}", t.frame_p99),
            format!("frame_ms_max={:.2}", t.frame_max),
            format!("cpu_update_ms={:.2}", t.update_mean),
            format!("cpu_render_ms={:.2}", t.render_mean),
        ]
    });
    let counters = [
        format!("world_draws={}", frame.world_draws),
        format!("object_slots={}", frame.object_slots),
        format!("object_live={}", frame.object_live),
        format!("grass_draws={}", frame.grass.draws),
        format!("grass_instances={}", frame.grass.instances),
        format!("lights_loaded={}", frame.lights_loaded),
        format!("lights_visible={}", frame.local_lights.visible),
        format!(
            "lights_visible_overflow={}",
            frame.local_lights.visible_overflow
        ),
        format!(
            "lights_overflowing_clusters={}",
            frame.local_lights.overflowing_clusters
        ),
        format!("lamp_shadows_selected={}", frame.lamp_shadows.selected),
        format!("lamp_shadows_sampled={}", frame.lamp_shadows.sampled),
        format!(
            "lamp_shadow_views_rendered={}",
            frame.lamp_shadows.views_rendered
        ),
        format!("lamp_atlas_repacks={}", frame.lamp_shadows.atlas_repacks),
        format!(
            "cascade_overflow={}",
            u8::from(frame.shadow_draws.cascade_overflow)
        ),
        format!("adapted_lum={:.4}", frame.exposure.adapted_luminance),
        format!("exposure={:.3}", frame.exposure.exposure),
        format!("nav_meshes={}", frame.navmesh.meshes),
        format!("nav_triangles={}", frame.navmesh.triangles),
        format!("nav_links_resolved={}", frame.navmesh.resolved_links),
        format!("nav_links_pending={}", frame.navmesh.pending_links),
    ];
    std::iter::once(format!("PERF fps={:.1}", frame.fps))
        .chain(timing_fields)
        .chain(passes)
        .chain(counters)
        .chain(interval_fields.iter().cloned())
        .chain(vram_fields.iter().cloned())
        .chain(draw_counts(frame.shadow_draws))
        .collect::<Vec<_>>()
        .join(" ")
}

fn draw_counts(draws: &ShadowDrawStats) -> impl Iterator<Item = String> + '_ {
    let cascades = draws
        .cascade_draws
        .iter()
        .enumerate()
        .flat_map(|(cascade, counts)| {
            [
                format!(
                    "cascade{cascade}_opaque_draws={}",
                    counts[CasterKind::Opaque.index()]
                ),
                format!(
                    "cascade{cascade}_alpha_draws={}",
                    counts[CasterKind::AlphaTest.index()]
                ),
                format!(
                    "cascade{cascade}_terrain_draws={}",
                    counts[CasterKind::Terrain.index()]
                ),
            ]
        });
    let lamps = [
        format!(
            "lamp_opaque_draws={}",
            draws.lamp_draws[CasterKind::Opaque.index()]
        ),
        format!(
            "lamp_alpha_draws={}",
            draws.lamp_draws[CasterKind::AlphaTest.index()]
        ),
        format!(
            "lamp_terrain_draws={}",
            draws.lamp_draws[CasterKind::Terrain.index()]
        ),
    ];
    cascades.chain(lamps)
}

fn sample_vram(device: &wgpu::Device) -> [String; 2] {
    let megabytes = |bytes: u64| bytes as f64 / BYTES_PER_MB;
    match device.generate_allocator_report() {
        Some(report) => [
            format!("vram_mb={:.0}", megabytes(report.total_allocated_bytes)),
            format!(
                "vram_reserved_mb={:.0}",
                megabytes(report.total_reserved_bytes)
            ),
        ],
        None => [
            String::from("vram_mb=-"),
            String::from("vram_reserved_mb=-"),
        ],
    }
}
