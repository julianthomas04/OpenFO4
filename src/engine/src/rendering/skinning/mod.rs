pub mod bind_layout;
pub mod character;
pub mod creatures;
pub mod gpu_mesh;
pub mod materials;
pub mod pipelines;
pub mod rig;
pub mod scene;
pub mod skin_vertex;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use glam::Mat4;

use crate::rendering::frustum::Aabb;
use bind_layout::{SkinnedBindLayout, SkinnedFrameBuffers};
use character::{CharacterId, CharacterInstance, CharacterKind};
use gpu_mesh::GpuSkinnedPart;
pub use materials::MaterialOverride;
use materials::{CharacterGpuContext, SurfaceKind};
use rig::{CreatureDef, CreatureLoadError, CreatureRig, PropModel};
use scene::{PartDraw, SkinnedScene};

use crate::rendering::{
    frustum,
    gpu_driven::{
        GpuInstance, INSTANCE_FLAGS_BLEND, INSTANCE_FLAGS_MULTIPLY, INSTANCE_FLAGS_TWO_SIDED,
    },
    shadows::lamps::ShadowInvalidations,
    texture_streaming::residency::TextureResidency,
};

const STATS_LOG_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
enum SpawnError {
    #[error("creature index {0} is not in the creature table")]
    UnknownCreature(usize),
    #[error(transparent)]
    Load(#[from] CreatureLoadError),
}

enum PendingSpawn {
    Creature(SpawnRequest),
    Prop {
        key: PropKey,
        overrides: Arc<[MaterialOverride]>,
        transform: Mat4,
    },
}

/// What identifies a prop's rig: the model plus the material swap its placement carries.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PropKey {
    /// Relative to the asset root, e.g. `meshes/x.glb`.
    pub model_path: String,
    /// Identifies `overrides`: placements with the same model and swap share a rig.
    pub swap: Option<String>,
}

/// The per-placement material swaps a prop spawns with.
#[derive(Clone, Debug)]
pub struct PropLook {
    pub swap: Option<String>,
    pub overrides: Arc<[MaterialOverride]>,
}

/// A prop model being read by a loader thread, or read and shared by the rigs of its swaps.
enum PropModelState {
    Parsing(mpsc::Receiver<Result<PropModel, CreatureLoadError>>),
    Ready(Arc<PropModel>),
    /// The file could not be read; every swap of the model fails without re-reading it.
    Failed,
}

enum PropRig {
    Ready(Arc<CreatureRig>),
    Loading,
    Failed,
}

pub struct SpawnRequest {
    pub creature: usize,
    pub transform: Mat4,
}

/// Owns the spawned characters and their per-frame GPU data. Requests from the
/// debug panel are queued and applied in `update`, where GPU services exist.
pub struct CharacterSystem {
    asset_root: String,
    creatures: Vec<CreatureDef>,
    layout: SkinnedBindLayout,
    buffers: SkinnedFrameBuffers,
    rigs: HashMap<usize, Arc<CreatureRig>>,
    prop_rigs: HashMap<PropKey, Arc<CreatureRig>>,
    prop_models: HashMap<String, PropModelState>,
    /// Props (model and swap) that failed to load, so each is reported and tried once.
    failed_props: HashSet<PropKey>,
    failed_spawns: Vec<CharacterId>,
    characters: Vec<CharacterInstance>,
    next_id: u32,
    pending_spawns: Vec<(CharacterId, PendingSpawn)>,
    pending_despawns: Vec<CharacterId>,
    spawn_error: Option<String>,
    retired_shadow_regions: Vec<Aabb>,
    draws: Vec<PartDraw>,
    visible: Vec<bool>,
    world_bounds: Vec<Aabb>,
    palette: Vec<Mat4>,
    instances: Vec<GpuInstance>,
    palette_bases: Vec<u32>,
    /// A character spawned or despawned since the frame data was built.
    roster_changed: bool,
    last_stats_log: Instant,
}

impl CharacterSystem {
    pub fn new(device: &wgpu::Device, asset_root: &str) -> Self {
        let layout = SkinnedBindLayout::new(device);
        Self {
            asset_root: asset_root.to_owned(),
            creatures: creatures::discover(asset_root),
            buffers: SkinnedFrameBuffers::new(device, &layout),
            layout,
            rigs: HashMap::new(),
            prop_rigs: HashMap::new(),
            prop_models: HashMap::new(),
            failed_props: HashSet::new(),
            failed_spawns: Vec::new(),
            characters: Vec::new(),
            next_id: 0,
            pending_spawns: Vec::new(),
            pending_despawns: Vec::new(),
            spawn_error: None,
            retired_shadow_regions: Vec::new(),
            draws: Vec::new(),
            visible: Vec::new(),
            world_bounds: Vec::new(),
            palette: Vec::new(),
            instances: Vec::new(),
            palette_bases: Vec::new(),
            roster_changed: false,
            last_stats_log: Instant::now(),
        }
    }

    pub fn bind_layout(&self) -> &SkinnedBindLayout {
        &self.layout
    }

    /// The creatures that can be spawned; `SpawnRequest::creature` indexes this.
    pub fn creatures(&self) -> &[CreatureDef] {
        &self.creatures
    }

    /// Reserves the id the character gets once the spawn is applied; a failed
    /// spawn never uses its id.
    pub fn queue_spawn(&mut self, request: SpawnRequest) -> CharacterId {
        self.queue(PendingSpawn::Creature(request))
    }

    /// Queues a static prop's own skinned model (`model_path` relative to the
    /// asset root, e.g. `meshes/x.glb`) as a character posed from outside. The
    /// model is read off-thread, so the spawn may take several `apply_pending`
    /// calls to appear.
    pub fn queue_prop_spawn(
        &mut self,
        model_path: String,
        look: &PropLook,
        transform: Mat4,
    ) -> CharacterId {
        let key = PropKey {
            model_path,
            swap: look.swap.clone(),
        };
        self.queue(PendingSpawn::Prop {
            key,
            overrides: Arc::clone(&look.overrides),
            transform,
        })
    }

    fn queue(&mut self, pending: PendingSpawn) -> CharacterId {
        let id = CharacterId(self.next_id);
        self.next_id += 1;
        self.pending_spawns.push((id, pending));
        id
    }

    /// Prop spawns that failed since the last call; they will never appear.
    pub fn take_failed_spawns(&mut self) -> Vec<CharacterId> {
        std::mem::take(&mut self.failed_spawns)
    }

    pub fn character(&self, id: CharacterId) -> Option<&CharacterInstance> {
        self.characters.iter().find(|character| character.id == id)
    }

    pub fn character_mut(&mut self, id: CharacterId) -> Option<&mut CharacterInstance> {
        self.characters
            .iter_mut()
            .find(|character| character.id == id)
    }

    pub fn queue_despawn(&mut self, id: CharacterId) {
        self.pending_despawns.push(id);
    }

    pub fn characters(&self) -> &[CharacterInstance] {
        &self.characters
    }

    pub fn characters_mut(&mut self) -> &mut [CharacterInstance] {
        &mut self.characters
    }

    /// Cause of the most recent failed spawn, until the next spawn succeeds.
    pub fn spawn_error(&self) -> Option<&str> {
        self.spawn_error.as_deref()
    }

    pub fn update(&mut self, dt: f32, view_proj: &Mat4, gpu: &mut CharacterGpuContext) {
        self.apply_pending(gpu);
        self.visible.clear();
        if self.characters.is_empty() {
            self.draws.clear();
            self.world_bounds.clear();
            return;
        }
        self.pose_characters(dt);
        if self.frame_data_stale() {
            self.world_bounds.clear();
            self.world_bounds
                .extend(self.characters.iter().map(CharacterInstance::world_bounds));
            self.build_frame_data();
            self.buffers.upload(
                gpu.device,
                gpu.queue,
                &self.layout,
                &self.instances,
                &self.palette,
                &self.palette_bases,
            );
        }
        self.cull(view_proj);
        self.log_stats();
    }

    pub fn scene(&self) -> SkinnedScene<'_> {
        SkinnedScene {
            bind_group: self.buffers.bind_group(),
            characters: &self.characters,
            draws: &self.draws,
            visible: &self.visible,
            world_bounds: &self.world_bounds,
        }
    }

    /// Characters are always at full detail: the streamer's demand sweep knows nothing of them.
    pub fn note_texture_demand(&self, residency: &mut TextureResidency) {
        for character in &self.characters {
            for mesh in &character.rig().meshes {
                for handle in mesh.texture_refs() {
                    residency.note_demand(handle.slot, f32::INFINITY);
                }
            }
        }
    }

    /// Cached shadow maps must redraw wherever a character changed pose or
    /// place, or left. A still, unmoved character invalidates nothing.
    pub fn invalidate_shadows(&mut self, invalidations: &mut ShadowInvalidations) {
        for region in self.retired_shadow_regions.drain(..) {
            invalidations.region(region);
        }
        for character in &mut self.characters {
            if let Some(region) = character.take_shadow_region() {
                invalidations.region(region);
            }
        }
    }

    /// Applies the queued despawns and every spawn whose model is ready.
    pub fn apply_pending(&mut self, gpu: &mut CharacterGpuContext) {
        self.apply_despawns(gpu);
        self.apply_spawns(gpu);
    }

    /// Releases the texture references of a rig no character uses any more;
    /// its clip packs are dropped with it and reload on the next spawn.
    fn apply_despawns(&mut self, gpu: &mut CharacterGpuContext) {
        for id in self.pending_despawns.drain(..) {
            self.pending_spawns.retain(|(pending, _)| *pending != id);
            let Some(position) = self
                .characters
                .iter()
                .position(|character| character.id == id)
            else {
                continue;
            };
            let character = self.characters.remove(position);
            self.roster_changed = true;
            self.retired_shadow_regions
                .push(character.shadow_footprint());
            let rig = Arc::clone(character.rig_handle());
            drop(character);
            if Arc::strong_count(&rig) == 2 {
                self.rigs.retain(|_, cached| !Arc::ptr_eq(cached, &rig));
                self.prop_rigs
                    .retain(|_, cached| !Arc::ptr_eq(cached, &rig));
                rig.release_textures(gpu);
            }
        }
    }

    fn apply_spawns(&mut self, gpu: &mut CharacterGpuContext) {
        for (id, pending) in std::mem::take(&mut self.pending_spawns) {
            match pending {
                PendingSpawn::Creature(request) => match self.rig_for(request.creature, gpu) {
                    Ok(rig) => {
                        tracing::info!(
                            "skin: spawned {} bones={} clip_packs={}",
                            rig.name,
                            rig.skeleton.len(),
                            rig.packs().len()
                        );
                        self.characters.push(CharacterInstance::new(
                            id,
                            CharacterKind::Creature,
                            rig,
                            request.transform,
                        ));
                        self.spawn_error = None;
                    }
                    Err(error) => {
                        tracing::error!("skin: spawn failed: {error}");
                        self.spawn_error = Some(error.to_string());
                    }
                },
                PendingSpawn::Prop {
                    key,
                    overrides,
                    transform,
                } => match self.prop_rig_for(&key, &overrides, gpu) {
                    PropRig::Ready(rig) => {
                        tracing::debug!(
                            "skin: spawned prop {} bones={}",
                            key.model_path,
                            rig.skeleton.len()
                        );
                        self.characters.push(CharacterInstance::new(
                            id,
                            CharacterKind::Prop,
                            rig,
                            transform,
                        ));
                    }
                    PropRig::Loading => self.pending_spawns.push((
                        id,
                        PendingSpawn::Prop {
                            key,
                            overrides,
                            transform,
                        },
                    )),
                    PropRig::Failed => self.failed_spawns.push(id),
                },
            }
        }
        let awaited: HashSet<&str> = self
            .pending_spawns
            .iter()
            .filter_map(|(_, pending)| match pending {
                PendingSpawn::Prop { key, .. } => Some(key.model_path.as_str()),
                PendingSpawn::Creature(_) => None,
            })
            .collect();
        let has_rig = |path: &str| self.prop_rigs.keys().any(|key| key.model_path == path);
        self.prop_models
            .retain(|path, _| awaited.contains(path.as_str()) || has_rig(path));
    }

    /// The shared rig of a prop model. The model file is parsed on a loader
    /// thread (`Loading` until it arrives); a failure is logged once per model
    /// and not retried.
    fn prop_rig_for(
        &mut self,
        key: &PropKey,
        overrides: &[MaterialOverride],
        gpu: &mut CharacterGpuContext,
    ) -> PropRig {
        if let Some(rig) = self.prop_rigs.get(key) {
            return PropRig::Ready(Arc::clone(rig));
        }
        let path = &key.model_path;
        if self.failed_props.contains(key) {
            return PropRig::Failed;
        }
        let state = self.prop_models.entry(path.clone()).or_insert_with(|| {
            let full_path = format!("{}/{path}", self.asset_root);
            let (sender, receiver) = mpsc::channel();
            rayon::spawn(move || {
                if sender.send(PropModel::parse(&full_path)).is_err() {
                    tracing::debug!("skin: prop {full_path} was parsed after nobody waited for it");
                }
            });
            PropModelState::Parsing(receiver)
        });
        if let PropModelState::Parsing(receiver) = state {
            match receiver.try_recv() {
                Ok(Ok(model)) => *state = PropModelState::Ready(Arc::new(model)),
                Ok(Err(error)) => {
                    tracing::error!("skin: prop {path} cannot be skinned: {error}");
                    *state = PropModelState::Failed;
                }
                Err(mpsc::TryRecvError::Empty) => return PropRig::Loading,
                Err(mpsc::TryRecvError::Disconnected) => {
                    tracing::error!("skin: the loader of prop {path} stopped without a result");
                    *state = PropModelState::Failed;
                }
            }
        }
        let model = match state {
            PropModelState::Ready(model) => model,
            PropModelState::Failed => {
                self.failed_props.insert(key.clone());
                return PropRig::Failed;
            }
            PropModelState::Parsing(_) => return PropRig::Loading,
        };
        match CreatureRig::from_prop_model(path, model, overrides, gpu) {
            Ok(rig) => {
                let rig = Arc::new(rig);
                self.prop_rigs.insert(key.clone(), Arc::clone(&rig));
                PropRig::Ready(rig)
            }
            Err(error) => {
                tracing::error!("skin: prop {path} cannot be skinned: {error}");
                self.failed_props.insert(key.clone());
                PropRig::Failed
            }
        }
    }

    fn rig_for(
        &mut self,
        creature: usize,
        gpu: &mut CharacterGpuContext,
    ) -> Result<Arc<CreatureRig>, SpawnError> {
        if let Some(rig) = self.rigs.get(&creature) {
            return Ok(Arc::clone(rig));
        }
        let def = self
            .creatures
            .get(creature)
            .ok_or(SpawnError::UnknownCreature(creature))?;
        let rig = Arc::new(CreatureRig::load(def, &self.asset_root, gpu)?);
        self.rigs.insert(creature, Arc::clone(&rig));
        Ok(rig)
    }

    fn pose_characters(&mut self, dt: f32) {
        for character in &mut self.characters {
            if let Err(error) = character.update_pose(dt) {
                tracing::error!(
                    "skin: pose update of {} #{} failed: {error}",
                    character.rig().name,
                    character.id.0
                );
            }
        }
    }

    /// Whether any character changed since the palettes and instances were built;
    /// still characters (frozen ragdoll props) cost nothing per frame.
    fn frame_data_stale(&mut self) -> bool {
        let roster_changed = std::mem::take(&mut self.roster_changed);
        let mut any_dirty = false;
        for character in &mut self.characters {
            any_dirty |= character.take_frame_dirty();
        }
        roster_changed || any_dirty
    }

    fn build_frame_data(&mut self) {
        self.draws.clear();
        self.palette.clear();
        self.instances.clear();
        self.palette_bases.clear();
        for (character_index, character) in self.characters.iter_mut().enumerate() {
            if character.is_faulted() {
                continue;
            }
            character.palette_offset = self.palette.len() as u32;
            character.write_palette(&mut self.palette);
            let rig = character.rig();
            for (mesh_index, mesh) in rig.meshes.iter().enumerate() {
                for (part_index, part) in mesh.parts.iter().enumerate() {
                    self.draws.push(PartDraw {
                        character: character_index,
                        mesh: mesh_index,
                        part: part_index,
                        instance_index: self.instances.len() as u32,
                    });
                    self.instances.push(gpu_instance(
                        character.transform(),
                        &self.world_bounds[character_index],
                        part,
                    ));
                    self.palette_bases.push(
                        character.palette_offset + rig.palette_start(mesh_index, part.skin) as u32,
                    );
                }
            }
        }
    }

    fn cull(&mut self, view_proj: &Mat4) {
        let planes = frustum::clip_planes(view_proj);
        self.visible.extend(
            self.world_bounds
                .iter()
                .map(|bounds| !bounds.is_outside(&planes)),
        );
    }

    fn log_stats(&mut self) {
        if self.last_stats_log.elapsed() >= STATS_LOG_INTERVAL {
            self.last_stats_log = Instant::now();
            tracing::info!(
                "skin: palette n_instances={} n_bones={}",
                self.characters.len(),
                self.palette.len()
            );
        }
    }
}

fn gpu_instance(transform: &Mat4, world: &Aabb, part: &GpuSkinnedPart) -> GpuInstance {
    let material = &part.material;
    let mut flags = 0;
    if material.kind.is_blended() {
        flags |= INSTANCE_FLAGS_BLEND;
    }
    if material.kind == SurfaceKind::Multiply {
        flags |= INSTANCE_FLAGS_MULTIPLY;
    }
    if material.two_sided {
        flags |= INSTANCE_FLAGS_TWO_SIDED;
    }
    GpuInstance {
        transform: transform.to_cols_array_2d(),
        aabb_min: world.min.to_array(),
        _pad0: 0.0,
        aabb_max: world.max.to_array(),
        _pad1: 0.0,
        index_offset: 0,
        index_count: part.index_count,
        vertex_offset: 0,
        material_id: material.material_id,
        alpha_cutoff: material.alpha_cutoff,
        alpha_mult: material.alpha_mult,
        flags,
        _pad2: 0,
    }
}
