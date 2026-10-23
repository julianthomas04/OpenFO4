use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};

use super::demand::NO_DEMAND;
use super::policy::ResidencyPolicy;
use super::stats::TextureStats;
use crate::assets::types::texture::{self, TextureError, TextureLayout, TexturePayload};
use crate::rendering::gpu_driven::{
    MAX_TEXTURES, PLACEHOLDER_FIRST_MIP, TextureArray, TextureHandle,
};

const IO_THREADS: usize = 2;
/// Reads queued at the IO threads at any time; the rest wait in the plan.
const MAX_IN_FLIGHT: usize = 32;
/// A texture must keep wanting fewer mips this long before it is downgraded.
const DOWNGRADE_HOLD: Duration = Duration::from_secs(3);
const MAX_DOWNGRADES_PER_PUMP: usize = 8;
/// Main-thread time after which `pump` stops starting further GPU work.
const PUMP_TIME_BUDGET: Duration = Duration::from_micros(1500);
/// Upload bytes after which `pump` stops applying further reads, on top of the
/// caller's budget; a read that would cross it waits for the next call.
const PUMP_BYTE_CAP: u64 = 4 * 1024 * 1024;
/// Reads at most this large are applied even when the budget is spent.
const SMALL_READ_BYTES: u64 = 64 * 1024;
/// Bytes of finished reads held back for upload; past it no new reads start.
const READY_BYTE_CAP: u64 = 64 * 1024 * 1024;
/// A failed upgrade read is retried after a backoff, this many times at most.
const MAX_UPGRADE_FAILURES: u32 = 5;
const UPGRADE_RETRY_BASE: Duration = Duration::from_secs(2);
/// Feedback requests older than this are dropped; reuses the downgrade hold so
/// a texture the GPU asked for is not shrunk again before it is asked about.
const FEEDBACK_HOLD: Duration = DOWNGRADE_HOLD;
const PLAN_MIN_INTERVAL: Duration = Duration::from_millis(50);
const PLAN_MAX_INTERVAL: Duration = Duration::from_millis(250);

pub const STUCK_LOG_ENV: &str = "OPENFO4_TEX_STUCK_LOG";
const STUCK_CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// A texture wanting a finer mip than it holds for this long counts as stuck.
const STUCK_AFTER: Duration = Duration::from_secs(5);
const MAX_STUCK_LOGGED_PER_CHECK: u32 = 10;

/// One resident texture as the planner sees it, for `OPENFO4_TEX_TRACE`.
#[derive(Debug, Clone)]
pub struct TextureTrace {
    pub slot: u32,
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub mip_count: u32,
    pub first_mip: u32,
    pub skip_limit: u32,
    pub wanted: u32,
    pub cpu_wanted: u32,
    pub feedback_wanted: Option<u32>,
    pub demand: f32,
}

/// A resident texture nothing has asked for, found by the stuck detector.
#[derive(Debug, Clone)]
pub struct UndemandedTexture {
    pub slot: u32,
    pub path: String,
    pub first_mip: u32,
    pub mip_count: u32,
    pub seconds: f32,
}

/// Largest skip a texture may take: the policy limit, reduced until the new
/// base level keeps a block-aligned extent (a block-compressed texture cannot
/// be created otherwise).
pub fn skip_limit(layout: &TextureLayout) -> u32 {
    let aligned = |skip: u32| {
        let (width, height) = layout.mip_extent(skip);
        width % layout.block_dim == 0 && height % layout.block_dim == 0
    };
    let mut limit = ResidencyPolicy::max_skip(layout);
    while limit > 0 && !aligned(limit) {
        limit -= 1;
    }
    limit
}

/// First mip to load for a texture seen for the first time.
pub fn initial_first_mip(policy: ResidencyPolicy, layout: &TextureLayout, demand: f32) -> u32 {
    policy.initial_skip(demand, layout).min(skip_limit(layout))
}

fn target_first_mip(
    policy: ResidencyPolicy,
    layout: &TextureLayout,
    demand: f32,
    feedback_mip: Option<u32>,
    current_first_mip: u32,
) -> u32 {
    policy
        .target_skip(demand, feedback_mip, current_first_mip, layout)
        .min(skip_limit(layout))
}

/// Skip the held feedback calls for, bounded like every target.
fn feedback_first_mip(policy: ResidencyPolicy, layout: &TextureLayout, feedback_mip: u32) -> u32 {
    policy
        .feedback_skip(feedback_mip, layout)
        .min(skip_limit(layout))
}

/// A texture holding file mips `first_mip..layout.mip_count`.
struct GpuTexture {
    texture: Arc<wgpu::Texture>,
    layout: Arc<TextureLayout>,
    first_mip: u32,
    bytes: u64,
}

impl GpuTexture {
    fn new(
        device: &wgpu::Device,
        layout: &Arc<TextureLayout>,
        first_mip: u32,
        srgb: bool,
        label: &str,
    ) -> Self {
        Self {
            texture: Arc::new(texture::create_texture(
                device, layout, first_mip, srgb, label,
            )),
            layout: Arc::clone(layout),
            first_mip,
            bytes: (first_mip..layout.mip_count)
                .map(|mip| layout.mip_bytes(mip))
                .sum(),
        }
    }

    fn view(&self) -> Arc<wgpu::TextureView> {
        Arc::new(
            self.texture
                .create_view(&wgpu::TextureViewDescriptor::default()),
        )
    }

    /// Copy file mips `levels` from `source` (which must hold them) into this
    /// texture, GPU to GPU.
    fn copy_mips_from(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &GpuTexture,
        levels: Range<u32>,
    ) {
        for level in levels {
            encoder.copy_texture_to_texture(
                mip_copy_info(&source.texture, level - source.first_mip),
                mip_copy_info(&self.texture, level - self.first_mip),
                self.layout.mip_copy_extent(level),
            );
        }
    }
}

fn mip_copy_info(texture: &wgpu::Texture, mip_level: u32) -> wgpu::TexelCopyTextureInfo<'_> {
    wgpu::TexelCopyTextureInfo {
        texture,
        mip_level,
        origin: wgpu::Origin3d::ZERO,
        aspect: wgpu::TextureAspect::All,
    }
}

enum State {
    /// Bound to a stand-in view; no read requested yet.
    Placeholder,
    Resident(GpuTexture),
    /// Reading the file failed; the slot shows its role's stand-in view for good.
    Failed,
}

struct Entry {
    handle: TextureHandle,
    path: String,
    srgb: bool,
    /// Reserved slot whose view stands in for this texture until it is
    /// resident, and for good if loading fails.
    placeholder_slot: u32,
    state: State,
    /// A read for this slot is queued at the IO threads or waiting to be applied.
    in_flight: bool,
    downgrade_since: Option<Instant>,
    upgrade_failures: u32,
    upgrade_retry_at: Option<Instant>,
    /// Since when the texture has wanted a finer mip than it holds; only
    /// tracked by the stuck detector.
    behind_since: Option<Instant>,
    /// Since when the texture is coarser than full resolution and neither
    /// demand nor feedback asks for anything; only tracked by the stuck detector.
    undemanded_since: Option<Instant>,
    last_failure: Option<String>,
}

impl Entry {
    fn resident_first_mip(&self) -> Option<u32> {
        match &self.state {
            State::Resident(gpu) => Some(gpu.first_mip),
            State::Placeholder | State::Failed => None,
        }
    }

    fn may_upgrade(&self, now: Instant) -> bool {
        self.upgrade_failures < MAX_UPGRADE_FAILURES
            && self.upgrade_retry_at.is_none_or(|retry_at| now >= retry_at)
    }

    fn note_upgrade_failure(&mut self, reason: &str, now: Instant) {
        self.upgrade_failures += 1;
        self.upgrade_retry_at =
            Some(now + UPGRADE_RETRY_BASE * 2u32.pow(self.upgrade_failures - 1));
        self.last_failure = Some(reason.to_owned());
        if self.upgrade_failures >= MAX_UPGRADE_FAILURES {
            tracing::warn!(
                "texture '{}' upgrade given up for good after {} failed attempts, staying at first_mip {:?}: {reason}",
                self.path,
                self.upgrade_failures,
                self.resident_first_mip(),
            );
        }
    }

    /// Whether a texture that has wanted fewer mips since `downgrade_since`
    /// has held that wish for `DOWNGRADE_HOLD`; the first call starts the clock.
    fn downgrade_hold_elapsed(&mut self, now: Instant) -> bool {
        let since = *self.downgrade_since.get_or_insert(now);
        now.duration_since(since) >= DOWNGRADE_HOLD
    }

    fn note_upgrade_success(&mut self) {
        self.upgrade_failures = 0;
        self.upgrade_retry_at = None;
    }

    /// A load that cannot succeed: bind the role's stand-in view for good.
    fn fail_placeholder(&mut self, array: &mut TextureArray, reason: &str) {
        tracing::warn!(
            "texture '{}' load given up for good, keeping its stand-in view: {reason}",
            self.path
        );
        self.last_failure = Some(reason.to_owned());
        self.state = State::Failed;
        array.replace_view(
            self.handle,
            array.reserved_view(self.placeholder_slot),
            PLACEHOLDER_FIRST_MIP,
        );
    }
}

enum IoRequest {
    /// First load: the layout is unknown, so the IO thread picks the mip range.
    Load {
        handle: TextureHandle,
        path: String,
        demand: f32,
    },
    /// Read the mips above the resident ones: `first_mip..resident_first`.
    Upgrade {
        handle: TextureHandle,
        path: String,
        layout: Arc<TextureLayout>,
        first_mip: u32,
        resident_first: u32,
    },
}

struct IoResult {
    handle: TextureHandle,
    path: String,
    outcome: Result<Arc<TexturePayload>, TextureError>,
}

impl IoRequest {
    fn identity(&self) -> (TextureHandle, String) {
        match self {
            Self::Load { handle, path, .. } | Self::Upgrade { handle, path, .. } => {
                (*handle, path.clone())
            }
        }
    }
}

/// `serve`, with a panic in the read or parse turned into an error result so
/// the request is still answered and the in-flight accounting stays right.
fn serve_guarded(request: IoRequest, asset_root: &str, policy: ResidencyPolicy) -> IoResult {
    let (handle, path) = request.identity();
    panic::catch_unwind(AssertUnwindSafe(|| serve(request, asset_root, policy))).unwrap_or_else(
        |payload| {
            let detail = payload
                .downcast_ref::<&str>()
                .map(|message| (*message).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic payload".to_owned());
            tracing::error!("texture IO panicked on '{path}': {detail}");
            let outcome = Err(TextureError::Io {
                path: path.clone(),
                source: std::io::Error::other(format!("IO thread panicked: {detail}")),
            });
            IoResult {
                handle,
                path,
                outcome,
            }
        },
    )
}

fn serve(request: IoRequest, asset_root: &str, policy: ResidencyPolicy) -> IoResult {
    match request {
        IoRequest::Load {
            handle,
            path,
            demand,
        } => {
            let full_path = format!("{asset_root}/{path}");
            let outcome = texture::read_layout(&full_path).and_then(|layout| {
                let first = initial_first_mip(policy, &layout, demand);
                let mip_count = layout.mip_count;
                texture::read_mips(&full_path, &Arc::new(layout), first..mip_count)
            });
            IoResult {
                handle,
                path,
                outcome: outcome.map(Arc::new),
            }
        }
        IoRequest::Upgrade {
            handle,
            path,
            layout,
            first_mip,
            resident_first,
        } => {
            let full_path = format!("{asset_root}/{path}");
            let outcome = texture::read_mips(&full_path, &layout, first_mip..resident_first);
            IoResult {
                handle,
                path,
                outcome: outcome.map(Arc::new),
            }
        }
    }
}

/// A mip the GPU was seen to sample, and when it was last seen.
#[derive(Debug, Copy, Clone)]
struct FeedbackRequest {
    mip: u32,
    seen: Instant,
}

/// Decides, per bindless slot, how many top mips are resident and moves
/// textures between resolutions. The streamer feeds it demand (screen pixels
/// per UV unit, per texture user); `pump` turns demand into background reads
/// and main-thread GPU work.
pub struct TextureResidency {
    policy: ResidencyPolicy,
    entries: HashMap<u32, Entry>,
    /// Largest demand among current users, per slot; `NO_DEMAND` if unused.
    /// Indexed by slot so a demand round is a plain scatter-max.
    demand: Vec<f32>,
    /// Demand being gathered by the running sweep; replaces `demand` when it
    /// completes, so a texture whose users went away can fall back.
    sweep_demand: Vec<f32>,
    /// Finest mip the GPU was seen to sample per slot (see `note_feedback`).
    feedback: Vec<Option<FeedbackRequest>>,
    /// Bumped by every `reset_slot`; `slot_recycled` records it per slot so
    /// readbacks that predate a slot's new occupant can be told apart.
    recycle_epoch: u64,
    slot_recycled: Vec<u64>,
    requests: Sender<IoRequest>,
    results: Receiver<IoResult>,
    /// Reads finished by the IO threads, waiting for upload budget.
    ready: VecDeque<IoResult>,
    ready_bytes: u64,
    /// Pumps in a row that left finished reads waiting without applying any.
    in_flight: usize,
    last_plan: Instant,
    plan_dirty: bool,
    /// The last plan had more reads to start than free slots; re-plan as soon
    /// as reads finish instead of waiting for the next interval.
    plan_backlog: bool,
    stuck_log: bool,
    undemanded_found: Vec<UndemandedTexture>,
    last_stuck_check: Instant,
    resident_bytes: u64,
    pub stats: TextureStats,
}

impl TextureResidency {
    pub fn new(policy: ResidencyPolicy, asset_root: &str) -> Self {
        let (requests, request_rx) = crossbeam_channel::unbounded::<IoRequest>();
        let (result_tx, results) = crossbeam_channel::unbounded::<IoResult>();
        for index in 0..IO_THREADS {
            let request_rx = request_rx.clone();
            let result_tx = result_tx.clone();
            let asset_root = asset_root.to_owned();
            std::thread::Builder::new()
                .name(format!("texture_io_{index}"))
                .spawn(move || {
                    crate::utils::lower_thread_priority();
                    while let Ok(request) = request_rx.recv() {
                        if result_tx
                            .send(serve_guarded(request, &asset_root, policy))
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .expect("spawn texture IO thread");
        }
        Self {
            policy,
            entries: HashMap::new(),
            demand: vec![NO_DEMAND; MAX_TEXTURES as usize],
            sweep_demand: vec![NO_DEMAND; MAX_TEXTURES as usize],
            feedback: vec![None; MAX_TEXTURES as usize],
            recycle_epoch: 0,
            slot_recycled: vec![0; MAX_TEXTURES as usize],
            requests,
            results,
            ready: VecDeque::new(),
            ready_bytes: 0,
            in_flight: 0,
            last_plan: Instant::now(),
            plan_dirty: false,
            plan_backlog: false,
            stuck_log: crate::utils::env_flag(STUCK_LOG_ENV, false),
            undemanded_found: Vec::new(),
            last_stuck_check: Instant::now(),
            resident_bytes: 0,
            stats: TextureStats::default(),
        }
    }

    pub fn policy(&self) -> &ResidencyPolicy {
        &self.policy
    }

    /// Put a texture into the bindless array. With a preloaded `payload` the
    /// texture is resident immediately; without one the slot shows the
    /// `placeholder_slot` view until the IO threads deliver. Returns the
    /// handle and the bytes uploaded now, or `None` if the array is full.
    ///
    /// A payload that is large and not `upload_now` is queued as an already
    /// finished read instead, so the budgeted pump applies it without a
    /// second round of IO.
    pub fn admit(
        &mut self,
        array: &mut TextureArray,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: &str,
        srgb: bool,
        payload: Option<&Arc<TexturePayload>>,
        upload_now: bool,
        placeholder_slot: u32,
    ) -> Result<Option<(TextureHandle, u64)>, TextureError> {
        if let Some(handle) = array.try_add_ref(path) {
            return Ok(Some((handle, 0)));
        }
        let upload_now =
            upload_now || payload.is_some_and(|p| p.data.len() as u64 <= SMALL_READ_BYTES);
        let (payload, deferred) = match payload {
            Some(p) if !upload_now => (None, Some(Arc::clone(p))),
            other => (other, None),
        };
        let resident = match payload {
            Some(payload) => {
                let gpu = GpuTexture::new(device, &payload.layout, payload.first_mip, srgb, path);
                texture::write_payload(queue, &gpu.texture, gpu.first_mip, payload)?;
                Some(gpu)
            }
            None => None,
        };
        let (view, first_mip) = match &resident {
            Some(gpu) => (gpu.view(), gpu.first_mip),
            None => (array.reserved_view(placeholder_slot), PLACEHOLDER_FIRST_MIP),
        };
        let Some(handle) = array.acquire(path, view, first_mip) else {
            return Ok(None);
        };

        let uploaded = payload.map_or(0, |p| p.data.len() as u64);
        self.stats.uploaded_bytes += uploaded;
        let queued_read = deferred.is_some();
        let state = if let Some(gpu) = resident {
            self.resident_bytes += gpu.bytes;
            State::Resident(gpu)
        } else {
            self.stats.placeholders += 1;
            State::Placeholder
        };
        self.reset_slot(handle.slot);
        self.entries.insert(
            handle.slot,
            Entry {
                handle,
                path: path.to_owned(),
                srgb,
                placeholder_slot,
                state,
                in_flight: queued_read,
                downgrade_since: None,
                upgrade_failures: 0,
                upgrade_retry_at: None,
                behind_since: None,
                undemanded_since: None,
                last_failure: None,
            },
        );
        if let Some(payload) = deferred {
            self.in_flight += 1;
            self.ready_bytes += payload.data.len() as u64;
            self.ready.push_back(IoResult {
                handle,
                path: path.to_owned(),
                outcome: Ok(payload),
            });
        }
        self.plan_dirty = true;
        Ok(Some((handle, uploaded)))
    }

    /// The slot was evicted from the array.
    pub fn forget(&mut self, handle: TextureHandle) {
        if self
            .entries
            .get(&handle.slot)
            .is_none_or(|entry| entry.handle != handle)
        {
            return;
        }
        self.reset_slot(handle.slot);
        // An outstanding read stays counted in `in_flight` until its result
        // arrives and is dropped as stale.
        if let Some(Entry {
            state: State::Resident(gpu),
            ..
        }) = self.entries.remove(&handle.slot)
        {
            self.resident_bytes -= gpu.bytes;
        }
    }

    fn reset_slot(&mut self, slot: u32) {
        self.demand[slot as usize] = NO_DEMAND;
        self.sweep_demand[slot as usize] = NO_DEMAND;
        self.feedback[slot as usize] = None;
        self.recycle_epoch += 1;
        self.slot_recycled[slot as usize] = self.recycle_epoch;
    }

    /// Marks the present moment for `recycled_since`.
    pub fn recycle_epoch(&self) -> u64 {
        self.recycle_epoch
    }

    /// Whether `slot` got a new occupant after `epoch` was read.
    pub fn recycled_since(&self, slot: u32, epoch: u64) -> bool {
        self.slot_recycled[slot as usize] > epoch
    }

    /// One user of `slot` covering `pixels_per_uv`; the most demanding user
    /// wins, immediately and in the running sweep. The caller holds a
    /// reference on the slot, so it cannot be stale.
    pub fn note_demand(&mut self, slot: u32, pixels_per_uv: f32) {
        for demand in [
            &mut self.demand[slot as usize],
            &mut self.sweep_demand[slot as usize],
        ] {
            if pixels_per_uv > *demand {
                *demand = pixels_per_uv;
            }
        }
    }

    /// Strongest demand the running sweep has seen for `slot`.
    pub fn sweep_demand(&self, slot: u32) -> f32 {
        self.sweep_demand[slot as usize]
    }

    /// The users noted so far are the new demand; the next pump re-plans.
    /// Starts the next sweep's gathering from nothing.
    pub fn finish_demand_sweep(&mut self) {
        std::mem::swap(&mut self.demand, &mut self.sweep_demand);
        self.sweep_demand.fill(NO_DEMAND);
        self.plan_dirty = true;
    }

    /// Cells were added: let the next pump re-plan.
    pub fn end_demand_round(&mut self) {
        self.plan_dirty = true;
    }

    /// The GPU sampled `slot` at `mip` (full-resolution terms, finest seen).
    /// A request is kept for `FEEDBACK_HOLD` against coarser sightings, and
    /// refreshed by equal or finer ones, so it does not flap. It can only make
    /// the slot finer than demand alone would.
    pub fn note_feedback(&mut self, slot: u32, mip: u32, now: Instant) {
        let request = &mut self.feedback[slot as usize];
        let held_finer =
            request.is_some_and(|r| r.mip < mip && now.duration_since(r.seen) < FEEDBACK_HOLD);
        if !held_finer {
            *request = Some(FeedbackRequest { mip, seen: now });
        }
    }

    fn feedback_mip(&self, slot: u32, now: Instant) -> Option<u32> {
        self.feedback[slot as usize]
            .filter(|r| now.duration_since(r.seen) < FEEDBACK_HOLD)
            .map(|r| r.mip)
    }

    /// Hand every resident slot's held feedback skip to the array for the
    /// texture waste view: slots without a live request, or not resident, get
    /// none. Only changed entries mark the array dirty.
    pub fn publish_feedback_skips(&self, array: &mut TextureArray, now: Instant) {
        for (&slot, entry) in &self.entries {
            let skip = match &entry.state {
                State::Resident(gpu) => self
                    .feedback_mip(slot, now)
                    .map(|mip| feedback_first_mip(self.policy, &gpu.layout, mip)),
                State::Placeholder | State::Failed => None,
            };
            array.set_feedback_skip(slot, skip);
        }
    }

    /// Apply finished reads within `byte_budget`; the first read of a call is
    /// always applied whatever its size, so progress is guaranteed. Then start
    /// the new reads and downgrades the demand calls for, and submit the GPU
    /// copies. Returns the bytes written to the GPU this call.
    pub fn pump(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        array: &mut TextureArray,
        byte_budget: u64,
    ) -> u64 {
        while let Ok(result) = self.results.try_recv() {
            self.ready_bytes += result_bytes(&result);
            self.ready.push_back(result);
        }

        let started = Instant::now();
        let byte_budget = byte_budget.min(PUMP_BYTE_CAP);
        let mut encoder: Option<wgpu::CommandEncoder> = None;
        let mut uploaded = 0u64;
        let mut applied = 0usize;
        while let Some(result) = self.ready.pop_front() {
            let incoming = result_bytes(&result);
            let within_budget =
                uploaded + incoming <= byte_budget && started.elapsed() < PUMP_TIME_BUDGET;
            let may_apply = applied == 0 || within_budget;
            if !may_apply {
                self.ready.push_front(result);
                break;
            }
            self.ready_bytes -= incoming;
            applied += 1;
            uploaded += self.apply(result, device, queue, array, &mut encoder);
        }

        let now = Instant::now();
        if self.plan_backlog && self.in_flight < MAX_IN_FLIGHT / 2 {
            self.plan_dirty = true;
        }
        let plan_due = (self.plan_dirty && now.duration_since(self.last_plan) >= PLAN_MIN_INTERVAL)
            || now.duration_since(self.last_plan) >= PLAN_MAX_INTERVAL;
        if plan_due {
            self.last_plan = now;
            self.plan_dirty = false;
            self.plan(now, started, device, array, &mut encoder);
        }

        if self.stuck_log && now.duration_since(self.last_stuck_check) >= STUCK_CHECK_INTERVAL {
            self.last_stuck_check = now;
            self.report_stuck(now);
        }

        if let Some(encoder) = encoder {
            queue.submit(std::iter::once(encoder.finish()));
        }
        self.stats.resident_bytes = self.resident_bytes;
        self.stats.pending = (self.in_flight + self.ready.len()) as u32;
        uploaded
    }

    fn apply(
        &mut self,
        result: IoResult,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        array: &mut TextureArray,
        encoder: &mut Option<wgpu::CommandEncoder>,
    ) -> u64 {
        self.in_flight -= 1;
        let Some(entry) = self
            .entries
            .get_mut(&result.handle.slot)
            .filter(|e| e.handle == result.handle)
        else {
            self.stats.stale_dropped += 1;
            return 0;
        };
        entry.in_flight = false;

        let payload = match result.outcome {
            Ok(payload) => payload,
            Err(error) => {
                crate::utils::warn_once(&format!("texture-stream:{}", result.path), || {
                    format!("texture '{}' could not be streamed: {error}", result.path)
                });
                Self::note_failed_read(entry, array, &error.to_string());
                return 0;
            }
        };

        match &entry.state {
            State::Failed => return 0,
            State::Resident(old) if payload.mip_end != old.first_mip => {
                self.stats.stale_dropped += 1;
                entry.last_failure = Some(format!(
                    "stale read dropped: mips {}..{} against resident first_mip {}",
                    payload.first_mip, payload.mip_end, old.first_mip
                ));
                return 0;
            }
            State::Placeholder | State::Resident(_) => {}
        }

        let bytes = payload.data.len() as u64;
        let label = entry.path.clone();
        let new_gpu = GpuTexture::new(
            device,
            &payload.layout,
            payload.first_mip,
            entry.srgb,
            &label,
        );
        if let Err(error) =
            texture::write_payload(queue, &new_gpu.texture, new_gpu.first_mip, &payload)
        {
            crate::utils::warn_once(&format!("texture-stream:{label}"), || {
                format!("texture '{label}' upload rejected: {error}")
            });
            Self::note_failed_read(entry, array, &format!("upload rejected: {error}"));
            return 0;
        }

        if let State::Resident(old) = &entry.state {
            let encoder = encoder.get_or_insert_with(|| create_encoder(device));
            new_gpu.copy_mips_from(encoder, old, old.first_mip..old.layout.mip_count);
        }

        self.swap_in(result.handle.slot, new_gpu, array);
        self.stats.upgrades += 1;
        self.stats.uploaded_bytes += bytes;
        bytes
    }

    /// A read or upload failed: a placeholder is given up on, a resident
    /// texture keeps its resolution and retries the upgrade after a backoff.
    fn note_failed_read(entry: &mut Entry, array: &mut TextureArray, reason: &str) {
        if matches!(entry.state, State::Placeholder) {
            entry.fail_placeholder(array, reason);
        } else {
            entry.note_upgrade_failure(reason, Instant::now());
        }
    }

    /// Make `gpu` the slot's texture and rebind its view.
    fn swap_in(&mut self, slot: u32, gpu: GpuTexture, array: &mut TextureArray) {
        let Some(entry) = self.entries.get_mut(&slot) else {
            return;
        };
        if matches!(entry.state, State::Placeholder) {
            array.resolve_placeholder(entry.handle, gpu.view(), gpu.first_mip);
        } else {
            array.replace_view(entry.handle, gpu.view(), gpu.first_mip);
        }
        self.resident_bytes += gpu.bytes;
        if let State::Resident(old) = std::mem::replace(&mut entry.state, State::Resident(gpu)) {
            self.resident_bytes -= old.bytes;
        }
        entry.downgrade_since = None;
        entry.note_upgrade_success();
    }

    fn plan(
        &mut self,
        now: Instant,
        pump_started: Instant,
        device: &wgpu::Device,
        array: &mut TextureArray,
        encoder: &mut Option<wgpu::CommandEncoder>,
    ) {
        let policy = self.policy;
        let mut loads: Vec<(f32, u32)> = Vec::new(); // (demand, slot)
        let mut downgrades: Vec<(u32, u32)> = Vec::new();

        for (&slot, entry) in &mut self.entries {
            let demand = self.demand[slot as usize];
            let has_demand = !demand.is_nan() && demand > NO_DEMAND;
            let feedback = self.feedback[slot as usize]
                .filter(|r| now.duration_since(r.seen) < FEEDBACK_HOLD)
                .map(|r| r.mip);
            if entry.in_flight || !(has_demand || feedback.is_some()) {
                continue;
            }
            match &entry.state {
                State::Placeholder if has_demand => loads.push((demand, slot)),
                State::Resident(gpu) => {
                    let target =
                        target_first_mip(policy, &gpu.layout, demand, feedback, gpu.first_mip);
                    match target.cmp(&gpu.first_mip) {
                        std::cmp::Ordering::Less => {
                            entry.downgrade_since = None;
                            if entry.may_upgrade(now) {
                                let by_demand = target_first_mip(
                                    policy,
                                    &gpu.layout,
                                    demand,
                                    None,
                                    gpu.first_mip,
                                );
                                let priority = if target < by_demand {
                                    self.stats.feedback_upgrades += 1;
                                    f32::INFINITY
                                } else {
                                    demand
                                };
                                loads.push((priority, slot));
                            }
                        }
                        std::cmp::Ordering::Greater => {
                            if entry.downgrade_hold_elapsed(now) {
                                downgrades.push((slot, target));
                            }
                        }
                        std::cmp::Ordering::Equal => {
                            entry.downgrade_since = None;
                        }
                    }
                }
                State::Placeholder | State::Failed => {}
            }
        }

        let capacity = if self.ready_bytes >= READY_BYTE_CAP {
            0
        } else {
            MAX_IN_FLIGHT.saturating_sub(self.in_flight)
        };
        self.plan_backlog = loads.len() > capacity;
        if loads.len() > capacity {
            if capacity > 0 {
                loads.select_nth_unstable_by(capacity - 1, |a, b| b.0.total_cmp(&a.0));
            }
            loads.truncate(capacity);
        }
        loads.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (_, slot) in loads {
            let Some(request) = self.io_request(slot, now, policy) else {
                continue;
            };
            if self.requests.send(request).is_err() {
                crate::utils::warn_once("texture-io-threads", || {
                    "texture IO threads are gone; textures will stay at their current resolution"
                        .to_owned()
                });
                break;
            }
            self.in_flight += 1;
            if let Some(entry) = self.entries.get_mut(&slot) {
                entry.in_flight = true;
            }
        }

        downgrades.sort_by_key(|&(slot, _)| slot);
        for (slot, target) in downgrades.into_iter().take(MAX_DOWNGRADES_PER_PUMP) {
            self.downgrade(slot, target, device, array, encoder);
            if pump_started.elapsed() >= PUMP_TIME_BUDGET {
                break;
            }
        }
    }

    /// Stuck detector: counts the resident textures that have wanted a finer
    /// mip than they hold for `STUCK_AFTER` and logs the first few with
    /// everything the planner decides on.
    fn report_stuck(&mut self, now: Instant) {
        let policy = self.policy;
        let (in_flight, ready, ready_bytes) = (self.in_flight, self.ready.len(), self.ready_bytes);
        let mut stuck = 0u32;
        let mut undemanded_count = 0u32;
        self.undemanded_found.clear();
        for (&slot, entry) in &mut self.entries {
            let State::Resident(gpu) = &entry.state else {
                entry.behind_since = None;
                entry.undemanded_since = None;
                continue;
            };
            let demand = self.demand[slot as usize];
            let raw_feedback = self.feedback[slot as usize];
            let feedback_mip = raw_feedback
                .filter(|r| now.duration_since(r.seen) < FEEDBACK_HOLD)
                .map(|r| r.mip);
            let undemanded = !(demand > NO_DEMAND) && feedback_mip.is_none() && gpu.first_mip > 0;
            if undemanded {
                let since = *entry.undemanded_since.get_or_insert(now);
                let waiting = now.duration_since(since);
                if waiting >= STUCK_AFTER {
                    undemanded_count += 1;
                    if self.undemanded_found.len() < MAX_STUCK_LOGGED_PER_CHECK as usize {
                        self.undemanded_found.push(UndemandedTexture {
                            slot,
                            path: entry.path.clone(),
                            first_mip: gpu.first_mip,
                            mip_count: gpu.layout.mip_count,
                            seconds: waiting.as_secs_f32(),
                        });
                    }
                }
            } else {
                entry.undemanded_since = None;
            }
            let wanted = target_first_mip(policy, &gpu.layout, demand, feedback_mip, gpu.first_mip);
            if wanted >= gpu.first_mip {
                entry.behind_since = None;
                continue;
            }
            let behind = now.duration_since(*entry.behind_since.get_or_insert(now));
            if behind < STUCK_AFTER {
                continue;
            }
            stuck += 1;
            if stuck > MAX_STUCK_LOGGED_PER_CHECK {
                continue;
            }
            let cpu_wanted = target_first_mip(policy, &gpu.layout, demand, None, gpu.first_mip);
            let feedback_wanted =
                feedback_mip.map(|mip| feedback_first_mip(policy, &gpu.layout, mip));
            let retry_in_ms = entry
                .upgrade_retry_at
                .map(|at| at.saturating_duration_since(now).as_millis());
            tracing::warn!(
                "texture stuck {:.1}s: slot {slot} '{}' resident first_mip {} (of {} mips, skip limit {}), wanted {wanted} \
                 (cpu {cpu_wanted}, feedback {feedback_wanted:?}); demand {demand}, feedback request {:?} age {:?} ms; \
                 in_flight {} may_upgrade {} failures {}/{MAX_UPGRADE_FAILURES} retry_in_ms {retry_in_ms:?} last_failure {:?}; \
                 global in_flight {in_flight} ready {ready} ready_bytes {ready_bytes}",
                behind.as_secs_f32(),
                entry.path,
                gpu.first_mip,
                gpu.layout.mip_count,
                skip_limit(&gpu.layout),
                raw_feedback.map(|r| r.mip),
                raw_feedback.map(|r| now.duration_since(r.seen).as_millis()),
                entry.in_flight,
                entry.may_upgrade(now),
                entry.upgrade_failures,
                entry.last_failure,
            );
        }
        self.stats.stuck = stuck;
        self.stats.undemanded = undemanded_count;
    }

    /// Resident textures whose path contains `needle` (lowercase), at most `limit`.
    pub fn trace_textures(&self, needle: &str, limit: usize) -> Vec<TextureTrace> {
        let now = Instant::now();
        let policy = self.policy;
        let mut traces: Vec<TextureTrace> = self
            .entries
            .iter()
            .filter_map(|(&slot, entry)| {
                let State::Resident(gpu) = &entry.state else {
                    return None;
                };
                if !entry.path.to_ascii_lowercase().contains(needle) {
                    return None;
                }
                let demand = self.demand[slot as usize];
                let feedback_mip = self.feedback_mip(slot, now);
                Some(TextureTrace {
                    slot,
                    path: entry.path.clone(),
                    width: gpu.layout.width,
                    height: gpu.layout.height,
                    mip_count: gpu.layout.mip_count,
                    first_mip: gpu.first_mip,
                    skip_limit: skip_limit(&gpu.layout),
                    wanted: target_first_mip(
                        policy,
                        &gpu.layout,
                        demand,
                        feedback_mip,
                        gpu.first_mip,
                    ),
                    cpu_wanted: target_first_mip(policy, &gpu.layout, demand, None, gpu.first_mip),
                    feedback_wanted: feedback_mip
                        .map(|mip| feedback_first_mip(policy, &gpu.layout, mip)),
                    demand,
                })
            })
            .collect();
        traces.sort_by_key(|t| t.slot);
        traces.truncate(limit);
        traces
    }

    /// Textures the last stuck check found without any demand, for the
    /// streamer to name their owners; empty until the next check finds some.
    pub fn take_undemanded(&mut self) -> Vec<UndemandedTexture> {
        std::mem::take(&mut self.undemanded_found)
    }

    /// The read the planner chose for `slot` (a placeholder load or an upgrade).
    fn io_request(&self, slot: u32, now: Instant, policy: ResidencyPolicy) -> Option<IoRequest> {
        let entry = self.entries.get(&slot)?;
        let demand = self.demand[slot as usize];
        match &entry.state {
            State::Placeholder => Some(IoRequest::Load {
                handle: entry.handle,
                path: entry.path.clone(),
                demand,
            }),
            State::Resident(gpu) => Some(IoRequest::Upgrade {
                handle: entry.handle,
                path: entry.path.clone(),
                layout: Arc::clone(&gpu.layout),
                first_mip: target_first_mip(
                    policy,
                    &gpu.layout,
                    demand,
                    self.feedback_mip(slot, now),
                    gpu.first_mip,
                ),
                resident_first: gpu.first_mip,
            }),
            State::Failed => None,
        }
    }

    /// Shrink a resident texture to `target_first` mips-skipped without IO.
    fn downgrade(
        &mut self,
        slot: u32,
        target_first: u32,
        device: &wgpu::Device,
        array: &mut TextureArray,
        encoder: &mut Option<wgpu::CommandEncoder>,
    ) {
        let Some(entry) = self.entries.get(&slot) else {
            return;
        };
        let State::Resident(old) = &entry.state else {
            return;
        };
        let new_gpu = GpuTexture::new(device, &old.layout, target_first, entry.srgb, &entry.path);
        let encoder = encoder.get_or_insert_with(|| create_encoder(device));
        new_gpu.copy_mips_from(encoder, old, target_first..old.layout.mip_count);
        self.swap_in(slot, new_gpu, array);
        self.stats.downgrades += 1;
    }
}

fn result_bytes(result: &IoResult) -> u64 {
    result
        .outcome
        .as_ref()
        .map_or(0, |payload| payload.data.len() as u64)
}

fn create_encoder(device: &wgpu::Device) -> wgpu::CommandEncoder {
    device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("texture_residency_copies"),
    })
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact sentinels and copied values"
)]
mod tests {
    use super::super::policy::bc1_layout;
    use super::*;

    fn policy() -> ResidencyPolicy {
        ResidencyPolicy {
            global_skip: 0,
            max_anisotropy: 16,
        }
    }

    fn residency() -> TextureResidency {
        TextureResidency::new(policy(), "no-such-asset-root")
    }

    fn placeholder_entry() -> Entry {
        Entry {
            handle: TextureHandle {
                slot: 9,
                generation: 1,
            },
            path: "textures/test.dds".to_owned(),
            srgb: true,
            placeholder_slot: 0,
            state: State::Placeholder,
            in_flight: false,
            downgrade_since: None,
            upgrade_failures: 0,
            upgrade_retry_at: None,
            behind_since: None,
            undemanded_since: None,
            last_failure: None,
        }
    }

    #[test]
    fn skip_limit_matches_the_policy_for_power_of_two_textures() {
        assert_eq!(skip_limit(&bc1_layout(1024, 1024, 11)), 4);
    }

    #[test]
    fn skip_limit_backs_off_until_the_base_level_is_block_aligned() {
        // 1000x1000: policy allows skip 3 (125x125) and skip 2 (250x250) is not a
        // multiple of the 4-texel block either; skip 1 (500x500) is.
        assert_eq!(skip_limit(&bc1_layout(1000, 1000, 10)), 1);
    }

    #[test]
    fn initial_first_mip_is_bounded_by_the_skip_limit() {
        // Demand 1 would pick the policy maximum 3, but only skip 1 is block aligned.
        assert_eq!(
            initial_first_mip(policy(), &bc1_layout(1000, 1000, 10), 1.0),
            1
        );
    }

    #[test]
    fn upgrade_is_allowed_for_a_texture_that_never_failed() {
        assert!(placeholder_entry().may_upgrade(Instant::now()));
    }

    #[test]
    fn failed_upgrades_retry_after_an_exponential_backoff() {
        let start = Instant::now();
        let mut entry = placeholder_entry();
        entry.note_upgrade_failure("io", start);
        assert!(
            !entry.may_upgrade(start + UPGRADE_RETRY_BASE.saturating_sub(Duration::from_millis(1)))
        );
        assert!(entry.may_upgrade(start + UPGRADE_RETRY_BASE));

        entry.note_upgrade_failure("io", start);
        assert!(!entry.may_upgrade(
            start + (UPGRADE_RETRY_BASE * 2).saturating_sub(Duration::from_millis(1))
        ));
        assert!(entry.may_upgrade(start + UPGRADE_RETRY_BASE * 2));
    }

    #[test]
    fn upgrade_is_given_up_for_good_after_the_maximum_failures() {
        let start = Instant::now();
        let mut entry = placeholder_entry();
        for _ in 0..MAX_UPGRADE_FAILURES {
            entry.note_upgrade_failure("io", start);
        }
        assert!(!entry.may_upgrade(start + Duration::from_hours(24)));
    }

    #[test]
    fn a_successful_upgrade_clears_the_failure_history() {
        let start = Instant::now();
        let mut entry = placeholder_entry();
        entry.note_upgrade_failure("io", start);
        entry.note_upgrade_success();
        assert!(entry.may_upgrade(start));
        assert_eq!(entry.upgrade_failures, 0);
    }

    #[test]
    fn a_failure_records_its_reason() {
        let mut entry = placeholder_entry();
        entry.note_upgrade_failure("disk gone", Instant::now());
        assert_eq!(entry.last_failure.as_deref(), Some("disk gone"));
    }

    #[test]
    fn downgrade_waits_for_the_hold_to_elapse() {
        let start = Instant::now();
        let mut entry = placeholder_entry();
        assert!(!entry.downgrade_hold_elapsed(start));
        assert!(!entry.downgrade_hold_elapsed(
            start + DOWNGRADE_HOLD.saturating_sub(Duration::from_millis(1))
        ));
        assert!(entry.downgrade_hold_elapsed(start + DOWNGRADE_HOLD));
    }

    #[test]
    fn downgrade_hold_restarts_after_the_wish_was_dropped() {
        let start = Instant::now();
        let mut entry = placeholder_entry();
        entry.downgrade_hold_elapsed(start);
        entry.downgrade_since = None;
        assert!(!entry.downgrade_hold_elapsed(start + DOWNGRADE_HOLD));
    }

    #[test]
    fn the_most_demanding_user_of_a_slot_wins() {
        let mut residency = residency();
        residency.note_demand(3, 10.0);
        residency.note_demand(3, 40.0);
        residency.note_demand(3, 25.0);
        assert_eq!(residency.demand[3], 40.0);
        assert_eq!(residency.sweep_demand(3), 40.0);
    }

    #[test]
    fn finishing_a_sweep_replaces_demand_so_unused_textures_can_fall_back() {
        let mut residency = residency();
        residency.note_demand(3, 40.0);
        residency.finish_demand_sweep();
        assert_eq!(residency.sweep_demand(3), NO_DEMAND);
        // The sweep that just finished still saw the user.
        assert_eq!(residency.demand[3], 40.0);

        residency.finish_demand_sweep();
        assert_eq!(residency.demand[3], NO_DEMAND);
    }

    #[test]
    fn finer_feedback_replaces_the_held_request() {
        let start = Instant::now();
        let mut residency = residency();
        residency.note_feedback(1, 3, start);
        residency.note_feedback(1, 1, start + Duration::from_millis(10));
        assert_eq!(
            residency.feedback_mip(1, start + Duration::from_millis(10)),
            Some(1)
        );
    }

    #[test]
    fn coarser_feedback_does_not_replace_a_held_finer_request() {
        let start = Instant::now();
        let mut residency = residency();
        residency.note_feedback(1, 1, start);
        residency.note_feedback(1, 4, start + FEEDBACK_HOLD / 2);
        assert_eq!(
            residency.feedback_mip(1, start + FEEDBACK_HOLD / 2),
            Some(1)
        );
    }

    #[test]
    fn coarser_feedback_replaces_a_request_whose_hold_ran_out() {
        let start = Instant::now();
        let mut residency = residency();
        residency.note_feedback(1, 1, start);
        residency.note_feedback(1, 4, start + FEEDBACK_HOLD);
        assert_eq!(residency.feedback_mip(1, start + FEEDBACK_HOLD), Some(4));
    }

    #[test]
    fn feedback_expires_after_the_hold() {
        let start = Instant::now();
        let mut residency = residency();
        residency.note_feedback(1, 2, start);
        assert_eq!(
            residency.feedback_mip(
                1,
                start + FEEDBACK_HOLD.saturating_sub(Duration::from_millis(1))
            ),
            Some(2)
        );
        assert_eq!(residency.feedback_mip(1, start + FEEDBACK_HOLD), None);
    }

    #[test]
    fn a_recycled_slot_is_recognised_by_epoch() {
        let mut residency = residency();
        let before = residency.recycle_epoch();
        residency.reset_slot(5);
        assert!(residency.recycled_since(5, before));
        assert!(!residency.recycled_since(5, residency.recycle_epoch()));
        assert!(!residency.recycled_since(6, before));
    }

    #[test]
    fn recycling_a_slot_drops_its_demand_and_feedback() {
        let start = Instant::now();
        let mut residency = residency();
        residency.note_demand(5, 80.0);
        residency.note_feedback(5, 0, start);
        residency.reset_slot(5);
        assert_eq!(residency.demand[5], NO_DEMAND);
        assert_eq!(residency.sweep_demand(5), NO_DEMAND);
        assert_eq!(residency.feedback_mip(5, start), None);
    }
}
