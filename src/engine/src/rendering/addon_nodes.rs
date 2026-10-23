//! Particle systems of placed models: the model an addon record names, spawned at each of its
//! `BSValueNode` attachments, and the systems a placed model carries itself. Attachments and
//! hosts live and die with their cell: each frame the set the streamer holds is compared with
//! the spawned one. Only attachments near the camera run; when a cap turns a near one away,
//! the farthest live one makes room. Model files are parsed on a worker thread.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use glam::{Mat4, Vec3, Vec4};

use super::effects::stage::EffectStage;
use super::particles::constants::{DESPAWN_HYSTERESIS_EU, EVICTION_MARGIN_EU, SPAWN_RADIUS_EU};
use super::particles::{
    BackgroundCache, CameraBasis, InstanceId, Lookup, ParticleSystemTemplate, ParticleWorld,
    ParticleWorldConfig, TemplateSet, node_transforms,
};
use super::world_streamer::{AddonAttachment, AttachmentKey, ParticleHost, WorldStreamer};
use crate::assets::types::glb_key::glb_key;
use crate::assets::types::mesh::CpuMesh;
use crate::assets::types::model_extras::effect_material;

#[derive(Debug, thiserror::Error)]
pub enum ParticleModelError {
    #[error("file unreadable: {0}")]
    Gltf(#[from] gltf::Error),
    #[error("{0}")]
    Model(#[from] anyhow::Error),
}

/// The particle systems of one model file.
#[derive(Debug)]
pub struct ParticleModel {
    pub templates: Vec<Arc<ParticleSystemTemplate>>,
    /// `BSMasterParticleSystem.max_emitter_objects` (0: the file sets no limit).
    pub max_emitter_objects: u32,
}

/// Where particle models come from. `load` runs on a worker thread.
pub trait ModelSource: Send + Sync + 'static {
    fn load(&self, model_path: &str) -> Result<ParticleModel, ParticleModelError>;
}

/// Reads converted GLB files below the asset root.
pub struct FileModels {
    asset_root: PathBuf,
}

impl FileModels {
    pub fn new(asset_root: &str) -> Self {
        Self {
            asset_root: PathBuf::from(asset_root),
        }
    }
}

impl ModelSource for FileModels {
    fn load(&self, model_path: &str) -> Result<ParticleModel, ParticleModelError> {
        let full_path = self.asset_root.join(model_path);
        let gltf::Gltf { document, blob } = gltf::Gltf::open(&full_path)?;
        let buffers = gltf::import_buffers(&document, full_path.parent(), blob)?;
        let model = CpuMesh::model_from_document(&document, &buffers, model_path)?;
        let nodes = node_transforms(&document, model_path);
        let materials: Vec<_> = document.materials().collect();
        let (set, without_material) = TemplateSet::build(&model, &nodes, |index| {
            let material = materials.get(index as usize)?;
            match effect_material(material) {
                Ok(parsed) => parsed.map(Arc::new),
                Err(error) => {
                    crate::utils::warn_once(
                        &format!("particle-material:{model_path}:{index}"),
                        || format!("{model_path}: effect material {index} unusable: {error}"),
                    );
                    None
                }
            }
        });
        report_template_findings(model_path, &set, &without_material);
        Ok(ParticleModel {
            templates: set.templates,
            max_emitter_objects: model
                .extras
                .master_particle_systems
                .iter()
                .map(|master| master.value.max_emitter_objects)
                .max()
                .unwrap_or(0),
        })
    }
}

fn report_template_findings(model_path: &str, set: &TemplateSet, without_material: &[String]) {
    for error in &set.failed {
        crate::utils::warn_once(&format!("particle-template:{model_path}:{error}"), || {
            format!("{model_path}: {error}; the system is skipped")
        });
    }
    if !without_material.is_empty() {
        crate::utils::warn_once(&format!("particle-material:{model_path}"), || {
            format!(
                "{model_path}: no effect material for particle systems {without_material:?}; they are skipped"
            )
        });
    }
    let unsupported: Vec<String> = set
        .unsupported_fields()
        .map(|(system, fields)| format!("{system}: {}", fields.join(", ")))
        .collect();
    if !unsupported.is_empty() {
        crate::utils::warn_once(&format!("particle-unsupported:{model_path}"), || {
            format!(
                "{model_path}: authored particle fields the simulation ignores: {}",
                unsupported.join("; ")
            )
        });
    }
}

/// Loads each model file once on a worker thread; a file that fails is reported once and stays
/// unavailable.
struct ModelCache {
    models: BackgroundCache<ParticleModel>,
}

impl ModelCache {
    fn new<S: ModelSource>(source: S) -> Self {
        Self {
            models: BackgroundCache::new("particle-models", move |model_path| {
                match source.load(model_path) {
                    Ok(model) => Some(model),
                    Err(error) => {
                        crate::utils::warn_once(&format!("particle-model:{model_path}"), || {
                            format!("particle model {model_path}: {error}; nothing spawns from it")
                        });
                        None
                    }
                }
            }),
        }
    }

    fn get(&mut self, model_path: &str) -> Lookup<ParticleModel> {
        self.models.get(model_path)
    }
}

/// What one attachment or host asks to spawn.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnRequest {
    /// Model path relative to the asset root.
    pub model_path: Arc<str>,
    /// ADDN master particle system cap (0: none).
    pub cap: u16,
    /// Engine space <- model.
    pub transform: Mat4,
}

impl SpawnRequest {
    fn position(&self) -> Vec3 {
        self.transform.w_axis.truncate()
    }
}

/// An attachment point or host the streamer holds, as the sync sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Wanted {
    pub owner: InstanceId,
    pub what: WantedKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WantedKind {
    Spawn(SpawnRequest),
    /// An addon node whose index no addon record answers to.
    Unresolved {
        index: i32,
    },
    /// An addon record without a model: nothing to spawn here (its light is the streamer's).
    Nothing,
}

const HOST_OWNER_BIT: u64 = 1 << 63;

/// The particle world owner of an addon attachment.
pub fn attachment_owner(key: AttachmentKey) -> InstanceId {
    (key.residency << 32) | u64::from(key.ordinal)
}

/// The particle world owner of a placed model's own systems.
pub fn host_owner(key: AttachmentKey) -> InstanceId {
    HOST_OWNER_BIT | attachment_owner(key)
}

/// How many instances of one model may run at once: the addon record's cap when set, else the
/// master's `max_emitter_objects`; `None` when neither limits it.
pub fn instance_cap(addon_cap: u16, max_emitter_objects: u32) -> Option<u32> {
    match (addon_cap, max_emitter_objects) {
        (0, 0) => None,
        (0, master) => Some(master),
        (addon, _) => Some(u32::from(addon)),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Entry {
    /// Running; `spawned < total` while the world was full for some of its systems, which are
    /// retried every sync.
    Live {
        model_path: Arc<str>,
        position: Vec3,
        spawned: usize,
        total: usize,
    },
    /// A cap (the model's or the world's) turned it away; retried every sync.
    Refused,
    /// Its model is still being read.
    Loading,
    ModelUnavailable,
    NoSystems,
    Unresolved,
    Nothing,
}

impl Entry {
    fn is_complete(&self) -> bool {
        match self {
            Self::Live { spawned, total, .. } => spawned >= total,
            Self::Refused | Self::Loading => false,
            Self::ModelUnavailable | Self::NoSystems | Self::Unresolved | Self::Nothing => true,
        }
    }
}

/// Why a spawn did not complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// The model already runs as often as its cap allows.
    ModelCap,
    /// The world holds as many systems as it may.
    WorldFull,
}

enum Placement {
    Placed(Entry),
    Refused {
        refusal: Refusal,
        partial: Option<Entry>,
    },
}

/// Counts of what the sync has done, for logging and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AddonFxStats {
    pub live: usize,
    pub refused: usize,
    pub loading: usize,
    pub model_unavailable: usize,
    pub no_systems: usize,
    pub unresolved: usize,
}

/// What an addon record answers for a node index: `(glb path, cap)`.
type AddonRow = Option<(Option<Arc<str>>, u16)>;

pub struct AddonFx<S = FileModels> {
    _source: std::marker::PhantomData<S>,
    models: ModelCache,
    world: ParticleWorld,
    entries: HashMap<InstanceId, Entry>,
    instances: HashMap<Arc<str>, u32>,
    seen: HashSet<InstanceId>,
    wanted: Vec<Wanted>,
    /// The streamer's [`WorldStreamer::loaded_generation`] that `wanted` was built from.
    wanted_generation: Option<u64>,
    addon_rows: HashMap<i32, AddonRow>,
    candidates: Vec<(f32, usize)>,
    /// Scopes in which this sync found no live attachment far enough to give way.
    world_has_no_victim: bool,
    models_without_victim: HashSet<Arc<str>>,
    models_reported_full: HashSet<Arc<str>>,
    logged: Option<AddonFxStats>,
}

impl AddonFx<FileModels> {
    pub fn new(asset_root: &str) -> Self {
        Self::with_source(FileModels::new(asset_root), ParticleWorldConfig::default())
    }
}

impl<S: ModelSource> AddonFx<S> {
    pub fn with_source(source: S, config: ParticleWorldConfig) -> Self {
        Self {
            _source: std::marker::PhantomData,
            models: ModelCache::new(source),
            world: ParticleWorld::new(config),
            entries: HashMap::new(),
            instances: HashMap::new(),
            seen: HashSet::new(),
            wanted: Vec::new(),
            wanted_generation: None,
            addon_rows: HashMap::new(),
            candidates: Vec::new(),
            world_has_no_victim: false,
            models_without_victim: HashSet::new(),
            models_reported_full: HashSet::new(),
            logged: None,
        }
    }

    pub fn world(&self) -> &ParticleWorld {
        &self.world
    }

    pub fn stats(&self) -> AddonFxStats {
        let mut stats = AddonFxStats::default();
        for entry in self.entries.values() {
            match entry {
                Entry::Live { .. } => stats.live += 1,
                Entry::Refused => stats.refused += 1,
                Entry::Loading => stats.loading += 1,
                Entry::ModelUnavailable => stats.model_unavailable += 1,
                Entry::NoSystems => stats.no_systems += 1,
                Entry::Unresolved => stats.unresolved += 1,
                Entry::Nothing => {}
            }
        }
        stats
    }

    /// Brings the spawned systems in line with what the streamer holds. The list of what the
    /// resident cells want is rebuilt only when the streamer's resident set changed.
    pub fn sync(&mut self, streamer: &WorldStreamer, camera_position: Vec3) {
        let generation = streamer.loaded_generation();
        let mut wanted = std::mem::take(&mut self.wanted);
        if self.wanted_generation != Some(generation) {
            self.wanted_generation = Some(generation);
            Self::collect_wanted(&mut self.addon_rows, streamer, &mut wanted);
        }
        self.sync_with(&wanted, camera_position);
        self.wanted = wanted;
    }

    fn collect_wanted(
        addon_rows: &mut HashMap<i32, AddonRow>,
        streamer: &WorldStreamer,
        wanted: &mut Vec<Wanted>,
    ) {
        wanted.clear();
        let rows = addon_rows;
        streamer.for_each_addon_attachment(|attachment| {
            wanted.push(addon_request(attachment, |index| {
                rows.entry(index)
                    .or_insert_with(|| {
                        streamer.addon_node(index).map(|row| {
                            (
                                row.model_path
                                    .as_deref()
                                    .map(|path| Arc::from(glb_key(path))),
                                row.cap,
                            )
                        })
                    })
                    .clone()
            }));
        });
        streamer.for_each_particle_host(|host| wanted.push(host_request(host)));
    }

    /// Spawns what is new and near, retries what a cap refused (nearest first, evicting the
    /// farthest live one for a nearer one), and despawns what is gone or far away.
    pub fn sync_with(&mut self, wanted: &[Wanted], camera_position: Vec3) {
        self.seen.clear();
        self.seen.extend(wanted.iter().map(|item| item.owner));
        let gone: Vec<InstanceId> = self
            .entries
            .keys()
            .filter(|owner| !self.seen.contains(owner))
            .copied()
            .collect();
        for owner in gone {
            self.remove(owner);
        }
        self.remove_distant(camera_position);
        self.collect_candidates(wanted, camera_position);
        self.world_has_no_victim = false;
        self.models_without_victim.clear();
        let mut candidates = std::mem::take(&mut self.candidates);
        for &(distance_squared, index) in &candidates {
            self.place_with_eviction(&wanted[index], distance_squared.sqrt(), camera_position);
        }
        candidates.clear();
        self.candidates = candidates;
        self.log_changes();
    }

    fn remove_distant(&mut self, camera_position: Vec3) {
        let limit = SPAWN_RADIUS_EU + DESPAWN_HYSTERESIS_EU;
        let distant: Vec<InstanceId> = self
            .entries
            .iter()
            .filter_map(|(owner, entry)| match entry {
                Entry::Live { position, .. }
                    if position.distance_squared(camera_position) > limit * limit =>
                {
                    Some(*owner)
                }
                _ => None,
            })
            .collect();
        for owner in distant {
            self.remove(owner);
        }
    }

    /// Resolves what needs no spawn at once and queues, nearest first, what does.
    fn collect_candidates(&mut self, wanted: &[Wanted], camera_position: Vec3) {
        self.candidates.clear();
        for (index, item) in wanted.iter().enumerate() {
            if self
                .entries
                .get(&item.owner)
                .is_some_and(Entry::is_complete)
            {
                continue;
            }
            let WantedKind::Spawn(request) = &item.what else {
                let entry = Self::resolve_without_spawn(&item.what);
                self.entries.insert(item.owner, entry);
                continue;
            };
            let distance_squared = request.position().distance_squared(camera_position);
            if distance_squared > SPAWN_RADIUS_EU * SPAWN_RADIUS_EU {
                if matches!(
                    self.entries.get(&item.owner),
                    Some(Entry::Refused | Entry::Loading)
                ) {
                    self.entries.remove(&item.owner);
                }
                continue;
            }
            self.candidates.push((distance_squared, index));
        }
        self.candidates
            .sort_by(|(near, _), (far, _)| near.total_cmp(far));
    }

    fn resolve_without_spawn(what: &WantedKind) -> Entry {
        match what {
            WantedKind::Unresolved { index } => {
                crate::utils::warn_once(&format!("addon_unresolved:{index}"), || {
                    format!("addon node index {index} has no addon record; nothing spawns")
                });
                Entry::Unresolved
            }
            WantedKind::Nothing | WantedKind::Spawn(_) => Entry::Nothing,
        }
    }

    fn place_with_eviction(&mut self, item: &Wanted, distance: f32, camera_position: Vec3) {
        let WantedKind::Spawn(request) = &item.what else {
            return;
        };
        loop {
            match self.place(item.owner, request) {
                Placement::Placed(entry) => {
                    self.entries.insert(item.owner, entry);
                    return;
                }
                Placement::Refused { refusal, partial } => {
                    self.entries
                        .insert(item.owner, partial.unwrap_or(Entry::Refused));
                    if !self.evict_farther(refusal, &request.model_path, distance, camera_position)
                    {
                        self.note_refusal(request);
                        return;
                    }
                }
            }
        }
    }

    fn note_refusal(&mut self, request: &SpawnRequest) {
        if self
            .models_reported_full
            .insert(Arc::clone(&request.model_path))
        {
            tracing::info!(
                "{}: a cap is reached; nearer attachments take the slots first",
                request.model_path
            );
        }
    }

    /// Despawns the farthest live attachment (of `model_path` when the model's cap refused)
    /// if it is clearly farther than `distance`. Returns whether it made room.
    fn evict_farther(
        &mut self,
        refusal: Refusal,
        model_path: &Arc<str>,
        distance: f32,
        camera_position: Vec3,
    ) -> bool {
        let searched_before = match refusal {
            Refusal::ModelCap => self.models_without_victim.contains(model_path),
            Refusal::WorldFull => self.world_has_no_victim,
        };
        if searched_before {
            return false;
        }
        let victim = self
            .entries
            .iter()
            .filter_map(|(owner, entry)| match entry {
                Entry::Live {
                    model_path: live_model,
                    position,
                    ..
                } if refusal == Refusal::WorldFull || live_model == model_path => {
                    Some((*owner, position.distance(camera_position)))
                }
                _ => None,
            })
            .max_by(|(_, near), (_, far)| near.total_cmp(far))
            .filter(|&(_, victim_distance)| victim_distance > distance + EVICTION_MARGIN_EU);
        if let Some((owner, _)) = victim {
            self.remove(owner);
            return true;
        }
        match refusal {
            Refusal::ModelCap => {
                self.models_without_victim.insert(Arc::clone(model_path));
            }
            Refusal::WorldFull => self.world_has_no_victim = true,
        }
        false
    }

    fn place(&mut self, owner: InstanceId, request: &SpawnRequest) -> Placement {
        let model = match self.models.get(&request.model_path) {
            Lookup::Pending => return Placement::Placed(Entry::Loading),
            Lookup::Unavailable => return Placement::Placed(Entry::ModelUnavailable),
            Lookup::Ready(model) => model,
        };
        if model.templates.is_empty() {
            return Placement::Placed(Entry::NoSystems);
        }
        let total = model.templates.len();
        let already = if let Some(Entry::Live { spawned, .. }) = self.entries.get(&owner) {
            *spawned
        } else {
            let running = self
                .instances
                .get(&request.model_path)
                .copied()
                .unwrap_or(0);
            if instance_cap(request.cap, model.max_emitter_objects)
                .is_some_and(|cap| running >= cap)
            {
                return Placement::Refused {
                    refusal: Refusal::ModelCap,
                    partial: None,
                };
            }
            0
        };
        let spawned = already
            + self
                .world
                .spawn_all(owner, &model.templates[already..], request.transform);
        if spawned == already {
            crate::utils::warn_once("particle_world_full", || {
                "particle world is full; further systems wait for a free slot".to_owned()
            });
            return Placement::Refused {
                refusal: Refusal::WorldFull,
                partial: (already > 0).then(|| live(request, spawned, total)),
            };
        }
        if already == 0 {
            *self
                .instances
                .entry(Arc::clone(&request.model_path))
                .or_insert(0) += 1;
        }
        let entry = live(request, spawned, total);
        if spawned < total {
            return Placement::Refused {
                refusal: Refusal::WorldFull,
                partial: Some(entry),
            };
        }
        Placement::Placed(entry)
    }

    fn remove(&mut self, owner: InstanceId) {
        let Some(entry) = self.entries.remove(&owner) else {
            return;
        };
        if let Entry::Live { model_path, .. } = entry {
            self.world.despawn(owner);
            if let Some(count) = self.instances.get_mut(&model_path) {
                *count = count.saturating_sub(1);
            }
        }
    }

    fn log_changes(&mut self) {
        let stats = self.stats();
        if self.logged != Some(stats) {
            self.logged = Some(stats);
            tracing::info!(
                "addon particles: {} live, {} refused by caps, {} loading, {} unresolved, {} without a model, {} without systems",
                stats.live,
                stats.refused,
                stats.loading,
                stats.unresolved,
                stats.model_unavailable,
                stats.no_systems
            );
        }
    }

    pub fn update(&mut self, dt: f32, camera_position: Vec3) {
        self.world.update(dt, camera_position);
    }

    /// Hands this frame's particle quads to the effect stage.
    pub fn submit(&self, camera: &CameraBasis, planes: &[Vec4], stage: &mut EffectStage) {
        let pool = stage.particle_pool();
        pool.clear();
        self.world.collect_draws(camera, planes, pool);
    }
}

fn live(request: &SpawnRequest, spawned: usize, total: usize) -> Entry {
    Entry::Live {
        model_path: Arc::clone(&request.model_path),
        position: request.position(),
        spawned,
        total,
    }
}

/// What an addon attachment asks for, given the addon record its index names (`None` when no
/// record answers to the index).
pub fn addon_request(
    attachment: AddonAttachment<'_>,
    row_of: impl FnOnce(i32) -> AddonRow,
) -> Wanted {
    let owner = attachment_owner(attachment.key);
    let what = match row_of(attachment.addon.index) {
        None => WantedKind::Unresolved {
            index: attachment.addon.index,
        },
        Some((None, _)) => WantedKind::Nothing,
        Some((Some(model_path), cap)) => WantedKind::Spawn(SpawnRequest {
            model_path,
            cap,
            transform: attachment.transform,
        }),
    };
    Wanted { owner, what }
}

fn host_request(host: ParticleHost<'_>) -> Wanted {
    Wanted {
        owner: host_owner(host.key),
        what: WantedKind::Spawn(SpawnRequest {
            model_path: Arc::clone(host.model_path),
            cap: 0,
            transform: host.transform,
        }),
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals or exact compositions of them"
)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use serde_json::{Value, json};

    use super::*;
    use crate::assets::types::effect_material::EffectMaterial;
    use crate::assets::types::model_extras::{AddonNodeExtras, NodeExtras, parse_versioned};
    use crate::assets::types::particle_system::ParticleSystem;
    use crate::rendering::particles::SystemInstance;

    fn material() -> Arc<EffectMaterial> {
        Arc::new(crate::rendering::effects::test_fixtures::stub_material())
    }

    fn system_json() -> Value {
        json!({
            "v": 1, "world_space": true, "max_particles": 10, "material": 0,
            "fade": { "near_begin": 0, "near_end": 0, "far_begin": 0, "far_end": 0 },
            "data": { "aspect_ratio": 1.0, "aspect_flags": 0, "speed_to_aspect": [1, 0, 1],
                      "subtexture_rects": [] },
            "emitters": [{
                "type": "box", "name": "Emitter", "node": "EmitterNode",
                "width": 0, "height": 0, "depth": 0,
                "speed": 0, "speed_variation": 0, "declination": 0, "declination_variation": 0,
                "planar_angle": 0, "planar_angle_variation": 0,
                "initial_color": [1, 1, 1, 1], "initial_radius": 1.0, "radius_variation": 0,
                "life_span": 100.0, "life_span_variation": 0,
                "emitter_ctlr": { "birth_rate": { "constant": 30 }, "active": { "constant": true },
                                  "cycle": "loop", "start": 0.0, "stop": 1.0 }
            }],
            "modifiers": [],
            "update": { "frequency": 1.0, "phase": 0.0 }
        })
    }

    fn template() -> Arc<ParticleSystemTemplate> {
        let node = NodeExtras {
            node_name: "System".to_owned(),
            transform: Mat4::IDENTITY,
            value: parse_versioned::<ParticleSystem>("particle_system", &system_json()).unwrap(),
        };
        let nodes = HashMap::from([("EmitterNode".to_owned(), Mat4::IDENTITY)]);
        Arc::new(ParticleSystemTemplate::new(&node, &nodes, &HashMap::new(), material()).unwrap())
    }

    struct FakeModels {
        models: HashMap<String, (usize, u32)>,
        loads: Arc<Mutex<HashMap<String, usize>>>,
    }

    impl FakeModels {
        fn with(models: &[(&str, usize, u32)]) -> Self {
            Self {
                models: models
                    .iter()
                    .map(|&(path, systems, limit)| (path.to_owned(), (systems, limit)))
                    .collect(),
                loads: Arc::default(),
            }
        }
    }

    impl ModelSource for FakeModels {
        fn load(&self, model_path: &str) -> Result<ParticleModel, ParticleModelError> {
            *self
                .loads
                .lock()
                .unwrap()
                .entry(model_path.to_owned())
                .or_default() += 1;
            let &(systems, max_emitter_objects) = self
                .models
                .get(model_path)
                .ok_or_else(|| anyhow::anyhow!("not found"))?;
            Ok(ParticleModel {
                templates: (0..systems).map(|_| template()).collect(),
                max_emitter_objects,
            })
        }
    }

    struct Harness {
        fx: AddonFx<FakeModels>,
        loads: Arc<Mutex<HashMap<String, usize>>>,
    }

    impl Harness {
        /// Syncs, waits for the model loads that sync started, and syncs again.
        fn sync(&mut self, wanted: &[Wanted]) {
            self.sync_from(Vec3::ZERO, wanted);
        }

        fn sync_from(&mut self, camera: Vec3, wanted: &[Wanted]) {
            self.fx.sync_with(wanted, camera);
            self.fx.models.models.wait_idle();
            self.fx.sync_with(wanted, camera);
        }
    }

    fn fx(models: &[(&str, usize, u32)], max_systems: usize) -> Harness {
        let source = FakeModels::with(models);
        let loads = Arc::clone(&source.loads);
        Harness {
            fx: AddonFx::with_source(
                source,
                ParticleWorldConfig {
                    max_systems,
                    ..ParticleWorldConfig::default()
                },
            ),
            loads,
        }
    }

    /// An attachment `distance` eu from the camera at the origin.
    fn spawn_at(owner: InstanceId, model_path: &str, cap: u16, distance: f32) -> Wanted {
        Wanted {
            owner,
            what: WantedKind::Spawn(SpawnRequest {
                model_path: Arc::from(model_path),
                cap,
                transform: Mat4::from_translation(Vec3::new(distance, 0.0, 0.0)),
            }),
        }
    }

    fn spawn(owner: InstanceId, model_path: &str, cap: u16) -> Wanted {
        spawn_at(owner, model_path, cap, owner as f32)
    }

    fn live_owners(harness: &Harness) -> Vec<InstanceId> {
        let mut owners: Vec<_> = harness
            .fx
            .world()
            .systems()
            .map(SystemInstance::owner)
            .collect();
        owners.sort_unstable();
        owners.dedup();
        owners
    }

    #[test]
    fn attachments_spawn_their_systems_and_despawn_when_gone() {
        let mut harness = fx(&[("fire.glb", 2, 0)], 100);

        harness.sync(&[spawn(1, "fire.glb", 0), spawn(2, "fire.glb", 0)]);
        assert_eq!(harness.fx.world().system_count(), 4);
        assert_eq!(harness.fx.stats().live, 2);

        harness.sync(&[spawn(2, "fire.glb", 0)]);
        assert_eq!(live_owners(&harness), vec![2]);
        assert_eq!(harness.fx.world().system_count(), 2);

        harness.sync(&[]);
        assert_eq!(harness.fx.world().system_count(), 0);
        assert_eq!(harness.fx.stats(), AddonFxStats::default());
    }

    #[test]
    fn a_model_is_pending_until_the_worker_has_read_it() {
        let mut harness = fx(&[("fire.glb", 1, 0)], 100);

        harness.fx.sync_with(&[spawn(1, "fire.glb", 0)], Vec3::ZERO);
        assert_eq!(harness.fx.stats().loading, 1);
        assert_eq!(harness.fx.world().system_count(), 0);

        harness.fx.models.models.wait_idle();
        harness.fx.sync_with(&[spawn(1, "fire.glb", 0)], Vec3::ZERO);
        assert_eq!(harness.fx.stats().live, 1);
        assert_eq!(harness.fx.stats().loading, 0);
    }

    #[test]
    fn a_persisting_attachment_is_not_spawned_twice() {
        let mut harness = fx(&[("fire.glb", 1, 0)], 100);
        for _ in 0..3 {
            harness.sync(&[spawn(1, "fire.glb", 0)]);
        }
        assert_eq!(harness.fx.world().system_count(), 1);
    }

    #[test]
    fn the_addon_cap_limits_instances_of_a_model_and_frees_with_despawn() {
        let mut harness = fx(&[("fire.glb", 1, 0)], 100);
        let all = [
            spawn(1, "fire.glb", 2),
            spawn(2, "fire.glb", 2),
            spawn(3, "fire.glb", 2),
        ];

        harness.sync(&all);
        assert_eq!(harness.fx.stats().live, 2);
        assert_eq!(harness.fx.stats().refused, 1);
        assert_eq!(live_owners(&harness), vec![1, 2], "the nearer two run");

        harness.sync(&[all[1].clone(), all[2].clone()]);
        assert_eq!(
            harness.fx.stats().live,
            2,
            "the refused one takes the freed slot"
        );
        assert_eq!(harness.fx.stats().refused, 0);
    }

    #[test]
    fn the_master_limit_applies_when_the_addon_sets_no_cap() {
        let mut harness = fx(&[("fire.glb", 1, 1)], 100);

        harness.sync(&[spawn(1, "fire.glb", 0), spawn(2, "fire.glb", 0)]);

        assert_eq!(harness.fx.stats().live, 1);
        assert_eq!(harness.fx.stats().refused, 1);
    }

    #[test]
    fn an_addon_cap_overrides_the_master_limit() {
        assert_eq!(instance_cap(5, 1), Some(5));
        assert_eq!(instance_cap(0, 3), Some(3));
        assert_eq!(instance_cap(0, 0), None);
    }

    #[test]
    fn caps_count_each_model_separately() {
        let mut harness = fx(&[("a.glb", 1, 0), ("b.glb", 1, 0)], 100);

        harness.sync(&[spawn(1, "a.glb", 1), spawn(2, "b.glb", 1)]);

        assert_eq!(harness.fx.stats().live, 2);
    }

    #[test]
    fn a_full_world_refuses_the_attachment_and_retries_later() {
        let mut harness = fx(&[("fire.glb", 1, 0)], 1);

        harness.sync(&[spawn(1, "fire.glb", 0), spawn(2, "fire.glb", 0)]);
        assert_eq!(harness.fx.stats().live, 1);
        assert_eq!(harness.fx.stats().refused, 1);

        harness.sync(&[spawn(2, "fire.glb", 0)]);
        assert_eq!(harness.fx.stats().live, 1);
        assert_eq!(harness.fx.stats().refused, 0);
    }

    #[test]
    fn a_nearer_attachment_takes_the_slot_of_the_farthest_live_one() {
        let mut harness = fx(&[("fire.glb", 1, 0)], 1);
        harness.sync(&[spawn_at(1, "fire.glb", 0, 2000.0)]);
        assert_eq!(live_owners(&harness), vec![1]);

        harness.sync(&[
            spawn_at(1, "fire.glb", 0, 2000.0),
            spawn_at(2, "fire.glb", 0, 100.0),
        ]);

        assert_eq!(live_owners(&harness), vec![2]);
        assert_eq!(harness.fx.stats().live, 1);
        assert_eq!(
            harness.fx.stats().refused,
            1,
            "the evicted one waits for a slot"
        );
    }

    #[test]
    fn eviction_needs_a_clear_distance_margin() {
        let mut harness = fx(&[("fire.glb", 1, 0)], 1);
        harness.sync(&[spawn_at(1, "fire.glb", 0, 1000.0)]);

        harness.sync(&[
            spawn_at(1, "fire.glb", 0, 1000.0),
            spawn_at(2, "fire.glb", 0, 1000.0 - EVICTION_MARGIN_EU / 2.0),
        ]);

        assert_eq!(
            live_owners(&harness),
            vec![1],
            "no swapping for a marginal gain"
        );
    }

    #[test]
    fn a_model_cap_evicts_only_among_instances_of_that_model() {
        let mut harness = fx(&[("a.glb", 1, 0), ("b.glb", 1, 0)], 100);
        harness.sync(&[
            spawn_at(1, "a.glb", 1, 3000.0),
            spawn_at(2, "b.glb", 1, 3500.0),
        ]);

        harness.sync(&[
            spawn_at(1, "a.glb", 1, 3000.0),
            spawn_at(2, "b.glb", 1, 3500.0),
            spawn_at(3, "a.glb", 1, 50.0),
        ]);

        assert_eq!(
            live_owners(&harness),
            vec![2, 3],
            "b's farther instance is left alone"
        );
    }

    #[test]
    fn attachments_beyond_the_spawn_radius_do_not_run() {
        let mut harness = fx(&[("fire.glb", 1, 0)], 100);

        harness.sync(&[spawn_at(1, "fire.glb", 0, SPAWN_RADIUS_EU + 1.0)]);

        assert_eq!(harness.fx.world().system_count(), 0);
        assert_eq!(harness.fx.stats().refused, 0);
    }

    #[test]
    fn a_live_attachment_survives_the_hysteresis_band_and_despawns_beyond_it() {
        let mut harness = fx(&[("fire.glb", 1, 0)], 100);
        let wanted = [spawn_at(1, "fire.glb", 0, SPAWN_RADIUS_EU - 1000.0)];
        let camera_at = |distance: f32| Vec3::new(-distance, 0.0, 0.0);
        // Distances from the camera to the attachment as the camera backs away.
        let in_band = SPAWN_RADIUS_EU + DESPAWN_HYSTERESIS_EU / 2.0 - (SPAWN_RADIUS_EU - 1000.0);
        let beyond = SPAWN_RADIUS_EU + DESPAWN_HYSTERESIS_EU + 1.0 - (SPAWN_RADIUS_EU - 1000.0);

        harness.sync_from(Vec3::ZERO, &wanted);
        assert_eq!(harness.fx.stats().live, 1);

        harness.sync_from(camera_at(in_band), &wanted);
        assert_eq!(
            harness.fx.stats().live,
            1,
            "inside the band it keeps running"
        );

        harness.sync_from(camera_at(beyond), &wanted);
        assert_eq!(harness.fx.world().system_count(), 0);
        assert_eq!(harness.fx.stats().live, 0);

        harness.sync_from(camera_at(in_band), &wanted);
        assert_eq!(
            harness.fx.world().system_count(),
            0,
            "it does not respawn inside the band"
        );

        harness.sync_from(Vec3::ZERO, &wanted);
        assert_eq!(
            harness.fx.stats().live,
            1,
            "back within the radius it runs again"
        );
    }

    #[test]
    fn a_partly_spawned_attachment_keeps_retrying_its_missing_systems() {
        let mut harness = fx(&[("one.glb", 1, 0), ("two.glb", 2, 0)], 2);

        harness.sync(&[
            spawn_at(1, "one.glb", 0, 10.0),
            spawn_at(2, "two.glb", 0, 20.0),
        ]);
        assert_eq!(
            harness.fx.world().system_count(),
            2,
            "the world holds one system of two.glb"
        );

        harness.sync(&[spawn_at(2, "two.glb", 0, 20.0)]);
        assert_eq!(
            harness.fx.world().system_count(),
            2,
            "both of its systems run now"
        );
        assert_eq!(live_owners(&harness), vec![2]);
        assert_eq!(harness.fx.stats().live, 1);
    }

    #[test]
    fn a_missing_model_is_counted_and_loaded_once() {
        let mut harness = fx(&[], 100);

        harness.sync(&[spawn(1, "gone.glb", 0), spawn(2, "gone.glb", 0)]);
        harness.sync(&[spawn(1, "gone.glb", 0), spawn(2, "gone.glb", 0)]);

        assert_eq!(harness.fx.stats().model_unavailable, 2);
        assert_eq!(harness.fx.world().system_count(), 0);
        assert_eq!(harness.loads.lock().unwrap()["gone.glb"], 1);
    }

    #[test]
    fn a_model_without_systems_is_counted() {
        let mut harness = fx(&[("empty.glb", 0, 0)], 100);

        harness.sync(&[spawn(1, "empty.glb", 0)]);

        assert_eq!(harness.fx.stats().no_systems, 1);
    }

    #[test]
    fn unresolved_indices_are_counted_and_forgotten_with_the_cell() {
        let mut harness = fx(&[("fire.glb", 1, 0)], 100);
        let unresolved = Wanted {
            owner: 9,
            what: WantedKind::Unresolved { index: 77 },
        };

        harness.sync(&[unresolved, spawn(1, "fire.glb", 0)]);
        assert_eq!(harness.fx.stats().unresolved, 1);
        assert_eq!(harness.fx.stats().live, 1);

        harness.sync(&[]);
        assert_eq!(harness.fx.stats().unresolved, 0);
    }

    #[test]
    fn attachment_and_host_owners_never_collide() {
        let key = AttachmentKey {
            residency: 3,
            ordinal: 5,
        };
        assert_ne!(attachment_owner(key), host_owner(key));
        let other = AttachmentKey {
            residency: 3,
            ordinal: 6,
        };
        assert_ne!(attachment_owner(key), attachment_owner(other));
    }

    fn attachment(addon: &AddonNodeExtras, transform: Mat4) -> AddonAttachment<'_> {
        AddonAttachment {
            key: AttachmentKey {
                residency: 1,
                ordinal: 0,
            },
            node_name: "ValueNode",
            addon,
            transform,
        }
    }

    #[test]
    fn an_attachment_spawns_its_records_model_at_the_attachment_transform() {
        let addon = AddonNodeExtras {
            v: 1,
            index: 5,
            flags: 0,
        };
        let transform = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0));

        let wanted = addon_request(attachment(&addon, transform), |index| {
            assert_eq!(index, 5);
            Some((Some(Arc::from("meshes\\fire.glb")), 4))
        });

        assert_eq!(
            wanted.what,
            WantedKind::Spawn(SpawnRequest {
                model_path: Arc::from("meshes\\fire.glb"),
                cap: 4,
                transform,
            })
        );
    }

    #[test]
    fn an_unknown_index_and_a_light_only_record_are_told_apart() {
        let addon = AddonNodeExtras {
            v: 1,
            index: 9,
            flags: 0,
        };
        let unknown = addon_request(attachment(&addon, Mat4::IDENTITY), |_| None);
        let light_only = addon_request(attachment(&addon, Mat4::IDENTITY), |_| Some((None, 0)));

        assert_eq!(unknown.what, WantedKind::Unresolved { index: 9 });
        assert_eq!(light_only.what, WantedKind::Nothing);
    }

    #[test]
    fn a_moved_attachment_keeps_its_spawn_transform_composition() {
        // The streamer composes placement * node transform; the spawn receives it unchanged.
        let placement = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0));
        let node = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2)
            * Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0));
        let addon = AddonNodeExtras {
            v: 1,
            index: 1,
            flags: 0,
        };

        let wanted = addon_request(attachment(&addon, placement * node), |_| {
            Some((Some(Arc::from("m.glb")), 0))
        });

        let WantedKind::Spawn(request) = wanted.what else {
            panic!("expected a spawn");
        };
        assert_eq!(
            request.transform.transform_point3(Vec3::ZERO),
            Vec3::new(10.0, 1.0, 0.0)
        );
    }
}
