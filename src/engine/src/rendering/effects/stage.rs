//! The effect stage: draws the resident effect meshes and the simulated particle systems in the
//! translucent pass.
//!
//! Per frame: [`EffectStage::submit_particles`] receives the particle draws, then
//! [`EffectStage::prepare`] culls the resident mesh instances, evaluates every material's
//! controller tracks, picks the lights of lit permutations, fills the per-frame and per-draw
//! constants and orders the draws. [`EffectStage::draw`] then only binds and issues.
//!
//! Pipelines and textures are built by a worker thread ([`super::warmup`]): a permutation or
//! material is requested when its cell becomes resident ([`EffectStage::prewarm`]) or, at the
//! latest, when a draw first wants it, and a draw whose permutation or textures are not ready is
//! skipped until they are. A draw that cannot work (a missing texture, a permutation that does
//! not compile, malformed constants) is skipped with a warning naming it and counted, never an
//! error that blanks the frame. Only a translated shader that is missing altogether is fatal:
//! the installed assets and the converted models disagree.
//!
//! Environment cubes: a permutation with the environment bit reads the cube its material names at
//! `t5`. The cube comes from the renderer's shared `CubeArray` (the one the glass materials use,
//! loaded once and kept for the session) and its per-slot cube view is bound directly in the
//! material's bind group; materials without an environment cube bind the array's black cube. The
//! cube is resolved on the render thread when the material is first requested, so the worker only
//! builds the bind group.

use std::collections::HashSet;
use std::sync::Arc;

use glam::{Mat4, Vec3};
use tracing::info;

use super::constants::{
    AnimatedParameters, ConstantsError, Descriptor, EffectDrawConstants, EffectDrawInputs,
    EffectLighting, EffectView, draw_constants, frame_constants,
};
use super::draw_buffer::{DrawBuffer, DrawBufferSpec};
use super::geometry::{
    ArenaRange, EffectGeometryPool, EffectInstance, EffectMeshData, ParticleArena,
};
use super::lifetime::{SweepClock, WeakSet};
use super::lights_pick::{LightPicker, PickableLight};
use super::material_bindings::{EffectTextureError, MaterialBindings, path_below_textures};
use super::particle_draw::{ParticleDraw, ParticleDrawPool};
use super::pipeline::{EffectPipelineKey, EffectPipelines};
use super::shader::{EffectShaderError, VertexLayoutKind, bindings};
use super::warmup::{Readiness, WarmupJob, WarmupResult, WarmupWorker};
use crate::assets::types::effect_material::{BlendMode, EffectMaterial, EffectTextures};
use crate::rendering::cube_array::{CubeArray, CubeHandle};
use crate::rendering::frustum::clip_planes;
use crate::rendering::lighting::frame_uniform::FrameUniform;
use crate::rendering::scene_copy::SceneCopy;
use crate::utils::{error_once, warn_once};

/// Frames between two sweeps that release the resources of retired effects.
const INITIAL_DRAW_CAPACITY: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum EffectStageError {
    #[error(transparent)]
    Shader(#[from] EffectShaderError),
    #[error(transparent)]
    Texture(#[from] EffectTextureError),
    #[error("effect material {vs} / {ps}: {source}")]
    Constants {
        vs: String,
        ps: String,
        #[source]
        source: ConstantsError,
    },
}

impl EffectStageError {
    /// What kind of fault this is, for reporting each kind of fault of a material once.
    fn kind(&self) -> &'static str {
        match self {
            Self::Shader(_) => "shader",
            Self::Texture(_) => "texture",
            Self::Constants { source, .. } => match source {
                ConstantsError::MalformedShaderName(_) => "malformed-shader-name",
                ConstantsError::TooManyLights { .. } => "too-many-lights",
                ConstantsError::TooManySubtextures { .. } => "too-many-subtextures",
                ConstantsError::NotAnEffectTarget { .. } => "not-an-effect-target",
                ConstantsError::TrackValueKind { .. } => "track-value-kind",
                ConstantsError::SingularModel => "singular-model",
            },
        }
    }
}

/// What one frame's [`EffectStage::prepare`] needs besides the resident instances and lights.
pub struct EffectFrameInputs<'a> {
    pub frame: &'a FrameUniform,
    /// Camera near plane, eu.
    pub near_plane: f32,
    /// Engine time: the clock the looping controller tracks run on.
    pub elapsed_seconds: f32,
}

/// What orders a draw among the others.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SortKey {
    /// Any blend factors set: the draw is translucent.
    pub blended: bool,
    /// Distance of the draw's bounds centre along the view direction.
    pub view_depth: f32,
}

impl SortKey {
    /// Unblended draws first, nearest first (the depth test rejects what they hide), then the
    /// blended draws farthest first so nearer ones blend over farther ones.
    fn compare(self, other: Self) -> std::cmp::Ordering {
        self.blended.cmp(&other.blended).then_with(|| {
            if self.blended {
                other.view_depth.total_cmp(&self.view_depth)
            } else {
                self.view_depth.total_cmp(&other.view_depth)
            }
        })
    }
}

/// The draw order: unblended draws first, nearest first, then the blended draws farthest first.
/// Equal keys keep their input order.
pub fn draw_order(keys: &[SortKey]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..keys.len()).collect();
    order.sort_by(|&a, &b| keys[a].compare(keys[b]));
    order
}

/// Mean of the six ambient cube faces: the cell ambient an effect is lit with.
pub fn mean_ambient(frame: &FrameUniform) -> Vec3 {
    let sum: Vec3 = frame.ambient_cube.iter().map(|face| face.truncate()).sum();
    sum / frame.ambient_cube.len() as f32
}

/// Whether a draw's constants need the lights around it: a permutation with the lighting bit
/// reads the light registers, and one with a lighting influence tints by the light at its centre
/// whatever the permutation is.
pub fn needs_lights(material: &EffectMaterial, descriptor: Descriptor) -> bool {
    descriptor.bits() & Descriptor::LIGHTING != 0 || material.lighting_influence > 0.0
}

/// Everything one draw's constants depend on, apart from the frame.
pub struct DrawSetup<'a> {
    pub material: &'a EffectMaterial,
    pub descriptor: Descriptor,
    pub model: Mat4,
    pub elapsed_seconds: f32,
    pub view: &'a EffectView,
    /// Every light that can light the draw; only read when [`needs_lights`].
    pub lights: &'a [PickableLight],
    pub picker: &'a mut LightPicker,
    pub ambient: Vec3,
    /// Engine-space centre of the draw's bounds.
    pub centre: Vec3,
    pub subtexture_rects: &'a [[f32; 4]],
}

/// The per-draw registers of a draw: the material's controller tracks evaluated at
/// `elapsed_seconds` and, when the draw [needs lights](needs_lights), the nearest lights.
pub fn draw_registers(setup: &mut DrawSetup) -> Result<EffectDrawConstants, EffectStageError> {
    let constants_error = |source| EffectStageError::Constants {
        vs: setup.material.vs.clone(),
        ps: setup.material.ps.clone(),
        source,
    };
    let animated =
        AnimatedParameters::at(setup.material, setup.elapsed_seconds).map_err(constants_error)?;
    let picked = if needs_lights(setup.material, setup.descriptor) {
        setup.picker.pick(setup.lights, setup.centre)
    } else {
        super::lights_pick::PickedLights::NONE
    };
    let lighting = EffectLighting {
        lights: picked.as_slice(),
        ambient: setup.ambient,
        centre: setup.centre,
    };
    draw_constants(&EffectDrawInputs {
        material: setup.material,
        descriptor: setup.descriptor,
        animated: &animated,
        model: setup.model,
        view: setup.view,
        lighting: &lighting,
        subtexture_rects: setup.subtexture_rects,
    })
    .map_err(constants_error)
}

enum PreparedGeometry {
    Mesh(Arc<EffectMeshData>),
    Particles(ArenaRange),
}

struct PreparedDraw {
    pipeline: EffectPipelineKey,
    material: Arc<EffectMaterial>,
    geometry: PreparedGeometry,
    soft: bool,
}

/// Where a candidate draw's data lives until it is prepared.
#[derive(Clone, Copy)]
enum Source {
    Mesh(usize),
    Particles(usize),
}

#[derive(Clone, Copy)]
struct Candidate {
    source: Source,
    key: SortKey,
}

/// How many draws were skipped, by reason.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EffectCounters {
    /// Draws that waited (once per frame) for a pipeline or a texture still being built.
    pub deferred_draws: u64,
    /// Draws skipped because they cannot work: a missing texture, a permutation that does not
    /// compile, malformed constants (once per frame and draw).
    pub skipped_draws: u64,
}

const DRAW_BUFFER: DrawBufferSpec = DrawBufferSpec {
    buffer_label: "effect_draw_constants",
    group_label: "effect_draw_bind_group",
    binding: bindings::DRAW_BINDING,
    block_size: bindings::DRAW_UNIFORM_SIZE,
    stride: bindings::DRAW_CONSTANTS_STRIDE,
};

struct FrameConstantsBuffer {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl FrameConstantsBuffer {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("effect_frame_constants"),
            size: bindings::FRAME_UNIFORM_SIZE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("effect_frame_bind_group"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: bindings::FRAME_BINDING,
                resource: buffer.as_entire_binding(),
            }],
        });
        Self { buffer, bind_group }
    }
}

/// Which pipelines and materials the worker has been asked for and has not delivered, and which
/// it failed to deliver.
#[derive(Default)]
struct WarmupState {
    pending_pipelines: HashSet<EffectPipelineKey>,
    failed_pipelines: HashSet<EffectPipelineKey>,
    pending_materials: WeakSet<EffectMaterial>,
    failed_materials: WeakSet<EffectMaterial>,
    /// Materials already offered to [`EffectStage::prewarm`], by pointer and vertex layout.
    prewarmed: WeakSet<EffectMaterial, VertexLayoutKind>,
    /// Faults already reported, by material pointer and [`EffectStageError::kind`].
    reported_faults: WeakSet<EffectMaterial, &'static str>,
}

/// The cube `t5` shows for `material`. Only a permutation with the environment bit reads it:
/// that is the real cube the material names, resolved through the renderer's shared
/// [`CubeArray`] (loaded once, kept for the session); every other material gets the array's black
/// cube, so it reflects nothing. A named cube that does not load is the error, carrying its path.
fn environment_cube(
    material: &EffectMaterial,
    cubes: &mut CubeArray,
) -> Result<wgpu::TextureView, String> {
    let wanted = Descriptor::of_shader_name(&material.ps)
        .is_ok_and(|descriptor| descriptor.bits() & Descriptor::ENVIRONMENT != 0)
        .then(|| EffectTextures::used(&material.textures.env))
        .flatten();
    let Some(stored) = wanted else {
        return Ok(cubes.view(CubeHandle::NONE).clone());
    };
    let path = path_below_textures(stored);
    match cubes.acquire(&path) {
        CubeHandle::NONE => Err(path),
        handle => Ok(cubes.view(handle).clone()),
    }
}

pub struct EffectStage {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipelines: EffectPipelines,
    geometry: EffectGeometryPool,
    arena: ParticleArena,
    materials: MaterialBindings,
    worker: WarmupWorker,
    warmup: WarmupState,
    frame: FrameConstantsBuffer,
    draw_constants: DrawBuffer,
    particles: ParticleDrawPool,
    draws: Vec<PreparedDraw>,
    instances: Vec<EffectInstance>,
    candidates: Vec<Candidate>,
    registers: Vec<EffectDrawConstants>,
    lights: Vec<PickableLight>,
    picker: LightPicker,
    counters: EffectCounters,
    sweep: SweepClock,
    reported: bool,
}

impl EffectStage {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene_layout: &wgpu::BindGroupLayout,
        asset_root: &str,
    ) -> Self {
        Self::with_pipelines(
            device,
            queue,
            EffectPipelines::new(device, scene_layout),
            asset_root,
        )
    }

    pub fn with_pipelines(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipelines: EffectPipelines,
        asset_root: &str,
    ) -> Self {
        let layouts = pipelines.layouts();
        let frame = FrameConstantsBuffer::new(device, &layouts.frame);
        let draw_constants =
            DrawBuffer::new(device, &layouts.draw, DRAW_BUFFER, INITIAL_DRAW_CAPACITY);
        let materials = MaterialBindings::new(device, queue, &layouts.material, asset_root);
        let worker = WarmupWorker::spawn(pipelines.compiler(), materials.loader());
        Self {
            device: device.clone(),
            queue: queue.clone(),
            geometry: EffectGeometryPool::default(),
            arena: ParticleArena::new(device),
            materials,
            worker,
            warmup: WarmupState::default(),
            pipelines,
            frame,
            draw_constants,
            particles: ParticleDrawPool::default(),
            draws: Vec::new(),
            instances: Vec::new(),
            candidates: Vec::new(),
            registers: Vec::new(),
            lights: Vec::new(),
            picker: LightPicker::default(),
            counters: EffectCounters::default(),
            sweep: SweepClock::default(),
            reported: false,
        }
    }

    /// The pool the next [`prepare`](Self::prepare) orders with the meshes. The caller clears it
    /// before filling, so a frame that never prepared leaves nothing behind.
    pub fn particle_pool(&mut self) -> &mut ParticleDrawPool {
        &mut self.particles
    }

    /// Whether any prepared draw reads the scene depth copy.
    pub fn needs_scene_copy(&self) -> bool {
        self.draws.iter().any(|draw| draw.soft)
    }

    pub fn has_work(&self) -> bool {
        !self.draws.is_empty()
    }

    /// Asks the worker to build what draws of `material` on `vertex` vertices need (pipeline and
    /// textures), without waiting for it. Call it when the material becomes resident so the
    /// first visible frame already has them; until they are ready the draws are skipped.
    /// Repeated calls are cheap.
    pub fn prewarm(
        &mut self,
        material: &Arc<EffectMaterial>,
        vertex: VertexLayoutKind,
        cubes: &mut CubeArray,
    ) {
        if !self.warmup.prewarmed.insert(material, vertex) {
            return;
        }
        match EffectPipelineKey::from_material(material, vertex) {
            Ok(key) => {
                self.pipeline_readiness(&key);
            }
            Err(error) => self.report_fault(material, &error.into()),
        }
        self.material_readiness(material, cubes);
    }

    fn pipeline_readiness(&mut self, key: &EffectPipelineKey) -> Readiness {
        if self.pipelines.existing(key).is_some() {
            Readiness::Ready
        } else if self.warmup.failed_pipelines.contains(key) {
            Readiness::Failed
        } else {
            if self.warmup.pending_pipelines.insert(*key) {
                self.worker.submit(WarmupJob::Pipeline(*key));
            }
            Readiness::Pending
        }
    }

    fn material_readiness(
        &mut self,
        material: &Arc<EffectMaterial>,
        cubes: &mut CubeArray,
    ) -> Readiness {
        if self.materials.group(material).is_some() {
            Readiness::Ready
        } else if self.warmup.failed_materials.contains(material, ()) {
            Readiness::Failed
        } else if self.warmup.pending_materials.contains(material, ()) {
            Readiness::Pending
        } else {
            match environment_cube(material, cubes) {
                Ok(cube) => {
                    self.warmup.pending_materials.insert(material, ());
                    self.worker
                        .submit(WarmupJob::Material(Arc::clone(material), cube));
                    Readiness::Pending
                }
                Err(path) => {
                    self.warmup.failed_materials.insert(material, ());
                    warn_once(&format!("effect-cube:{path}"), || {
                        format!(
                            "skipping effect draws of material {} / {}: environment cube {path} \
                             not loaded",
                            material.vs, material.ps
                        )
                    });
                    Readiness::Failed
                }
            }
        }
    }

    /// Takes in what the worker has finished. A translated shader missing altogether is fatal.
    fn collect_warmup(&mut self) -> Result<(), EffectStageError> {
        let mut fatal = None;
        while let Some(result) = self.worker.try_next() {
            if let Err(error) = self.accept(result) {
                fatal.get_or_insert(error);
            }
        }
        fatal.map_or(Ok(()), Err)
    }

    fn accept(&mut self, result: WarmupResult) -> Result<(), EffectStageError> {
        match result {
            WarmupResult::Pipeline(key, built) => {
                self.warmup.pending_pipelines.remove(&key);
                match built {
                    Ok(pipeline) => self.pipelines.insert(key, pipeline),
                    Err(error) => {
                        self.warmup.failed_pipelines.insert(key);
                        if matches!(error, EffectShaderError::Load(_)) {
                            return Err(error.into());
                        }
                        error_once(&format!("effect-pipeline:{key:?}"), || {
                            format!("skipping effect draws that need {key:?}: {error}")
                        });
                    }
                }
            }
            WarmupResult::Material(material, built) => {
                self.warmup.pending_materials.remove(&material, ());
                match built {
                    Ok(group) => self.materials.insert(&material, group),
                    Err(error) => {
                        self.warmup.failed_materials.insert(&material, ());
                        warn_once(&format!("effect-texture:{}", error.path), || {
                            format!(
                                "skipping effect draws of material {} / {}: {error}",
                                material.vs, material.ps
                            )
                        });
                    }
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn pending_warmups(&self) -> usize {
        self.warmup.pending_pipelines.len() + self.warmup.pending_materials.len()
    }

    /// Blocks until the worker delivered everything requested so far. A fatal result is
    /// returned.
    #[cfg(test)]
    pub fn wait_for_warmup(&mut self) -> Result<(), EffectStageError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let mut fatal = None;
        while self.pending_warmups() > 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "the warm-up worker did not finish"
            );
            if let Some(result) = self
                .worker
                .next_within(std::time::Duration::from_millis(50))
                && let Err(error) = self.accept(result)
            {
                fatal.get_or_insert(error);
            }
        }
        fatal.map_or(Ok(()), Err)
    }

    /// Warns once per material and kind of fault that its draws are skipped.
    fn report_fault(&mut self, material: &Arc<EffectMaterial>, error: &EffectStageError) {
        if self.warmup.reported_faults.insert(material, error.kind()) {
            warn_once(
                &format!(
                    "effect-fault:{}:{}:{}",
                    material.vs,
                    material.ps,
                    error.kind()
                ),
                || format!("skipping effect draws: {error}"),
            );
        }
    }

    /// Prepares this frame's draws. `for_each_instance` visits the resident effect placements;
    /// `gather_lights` fills the list of every light that can light an effect, and is only
    /// called when a draw of this frame needs lights.
    ///
    /// An error is fatal (a translated shader is missing); draws that cannot be drawn are
    /// skipped and counted, see [`EffectCounters`].
    pub fn prepare(
        &mut self,
        inputs: &EffectFrameInputs,
        cubes: &mut CubeArray,
        for_each_instance: impl FnOnce(&mut dyn FnMut(&EffectInstance)),
        gather_lights: impl FnOnce(&mut Vec<PickableLight>),
    ) -> Result<(), EffectStageError> {
        self.collect_warmup()?;
        self.draws.clear();
        let particles = std::mem::take(&mut self.particles);
        let planes = clip_planes(&inputs.frame.view_proj);
        let camera = inputs.frame.camera_pos.truncate();
        let forward = inputs.frame.camera_forward.truncate();
        let view_depth = |centre: Vec3| (centre - camera).dot(forward);

        let mut instances = std::mem::take(&mut self.instances);
        instances.clear();
        for_each_instance(&mut |instance| {
            self.prewarm(&instance.material, VertexLayoutKind::Mesh, cubes);
            if !instance.bounds.is_outside(&planes) {
                instances.push(instance.clone());
            }
        });
        let blended = |material: &EffectMaterial| material.blend.mode != BlendMode::None;
        let mut candidates = std::mem::take(&mut self.candidates);
        candidates.clear();
        candidates.extend(
            instances
                .iter()
                .enumerate()
                .map(|(index, instance)| Candidate {
                    source: Source::Mesh(index),
                    key: SortKey {
                        blended: blended(&instance.material),
                        view_depth: view_depth(instance.bounds.center()),
                    },
                }),
        );
        candidates.extend(
            particles
                .draws()
                .iter()
                .enumerate()
                .map(|(index, draw)| Candidate {
                    source: Source::Particles(index),
                    key: SortKey {
                        blended: blended(&draw.material),
                        view_depth: view_depth(draw.bounds_centre),
                    },
                }),
        );
        candidates.sort_by(|a, b| a.key.compare(b.key));

        if !candidates.is_empty() {
            self.prepare_draws(
                inputs,
                cubes,
                &instances,
                particles.draws(),
                &candidates,
                gather_lights,
            );
        }
        self.instances = instances;
        self.candidates = candidates;
        self.particles = particles;
        self.particles.clear();
        self.sweep_if_due();
        Ok(())
    }

    fn prepare_draws(
        &mut self,
        inputs: &EffectFrameInputs,
        cubes: &mut CubeArray,
        instances: &[EffectInstance],
        particles: &[ParticleDraw],
        ordered: &[Candidate],
        gather_lights: impl FnOnce(&mut Vec<PickableLight>),
    ) {
        let arena_ranges = self.arena.upload(&self.device, &self.queue, particles);

        let view = EffectView::of_frame(inputs.frame, inputs.near_plane);
        let ambient = mean_ambient(inputs.frame);
        let mut gather_lights = Some(gather_lights);
        let mut registers = std::mem::take(&mut self.registers);
        registers.clear();
        for candidate in ordered {
            let (material, model, elapsed, centre, rects, vertex, geometry) = match candidate.source
            {
                Source::Mesh(index) => {
                    let instance = &instances[index];
                    (
                        &instance.material,
                        instance.model,
                        inputs.elapsed_seconds,
                        instance.bounds.center(),
                        &[][..],
                        VertexLayoutKind::Mesh,
                        PreparedGeometry::Mesh(Arc::clone(&instance.mesh)),
                    )
                }
                Source::Particles(index) => {
                    let draw = &particles[index];
                    (
                        &draw.material,
                        draw.model,
                        draw.elapsed_seconds,
                        draw.bounds_centre,
                        &draw.subtexture_rects[..],
                        VertexLayoutKind::Particle,
                        PreparedGeometry::Particles(arena_ranges[index]),
                    )
                }
            };
            let key = match EffectPipelineKey::from_material(material, vertex) {
                Ok(key) => key,
                Err(error) => {
                    self.skip(material, &error.into());
                    continue;
                }
            };
            let readiness = [
                self.pipeline_readiness(&key),
                self.material_readiness(material, cubes),
            ];
            if readiness.contains(&Readiness::Failed) {
                self.counters.skipped_draws += 1;
                continue;
            }
            if readiness.contains(&Readiness::Pending) {
                self.counters.deferred_draws += 1;
                continue;
            }
            let descriptor = match Descriptor::of_shader_name(&material.ps) {
                Ok(descriptor) => descriptor,
                Err(source) => {
                    let error = EffectStageError::Constants {
                        vs: material.vs.clone(),
                        ps: material.ps.clone(),
                        source,
                    };
                    self.skip(material, &error);
                    continue;
                }
            };
            if needs_lights(material, descriptor)
                && let Some(gather) = gather_lights.take()
            {
                self.lights.clear();
                gather(&mut self.lights);
            }
            let constants = draw_registers(&mut DrawSetup {
                material,
                descriptor,
                model,
                elapsed_seconds: elapsed,
                view: &view,
                lights: &self.lights,
                picker: &mut self.picker,
                ambient,
                centre,
                subtexture_rects: rects,
            });
            let constants = match constants {
                Ok(constants) => constants,
                Err(error) => {
                    self.skip(material, &error);
                    continue;
                }
            };
            if let PreparedGeometry::Mesh(data) = &geometry {
                self.geometry.acquire(&self.device, data);
            }
            registers.push(constants);
            self.draws.push(PreparedDraw {
                pipeline: key,
                material: Arc::clone(material),
                geometry,
                soft: descriptor.bits() & Descriptor::SOFT != 0,
            });
        }

        self.upload_constants(inputs, &registers);
        self.registers = registers;
        self.report_once();
    }

    fn skip(&mut self, material: &Arc<EffectMaterial>, error: &EffectStageError) {
        self.counters.skipped_draws += 1;
        self.report_fault(material, error);
    }

    /// How many draws were skipped so far, by reason.
    pub fn counters(&self) -> EffectCounters {
        self.counters
    }

    fn upload_constants(&mut self, inputs: &EffectFrameInputs, registers: &[EffectDrawConstants]) {
        self.queue.write_buffer(
            &self.frame.buffer,
            0,
            bytemuck::bytes_of(&frame_constants(inputs.frame, inputs.near_plane)),
        );
        self.draw_constants.reserve(
            &self.device,
            &self.pipelines.layouts().draw,
            registers.len(),
        );
        self.queue.write_buffer(
            &self.draw_constants.buffer,
            0,
            bytemuck::cast_slice(registers),
        );
    }

    fn report_once(&mut self) {
        if !self.reported && !self.draws.is_empty() {
            self.reported = true;
            info!(
                "effects: {} draws, {} permutations compiled",
                self.draws.len(),
                self.pipelines.pipeline_count()
            );
        }
    }

    fn sweep_if_due(&mut self) {
        if self.sweep.tick() {
            self.geometry.release_unused();
            self.materials.release_unused();
            let state = &mut self.warmup;
            state.pending_materials.prune();
            state.failed_materials.prune();
            state.prewarmed.prune();
            state.reported_faults.prune();
        }
    }

    /// Issues the prepared draws into the translucent pass. Returns whether it bound anything
    /// the caller's own bindings must be restored after.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, scene: &SceneCopy) -> bool {
        if self.draws.is_empty() {
            return false;
        }
        pass.set_bind_group(bindings::SCENE_GROUP, scene.bind_group(), &[]);
        pass.set_bind_group(bindings::FRAME_GROUP, &self.frame.bind_group, &[]);
        let mut bound_pipeline: Option<&EffectPipelineKey> = None;
        for (index, draw) in self.draws.iter().enumerate() {
            let pipeline = self
                .pipelines
                .existing(&draw.pipeline)
                .expect("prepare creates the pipeline of every draw");
            if bound_pipeline != Some(&draw.pipeline) {
                pass.set_pipeline(pipeline);
                bound_pipeline = Some(&draw.pipeline);
            }
            let material = self
                .materials
                .group(&draw.material)
                .expect("prepare creates the bind group of every draw's material");
            pass.set_bind_group(bindings::MATERIAL_GROUP, material, &[]);
            pass.set_bind_group(
                bindings::DRAW_GROUP,
                &self.draw_constants.bind_group,
                &[index as u32 * bindings::DRAW_CONSTANTS_STRIDE as u32],
            );
            match &draw.geometry {
                PreparedGeometry::Mesh(data) => {
                    let mesh = self
                        .geometry
                        .get(data)
                        .expect("prepare uploads the mesh of every draw");
                    pass.set_vertex_buffer(0, mesh.vertices.slice(..));
                    pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                }
                PreparedGeometry::Particles(range) => {
                    if range.index_count == 0 {
                        continue;
                    }
                    pass.set_vertex_buffer(0, self.arena.vertex_buffer().slice(..));
                    pass.set_index_buffer(
                        self.arena.index_buffer().slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    pass.draw_indexed(
                        range.first_index..range.first_index + range.index_count,
                        range.base_vertex,
                        0..1,
                    );
                }
            }
        }
        true
    }

    #[cfg(test)]
    fn pending_particle_count(&self) -> usize {
        self.particles.draws().len()
    }

    /// Draws prepared for this frame.
    pub fn draw_count(&self) -> usize {
        self.draws.len()
    }

    /// Pipelines compiled so far.
    pub fn permutation_count(&self) -> usize {
        self.pipelines.pipeline_count()
    }
}

#[cfg(test)]
mod tests {
    use bytemuck::Zeroable;
    use serde_json::json;

    use super::*;
    use crate::rendering::effects::lights_pick::LightPicker;
    use crate::rendering::effects::shader::stubs;
    use crate::rendering::effects::test_fixtures::{stub_material, stub_material_with_blend};
    use crate::rendering::placed_light::GpuPlacedLight;

    fn key(blended: bool, view_depth: f32) -> SortKey {
        SortKey {
            blended,
            view_depth,
        }
    }

    #[test]
    fn blended_draws_are_sorted_farthest_first() {
        let order = draw_order(&[key(true, 5.0), key(true, 20.0), key(true, 10.0)]);
        assert_eq!(order, [1, 2, 0]);
    }

    #[test]
    fn unblended_draws_come_first_nearest_first() {
        let order = draw_order(&[key(true, 50.0), key(false, 30.0), key(false, 10.0)]);
        assert_eq!(order, [2, 1, 0]);
    }

    #[test]
    fn equal_keys_keep_their_submission_order() {
        let order = draw_order(&[key(true, 7.0), key(true, 7.0), key(true, 7.0)]);
        assert_eq!(order, [0, 1, 2]);
    }

    #[test]
    fn no_draws_have_no_order() {
        assert_eq!(draw_order(&[]), [] as [usize; 0]);
    }

    fn view() -> EffectView {
        EffectView {
            view_projection: Mat4::IDENTITY,
            camera: Vec3::ZERO,
            near_plane: 0.5,
        }
    }

    const AMBIENT: f32 = 0.25;

    fn registers_of(
        material: &EffectMaterial,
        view: &EffectView,
        lights: &[PickableLight],
        elapsed_seconds: f32,
    ) -> Result<EffectDrawConstants, EffectStageError> {
        let descriptor = Descriptor::of_shader_name(&material.ps).map_err(|source| {
            EffectStageError::Constants {
                vs: material.vs.clone(),
                ps: material.ps.clone(),
                source,
            }
        })?;
        draw_registers(&mut DrawSetup {
            material,
            descriptor,
            model: Mat4::IDENTITY,
            elapsed_seconds,
            view,
            lights,
            picker: &mut LightPicker::default(),
            ambient: Vec3::splat(AMBIENT),
            centre: Vec3::ZERO,
            subtexture_rects: &[],
        })
    }

    fn material_with_emissive_track() -> EffectMaterial {
        let mut value = serde_json::to_value(stub_material()).unwrap();
        value["controllers"] = json!([{
            "target": "emissive_mult", "cycle": "loop", "start": 0.0, "stop": 2.0,
            "frequency": 1.0, "phase": 0.0, "times": [0.0, 2.0], "values": [1.0, 3.0]
        }]);
        serde_json::from_value(value).unwrap()
    }

    fn material_with_lighting_influence(influence: f32) -> EffectMaterial {
        let mut value = serde_json::to_value(stub_material()).unwrap();
        value["lighting_influence"] = json!(influence);
        serde_json::from_value(value).unwrap()
    }

    fn bright_light_at_the_origin() -> [PickableLight; 1] {
        [PickableLight::steady(GpuPlacedLight {
            radius: 100.0,
            color: [1.0, 1.0, 1.0],
            ..GpuPlacedLight::zeroed()
        })]
    }

    #[test]
    fn controller_tracks_are_evaluated_at_the_draws_time() {
        let material = material_with_emissive_track();
        let view = view();
        let scale_at =
            |seconds: f32| registers_of(&material, &view, &[], seconds).unwrap().ps_cb1[1].x;

        assert!(
            (scale_at(0.0) - 1.0).abs() < 1e-5,
            "the track starts at its first key"
        );
        assert!(
            (scale_at(1.0) - 2.0).abs() < 1e-5,
            "and interpolates towards the last"
        );
        assert!(
            (scale_at(0.5) - scale_at(2.5)).abs() < 1e-5,
            "a loop repeats after its range"
        );
    }

    #[test]
    fn the_lighting_permutation_fills_the_light_slots() {
        let mut lit = stub_material();
        lit.ps = "effect_ps_00000405".to_owned();
        let view = view();
        let lights = bright_light_at_the_origin();

        let with_light = registers_of(&lit, &view, &lights, 0.0).unwrap();
        let without_light = registers_of(&lit, &view, &[], 0.0).unwrap();

        assert_ne!(
            with_light.ps_cb2[9], without_light.ps_cb2[9],
            "light colour row"
        );
        assert!(with_light.ps_cb2[13].truncate().x > AMBIENT);
    }

    #[test]
    fn a_lighting_influence_tints_by_the_nearby_lights_on_any_permutation() {
        let tinted = material_with_lighting_influence(0.5);
        let view = view();
        let lights = bright_light_at_the_origin();

        let with_light = registers_of(&tinted, &view, &lights, 0.0).unwrap();
        let without_light = registers_of(&tinted, &view, &[], 0.0).unwrap();

        assert!(
            with_light.ps_cb2[13].x > without_light.ps_cb2[13].x,
            "the tint register carries the light at the centre"
        );
        assert!(
            with_light.ps_cb2[..12]
                .iter()
                .all(|row| *row == glam::Vec4::ZERO),
            "the light slots belong to the lighting permutations"
        );
    }

    #[test]
    fn a_permutation_without_lighting_and_influence_ignores_the_lights() {
        let plain = stub_material();
        let view = view();
        let lights = bright_light_at_the_origin();

        let with_light = registers_of(&plain, &view, &lights, 0.0).unwrap();
        let without_light = registers_of(&plain, &view, &[], 0.0).unwrap();

        assert_eq!(with_light.ps_cb2, without_light.ps_cb2);
        assert!((with_light.ps_cb2[13].x - AMBIENT).abs() < 1e-6);
    }

    #[test]
    fn a_malformed_shader_name_is_an_error_naming_the_material() {
        let mut material = stub_material();
        material.ps = "water_ps_basic".to_owned();
        let view = view();

        let error = registers_of(&material, &view, &[], 0.0).unwrap_err();

        assert!(matches!(error, EffectStageError::Constants { .. }));
        assert!(error.to_string().contains("water_ps_basic"), "{error}");
    }

    #[test]
    fn the_ambient_is_the_mean_of_the_cube_faces() {
        let mut frame = FrameUniform::zeroed();
        for (index, face) in frame.ambient_cube.iter_mut().enumerate() {
            *face = glam::Vec4::new(index as f32, 0.0, 6.0, 1.0);
        }
        assert!((mean_ambient(&frame) - Vec3::new(2.5, 0.0, 6.0)).length() < 1e-6);
    }

    mod gpu {
        use super::*;
        use crate::rendering::cube_array::CubeArray;
        use crate::rendering::effects::shader::{EffectVertex, ParticleVertex};
        use crate::rendering::gpu_test::test_device;
        use crate::rendering::lighting::frame_uniform::FrameUniform;
        use crate::rendering::post::hdr_target::HDR_FORMAT;
        use crate::rendering::texture::Texture;
        use crate::rendering::{effects::geometry::EffectMeshData, frustum::Aabb};

        const SIZE: u32 = 8;

        fn leaked_stubs() -> &'static crate::rendering::shaders::translated::TranslatedShaders {
            Box::leak(Box::new(stubs::translated()))
        }

        /// A full-screen quad in clip space: the stub vertex shader maps positions through
        /// `vs_cb2[0..4]`, which for an identity model is the view-projection, so with an
        /// identity view-projection the model-space position is the clip position.
        fn quad() -> Arc<EffectMeshData> {
            let corner = |x: f32, y: f32| EffectVertex {
                position: [x, y, 0.5],
                uv: [0.5, 0.5],
                normal: [0.0, 0.0, 1.0],
                tangent: [1.0, 0.0, 0.0, 1.0],
                colour: [255; 4],
            };
            Arc::new(EffectMeshData::from_vertices(
                vec![
                    corner(-1.0, -1.0),
                    corner(1.0, -1.0),
                    corner(1.0, 1.0),
                    corner(-1.0, 1.0),
                ],
                vec![0, 1, 2, 0, 2, 3],
            ))
        }

        fn frame() -> FrameUniform {
            let mut frame = FrameUniform::zeroed();
            frame.view_proj = Mat4::IDENTITY;
            frame.camera_forward = glam::Vec4::new(0.0, 0.0, 1.0, 0.0);
            frame
        }

        fn render_target(device: &wgpu::Device) -> (wgpu::Texture, wgpu::Texture) {
            let make = |format, label| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: SIZE,
                        height: SIZE,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                })
            };
            (
                make(HDR_FORMAT, "effect_test_colour"),
                make(Texture::DEPTH_FORMAT, "effect_test_depth"),
            )
        }

        /// IEEE half to single, for the non-negative finite values the tests read back.
        fn f16_to_f32(bits: u16) -> f32 {
            let exponent = i32::from((bits >> 10) & 0x1f);
            let mantissa = f32::from(bits & 0x3ff);
            let magnitude = if exponent == 0 {
                mantissa * 2f32.powi(-24)
            } else {
                (1.0 + mantissa / 1024.0) * 2f32.powi(exponent - 15)
            };
            if bits & 0x8000 != 0 {
                -magnitude
            } else {
                magnitude
            }
        }

        fn read_centre_pixel(
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            colour: &wgpu::Texture,
        ) -> [f32; 4] {
            let bytes_per_row = 256;
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("effect_test_readback"),
                size: u64::from(bytes_per_row) * u64::from(SIZE),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            encoder.copy_texture_to_buffer(
                colour.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(bytes_per_row),
                        rows_per_image: Some(SIZE),
                    },
                },
                wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit([encoder.finish()]);
            let slice = buffer.slice(..);
            slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .expect("the device polls");
            let data = slice
                .get_mapped_range()
                .expect("the readback buffer is mapped");
            let offset = (SIZE / 2 * bytes_per_row + SIZE / 2 * 8) as usize;
            let half = |i: usize| {
                f16_to_f32(u16::from_le_bytes([
                    data[offset + 2 * i],
                    data[offset + 2 * i + 1],
                ]))
            };
            [half(0), half(1), half(2), half(3)]
        }

        fn draw_stub(
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            stage: &mut EffectStage,
        ) -> [f32; 4] {
            let scene = SceneCopy::new(device, SIZE, SIZE);
            let (colour, depth) = render_target(device);
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("effect_test_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &colour.create_view(&wgpu::TextureViewDescriptor::default()),
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth.create_view(&wgpu::TextureViewDescriptor::default()),
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(0.0),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                assert!(stage.draw(&mut pass, &scene));
            }
            queue.submit([encoder.finish()]);
            read_centre_pixel(device, queue, &colour)
        }

        fn stage_over(
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            scene: &SceneCopy,
            shaders: &'static crate::rendering::shaders::translated::TranslatedShaders,
            asset_root: &str,
        ) -> EffectStage {
            let pipelines = EffectPipelines::with_shaders(device, scene.layout(), shaders);
            EffectStage::with_pipelines(device, queue, pipelines, asset_root)
        }

        fn quad_instance(material: EffectMaterial) -> EffectInstance {
            EffectInstance {
                mesh: quad(),
                material: Arc::new(material),
                model: Mat4::IDENTITY,
                bounds: Aabb {
                    min: Vec3::new(-1.0, -1.0, 0.5),
                    max: Vec3::new(1.0, 1.0, 0.5),
                },
            }
        }

        fn cubes_over(stage: &EffectStage, asset_root: &std::path::Path) -> CubeArray {
            CubeArray::new(&stage.device, &stage.queue, asset_root)
        }

        fn prepare_with_cubes(
            stage: &mut EffectStage,
            cubes: &mut CubeArray,
            frame: &FrameUniform,
            instances: &[EffectInstance],
        ) -> Result<(), EffectStageError> {
            stage.prepare(
                &EffectFrameInputs {
                    frame,
                    near_plane: 0.5,
                    elapsed_seconds: 0.0,
                },
                cubes,
                |visit| instances.iter().for_each(visit),
                |_| {},
            )
        }

        fn prepare_with(
            stage: &mut EffectStage,
            frame: &FrameUniform,
            instances: &[EffectInstance],
        ) -> Result<(), EffectStageError> {
            let mut cubes = cubes_over(stage, std::path::Path::new("unused"));
            prepare_with_cubes(stage, &mut cubes, frame, instances)
        }

        /// Prepares once, lets the worker finish what that asked for and prepares again, the
        /// way consecutive frames do.
        fn prepare_twice(
            stage: &mut EffectStage,
            frame: &FrameUniform,
            instances: &[EffectInstance],
        ) {
            prepare_with(stage, frame, instances).unwrap();
            stage.wait_for_warmup().unwrap();
            prepare_with(stage, frame, instances).unwrap();
        }

        #[test]
        fn a_stub_effect_quad_is_drawn_into_the_target() {
            let Some((device, queue)) = test_device("effect stage quad") else {
                return;
            };
            let scene = SceneCopy::new(&device, SIZE, SIZE);
            let mut stage = stage_over(&device, &queue, &scene, leaked_stubs(), "unused");
            let instances = [quad_instance(stub_material_with_blend(
                &json!({ "mode": "none" }),
            ))];
            let frame = frame();

            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            prepare_twice(&mut stage, &frame, &instances);
            let pixel = draw_stub(&device, &queue, &mut stage);

            assert_eq!(stage.draw_count(), 1);
            assert_eq!(stage.permutation_count(), 1);
            assert!(
                pixel[..3].iter().any(|channel| *channel != 0.0),
                "the quad covers the centre pixel: {pixel:?}"
            );
            assert!(pollster::block_on(scope.pop()).is_none());
        }

        #[test]
        fn a_draw_waits_for_its_pipeline_and_is_drawn_once_it_is_ready() {
            let Some((device, queue)) = test_device("effect stage deferred draw") else {
                return;
            };
            let scene = SceneCopy::new(&device, SIZE, SIZE);
            let mut stage = stage_over(&device, &queue, &scene, leaked_stubs(), "unused");
            let instances = [quad_instance(stub_material())];
            let frame = frame();

            prepare_with(&mut stage, &frame, &instances).unwrap();
            let waiting = (stage.draw_count(), stage.counters());
            stage.wait_for_warmup().unwrap();
            prepare_with(&mut stage, &frame, &instances).unwrap();

            assert_eq!(waiting.0, 0, "never blocks the frame on the compile");
            assert_eq!(waiting.1.deferred_draws, 1);
            assert_eq!(stage.draw_count(), 1);
            assert_eq!(stage.counters().skipped_draws, 0);
        }

        #[test]
        fn a_prewarmed_material_draws_on_its_first_frame() {
            let Some((device, queue)) = test_device("effect stage prewarm") else {
                return;
            };
            let scene = SceneCopy::new(&device, SIZE, SIZE);
            let mut stage = stage_over(&device, &queue, &scene, leaked_stubs(), "unused");
            let instances = [quad_instance(stub_material())];
            let frame = frame();

            let mut cubes = cubes_over(&stage, std::path::Path::new("unused"));
            stage.prewarm(&instances[0].material, VertexLayoutKind::Mesh, &mut cubes);
            stage.prewarm(&instances[0].material, VertexLayoutKind::Mesh, &mut cubes);
            stage.wait_for_warmup().unwrap();
            prepare_with(&mut stage, &frame, &instances).unwrap();

            assert_eq!(stage.draw_count(), 1);
            assert_eq!(stage.counters(), EffectCounters::default());
            assert_eq!(stage.permutation_count(), 1, "requested once");
        }

        #[test]
        fn a_material_that_became_resident_is_warmed_without_being_visible() {
            let Some((device, queue)) = test_device("effect stage resident warmup") else {
                return;
            };
            let scene = SceneCopy::new(&device, SIZE, SIZE);
            let mut stage = stage_over(&device, &queue, &scene, leaked_stubs(), "unused");
            let mut offscreen = quad_instance(stub_material());
            offscreen.bounds = Aabb {
                min: Vec3::new(100.0, 100.0, 0.5),
                max: Vec3::new(101.0, 101.0, 0.5),
            };
            let frame = frame();

            prepare_with(&mut stage, &frame, std::slice::from_ref(&offscreen)).unwrap();
            stage.wait_for_warmup().unwrap();

            assert_eq!(stage.draw_count(), 0, "culled");
            assert_eq!(stage.permutation_count(), 1, "but its pipeline was built");
        }

        #[test]
        fn a_draw_with_a_missing_texture_is_skipped_and_counted_without_blanking_the_frame() {
            let Some((device, queue)) = test_device("effect stage missing texture") else {
                return;
            };
            let scene = SceneCopy::new(&device, SIZE, SIZE);
            let mut stage = stage_over(&device, &queue, &scene, leaked_stubs(), "no-such-root");
            let mut broken = stub_material();
            broken.textures.source = "effects/missing_d.dds".to_owned();
            let instances = [quad_instance(broken), quad_instance(stub_material())];
            let frame = frame();

            prepare_twice(&mut stage, &frame, &instances);
            let pixel = draw_stub(&device, &queue, &mut stage);

            assert_eq!(stage.draw_count(), 1, "the intact draw survives");
            assert_eq!(stage.counters().skipped_draws, 1);
            assert!(
                pixel[..3].iter().any(|channel| *channel != 0.0),
                "{pixel:?}"
            );
            prepare_with(&mut stage, &frame, &instances).unwrap();
            assert_eq!(
                stage.counters().skipped_draws,
                2,
                "counted again every frame"
            );
        }

        fn root_with_env_cube(name: &str) -> tempfile::TempDir {
            use crate::assets::types::texture::cube_test_support::{DdsSpec, Format, Header};
            let root = tempfile::TempDir::new().unwrap();
            let textures = root.path().join("textures").join("shared");
            std::fs::create_dir_all(&textures).unwrap();
            DdsSpec::cube(4, 3, Format::Bgra8, Header::Legacy).write(&textures, name);
            root
        }

        fn env_material(cube: &str, scale: f32) -> EffectMaterial {
            let mut material = stub_material();
            material.ps = stubs::ENV_PS.to_owned();
            material.textures.env = cube.to_owned();
            material.env_map_scale = scale;
            material
        }

        fn draw_env_quad(
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            root: &std::path::Path,
            material: EffectMaterial,
        ) -> (Option<[f32; 4]>, EffectCounters) {
            let scene = SceneCopy::new(device, SIZE, SIZE);
            let mut stage = stage_over(device, queue, &scene, leaked_stubs(), "unused");
            let mut cubes = cubes_over(&stage, root);
            let instances = [quad_instance(material)];
            let frame = frame();
            prepare_with_cubes(&mut stage, &mut cubes, &frame, &instances).unwrap();
            stage.wait_for_warmup().unwrap();
            prepare_with_cubes(&mut stage, &mut cubes, &frame, &instances).unwrap();
            let pixel = (stage.draw_count() > 0).then(|| draw_stub(device, queue, &mut stage));
            (pixel, stage.counters())
        }

        #[test]
        fn an_environment_permutation_reflects_its_own_cube_scaled_by_the_environment_scale() {
            let Some((device, queue)) = test_device("effect stage environment cube") else {
                return;
            };
            let root = root_with_env_cube("cube.dds");
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

            let (pixel, counters) = draw_env_quad(
                &device,
                &queue,
                root.path(),
                env_material(r"shared\cube.dds", 0.5),
            );

            let pixel = pixel.expect("the draw is not skipped");
            assert_eq!(counters.skipped_draws, 0);
            assert!(
                (pixel[0] - 0.5).abs() < 0.01 && pixel[1] < 0.01 && pixel[2] < 0.01,
                "the +X face is red, scaled by 0.5: {pixel:?}"
            );
            assert!(pollster::block_on(scope.pop()).is_none());
        }

        #[test]
        fn an_environment_permutation_without_a_cube_reflects_black() {
            let Some((device, queue)) = test_device("effect stage environment without cube") else {
                return;
            };
            let root = root_with_env_cube("unused.dds");

            let (pixel, counters) =
                draw_env_quad(&device, &queue, root.path(), env_material("", 1.0));

            let pixel = pixel.expect("the draw is not skipped");
            assert_eq!(counters.skipped_draws, 0);
            assert!(
                pixel[..3].iter().all(|channel| *channel < 0.01),
                "{pixel:?}"
            );
        }

        #[test]
        fn an_environment_cube_that_fails_to_load_skips_the_draw_and_counts_it() {
            let Some((device, queue)) = test_device("effect stage missing environment cube") else {
                return;
            };
            let root = root_with_env_cube("present.dds");

            let (pixel, counters) = draw_env_quad(
                &device,
                &queue,
                root.path(),
                env_material("shared/absent.dds", 1.0),
            );

            assert_eq!(counters.skipped_draws, 2, "one per prepared frame");
            assert_eq!(pixel, None, "nothing is drawn for the skipped material");
        }

        #[test]
        fn the_cube_of_a_permutation_without_the_environment_bit_is_not_loaded() {
            let Some((device, queue)) = test_device("effect stage env cube ignored") else {
                return;
            };
            let root = root_with_env_cube("present.dds");
            let mut material = stub_material();
            material.textures.env = "shared/absent.dds".to_owned();

            let (pixel, counters) = draw_env_quad(&device, &queue, root.path(), material);

            assert!(pixel.is_some());
            assert_eq!(counters.skipped_draws, 0);
        }

        #[test]
        fn a_malformed_shader_name_skips_the_draw_instead_of_failing_the_frame() {
            let Some((device, queue)) = test_device("effect stage malformed name") else {
                return;
            };
            let scene = SceneCopy::new(&device, SIZE, SIZE);
            let mut stage = stage_over(&device, &queue, &scene, leaked_stubs(), "unused");
            let mut malformed = stub_material();
            malformed.ps = "water_ps_basic".to_owned();
            let instances = [quad_instance(malformed), quad_instance(stub_material())];

            prepare_twice(&mut stage, &frame(), &instances);

            assert_eq!(stage.draw_count(), 1);
            assert_eq!(stage.counters().skipped_draws, 2, "one per prepared frame");
        }

        #[test]
        fn a_permutation_that_does_not_compile_is_skipped_not_fatal() {
            let Some((device, queue)) = test_device("effect stage invalid permutation") else {
                return;
            };
            let scene = SceneCopy::new(&device, SIZE, SIZE);
            let broken: &'static crate::rendering::shaders::translated::TranslatedShaders =
                Box::leak(Box::new(stubs::translated_with_mesh_vertex_wgsl(
                    "fn broken( {",
                )));
            let mut stage = stage_over(&device, &queue, &scene, broken, "unused");
            let instances = [quad_instance(stub_material())];

            prepare_twice(&mut stage, &frame(), &instances);

            assert_eq!(stage.draw_count(), 0);
            assert_eq!(stage.counters().skipped_draws, 1);
        }

        #[test]
        fn lights_are_only_gathered_when_a_draw_needs_them() {
            let Some((device, queue)) = test_device("effect stage lazy lights") else {
                return;
            };
            let scene = SceneCopy::new(&device, SIZE, SIZE);
            let mut stage = stage_over(&device, &queue, &scene, leaked_stubs(), "unused");
            let frame = frame();
            let gathers = std::cell::Cell::new(0);
            let mut cubes = cubes_over(&stage, std::path::Path::new("unused"));
            let mut run = |stage: &mut EffectStage, instances: &[EffectInstance]| {
                stage
                    .prepare(
                        &EffectFrameInputs {
                            frame: &frame,
                            near_plane: 0.5,
                            elapsed_seconds: 0.0,
                        },
                        &mut cubes,
                        |visit| instances.iter().for_each(visit),
                        |_| gathers.set(gathers.get() + 1),
                    )
                    .unwrap();
                stage.wait_for_warmup().unwrap();
            };
            let plain = [quad_instance(stub_material())];
            let tinted = [quad_instance(material_with_lighting_influence(0.3))];

            run(&mut stage, &plain);
            run(&mut stage, &plain);
            assert_eq!(gathers.get(), 0, "an unlit draw gathers nothing");
            run(&mut stage, &tinted);
            run(&mut stage, &tinted);

            assert_eq!(
                gathers.get(),
                1,
                "a lit draw gathers once, on the frame it is drawn"
            );
        }

        #[test]
        fn refilling_the_particle_pool_replaces_the_pending_list() {
            let Some((device, queue)) = test_device("effect stage particle replace") else {
                return;
            };
            let scene = SceneCopy::new(&device, SIZE, SIZE);
            let mut stage = stage_over(&device, &queue, &scene, leaked_stubs(), "unused");
            let material = Arc::new(stub_material());

            for _ in 0..3 {
                let pool = stage.particle_pool();
                pool.clear();
                pool.push_with(&material, |_| {});
                pool.push_with(&material, |_| {});
            }

            assert_eq!(stage.pending_particle_count(), 2);
        }

        #[test]
        fn particle_draws_are_ordered_with_meshes_and_drawn() {
            let Some((device, queue)) = test_device("effect stage particles") else {
                return;
            };
            let scene = SceneCopy::new(&device, SIZE, SIZE);
            let mut stage = stage_over(&device, &queue, &scene, leaked_stubs(), "unused");
            let mut material = stub_material_with_blend(&json!({ "mode": "alpha" }));
            material.vs = stubs::PARTICLE_VS.to_owned();
            material.ps = stubs::PARTICLE_PS.to_owned();
            let material = Arc::new(material);
            let corner = |x: f32, y: f32, u: f32, v: f32| ParticleVertex {
                position: [x, y, 0.5],
                sub_texture: 0.0,
                uv: [u, v],
                distortion: 1.0,
                colour: [255; 4],
            };
            let frame = frame();
            for _ in 0..2 {
                let pool = stage.particle_pool();
                pool.clear();
                pool.push_with(&material, |draw| {
                    draw.vertices.extend([
                        corner(-1.0, -1.0, 0.0, 1.0),
                        corner(1.0, -1.0, 1.0, 1.0),
                        corner(1.0, 1.0, 1.0, 0.0),
                        corner(-1.0, 1.0, 0.0, 0.0),
                    ]);
                    draw.indices.extend([0, 1, 2, 0, 2, 3]);
                    draw.subtexture_rects.push([0.0, 1.0, 0.0, 1.0]);
                    draw.bounds_centre = Vec3::new(0.0, 0.0, 0.5);
                });
                prepare_with(&mut stage, &frame, &[]).unwrap();
                stage.wait_for_warmup().unwrap();
            }
            let pixel = draw_stub(&device, &queue, &mut stage);

            assert_eq!(stage.draw_count(), 1);
            assert!(
                pixel[..3].iter().any(|channel| *channel != 0.0),
                "{pixel:?}"
            );
        }

        #[test]
        fn a_draw_with_a_shader_the_installer_did_not_write_is_a_fatal_error() {
            let Some((device, queue)) = test_device("effect stage missing shader") else {
                return;
            };
            let scene = SceneCopy::new(&device, SIZE, SIZE);
            let empty: &'static crate::rendering::shaders::translated::TranslatedShaders =
                Box::leak(Box::default());
            let mut stage = stage_over(&device, &queue, &scene, empty, "unused");
            let instances = [quad_instance(stub_material())];
            let frame = frame();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);

            let error = loop {
                match prepare_with(&mut stage, &frame, &instances) {
                    Ok(()) => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "no fatal error arrived"
                        );
                        std::thread::yield_now();
                    }
                    Err(error) => break error,
                }
            };

            assert!(matches!(error, EffectStageError::Shader(_)), "{error}");
            assert_eq!(
                stage.warmup.failed_pipelines.len(),
                1,
                "the key is failed, so it is not requested again"
            );
        }
    }
}
