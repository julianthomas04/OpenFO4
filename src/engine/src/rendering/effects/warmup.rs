//! The worker threads that compile effect pipelines and load material textures, so a frame
//! never waits for either. The effect stage sends a job when it first sees a permutation or a
//! material (when a cell becomes resident, or at the latest when a draw first wants it), skips
//! the draws that need it until the result is back and draws them from the next frame on.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::Duration;

use super::material_bindings::{BuiltGroup, EffectTextureError, MaterialLoader};
use super::pipeline::{EffectPipelineKey, PipelineCompiler};
use super::shader::EffectShaderError;
use crate::assets::types::effect_material::EffectMaterial;

/// Whether a pipeline or material is usable by a draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    /// The worker is still building it: the draw waits.
    Pending,
    /// It cannot be built: the draw is skipped for good.
    Failed,
}

pub enum WarmupJob {
    Pipeline(EffectPipelineKey),
    /// The material and the environment cube its `t5` shows.
    Material(Arc<EffectMaterial>, wgpu::TextureView),
}

pub enum WarmupResult {
    Pipeline(
        EffectPipelineKey,
        Result<wgpu::RenderPipeline, EffectShaderError>,
    ),
    Material(Arc<EffectMaterial>, Result<BuiltGroup, EffectTextureError>),
}

/// A worker thread that turns jobs into outcomes in submission order, so a frame never waits
/// for the work.
pub struct JobWorker<Job, Outcome> {
    name: &'static str,
    jobs: Sender<Job>,
    results: Receiver<Outcome>,
}

pub type WarmupWorker = JobWorker<WarmupJob, WarmupResult>;

impl<Job: Send + 'static, Outcome: Send + 'static> JobWorker<Job, Outcome> {
    /// Starts the thread `name`, which runs `handler` on every job until the worker is dropped.
    pub fn spawn_with(
        name: &'static str,
        mut handler: impl FnMut(Job) -> Outcome + Send + 'static,
    ) -> Self {
        let (jobs, job_queue) = mpsc::channel::<Job>();
        let (result_sender, results) = mpsc::channel();
        std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                crate::utils::lower_thread_priority();
                while let Ok(job) = job_queue.recv() {
                    if result_sender.send(handler(job)).is_err() {
                        return;
                    }
                }
            })
            .unwrap_or_else(|error| panic!("the {name} thread can be spawned: {error}"));
        Self {
            name,
            jobs,
            results,
        }
    }

    pub fn submit(&self, job: Job) {
        self.jobs.send(job).unwrap_or_else(|_| {
            panic!(
                "the {} thread outlives the stage that owns its queue",
                self.name
            )
        });
    }

    /// The next finished job, without waiting.
    pub fn try_next(&self) -> Option<Outcome> {
        match self.results.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => self.ended(),
        }
    }

    /// The next finished job, waiting up to `timeout` for it.
    pub fn next_within(&self, timeout: Duration) -> Option<Outcome> {
        match self.results.recv_timeout(timeout) {
            Ok(result) => Some(result),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => self.ended(),
        }
    }

    fn ended(&self) -> ! {
        panic!("the {} thread ended while its stage is alive", self.name)
    }
}

impl WarmupWorker {
    pub fn spawn(compiler: Arc<PipelineCompiler>, materials: Arc<MaterialLoader>) -> Self {
        Self::spawn_with("effect-warmup", move |job| match job {
            WarmupJob::Pipeline(key) => WarmupResult::Pipeline(key, compiler.build(&key)),
            WarmupJob::Material(material, cube) => {
                let group = materials.build_group(&material, &cube);
                WarmupResult::Material(material, group)
            }
        })
    }
}
