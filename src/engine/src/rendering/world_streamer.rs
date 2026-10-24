use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use glam::{Mat3, Mat4, Vec3};
use tracing::{debug, error, info, info_span, warn};

use crate::anim::{AnimationRig, JointIndex};
use crate::assets::AssetServer;
use crate::assets::db::DbClient;
use crate::assets::db::addon_nodes::AddonNodes;
use crate::assets::db::landscapes::Landscape;
use crate::assets::db::lights::Light;
use crate::assets::db::material_swaps::SwapEntry;
use crate::assets::db::placed_objects::PlacedObject;
use crate::assets::layout::{MESHES_DIR, TEXTURES_DIR, below_meshes, texture_key};
use crate::assets::types::animated_mesh::{AnimatedModel, LoadedModel, ModelAsset, ModelLoadError};
use crate::assets::types::effect_params::{EffectBlend, EffectParams, EnvParams};
use crate::assets::types::glb_key::glb_key;
use crate::assets::types::grass::CellGrass;
use crate::assets::types::grass_placement::GrassSettings;
use crate::assets::types::material_name::MaterialName;
use crate::assets::types::mesh::CpuMesh;
use crate::assets::types::model_extras::ModelExtras;
use crate::assets::types::terrain;
use crate::assets::types::texture::{self, TexturePayload};
use crate::assets::types::water_shader::WaterShaderToken;
use crate::assets::types::weapon_assembly::ModRow;
use crate::assets::world_settings::LoadedGrid;
use crate::audio::place::PlaceSnapshot;
use crate::bethesda_space;
use crate::navmesh::{CellNavmeshes, NavMesh, NavResidency, NavWorld, build_meshes};
use crate::rendering::animated_objects::{AnimatedObjectEvent, AnimatedRefr};
use crate::rendering::cube_array::CubeArray;
use crate::rendering::effects::geometry::{EffectInstance, EffectMeshData, RefractionInstance};
use crate::rendering::frustum::Aabb;
use crate::rendering::gpu_driven::{
    DRAW_BUCKET_COUNT, FALLBACK_MATERIAL_ID, FALLBACK_SLOT, GpuInstance, GpuTerrainInstance,
    INSTANCE_FLAGS_BLEND, INSTANCE_FLAGS_DECAL, INSTANCE_FLAGS_EFFECT, INSTANCE_FLAGS_HIDDEN_FX,
    INSTANCE_FLAGS_MARKER, INSTANCE_FLAGS_MULTIPLY, INSTANCE_FLAGS_TWO_SIDED, INSTANCE_FLAGS_WATER,
    InstanceBuffer, LEGACY_SURFACE, MaterialKey, MaterialRegistry, MaterialSlots,
    SPLAT_FALLBACK_LAYER, SlotChanges, SplatmapArray, SurfaceParams, TERRAIN_FLAGS_BAKED,
    TerrainInstanceBuffer, TextureArray, TextureHandle, potential_buckets,
};
use crate::rendering::gpu_slab::{GpuSlab, SlabElement};
use crate::rendering::grass::{GrassModelCache, GrassModels, GrassStage};
use crate::rendering::grass_streaming::GrassWorkerData;
use crate::rendering::mega_buffer::MegaBuffer;
use crate::rendering::placed_light::{
    GpuPlacedLight, LightBase, LightBaseError, LightPlacement, PlacedLight, PlacedLightError,
};
use crate::rendering::ragdoll_objects::{RagdollEvent, RagdollRefr, SkinPlan};
use crate::rendering::shadows::lamps::{
    ShadowInvalidations,
    source::{LampCandidate, LampKey, ShadowLampSource},
};
use crate::rendering::skinning::materials::CharacterGpuContext;
use crate::rendering::skinning::{MaterialOverride, PropLook};
use crate::rendering::texture_streaming::demand::{
    self, CameraMotion, DemandView, NO_DEMAND, ScreenView,
};
use crate::rendering::texture_streaming::material::{MaterialSource, build_gpu_material};
use crate::rendering::texture_streaming::role::{Admission, admit_texture};
use crate::rendering::texture_streaming::{
    ResidencyPolicy, TextureResidency, residency, uv_density,
};
use crate::rendering::vertex::Vertex;
use crate::rendering::water::PlacedWater;

mod one_shot;
use crate::shared_cache::SharedCache;
use crate::sim::collision::{
    CellColliderBatch, CollisionMeta, ObjectColliderGroup, PartShape, Placement, SidecarData,
    TerrainColliderSpec, load_manifest, load_sidecar_file, manifest_key,
};
use crate::sim::{EntityId, SimCommand, refr_entity_id};
use crate::water_world::{LoadedCell, VolumeShapes};
use one_shot::OneShot;

mod effect_shapes;
pub(crate) use effect_shapes::{AddonAttachment, AttachmentKey};
use effect_shapes::{
    ObjectVisibility, ShapeClass, ShapeFacts, ShapeRoute, StoredAttachment, attachments_of,
    classify,
};
mod fire_hooks;
use fire_hooks::{
    AddonLightSources, LightModelRef, LightSink, StoredParticleHost, particle_hosts_of,
    place_addon_lights,
};
pub(crate) use fire_hooks::{LightKey, ModelLight, ParticleHost};
mod load_pacing;
mod row_poses;
mod weapons;
use load_pacing::{IntegrationProgress, LoadFrameMeter, integration_must_stop};
use weapons::{
    WeaponAssemblies, WeaponBase, WeaponRecords, WeaponRefr, WeaponSinks, add_weapon_parts,
    trace_enabled,
};

const CELL_SIZE_BETH: f32 = 4096.0;
pub(crate) const SCALE: f32 = bethesda_space::UNITS_TO_ENGINE;
/// Path substring (case-insensitive) of the textures whose demand is traced.
const TEX_TRACE_ENV: &str = "OPENFO4_TEX_TRACE";
const TEX_TRACE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
const TEX_TRACE_MAX_LINES: usize = 20;
pub const CELL_SIZE_WORLD: f32 = CELL_SIZE_BETH * SCALE;
// The sim derives cell grids from positions with its own copy of this value.
const _: () = assert!(CELL_SIZE_WORLD == crate::sim::CELL_SIZE_WORLD);
pub(crate) const STREAM_RADIUS: i32 = 8;
/// The largest `uGridsToLoad` side the resident cells cover; a larger INI value is clamped.
pub(crate) const MAX_GRIDS_TO_LOAD: u32 = 2 * STREAM_RADIUS.unsigned_abs() + 1;
// Splat layers are only consumed by the full ring plus LOD cells without a
// baked level-4 tile; keep that count within the 2048-layer splatmap array.
// Overflow degrades gracefully (fallback layer + warning) but renders wrong
// splats.
const LOD_RADIUS: i32 = 35;

/// How aggressively cells are prepared and integrated.
///
/// `Streaming` is the gameplay mode: one active worker, one integrated cell
/// per frame, the 8 MB upload budget — tuned for hitch-free frame pacing.
/// `Loading` is for behind loading screens: the whole worker pool pulls
/// requests and the integrate/upload budgets open up — tuned to fill the
/// world as fast as the machine allows. The engine boots in Loading for the
/// initial spawn and drops to Streaming when `SPAWN_COMPLETE` fires, unless
/// pinned by `OPENFO4_LOAD_MODE=streaming|loading` or the debug-menu toggle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum LoadMode {
    Streaming,
    Loading,
}

impl LoadMode {
    fn label(self) -> &'static str {
        match self {
            LoadMode::Streaming => "streaming",
            LoadMode::Loading => "loading",
        }
    }
}

/// Loading-mode integrate budget. Early spawn frames are byte-budget bound
/// (every texture fresh), late frames are cell-count bound (warm cells cost
/// ~nothing) — 32 keeps the warm tail from capping below worker throughput
/// (~1000 cells/s at ~60 fps needs >16/frame).
const LOADING_CELLS_PER_FRAME: usize = 32;

/// Loading-mode upload budget. Behind a loading screen frame pacing doesn't
/// matter, and what does matter is amortization: early integrate frames pay
/// fixed per-frame costs (material bind-group rebuild over every resident
/// view while the slot high-water grows) regardless of how many cells land,
/// so fewer, denser frames integrate strictly faster. At 64 MB the first
/// seconds managed only ~2-3 fresh-texture cells per frame against the
/// 16-cell budget (measured 2026-06-12, `logs/bench_cpf`).
const LOADING_UPLOAD_BUDGET_BYTES: u64 = 256 * 1024 * 1024;

/// REFR-level classification of placed effect models for the debug "Show FX"
/// toggle: base records whose model lives under `Effects\` (mist and fog cards,
/// god-ray and light-beam cones, steam, drips), except `Effects\DecalsPlaced\`,
/// which is ordinary scorch and stain decal geometry. The only placed FX outside
/// `Effects\` are the `Water\FXWaterfall*` movable statics, caught by the fx
/// filename prefix, which applies to movable statics only (`fx_filename_rule`):
/// STAT fx-named lookalikes like `Interiors\...\IBeam01` are real geometry.
/// Per-shape effect classification is `MaterialName::is_hidden_effect_shape`.
fn is_fx_model(model_path: &str, fx_filename_rule: bool) -> bool {
    let lowered = model_path.to_ascii_lowercase().replace('/', "\\");
    let below = below_meshes(&lowered);
    if let Some(rest) = below.strip_prefix("effects\\") {
        !rest.starts_with("decalsplaced\\")
    } else if fx_filename_rule {
        below
            .rsplit('\\')
            .next()
            .is_some_and(|file| file.starts_with("fx"))
    } else {
        false
    }
}

struct BaseObjectInfo {
    model_path: String,
    is_marker: bool,
    /// Placed effect model (see `is_fx_model`): hidden unless the debug "Show FX"
    /// toggle shows it.
    is_fx: bool,
    lod_model_path: Option<String>,
    has_distant_lod: bool,
    /// Door, container, activator or furniture base: activatable once its model animates.
    interactable: bool,
    /// WATR form key of a water activator (`ACTI.WNAM`): its water shapes draw with it.
    water_type: Option<Arc<str>>,
    /// Set for a weapon: the REFR draws and collides as the assembly of its
    /// default mods instead of the single model at `model_path`.
    weapon: Option<Arc<WeaponBase>>,
}

/// The texture cache key of a terrain layer diffuse, which is also its path below the
/// asset root.
fn terrain_layer_texture_key(path: &str) -> String {
    format!("{TEXTURES_DIR}/{path}")
}

enum CellRequest {
    Full {
        gx: i32,
        gy: i32,
        worldspace: String,
        epoch: u64,
    },
    Lod {
        gx: i32,
        gy: i32,
        worldspace: String,
        epoch: u64,
    },
    /// A single interior cell, resolved by `form_key` (interiors have no grid or
    /// worldspace). `grid` is the synthetic bookkeeping key the result
    /// integrates under (the interior's anchor grid); the streamer holds
    /// `current_cell` there so the one cell stays in the collision ring.
    Interior {
        cell_form_key: String,
        grid: (i32, i32),
        epoch: u64,
    },
}

impl CellRequest {
    /// Worldspace generation this request belongs to (see
    /// [`WorldStreamer::epoch`]). A worker drops a pulled request whose epoch
    /// no longer matches the shared epoch instead of doing its work.
    fn epoch(&self) -> u64 {
        match self {
            CellRequest::Full { epoch, .. }
            | CellRequest::Lod { epoch, .. }
            | CellRequest::Interior { epoch, .. } => *epoch,
        }
    }
}

enum WorkerResponse {
    Full(CellLoadResult),
    Lod(CellLoadResult),
}

/// A GLB submesh parsed and ready for upload, with its local-space AABB and
/// material computed once at parse time (not per placed instance, not per
/// cell integration).
struct PreparedSubMesh {
    /// Mega-buffer key `{path}#{submesh index}`, built once on the worker.
    mega_key: Arc<str>,
    mesh: CpuMesh,
    local_aabb: Option<Aabb>,
    /// World units one UV unit spans on this submesh in local space (infinite
    /// without usable UVs, which asks for full detail). Computed once at parse
    /// time; texture demand scales it by each placement's scale.
    world_per_uv: f32,
    material: SubMeshMaterial,
    /// Present for parts of an animated model: the vertices are in the owning
    /// node's space and the node's joint places them.
    anim: Option<PartAnim>,
    /// The renderer that draws the submesh: effect and refraction shapes are
    /// not uploaded to the object buffers.
    route: ShapeRoute,
}

/// A model file parsed for placement: its submeshes plus the node-level extras.
struct PreparedModel {
    submeshes: Vec<PreparedSubMesh>,
    extras: ModelExtras,
}

impl PreparedModel {
    fn of_submeshes(submeshes: Vec<PreparedSubMesh>) -> Self {
        Self {
            submeshes,
            extras: ModelExtras::default(),
        }
    }

    /// Whether placing the model does anything: it has geometry or node extras.
    fn has_content(&self) -> bool {
        !self.submeshes.is_empty() || !self.extras.is_empty()
    }
}

impl std::ops::Deref for PreparedModel {
    type Target = [PreparedSubMesh];

    fn deref(&self) -> &[PreparedSubMesh] {
        &self.submeshes
    }
}

/// Where a part of an animated model sits in its skeleton.
struct PartAnim {
    rig: Arc<AnimationRig>,
    joint: JointIndex,
    /// The node's rest transform in model space; placing the part at rest.
    rest_in_model: Mat4,
}

impl PartAnim {
    /// Whether any clip can move the part.
    fn is_moving(&self) -> bool {
        self.rig.is_moving(self.joint)
    }
}

/// Parsed form of a GLB submesh's material name (the converter's
/// "diffuse|normal[|alpha tokens][|ts][|src:...]" contract). Parsed once on
/// the worker at GLB parse time: integration never re-parses strings, and the
/// worker knows exactly which DDS files to pre-read. The path strings are the
/// texture cache/refcount keys the main thread uses — single construction
/// site (material-swap overrides clone this struct with replaced paths, built
/// from installer-resolved entries — see `compute_swap_overrides`).
struct SubMeshMaterial {
    diffuse_path: Option<String>,
    normal_path: Option<String>,
    specular_path: Option<String>,
    glow_path: Option<String>,
    surface: SurfaceParams,
    alpha_cutoff: f32,
    alpha_mult: f32,
    is_blend: bool,
    two_sided: bool,
    /// Coplanar overlay decal ("decal" material token, from BGSM/BGEM
    /// bDecal/bDepthBias). Routed into a depth-biased draw bucket so it wins the
    /// depth test against the surface it sits on instead of z-fighting it.
    is_decal: bool,
    /// Untextured blend shape without effect parameters: CK helper meshes
    /// (`AttachRef` platform/rotate helpers, teleport and spline markers, the
    /// "`VisibilityEditorMarker`" boxes baked into 46 plant NIFs). Treated as
    /// marker geometry: hidden by default, revealed by the "Show markers"
    /// toggle, like the CK's M key. Effect shapes are never helpers.
    is_helper_shape: bool,
    /// Effect-shader parameters (BGEM glass, inline effect shapes); effect
    /// shapes always draw.
    effect: Option<EffectParams>,
    /// Environment reflection (BGEM or BGSM with environment mapping).
    env: Option<EnvParams>,
    /// `BSWaterShaderProperty` shape ("water" material token): drawn by the
    /// water stage, never by the object buckets, and not toggleable FX.
    water: Option<WaterShaderToken>,
    /// `fx` token: an effect-shader shape (water shapes included).
    is_effect_shader: bool,
    /// `eff` token: the converter wrote effect extras for this shape.
    has_effect_key: bool,
    /// `refr` token: a heat-haze shape.
    is_refraction: bool,
    /// Set from the shape's route at parse time; overrides the hidden-effect rule.
    object_visibility: ObjectVisibility,
    /// Normalized census key of the BGSM/BGEM this material came from
    /// ("materials/.../foo.bgsm", the "src:" name token). Material-swap
    /// entries match against this; None for inline-texture shapes, which no
    /// swap can target.
    source_material: Option<String>,
    /// Cached `gpu_key`: built on the worker at parse time so integration
    /// only hashes it.
    gpu_key: std::sync::OnceLock<Option<MaterialKey>>,
}

/// Manual `Clone`: the cached `gpu_key` is derived from the other fields, so a
/// clone (which callers may then mutate, e.g. swapped paths) starts with an
/// empty cache instead of inheriting a key that could be stale.
impl Clone for SubMeshMaterial {
    fn clone(&self) -> Self {
        Self {
            diffuse_path: self.diffuse_path.clone(),
            normal_path: self.normal_path.clone(),
            specular_path: self.specular_path.clone(),
            glow_path: self.glow_path.clone(),
            surface: self.surface,
            alpha_cutoff: self.alpha_cutoff,
            alpha_mult: self.alpha_mult,
            is_blend: self.is_blend,
            two_sided: self.two_sided,
            is_decal: self.is_decal,
            is_helper_shape: self.is_helper_shape,
            effect: self.effect.clone(),
            env: self.env.clone(),
            water: self.water,
            is_effect_shader: self.is_effect_shader,
            has_effect_key: self.has_effect_key,
            is_refraction: self.is_refraction,
            object_visibility: self.object_visibility,
            source_material: self.source_material.clone(),
            gpu_key: std::sync::OnceLock::new(),
        }
    }
}

impl SubMeshMaterial {
    fn source(&self) -> MaterialSource<'_> {
        MaterialSource {
            diffuse: self.diffuse_path.as_deref(),
            normal: self.normal_path.as_deref(),
            specular: self.specular_path.as_deref(),
            glow: self.glow_path.as_deref(),
            surface: &self.surface,
            effect: self.effect.as_ref(),
            env: self.env.as_ref(),
        }
    }

    /// `None` for shapes without a diffuse map that are not effect shapes: they
    /// draw the magenta fallback material.
    fn gpu_key(&self) -> Option<&MaterialKey> {
        self.gpu_key.get_or_init(|| self.source().key()).as_ref()
    }
}

/// Blend routing flags of a material: `BLEND`, plus `MULTIPLY` for a
/// multiplicative effect. Additive and alpha blends share the plain blend route.
fn blend_flags(mat: &SubMeshMaterial) -> u32 {
    if !mat.is_blend {
        return 0;
    }
    let multiplies = mat
        .effect
        .as_ref()
        .is_some_and(|effect| effect.blend == EffectBlend::Multiply);
    INSTANCE_FLAGS_BLEND
        | if multiplies {
            INSTANCE_FLAGS_MULTIPLY
        } else {
            0
        }
}

impl SubMeshMaterial {
    /// See `MaterialName::is_hidden_effect_shape`; the shape's route can override the rule.
    fn is_hidden_effect_shape(&self) -> bool {
        match self.object_visibility {
            ObjectVisibility::Hidden => true,
            ObjectVisibility::Drawn => false,
            ObjectVisibility::ByMaterialRule => MaterialName::hides_effect_shape_by_default(
                self.effect.is_some(),
                self.water.is_some(),
                self.source_material.as_deref(),
            ),
        }
    }
}

/// The visibility class of one instance: a marker (hidden unless "Show
/// markers"), hidden FX (hidden unless "Show FX"; an FX model or a non-BGEM
/// effect shape), or plain geometry. Marker semantics win over FX.
fn visibility_flag(placement_is_marker: bool, placement_is_fx: bool, mat: &SubMeshMaterial) -> u32 {
    if placement_is_marker || mat.is_helper_shape {
        INSTANCE_FLAGS_MARKER
    } else if placement_is_fx || mat.is_hidden_effect_shape() {
        INSTANCE_FLAGS_HIDDEN_FX
    } else {
        0
    }
}

/// Every instance flag a material determines on its own; the visibility class
/// also depends on the placement and is added by the caller.
fn material_instance_flags(mat: &SubMeshMaterial) -> u32 {
    let mut flags = blend_flags(mat);
    if mat.two_sided {
        flags |= INSTANCE_FLAGS_TWO_SIDED;
    }
    if mat.is_decal {
        flags |= INSTANCE_FLAGS_DECAL;
    }
    if mat.effect.is_some() {
        flags |= INSTANCE_FLAGS_EFFECT;
    }
    flags
}

impl SubMeshMaterial {
    /// An untextured blend shape without effect parameters is a CK helper,
    /// whose marker semantics win; every effect shape draws.
    fn from_name(name: MaterialName) -> Self {
        let is_helper_shape = name.is_helper_shape();
        Self {
            diffuse_path: name.diffuse_path,
            normal_path: name.normal_path,
            specular_path: name.specular_path,
            glow_path: name.glow_path,
            surface: name.surface,
            alpha_cutoff: name.alpha_cutoff,
            alpha_mult: name.alpha_mult,
            is_blend: name.is_blend,
            two_sided: name.two_sided,
            is_decal: name.is_decal,
            is_helper_shape,
            effect: name.effect,
            env: name.env,
            water: name.water,
            is_effect_shader: name.is_effect,
            has_effect_key: name.effect_key.is_some(),
            is_refraction: name.is_refraction,
            object_visibility: ObjectVisibility::ByMaterialRule,
            source_material: name.source_material,
            gpu_key: std::sync::OnceLock::default(),
        }
    }

    /// Decode a GLB material name. `None` (and the empty segments of a present
    /// name) mean "no texture", not a name to synthesize from — the magenta
    /// `material_id` 0 fallback is deliberate for marker gizmos. A name that
    /// fails to decode is logged once and also renders the magenta fallback.
    fn parse(name: Option<&str>) -> Self {
        let Some(name) = name else {
            return Self::from_name(MaterialName::untextured());
        };
        match MaterialName::parse(name) {
            Ok(decoded) => Self::from_name(decoded),
            Err(e) => {
                crate::utils::warn_once(name, || {
                    format!("material name '{name}' is invalid: {e}; using the fallback material")
                });
                Self::from_name(MaterialName::untextured())
            }
        }
    }
}

/// All placements of one (mesh path, material swap) combination within a
/// cell, sharing the parsed data. Variants of one mesh split into separate
/// groups but share geometry: mega-buffer slots, AABBs, and colliders are
/// keyed by path alone — only the per-instance material differs.
struct PreparedMeshGroup {
    /// Model path relative to the asset root.
    model_path: Arc<str>,
    submeshes: Arc<PreparedModel>,
    transforms: Vec<Mat4>,
    /// Part-to-weapon matrix per transform for the parts of assembled weapons;
    /// empty when every transform is a plain placement.
    locals: Vec<Mat4>,
    /// Editor-only marker placements (hidden unless the debug toggle is on).
    is_marker: bool,
    /// Placed effect models (hidden unless the debug "Show FX" toggle is on).
    is_fx: bool,
    /// WATR of the water shapes of this mesh (the placed activator's `WNAM`).
    water_type: Option<Arc<str>>,
    /// The collision of a water mesh: the volume its water fills.
    water_volume: Option<Arc<VolumeShapes>>,
    /// Material-swap overrides, one slot per submesh (empty when the group
    /// has no swap). None = the submesh keeps its GLB material.
    overrides: Vec<Option<SubMeshMaterial>>,
    /// Havok-dynamic clutter: each transform becomes a rapier rigid body, and
    /// its render instances are rewritten every frame from the body pose.
    dynamic: bool,
    /// The mesh's sidecar describes a ragdoll: its rows are tracked per entity
    /// so the skinned replacement can hide and restore them.
    ragdoll: bool,
    /// Per-transform REFR entity id (parallel to `transforms`), populated only
    /// when `dynamic` or the mesh is an animated model (empty otherwise). Keys
    /// the dynamic-instance row map so a pose maps back to its instance slots.
    entities: Vec<Option<EntityId>>,
}

/// Worker output: everything CPU-heavy (`SQLite`, GLB parse, terrain mesh
/// build, DDS read+parse) already done; the main thread only uploads and does
/// bookkeeping.
struct CellLoadResult {
    grid: (i32, i32),
    /// Worldspace generation this result was produced for, copied from its
    /// request. The main thread drops results whose epoch no longer matches
    /// (a worker was mid-cell when the worldspace switched): grid coords are
    /// per-worldspace, so a stale (gx,gy) must never integrate as the new
    /// worldspace's cell at the same grid.
    epoch: u64,
    terrain: Option<terrain::TerrainCellCpu>,
    meshes: Vec<PreparedMeshGroup>,
    /// Procedural grass per grass type, empty for LOD and interior cells.
    grass: Vec<CellGrass>,
    /// The parsed models `grass` places, prepared on the worker.
    grass_models: GrassModels,
    /// Placed lights of the cell, empty for LOD cells.
    lights: Vec<GpuPlacedLight>,
    /// The shadow-type lights among `lights`.
    shadow_lamps: Vec<ShadowLampSource>,
    /// The lights among `lights` whose record names a light model.
    light_models: Vec<LightModelRef>,
    /// How many of `lights` are the lights of addon nodes.
    addon_lights: u32,
    /// Static collision specs for the sim, built alongside the meshes.
    /// Always `None` for LOD cells (distant shells never collide).
    colliders: Option<Arc<CellColliderBatch>>,
    /// Pre-baked rapier shapes for `colliders` (sim-opaque; see `sim::bake`).
    baked_colliders: Option<crate::sim::bake::BakedCellColliders>,
    /// Activatable animated REFRs of the cell, handed to the animation runtime
    /// when the cell integrates. Always empty for LOD cells.
    animated_refs: Vec<AnimatedRefr>,
    /// Ragdoll-capable REFRs of the cell, handed to the ragdoll runtime when
    /// the cell integrates. Always empty for LOD cells.
    ragdoll_refs: Vec<RagdollRefr>,
    /// Pre-read mips for every texture this cell needs, keyed by the exact
    /// cache/refcount path strings the main thread uses. The worker reads only
    /// the mips the cell's distance calls for. Three states: `Some(Some)` =
    /// read and ready to upload; `Some(None)` = the worker tried and failed
    /// (missing/unreadable — already logged, the main thread must NOT probe
    /// the disk again); key absent = the worker believed it GPU-resident
    /// (main thread: `try_add_ref`, placeholder plus async load on the rare
    /// release race).
    textures: HashMap<String, Option<Arc<TexturePayload>>>,
    /// Navigation meshes of the cell, decoded and indexed on the worker. Empty for LOD cells.
    navmeshes: Vec<NavMesh>,
}

/// All placed instances of one mega-buffer slot within a cell. Keeping the
/// key alongside the instances lets the streamer re-patch the baked
/// offsets after a mega-buffer compaction.
struct CellObjectGroup {
    mega_key: Arc<str>,
    instances: Vec<GpuInstance>,
}

/// One water instance of a cell: where it sits in `CellResources::object_groups`
/// (so a mega-buffer remap that patches the instances patches the water draw
/// too) and what shades it.
struct WaterRef {
    group: usize,
    instance: usize,
    water_type: Option<Arc<str>>,
    volume: Option<Arc<VolumeShapes>>,
    shader: WaterShaderToken,
}

/// Where one dynamic REFR's instances live within a cell, so the per-frame
/// pose readback can rewrite them. One row per (submesh × this REFR) instance.
struct DynRow {
    /// Index into `CellResources.object_groups`.
    group_idx: usize,
    /// Index within that group's `instances` (== the placement's index).
    instance_idx: usize,
    /// The submesh's local-space AABB, re-transformed each frame so the GPU
    /// cull bounds track the moving body (None = the 1-unit fallback box).
    local_aabb: Option<Aabb>,
    /// The placement's uniform scale, recomposed with the body's
    /// translation/rotation (rapier poses carry no scale).
    scale: f32,
    /// Where the row sits within its REFR (a weapon part), composed after the
    /// body pose; identity for every other row.
    local: Mat4,
    /// Joint placing the row under a skeletal (animated) pose; `None` for
    /// rigid-body rows.
    joint: Option<JointIndex>,
    /// World matrix last written to the instance buffer; NaN until the first
    /// write so the first pose always lands. A transform within `POSE_EPS_*` of
    /// this one is skipped (no GPU write, no shadow invalidation).
    last_transform: Mat4,
}

/// Translation delta (engine units) below which a dynamic pose is unchanged.
const POSE_EPS_TRANSLATION: f32 = 0.01;
/// Per-element delta of the scale-rotation basis below which it is unchanged.
const POSE_EPS_BASIS: f32 = 1e-3;

/// Flattened slab base offset of each object group within a cell. The instance
/// slab is `object_groups` concatenated in group order — the same order
/// `upload_cell_instances` flattens and `repack_instances` rebuilds — so the
/// slot of group `g`'s instance `i` is `instance_range.offset + base[g] + i`.
/// Single source for that layout so the per-frame dynamic-pose rewrite can't
/// drift from how the slab is actually packed.
fn group_base_offsets(groups: &[CellObjectGroup]) -> Vec<u32> {
    let mut base = Vec::with_capacity(groups.len());
    let mut acc = 0u32;
    for g in groups {
        base.push(acc);
        acc += g.instances.len() as u32;
    }
    base
}

type PersistentBuckets = HashMap<(i32, i32), Vec<PlacedObject>>;
/// (mesh path, `is_marker`, `is_fx`, material swap, water type, is weapon part).
/// Weapon parts group apart from plain placements of the same mesh: their rows
/// follow the weapon's body even when the part's own mesh has no collision.
type GroupKey = (String, bool, bool, Option<String>, Option<Arc<str>>, bool);

/// The placements of one mesh group: world transforms, the parallel REFR
/// entity ids, and for weapon parts the parallel part-to-weapon matrices.
#[derive(Default)]
struct GroupInstances {
    transforms: Vec<Mat4>,
    entities: Vec<Option<EntityId>>,
    /// Empty while no placement is a weapon part (identity for every one then).
    locals: Vec<Mat4>,
}

impl GroupInstances {
    fn push(&mut self, transform: Mat4, entity: Option<EntityId>, local: Option<Mat4>) {
        if let Some(local) = local {
            self.locals.resize(self.transforms.len(), Mat4::IDENTITY);
            self.locals.push(local);
        } else if !self.locals.is_empty() {
            self.locals.push(Mat4::IDENTITY);
        }
        self.transforms.push(transform);
        self.entities.push(entity);
    }
}

/// World-space AABB of a mesh's local AABB under `transform` (its 8 transformed
/// corners), or a 1-unit box at the transform origin when the mesh has none.
/// Shared by the initial instance build and the per-frame dynamic-pose rewrite
/// so the GPU cull bounds match in both paths.
fn world_aabb(local: Option<&Aabb>, transform: &Mat4) -> ([f32; 3], [f32; 3]) {
    if let Some(aabb) = local {
        let mut wmin = Vec3::splat(f32::INFINITY);
        let mut wmax = Vec3::splat(f32::NEG_INFINITY);
        for c in &aabb.corners() {
            let w = transform.transform_point3(*c);
            wmin = wmin.min(w);
            wmax = wmax.max(w);
        }
        (wmin.to_array(), wmax.to_array())
    } else {
        let p = transform.w_axis;
        (
            [p.x - 1.0, p.y - 1.0, p.z - 1.0],
            [p.x + 1.0, p.y + 1.0, p.z + 1.0],
        )
    }
}

/// The placements of one object group a demand source covers.
struct SourcePlacements {
    /// Index into `CellResources::object_groups`; its instances carry the
    /// world AABBs.
    group: usize,
    /// Start of the group's per-instance scales in `CellResources::placement_scales`.
    scales_start: usize,
}

/// One submesh (or terrain cell) and the textures it samples: everything a
/// demand pass needs, so a pass allocates nothing.
struct DemandSource {
    /// Range of `CellResources::texture_refs` this source acquired.
    textures: std::ops::Range<usize>,
    /// World units one UV unit spans on it, before placement scale.
    world_per_uv: f32,
    /// Union of the world AABBs of everything that uses the textures, and the
    /// largest placement scale: together an upper bound of the demand, cheap
    /// enough to test before walking the placements.
    bounds_min: Vec3,
    bounds_max: Vec3,
    max_scale: f32,
    /// `None`: one box (a terrain cell), `bounds_*` is its footprint.
    placements: Option<SourcePlacements>,
}

/// Largest scale a placement transform applies along any axis.
fn max_axis_scale(transform: &[[f32; 4]; 4]) -> f32 {
    let axis_length_squared = |c: usize| {
        transform[c][0] * transform[c][0]
            + transform[c][1] * transform[c][1]
            + transform[c][2] * transform[c][2]
    };
    axis_length_squared(0)
        .max(axis_length_squared(1))
        .max(axis_length_squared(2))
        .sqrt()
}

/// Demand a submesh places on its textures: the strongest over all its
/// placements (world AABB min, max and scale), each judged at the nearest point
/// of its AABB.
fn placements_demand(
    view: &DemandView,
    world_per_uv: f32,
    placements: impl Iterator<Item = ([f32; 3], [f32; 3], f32)>,
) -> f32 {
    placements
        .map(|(min, max, scale)| {
            view.pixels_per_uv(Vec3::from(min), Vec3::from(max), world_per_uv * scale)
        })
        .fold(NO_DEMAND, f32::max)
}

/// Everything a resident cell owns on the GPU side. Releasing these (and only
/// these) on unload is what keeps instance/mega/texture memory bounded.
#[derive(Default)]
struct CellResources {
    /// Unique per integration, so a lamp key never outlives its cell.
    residency: u64,
    /// Union of the cell's object and terrain AABBs: where its shadow casters
    /// are.
    caster_bounds: Option<Aabb>,
    /// One group per successful `mega.allocate()` call; releases balance the
    /// per-allocate refcount bumps.
    object_groups: Vec<CellObjectGroup>,
    /// Flattened slab offset of each `object_groups` entry (see
    /// `group_base_offsets`); fixed once the cell's groups are built.
    group_base: Vec<u32>,
    /// This cell's (offset, count) slab range in the object instance buffer:
    /// the `object_groups` instances flattened in group order. None for cells
    /// with no instances or whose slab allocation failed (the scheduled
    /// repack assigns those a range).
    instance_range: Option<(u32, u32)>,
    /// CPU mirror of the cell's placed lights, kept so a light-slab repack can
    /// rebuild the slab. Empty for LOD cells.
    lights: Vec<GpuPlacedLight>,
    /// This cell's (offset, count) slab range in the placed-light buffer. None
    /// for cells without lights or whose allocation failed (the scheduled
    /// light repack assigns those a range).
    light_range: Option<(u32, u32)>,
    /// The shadow-type lights among `lights`.
    shadow_lamps: Vec<ShadowLampSource>,
    /// The lights among `lights` whose record names a light model.
    light_models: Vec<LightModelRef>,
    terrain: Option<GpuTerrainInstance>,
    /// Lowest terrain vertex of a full-detail cell (engine units): the land
    /// height the cell water rule compares the water height against.
    land_lowest_y: Option<f32>,
    terrain_mega_key: Option<String>,
    splat_key: Option<String>,
    /// One entry per successful texture acquire (diffuse, normal, layers).
    texture_refs: Vec<TextureHandle>,
    /// Which of `texture_refs` each submesh or terrain cell uses and where;
    /// what every texture demand pass iterates.
    demand_sources: Vec<DemandSource>,
    /// Largest axis scale of every placement, in the order of
    /// `object_groups[..].instances`, so a demand pass does not recompute it.
    placement_scales: Vec<f32>,
    /// Marker-flagged instances this cell contributes; tallied at build time
    /// so the resident total updates incrementally instead of an
    /// O(all instances) recount per membership change.
    marker_count: u32,
    /// Hidden-FX-flagged instances this cell contributes; same incremental
    /// scheme as `marker_count`.
    fx_count: u32,
    /// The water-flagged instances among `object_groups`, for the water stage.
    water_surfaces: Vec<WaterRef>,
    /// Effect-shader placements, drawn by the effect stage; not in the object buffers.
    effect_instances: Vec<EffectInstance>,
    /// Heat-haze placements, drawn by the refraction stage; not in the object buffers.
    refraction_instances: Vec<RefractionInstance>,
    /// Addon nodes of the placed models, for the addon attachments.
    addon_attachments: Vec<StoredAttachment>,
    /// Placed models with particle systems of their own.
    particle_hosts: Vec<StoredParticleHost>,
    /// Per-draw-bucket potential this cell contributes: for each instance, the
    /// buckets it could be culled into across all toggle states (see
    /// [`potential_buckets`]). Tallied at build time and rolled into the
    /// streamer's resident total like `marker_count`, so the per-bucket indirect
    /// buffers can be sized to a hard upper bound instead of the whole pool.
    bucket_potential: [u32; DRAW_BUCKET_COUNT],
    /// Collision payload (batch + worker bake), held for the cell's whole
    /// full-detail residency so the collision ring can register/deregister
    /// it any number of times without rebuilding shapes. `None` for LOD
    /// cells and cells whose load produced no colliders.
    collider_payload: Option<(
        Arc<CellColliderBatch>,
        Option<crate::sim::bake::BakedCellColliders>,
    )>,
    /// Dynamic-clutter instance rows keyed by REFR entity id. Empty for cells
    /// with no Havok-dynamic placements. The per-frame pose readback writes
    /// through these into both the CPU mirror (`object_groups`) and the GPU
    /// slab, so a repack (which rebuilds the slab from the mirror) preserves the
    /// moved pose.
    dynamic_instances: HashMap<EntityId, Vec<DynRow>>,
    /// Entities handed to the animation runtime at integration, withdrawn when
    /// the cell retires.
    animated_entities: Vec<EntityId>,
    /// Entities handed to the ragdoll runtime at integration, withdrawn when
    /// the cell retires.
    ragdoll_entities: Vec<EntityId>,
    /// Whether the grass stage holds this cell's grass, under `residency`.
    has_grass: bool,
    /// The navigation meshes this cell added to the nav world, withdrawn when it
    /// retires.
    nav_meshes: CellNavmeshes,
}

/// Per-frame time budget for releasing cells that left the rings.
const RETIRE_BUDGET: std::time::Duration = std::time::Duration::from_millis(1);

/// Main-thread time a frame may spend on the texture demand sweep. The
/// resident cells' demand (about 8 ms of work in dense areas) is gathered over
/// the frames this allows, so the demand lags the camera by the sweep's length,
/// which the lookahead covers.
const DEMAND_SWEEP_BUDGET: Duration = Duration::from_micros(300);

/// A pass over the resident cells gathering texture demand: grids, and whether
/// each is a LOD cell.
struct DemandSweep {
    cells: Vec<((i32, i32), bool)>,
    next: usize,
}

/// Minimum spacing of demand recomputes forced only by cell releases.
const DEMAND_STALE_INTERVAL: Duration = Duration::from_millis(500);
/// Integrated-cell bounds remembered between bind group rebuilds before the
/// shadow invalidation degrades to "everything".
const MAX_TRACKED_INTEGRATIONS: usize = 1024;
/// DDS headers kept for texture preloads; missing files cache as weightless
/// negatives and are never evicted.
const LAYOUT_CACHE_CAP: u64 = 65_536;

/// Per-frame cap on bytes pushed through queue.write_* during cell
/// integration (texture uploads + fresh mega-buffer geometry). ~8 MB of
/// staging memcpy is ~1-2 ms of main-thread time, while still guaranteeing
/// ≥480 MB/s of integration throughput at 60 fps. Splat layers (64 KB/cell)
/// are immaterial and deliberately uncounted, as is the rare sync-fallback
/// texture load (size unknown before the read).
pub const UPLOAD_BUDGET_BYTES: u64 = 8 * 1024 * 1024;

const DEFER_ADMIT_ENV: &str = "OPENFO4_ADMIT_DEFER";

/// `OPENFO4_ADMIT_DEFER=0` restores the old behaviour of uploading every
/// payload of a cell during its integration, however large.
fn defer_admit_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| crate::utils::env_flag(DEFER_ADMIT_ENV, true))
}

/// Tracks integration upload bytes for one frame. Cells are integration-
/// atomic: the budget is checked between results, never inside one, and at
/// least one buffered result lands per frame so progress is guaranteed (a
/// single oversized cell overshoots its frame and defers the rest).
pub struct UploadBudget {
    used: u64,
    limit: u64,
}

impl UploadBudget {
    pub fn new(limit: u64) -> Self {
        Self { used: 0, limit }
    }

    fn add(&mut self, bytes: u64) {
        self.used += bytes;
    }

    fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.used)
    }

    fn exhausted(&self) -> bool {
        self.used >= self.limit
    }
}

/// Mutable references to every GPU-side resource the streamer feeds. Bundled
/// so integration/release helpers don't need ten parameters each.
pub struct StreamTargets<'a> {
    pub object_instances: &'a mut InstanceBuffer,
    pub placed_lights: &'a mut GpuSlab<GpuPlacedLight>,
    pub terrain_instances: &'a mut TerrainInstanceBuffer,
    pub object_mega: &'a mut MegaBuffer,
    pub terrain_mega: &'a mut MegaBuffer,
    pub texture_array: &'a mut TextureArray,
    pub residency: &'a mut TextureResidency,
    pub splatmaps: &'a mut SplatmapArray,
    pub material_registry: &'a mut MaterialRegistry,
    pub cube_array: &'a mut CubeArray,
    pub grass: &'a mut GrassStage,
    pub asset_server: &'a mut AssetServer,
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    /// Fresh each frame (see `UPLOAD_BUDGET_BYTES`).
    pub upload_budget: UploadBudget,
}

pub struct StreamUpdate {
    /// Bindless texture slot table changes since the previous update; the
    /// material bind group is stale when any list is non-empty.
    pub slot_changes: SlotChanges,
    /// Collider add/remove commands for the sim, emitted at the same
    /// lifecycle points where the cell's GPU residency changed.
    pub sim_commands: Vec<SimCommand>,
    /// Set exactly once, on the frame the initial spawn finishes integrating.
    pub spawn_complete: Option<SpawnCompleteReport>,
    /// True on exactly one frame per load: the frame the full-detail ring (the
    /// ring that carries the colliders) finishes integrating. The LOD ring
    /// usually still loads after it.
    pub full_ring_complete: bool,
}

/// Timing summary for the initial spawn. The rings load concurrently with
/// full-detail prioritized, so `full`/`lod` are each ring's time-to-finish
/// measured from `initial_load`, not disjoint shares of `total`.
pub struct SpawnCompleteReport {
    pub total: Duration,
    /// Full-detail ring finish time and integrated cell count.
    pub full: Duration,
    pub full_cells: usize,
    /// LOD ring finish time and integrated cell count.
    pub lod: Duration,
    pub lod_cells: usize,
}

/// Rolling integration-rate tracker for the debug UI: cells/s averaged over
/// the last half second of integrations. Reads 0 within WINDOW of the last
/// integration.
struct SpawnRate {
    times: VecDeque<std::time::Instant>,
}

impl SpawnRate {
    /// Averaging window.
    const WINDOW: Duration = Duration::from_millis(500);

    fn new() -> Self {
        Self {
            times: VecDeque::new(),
        }
    }

    fn prune(&mut self, now: std::time::Instant) {
        while self.times.front().is_some_and(|t| now - *t > Self::WINDOW) {
            self.times.pop_front();
        }
    }

    fn record(&mut self) {
        let now = std::time::Instant::now();
        // Pruning here too keeps the deque bounded even while the debug
        // menu (the only reader) is closed.
        self.prune(now);
        self.times.push_back(now);
    }

    fn cells_per_sec(&mut self) -> f32 {
        self.prune(std::time::Instant::now());
        self.times.len() as f32 / Self::WINDOW.as_secs_f32()
    }
}

pub struct WorldStreamer {
    db_worker_tx: crossbeam_channel::Sender<CellRequest>,
    /// A consumer handle on the request channel, kept so `switch_worldspace`
    /// can purge the superseded backlog in one shot (the workers hold their own
    /// clones — crossbeam's channel is multi-consumer).
    req_rx: crossbeam_channel::Receiver<CellRequest>,
    db_worker_rx: mpsc::Receiver<WorkerResponse>,
    current_cell: (i32, i32),
    starting_cell: (i32, i32),
    pending_cells: HashSet<(i32, i32)>,
    /// Full-ring cells whose request could not be sent. The ring-delta fast
    /// path only requests cells that newly entered the ring, so these are
    /// retried on the next crossing while they remain in the ring.
    missing_full: HashSet<(i32, i32)>,
    loaded: HashMap<(i32, i32), CellResources>,
    loaded_generation: u64,
    pub worldspace: String,
    /// Set while a single interior cell is loaded in place of a worldspace
    /// (debug interior view): holds that cell's `form_key`. Freezes the
    /// camera-driven ring in `update_gpu_driven`; cleared by `switch_worldspace`.
    interior_cell: Option<String>,
    lod_pending_cells: HashSet<(i32, i32)>,
    /// LOD counterpart of `missing_full`.
    missing_lod: HashSet<(i32, i32)>,
    lod_loaded: HashMap<(i32, i32), CellResources>,
    stream_radius: i32,
    lod_radius: i32,
    max_cells_per_frame: usize,
    // Deques, not Vecs: in loading mode the pool outruns integration by
    // thousands of buffered cells, and CellLoadResult is ~64 KB by value
    // (inline splatmap) — Vec::remove(0) made every integrate memmove the
    // whole backlog (measured 300 ms/frame at peak).
    buffered_full: VecDeque<CellLoadResult>,
    buffered_lod: VecDeque<CellLoadResult>,
    /// Cells that left the rings but still hold their slab ranges and refs;
    /// released within a per-frame time budget so a boundary crossing doesn't
    /// free ~100 cells in one frame. They keep drawing until freed (the cells
    /// are beyond the ring edge, so a few frames are invisible).
    retire_queue: VecDeque<CellResources>,
    /// Resident marker-flagged instances; recounted on membership changes.
    marker_instance_count: u32,
    /// Resident hidden-FX-flagged instances; same scheme as `marker_instance_count`.
    fx_instance_count: u32,
    /// Resident per-draw-bucket potential: the sum of every resident cell's
    /// `bucket_potential`, maintained incrementally as cells load and retire.
    /// A hard upper bound on how many draws the GPU cull can emit into each
    /// bucket this frame — the per-bucket indirect buffers are sized from it.
    bucket_potential_total: [u32; DRAW_BUCKET_COUNT],
    /// Collider commands accumulated during integrates/unloads, drained into
    /// the `StreamUpdate` each frame.
    sim_commands: Vec<SimCommand>,
    /// Navigation meshes of the resident full-detail and interior cells. Owned here because the
    /// streamer decides residency; consumers read it through `nav_world`.
    nav: NavResidency,
    /// Grids whose colliders are currently registered with the sim — the
    /// collision ring (`max(collision_radius, physics_radius)`), a subset of
    /// `loaded`.
    collision_registered: HashSet<(i32, i32)>,
    /// Cells around the player that register static colliders: vanilla's loaded
    /// block (`uGridsToLoad`), where Havok is active. Render residency stays at
    /// `STREAM_RADIUS`; each full cell keeps its collider payload in
    /// `CellResources` and registers/deregisters as the ring moves over it
    /// (`sync_collision_ring`). Cost tracks resident collider count (every
    /// broad-phase fold re-optimizes the BVH and scans all stored pairs): radius 8
    /// was ~150k colliders at 40-80 ms of main-thread time per streaming frame,
    /// radius 2 is ~13k.
    collision_radius: i32,
    /// Debug-tunable dynamic-physics simulation radius (Chebyshev cells). Drives
    /// dynamic-body activation and floors the collision ring (so bodies always
    /// have static ground). Defaults to `collision_radius`; bigger costs more
    /// (more colliders folded + stepped while dynamics are awake).
    physics_radius: i32,
    /// Dynamic REFR entity id -> its owning resident cell, so the per-frame pose
    /// readback finds the cell's `dynamic_instances` in O(1). Maintained beside
    /// the `loaded` map (integrate adds, retire removes).
    entity_cell: HashMap<EntityId, (i32, i32)>,
    /// Animated objects entering or leaving residency, drained each frame by
    /// the animation runtime.
    animated_events: Vec<AnimatedObjectEvent>,
    ragdoll_events: Vec<RagdollEvent>,
    /// `(center, radius)` of the last `SetPhysicsCenter` pushed. The command is
    /// re-pushed only when the center/radius changes or the collision ring gains
    /// or loses a cell — so a stationary player's idle frames push nothing,
    /// keeping `sim_commands` empty and skipping the sim's per-frame reconcile
    /// scan. Sentinel forces a push on the first sync.
    last_physics_center: ((i32, i32), i32),
    /// `(current cell, ring)` the collision ring was last reconciled for.
    collision_sync_key: Option<((i32, i32), i32)>,
    /// `loaded` membership (or a cell's light slab range) changed since the
    /// collision ring was last reconciled.
    collision_ring_dirty: bool,
    /// Same trigger for the cached lamp-shadow candidate list.
    lamp_candidates_dirty: bool,
    /// Reused scratch for `apply_dynamic_poses`: per-cell slab writes.
    pose_writes: Vec<(u32, GpuInstance)>,
    /// Scratch for grouping this frame's poses by cell (cell, index into poses).
    pose_order: Vec<((i32, i32), usize)>,
    pose_run: Vec<GpuInstance>,
    /// A cell's slab allocation failed (instance buffer at the device cap):
    /// repack at the end of the update to recover it.
    needs_repack: bool,
    /// Caster changes for the lamp-shadow cache, drained once per frame.
    shadow_invalidations: ShadowInvalidations,
    /// Caster bounds of cells integrated since the material bind group was
    /// last rebuilt: their shadows may have sampled a recycled texture slot's
    /// previous texture (see `on_materials_rebound`).
    integrated_since_rebind: Vec<Aabb>,
    /// `integrated_since_rebind` hit its cap; invalidate everything instead.
    rebind_overflow: bool,
    /// Texture demand is measured from here; no camera inside an interior,
    /// where every texture is treated as needed at full resolution.
    demand_view: DemandView,
    /// Size and field of view of the screen the textures are shown on.
    screen: ScreenView,
    /// `OPENFO4_TEX_TRACE`: lowercase path substring of the textures to trace.
    tex_trace: Option<String>,
    last_tex_trace: std::time::Instant,
    motion: CameraMotion,
    /// View the last demand sweep started from (`None`: no sweep yet).
    last_demand_view: Option<DemandView>,
    /// The demand sweep in progress.
    sweep: Option<DemandSweep>,
    last_demand_pass: std::time::Instant,
    /// A cell was released since the last full demand pass, so some textures'
    /// recorded demand may be higher than their remaining users need.
    demand_stale: bool,
    /// Source of `CellResources::residency`.
    next_residency: u64,
    /// Same as `needs_repack` for the placed-light slab.
    needs_light_repack: bool,
    /// Set by `initial_load`; once the initial full+LOD rings finish
    /// integrating, a `SPAWN_COMPLETE` line is logged and this clears.
    spawn_timer: Option<std::time::Instant>,
    /// Per-ring finish times relative to `spawn_timer`, recorded the first
    /// frame each ring's pending+buffered sets drain during the initial
    /// spawn. Feed the `SpawnCompleteReport` breakdown.
    spawn_full_done: Option<Duration>,
    spawn_lod_done: Option<Duration>,
    /// Armed with the spawn timer; fires once when the full ring drains.
    full_ring_signal: OneShot,
    /// Last ~500 ms bucket a `SPAWN_PROGRESS` line was logged for.
    spawn_progress_tick: u64,
    /// Worst frame gap in `LoadMode::Loading`, reported at `SPAWN_COMPLETE`.
    load_frames: LoadFrameMeter,
    /// See [`LoadMode`]. Drives the integrate/upload budgets here and, via
    /// `active_workers`, how many pool threads pull requests.
    mode: LoadMode,
    /// Env var / debug-menu choices stick; only the unpinned boot default
    /// auto-drops to Streaming at `SPAWN_COMPLETE`.
    mode_pinned: bool,
    /// Threads spawned in the worker pool (all of them, always; `mode` only
    /// gates how many actively pull).
    pool_size: usize,
    /// Mode gate shared with the workers: worker `i` pulls requests only
    /// while `i < active_workers`.
    active_workers: Arc<AtomicUsize>,
    /// Debug-UI integration rates, one per ring.
    full_spawn_rate: SpawnRate,
    lod_spawn_rate: SpawnRate,
    /// Worldspace generation, bumped by [`switch_worldspace`]. Stamped onto
    /// every request and echoed back on every result; the main thread drops
    /// results whose epoch differs and `epoch_arc` lets workers skip queued
    /// requests of a superseded worldspace at pull time.
    epoch: u64,
    /// Shared mirror of `epoch` the worker pool reads (the workers only ever
    /// compare, never write).
    epoch_arc: Arc<AtomicU64>,
    /// Read-only DB path, kept so a worldspace switch can resolve the new
    /// start cell and spawn its persistent prefetch without the streamer
    /// holding a live `Connection` (which is not `Sync`).
    db_path: String,
    /// The pool's shared lookups/caches, retained so a switch can launch the
    /// new worldspace's persistent-prefetch thread exactly as `new` does.
    worker_shared: Arc<WorkerShared>,
}

impl WorldStreamer {
    /// `resident_textures` is `TextureArray`'s path mirror (see
    /// `TextureArray::resident_paths`): the worker skips pre-reading DDS
    /// files that are already GPU-resident.
    pub fn new(
        db_path: &str,
        asset_root: &str,
        worldspace: &str,
        resident_textures: Arc<Mutex<HashSet<String>>>,
        texture_policy: ResidencyPolicy,
        loaded_grid: LoadedGrid,
        grass_settings: GrassSettings,
    ) -> anyhow::Result<Self> {
        let db = DbClient::new(db_path)?;

        let lights = load_lights(&db);
        let (base_object_lookup, weapon_mods) = build_base_object_lookup(&db, &lights);
        let light_lookup = build_light_lookup(&lights, db.schema().light_columns_present);
        let light_models = build_light_models(&lights);
        let addon_nodes = db.get_addon_nodes()?;

        // Material swaps (FO4 MSWP): base-record swaps (MODS) + the
        // pre-resolved substitution entries. A ref-level swap (XMSP) comes per
        // row from placed_objects. Tolerate their absence so an engine built
        // after the swap work still runs against an older database.
        let base_material_swaps = db.get_base_material_swaps().unwrap_or_else(|e| {
            warn!("base material swaps unavailable (pre-swap DB?): {e}");
            HashMap::new()
        });
        let swap_entries = db.get_material_swap_entries().unwrap_or_else(|e| {
            warn!("material swap entries unavailable (pre-swap DB?): {e}");
            HashMap::new()
        });
        info!(
            "Material swaps: {} swap-bearing bases, {} swaps with {} entries",
            base_material_swaps.len(),
            swap_entries.len(),
            swap_entries.values().map(|v| v.len()).sum::<usize>()
        );

        let ltex_records = db.get_landscape_textures()?;
        let mut ltex_to_diffuse: HashMap<String, String> = HashMap::new();
        let mut ltex_missing = 0usize;
        for r in &ltex_records {
            if let Some(diffuse) = &r.texture_set_diffuse {
                ltex_to_diffuse.insert(r.form_key.clone(), diffuse.clone());
            } else {
                ltex_missing += 1;
                warn!(
                    "LTEX {} ({}) has no diffuse path; terrain layers using it render fallback",
                    r.form_key,
                    r.editor_id.as_deref().unwrap_or("?")
                );
            }
        }
        if ltex_missing > 0 {
            warn!(
                "{}/{} landscape textures missing a diffuse path",
                ltex_missing,
                ltex_records.len()
            );
        }
        let ltex_to_diffuse = Arc::new(ltex_to_diffuse);
        let grass = GrassWorkerData::load(&db, asset_root, grass_settings);

        let starting_cell = Self::resolve_start_cell(&db, worldspace);

        // Loading mode wants everything the machine has; the -1 reserves the
        // main thread, which must keep draining the response channel and
        // doing GPU-upload integration — if it starves, the whole pipeline
        // stalls behind it. Workers' heavy inner steps (GLB parse, DDS read,
        // shape bake) fan out across the global rayon pool while the worker
        // blocks, so worker-vs-rayon oversubscription is harmless.
        let default_pool_size = thread::available_parallelism()
            .map_or(4, std::num::NonZero::get)
            .saturating_sub(1)
            .max(2);
        let pool_size =
            crate::utils::env_or_valid("OPENFO4_LOAD_WORKERS", default_pool_size, |&n| n >= 1);

        let (mode, mode_pinned) = match std::env::var("OPENFO4_LOAD_MODE") {
            Ok(v) => match v.to_ascii_lowercase().as_str() {
                "streaming" => (LoadMode::Streaming, true),
                "loading" => (LoadMode::Loading, true),
                other => {
                    warn!(
                        "OPENFO4_LOAD_MODE='{other}' not recognized \
                         (streaming|loading); using the loading default"
                    );
                    (LoadMode::Loading, false)
                }
            },
            Err(_) => (LoadMode::Loading, false),
        };
        let active_workers = Arc::new(AtomicUsize::new(match mode {
            LoadMode::Streaming => 1,
            LoadMode::Loading => pool_size,
        }));
        let epoch_arc = Arc::new(AtomicU64::new(0));
        info!(
            "Cell worker pool: {} threads, mode {} ({} active)",
            pool_size,
            mode.label(),
            active_workers.load(Ordering::Relaxed)
        );

        // Requests: one shared multi-consumer queue the active workers pull
        // from. Responses: bounded — cell payloads carry parsed meshes
        // (multi-MB); workers block instead of piling them up faster than
        // the per-frame budget.
        let (tx_req, rx_req) = crossbeam_channel::unbounded::<CellRequest>();
        let (tx_res, rx_res) = mpsc::sync_channel::<WorkerResponse>(pool_size * 2);

        let shared = Arc::new(WorkerShared {
            base_object_lookup,
            weapons: WeaponAssemblies::new(asset_root, weapon_mods),
            light_lookup,
            light_models,
            addon_nodes,
            base_material_swaps,
            swap_entries,
            ltex_to_diffuse,
            grass,
            grass_models: GrassModelCache::new(asset_root),
            asset_root: asset_root.to_string(),
            collision_manifest: load_manifest(&format!(
                "{asset_root}/{MESHES_DIR}/collision_manifest.json"
            )),
            resident_textures,
            texture_policy,
            demand_view: Mutex::new(DemandView::default()),
            mesh_cache: SharedCache::new(MESH_CACHE_CAP),
            collider_cache: SharedCache::new(COLLIDER_CACHE_CAP),
            layout_cache: SharedCache::new(LAYOUT_CACHE_CAP),
            dds_cache: SharedCache::new(DDS_CACHE_MAX_BYTES),
            bake_cache: crate::sim::bake::ShapeBakeCache::new(crate::sim::bake::physics_cache_dir(
                asset_root,
            )),
            ws_editor_ids: Mutex::new(HashMap::new()),
            tile_exists: Mutex::new(HashMap::new()),
            persistent_by_grid: Mutex::new(HashMap::new()),
            active_workers: active_workers.clone(),
            epoch: epoch_arc.clone(),
        });

        for index in 0..pool_size {
            let rx = rx_req.clone();
            let tx = tx_res.clone();
            let shared = shared.clone();
            // Each worker gets its own read-only connection (`Connection` is
            // Send, not Sync); the path stays encapsulated in DbClient.
            let worker_db = db.try_clone()?;
            thread::Builder::new()
                .name(format!("db_worker_{index}"))
                .spawn(move || db_worker(index, rx, tx, worker_db, shared))?;
        }
        drop(tx_res);

        // Prefetch the worldspace persistent REFRs now, before the workers
        // see their first requests: otherwise the whole pool stacks up at
        // t=0 behind whichever worker runs the one-shot ~33k-row query (see
        // worldspace_persistents).
        Self::spawn_persistent_prefetch(db_path, &shared, worldspace);

        Ok(Self {
            db_worker_tx: tx_req,
            req_rx: rx_req,
            db_worker_rx: rx_res,
            current_cell: starting_cell,
            starting_cell,
            pending_cells: HashSet::new(),
            missing_full: HashSet::new(),
            loaded: HashMap::new(),
            loaded_generation: 0,
            worldspace: worldspace.to_string(),
            interior_cell: None,
            lod_pending_cells: HashSet::new(),
            missing_lod: HashSet::new(),
            lod_loaded: HashMap::new(),
            stream_radius: STREAM_RADIUS,
            lod_radius: LOD_RADIUS,
            max_cells_per_frame: crate::utils::env_or("OPENFO4_CELLS_PER_FRAME", 1),
            buffered_full: VecDeque::new(),
            buffered_lod: VecDeque::new(),
            retire_queue: VecDeque::new(),
            marker_instance_count: 0,
            fx_instance_count: 0,
            bucket_potential_total: [0; DRAW_BUCKET_COUNT],
            sim_commands: Vec::new(),
            nav: NavResidency::new(),
            collision_registered: HashSet::new(),
            collision_radius: loaded_grid.radius(),
            physics_radius: loaded_grid.radius(),
            entity_cell: HashMap::new(),
            animated_events: Vec::new(),
            ragdoll_events: Vec::new(),
            last_physics_center: ((i32::MIN, i32::MIN), i32::MIN),
            collision_sync_key: None,
            collision_ring_dirty: true,
            lamp_candidates_dirty: true,
            pose_writes: Vec::new(),
            pose_order: Vec::new(),
            pose_run: Vec::new(),
            needs_repack: false,
            shadow_invalidations: ShadowInvalidations::default(),
            integrated_since_rebind: Vec::new(),
            rebind_overflow: false,
            demand_view: DemandView::default(),
            screen: ScreenView::default(),
            tex_trace: std::env::var(TEX_TRACE_ENV)
                .ok()
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty()),
            last_tex_trace: std::time::Instant::now(),
            motion: CameraMotion::default(),
            last_demand_view: None,
            sweep: None,
            last_demand_pass: std::time::Instant::now(),
            demand_stale: false,
            next_residency: 0,
            needs_light_repack: false,
            spawn_timer: None,
            spawn_full_done: None,
            full_ring_signal: OneShot::default(),
            spawn_lod_done: None,
            spawn_progress_tick: 0,
            load_frames: LoadFrameMeter::new(),
            mode,
            mode_pinned,
            pool_size,
            active_workers,
            full_spawn_rate: SpawnRate::new(),
            lod_spawn_rate: SpawnRate::new(),
            epoch: 0,
            epoch_arc,
            db_path: db_path.to_string(),
            worker_shared: shared,
        })
    }

    fn desired_cells(&self) -> Vec<(i32, i32)> {
        self.rings().full_cells()
    }

    fn desired_lod_cells(&self) -> Vec<(i32, i32)> {
        self.rings().lod_cells()
    }

    /// Full-ring cells the move from `previous` brought into the full ring.
    /// Cells inside both rings were already requested (or are resident), so
    /// the per-crossing work is the ring delta, not the whole ring.
    fn full_cells_entering(&self, previous: &Rings) -> Vec<(i32, i32)> {
        self.rings().full_cells_entering(previous)
    }

    /// LOD-ring cells the move from `previous` brought into the LOD ring,
    /// including cells that were full-detail before and now sit in the LOD
    /// annulus. Walks the square with arithmetic only; no allocation of the
    /// whole ring.
    fn lod_cells_entering(&self, previous: &Rings) -> Vec<(i32, i32)> {
        self.rings().lod_cells_entering(previous)
    }

    fn rings(&self) -> Rings {
        Rings {
            center: self.current_cell,
            full_radius: self.stream_radius,
            outer_radius: self.stream_radius + self.lod_radius,
        }
    }

    fn in_full_ring(&self, grid: (i32, i32)) -> bool {
        self.rings().in_full(grid)
    }

    fn in_lod_ring(&self, grid: (i32, i32)) -> bool {
        self.rings().in_lod(grid)
    }

    /// Nearest-first (Chebyshev ring, then Euclidean) order around the current
    /// cell, so the cells under and next to the player are decoded first.
    fn nearest_first(&self, grids: &[(i32, i32)]) -> Vec<(i32, i32)> {
        self.rings().nearest_first(grids)
    }

    fn request_cells(&mut self, grids: &[(i32, i32)]) {
        let _span = info_span!("request_cells", count = grids.len()).entered();
        for (gx, gy) in self.nearest_first(grids) {
            if self.pending_cells.contains(&(gx, gy)) {
                continue;
            }
            self.pending_cells.insert((gx, gy));
            let sent = self.db_worker_tx.send(CellRequest::Full {
                gx,
                gy,
                worldspace: self.worldspace.clone(),
                epoch: self.epoch,
            });
            if let Err(e) = sent {
                warn!(
                    "full cell ({gx}, {gy}) request not sent, will retry on the next crossing: {e}"
                );
                self.pending_cells.remove(&(gx, gy));
                self.missing_full.insert((gx, gy));
            }
        }
    }

    fn request_lod_cells(&mut self, grids: &[(i32, i32)]) {
        for (gx, gy) in self.nearest_first(grids) {
            if self.lod_pending_cells.contains(&(gx, gy)) {
                continue;
            }
            self.lod_pending_cells.insert((gx, gy));
            let sent = self.db_worker_tx.send(CellRequest::Lod {
                gx,
                gy,
                worldspace: self.worldspace.clone(),
                epoch: self.epoch,
            });
            if let Err(e) = sent {
                warn!(
                    "LOD cell ({gx}, {gy}) request not sent, will retry on the next crossing: {e}"
                );
                self.lod_pending_cells.remove(&(gx, gy));
                self.missing_lod.insert((gx, gy));
            }
        }
    }

    #[tracing::instrument(skip_all)]
    pub(crate) fn update_gpu_driven(
        &mut self,
        camera_eye: Vec3,
        targets: &mut StreamTargets,
    ) -> StreamUpdate {
        let mut membership_changed = false;
        let frame_started = Instant::now();
        self.load_frames.observe(frame_started, self.mode);
        self.motion.observe(frame_started, camera_eye);
        self.set_demand_camera(self.interior_cell.is_none().then_some(camera_eye));
        // Streaming: default 1 cell per frame (debug-UI tunable): an
        // integrate frame's true cost is ~3x its update_gpu_driven span —
        // the staged bytes flush and the rebuilt material bind group
        // re-tracks inside the SAME frame's queue.submit (measured
        // 2026-06-11: integrate frames cost 5 ms p50 / 16 ms p90 end-to-end
        // at 4 cells/frame, ~10% of frames in dense areas). Worker
        // throughput (~60 cells/s) is far below 1 cell x framerate, so fill
        // latency doesn't suffer. The upload-budget check below only binds
        // when the budget is above 1 — which Loading mode forces (see
        // LOADING_CELLS_PER_FRAME).
        let max_cells_per_frame = self.effective_cells_per_frame();

        self.drain_worker_responses();

        // Integrate buffered results one at a time — full cells (player-
        // facing) before LOD — stopping once the cell count OR the upload
        // byte budget is spent. The first successful integration is exempt
        // so a single oversized cell can't stall streaming forever; gated/
        // stale drops (integrate returns false) cost nothing and don't
        // consume that guaranteed slot. The channel re-drains between
        // integrations: a fat loading-mode frame that only drained at frame
        // start left the bounded response channel full for its whole
        // duration and stalled every worker mid-spawn.
        let integrate_span = info_span!(
            "integrate_cells",
            mode = self.mode.label(),
            integrated = tracing::field::Empty
        )
        .entered();
        let mut integrated = 0usize;
        let integrate_started = Instant::now();
        loop {
            if integration_must_stop(
                self.mode,
                &IntegrationProgress {
                    integrated,
                    cell_cap: max_cells_per_frame,
                    bytes_exhausted: targets.upload_budget.exhausted(),
                    elapsed: integrate_started.elapsed(),
                },
            ) {
                break;
            }
            let Some(mut result) = self.buffered_full.pop_front() else {
                break;
            };
            let changed = self.integrate_full(&mut result, targets);
            crate::utils::drop_off_thread(result);
            if changed {
                membership_changed = true;
                integrated += 1;
                self.full_spawn_rate.record();
            }
            self.drain_worker_responses();
        }
        loop {
            if integration_must_stop(
                self.mode,
                &IntegrationProgress {
                    integrated,
                    cell_cap: max_cells_per_frame,
                    bytes_exhausted: targets.upload_budget.exhausted(),
                    elapsed: integrate_started.elapsed(),
                },
            ) {
                break;
            }
            let Some(mut result) = self.buffered_lod.pop_front() else {
                break;
            };
            let changed = self.integrate_lod(&mut result, targets);
            crate::utils::drop_off_thread(result);
            if changed {
                membership_changed = true;
                integrated += 1;
                self.lod_spawn_rate.record();
            }
            self.drain_worker_responses();
        }

        integrate_span.record("integrated", integrated);
        drop(integrate_span);

        // Initial-spawn benchmark hook: both rings fully integrated.
        let mut spawn_complete = None;
        let mut full_ring_complete = false;
        if let Some(start) = self.spawn_timer {
            if self.spawn_full_done.is_none()
                && self.pending_cells.is_empty()
                && self.buffered_full.is_empty()
            {
                self.spawn_full_done = Some(start.elapsed());
            }
            let full_ring_drained = self.pending_cells.is_empty()
                && self.buffered_full.is_empty()
                && !self.loaded.is_empty();
            full_ring_complete = self.full_ring_signal.fire(full_ring_drained);
            if full_ring_complete {
                info!(
                    "FULL_RING_COMPLETE elapsed_ms={} full_cells={} lod_loaded={}/{}",
                    start.elapsed().as_millis(),
                    self.loaded.len(),
                    self.lod_loaded.len(),
                    self.lod_loaded.len() + self.lod_pending_cells.len(),
                );
            }
            if self.spawn_lod_done.is_none()
                && self.lod_pending_cells.is_empty()
                && self.buffered_lod.is_empty()
            {
                self.spawn_lod_done = Some(start.elapsed());
            }
            // Ramp-up visibility: one INTEGRATED-side progress line per
            // ~500 ms while the initial spawn runs (worker-side progress is
            // already visible as db_query_cell span closes).
            let tick = start.elapsed().as_millis() as u64 / 500;
            if tick != self.spawn_progress_tick {
                self.spawn_progress_tick = tick;
                info!(
                    "SPAWN_PROGRESS t_ms={} full={}/{} lod={}/{} buffered={}",
                    start.elapsed().as_millis(),
                    self.loaded.len(),
                    self.loaded.len() + self.pending_cells.len(),
                    self.lod_loaded.len(),
                    self.lod_loaded.len() + self.lod_pending_cells.len(),
                    self.buffered_full.len() + self.buffered_lod.len(),
                );
            }
            if self.pending_cells.is_empty()
                && self.lod_pending_cells.is_empty()
                && self.buffered_full.is_empty()
                && self.buffered_lod.is_empty()
                && !self.loaded.is_empty()
            {
                info!(
                    "SPAWN_COMPLETE elapsed_ms={} full_cells={} lod_cells={} cells_per_frame={} mode={} workers={} worst_frame_ms={:.1}",
                    start.elapsed().as_millis(),
                    self.loaded.len(),
                    self.lod_loaded.len(),
                    max_cells_per_frame,
                    self.mode.label(),
                    self.active_workers.load(Ordering::Relaxed),
                    self.load_frames.worst_ms(),
                );
                let total = start.elapsed();
                spawn_complete = Some(SpawnCompleteReport {
                    total,
                    full: self.spawn_full_done.unwrap_or(total),
                    full_cells: self.loaded.len(),
                    lod: self.spawn_lod_done.unwrap_or(total),
                    lod_cells: self.lod_loaded.len(),
                });
                self.spawn_timer = None;
                self.flush_sidecar_hash_index_in_background();
                // Initial spawn done: hand the frame budget back to
                // gameplay. Env-var/debug-menu choices are pinned and stick.
                if !self.mode_pinned && self.mode == LoadMode::Loading {
                    self.apply_mode(LoadMode::Streaming);
                    info!("initial spawn integrated: load mode -> streaming");
                }
            }
        }

        // Handle cell movement — frozen in interior mode: the single interior
        // cell stays resident and the free camera must never drive streaming.
        let new_cell = WorldStreamer::camera_to_grid(camera_eye);
        if self.interior_cell.is_none() && new_cell != self.current_cell {
            let _span = info_span!("cell_change").entered();
            let old_cell = self.current_cell;
            self.current_cell = new_cell;
            info!(
                "Cell changed: ({}, {}) -> ({}, {})",
                old_cell.0, old_cell.1, new_cell.0, new_cell.1
            );

            // The rings are squares around the current cell: membership is
            // arithmetic, no per-change set building.
            let rings = self.rings();
            let previous_rings = Rings {
                center: old_cell,
                ..rings
            };

            // Unload full-detail cells that left both rings. A full cell that
            // merely dropped into the LOD ring stays resident until its LOD
            // replacement arrives (pop-free demotion in integrate_lod).
            let to_unload_full: Vec<(i32, i32)> = self
                .loaded
                .keys()
                .filter(|c| !rings.in_any(**c))
                .copied()
                .collect();
            // Collider deregistration is sync_collision_ring's job (it sees
            // the grid leave `loaded`), so unloads don't push commands here.
            {
                let _span = info_span!(
                    "cell_change_queue_retire_full",
                    cells = to_unload_full.len()
                )
                .entered();
                for grid in to_unload_full {
                    if let Some(res) = self.loaded.remove(&grid) {
                        self.mark_loaded_changed();
                        self.defer_retire(res);
                        membership_changed = true;
                    }
                }
            }

            // Unload LOD cells that left both rings.
            let to_unload_lod: Vec<(i32, i32)> = self
                .lod_loaded
                .keys()
                .filter(|c| !rings.in_any(**c))
                .copied()
                .collect();
            {
                let _span = info_span!("cell_change_queue_retire_lod", cells = to_unload_lod.len())
                    .entered();
                for grid in to_unload_lod {
                    if let Some(res) = self.lod_loaded.remove(&grid) {
                        self.defer_retire(res);
                        membership_changed = true;
                    }
                }
            }

            // Prune stale pending markers and buffered payloads. In-flight
            // responses whose markers were pruned are dropped by the gates in
            // integrate_full/integrate_lod.
            let prune_span = info_span!("cell_change_prune").entered();
            self.pending_cells.retain(|cell| rings.in_full(*cell));
            self.lod_pending_cells.retain(|cell| rings.in_lod(*cell));
            retain_deferred_drop(&mut self.buffered_full, |r| rings.in_full(r.grid));
            retain_deferred_drop(&mut self.buffered_lod, |r| rings.in_lod(r.grid));
            drop(prune_span);

            // Cells whose earlier request never went out are still wanted when
            // they remain in the ring; fold them into the delta.
            self.missing_full.retain(|cell| rings.in_full(*cell));
            self.missing_lod.retain(|cell| rings.in_lod(*cell));
            let retry_full: Vec<(i32, i32)> = self.missing_full.drain().collect();
            let retry_lod: Vec<(i32, i32)> = self.missing_lod.drain().collect();

            let enter_span = info_span!("cell_change_enumerate_and_request").entered();
            let to_load_full: Vec<(i32, i32)> = self
                .full_cells_entering(&previous_rings)
                .into_iter()
                .chain(retry_full)
                .filter(|c| !self.loaded.contains_key(c) && !self.pending_cells.contains(c))
                .collect();
            if !to_load_full.is_empty() {
                info!("Requesting {} full cells", to_load_full.len());
                self.request_cells(&to_load_full);
            }

            let to_load_lod: Vec<(i32, i32)> = self
                .lod_cells_entering(&previous_rings)
                .into_iter()
                .chain(retry_lod)
                .filter(|c| !self.lod_loaded.contains_key(c) && !self.lod_pending_cells.contains(c))
                .collect();
            if !to_load_lod.is_empty() {
                info!("Requesting {} LOD cells", to_load_lod.len());
                self.request_lod_cells(&to_load_lod);
            }
            drop(enter_span);
        }

        {
            let _span =
                info_span!("drain_retire_queue", queued = self.retire_queue.len()).entered();
            self.drain_retire_queue(targets, RETIRE_BUDGET);
        }

        // Reconcile the collision ring every frame: integrates may have
        // produced payloads inside it, and cell movement/unloads may have
        // pulled cells out. O(ring area) hashmap probes on clean frames.
        self.sync_collision_ring();

        // A mega-buffer compaction during the integrates above moved slots:
        // re-patch the offsets baked into the per-cell CPU instance copies.
        // The object side then needs every cell's slab range re-uploaded —
        // exactly what a repack does, so it doubles as a free defrag. The
        // terrain side is picked up by the terrain rebuild below.
        let object_remap = targets.object_mega.take_remapped();
        if object_remap {
            let _span = info_span!("patch_object_mega_offsets").entered();
            self.patch_object_mega_offsets(targets.object_mega);
        }
        if targets.terrain_mega.take_remapped() {
            self.patch_terrain_mega_offsets(targets.terrain_mega);
            membership_changed = true;
        }

        let repack_instances =
            self.needs_repack || object_remap || targets.object_instances.needs_compaction();
        let repack_lights = self.needs_light_repack || targets.placed_lights.needs_compaction();
        if repack_instances || repack_lights {
            // A repack resets the slabs from resident cells only: queued
            // cells' ranges would dangle, so release them first.
            self.drain_retire_queue(targets, std::time::Duration::MAX);
        }
        if repack_instances {
            self.repack_instances(targets);
        }
        if repack_lights {
            self.repack_lights(targets);
        }

        // The terrain instance vec stays on the rebuild-everything path: at
        // ~1700 entries x 80 B it costs microseconds, not worth slab ranges.
        if membership_changed {
            let _span = info_span!("rebuild_terrain_instances").entered();
            self.rebuild_terrain_instances(targets.terrain_instances);
        }

        debug_assert!(
            self.loaded.keys().all(|g| !self.lod_loaded.contains_key(g)),
            "a cell grid must never be resident in both the full and LOD maps"
        );

        let uploaded = {
            let _span = info_span!("texture_residency").entered();
            self.update_texture_demand(targets.residency);
            let _pump = info_span!("texture_pump").entered();
            targets.residency.pump(
                targets.device,
                targets.queue,
                targets.texture_array,
                targets.upload_budget.remaining(),
            )
        };
        targets.upload_budget.add(uploaded);
        self.trace_textures(targets.residency);
        for texture in targets.residency.take_undemanded() {
            warn!(
                "texture undemanded {:.1}s: slot {} '{}' resident first_mip {} (of {} mips); owners: {}",
                texture.seconds,
                texture.slot,
                texture.path,
                texture.first_mip,
                texture.mip_count,
                self.describe_texture_owners(texture.slot),
            );
        }

        StreamUpdate {
            slot_changes: targets.texture_array.take_slot_changes(),
            sim_commands: std::mem::take(&mut self.sim_commands),
            spawn_complete,
            full_ring_complete,
        }
    }

    /// Size and field of view of the screen; takes effect at the next demand
    /// camera update.
    pub(crate) fn set_screen(&mut self, screen: ScreenView) {
        self.screen = screen;
    }

    /// Measure texture demand from `camera` (none: interior) and tell the
    /// workers.
    fn set_demand_camera(&mut self, camera: Option<Vec3>) {
        self.demand_view = DemandView::new(camera, self.screen, self.motion.lookahead());
        *self.worker_shared.demand_view.lock().unwrap() = self.demand_view;
    }

    /// Record every texture of `res` as a user at the strongest demand its
    /// sources place on it. A source whose bounding box cannot beat what its
    /// textures already have in the running sweep is skipped without walking
    /// its placements.
    fn note_cell_demand(res: &CellResources, view: &DemandView, residency: &mut TextureResidency) {
        for source in &res.demand_sources {
            let textures = &res.texture_refs[source.textures.clone()];
            let bound = view.pixels_per_uv(
                source.bounds_min,
                source.bounds_max,
                source.world_per_uv * source.max_scale,
            );
            if textures
                .iter()
                .all(|handle| residency.sweep_demand(handle.slot) >= bound)
            {
                continue;
            }
            let demand = match &source.placements {
                None => bound,
                Some(SourcePlacements {
                    group,
                    scales_start,
                }) => {
                    let instances = &res.object_groups[*group].instances;
                    let scales =
                        &res.placement_scales[*scales_start..*scales_start + instances.len()];
                    placements_demand(
                        view,
                        source.world_per_uv,
                        instances
                            .iter()
                            .zip(scales)
                            .map(|(i, &scale)| (i.aabb_min, i.aabb_max, scale)),
                    )
                }
            };
            for handle in textures {
                residency.note_demand(handle.slot, demand);
            }
        }
    }

    /// `OPENFO4_TEX_TRACE`: one line per traced resident texture with the
    /// source that sets its demand.
    fn trace_textures(&mut self, residency: &TextureResidency) {
        let Some(needle) = self.tex_trace.as_deref() else {
            return;
        };
        if self.last_tex_trace.elapsed() < TEX_TRACE_INTERVAL {
            return;
        }
        self.last_tex_trace = std::time::Instant::now();
        let view = self.demand_view;
        for t in residency.trace_textures(needle, TEX_TRACE_MAX_LINES) {
            info!(
                "tex trace slot {} '{}' {}x{} mips {} resident first_mip {} skip_limit {} wanted {} (cpu {}, feedback {:?}) \
                 demand {} px/uv; view: screen {}x{} fovy {:.1}deg pixel_scale {:.1} lookahead {:.1}; driver: {}",
                t.slot,
                t.path,
                t.width,
                t.height,
                t.mip_count,
                t.first_mip,
                t.skip_limit,
                t.wanted,
                t.cpu_wanted,
                t.feedback_wanted,
                t.demand,
                self.screen.width,
                self.screen.height,
                self.screen.vertical_fov.to_degrees(),
                view.pixel_scale(),
                view.lookahead(),
                self.demand_driver(&view, t.slot),
            );
        }
    }

    /// The source placing the strongest demand on `slot`, described for the trace.
    fn demand_driver(&self, view: &DemandView, slot: u32) -> String {
        let mut best: Option<(f32, String)> = None;
        let cells = self
            .loaded
            .iter()
            .map(|(grid, res)| (grid, "full", res))
            .chain(self.lod_loaded.iter().map(|(grid, res)| (grid, "lod", res)));
        for (grid, kind, res) in cells {
            for source in &res.demand_sources {
                if !res.texture_refs[source.textures.clone()]
                    .iter()
                    .any(|h| h.slot == slot)
                {
                    continue;
                }
                let (pixels, text) = match &source.placements {
                    None => {
                        let pixels = view.pixels_per_uv(
                            source.bounds_min,
                            source.bounds_max,
                            source.world_per_uv,
                        );
                        let distance =
                            view.effective_distance(source.bounds_min, source.bounds_max);
                        (
                            pixels,
                            format!(
                                "terrain world_per_uv {:.2} distance {distance:?}",
                                source.world_per_uv
                            ),
                        )
                    }
                    Some(SourcePlacements {
                        group,
                        scales_start,
                    }) => {
                        let group = &res.object_groups[*group];
                        let scales = &res.placement_scales
                            [*scales_start..*scales_start + group.instances.len()];
                        let Some((pixels, instance, scale)) = group
                            .instances
                            .iter()
                            .zip(scales)
                            .map(|(i, &scale)| {
                                let pixels = view.pixels_per_uv(
                                    Vec3::from(i.aabb_min),
                                    Vec3::from(i.aabb_max),
                                    source.world_per_uv * scale,
                                );
                                (pixels, i, scale)
                            })
                            .max_by(|a, b| a.0.total_cmp(&b.0))
                        else {
                            continue;
                        };
                        let distance = view.effective_distance(
                            Vec3::from(instance.aabb_min),
                            Vec3::from(instance.aabb_max),
                        );
                        let bucket = if instance.flags & INSTANCE_FLAGS_BLEND != 0 {
                            "blend"
                        } else if instance.flags & INSTANCE_FLAGS_DECAL != 0 {
                            "decal"
                        } else if instance.alpha_cutoff > 0.0 {
                            "alphatest"
                        } else {
                            "opaque"
                        };
                        (
                            pixels,
                            format!(
                                "{} x{} bucket {bucket} world_per_uv {:.2} max_scale {:.2} (placement scale {scale:.2}) distance {distance:?}",
                                group.mega_key,
                                group.instances.len(),
                                source.world_per_uv,
                                source.max_scale,
                            ),
                        )
                    }
                };
                if best.as_ref().is_none_or(|(b, _)| pixels > *b) {
                    best = Some((
                        pixels,
                        format!("{kind} cell {grid:?} {text} -> {pixels} px/uv"),
                    ));
                }
            }
        }
        best.map_or_else(|| "none loaded".to_owned(), |(_, text)| text)
    }

    /// The loaded sources that bind `slot`, for the undemanded-texture log: a
    /// full or LOD cell, then its terrain or mesh group with the instance
    /// count and the first instance's flags and alpha cutoff.
    fn describe_texture_owners(&self, slot: u32) -> String {
        const MAX_OWNERS: usize = 3;
        let mut owners = Vec::new();
        let cells = self
            .loaded
            .iter()
            .map(|(grid, res)| (grid, "full", res))
            .chain(self.lod_loaded.iter().map(|(grid, res)| (grid, "lod", res)));
        'cells: for (grid, kind, res) in cells {
            for source in &res.demand_sources {
                if !res.texture_refs[source.textures.clone()]
                    .iter()
                    .any(|handle| handle.slot == slot)
                {
                    continue;
                }
                let what = match &source.placements {
                    None => "terrain".to_owned(),
                    Some(SourcePlacements { group, .. }) => {
                        let group = &res.object_groups[*group];
                        let first = group.instances.first();
                        format!(
                            "{} x{} flags {} alpha_cutoff {}",
                            group.mega_key,
                            group.instances.len(),
                            first.map_or(0, |i| i.flags),
                            first.map_or(0.0, |i| i.alpha_cutoff),
                        )
                    }
                };
                owners.push(format!("{kind} cell {grid:?} {what}"));
                if owners.len() >= MAX_OWNERS {
                    break 'cells;
                }
            }
        }
        if owners.is_empty() {
            "none loaded".to_owned()
        } else {
            owners.join("; ")
        }
    }

    /// Cells for a new demand sweep: full-detail cells nearest the camera
    /// first, then the LOD cells.
    fn begin_demand_sweep(&self) -> DemandSweep {
        let mut near: Vec<(i32, i32)> = self.loaded.keys().copied().collect();
        if let Some(camera) = self.demand_view.camera() {
            let (cx, cy) = Self::camera_to_grid(camera);
            near.sort_unstable_by_key(|&(gx, gy)| (gx - cx).abs().max((gy - cy).abs()));
        }
        let cells = near
            .into_iter()
            .map(|grid| (grid, false))
            .chain(self.lod_loaded.keys().map(|&grid| (grid, true)))
            .collect();
        DemandSweep { cells, next: 0 }
    }

    /// Advance the texture demand sweep by at most `DEMAND_SWEEP_BUDGET`. A
    /// sweep (one pass over every resident cell) starts when the camera moved,
    /// the screen changed or cells were released, and replaces the demand when
    /// it completes. Cells integrated meanwhile note their demand directly.
    fn update_texture_demand(&mut self, residency: &mut TextureResidency) {
        let view = self.demand_view;
        if self.sweep.is_none() {
            let view_moved = self
                .last_demand_view
                .is_none_or(|previous| view.differs_from(&previous));
            let release_due =
                self.demand_stale && self.last_demand_pass.elapsed() >= DEMAND_STALE_INTERVAL;
            if !view_moved && !release_due {
                return;
            }
            self.sweep = Some(self.begin_demand_sweep());
            self.last_demand_view = Some(view);
            self.demand_stale = false;
        }

        let _span = info_span!("texture_demand_sweep").entered();
        let started = Instant::now();
        let Some(sweep) = self.sweep.as_mut() else {
            return;
        };
        while let Some(&(grid, lod)) = sweep.cells.get(sweep.next) {
            sweep.next += 1;
            let cells = if lod { &self.lod_loaded } else { &self.loaded };
            if let Some(res) = cells.get(&grid) {
                Self::note_cell_demand(res, &view, residency);
            }
            if started.elapsed() >= DEMAND_SWEEP_BUDGET {
                break;
            }
        }
        if sweep.next >= sweep.cells.len() {
            residency.finish_demand_sweep();
            self.sweep = None;
            self.last_demand_pass = Instant::now();
        }
    }

    /// The material bind group was just rebuilt. When texture slots were
    /// recycled before that, shadows cast by cells integrated since the last
    /// rebuild may have sampled the previous occupant's texture: re-render
    /// only those regions. Likewise for each resolved placeholder slot: every
    /// resident cell referencing it may have cast alpha-tested shadows with a
    /// white stand-in, so exactly those cells' regions re-render.
    pub(crate) fn on_materials_rebound(&mut self, slots_recycled: bool, placeholder_slots: &[u32]) {
        if !placeholder_slots.is_empty() {
            for res in self.loaded.values().chain(self.lod_loaded.values()) {
                let references_resolved = res
                    .texture_refs
                    .iter()
                    .any(|r| placeholder_slots.contains(&r.slot));
                if let (true, Some(bounds)) = (references_resolved, res.caster_bounds) {
                    self.shadow_invalidations.region(bounds);
                }
            }
        }
        if slots_recycled {
            if self.rebind_overflow {
                self.shadow_invalidations.everything();
            } else {
                for bounds in &self.integrated_since_rebind {
                    self.shadow_invalidations.region(*bounds);
                }
            }
        }
        self.integrated_since_rebind.clear();
        self.rebind_overflow = false;
    }

    /// `loaded` membership or a cell's light slab range changed: invalidates
    /// the per-frame collision-ring reconcile and the lamp candidate cache.
    fn mark_loaded_changed(&mut self) {
        self.collision_ring_dirty = true;
        self.lamp_candidates_dirty = true;
        self.loaded_generation = self.loaded_generation.wrapping_add(1);
    }

    /// Changes whenever the resident cells, or what they carry, change.
    pub(crate) fn loaded_generation(&self) -> u64 {
        self.loaded_generation
    }

    /// Move worker responses off the bounded channel into the (unbounded)
    /// local buffers. Called per frame AND between cell integrations so a
    /// long integrate frame never leaves workers blocked on a full channel.
    fn drain_worker_responses(&mut self) {
        let _span = info_span!("drain_worker_responses").entered();
        while let Ok(response) = self.db_worker_rx.try_recv() {
            // A worker mid-cell when the worldspace switched finishes and
            // ships its result with the old epoch; discard it (its grid key
            // could otherwise alias a freshly requested cell of the new
            // worldspace).
            match response {
                WorkerResponse::Full(r) if r.epoch == self.epoch => self.buffered_full.push_back(r),
                WorkerResponse::Lod(r) if r.epoch == self.epoch => self.buffered_lod.push_back(r),
                stale => crate::utils::drop_off_thread(stale),
            }
        }
    }

    /// Integrate a full-detail cell result. Gated: drops responses whose
    /// request marker was pruned, that left the ring, or that are already
    /// resident (double-response after a re-request). Returns true when cell
    /// membership changed.
    fn integrate_full(&mut self, result: &mut CellLoadResult, targets: &mut StreamTargets) -> bool {
        let _span = info_span!("integrate_full").entered();
        let grid = result.grid;
        if !self.pending_cells.remove(&grid) {
            return false;
        }
        if !self.in_full_ring(grid) || self.loaded.contains_key(&grid) {
            return false;
        }

        // Held, not registered: sync_collision_ring registers it once the
        // collision ring covers this grid (often this same frame).
        let mut res = CellResources {
            collider_payload: result
                .colliders
                .take()
                .map(|batch| (batch, result.baked_colliders.take())),
            ..CellResources::default()
        };

        if let Some(cpu) = result.terrain.take() {
            self.build_terrain_instance(&cpu, false, &result.textures, &mut res, targets);
        }
        {
            let _span = info_span!("build_mesh_groups", groups = result.meshes.len()).entered();
            for group in &result.meshes {
                self.build_mesh_group(group, &result.textures, &mut res, targets);
            }
        }
        self.upload_cell_instances(&mut res, targets);
        Self::add_cell_grass(&mut res, &result.grass, &result.grass_models, targets);
        res.lights = std::mem::take(&mut result.lights);
        res.shadow_lamps = std::mem::take(&mut result.shadow_lamps);
        res.light_models = std::mem::take(&mut result.light_models);
        self.upload_cell_lights(&mut res, targets);
        self.log_addon_attachments(&res, result.addon_lights);
        res.nav_meshes = self.nav.integrate(std::mem::take(&mut result.navmeshes));

        // Promotion: this grid was resident as LOD — swap the LOD twin out.
        if let Some(old) = self.lod_loaded.remove(&grid) {
            self.retire_cell(&old, targets);
        }

        info!(
            "Cell ({}, {}) ready: {} mesh groups, {} placed lights ({} shadow-type)",
            grid.0,
            grid.1,
            result.meshes.len(),
            res.lights.len(),
            res.shadow_lamps.len()
        );
        self.marker_instance_count += res.marker_count;
        self.fx_instance_count += res.fx_count;
        for bucket in 0..DRAW_BUCKET_COUNT {
            self.bucket_potential_total[bucket] += res.bucket_potential[bucket];
        }
        // Index this cell's dynamic REFRs so the pose readback finds them O(1).
        for entity in res.dynamic_instances.keys() {
            self.entity_cell.insert(*entity, grid);
        }
        res.animated_entities = result
            .animated_refs
            .iter()
            .map(|refr| refr.entity)
            .collect();
        self.animated_events.extend(
            result
                .animated_refs
                .drain(..)
                .map(AnimatedObjectEvent::Added),
        );
        res.ragdoll_entities = result.ragdoll_refs.iter().map(|refr| refr.entity).collect();
        self.ragdoll_events
            .extend(result.ragdoll_refs.drain(..).map(RagdollEvent::Added));
        Self::note_cell_demand(&res, &self.demand_view, targets.residency);
        targets.residency.end_demand_round();
        self.loaded.insert(grid, res);
        self.mark_loaded_changed();
        true
    }

    /// Integrate a LOD cell result, same gating as `integrate_full`. Handles
    /// demotion: a previously full-detail cell is released when its LOD
    /// replacement arrives.
    fn integrate_lod(&mut self, result: &mut CellLoadResult, targets: &mut StreamTargets) -> bool {
        let _span = info_span!("integrate_lod").entered();
        let grid = result.grid;
        if !self.lod_pending_cells.remove(&grid) {
            return false;
        }
        if !self.in_lod_ring(grid) || self.lod_loaded.contains_key(&grid) {
            return false;
        }

        let mut res = CellResources::default();

        if let Some(cpu) = result.terrain.take() {
            self.build_terrain_instance(&cpu, true, &result.textures, &mut res, targets);
        }
        for group in &result.meshes {
            self.build_mesh_group(group, &result.textures, &mut res, targets);
        }
        self.upload_cell_instances(&mut res, targets);

        // Demotion: full-detail twin leaves only now that its replacement is
        // ready. Its colliders, if it still had any registered, are
        // deregistered by sync_collision_ring when it leaves `loaded`.
        if let Some(old) = self.loaded.remove(&grid) {
            self.mark_loaded_changed();
            self.retire_cell(&old, targets);
        }

        self.marker_instance_count += res.marker_count;
        self.fx_instance_count += res.fx_count;
        for bucket in 0..DRAW_BUCKET_COUNT {
            self.bucket_potential_total[bucket] += res.bucket_potential[bucket];
        }
        Self::note_cell_demand(&res, &self.demand_view, targets.residency);
        targets.residency.end_demand_round();
        self.lod_loaded.insert(grid, res);
        true
    }

    /// Register/deregister cell colliders so exactly the resident full cells
    /// within the collision ring of the player are in the rapier world, and
    /// move the sim's dynamic-physics center/radius. The ring is
    /// `max(collision_radius, physics_radius)` so a raised debug radius still
    /// gives dynamic bodies static ground to rest on, while the player keeps at
    /// least the vanilla radius-2 static collision.
    /// Payloads stay in `CellResources` for the cell's whole residency, so a
    /// cell can cross the ring boundary repeatedly at Arc-clone cost — the
    /// shapes are never rebuilt.
    ///
    /// All eligible cells register in the SAME frame deliberately: each
    /// broad-phase fold pays a roughly tree-sized constant (optimization +
    /// refit + full pair scan), so one fold absorbing a whole ring shift
    /// beats spreading the inserts across frames — measured: one-cell-per-
    /// frame throttling produced 5x more folds and 2.5x more >33 ms frames.
    fn sync_collision_ring(&mut self) {
        let _span = info_span!("sync_collision_ring").entered();
        let (cx, cy) = self.current_cell;
        let ring = collision_ring_radius(self.collision_radius, self.physics_radius);
        let in_ring = |g: &(i32, i32)| (g.0 - cx).abs().max((g.1 - cy).abs()) <= ring;

        let key = (self.current_cell, ring);
        let center = (self.current_cell, self.physics_radius);
        if !self.collision_ring_dirty
            && self.collision_sync_key == Some(key)
            && center == self.last_physics_center
        {
            return;
        }
        self.collision_ring_dirty = false;
        self.collision_sync_key = Some(key);

        let stale: Vec<(i32, i32)> = self
            .collision_registered
            .iter()
            .filter(|g| !in_ring(g) || !self.loaded.contains_key(g))
            .copied()
            .collect();
        let mut ring_changed = !stale.is_empty();
        for grid in stale {
            self.collision_registered.remove(&grid);
            self.sim_commands
                .push(SimCommand::RemoveCellColliders { grid });
        }

        for dx in -ring..=ring {
            for dy in -ring..=ring {
                let grid = (cx + dx, cy + dy);
                if self.collision_registered.contains(&grid) {
                    continue;
                }
                let Some(res) = self.loaded.get(&grid) else {
                    continue;
                };
                let Some((batch, baked)) = &res.collider_payload else {
                    continue;
                };
                self.sim_commands.push(SimCommand::AddCellColliders {
                    grid,
                    batch: batch.clone(),
                    baked: baked.clone(),
                });
                self.collision_registered.insert(grid);
                ring_changed = true;
            }
        }

        // Drive dynamic-body activation — but only when the active set could have
        // changed: the center/radius moved, or the ring gained/lost a cell (new
        // bodies to promote / despawned bodies). A stationary player's idle
        // frames push nothing, so `sim_commands` stays empty (fast path) and the
        // sim skips its per-frame radius reconcile.
        if ring_changed || center != self.last_physics_center {
            self.sim_commands.push(SimCommand::SetPhysicsCenter {
                grid: self.current_cell,
                radius: self.physics_radius,
            });
            self.last_physics_center = center;
        }
    }

    /// Retire a resident cell: roll its marker tally out of the running
    /// total, then release its GPU resources. Every removal from
    /// `loaded`/`lod_loaded` must come through here so the tally balances
    /// the `+=` in `integrate_full/integrate_lod`.
    fn retire_cell(&mut self, res: &CellResources, targets: &mut StreamTargets) {
        self.forget_cell_entities(res);
        self.finish_retire(res, targets);
    }

    /// Unload bookkeeping that must happen at once: a reload of the same grid
    /// before the queued release runs re-indexes its entities.
    fn defer_retire(&mut self, res: CellResources) {
        self.forget_cell_entities(&res);
        self.retire_queue.push_back(res);
    }

    /// The navigation meshes of the resident full-detail and interior cells.
    pub(crate) fn nav_world(&self) -> &NavWorld {
        self.nav.world()
    }

    fn forget_cell_entities(&mut self, res: &CellResources) {
        self.nav.retire(&res.nav_meshes);
        for entity in res.dynamic_instances.keys() {
            self.entity_cell.remove(entity);
        }
        self.animated_events.extend(
            res.animated_entities
                .iter()
                .map(|entity| AnimatedObjectEvent::Removed(*entity)),
        );
        self.ragdoll_events.extend(
            res.ragdoll_entities
                .iter()
                .map(|entity| RagdollEvent::Removed(*entity)),
        );
    }

    /// Ragdoll-capable objects that entered or left residency since the last call.
    pub(crate) fn take_ragdoll_events(&mut self) -> Vec<RagdollEvent> {
        std::mem::take(&mut self.ragdoll_events)
    }

    /// Animated objects that entered or left residency since the last call.
    pub(crate) fn take_animated_events(&mut self) -> Vec<AnimatedObjectEvent> {
        std::mem::take(&mut self.animated_events)
    }

    /// Release queued cells oldest-first until `budget` is spent (at least one
    /// cell per call, so the queue always makes progress).
    fn drain_retire_queue(&mut self, targets: &mut StreamTargets, budget: std::time::Duration) {
        let start = std::time::Instant::now();
        while let Some(res) = self.retire_queue.pop_front() {
            self.finish_retire(&res, targets);
            drop(res);
            if start.elapsed() >= budget {
                break;
            }
        }
    }

    fn finish_retire(&mut self, res: &CellResources, targets: &mut StreamTargets) {
        let _span = tracing::trace_span!("retire_cell").entered();
        self.demand_stale = true;
        if let Some(bounds) = res.caster_bounds {
            self.shadow_invalidations.region(bounds);
        }
        if res.has_grass {
            with_grass_gpu(targets, |grass, gpu| grass.remove_cell(res.residency, gpu));
        }
        self.marker_instance_count -= res.marker_count;
        self.fx_instance_count -= res.fx_count;
        for bucket in 0..DRAW_BUCKET_COUNT {
            debug_assert!(
                self.bucket_potential_total[bucket] >= res.bucket_potential[bucket],
                "bucket {bucket} potential underflow: resident {} < retiring cell {}",
                self.bucket_potential_total[bucket],
                res.bucket_potential[bucket],
            );
            self.bucket_potential_total[bucket] =
                self.bucket_potential_total[bucket].saturating_sub(res.bucket_potential[bucket]);
        }
        Self::release_cell(res, targets);
    }

    /// Hands the cell's grass to the grass stage, keyed by the cell's residency id.
    /// The buffers it creates count against the frame's upload budget.
    fn add_cell_grass(
        res: &mut CellResources,
        grass: &[CellGrass],
        models: &GrassModels,
        targets: &mut StreamTargets,
    ) {
        if grass.is_empty() {
            return;
        }
        let residency = res.residency;
        let added = with_grass_gpu(targets, |stage, gpu| {
            stage.add_cell(residency, grass, models, gpu)
        });
        res.has_grass = added.holds_grass;
        targets.upload_budget.add(added.uploaded_bytes);
    }

    /// Release every GPU resource a cell owns.
    fn release_cell(res: &CellResources, targets: &mut StreamTargets) {
        {
            let _span = tracing::trace_span!("release_slab_ranges").entered();
            if let Some((offset, count)) = res.instance_range {
                targets.object_instances.free(offset, count, targets.queue);
            }
            if let Some((offset, count)) = res.light_range {
                targets.placed_lights.free(offset, count, targets.queue);
            }
        }
        let _span = tracing::trace_span!("release_cell_buffers").entered();
        Self::release_cell_buffers(res, targets);
    }

    /// Release the refcounted, per-cell resources (mega-buffer slots, splat
    /// layers, texture slots) but NOT the object instance-slab or placed-light
    /// slab ranges. Those are freed by `release_cell` for incremental unloads;
    /// the whole-world teardown in `switch_worldspace` skips that per-cell
    /// free and resets the slabs wholesale instead (see there).
    fn release_cell_buffers(res: &CellResources, targets: &mut StreamTargets) {
        for group in &res.object_groups {
            targets.object_mega.release(&group.mega_key);
        }
        if let Some(key) = &res.terrain_mega_key {
            targets.terrain_mega.release(key);
        }
        if let Some(key) = &res.splat_key {
            targets.splatmaps.release(key);
        }
        for &handle in &res.texture_refs {
            if targets.texture_array.release(handle) {
                targets.residency.forget(handle);
            }
        }
    }

    /// Flatten the cell's groups into one contiguous slab range and upload
    /// it: one allocate + one write per cell, ~tens of KB, instead of the old
    /// O(total instances) vec rebuild + 48 MB re-upload per membership
    /// change. On allocation failure (instance buffer at the device cap) the
    /// cell's objects stay invisible until the scheduled repack assigns them
    /// a range.
    fn upload_cell_instances(&mut self, res: &mut CellResources, targets: &mut StreamTargets) {
        let _span = info_span!("upload_cell_instances").entered();
        self.next_residency += 1;
        res.residency = self.next_residency;
        res.group_base = group_base_offsets(&res.object_groups);
        res.caster_bounds = caster_bounds(res);
        if let Some(bounds) = res.caster_bounds {
            self.shadow_invalidations.region(bounds);
            if self.integrated_since_rebind.len() < MAX_TRACKED_INTEGRATIONS {
                self.integrated_since_rebind.push(bounds);
            } else {
                self.rebind_overflow = true;
            }
        }
        let total: usize = res.object_groups.iter().map(|g| g.instances.len()).sum();
        if total == 0 {
            return;
        }
        let mut flat: Vec<GpuInstance> = Vec::with_capacity(total);
        for group in &res.object_groups {
            flat.extend_from_slice(&group.instances);
        }
        match targets
            .object_instances
            .allocate(total as u32, targets.device, targets.queue)
        {
            Some(offset) => {
                targets.object_instances.write(offset, &flat, targets.queue);
                targets
                    .upload_budget
                    .add((total * std::mem::size_of::<GpuInstance>()) as u64);
                res.instance_range = Some((offset, total as u32));
            }
            None => {
                self.needs_repack = true;
            }
        }
    }

    /// Reports the addon attachments of a cell that became resident, naming the node indices no
    /// addon record answers to.
    fn log_addon_attachments(&self, res: &CellResources, lights: u32) {
        if res.addon_attachments.is_empty() {
            return;
        }
        let mut unresolved = 0;
        for attachment in &res.addon_attachments {
            let index = attachment.addon.index;
            if self.worker_shared.addon_nodes.get(index).is_none() {
                unresolved += 1;
                crate::utils::warn_once(&format!("addon_index_unresolved:{index}"), || {
                    format!(
                        "addon node {:?} has index {index}, which no addon record answers to; it spawns nothing",
                        attachment.node_name
                    )
                });
            }
        }
        info!(
            "addon attachments: {} refs, {lights} lights, {unresolved} unresolved",
            res.addon_attachments.len()
        );
    }

    /// Flatten the cell's placed lights into one contiguous slab range and
    /// upload it, mirroring `upload_cell_instances`: on allocation failure the
    /// cell's lights stay dark until the scheduled repack assigns a range.
    fn upload_cell_lights(&mut self, res: &mut CellResources, targets: &mut StreamTargets) {
        if res.lights.is_empty() {
            return;
        }
        let count = res.lights.len() as u32;
        match targets
            .placed_lights
            .allocate(count, targets.device, targets.queue)
        {
            Some(offset) => {
                targets
                    .placed_lights
                    .write(offset, &res.lights, targets.queue);
                targets
                    .upload_budget
                    .add(res.lights.len() as u64 * std::mem::size_of::<GpuPlacedLight>() as u64);
                res.light_range = Some((offset, count));
            }
            None => {
                self.needs_light_repack = true;
            }
        }
    }

    /// Rebuild the whole object slab tightly packed from the per-cell CPU
    /// copies and re-point every cell's range. O(total instances) — the cost of
    /// the old per-membership-change rebuild — but runs only on fragmentation
    /// pressure, a slab allocation failure, or an object mega-buffer
    /// compaction (which invalidates every baked offset anyway).
    fn repack_instances(&mut self, targets: &mut StreamTargets) {
        let _span = info_span!("repack_instances").entered();
        // Cells whose slab allocation failed rendered no casters until now;
        // cached cascades covering them have to render again.
        let newly_placed: Vec<Aabb> = self
            .loaded
            .values()
            .chain(self.lod_loaded.values())
            .filter(|r| r.instance_range.is_none())
            .filter_map(|r| r.caster_bounds)
            .collect();
        let mut cells: Vec<&mut CellResources> = self
            .loaded
            .values_mut()
            .chain(self.lod_loaded.values_mut())
            .collect();
        let total: usize = cells
            .iter()
            .flat_map(|r| r.object_groups.iter())
            .map(|g| g.instances.len())
            .sum();
        let uploaded = repack_slab(
            targets.object_instances,
            &mut cells,
            total,
            |res, packed| {
                for group in &res.object_groups {
                    packed.extend_from_slice(&group.instances);
                }
            },
            |res| &mut res.instance_range,
            targets.device,
            targets.queue,
        );
        self.needs_repack = false;
        for bounds in newly_placed {
            self.shadow_invalidations.region(bounds);
        }
        info!("instance slab repacked: {} live instances", uploaded);
    }

    /// Placed-light twin of `repack_instances`.
    fn repack_lights(&mut self, targets: &mut StreamTargets) {
        let _span = info_span!("repack_lights").entered();
        let mut cells: Vec<&mut CellResources> = self
            .loaded
            .values_mut()
            .chain(self.lod_loaded.values_mut())
            .collect();
        let total: usize = cells.iter().map(|r| r.lights.len()).sum();
        let uploaded = repack_slab(
            targets.placed_lights,
            &mut cells,
            total,
            |res, packed| packed.extend_from_slice(&res.lights),
            |res| &mut res.light_range,
            targets.device,
            targets.queue,
        );
        self.needs_light_repack = false;
        self.mark_loaded_changed();
        info!("placed light slab repacked: {} live lights", uploaded);
    }

    /// Rebuild the terrain instance vec from the resident cells. Cheap
    /// (O(resident cells)); runs on any membership change.
    fn rebuild_terrain_instances(&self, terrain_instances: &mut TerrainInstanceBuffer) {
        terrain_instances.instances.clear();
        for res in self.loaded.values().chain(self.lod_loaded.values()) {
            if let Some(t) = &res.terrain {
                terrain_instances.instances.push(*t);
            }
        }
        terrain_instances.dirty = true;
    }

    /// An object mega-buffer compaction moved every slot: re-read each
    /// group's slot by key and rewrite the offsets baked into its CPU
    /// instance copies. The caller must follow with `repack_instances` to push
    /// the patched copies to the GPU.
    fn patch_object_mega_offsets(&mut self, object_mega: &MegaBuffer) {
        for res in self.loaded.values_mut().chain(self.lod_loaded.values_mut()) {
            for group in &mut res.object_groups {
                if let Some(slot) = object_mega.slot(&group.mega_key) {
                    for inst in &mut group.instances {
                        inst.index_offset = slot.index_offset;
                        inst.index_count = slot.index_count;
                        inst.vertex_offset = slot.vertex_offset;
                    }
                }
            }
        }
    }

    /// Terrain twin of `patch_object_mega_offsets`; the caller must follow
    /// with `rebuild_terrain_instances`.
    fn patch_terrain_mega_offsets(&mut self, terrain_mega: &MegaBuffer) {
        for res in self.loaded.values_mut().chain(self.lod_loaded.values_mut()) {
            if let (Some(key), Some(t)) = (&res.terrain_mega_key, res.terrain.as_mut())
                && let Some(slot) = terrain_mega.slot(key)
            {
                t.index_offset = slot.index_offset;
                t.index_count = slot.index_count;
                t.vertex_offset = slot.vertex_offset;
            }
        }
    }

    /// Acquire a texture slot with refcounting and record the reference on the
    /// cell. Exhaustion and load failures return `fallback_slot` with no
    /// recorded reference.
    ///
    /// `preloaded` is the cell's worker-side pre-read (see
    /// `CellLoadResult::textures`): the hot path here is a pure GPU upload. A
    /// texture the worker saw resident but that was evicted before
    /// integration gets a placeholder view and an asynchronous load; the
    /// main thread never reads the disk.
    fn acquire_texture(
        &mut self,
        path: &str,
        fallback_slot: u32,
        srgb: bool,
        preloaded: &HashMap<String, Option<Arc<TexturePayload>>>,
        res: &mut CellResources,
        targets: &mut StreamTargets,
    ) -> u32 {
        if let Some(handle) = targets.texture_array.try_add_ref(path) {
            res.texture_refs.push(handle);
            return handle.slot;
        }
        let payload = match preloaded.get(path) {
            Some(Some(payload)) => Some(payload),
            // The worker already tried, failed and logged the cause.
            Some(None) => return fallback_slot,
            None => None,
        };
        let _span = info_span!("texture_admit").entered();
        // Cells integrate atomically, so one texture-heavy cell would upload
        // all of its payloads in a single frame. Once the frame's byte budget
        // is spent large payloads are handed to the residency as finished
        // reads, which its per-frame budgeted pump applies without more IO.
        let upload_now = !(defer_admit_enabled() && targets.upload_budget.exhausted());
        let mut drained = false;
        loop {
            match admit_texture(
                targets.texture_array,
                targets.residency,
                targets.device,
                targets.queue,
                path,
                srgb,
                fallback_slot,
                payload,
                upload_now,
            ) {
                Admission::Admitted {
                    handle,
                    uploaded_bytes,
                } => {
                    targets.upload_budget.add(uploaded_bytes);
                    res.texture_refs.push(handle);
                    return handle.slot;
                }
                // Array exhausted: nothing was inserted. Cells waiting in the
                // retire queue still hold slots, so release them and retry once.
                Admission::Exhausted if !drained && !self.retire_queue.is_empty() => {
                    drained = true;
                    self.drain_retire_queue(targets, std::time::Duration::MAX);
                }
                Admission::Exhausted | Admission::Failed => return fallback_slot,
            }
        }
    }

    fn acquire_material_slots(
        &mut self,
        mat: &SubMeshMaterial,
        preloaded: &HashMap<String, Option<Arc<TexturePayload>>>,
        res: &mut CellResources,
        targets: &mut StreamTargets,
    ) -> MaterialSlots {
        mat.source().resolve_slots(|role, path| {
            self.acquire_texture(
                path,
                role.fallback_slot(),
                role.is_srgb(),
                preloaded,
                res,
                targets,
            )
        })
    }

    /// Records the placements of a water submesh as water-flagged instances:
    /// kept in the slab so mega-buffer compaction and cell retirement treat
    /// them like any instance, but skipped by every cull and shadow pass. The
    /// water stage reads them back through `for_each_placed_water`. No textures
    /// are acquired: the water shader samples the WATR's own noise textures.
    fn push_water_instances(
        group: &PreparedMeshGroup,
        prepared: &PreparedSubMesh,
        shader: WaterShaderToken,
        slot: &crate::rendering::mega_buffer::MeshSlot,
        res: &mut CellResources,
    ) {
        let group_idx = res.object_groups.len();
        let mut cell_group = CellObjectGroup {
            mega_key: Arc::clone(&prepared.mega_key),
            instances: Vec::with_capacity(group.transforms.len()),
        };
        for (instance_idx, placement) in group.transforms.iter().enumerate() {
            let transform = match &prepared.anim {
                Some(part) => *placement * part.rest_in_model,
                None => *placement,
            };
            let (aabb_min, aabb_max) = world_aabb(prepared.local_aabb.as_ref(), &transform);
            cell_group.instances.push(GpuInstance {
                transform: transform.to_cols_array_2d(),
                aabb_min,
                _pad0: 0.0,
                aabb_max,
                _pad1: 0.0,
                index_offset: slot.index_offset,
                index_count: slot.index_count,
                vertex_offset: slot.vertex_offset,
                material_id: FALLBACK_MATERIAL_ID,
                alpha_cutoff: 0.0,
                alpha_mult: 1.0,
                flags: INSTANCE_FLAGS_WATER,
                _pad2: 0,
            });
            res.water_surfaces.push(WaterRef {
                group: group_idx,
                instance: instance_idx,
                water_type: group.water_type.clone(),
                volume: group.water_volume.clone(),
                shader,
            });
        }
        res.object_groups.push(cell_group);
    }

    /// Records the placements of an effect or refraction submesh for their stage. Such a
    /// submesh never enters the object buffers, so no cull, shadow or texture pass sees it.
    /// Returns whether the submesh was taken; marker placements keep every shape in the object
    /// path, where the marker flag hides them.
    fn push_effect_placements(
        group: &PreparedMeshGroup,
        prepared: &PreparedSubMesh,
        res: &mut CellResources,
    ) -> bool {
        if matches!(prepared.route, ShapeRoute::Object) {
            return false;
        }
        let Some(local_aabb) = prepared.local_aabb else {
            crate::utils::warn_once(&format!("effect-bounds:{}", prepared.mega_key), || {
                format!(
                    "{}: effect shape has no vertices to bound; its {} placements are skipped",
                    prepared.mega_key,
                    group.transforms.len()
                )
            });
            return true;
        };
        match &prepared.route {
            ShapeRoute::Object => return false,
            ShapeRoute::Effect { mesh, material } => {
                res.effect_instances
                    .extend(group.transforms.iter().map(|placement| EffectInstance {
                        mesh: Arc::clone(mesh),
                        material: Arc::clone(material),
                        model: *placement,
                        bounds: local_aabb.transformed(placement),
                    }));
            }
            ShapeRoute::Refraction { mesh, material } => {
                res.refraction_instances
                    .extend(group.transforms.iter().map(|placement| RefractionInstance {
                        mesh: Arc::clone(mesh),
                        material: Arc::clone(material),
                        model: *placement,
                        bounds: local_aabb.transformed(placement),
                    }));
            }
        }
        true
    }

    /// Add every placement of one mesh path to the cell. Geometry upload and
    /// material/texture acquisition happen once per submesh; per-placement work
    /// is just an 8-corner AABB transform and an instance push.
    fn build_mesh_group(
        &mut self,
        group: &PreparedMeshGroup,
        preloaded: &HashMap<String, Option<Arc<TexturePayload>>>,
        res: &mut CellResources,
        targets: &mut StreamTargets,
    ) {
        if !group.is_marker {
            res.addon_attachments
                .extend(attachments_of(&group.submeshes.extras, &group.transforms));
            if !is_animated_model(&group.submeshes) {
                res.particle_hosts.extend(particle_hosts_of(
                    &group.model_path,
                    &group.submeshes.extras,
                    &group.transforms,
                ));
            }
        }
        for (submesh_index, prepared) in group.submeshes.iter().enumerate() {
            if !group.is_marker && Self::push_effect_placements(group, prepared, res) {
                continue;
            }
            // Material name already decoded at GLB parse time on the worker
            // (SubMeshMaterial::parse) — no string work here. A material-swap group
            // overrides per-submesh materials (different textures / alpha /
            // cull flags); geometry below stays keyed by path and is shared
            // across variants.
            let mat = group
                .overrides
                .get(submesh_index)
                .and_then(|o| o.as_ref())
                .unwrap_or(&prepared.material);

            // Keyed by submesh index, not texture path: two submeshes of one
            // GLB can share a texture but never share geometry.
            let mesh_key = &prepared.mega_key;
            let Some((slot, fresh)) = targets.object_mega.allocate(
                mesh_key,
                &prepared.mesh.vertices,
                &prepared.mesh.indices,
                targets.device,
                targets.queue,
            ) else {
                // Buffer exhausted even after compaction: render without this
                // submesh rather than crash. allocate() already warned.
                continue;
            };
            if fresh {
                targets.upload_budget.add(
                    (prepared.mesh.vertices.len() * std::mem::size_of::<Vertex>()
                        + prepared.mesh.indices.len() * std::mem::size_of::<u32>())
                        as u64,
                );
            }

            if let Some(shader) = mat.water {
                Self::push_water_instances(group, prepared, shader, &slot, res);
                continue;
            }

            let texture_start = res.texture_refs.len();
            let material_id = match mat.gpu_key() {
                Some(key) => {
                    let slots = self.acquire_material_slots(mat, preloaded, res, targets);
                    if mat.alpha_cutoff > 0.0 && mat.diffuse_path.is_some() {
                        targets.texture_array.mark_alpha_tested(slots.diffuse);
                    }
                    build_gpu_material(
                        &mat.source(),
                        key,
                        slots,
                        targets.cube_array,
                        targets.material_registry,
                    )
                }
                None => FALLBACK_MATERIAL_ID,
            };

            let instance_count = group.transforms.len() as u32;
            let visibility = visibility_flag(group.is_marker, group.is_fx, mat);
            if visibility == INSTANCE_FLAGS_MARKER {
                res.marker_count += instance_count;
            } else if visibility == INSTANCE_FLAGS_HIDDEN_FX {
                res.fx_count += instance_count;
            }
            let flags = material_instance_flags(mat) | visibility;
            for bucket in potential_buckets(flags, mat.alpha_cutoff) {
                res.bucket_potential[bucket] += instance_count;
            }

            let mut cell_group = CellObjectGroup {
                mega_key: Arc::clone(mesh_key),
                instances: Vec::with_capacity(group.transforms.len()),
            };

            let scales_start = res.placement_scales.len();
            let mut bounds_min = Vec3::splat(f32::INFINITY);
            let mut bounds_max = Vec3::splat(f32::NEG_INFINITY);
            let mut max_scale = 0.0f32;
            for placement in &group.transforms {
                let transform = &match &prepared.anim {
                    Some(part) => *placement * part.rest_in_model,
                    None => *placement,
                };
                let (aabb_min, aabb_max) = world_aabb(prepared.local_aabb.as_ref(), transform);
                let scale = max_axis_scale(&transform.to_cols_array_2d());
                res.placement_scales.push(scale);
                max_scale = max_scale.max(scale);
                bounds_min = bounds_min.min(Vec3::from(aabb_min));
                bounds_max = bounds_max.max(Vec3::from(aabb_max));

                cell_group.instances.push(GpuInstance {
                    transform: transform.to_cols_array_2d(),
                    aabb_min,
                    _pad0: 0.0,
                    aabb_max,
                    _pad1: 0.0,
                    index_offset: slot.index_offset,
                    index_count: slot.index_count,
                    vertex_offset: slot.vertex_offset,
                    material_id,
                    alpha_cutoff: mat.alpha_cutoff,
                    alpha_mult: mat.alpha_mult,
                    flags,
                    _pad2: 0,
                });
            }

            // Record this submesh's dynamic-clutter rows so the per-frame pose
            // readback can rewrite them. One row per (this submesh × the REFR's
            // instance); recorded after the push so group_idx is final and a
            // submesh skipped on mega-allocation failure contributes none.
            let group_idx = res.object_groups.len();
            let moving_joint = prepared
                .anim
                .as_ref()
                .filter(|part| !group.dynamic && part.is_moving())
                .map(|part| part.joint);
            if group.dynamic || group.ragdoll || moving_joint.is_some() {
                for (i, entity) in group.entities.iter().enumerate() {
                    if let Some(entity) = entity {
                        let scale = group.transforms[i].to_scale_rotation_translation().0.x;
                        res.dynamic_instances
                            .entry(*entity)
                            .or_default()
                            .push(DynRow {
                                group_idx,
                                instance_idx: i,
                                local_aabb: prepared.local_aabb,
                                scale,
                                local: group.locals.get(i).copied().unwrap_or(Mat4::IDENTITY),
                                joint: moving_joint,
                                last_transform: Mat4::NAN,
                            });
                    }
                }
            }

            res.object_groups.push(cell_group);
            if res.texture_refs.len() > texture_start {
                res.demand_sources.push(DemandSource {
                    textures: texture_start..res.texture_refs.len(),
                    world_per_uv: prepared.world_per_uv,
                    bounds_min,
                    bounds_max,
                    max_scale,
                    placements: Some(SourcePlacements {
                        group: group_idx,
                        scales_start,
                    }),
                });
            }
        }
    }

    fn build_terrain_instance(
        &mut self,
        cpu: &terrain::TerrainCellCpu,
        lod: bool,
        preloaded: &HashMap<String, Option<Arc<TexturePayload>>>,
        res: &mut CellResources,
        targets: &mut StreamTargets,
    ) {
        if !lod {
            res.land_lowest_y = Some(cpu.world_bounds().0.y);
        }
        // Full and LOD variants are distinct geometry — distinct keys, or a
        // promotion would forever return the cached LOD slot.
        let variant = if lod { "lod" } else { "full" };
        let mesh_key = format!("terrain_{}_{}_{}", variant, cpu.grid_x, cpu.grid_y);
        let Some((slot, fresh)) = targets.terrain_mega.allocate(
            &mesh_key,
            &cpu.vertices,
            &cpu.indices,
            targets.device,
            targets.queue,
        ) else {
            // Buffer exhausted even after compaction: skip this cell's terrain.
            return;
        };
        if fresh {
            targets.upload_budget.add(
                (cpu.vertices.len() * std::mem::size_of::<Vertex>()
                    + cpu.indices.len() * std::mem::size_of::<u32>()) as u64,
            );
        }
        res.terrain_mega_key = Some(mesh_key);

        let (bounds_min, bounds_max) = cpu.world_bounds();
        let world_per_uv = cpu.world_per_uv();
        let texture_start = res.texture_refs.len();
        let (splat_layer, layer_indices, flags) = if let Some(tile) = &cpu.baked_tile {
            // Baked level-4 LOD tile: one shared texture (up to 16 cells per
            // tile via the refcount), no splat slot consumed — the shader
            // ignores splat weights when TERRAIN_FLAGS_BAKED is set. A load
            // failure here, despite the worker's existence check, leaves the
            // magenta FALLBACK_SLOT as the deliberate last resort.
            let slot = self.acquire_texture(tile, FALLBACK_SLOT, true, preloaded, res, targets);
            (
                SPLAT_FALLBACK_LAYER,
                [slot, FALLBACK_SLOT, FALLBACK_SLOT, FALLBACK_SLOT],
                [TERRAIN_FLAGS_BAKED, 0, 0, 0],
            )
        } else {
            // Splat data is identical for full and LOD variants — shared key;
            // the refcount covers the promotion/demotion overlap window.
            let splat_key = format!("splatmap_{}_{}", cpu.grid_x, cpu.grid_y);
            let mut splat_layer =
                targets
                    .splatmaps
                    .acquire(&splat_key, &cpu.splatmap_data, targets.queue);
            if splat_layer == SPLAT_FALLBACK_LAYER && !self.retire_queue.is_empty() {
                self.drain_retire_queue(targets, std::time::Duration::MAX);
                splat_layer =
                    targets
                        .splatmaps
                        .acquire(&splat_key, &cpu.splatmap_data, targets.queue);
            }
            if splat_layer != SPLAT_FALLBACK_LAYER {
                res.splat_key = Some(splat_key);
            }

            // Layer textures
            let mut layer_indices = [0u32; 4];
            for (i, path_opt) in cpu.texture_paths.iter().enumerate() {
                if let Some(path) = path_opt {
                    // Key construction pinned to the worker's pre-read
                    // (db_worker's preload_textures block): both sites must
                    // build byte-identical strings.
                    let tex_path = terrain_layer_texture_key(path);
                    layer_indices[i] = self.acquire_texture(
                        &tex_path,
                        FALLBACK_SLOT,
                        true,
                        preloaded,
                        res,
                        targets,
                    );
                }
            }
            (splat_layer, layer_indices, [0u32; 4])
        };

        if res.texture_refs.len() > texture_start {
            res.demand_sources.push(DemandSource {
                textures: texture_start..res.texture_refs.len(),
                world_per_uv,
                bounds_min,
                bounds_max,
                max_scale: 1.0,
                placements: None,
            });
        }

        let cell_aabb_min = [
            cpu.grid_x as f32 * CELL_SIZE_WORLD,
            -1000.0,
            -((cpu.grid_y + 1) as f32 * CELL_SIZE_WORLD),
        ];
        let cell_aabb_max = [
            (cpu.grid_x + 1) as f32 * CELL_SIZE_WORLD,
            5000.0,
            -(cpu.grid_y as f32 * CELL_SIZE_WORLD),
        ];

        res.terrain = Some(GpuTerrainInstance {
            aabb_min: cell_aabb_min,
            _pad0: 0.0,
            aabb_max: cell_aabb_max,
            _pad1: 0.0,
            index_offset: slot.index_offset,
            index_count: slot.index_count,
            vertex_offset: slot.vertex_offset,
            splat_layer,
            layer_indices,
            flags,
        });
    }

    pub(crate) fn camera_to_grid(eye: Vec3) -> (i32, i32) {
        let gx = (eye.x / CELL_SIZE_WORLD).floor() as i32;
        let gy = (-eye.z / CELL_SIZE_WORLD).floor() as i32;
        (gx, gy)
    }

    /// Where the player is for the ambient music: the worldspace or loaded interior, the grid cell
    /// and the point in Bethesda units.
    pub(crate) fn music_place(&self, eye: Vec3) -> PlaceSnapshot {
        Self::place_snapshot(&self.worldspace, self.interior_cell.as_deref(), eye)
    }

    fn place_snapshot(worldspace: &str, interior_cell: Option<&str>, eye: Vec3) -> PlaceSnapshot {
        let point = bethesda_space::position_from_engine(eye);
        PlaceSnapshot {
            worldspace: worldspace.to_string(),
            interior_cell: interior_cell.map(str::to_string),
            grid: Self::camera_to_grid(eye),
            point_beth: (point.x, point.y),
        }
    }

    /// Bethesda world position (the `placed_objects` / XTEL coordinate space) to an
    /// engine-space position: `(x, z, -y) * SCALE`. The single source of truth for
    /// this mapping — shared by the per-object placement, the interior/teleport
    /// boot precompute, and the `OPENFO4_SPAWN` dev override.
    pub(crate) fn beth_to_engine(x: f32, y: f32, z: f32) -> Vec3 {
        bethesda_space::position_to_engine(x, y, z)
    }

    pub(crate) fn starting_position(&self) -> Vec3 {
        let x = self.starting_cell.0 as f32 * CELL_SIZE_WORLD + CELL_SIZE_WORLD * 0.5;
        let z = -(self.starting_cell.1 as f32 * CELL_SIZE_WORLD + CELL_SIZE_WORLD * 0.5);
        Vec3::new(x, 200.0, z)
    }

    pub(crate) fn initial_load(&mut self, camera_eye: Vec3) {
        self.current_cell = WorldStreamer::camera_to_grid(camera_eye);
        self.set_demand_camera(Some(camera_eye));
        info!(
            "Initial load at cell ({}, {})",
            self.current_cell.0, self.current_cell.1
        );
        self.begin_load();
    }

    /// Call on every frame the streamer update is skipped (menus), so the
    /// first update after the pause does not record the pause as a slow
    /// frame in `worst_frame_ms`.
    pub fn suspend_frame_meter(&mut self) {
        self.load_frames.suspend();
    }

    /// Arm the spawn-completion instrumentation (the timer + per-ring finish
    /// markers the `SPAWN_PROGRESS/SPAWN_COMPLETE` logging reads). Shared by every
    /// load entry point — ring load (`begin_load`) and single-cell interior load
    /// (`enter_interior`) — so the auto-drop back to Streaming fires for each.
    fn arm_spawn_timer(&mut self) {
        self.spawn_timer = Some(std::time::Instant::now());
        self.spawn_full_done = None;
        self.spawn_lod_done = None;
        self.full_ring_signal.arm();
        self.spawn_progress_tick = 0;
        self.load_frames.reset();
    }

    /// Arm the spawn-completion instrumentation and request the full + LOD
    /// rings around `current_cell`. Shared by `initial_load` (boot) and
    /// `switch_worldspace`; both set `current_cell` first.
    fn begin_load(&mut self) {
        self.arm_spawn_timer();

        // Requests go out nearest-first (see `nearest_first`). A spatially
        // interleaved order was tried and measured SLOWER cold — under the
        // narrow loader pool, adjacent in-flight cells are better, because one
        // shared load unblocks many cells at once.
        let grids = self.desired_cells();
        self.request_cells(&grids);
        let lod_grids = self.desired_lod_cells();
        self.request_lod_cells(&lod_grids);
    }

    /// Advance the worldspace generation. The plain `epoch` (compared on the
    /// main thread in `drain_worker_responses`) and the shared `epoch_arc` (read
    /// by workers) must always hold the same value; this is the only site that
    /// advances either, so they cannot drift.
    fn bump_epoch(&mut self) {
        self.epoch += 1;
        self.epoch_arc.store(self.epoch, Ordering::Release);
    }

    /// Tear down the current worldspace and begin streaming `worldspace`,
    /// starting at `start_cell` (precomputed per worldspace at boot, so no DB
    /// query runs on this frame). Releases every resident cell's GPU resources,
    /// drops the in-flight pipeline, and bumps the epoch so superseded worker
    /// requests/results are discarded. Returns the new player start position;
    /// the caller applies the queued collider removals (`take_sim_commands`) +
    /// `flush_colliders`, then moves the camera and teleports the player.
    ///
    /// The worker-side memo caches are intentionally NOT evicted: the
    /// per-worldspace maps (`persistent_by_grid`, `ws_editor_ids`, `tile_exists`)
    /// are keyed by worldspace, so the old entries are never read again (and are
    /// reused on a switch back), while the path-keyed caches are
    /// worldspace-independent. They grow by one entry per distinct worldspace
    /// visited — bounded, but a focused eviction pass is tracked separately.
    ///
    /// Safe to re-invoke for rapid re-selection: each call is a full teardown
    /// and reset, and the epoch bump invalidates anything still in flight. It
    /// is NOT an equality no-op — the caller is responsible for skipping a call
    /// whose target equals the current worldspace (a same-worldspace call would
    /// pointlessly tear the world down and teleport to its start).
    pub(crate) fn switch_worldspace(
        &mut self,
        worldspace: &str,
        start_cell: (i32, i32),
        targets: &mut StreamTargets,
    ) -> Vec3 {
        // Leaving any interior view: a worldspace switch is always a fresh
        // worldspace load, even back to the one whose name we still hold.
        self.interior_cell = None;
        self.teardown_resident(targets);

        self.worldspace = worldspace.to_string();
        self.starting_cell = start_cell;
        self.current_cell = start_cell;
        let start = self.starting_position();
        self.set_demand_camera(Some(start));

        // Fast-fill like boot unless the user pinned a mode; begin_load re-arms
        // the spawn-complete instrumentation so the auto-drop back to Streaming
        // fires for this load too.
        if !self.mode_pinned {
            self.apply_mode(LoadMode::Loading);
        }
        Self::spawn_persistent_prefetch(&self.db_path, &self.worker_shared, worldspace);
        self.begin_load();

        info!(
            "Worldspace -> {} (epoch {}), start cell ({}, {})",
            worldspace, self.epoch, start_cell.0, start_cell.1
        );

        self.starting_position()
    }

    /// Release every resident cell and supersede the in-flight pipeline — the
    /// teardown shared by `switch_worldspace` and `enter_interior`. Deregisters
    /// colliders (queued for the caller to apply *before* teleporting), frees
    /// every per-cell GPU buffer, resets the running tallies, clears the
    /// pending/buffered sets, bumps the epoch + purges the request channel so
    /// superseded work is dropped, and resets the GPU instance stores. Leaves
    /// `worldspace`/`interior_cell`/`current_cell` for the caller to set.
    fn teardown_resident(&mut self, targets: &mut StreamTargets) {
        self.shadow_invalidations.everything();
        with_grass_gpu(targets, GrassStage::clear_cells);
        self.demand_stale = true;
        // Deregister colliders before their payloads are freed. The caller
        // drains these (take_sim_commands) and applies them to the sim before
        // teleporting the player, so the ground-snap can't latch onto the old
        // content's terrain at an aliased grid.
        for grid in std::mem::take(&mut self.collision_registered) {
            self.sim_commands
                .push(SimCommand::RemoveCellColliders { grid });
        }
        // Per-cell refcounted release (see release_cell_buffers); the instance
        // stores are reset wholesale below, not freed per-cell — a full-world
        // teardown of thousands of cells would otherwise issue thousands of
        // slab frees (heap alloc + GPU zeroing write each), all discarded by
        // the same-frame repack.
        for res in std::mem::take(&mut self.retire_queue) {
            Self::release_cell_buffers(&res, targets);
        }
        for (_, res) in self.loaded.drain() {
            Self::release_cell_buffers(&res, targets);
        }
        self.mark_loaded_changed();
        self.nav.clear();
        for (_, res) in self.lod_loaded.drain() {
            Self::release_cell_buffers(&res, targets);
        }
        self.marker_instance_count = 0;
        self.fx_instance_count = 0;
        self.bucket_potential_total = [0; DRAW_BUCKET_COUNT];
        // The drained cells took their dynamic-instance maps with them; the sim
        // despawns the bodies via the RemoveCellColliders pushed above. Reset the
        // physics-center tracker so the new content's first ring sync re-pushes.
        self.entity_cell.clear();
        self.animated_events.clear();
        self.animated_events.push(AnimatedObjectEvent::Cleared);
        self.ragdoll_events.clear();
        self.ragdoll_events.push(RagdollEvent::Cleared);
        self.last_physics_center = ((i32::MIN, i32::MIN), i32::MIN);

        // Supersede the in-flight pipeline: bump the generation, then purge the
        // request channel in one shot so the superseded backlog can't pile up
        // (it would otherwise drain only one entry per worker pull — slow under
        // a pinned single-worker mode, and unbounded across rapid re-selection).
        // The per-pull epoch skip in db_worker stays as a cheap backstop for the
        // window between a worker's recv and this drain; results a worker is
        // mid-producing arrive stale and are dropped in drain_worker_responses.
        self.pending_cells.clear();
        self.lod_pending_cells.clear();
        self.missing_full.clear();
        self.missing_lod.clear();
        self.buffered_full.clear();
        self.buffered_lod.clear();
        self.bump_epoch();
        while self.req_rx.try_recv().is_ok() {}

        // Reset the GPU instance stores now that no cells are resident. The
        // object and placed-light slabs are reset by the unconditional repacks
        // in update_gpu_driven (needs_repack / needs_light_repack ->
        // reset_packed over the now-empty cells); the terrain
        // instance vec has no repack path, so clear+dirty it here — otherwise
        // the switch frame sees membership_changed == false and the terrain
        // cull keeps dispatching the old content's instances over freed mega
        // ranges until the first new cell integrates.
        self.needs_repack = true;
        self.needs_light_repack = true;
        self.rebuild_terrain_instances(targets.terrain_instances);
    }

    /// Tear down the current content and load a single interior cell (resolved
    /// by `cell_form_key`), keyed at `grid` — the interior's precomputed
    /// anchor grid. `current_cell` is parked there and the camera-driven ring
    /// is frozen (see `update_gpu_driven`), so the one cell stays resident and
    /// in the collision ring however the player moves. Returns the player start
    /// position (`start_world_pos`); the caller applies the queued collider
    /// removals + `flush_colliders`, enables sim interior mode, then teleports.
    pub(crate) fn enter_interior(
        &mut self,
        cell_form_key: &str,
        grid: (i32, i32),
        start_world_pos: Vec3,
        targets: &mut StreamTargets,
    ) -> Vec3 {
        self.teardown_resident(targets);
        self.interior_cell = Some(cell_form_key.to_string());
        self.set_demand_camera(None);
        self.starting_cell = grid;
        self.current_cell = grid;

        if !self.mode_pinned {
            self.apply_mode(LoadMode::Loading);
        }
        self.arm_spawn_timer();

        // One frozen cell, requested directly by form_key: no ring, no LOD, and
        // no worldspace-persistent prefetch (interiors have none).
        self.pending_cells.insert(grid);
        let _ = self.db_worker_tx.send(CellRequest::Interior {
            cell_form_key: cell_form_key.to_string(),
            grid,
            epoch: self.epoch,
        });

        info!(
            "Interior -> {} (epoch {}), start cell ({}, {})",
            cell_form_key, self.epoch, grid.0, grid.1
        );

        start_world_pos
    }

    /// Whether a single interior cell is loaded in place of a worldspace.
    pub(crate) fn in_interior(&self) -> bool {
        self.interior_cell.is_some()
    }

    /// The `form_key` of the loaded interior cell, if any — the single source of
    /// truth the debug dropdown reads to show its current selection.
    /// Every shadow-type lamp of the resident full-detail cells whose lights
    /// have a slab range.
    pub(crate) fn shadow_lamp_candidates(&mut self, out: &mut Vec<LampCandidate>) {
        if !std::mem::take(&mut self.lamp_candidates_dirty) {
            return;
        }
        let _span = info_span!("shadow_lamp_candidates").entered();
        out.clear();
        for res in self.loaded.values() {
            let Some((offset, _)) = res.light_range else {
                continue;
            };
            out.extend(res.shadow_lamps.iter().map(|source| LampCandidate {
                key: LampKey {
                    residency: res.residency,
                    light_index: source.light_index,
                },
                light: res.lights[source.light_index as usize],
                spec: source.spec,
                slab_index: offset + source.light_index,
            }));
        }
    }

    /// The caller dropped its candidate list; the next request must rebuild it.
    pub(crate) fn invalidate_lamp_candidates(&mut self) {
        self.lamp_candidates_dirty = true;
    }

    pub(crate) fn take_shadow_invalidations(&mut self) -> ShadowInvalidations {
        self.shadow_invalidations.take()
    }

    pub(crate) fn current_interior(&self) -> Option<&str> {
        self.interior_cell.as_deref()
    }

    /// Every resident water instance (full-detail cells only), read from the
    /// CPU mirror of the instance slab so offsets stay current across
    /// mega-buffer remaps.
    pub(crate) fn for_each_placed_water(&self, mut visit: impl FnMut(PlacedWater<'_>)) {
        for res in self.loaded.values() {
            for water in &res.water_surfaces {
                let instance = &res.object_groups[water.group].instances[water.instance];
                visit(PlacedWater {
                    transform: Mat4::from_cols_array_2d(&instance.transform),
                    aabb_min: Vec3::from(instance.aabb_min),
                    aabb_max: Vec3::from(instance.aabb_max),
                    index_offset: instance.index_offset,
                    index_count: instance.index_count,
                    vertex_offset: instance.vertex_offset,
                    water_type: water.water_type.as_deref(),
                    shader: water.shader,
                    volume: water.volume.as_ref(),
                });
            }
        }
    }

    /// Every effect-shader placement of the resident full-detail cells, for the effect stage.
    pub(crate) fn for_each_effect_instance(&self, mut visit: impl FnMut(&EffectInstance)) {
        for res in self.loaded.values() {
            res.effect_instances.iter().for_each(&mut visit);
        }
    }

    /// Every heat-haze placement of the resident full-detail cells, for the refraction stage.
    pub(crate) fn for_each_refraction_instance(&self, mut visit: impl FnMut(&RefractionInstance)) {
        for res in self.loaded.values() {
            res.refraction_instances.iter().for_each(&mut visit);
        }
    }

    /// Every addon node of the placed models of the resident full-detail cells, with the engine
    /// transform the addon attaches at. Attachments live and die with their cell.
    pub(crate) fn for_each_addon_attachment(&self, mut visit: impl FnMut(AddonAttachment<'_>)) {
        for res in self.loaded.values() {
            for (ordinal, attachment) in res.addon_attachments.iter().enumerate() {
                visit(AddonAttachment {
                    key: AttachmentKey {
                        residency: res.residency,
                        ordinal: ordinal as u32,
                    },
                    node_name: &attachment.node_name,
                    addon: &attachment.addon,
                    transform: attachment.transform,
                });
            }
        }
    }

    /// Every placed light of the resident full-detail cells as the GPU slab holds it
    /// (pre-animation colour), for the lights an effect draw is lit by.
    pub(crate) fn for_each_placed_light(&self, mut visit: impl FnMut(LightKey, &GpuPlacedLight)) {
        for res in self.loaded.values() {
            for (light_index, light) in res.lights.iter().enumerate() {
                visit(
                    LightKey {
                        residency: res.residency,
                        light_index: light_index as u32,
                    },
                    light,
                );
            }
        }
    }

    /// Every placed light of the resident full-detail cells whose record names a light model,
    /// which may animate it.
    pub(crate) fn for_each_model_light(&self, mut visit: impl FnMut(ModelLight<'_>)) {
        for res in self.loaded.values() {
            for source in &res.light_models {
                visit(ModelLight {
                    key: LightKey {
                        residency: res.residency,
                        light_index: source.light_index,
                    },
                    slab_index: res
                        .light_range
                        .map(|(offset, _)| offset + source.light_index),
                    base: &res.lights[source.light_index as usize],
                    model: &source.model,
                    frame: source.frame,
                });
            }
        }
    }

    /// Every placed model of the resident full-detail cells that carries particle systems of
    /// its own.
    pub(crate) fn for_each_particle_host(&self, mut visit: impl FnMut(ParticleHost<'_>)) {
        for res in self.loaded.values() {
            for (ordinal, host) in res.particle_hosts.iter().enumerate() {
                visit(ParticleHost {
                    key: AttachmentKey {
                        residency: res.residency,
                        ordinal: ordinal as u32,
                    },
                    model_path: &host.model_path,
                    transform: host.transform,
                });
            }
        }
    }

    /// The addon record a node index stands for.
    pub(crate) fn addon_node(
        &self,
        index: i32,
    ) -> Option<&crate::assets::db::addon_nodes::AddonNode> {
        self.worker_shared.addon_nodes.get(index)
    }

    /// Every resident full-detail cell with terrain: the cells FO4 would have
    /// loaded, which get their own water surface (`water_world::cell_water`).
    pub(crate) fn for_each_loaded_cell(&self, mut visit: impl FnMut(LoadedCell)) {
        for (&(x, y), res) in &self.loaded {
            if let Some(land_lowest_y) = res.land_lowest_y {
                visit(LoadedCell {
                    x,
                    y,
                    land_lowest_y,
                });
            }
        }
    }

    /// Resolve a worldspace's player start grid: the most-populated gridded
    /// cell, else (0,0). The fallback cases are distinguished, not lumped: an
    /// empty worldspace (no populated gridded cell) and a row with a null grid
    /// coordinate are expected and logged at info/warn, while a genuine query
    /// failure is surfaced as a warn rather than silently swallowed. Run at boot
    /// (the streamer's own start, and the debug dropdown's per-worldspace
    /// precompute) so a switch never pays this query on a render frame.
    pub(crate) fn resolve_start_cell(db: &DbClient, worldspace: &str) -> (i32, i32) {
        match db.get_populated_cell(worldspace) {
            Ok(c) => {
                if let (Some(x), Some(y)) = (c.grid_x, c.grid_y) {
                    (i32::from(x), i32::from(y))
                } else {
                    warn!(
                        "worldspace {worldspace}: populated cell has a null grid coordinate; starting at (0, 0)"
                    );
                    (0, 0)
                }
            }
            Err(e) => {
                if let Some(rusqlite::Error::QueryReturnedNoRows) =
                    e.downcast_ref::<rusqlite::Error>()
                {
                    info!(
                        "worldspace {worldspace} has no populated gridded cell; starting at (0, 0)"
                    );
                } else {
                    warn!("worldspace {worldspace}: start-cell query failed: {e}");
                }
                (0, 0)
            }
        }
    }

    /// Saves the collider-cache sidecar hash index off the main thread: the
    /// initial load has just hashed every sidecar its cells needed.
    fn flush_sidecar_hash_index_in_background(&self) {
        let shared = self.worker_shared.clone();
        let spawned = thread::Builder::new()
            .name("sidecar_hash_index_flush".into())
            .spawn(move || {
                crate::utils::lower_thread_priority();
                shared.bake_cache.flush_sidecar_index();
            });
        if let Err(e) = spawned {
            tracing::warn!("collision cache: can't start the index flush thread: {e}");
        }
    }

    /// Warm a worldspace's persistent REFRs on a background thread before the
    /// pool needs them (see `worldspace_persistents`). Opens its own read-only
    /// connection from the path; a spawn or open failure only forfeits the
    /// prefetch (the pool falls back to the lock-guarded lazy load).
    fn spawn_persistent_prefetch(db_path: &str, shared: &Arc<WorkerShared>, worldspace: &str) {
        let shared = shared.clone();
        let worldspace = worldspace.to_string();
        let db_path = db_path.to_string();
        let spawned = thread::Builder::new()
            .name("persistent_prefetch".into())
            .spawn(move || {
                crate::utils::lower_thread_priority();
                match DbClient::new(&db_path) {
                    Ok(db) => {
                        // Load failures are logged inside; the call only fills the shared cache.
                        drop(worldspace_persistents(&db, &shared, &worldspace));
                    }
                    Err(e) => warn!("persistent prefetch: failed to open DB: {e}"),
                }
            });
        if let Err(e) = spawned {
            warn!("persistent prefetch: failed to spawn thread: {e}");
        }
    }

    /// Take the collider commands accumulated so far this frame. The switch
    /// path uses this to apply its `RemoveCellColliders` to the sim *before*
    /// the player is teleported (see the caller); `update_gpu_driven` drains
    /// the same vec, so whatever is taken here must be applied by the caller.
    pub(crate) fn take_sim_commands(&mut self) -> Vec<SimCommand> {
        std::mem::take(&mut self.sim_commands)
    }

    pub(crate) fn loaded_cell_count(&self) -> usize {
        self.loaded.len()
    }

    pub(crate) fn pending_cell_count(&self) -> usize {
        self.pending_cells.len()
    }

    pub(crate) fn lod_loaded_cell_count(&self) -> usize {
        self.lod_loaded.len()
    }

    pub(crate) fn lod_pending_cell_count(&self) -> usize {
        self.lod_pending_cells.len()
    }

    /// (full, LOD) integration rates in cells/s, averaged over the last
    /// 0.5 s (see `SpawnRate`). `&mut` because reading prunes aged entries.
    pub(crate) fn spawn_rates(&mut self) -> (f32, f32) {
        (
            self.full_spawn_rate.cells_per_sec(),
            self.lod_spawn_rate.cells_per_sec(),
        )
    }

    pub(crate) fn marker_instance_count(&self) -> u32 {
        self.marker_instance_count
    }

    pub(crate) fn fx_instance_count(&self) -> u32 {
        self.fx_instance_count
    }

    /// Resident per-bucket potential — the hard upper bound used to size the
    /// object indirect-draw buffers (see [`potential_buckets`]).
    pub(crate) fn bucket_potential(&self) -> [u32; DRAW_BUCKET_COUNT] {
        self.bucket_potential_total
    }

    pub(crate) fn stream_radius_mut(&mut self) -> &mut i32 {
        &mut self.stream_radius
    }

    pub(crate) fn lod_radius_mut(&mut self) -> &mut i32 {
        &mut self.lod_radius
    }

    pub(crate) fn max_cells_per_frame_mut(&mut self) -> &mut usize {
        &mut self.max_cells_per_frame
    }

    pub(crate) fn physics_radius_mut(&mut self) -> &mut i32 {
        &mut self.physics_radius
    }

    pub(crate) fn mode(&self) -> LoadMode {
        self.mode
    }

    /// External (debug-menu) mode choice: pins the mode so `SPAWN_COMPLETE`
    /// won't auto-revert it.
    pub(crate) fn set_mode(&mut self, mode: LoadMode) {
        self.mode_pinned = true;
        if mode != self.mode {
            self.apply_mode(mode);
            info!("load mode -> {} (manual)", mode.label());
        }
    }

    /// Mode switches are safe mid-stream: workers check the gate between
    /// cells, and in-flight responses are gated/dropped by
    /// `integrate_full/integrate_lod` like any other stale result.
    fn apply_mode(&mut self, mode: LoadMode) {
        self.mode = mode;
        let active = match mode {
            LoadMode::Streaming => 1,
            LoadMode::Loading => self.pool_size,
        };
        self.active_workers.store(active, Ordering::Release);
    }

    pub(crate) fn pool_size(&self) -> usize {
        self.pool_size
    }

    pub(crate) fn active_worker_count(&self) -> usize {
        self.active_workers.load(Ordering::Relaxed)
    }

    fn effective_cells_per_frame(&self) -> usize {
        cells_per_frame(self.mode, self.max_cells_per_frame)
    }

    /// Per-frame integration upload cap for the current mode (see
    /// `UPLOAD_BUDGET_BYTES` / `LOADING_UPLOAD_BUDGET_BYTES`).
    pub(crate) fn upload_budget_limit(&self) -> u64 {
        match self.mode {
            LoadMode::Streaming => UPLOAD_BUDGET_BYTES,
            LoadMode::Loading => LOADING_UPLOAD_BUDGET_BYTES,
        }
    }
}

/// Engine-space rotation of a placed reference from its Bethesda Euler angles.
/// Shared by every placement (instances and lights) so they agree on the
/// convention.
pub(crate) fn placement_rotation(obj: &PlacedObject) -> Mat4 {
    Mat4::from_quat(bethesda_space::rotation_to_engine(
        obj.rotation_x as f32,
        obj.rotation_y as f32,
        obj.rotation_z as f32,
    ))
}

/// Rewrite `slab` as the tightly packed concatenation of every cell's
/// `contents` and point each cell's `range` at its part. Cells whose part did
/// not fit (device cap) lose their range so nothing points at unwritten slots.
/// Returns the uploaded element count.
fn repack_slab<T: SlabElement>(
    slab: &mut GpuSlab<T>,
    cells: &mut [&mut CellResources],
    total: usize,
    contents: impl Fn(&CellResources, &mut Vec<T>),
    range: fn(&mut CellResources) -> &mut Option<(u32, u32)>,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> u32 {
    let mut packed: Vec<T> = Vec::with_capacity(total);
    for cell in cells.iter_mut() {
        let start = packed.len() as u32;
        contents(cell, &mut packed);
        let count = packed.len() as u32 - start;
        *range(cell) = (count > 0).then_some((start, count));
    }
    let uploaded = slab.reset_packed(&packed, device, queue);
    if (uploaded as usize) < packed.len() {
        for cell in cells.iter_mut() {
            let cell_range = range(cell);
            if cell_range.is_some_and(|(start, count)| start + count > uploaded) {
                *cell_range = None;
            }
        }
    }
    uploaded
}

// Cache caps (all four caches are SharedCache instances, global across the
// worker pool — the same meshes appear in many cells, so per-worker copies
// would multiply both misses and memory).
//
// Mesh cap: sized for the unique-mesh working set of a full+LOD ring (~5k
// paths once SCOL combo meshes spawn). At 512 the cache thrashed: every ring
// shift re-parsed multi-MB SCOL GLBs (measured 25x per-cell parse cost in
// the 2026-06-11 fly soak).
const MESH_CACHE_CAP: u64 = 4096;

const COLLIDER_CACHE_CAP: u64 = 2048;

// Byte-bounded rather than entry-bounded because entries span ~100 KB to
// ~20 MB. Must hold the worker->integrate handoff window: when integration
// lags the workers, evicting a preloaded DDS before its cell integrates
// forces a synchronous main-thread re-read. 64 MB thrashed once SCOL
// textures joined the working set.
const DDS_CACHE_MAX_BYTES: u64 = 256 * 1024 * 1024;

/// Runs `f` with the grass stage and the GPU services its materials need, which
/// `StreamTargets` holds as separate fields.
fn with_grass_gpu<R>(
    targets: &mut StreamTargets,
    f: impl FnOnce(&mut GrassStage, &mut CharacterGpuContext) -> R,
) -> R {
    let mut gpu = CharacterGpuContext {
        device: targets.device,
        queue: targets.queue,
        texture_array: &mut *targets.texture_array,
        residency: &mut *targets.residency,
        material_registry: &mut *targets.material_registry,
        cube_array: &mut *targets.cube_array,
    };
    f(&mut *targets.grass, &mut gpu)
}

/// Everything the worker pool shares. Lookups are read-only after
/// construction; the caches are internally synchronized (`SharedCache`);
/// the small maps are Mutex'd one-shot/memoized lookups.
struct WorkerShared {
    base_object_lookup: HashMap<String, BaseObjectInfo>,
    /// Assembled weapons of the WEAP bases in `base_object_lookup`.
    weapons: WeaponAssemblies,
    /// LIGH `form_key` -> validated light parameters (lights whose row failed
    /// validation are absent and were logged at build time).
    light_lookup: HashMap<String, LightBase>,
    /// LIGH `form_key` -> light model path relative to the asset root.
    light_models: HashMap<String, Arc<str>>,
    /// ADDN rows by `BSValueNode` index.
    addon_nodes: AddonNodes,
    base_material_swaps: HashMap<String, String>,
    swap_entries: HashMap<String, Arc<Vec<SwapEntry>>>,
    ltex_to_diffuse: Arc<HashMap<String, String>>,
    grass: GrassWorkerData,
    grass_models: GrassModelCache,
    asset_root: String,
    /// Collision metadata for every mesh that has a `.col.json`, loaded once at
    /// startup (key = lowercased `meshes\<rel>.glb`). Cell assembly looks dynamic/
    /// mass up here in-RAM — no filesystem access — and absence means "no
    /// collision". The bulky shapes are read lazily from the loose sidecar only
    /// when a cold `.colbin` miss forces a bake.
    collision_manifest: HashMap<String, CollisionMeta>,
    /// `TextureArray`'s path mirror (see `TextureArray::resident_paths`):
    /// workers skip pre-reading DDS files that are already GPU-resident.
    resident_textures: Arc<Mutex<HashSet<String>>>,
    texture_policy: ResidencyPolicy,
    /// Where texture demand is measured from; the main thread publishes it
    /// every frame. Without a camera (interior) every texture loads at full
    /// resolution.
    demand_view: Mutex<DemandView>,
    /// Path -> DDS header. Present entries weigh one (LRU-bounded by
    /// `LAYOUT_CACHE_CAP`); `None` weighs nothing, is pinned,
    /// and records missing/unreadable files so they are probed once.
    layout_cache: SharedCache<String, Option<Arc<texture::TextureLayout>>>,
    /// Path -> parsed submeshes. Failed loads cache as empty vecs so broken
    /// meshes aren't re-parsed per cell.
    mesh_cache: SharedCache<String, Arc<PreparedModel>>,
    /// Mesh path -> parsed sidecar shapes; populated lazily, only when a cold
    /// `.colbin` miss forces a bake; `None` records missing/unreadable sidecars
    /// so absent files aren't re-probed.
    collider_cache: SharedCache<String, Option<SidecarData>>,
    /// (Texture path, first mip) -> the mips from there down; `None` records
    /// unreadable files — they weigh zero bytes and never evict.
    ///
    /// This complements (not replaces) the resident-set check: the resident
    /// set dedups against what's already on the GPU; this cache dedups
    /// across cells in flight together — a boundary crossing requests a
    /// whole row of cells sharing ground/rock textures, none resident yet.
    dds_cache: SharedCache<(String, u32), Option<Arc<TexturePayload>>>,
    bake_cache: crate::sim::bake::ShapeBakeCache,
    /// Baked terrain-LOD tile lookups, memoized for the pool's lifetime: one
    /// DB round-trip per worldspace and one fs metadata call per tile.
    ws_editor_ids: Mutex<HashMap<String, Option<String>>>,
    tile_exists: Mutex<HashMap<String, bool>>,
    /// Worldspace persistent REFRs (WRLD.TopCell) are unreachable through
    /// the grid query — loaded once per worldspace, bucketed by the grid
    /// cell their position falls in, so each streamed cell spawns its own
    /// share. The one-shot query runs INSIDE the lock on purpose: that IS
    /// the dedup (other workers genuinely need the data and wait for it).
    persistent_by_grid: Mutex<HashMap<String, Arc<PersistentBuckets>>>,
    /// Mode gate: worker `i` pulls requests only while `i < active_workers`.
    active_workers: Arc<AtomicUsize>,
    /// Current worldspace generation (see [`WorldStreamer::epoch`]). A worker
    /// discards a pulled request whose epoch no longer matches, draining the
    /// superseded backlog at pull cost instead of fully processing it.
    epoch: Arc<AtomicU64>,
}

/// The full-detail square and the LOD square around it, as plain arithmetic.
#[derive(Clone, Copy)]
struct Rings {
    center: (i32, i32),
    full_radius: i32,
    outer_radius: i32,
}

impl Rings {
    fn chebyshev(&self, grid: (i32, i32)) -> i32 {
        (grid.0 - self.center.0)
            .abs()
            .max((grid.1 - self.center.1).abs())
    }

    fn in_full(&self, grid: (i32, i32)) -> bool {
        self.chebyshev(grid) <= self.full_radius
    }

    fn in_lod(&self, grid: (i32, i32)) -> bool {
        let distance = self.chebyshev(grid);
        distance <= self.outer_radius && distance > self.full_radius
    }

    fn in_any(&self, grid: (i32, i32)) -> bool {
        self.chebyshev(grid) <= self.outer_radius
    }

    /// Every cell of the full-detail square.
    fn full_cells(&self) -> Vec<(i32, i32)> {
        let (cx, cy) = self.center;
        let radius = self.full_radius;
        (-radius..=radius)
            .flat_map(|dx| (-radius..=radius).map(move |dy| (cx + dx, cy + dy)))
            .collect()
    }

    /// Every cell of the LOD annulus around the full-detail square.
    fn lod_cells(&self) -> Vec<(i32, i32)> {
        let (cx, cy) = self.center;
        let outer = self.outer_radius;
        (-outer..=outer)
            .flat_map(|dx| (-outer..=outer).map(move |dy| (cx + dx, cy + dy)))
            .filter(|cell| self.in_lod(*cell))
            .collect()
    }

    /// Full-ring cells that were outside the full ring of `previous`.
    fn full_cells_entering(&self, previous: &Rings) -> Vec<(i32, i32)> {
        self.full_cells()
            .into_iter()
            .filter(|cell| !previous.in_full(*cell))
            .collect()
    }

    /// LOD-ring cells that were not in the LOD ring of `previous`, including
    /// cells that were full-detail before.
    fn lod_cells_entering(&self, previous: &Rings) -> Vec<(i32, i32)> {
        self.lod_cells()
            .into_iter()
            .filter(|cell| !previous.in_lod(*cell))
            .collect()
    }

    /// `grids` ordered nearest-first (Chebyshev ring, then Euclidean) around the center.
    fn nearest_first(&self, grids: &[(i32, i32)]) -> Vec<(i32, i32)> {
        let mut sorted = grids.to_vec();
        sorted.sort_by_key(|&(gx, gy)| {
            let (dx, dy) = (gx - self.center.0, gy - self.center.1);
            (dx.abs().max(dy.abs()), dx * dx + dy * dy)
        });
        sorted
    }
}

/// Cells integrated per frame: the configured cap, never below one while
/// streaming and never below the loading-screen floor while loading.
fn cells_per_frame(mode: LoadMode, configured: usize) -> usize {
    match mode {
        LoadMode::Streaming => configured.max(1),
        LoadMode::Loading => configured.max(LOADING_CELLS_PER_FRAME),
    }
}

/// Cells around the player that register static colliders: the vanilla-like
/// minimum, widened to wherever dynamic bodies are kept awake.
fn collision_ring_radius(collision_radius: i32, physics_radius: i32) -> i32 {
    collision_radius.max(physics_radius)
}

/// `VecDeque::retain` that frees the discarded results on the drop thread.
fn retain_deferred_drop(
    queue: &mut VecDeque<CellLoadResult>,
    keep: impl Fn(&CellLoadResult) -> bool,
) {
    let (kept, stale): (VecDeque<_>, VecDeque<_>) =
        std::mem::take(queue).into_iter().partition(|r| keep(r));
    *queue = kept;
    if !stale.is_empty() {
        crate::utils::drop_off_thread(stale);
    }
}

/// Mesh-cache loader: GLB -> prepared submeshes (parse + AABB + material
/// decode, once per path — never per placed instance or per cell). An animated
/// model yields one part per mesh node, vertices kept in the node's space.
fn parse_glb_submeshes(shared: &WorkerShared, path: &str) -> Arc<PreparedModel> {
    let full_path = format!("{}/{}", shared.asset_root, path);
    let model = match load_model(&full_path, path, is_havok_dynamic(shared, path)) {
        Ok(ModelAsset {
            model: LoadedModel::Static(meshes),
            extras,
        }) => PreparedModel {
            submeshes: meshes
                .into_iter()
                .enumerate()
                .map(|(index, mesh)| prepare_submesh(path, index, mesh, None))
                .collect(),
            extras,
        },
        Ok(ModelAsset {
            model: LoadedModel::Animated(model),
            extras,
        }) => PreparedModel {
            submeshes: prepare_animated_submeshes(path, model),
            extras,
        },
        Err(e) => {
            crate::utils::warn_once(path, || {
                format!(
                    "mesh '{full_path}' missing or failed to parse: {e}; refs using it render nothing"
                )
            });
            PreparedModel::of_submeshes(Vec::new())
        }
    };
    Arc::new(model)
}

fn is_havok_dynamic(shared: &WorkerShared, path: &str) -> bool {
    shared
        .collision_manifest
        .get(&manifest_key(path))
        .is_some_and(|meta| meta.dynamic)
}

/// An animated model whose animation data is unusable still renders, static.
/// A Havok-dynamic model is rigid-body driven, so it loads static as well: its
/// node transforms stay baked into the vertices.
fn load_model(
    full_path: &str,
    path: &str,
    havok_dynamic: bool,
) -> Result<ModelAsset, ModelLoadError> {
    if havok_dynamic {
        return load_static_model(full_path);
    }
    match LoadedModel::load(full_path) {
        Err(
            error @ (ModelLoadError::Import(_)
            | ModelLoadError::Rig(_)
            | ModelLoadError::PartWithoutJoint(_)),
        ) => {
            crate::utils::warn_once(path, || {
                format!("animated mesh '{full_path}' is unusable ({error}); rendering it static")
            });
            load_static_model(full_path)
        }
        other => other,
    }
}

fn load_static_model(full_path: &str) -> Result<ModelAsset, ModelLoadError> {
    let model = CpuMesh::load_model(full_path)?;
    Ok(ModelAsset {
        model: LoadedModel::Static(model.meshes),
        extras: model.extras,
    })
}

fn prepare_animated_submeshes(path: &str, model: AnimatedModel) -> Vec<PreparedSubMesh> {
    info!(
        "anim: model {path} clips={} parts={} roles={}",
        model.rig.asset.clips.len(),
        model.parts.len(),
        model.rig.roles().describe(&model.rig.asset.clips)
    );
    model
        .parts
        .into_iter()
        .enumerate()
        .map(|(index, part)| {
            let anim = PartAnim {
                rig: Arc::clone(&model.rig),
                joint: part.joint,
                rest_in_model: model.rig.rest_model()[part.joint as usize],
            };
            prepare_submesh(path, index, part.mesh, Some(anim))
        })
        .collect()
}

fn prepare_submesh(
    path: &str,
    index: usize,
    mesh: CpuMesh,
    anim: Option<PartAnim>,
) -> PreparedSubMesh {
    let local_aabb = Aabb::from_points(mesh.vertices.iter().map(|v| Vec3::from(v.position)));
    let mut material = SubMeshMaterial::parse(mesh.texture_path.as_deref());
    let ShapeClass { route, visibility } = classify(
        &ShapeFacts {
            is_refraction: material.is_refraction,
            is_effect: material.is_effect_shader,
            has_effect_key: material.has_effect_key,
            is_helper: material.is_helper_shape,
            is_water: material.water.is_some(),
            animated: anim.is_some(),
        },
        mesh.effect.clone(),
        mesh.refraction.clone(),
        || Arc::new(EffectMeshData::from_cpu_mesh(&mesh)),
    );
    material.object_visibility = visibility;
    material.gpu_key();
    // Without usable UVs the texture is assumed to span the mesh
    // once: detail follows the mesh's distance instead of an
    // infinite demand at any range.
    let world_per_uv = uv_density::world_per_uv(&mesh.vertices, &mesh.indices)
        .or_else(|| {
            local_aabb
                .as_ref()
                .map(|aabb| (aabb.max - aabb.min).max_element())
                .filter(|extent| extent.is_finite() && *extent > 0.0)
        })
        .unwrap_or(demand::NO_UV_DENSITY);
    PreparedSubMesh {
        mega_key: format!("{path}#{index}").into(),
        mesh,
        local_aabb,
        world_per_uv,
        material,
        anim,
        route,
    }
}

/// Loose-sidecar shape loader: `foo.glb` -> `{asset_root}/foo.col.json`. `None`
/// means no collider (missing or unparseable sidecar). Read only on a cold
/// `.colbin` miss.
fn load_collider_sidecar(asset_root: &str, path: &str) -> Option<SidecarData> {
    let stem = path.strip_suffix(".glb")?;
    load_sidecar_file(&format!("{asset_root}/{stem}.col.json"))
}

/// `foo.glb` -> `{asset_root}/foo.col.json`, the sidecar carrying the ragdoll bodies.
fn ragdoll_sidecar_path(asset_root: &str, path: &str) -> Option<String> {
    let stem = path.strip_suffix(".glb")?;
    Some(format!("{asset_root}/{stem}.col.json"))
}

/// Lazy shape provider for `bake_cell`: resolves a mesh's shapes through the
/// per-process collider cache (so repeated scales of one mesh read the sidecar
/// once). Called only on a cold `.colbin` miss; a warm launch never reaches here.
/// It is only ever called for a `path` the manifest already said has collision,
/// so an empty result here means the sidecar is missing/unreadable/degenerate
/// (manifest⇄disk drift) — warned once rather than silently dropping the
/// collider, since the static collider path is otherwise silent.
fn load_collision_shapes(shared: &WorkerShared, path: &str) -> Arc<Vec<PartShape>> {
    let key = [path.to_string()];
    let shapes = shared
        .collider_cache
        .resolve(
            &key,
            |p| load_collider_sidecar(&shared.asset_root, p),
            |_| 1,
        )
        .get(path)
        .cloned()
        .flatten()
        .map(|sd| sd.shapes)
        .unwrap_or_default();
    if shapes.is_empty() {
        crate::utils::warn_once(path, || {
            format!(
                "collision: manifest lists collision for {path} but its sidecar loaded no \
                 usable shapes (missing, unreadable, or all-degenerate); no collider"
            )
        });
    }
    bind_shapes_to_joints(shared, path, shapes)
}

fn bake_cell_colliders(
    shared: &WorkerShared,
    batch: &CellColliderBatch,
) -> crate::sim::bake::BakedCellColliders {
    shared.bake_cache.bake_cell(
        batch,
        |path| load_collision_shapes(shared, path),
        |path| animation_rig_for(shared, path).and_then(|rig| rig.moving_joint_fingerprint()),
        |path| ragdoll_sidecar_path(&shared.asset_root, path),
    )
}

/// The volume a water mesh's water fills, from its collision sidecar. `None`
/// (logged once) when the mesh has no usable collision: its water is drawn
/// but covers no position for height queries.
fn load_water_volume(shared: &WorkerShared, path: &str) -> Option<Arc<VolumeShapes>> {
    let volume = load_collider_sidecar(&shared.asset_root, path)
        .and_then(|sidecar| VolumeShapes::from_sidecar(&sidecar.shapes, path))
        .map(Arc::new);
    if volume.is_none() {
        crate::utils::warn_once(&format!("water-volume:{path}"), || {
            format!(
                "water mesh {path} has no usable collision sidecar; its water covers no position"
            )
        });
    }
    volume
}

fn animation_rig_for(shared: &WorkerShared, path: &str) -> Option<Arc<AnimationRig>> {
    shared
        .mesh_cache
        .resolve(
            &[path.to_string()],
            |p| parse_glb_submeshes(shared, p),
            |_| 1,
        )
        .get(path)
        .and_then(|submeshes| animation_rig_of(submeshes))
        .cloned()
}

fn animation_rig_of(submeshes: &[PreparedSubMesh]) -> Option<&Arc<AnimationRig>> {
    submeshes
        .iter()
        .find_map(|submesh| submesh.anim.as_ref())
        .map(|anim| &anim.rig)
}

fn is_animated_model(submeshes: &[PreparedSubMesh]) -> bool {
    animation_rig_of(submeshes).is_some()
}

/// A door/container/activator/furniture placement awaiting the check that its
/// model animates.
struct InteractablePlacement<'a> {
    mesh_path: String,
    form_key: &'a str,
    transform: Mat4,
    activatable: bool,
}

/// The placements whose model animates, as activatable objects. A Havok-dynamic
/// model loads static, so it never carries a rig and is not activatable.
fn animated_refs(
    interactables: Vec<InteractablePlacement<'_>>,
    resolved: &HashMap<String, Arc<PreparedModel>>,
) -> Vec<AnimatedRefr> {
    interactables
        .into_iter()
        .filter_map(|placement| {
            let rig = animation_rig_of(resolved.get(&placement.mesh_path)?)?;
            Some(AnimatedRefr {
                entity: refr_entity_id(placement.form_key),
                model_path: placement.mesh_path,
                form_key: placement.form_key.to_owned(),
                world: placement.transform,
                rig: Arc::clone(rig),
                activatable: placement.activatable,
            })
        })
        .filter(AnimatedRefr::is_runtime_object)
        .collect()
}

/// Resolve each shape's owning node to the animated joint it follows. Shapes of
/// a static model, shapes without node data and shapes on nodes no clip moves
/// stay static (joint `None`).
fn bind_shapes_to_joints(
    shared: &WorkerShared,
    path: &str,
    shapes: Arc<Vec<PartShape>>,
) -> Arc<Vec<PartShape>> {
    let Some(rig) = animation_rig_for(shared, path) else {
        return shapes;
    };
    if shapes.iter().all(|part| part.node.is_none()) {
        crate::utils::warn_once(&format!("collision-nodes:{path}"), || {
            format!(
                "collision: {path} is animated but its sidecar has no node data (old converter output); colliders stay static"
            )
        });
        return shapes;
    }
    Arc::new(
        shapes
            .iter()
            .map(|part| {
                let joint = part.node.as_deref().and_then(|node| if let Some(joint) = rig.joint_by_name(node) { rig.is_moving(joint).then_some(joint) } else {
                    crate::utils::warn_once(&format!("collision-node:{path}:{node}"), || {
                        format!("collision: {path} shape names node {node:?}, which the model lacks; shape stays static")
                    });
                    None
                });
                PartShape { joint, ..part.clone() }
            })
            .collect(),
    )
}

/// Worldspace persistent REFRs (WRLD.TopCell), loaded once per worldspace
/// and bucketed by grid cell. The one-shot query runs INSIDE the map lock on
/// purpose — that IS the dedup: every cell of the worldspace needs the data,
/// so callers that lose the race wait for it. At boot a prefetch thread
/// (spawned in `WorldStreamer::new`) runs this before the workers reach their
/// first cell; previously all 31 workers stacked up behind whichever one ran
/// the ~33k-row query at t=0.
fn worldspace_persistents(
    db: &DbClient,
    shared: &WorkerShared,
    worldspace: &str,
) -> Arc<HashMap<(i32, i32), Vec<PlacedObject>>> {
    shared
        .persistent_by_grid
        .lock()
        .unwrap()
        .entry(worldspace.to_string())
        .or_insert_with(|| match db.get_worldspace_persistent_objects(worldspace) {
            Ok(objs) => {
                let mut buckets: HashMap<(i32, i32), Vec<PlacedObject>> = HashMap::new();
                let total = objs.len();
                for obj in objs {
                    // Same frame as the grid coords: Bethesda cell = floor(pos / 4096).
                    let pgx = (obj.position_x / f64::from(CELL_SIZE_BETH)).floor() as i32;
                    let pgy = (obj.position_y / f64::from(CELL_SIZE_BETH)).floor() as i32;
                    buckets.entry((pgx, pgy)).or_default().push(obj);
                }
                info!(
                    "Loaded {} worldspace persistent refs across {} grid cells",
                    total,
                    buckets.len()
                );
                Arc::new(buckets)
            }
            Err(e) => {
                warn!("Failed to load worldspace persistent refs: {}", e);
                Arc::new(HashMap::new())
            }
        })
        .clone()
}

/// One worker-pool thread: pull cell requests while the mode gate admits
/// this worker's index, prepare each cell, ship the result. The whole pool
/// is spawned up front; Streaming mode parks all but worker 0. The gate
/// check sits between cells, so a mid-cell worker finishes (and sends)
/// before parking, and a parked worker costs a 25 ms poll, not a request.
#[allow(
    clippy::needless_pass_by_value,
    reason = "thread entry point: the worker owns its channel ends, DB connection and shared state for its lifetime"
)]
fn db_worker(
    index: usize,
    rx: crossbeam_channel::Receiver<CellRequest>,
    tx: mpsc::SyncSender<WorkerResponse>,
    db: DbClient,
    shared: Arc<WorkerShared>,
) {
    // Below-normal priority (like the cache loader pool): in loading mode
    // the pool may saturate every core, and the main thread must keep
    // winning the scheduler or integration throughput collapses.
    crate::utils::lower_thread_priority();
    loop {
        if index >= shared.active_workers.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(25));
            continue;
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(request) => {
                // The worldspace switched after this request was queued:
                // drop it at pull cost instead of paying its full prepare,
                // so the new worldspace's requests aren't stuck behind a
                // backlog of superseded ones.
                if request.epoch() != shared.epoch.load(Ordering::Acquire) {
                    continue;
                }
                let response = process_request(&request, &db, &shared);
                // Bounded channel: blocking here until the main thread
                // drains is the backpressure that keeps payload memory flat.
                if tx.send(response).is_err() {
                    break;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// Prepare one cell: `SQLite` queries, terrain CPU build, transform
/// resolution, GLB parse / sidecar load / shape bake / DDS pre-read — all
/// cached and deduped across the pool via `WorkerShared`. A pure function of
/// the request (plus caches): any worker can prepare any cell, and the main
/// thread's pending-set gating already tolerates arbitrary response order.
fn process_request(request: &CellRequest, db: &DbClient, shared: &WorkerShared) -> WorkerResponse {
    let (grid, is_lod, epoch) = match request {
        CellRequest::Full { gx, gy, epoch, .. } => ((*gx, *gy), false, *epoch),
        CellRequest::Lod { gx, gy, epoch, .. } => ((*gx, *gy), true, *epoch),
        CellRequest::Interior { grid, epoch, .. } => (*grid, false, *epoch),
    };
    let wrap = |result: CellLoadResult| {
        if is_lod {
            WorkerResponse::Lod(result)
        } else {
            WorkerResponse::Full(result)
        }
    };

    let _span = info_span!("db_query_cell", gx = grid.0, gy = grid.1, is_lod).entered();

    let mut result = CellLoadResult {
        grid,
        epoch,
        terrain: None,
        meshes: Vec::new(),
        grass: Vec::new(),
        grass_models: GrassModels::new(),
        lights: Vec::new(),
        shadow_lamps: Vec::new(),
        light_models: Vec::new(),
        addon_lights: 0,
        colliders: None,
        baked_colliders: None,
        animated_refs: Vec::new(),
        ragdoll_refs: Vec::new(),
        textures: HashMap::new(),
        navmeshes: Vec::new(),
    };

    match &request {
        // Exterior cell: resolved by grid + worldspace, with terrain and the
        // worldspace's persistent REFRs folded in.
        CellRequest::Full {
            gx, gy, worldspace, ..
        }
        | CellRequest::Lod {
            gx, gy, worldspace, ..
        } => {
            let (gx, gy, worldspace) = (*gx, *gy, worldspace.as_str());
            // Built alongside the render data for full cells, packed into
            // result.colliders before every send.
            let mut terrain_collider: Option<TerrainColliderSpec> = None;

            let cell = match db.get_cell(gx, gy, worldspace) {
                Ok(c) => c,
                Err(e) => {
                    warn!("Cell ({}, {}) not found: {}", gx, gy, e);
                    return wrap(result);
                }
            };

            let cell_form_key = if let Some(fk) = &cell.form_key {
                fk.clone()
            } else {
                warn!("Cell ({}, {}) has no form_key", gx, gy);
                return wrap(result);
            };

            if !is_lod {
                result.navmeshes = load_cell_navmeshes(db, &cell_form_key);
            }

            {
                let _span = info_span!("build_terrain").entered();
                if let Some(landscape) =
                    terrain_landscape(db.get_landscape_at(worldspace, gx, gy), gx, gy)
                {
                    let built = if is_lod {
                        terrain::build_cell_cpu_lod(&landscape, &shared.ltex_to_diffuse)
                    } else {
                        terrain::build_cell_cpu(&landscape, &shared.ltex_to_diffuse)
                    };
                    match built {
                        Ok(cpu) => {
                            if !is_lod {
                                // Copy positions before the cpu mesh moves on:
                                // the physics trimesh must match the render mesh
                                // exactly. (Baked-tile UV remaps don't touch
                                // positions, so copying here is safe.)
                                terrain_collider = Some(TerrainColliderSpec {
                                    vertices: cpu
                                        .vertices
                                        .iter()
                                        .map(|v| Vec3::from(v.position))
                                        .collect(),
                                    indices: cpu.indices.as_chunks::<3>().0.to_vec(),
                                });
                            }
                            result.terrain = Some(cpu);
                        }
                        Err(e) => warn!("Failed to build terrain for ({}, {}): {}", gx, gy, e),
                    }
                    if !is_lod {
                        result.grass =
                            shared
                                .grass
                                .place(db, worldspace, &cell_form_key, &landscape);
                    }
                }
            }
            result.grass_models = shared.grass_models.prepare(&result.grass);

            // Baked level-4 LOD tiles texture every LOD cell (vanilla distant
            // terrain, and no splat-layer cost) and any full-detail cell whose
            // LAND has no splat layers (unpainted border cells — magenta
            // otherwise). Decided here, not on the main thread: the UV remap and
            // the mode must be set atomically.
            if let Some(cpu) = result.terrain.as_mut() {
                let unpainted = cpu.texture_paths.iter().all(std::option::Option::is_none);
                if is_lod || unpainted {
                    let editor_id = shared
                        .ws_editor_ids
                        .lock()
                        .unwrap()
                        .entry(worldspace.to_string())
                        .or_insert_with(|| baked_tile_editor_id(db, worldspace))
                        .clone();
                    if let Some(eid) = editor_id {
                        let tx = terrain::baked_tile_coord(gx);
                        let ty = terrain::baked_tile_coord(gy);
                        // Single construction site: this string is also the
                        // texture cache/refcount key on the main thread.
                        let rel = format!("{TEXTURES_DIR}/Terrain/{eid}/{eid}.4.{tx}.{ty}.DDS");
                        let exists = *shared
                            .tile_exists
                            .lock()
                            .unwrap()
                            .entry(rel.clone())
                            .or_insert_with(|| {
                                std::path::Path::new(&format!("{}/{rel}", shared.asset_root))
                                    .is_file()
                            });
                        if exists {
                            terrain::apply_baked_tile_uvs(cpu, rel);
                        }
                    }
                }
            }

            let placed_objects = {
                let _span = info_span!("db_get_placed_objects").entered();
                match db.get_placed_objects(&cell_form_key) {
                    Ok(objs) => objs,
                    Err(e) => {
                        warn!("Failed to load placed objects for ({}, {}): {}", gx, gy, e);
                        if !is_lod {
                            // Ship the terrain collider anyway: a cell that lost
                            // its objects should still be walkable ground.
                            let batch = Arc::new(CellColliderBatch {
                                terrain: terrain_collider.take(),
                                objects: Vec::new(),
                            });
                            result.baked_colliders = Some(bake_cell_colliders(shared, &batch));
                            result.colliders = Some(batch);
                        }
                        return wrap(result);
                    }
                }
            };

            let persistent_buckets = worldspace_persistents(db, shared, worldspace);
            let persistent_objs: &[PlacedObject] = persistent_buckets
                .get(&(gx, gy))
                .map_or(&[], std::vec::Vec::as_slice);

            assemble_cell_payload(
                &mut result,
                placed_objects.iter().chain(persistent_objs.iter()),
                placed_objects.len() + persistent_objs.len(),
                terrain_collider,
                is_lod,
                shared,
            );
        }
        // Interior cell: resolved directly by form_key (no grid/worldspace).
        // No LAND terrain and no worldspace-persistent refs — the floor is the
        // placed object meshes themselves.
        CellRequest::Interior { cell_form_key, .. } => {
            let cell = match db.get_cell_by_form_key(cell_form_key) {
                Ok(c) => c,
                Err(e) => {
                    if let Some(rusqlite::Error::QueryReturnedNoRows) =
                        e.downcast_ref::<rusqlite::Error>()
                    {
                        info!("Interior cell {cell_form_key} not found");
                    } else {
                        warn!("Interior cell {cell_form_key} query failed: {e}");
                    }
                    return wrap(result);
                }
            };
            let cell_form_key = if let Some(fk) = &cell.form_key {
                fk.clone()
            } else {
                warn!("Interior cell row has no form_key");
                return wrap(result);
            };
            result.navmeshes = load_cell_navmeshes(db, &cell_form_key);
            let placed_objects = match db.get_placed_objects(&cell_form_key) {
                Ok(objs) => objs,
                Err(e) => {
                    warn!("Failed to load placed objects for interior {cell_form_key}: {e}");
                    return wrap(result);
                }
            };
            assemble_cell_payload(
                &mut result,
                placed_objects.iter(),
                placed_objects.len(),
                None,
                false,
                shared,
            );
        }
    }

    wrap(result)
}

/// The LAND a cell's terrain is built from. A cell without LAND anywhere in its worldspace chain has
/// no terrain; a failed lookup is logged and builds none either.
fn terrain_landscape(
    lookup: anyhow::Result<Option<Landscape>>,
    gx: i32,
    gy: i32,
) -> Option<Landscape> {
    lookup.unwrap_or_else(|error| {
        warn!("Failed to look up the landscape of ({gx}, {gy}): {error:#}");
        None
    })
}

/// The editor id naming the baked terrain tile folder of a worldspace: that of the worldspace whose
/// LOD data it uses, since a child worldspace has no tiles of its own. `None` is logged.
fn baked_tile_editor_id(db: &DbClient, worldspace: &str) -> Option<String> {
    let editor_id = db
        .lod_data_owner(worldspace)
        .and_then(|owner| db.get_worldspace(&owner))
        .map(|owner| owner.editor_id);
    editor_id.unwrap_or_else(|error| {
        warn!("No baked terrain tiles for worldspace {worldspace}: {error:#}");
        None
    })
}

/// Decodes and builds the navigation meshes of one cell on the worker. A cell whose query fails
/// or whose meshes are malformed loses those meshes only; each failure is logged with its form key.
fn load_cell_navmeshes(db: &DbClient, cell_form_key: &str) -> Vec<NavMesh> {
    let _span = info_span!("load_navmeshes").entered();
    let records = match db.get_navmeshes_for_cell(cell_form_key) {
        Ok(records) => records,
        Err(e) => {
            error!("Failed to load navmeshes for cell {cell_form_key}: {e}");
            return Vec::new();
        }
    };
    let outcome = build_meshes(&records);
    for failure in &outcome.failures {
        error!(
            "navmesh {} in cell {cell_form_key} skipped: {}",
            failure.form_key, failure.error
        );
    }
    outcome.meshes
}

/// The REFR's world matrix: position, rotation and uniform scale.
fn refr_world(obj: &PlacedObject) -> Mat4 {
    let position = bethesda_space::position_to_engine(
        obj.position_x as f32,
        obj.position_y as f32,
        obj.position_z as f32,
    );
    Mat4::from_translation(position)
        * placement_rotation(obj)
        * Mat4::from_scale(Vec3::splat(obj.scale as f32))
}

/// Expands a weapon REFR into one mesh group entry and one collision placement
/// per assembled part. All parts carry the REFR's pose, scale and entity id,
/// so they render and simulate as one object.
fn expand_weapon_refr(
    obj: &PlacedObject,
    weapon: &WeaponBase,
    info: &BaseObjectInfo,
    shared: &WorkerShared,
    sinks: &mut WeaponSinks,
) {
    if !sinks.claim(obj.form_key.as_deref()) {
        return;
    }
    let placed = shared
        .weapons
        .placed(weapon, &info.model_path, &shared.collision_manifest);
    if trace_enabled() {
        info!(
            "{}",
            placed.trace_line(obj.form_key.as_deref().unwrap_or("?"))
        );
    }
    if placed.skip.is_some() {
        return;
    }
    add_weapon_parts(
        &placed,
        &WeaponRefr {
            world: refr_world(obj),
            scale: obj.scale as f32,
            ref_key: obj.form_key.as_deref(),
        },
        sinks,
    );
}

/// Resolve placements -> render mesh groups + collider batch + preloaded DDS,
/// the half of cell preparation shared by exterior (grid + worldspace) and
/// interior (single `form_key`) loads. Pure transform/parse/bake/preload work:
/// the caller has already set `result.grid` and `result.terrain`; this fills
/// `meshes`, `colliders`, `baked_colliders`, and `textures`. `is_lod` selects
/// LOD mesh variants and suppresses collision (distant shells never collide);
/// `terrain_collider` is the caller's prebuilt terrain trimesh, packed into the
/// collision batch (always `None` for interiors and LOD).
fn assemble_cell_payload<'a>(
    result: &mut CellLoadResult,
    placements: impl Iterator<Item = &'a PlacedObject>,
    placement_count: usize,
    mut terrain_collider: Option<TerrainColliderSpec>,
    is_lod: bool,
    shared: &WorkerShared,
) {
    let (gx, gy) = result.grid;

    // Resolve placements to mesh paths, grouped so each path parses once.
    // Keyed by (path, is_marker, is_fx, swap): a mesh placed under more
    // than one classification must split into per-flag groups so only the
    // marker copies carry INSTANCE_FLAGS_MARKER (and only the FX copies
    // INSTANCE_FLAGS_HIDDEN_FX), and
    // material-swap variants split so each group binds its own textures
    // (geometry stays shared — mega-buffer slots are keyed by path).
    // Per key: the placement transforms plus the parallel per-transform entity
    // ids (used only if the mesh turns out Havok-dynamic; see the assembly
    // below). Combined into one map so the key string is hashed/owned once.
    let mut groups_by_key: HashMap<GroupKey, GroupInstances> = HashMap::new();
    // Collision placements per mesh path. Full cells only; marker REFRs
    // (and everything skipped before the transform is built) collide
    // exactly as much as they render: not at all.
    let mut placements_by_path: HashMap<String, Vec<Placement>> = HashMap::new();
    let mut expanded_weapon_refs: HashSet<String> = HashSet::new();
    // Full-detail placements of doors/containers/activators/furniture; the ones
    // whose model turns out animated become activatable objects.
    let mut interactables: Vec<InteractablePlacement<'a>> = Vec::new();
    {
        let _span = info_span!("resolve_transforms", count = placement_count).entered();
        for obj in placements {
            // Installer-resolved initial enable state (InitiallyDisabled flag
            // + XESP chains). Replaces the old per-field checks, which were
            // wrong in both directions: a non-opposite child of an enabled
            // parent is visible, an opposite child of an enabled parent
            // (the XESP->PlayerRef trick) is permanently hidden.
            if obj.initially_hidden {
                continue;
            }

            let Some(base_key) = &obj.base_object else {
                continue;
            };

            // Lights are never baked into precombined meshes, so they are
            // placed before the precombined skip, in full-detail cells only.
            if !is_lod && let Some(light) = shared.light_lookup.get(base_key) {
                let light_index = result.lights.len() as u32;
                if place_light(
                    light.place(obj),
                    base_key,
                    &mut result.lights,
                    &mut result.shadow_lamps,
                ) && let Some(model) = shared.light_models.get(base_key)
                {
                    result.light_models.push(LightModelRef {
                        light_index,
                        model: Arc::clone(model),
                        frame: Mat3::from_mat4(placement_rotation(obj)) * obj.scale as f32,
                    });
                }
            }

            if !is_lod && obj.is_in_precombined {
                continue;
            }

            // A light base without a model has no entry here: its ref is only
            // a light.
            let Some(info) = shared.base_object_lookup.get(base_key) else {
                continue;
            };

            if let Some(weapon) = &info.weapon {
                if !is_lod {
                    expand_weapon_refr(
                        obj,
                        weapon,
                        info,
                        shared,
                        &mut WeaponSinks {
                            placements_by_path: &mut placements_by_path,
                            groups_by_key: &mut groups_by_key,
                            expanded_refs: &mut expanded_weapon_refs,
                        },
                    );
                }
                continue;
            }

            // Effective material swap: the ref's XMSP overrides the base
            // record's MODS. Applies to LOD passes too — vanilla authors
            // explicit substitutions for LOD materials (94 entries: machine
            // kit colors, DecoKit brick walls, wet coast cliffs, Glowing
            // Sea rocks); LOD models without authored LOD entries match
            // nothing and render default, exactly like vanilla (all
            // billboards share one generic LOD ad). Swaps without resolved
            // entries don't split the instancing group.
            let swap = obj
                .material_swap
                .clone()
                .or_else(|| shared.base_material_swaps.get(base_key).cloned())
                .filter(|s| shared.swap_entries.contains_key(s));

            // Record flag 0x800000 "Is Marker" (set on ACTI/DOOR/FURN/STAT)
            // is the runtime hide signal — except Bethesda also sets it on
            // LOD-only statics (meshes under LOD\) to suppress their
            // full-detail render: those belong to the distant-LOD pass
            // only and are not toggleable markers. XPRM primitives and
            // filename heuristics are deliberately NOT used: vanilla
            // renders refs with primitives normally (occlusion volumes on
            // real geometry), and marker NIFs hide via their geometry
            // (EditorMarker shapes / untextured effect shaders), which is
            // handled at submesh level in build_mesh_group.
            let lp = info.model_path.to_ascii_lowercase();
            let is_lod_shell =
                info.is_marker && (lp.starts_with("lod\\") || lp.starts_with("lod/"));
            let is_marker = info.is_marker && !is_lod_shell;
            // Marker semantics win when both apply.
            let is_fx = info.is_fx && !is_marker;
            if is_lod {
                if is_marker {
                    // True markers have no LOD representation.
                    continue;
                }
            } else if is_lod_shell {
                continue;
            }

            let mesh_path = if is_lod {
                if let Some(lod_path) = &info.lod_model_path {
                    glb_key(lod_path)
                } else if info.has_distant_lod {
                    glb_key(&info.model_path)
                } else {
                    continue;
                }
            } else {
                glb_key(&info.model_path)
            };

            let scale = obj.scale as f32;
            let transform = refr_world(obj);

            if !is_lod && !is_marker {
                // Same matrix the renderer uses, decomposed: rapier poses
                // carry no scale, so it travels separately and is baked
                // into the shape data at collider build time.
                let (_, rotation, translation) = transform.to_scale_rotation_translation();
                placements_by_path
                    .entry(mesh_path.clone())
                    .or_default()
                    .push(Placement {
                        translation,
                        rotation,
                        scale,
                        ref_key: obj.form_key.clone(),
                        ragdoll_pose: obj.ragdoll_pose.as_deref().map(Arc::from),
                        part_local: None,
                    });
            }

            if let (false, Some(form_key)) = (is_lod, obj.form_key.as_deref()) {
                interactables.push(InteractablePlacement {
                    mesh_path: mesh_path.clone(),
                    form_key,
                    transform,
                    activatable: info.interactable,
                });
            }

            let entity = obj.form_key.as_deref().map(refr_entity_id);
            groups_by_key
                .entry((
                    mesh_path,
                    is_marker,
                    is_fx,
                    swap,
                    info.water_type.clone(),
                    false,
                ))
                .or_default()
                .push(transform, entity, None);
        }
    }

    {
        let _span = info_span!("parse_meshes", paths = groups_by_key.len()).entered();
        // Dedupe: a path can appear under several flag/swap keys but
        // must resolve only once.
        let paths: Vec<String> = groups_by_key
            .keys()
            .map(|(path, ..)| path)
            .collect::<HashSet<_>>()
            .into_iter()
            .cloned()
            .collect();
        let resolved =
            shared
                .mesh_cache
                .resolve(&paths, |path| parse_glb_submeshes(shared, path), |_| 1);

        // Mesh paths whose sidecar marks them Havok-dynamic; their render
        // instances become per-frame-updated rigid bodies. Built from the same
        // sidecar resolve as the collision batch below (LOD cells stay empty).
        let mut dynamic_paths: HashSet<String> = HashSet::new();
        // Mesh paths whose sidecar describes a ragdoll: their placements track
        // entities (row hiding, skinned replacement) like animated objects.
        let mut ragdoll_paths: HashSet<String> = HashSet::new();

        // Assemble the cell's collision batch from .col.json sidecars
        // only. No sidecar means the vanilla NIF has no Havok collision
        // (wires, vines, decals, FX) and the mesh is walk-through — a
        // render-AABB fallback box here turns ~28% of Commonwealth
        // placements into invisible walls.
        if !is_lod {
            let mut objects = Vec::new();
            if !placements_by_path.is_empty() {
                // Decide which meshes collide and which are dynamic from the
                // in-memory manifest — no filesystem access. A manifest entry
                // exists iff the mesh has a `.col.json` (the installer indexes
                // only meshes with collision); the shapes themselves load lazily
                // at bake time, only on a cold `.colbin` miss.
                let _load_span =
                    info_span!("load_collision", paths = placements_by_path.len()).entered();
                for (path, placements) in placements_by_path.drain() {
                    if let Some(meta) = shared.collision_manifest.get(&manifest_key(&path)) {
                        if meta.dynamic {
                            dynamic_paths.insert(path.clone());
                        }
                        let ragdoll_sidecar = meta
                            .ragdoll
                            .then(|| ragdoll_sidecar_path(&shared.asset_root, &path))
                            .flatten();
                        if meta.ragdoll {
                            ragdoll_paths.insert(path.clone());
                        }
                        objects.push(ObjectColliderGroup {
                            path,
                            placements,
                            dynamic: meta.dynamic,
                            mass: meta.mass,
                            ragdoll_sidecar,
                            water: meta.water.clone(),
                        });
                    }
                }
            }
            let batch = Arc::new(CellColliderBatch {
                terrain: terrain_collider.take(),
                objects,
            });
            {
                // Trimesh BVH builds happen here, off the main thread — add_cell
                // only instantiates colliders from the result. Shapes are read from
                // the loose `.col.json` only for (path, scale) pairs not already
                // baked to `.colbin` (cold); a warm launch reads no shapes at all.
                let _span = info_span!("bake_colliders").entered();
                result.baked_colliders = Some(bake_cell_colliders(shared, &batch));
            }
            result.colliders = Some(batch);
        }

        let mut swapped_groups = 0usize;
        for (
            (path, is_marker, is_fx, swap, water_type, weapon_part),
            GroupInstances {
                transforms,
                entities,
                locals,
            },
        ) in groups_by_key
        {
            let dynamic = weapon_part || dynamic_paths.contains(&path);
            if let Some(submeshes) = resolved.get(&path) {
                if submeshes.has_content() {
                    let ragdoll = ragdoll_paths.contains(&path);
                    let tracks_entities =
                        dynamic || ragdoll || (!is_lod && is_animated_model(submeshes));
                    let overrides = swap
                        .as_ref()
                        .and_then(|s| shared.swap_entries.get(s))
                        .map(|entries| compute_swap_overrides(submeshes, entries))
                        .unwrap_or_default();
                    if overrides.iter().any(std::option::Option::is_some) {
                        swapped_groups += 1;
                    }
                    if ragdoll {
                        let skin = ragdoll_skin_plan(
                            is_marker || is_fx,
                            swap.as_deref(),
                            submeshes,
                            &shared.swap_entries,
                        );
                        result
                            .ragdoll_refs
                            .extend(entities.iter().zip(&transforms).filter_map(
                                |(entity, world)| {
                                    Some(RagdollRefr {
                                        entity: (*entity)?,
                                        model_path: path.clone(),
                                        world: *world,
                                        skin: skin.clone(),
                                    })
                                },
                            ));
                    }
                    let water_volume = water_type
                        .as_ref()
                        .and_then(|_| load_water_volume(shared, &path));
                    if !is_lod && !is_marker {
                        result.addon_lights += place_addon_lights(
                            &AddonLightSources {
                                addon_nodes: &shared.addon_nodes,
                                light_lookup: &shared.light_lookup,
                                light_models: &shared.light_models,
                            },
                            &submeshes.extras,
                            &transforms,
                            &mut LightSink {
                                lights: &mut result.lights,
                                shadow_lamps: &mut result.shadow_lamps,
                                light_models: &mut result.light_models,
                            },
                        );
                    }
                    result.meshes.push(PreparedMeshGroup {
                        model_path: Arc::from(path.as_str()),
                        submeshes: submeshes.clone(),
                        transforms,
                        locals,
                        is_marker,
                        is_fx,
                        water_type,
                        water_volume,
                        overrides,
                        dynamic,
                        ragdoll,
                        entities: if tracks_entities {
                            entities
                        } else {
                            Vec::new()
                        },
                    });
                } else if is_marker {
                    // Marker NIFs whose shapes were all stripped at
                    // conversion (EditorMarker geometry: XMarkers, flag
                    // and comment markers) parse to empty GLBs.
                    // Substitute a gizmo box so the "Show markers"
                    // toggle has something to reveal.
                    result.meshes.push(PreparedMeshGroup {
                        model_path: Arc::from(""),
                        submeshes: marker_gizmo(),
                        transforms,
                        locals: Vec::new(),
                        is_marker: true,
                        is_fx: false,
                        water_type: None,
                        water_volume: None,
                        overrides: Vec::new(),
                        dynamic: false,
                        ragdoll: false,
                        entities: Vec::new(),
                    });
                }
            }
        }
        if swapped_groups > 0 {
            debug!("cell ({gx},{gy}): {swapped_groups} mesh groups material-swapped");
        }
        result.animated_refs = animated_refs(interactables, &resolved);
    }

    // Pre-read the mips of every texture the cell will acquire at
    // integration, so the main thread's acquire_texture is a pure GPU upload.
    // The keys built here must be byte-identical to the ones acquire_texture
    // is called with: submesh paths come from the same SubMeshMaterial::parse
    // output, the terrain layer key is pinned to its twin in
    // build_terrain_instance, and the baked tile string has a single
    // construction site above.
    preload_textures(result, shared);
}

/// Worker half of texture residency: read, for every texture the cell needs
/// and the GPU does not hold yet, exactly the mips its distance to the camera
/// calls for.
fn preload_textures(result: &mut CellLoadResult, shared: &WorkerShared) {
    let view = *shared.demand_view.lock().unwrap();

    let mut demands: HashMap<String, f32> = HashMap::new();
    let mut note = |path: &str, demand: f32| {
        demands
            .entry(path.to_string())
            .and_modify(|d| *d = d.max(demand))
            .or_insert(demand);
    };
    for group in &result.meshes {
        for (i, sub) in group.submeshes.iter().enumerate() {
            // Material-swap overrides replace texture paths; pre-read
            // exactly what build_mesh_group will acquire.
            let mat = group
                .overrides
                .get(i)
                .and_then(|o| o.as_ref())
                .unwrap_or(&sub.material);
            let demand = placements_demand(
                &view,
                sub.world_per_uv,
                group.transforms.iter().map(|placement| {
                    let transform = match &sub.anim {
                        Some(part) => *placement * part.rest_in_model,
                        None => *placement,
                    };
                    let (min, max) = world_aabb(sub.local_aabb.as_ref(), &transform);
                    (min, max, max_axis_scale(&transform.to_cols_array_2d()))
                }),
            );
            for (_, path) in mat.source().texture_paths() {
                if let Some(path) = path {
                    note(path, demand);
                }
            }
        }
    }
    if let Some(cpu) = &result.terrain {
        let (min, max) = cpu.world_bounds();
        let demand = view.pixels_per_uv(min, max, cpu.world_per_uv());
        if let Some(tile) = &cpu.baked_tile {
            note(tile, demand);
        } else {
            for path in cpu.texture_paths.iter().flatten() {
                note(&terrain_layer_texture_key(path), demand);
            }
        }
    }

    // One lock per cell: skip everything already GPU-resident (the main
    // thread will just bump its refcount; residency adjusts its resolution).
    let to_read: Vec<(String, f32)> = {
        let resident = shared.resident_textures.lock().unwrap();
        demands
            .into_iter()
            .filter(|(path, _)| !resident.contains(path))
            .collect()
    };
    let _span = info_span!("preload_textures", count = to_read.len()).entered();

    let paths: Vec<String> = to_read.iter().map(|(path, _)| path.clone()).collect();
    let layouts = shared.layout_cache.resolve(
        &paths,
        |path| {
            let full_path = format!("{}/{}", shared.asset_root, path);
            match texture::read_layout(&full_path) {
                Ok(layout) => Some(Arc::new(layout)),
                Err(e) => {
                    crate::utils::warn_once(&format!("texture-preload:{path}"), || {
                        format!("texture '{path}' unreadable: {e}")
                    });
                    None
                }
            }
        },
        |layout| u64::from(layout.is_some()),
    );

    let mut wanted: Vec<(String, u32)> = Vec::with_capacity(to_read.len());
    for (path, demand) in to_read {
        match layouts.get(&path).and_then(Option::as_ref) {
            Some(layout) => {
                let first = residency::initial_first_mip(shared.texture_policy, layout, demand);
                wanted.push((path, first));
            }
            None => {
                result.textures.insert(path, None);
            }
        }
    }

    let payloads = shared.dds_cache.resolve(
        &wanted,
        |(path, first)| {
            let layout = layouts.get(path).and_then(Option::as_ref)?;
            let full_path = format!("{}/{}", shared.asset_root, path);
            match texture::read_mips(&full_path, layout, *first..layout.mip_count) {
                Ok(payload) => Some(Arc::new(payload)),
                Err(e) => {
                    crate::utils::warn_once(&format!("texture-preload:{path}"), || {
                        format!("texture '{path}' unreadable: {e}")
                    });
                    None
                }
            }
        },
        |payload| payload.as_ref().map_or(0, |p| p.data.len() as u64),
    );
    for key in wanted {
        let payload = payloads.get(&key).cloned().flatten();
        result.textures.insert(key.0, payload);
    }
}

/// Shared stand-in for marker NIFs that convert to empty GLBs: a chest-high
/// box (3 world units; SCALE=0.1 ⇒ ~30 Bethesda units) that deliberately
/// renders with the magenta fallback material so spawned-but-invisible
/// markers are unmistakable when the "Show markers" toggle is on.
fn marker_gizmo() -> Arc<PreparedModel> {
    static GIZMO: std::sync::OnceLock<Arc<PreparedModel>> = std::sync::OnceLock::new();
    GIZMO
        .get_or_init(|| {
            const H: f32 = 1.5;
            let mut vertices = Vec::with_capacity(24);
            let mut indices = Vec::with_capacity(36);
            // Per face: normal plus two in-plane axes with u × v = n, so
            // every quad winds CCW seen from outside (front-face convention).
            let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
                ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
                ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
                ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
                ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
                ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
                ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
            ];
            for (normal, u, v) in faces {
                let (n, u, v) = (Vec3::from(normal), Vec3::from(u), Vec3::from(v));
                let base = vertices.len() as u32;
                for (su, sv) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    let p = (n + u * su + v * sv) * H;
                    vertices.push(Vertex {
                        position: p.to_array(),
                        tex_coords: [su * 0.5 + 0.5, sv * 0.5 + 0.5],
                        normal: n.to_array(),
                        tangent: [u.x, u.y, u.z, 1.0],
                    });
                }
                indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
            let mesh = CpuMesh {
                vertices,
                topology: crate::assets::types::mesh::Topology::Triangles,
                indices,
                texture_path: None,
                vertex_colors: None,
                effect: None,
                refraction: None,
            };
            let local_aabb =
                Aabb::from_points(mesh.vertices.iter().map(|v| Vec3::from(v.position)));
            let material = SubMeshMaterial::parse(None);
            material.gpu_key();
            let world_per_uv = uv_density::world_per_uv(&mesh.vertices, &mesh.indices)
                .unwrap_or(demand::NO_UV_DENSITY);
            Arc::new(PreparedModel::of_submeshes(vec![PreparedSubMesh {
                mega_key: "marker_gizmo#0".into(),
                mesh,
                local_aabb,
                world_per_uv,
                material,
                anim: None,
                route: ShapeRoute::Object,
            }]))
        })
        .clone()
}

/// Per-submesh swapped materials for one placement group: for each submesh
/// whose source BGSM (the GLB "src:" token) matches a substitution's
/// `original_material`, build a `SubMeshMaterial` with the installer-resolved
/// replacement textures and render flags. Both sides are normalized census
/// keys ("materials/.../foo.bgsm"), so matching is exact string equality.
/// Shader class doesn't change with a swap: helper/FX semantics stay with the
/// original shape.
fn compute_swap_overrides(
    submeshes: &[PreparedSubMesh],
    entries: &[SwapEntry],
) -> Vec<Option<SubMeshMaterial>> {
    submeshes
        .iter()
        .map(|sub| {
            let src = sub.material.source_material.as_deref()?;
            let entry = entries.iter().find(|e| e.original_material == src)?;
            let material = swapped_material(sub, entry);
            material.gpu_key();
            Some(material)
        })
        .collect()
}

/// How a ragdoll placement's skinned replacement is drawn. The static rows hide
/// marker geometry and apply the placement's material swap; a skinned
/// instance reproduces the swap by name (helper and water parts are handled
/// where the skinned mesh is uploaded) and cannot express an entry without one.
fn ragdoll_skin_plan(
    hidden_by_default: bool,
    swap: Option<&str>,
    submeshes: &[PreparedSubMesh],
    swap_entries: &HashMap<String, Arc<Vec<SwapEntry>>>,
) -> SkinPlan {
    if hidden_by_default {
        return SkinPlan::HiddenByDefault;
    }
    let mut overrides: Vec<MaterialOverride> = Vec::new();
    if let Some(entries) = swap.and_then(|swap| swap_entries.get(swap)) {
        for source in submeshes
            .iter()
            .filter_map(|sub| sub.material.source_material.as_deref())
        {
            let Some(entry) = entries
                .iter()
                .find(|entry| entry.original_material == source)
            else {
                continue;
            };
            let Some(replacement) = entry
                .material_name
                .as_deref()
                .filter(|name| MaterialName::parse(name).is_ok())
            else {
                return SkinPlan::Unsupported(format!(
                    "the material swap of {source} has no usable material name"
                ));
            };
            if overrides.iter().all(|existing| existing.original != source) {
                overrides.push(MaterialOverride {
                    original: source.to_owned(),
                    replacement: replacement.to_owned(),
                });
            }
        }
    }
    let swap = if overrides.is_empty() {
        None
    } else {
        swap.map(str::to_owned)
    };
    SkinPlan::Skin(PropLook {
        swap,
        overrides: overrides.into(),
    })
}

/// Replacement material of one swap entry. The entry's `material_name` (same
/// grammar as a GLB name) carries textures, alpha, lighting and effect/env
/// parameters; databases
/// converted before that column existed fall back to the entry's plain
/// texture and alpha fields with matte lighting. Shader class doesn't change
/// with a swap: decal/helper semantics and the source key stay with the
/// original shape.
fn swapped_material(original: &PreparedSubMesh, entry: &SwapEntry) -> SubMeshMaterial {
    let named = entry.material_name.as_deref().and_then(|name| match MaterialName::parse(name) {
        Ok(decoded) => Some(SubMeshMaterial::from_name(decoded)),
        Err(e) => {
            crate::utils::warn_once(name, || {
                format!("material swap name '{name}' is invalid: {e}; using the entry's plain fields")
            });
            None
        }
    });
    let replacement = named.unwrap_or_else(|| SubMeshMaterial {
        diffuse_path: entry.diffuse.as_deref().map(texture_key),
        normal_path: entry.normal.as_deref().map(texture_key),
        specular_path: None,
        glow_path: None,
        surface: LEGACY_SURFACE,
        alpha_cutoff: entry.alpha_test.map_or(0.0, |n| f32::from(n) / 255.0),
        alpha_mult: entry.alpha_mult,
        is_blend: entry.is_blend,
        two_sided: entry.two_sided,
        is_decal: false,
        is_helper_shape: false,
        effect: None,
        env: None,
        water: None,
        is_effect_shader: false,
        has_effect_key: false,
        is_refraction: false,
        object_visibility: ObjectVisibility::ByMaterialRule,
        source_material: None,
        gpu_key: std::sync::OnceLock::default(),
    });
    SubMeshMaterial {
        is_decal: original.material.is_decal,
        is_helper_shape: original.material.is_helper_shape,
        is_effect_shader: original.material.is_effect_shader,
        has_effect_key: original.material.has_effect_key,
        is_refraction: original.material.is_refraction,
        object_visibility: original.material.object_visibility,
        water: original.material.water,
        source_material: original.material.source_material.clone(),
        ..replacement
    }
}

/// Lights that fail to load leave placed lamps dark; like the material-swap
/// tables, that degrades the scene instead of blocking boot.
fn load_lights(db: &DbClient) -> Vec<Light> {
    db.get_lights().unwrap_or_else(|e| {
        warn!("light bases unavailable, placed lights will not render: {e}");
        Vec::new()
    })
}

fn build_light_models(lights: &[Light]) -> HashMap<String, Arc<str>> {
    lights
        .iter()
        .filter_map(|light| {
            let model = light
                .model_path
                .as_deref()
                .filter(|path| !path.is_empty())?;
            Some((light.form_key.clone()?, Arc::from(glb_key(model).as_str())))
        })
        .collect()
}

fn build_light_lookup(lights: &[Light], light_columns_present: bool) -> HashMap<String, LightBase> {
    let mut lookup = HashMap::new();
    let mut without_form_key = 0usize;
    let mut unusable = 0usize;
    for light in lights {
        let Some(form_key) = &light.form_key else {
            without_form_key += 1;
            continue;
        };
        match LightBase::from_row(light) {
            Ok(base) => {
                lookup.insert(form_key.clone(), base);
            }
            Err(
                e @ (LightBaseError::MissingRadius
                | LightBaseError::MissingColor
                | LightBaseError::MissingFlags),
            ) => {
                unusable += 1;
                warn!("light {form_key} has incomplete data and is dropped: {e}");
            }
            Err(
                e @ (LightBaseError::InvalidColor(_)
                | LightBaseError::FlagsOutOfRange(_)
                | LightBaseError::AmbiguousShadowType(_)),
            ) => {
                unusable += 1;
                warn!("light {form_key} has corrupt data and is dropped: {e}");
            }
        }
    }
    if without_form_key > 0 {
        warn!("{without_form_key} light rows have no form_key and are dropped");
    }
    info!(
        "Placed lights: {} light bases, {} usable, {} dropped; light-data columns {}",
        lights.len(),
        lookup.len(),
        unusable + without_form_key,
        if light_columns_present {
            "present"
        } else {
            "missing (attenuation, FOV and flicker use defaults)"
        },
    );
    lookup
}

fn instance_aabb(instance: &GpuInstance) -> Aabb {
    Aabb {
        min: Vec3::from(instance.aabb_min),
        max: Vec3::from(instance.aabb_max),
    }
}

fn caster_bounds(res: &CellResources) -> Option<Aabb> {
    let objects = res
        .object_groups
        .iter()
        .flat_map(|group| &group.instances)
        .map(instance_aabb);
    let terrain = res.terrain.iter().map(|terrain| Aabb {
        min: Vec3::from(terrain.aabb_min),
        max: Vec3::from(terrain.aabb_max),
    });
    Aabb::from_points(objects.chain(terrain).flat_map(|aabb| [aabb.min, aabb.max]))
}

/// Append a reference's light to the cell payload. Each failure kind is
/// reported once per light base, not once per reference.
fn place_light(
    placement: Result<LightPlacement, PlacedLightError>,
    base_key: &str,
    lights: &mut Vec<GpuPlacedLight>,
    shadow_lamps: &mut Vec<ShadowLampSource>,
) -> bool {
    match placement {
        Ok(LightPlacement::Lit(PlacedLight { gpu, shadow })) => {
            if let Some(spec) = shadow {
                shadow_lamps.push(ShadowLampSource {
                    light_index: lights.len() as u32,
                    spec,
                });
            }
            lights.push(gpu);
            return true;
        }
        Ok(LightPlacement::FadedOut) => {
            crate::utils::info_once(&format!("placed_light_faded_out:{base_key}"), || {
                format!("placed light of {base_key} skipped: combined fade 0 switches it off")
            });
        }
        Err(e @ PlacedLightError::NonPositiveRadius(_)) => {
            crate::utils::warn_once(&format!("placed_light_radius:{base_key}"), || {
                format!("placed light of {base_key} dropped: {e}")
            });
        }
        Err(e @ PlacedLightError::InvalidFov(_)) => {
            crate::utils::warn_once(&format!("placed_light_fov:{base_key}"), || {
                format!("placed light of {base_key} dropped: {e}")
            });
        }
        Err(e @ PlacedLightError::NonFiniteFade(_)) => {
            crate::utils::warn_once(&format!("placed_light_fade:{base_key}"), || {
                format!("placed light of {base_key} dropped: {e}")
            });
        }
        Err(e @ PlacedLightError::InvalidNearClip(_)) => {
            crate::utils::warn_once(&format!("placed_light_near_clip:{base_key}"), || {
                format!("placed light of {base_key} dropped: {e}")
            });
        }
        Err(e @ PlacedLightError::NearClipBeyondRadius { .. }) => {
            crate::utils::warn_once(&format!("placed_light_near_clip_radius:{base_key}"), || {
                format!("placed light of {base_key} dropped: {e}")
            });
        }
    }
    false
}

/// A failed table read leaves that category of base objects unplaceable; say so
/// instead of silently rendering a world without it.
fn rows_or_warn<T>(query: &str, rows: anyhow::Result<Vec<T>>) -> Vec<T> {
    rows.unwrap_or_else(|e| {
        warn!("{query} failed, its objects will not be placed: {e:#}");
        Vec::new()
    })
}

/// The base-object lookup, and the mod rows the weapon assemblies read.
fn build_base_object_lookup(
    db: &DbClient,
    lights: &[Light],
) -> (HashMap<String, BaseObjectInfo>, HashMap<String, ModRow>) {
    let mut map = HashMap::new();
    let mut water_activators: Vec<(String, Arc<str>)> = Vec::new();
    // `fx_filename`: whether the fx-filename half of `is_fx_model` applies
    // (movable statics only).
    let mut insert = |fk: &Option<String>,
                      mp: &Option<String>,
                      marker: bool,
                      lod: Option<String>,
                      distant: bool,
                      fx_filename: bool,
                      interactable: bool| {
        if let (Some(fk), Some(mp)) = (fk, mp) {
            map.insert(
                fk.clone(),
                BaseObjectInfo {
                    model_path: mp.clone(),
                    is_marker: marker,
                    is_fx: is_fx_model(mp, fx_filename),
                    lod_model_path: lod,
                    has_distant_lod: distant,
                    interactable,
                    water_type: None,
                    weapon: None,
                },
            );
        }
    };
    for s in rows_or_warn("get_statics", db.get_statics()) {
        insert(
            &s.form_key,
            &s.model_path,
            s.is_marker,
            s.lod_model_path.clone(),
            s.has_distant_lod,
            false,
            false,
        );
    }
    for s in rows_or_warn("get_furniture", db.get_furniture()) {
        insert(
            &s.form_key,
            &s.model_path,
            s.is_marker,
            None,
            false,
            false,
            true,
        );
    }
    for s in rows_or_warn("get_doors", db.get_doors()) {
        insert(
            &s.form_key,
            &s.model_path,
            s.is_marker,
            None,
            false,
            false,
            true,
        );
    }
    for s in rows_or_warn("get_containers", db.get_containers()) {
        insert(
            &s.form_key,
            &s.model_path,
            s.is_marker,
            None,
            false,
            false,
            true,
        );
    }
    for s in rows_or_warn("get_movable_statics", db.get_movable_statics()) {
        insert(
            &s.form_key,
            &s.model_path,
            s.is_marker,
            None,
            false,
            true,
            false,
        );
    }
    for s in rows_or_warn("get_activators", db.get_activators()) {
        insert(
            &s.form_key,
            &s.model_path,
            s.is_marker,
            None,
            false,
            false,
            true,
        );
        if let (Some(form_key), Some(water_type)) = (&s.form_key, &s.water_type) {
            water_activators.push((form_key.clone(), Arc::from(water_type.as_str())));
        }
    }
    for s in rows_or_warn("get_flora", db.get_flora()) {
        insert(
            &s.form_key,
            &s.model_path,
            s.is_marker,
            None,
            false,
            false,
            false,
        );
    }
    for s in rows_or_warn("get_trees", db.get_trees()) {
        insert(
            &s.form_key,
            &s.model_path,
            s.is_marker,
            None,
            false,
            false,
            false,
        );
    }
    // Light bases with a model (lamp fixtures, glow meshes, effect cards)
    // render it like any other object; their light comes from `light_lookup`.
    for l in lights {
        insert(
            &l.form_key,
            &l.model_path,
            l.is_marker,
            None,
            false,
            false,
            false,
        );
    }
    // SCOL merged combo meshes (building facades, street assemblies, tree
    // clusters). No per-SCOL LOD model exists, so they only render inside the
    // full-detail ring.
    for s in rows_or_warn("get_static_collections", db.get_static_collections()) {
        insert(
            &s.form_key,
            &s.model_path,
            s.is_marker,
            None,
            false,
            false,
            false,
        );
    }
    for (form_key, water_type) in water_activators {
        if let Some(info) = map.get_mut(&form_key) {
            info.water_type = Some(water_type);
        }
    }
    let weapon_mods = add_weapon_bases(db, &mut map);
    (map, weapon_mods)
}

/// Registers every weapon base that has a model. Returns the mod rows its
/// default combination needs; both stay empty for a database converted before
/// weapon assembly, whose placed weapons then stay undrawn (the schema logs it).
fn add_weapon_bases(
    db: &DbClient,
    map: &mut HashMap<String, BaseObjectInfo>,
) -> HashMap<String, ModRow> {
    let rows = match db.get_weapons() {
        Ok(Some(rows)) => rows,
        Ok(None) => return HashMap::new(),
        Err(e) => {
            warn!("get_weapons failed, placed weapons will not be drawn: {e:#}");
            return HashMap::new();
        }
    };
    let records = WeaponRecords::load(rows, |keys| match db.get_mod_models(keys) {
        Ok(rows) => rows.unwrap_or_default(),
        Err(e) => {
            warn!("get_mod_models failed, weapon mods will not be drawn: {e:#}");
            Vec::new()
        }
    });
    for (base, model_path) in records.bases {
        map.insert(
            base.form_key.clone(),
            BaseObjectInfo {
                model_path,
                is_marker: false,
                is_fx: false,
                lod_model_path: None,
                has_distant_lod: false,
                interactable: false,
                water_type: None,
                weapon: Some(Arc::new(base)),
            },
        );
    }
    records.mods
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact sentinels and copied values"
)]
mod tests {
    use std::collections::HashSet;

    use bytemuck::Zeroable;

    use super::*;

    fn landscape() -> Landscape {
        Landscape {
            form_key: None,
            editor_id: None,
            cell: "cell".to_owned(),
            grid_x: 1,
            grid_y: 2,
            vertex_height_map: Vec::new(),
            vertex_colors: None,
            layers: None,
            flags: None,
        }
    }

    #[test]
    fn terrain_is_built_only_from_a_landscape_that_was_found() {
        assert!(terrain_landscape(Ok(Some(landscape())), 1, 2).is_some());
        assert!(terrain_landscape(Ok(None), 1, 2).is_none());
        assert!(terrain_landscape(Err(anyhow::anyhow!("db down")), 1, 2).is_none());
    }

    #[test]
    fn the_music_place_agrees_with_the_grid_cell_and_converts_the_point_to_bethesda_units() {
        let eye = Vec3::new(
            3.0 * CELL_SIZE_WORLD + 12.3,
            50.0,
            2.0 * CELL_SIZE_WORLD - 45.6,
        );

        let place = WorldStreamer::place_snapshot("00003C:Fallout4.esm", None, eye);

        assert_eq!(place.grid, (3, -2));
        assert!((place.point_beth.0 - (3.0 * CELL_SIZE_BETH + 123.0)).abs() < 0.01);
        assert!((place.point_beth.1 - (-2.0 * CELL_SIZE_BETH + 456.0)).abs() < 0.01);
        assert_eq!(
            (
                (place.point_beth.0 / CELL_SIZE_BETH).floor() as i32,
                (place.point_beth.1 / CELL_SIZE_BETH).floor() as i32
            ),
            place.grid
        );
        assert_eq!(place.interior_cell, None);
    }

    #[test]
    fn the_music_place_resolves_to_the_region_whose_polygon_holds_the_eye() {
        use crate::assets::db::music::{RegionArea, RegionMusic};
        use crate::audio::music_stack::PlaceSource;
        use crate::audio::place::PlaceIndex;
        use crate::audio::test_support::catalog::{music_type, world_catalog};

        const WORLDSPACE: &str = "00003C:Fallout4.esm";
        let eye = Vec3::new(
            3.0 * CELL_SIZE_WORLD + 12.3,
            50.0,
            2.0 * CELL_SIZE_WORLD - 45.6,
        );
        let (centre_x, centre_y) = (3.0 * CELL_SIZE_BETH + 123.0, -2.0 * CELL_SIZE_BETH + 456.0);
        let mut catalog = world_catalog(
            &[(WORLDSPACE, "MUSzExplore")],
            vec![
                music_type("MUSzExplore", 0, &[]),
                music_type("MUSzPublic", 0, &[]),
            ],
            vec![],
        );
        catalog.region_music.push(RegionMusic {
            region: "R1".into(),
            worldspace: Some(WORLDSPACE.into()),
            music_type: "MUSzPublic".into(),
            priority: 55,
            override_flag: false,
            record_order: 0,
        });
        catalog.region_areas.push(RegionArea {
            region: "R1".into(),
            area: 0,
            points: vec![
                (centre_x - 100.0, centre_y - 100.0),
                (centre_x + 100.0, centre_y - 100.0),
                (centre_x + 100.0, centre_y + 100.0),
                (centre_x - 100.0, centre_y + 100.0),
            ],
        });
        let index = PlaceIndex::from_catalog(&catalog).unwrap();

        let inside = index
            .resolve(&WorldStreamer::place_snapshot(WORLDSPACE, None, eye))
            .unwrap();
        let moved_eye = eye + Vec3::new(CELL_SIZE_WORLD, 0.0, 0.0);
        let outside = index
            .resolve(&WorldStreamer::place_snapshot(WORLDSPACE, None, moved_eye))
            .unwrap();

        assert_eq!(
            (inside[0].music_type.as_str(), inside[0].source),
            ("MUSzPublic", PlaceSource::Region)
        );
        assert_eq!(outside.len(), 1);
        assert_eq!(outside[0].source, PlaceSource::Worldspace);
    }

    #[test]
    fn the_music_place_of_an_interior_names_its_cell() {
        let place = WorldStreamer::place_snapshot("w", Some("0016D8:Fallout4.esm"), Vec3::ZERO);
        assert_eq!(place.interior_cell.as_deref(), Some("0016D8:Fallout4.esm"));
    }

    fn material_flags(name: &str) -> u32 {
        material_instance_flags(&SubMeshMaterial::parse(Some(name)))
    }

    #[test]
    fn plain_opaque_and_alpha_tested_materials_carry_no_blend_flags() {
        assert_eq!(
            blend_flags(&SubMeshMaterial::parse(Some("a_d.dds|a_n.dds"))),
            0
        );
        assert_eq!(
            blend_flags(&SubMeshMaterial::parse(Some("a_d.dds|a_n.dds|test:128"))),
            0
        );
    }

    #[test]
    fn alpha_and_additive_blends_route_as_plain_blend() {
        assert_eq!(
            blend_flags(&SubMeshMaterial::parse(Some("a_d.dds|a_n.dds|blend"))),
            INSTANCE_FLAGS_BLEND
        );
        assert_eq!(
            blend_flags(&SubMeshMaterial::parse(Some(
                "a_d.dds||blend|bm:add|bc:FFFFFF|bs:1"
            ))),
            INSTANCE_FLAGS_BLEND
        );
    }

    #[test]
    fn multiplicative_blend_sets_the_multiply_flag_with_blend() {
        assert_eq!(
            blend_flags(&SubMeshMaterial::parse(Some(
                "a_d.dds||blend|bm:mul|bc:FFFFFF|bs:1"
            ))),
            INSTANCE_FLAGS_BLEND | INSTANCE_FLAGS_MULTIPLY
        );
    }

    #[test]
    fn multiply_mode_without_blend_does_not_route_as_blend() {
        assert_eq!(
            blend_flags(&SubMeshMaterial::parse(Some(
                "a_d.dds||bm:mul|bc:FFFFFF|bs:1"
            ))),
            0
        );
    }

    #[test]
    fn every_effect_material_is_flagged_so_it_casts_no_shadow() {
        for name in [
            "a_d.dds||blend|fx|bc:FFFFFF|bs:1",
            "a_d.dds||blend|bm:add|bc:FFFFFF|bs:1",
            "a_d.dds||bc:FFFFFF|bs:1",
        ] {
            assert_ne!(material_flags(name) & INSTANCE_FLAGS_EFFECT, 0, "{name}");
        }
    }

    #[test]
    fn lit_materials_are_not_flagged_as_effects() {
        assert_eq!(
            material_flags("a_d.dds|a_n.dds|blend|ts") & INSTANCE_FLAGS_EFFECT,
            0
        );
        assert_eq!(material_flags("a_d.dds|a_n.dds") & INSTANCE_FLAGS_EFFECT, 0);
    }

    fn visibility_of(name: &str, placement_is_marker: bool, placement_is_fx: bool) -> u32 {
        visibility_flag(
            placement_is_marker,
            placement_is_fx,
            &SubMeshMaterial::parse(Some(name)),
        )
    }

    #[test]
    fn visibility_class_truth_table() {
        const BGEM_GLASS: &str = "||blend|0.68|fx|bc:000000|bs:1|src:materials\\g.bgem";
        const INLINE_EFFECT: &str = "a_d.dds||blend|fx|bc:FFFFFF|bs:1";
        const TEXTURELESS_INLINE_EFFECT: &str = "||blend|fx|bc:FFFFFF|bs:1";
        const LEGACY_FX: &str = "a_d.dds|a_n.dds|blend|fx";
        const WATER: &str = "w_d.dds|w_n.dds|fx|water:196:80000008:00000001:0,0:1,1";
        const LIT: &str = "a_d.dds|a_n.dds";
        const HELPER: &str = "||blend";
        let cases = [
            ("bgem effect", BGEM_GLASS, false, false, 0),
            (
                "inline effect",
                INLINE_EFFECT,
                false,
                false,
                INSTANCE_FLAGS_HIDDEN_FX,
            ),
            (
                "textureless inline effect",
                TEXTURELESS_INLINE_EFFECT,
                false,
                false,
                INSTANCE_FLAGS_HIDDEN_FX,
            ),
            (
                "legacy textured fx",
                LEGACY_FX,
                false,
                false,
                INSTANCE_FLAGS_HIDDEN_FX,
            ),
            ("water", WATER, false, false, 0),
            ("lit geometry", LIT, false, false, 0),
            (
                "fx model hides lit geometry",
                LIT,
                false,
                true,
                INSTANCE_FLAGS_HIDDEN_FX,
            ),
            (
                "fx model hides bgem effect",
                BGEM_GLASS,
                false,
                true,
                INSTANCE_FLAGS_HIDDEN_FX,
            ),
            ("helper volume", HELPER, false, false, INSTANCE_FLAGS_MARKER),
            ("marker placement", LIT, true, false, INSTANCE_FLAGS_MARKER),
            (
                "marker wins over fx",
                INLINE_EFFECT,
                true,
                true,
                INSTANCE_FLAGS_MARKER,
            ),
        ];
        for (label, name, is_marker, is_fx, expected) in cases {
            assert_eq!(visibility_of(name, is_marker, is_fx), expected, "{label}");
        }
    }

    fn cpu_mesh(material_name: &str) -> CpuMesh {
        CpuMesh {
            vertices: Vec::new(),
            topology: crate::mesh::Topology::default(),
            indices: Vec::new(),
            texture_path: Some(material_name.to_string()),
            vertex_colors: None,
            effect: None,
            refraction: None,
        }
    }

    fn refraction_extras() -> Arc<crate::assets::types::effect_material::RefractionMaterial> {
        Arc::new(
            serde_json::from_value(serde_json::json!({
                "v": 1, "strength": 1.0, "normal_a": "a.dds", "normal_b": "b.dds",
                "uv_offset": [0.0, 0.0], "uv_scale": [1.0, 1.0],
                "vertex_colors": true, "double_sided": true
            }))
            .unwrap(),
        )
    }

    const FIRE_EFFECT_NAME: &str = "a_d.dds||blend|fx|eff:0000000a";
    const LEFTOVER_INLINE_EFFECT_NAME: &str = "a_d.dds||blend|fx|bc:FFFFFF|bs:1";
    const REFRACTION_NAME: &str = "a_n.dds|a_n.dds|refr|eff:0000000b";

    #[test]
    fn a_shape_with_effect_extras_belongs_to_the_effect_stage_only() {
        let mut mesh = cpu_mesh(FIRE_EFFECT_NAME);
        mesh.effect = Some(Arc::new(
            crate::rendering::effects::test_fixtures::stub_material(),
        ));

        let prepared = prepare_submesh("fire.glb", 0, mesh, None);

        assert!(matches!(prepared.route, ShapeRoute::Effect { .. }));
    }

    #[test]
    fn a_leftover_inline_effect_shape_stays_in_the_object_path_behind_the_fx_toggle() {
        let prepared = prepare_submesh("fog.glb", 0, cpu_mesh(LEFTOVER_INLINE_EFFECT_NAME), None);

        assert!(matches!(prepared.route, ShapeRoute::Object));
        assert_eq!(
            visibility_flag(false, false, &prepared.material),
            INSTANCE_FLAGS_HIDDEN_FX
        );
    }

    #[test]
    fn a_refraction_shape_with_extras_belongs_to_the_refraction_stage() {
        let mut mesh = cpu_mesh(REFRACTION_NAME);
        mesh.refraction = Some(refraction_extras());

        let prepared = prepare_submesh("haze.glb", 0, mesh, None);

        assert!(matches!(prepared.route, ShapeRoute::Refraction { .. }));
    }

    #[test]
    fn a_refraction_shape_no_stage_draws_stays_behind_the_fx_toggle() {
        let prepared = prepare_submesh("haze.glb", 0, cpu_mesh(REFRACTION_NAME), None);

        assert!(matches!(prepared.route, ShapeRoute::Object));
        assert_eq!(
            visibility_flag(false, false, &prepared.material),
            INSTANCE_FLAGS_HIDDEN_FX
        );
    }

    #[test]
    fn an_effect_shape_on_a_skinned_model_is_drawn_by_the_object_path() {
        let mut material = SubMeshMaterial::parse(Some(LEFTOVER_INLINE_EFFECT_NAME));
        material.object_visibility = ObjectVisibility::Drawn;

        assert_eq!(visibility_flag(false, false, &material), 0);
        assert_ne!(
            material_instance_flags(&material) & INSTANCE_FLAGS_EFFECT,
            0
        );
        assert!(
            !crate::rendering::skinning::materials::PartPolicy::Prop(&[])
                .hides_part_by_default(Some(LEFTOVER_INLINE_EFFECT_NAME))
        );
        assert!(
            crate::rendering::skinning::materials::PartPolicy::Prop(&[])
                .hides_part_by_default(Some(REFRACTION_NAME))
        );
    }
    #[test]
    fn effects_models_are_fx_except_placed_decals() {
        // (path, fx_filename_rule, expected)
        let cases = [
            ("Meshes\\Effects\\Mist01.nif", false, true),
            ("meshes/effects/mist01.nif", false, true),
            ("Effects\\Mist01.nif", false, true),
            ("Meshes\\Effects\\DecalsPlaced\\Scorch01.nif", true, false),
            ("Meshes\\Props\\Crate.nif", true, false),
            ("Meshes\\Water\\FXWaterfall01.nif", true, true),
            ("Meshes\\Water\\FXWaterfall01.nif", false, false),
            ("Meshes\\fxstuff\\Crate.nif", true, false),
            ("Meshes\\Interiors\\IBeam01.nif", true, false),
        ];
        for (path, rule, expected) in cases {
            assert_eq!(is_fx_model(path, rule), expected, "{path} rule={rule}");
        }
    }

    #[test]
    fn two_sided_and_decal_materials_carry_their_flags() {
        let flags = material_flags("a_d.dds|a_n.dds|decal|ts");
        assert_eq!(flags, INSTANCE_FLAGS_DECAL | INSTANCE_FLAGS_TWO_SIDED);
    }

    fn rings(center: (i32, i32), full_radius: i32, lod_width: i32) -> Rings {
        Rings {
            center,
            full_radius,
            outer_radius: full_radius + lod_width,
        }
    }

    #[test]
    fn full_ring_is_a_chebyshev_square_including_its_edge() {
        let rings = rings((10, -4), 2, 3);
        assert!(rings.in_full((12, -6)));
        assert!(rings.in_full((8, -2)));
        assert!(!rings.in_full((13, -4)));
        assert!(!rings.in_full((10, -7)));
    }

    #[test]
    fn lod_ring_is_the_annulus_outside_the_full_ring() {
        let rings = rings((0, 0), 2, 3);
        assert!(!rings.in_lod((2, 2)));
        assert!(rings.in_lod((3, 0)));
        assert!(rings.in_lod((5, -5)));
        assert!(!rings.in_lod((6, 0)));
    }

    #[test]
    fn every_cell_is_in_at_most_one_ring() {
        let rings = rings((0, 0), 2, 3);
        let cells = (-8..=8).flat_map(|x| (-8..=8).map(move |y| (x, y)));
        for cell in cells {
            assert!(
                !(rings.in_full(cell) && rings.in_lod(cell)),
                "{cell:?} is in both rings"
            );
            assert_eq!(
                rings.in_any(cell),
                rings.in_full(cell) || rings.in_lod(cell),
                "{cell:?}"
            );
        }
    }

    #[test]
    fn full_cells_cover_exactly_the_full_square() {
        let rings = rings((3, 7), 2, 3);
        let cells = rings.full_cells();
        assert_eq!(cells.len(), 25);
        assert!(cells.iter().all(|cell| rings.in_full(*cell)));
        assert_eq!(cells.iter().collect::<HashSet<_>>().len(), 25);
    }

    #[test]
    fn lod_cells_cover_exactly_the_annulus() {
        let rings = rings((3, 7), 2, 3);
        let cells = rings.lod_cells();
        // (2*5+1)^2 - (2*2+1)^2
        assert_eq!(cells.len(), 96);
        assert!(cells.iter().all(|cell| rings.in_lod(*cell)));
        assert_eq!(cells.iter().collect::<HashSet<_>>().len(), 96);
    }

    #[test]
    fn a_zero_radius_full_ring_is_the_center_cell() {
        assert_eq!(rings((4, 4), 0, 1).full_cells(), vec![(4, 4)]);
    }

    #[test]
    fn standing_still_brings_no_cells_into_either_ring() {
        let rings = rings((0, 0), 2, 3);
        assert_eq!(rings.full_cells_entering(&rings), []);
        assert_eq!(rings.lod_cells_entering(&rings), []);
    }

    #[test]
    fn crossing_one_cell_east_brings_in_one_full_column() {
        let previous = rings((0, 0), 2, 3);
        let current = rings((1, 0), 2, 3);
        let entering: HashSet<_> = current.full_cells_entering(&previous).into_iter().collect();
        let expected: HashSet<_> = (-2..=2).map(|dy| (3, dy)).collect();
        assert_eq!(entering, expected);
    }

    #[test]
    fn crossing_one_cell_east_brings_in_the_far_lod_column_and_the_dropped_full_column() {
        let previous = rings((0, 0), 2, 3);
        let current = rings((1, 0), 2, 3);
        let entering: HashSet<_> = current.lod_cells_entering(&previous).into_iter().collect();
        let outer_column = (-5..=5).map(|dy| (6, dy));
        let dropped_full_column = (-2..=2).map(|dy| (-2, dy));
        let expected: HashSet<_> = outer_column.chain(dropped_full_column).collect();
        assert_eq!(entering, expected);
    }

    #[test]
    fn teleporting_brings_in_the_whole_full_ring() {
        let previous = rings((0, 0), 2, 3);
        let current = rings((100, 100), 2, 3);
        assert_eq!(current.full_cells_entering(&previous).len(), 25);
    }

    #[test]
    fn nearest_first_orders_by_ring_then_distance() {
        let rings = rings((10, 10), 2, 3);
        let sorted = rings.nearest_first(&[(12, 12), (11, 11), (10, 11), (10, 10), (9, 10)]);
        assert_eq!(sorted[0], (10, 10));
        assert_eq!(sorted[3], (11, 11));
        assert_eq!(sorted[4], (12, 12));
        assert_eq!(
            sorted[1..3].iter().collect::<HashSet<_>>(),
            HashSet::from([&(10, 11), &(9, 10)])
        );
    }

    #[test]
    fn nearest_first_prefers_a_closer_cell_over_a_further_one_in_the_same_ring() {
        let rings = rings((0, 0), 3, 3);
        let sorted = rings.nearest_first(&[(3, 3), (3, 0)]);
        assert_eq!(sorted, vec![(3, 0), (3, 3)]);
    }

    #[test]
    fn collision_ring_is_the_vanilla_radius_widened_by_the_physics_radius() {
        assert_eq!(collision_ring_radius(2, 0), 2);
        assert_eq!(collision_ring_radius(2, 2), 2);
        assert_eq!(collision_ring_radius(2, 5), 5);
    }

    #[test]
    fn streaming_integrates_at_least_one_cell_per_frame() {
        assert_eq!(cells_per_frame(LoadMode::Streaming, 0), 1);
        assert_eq!(cells_per_frame(LoadMode::Streaming, 3), 3);
    }

    #[test]
    fn loading_integrates_at_least_the_loading_floor() {
        assert_eq!(
            cells_per_frame(LoadMode::Loading, 1),
            LOADING_CELLS_PER_FRAME
        );
        assert_eq!(
            cells_per_frame(LoadMode::Loading, LOADING_CELLS_PER_FRAME + 8),
            LOADING_CELLS_PER_FRAME + 8
        );
    }

    fn group_with(instances: usize) -> CellObjectGroup {
        CellObjectGroup {
            mega_key: Arc::from("key"),
            instances: vec![GpuInstance::zeroed(); instances],
        }
    }

    #[test]
    fn group_base_offsets_are_running_instance_counts() {
        let groups = [group_with(3), group_with(0), group_with(5), group_with(2)];
        assert_eq!(group_base_offsets(&groups), vec![0, 3, 3, 8]);
    }

    #[test]
    fn group_base_offsets_of_no_groups_is_empty() {
        assert_eq!(group_base_offsets(&[]), Vec::<u32>::new());
    }

    #[test]
    fn world_aabb_of_a_translated_box_is_shifted() {
        let local = Aabb {
            min: Vec3::new(-1.0, -2.0, -3.0),
            max: Vec3::new(1.0, 2.0, 3.0),
        };
        let (min, max) = world_aabb(
            Some(&local),
            &Mat4::from_translation(Vec3::new(10.0, 0.0, 5.0)),
        );
        assert_eq!(min, [9.0, -2.0, 2.0]);
        assert_eq!(max, [11.0, 2.0, 8.0]);
    }

    #[test]
    fn world_aabb_rotated_a_quarter_turn_swaps_the_extents() {
        let local = Aabb {
            min: Vec3::new(-1.0, -2.0, -3.0),
            max: Vec3::new(1.0, 2.0, 3.0),
        };
        let quarter_turn_about_z = Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let (min, max) = world_aabb(Some(&local), &quarter_turn_about_z);
        let expected_min = Vec3::new(-2.0, -1.0, -3.0);
        let expected_max = Vec3::new(2.0, 1.0, 3.0);
        assert!(Vec3::from(min).abs_diff_eq(expected_min, 1e-5), "{min:?}");
        assert!(Vec3::from(max).abs_diff_eq(expected_max, 1e-5), "{max:?}");
    }

    #[test]
    fn world_aabb_without_a_mesh_box_is_a_unit_box_at_the_origin_of_the_transform() {
        let (min, max) = world_aabb(None, &Mat4::from_translation(Vec3::new(4.0, 5.0, 6.0)));
        assert_eq!(min, [3.0, 4.0, 5.0]);
        assert_eq!(max, [5.0, 6.0, 7.0]);
    }

    #[test]
    fn max_axis_scale_is_the_largest_column_length() {
        let transform = Mat4::from_scale(Vec3::new(2.0, 3.0, 4.0)).to_cols_array_2d();
        assert!((max_axis_scale(&transform) - 4.0).abs() < 1e-6);
    }

    #[test]
    fn max_axis_scale_ignores_rotation() {
        let transform = Mat4::from_rotation_y(1.0) * Mat4::from_scale(Vec3::splat(2.5));
        assert!((max_axis_scale(&transform.to_cols_array_2d()) - 2.5).abs() < 1e-5);
    }

    #[test]
    fn placements_demand_is_the_strongest_placement() {
        let view = DemandView::new(
            Some(Vec3::ZERO),
            ScreenView {
                vertical_fov: std::f32::consts::FRAC_PI_2,
                width: 1000,
                height: 1000,
            },
            0.0,
        );
        let near = ([9.0, 0.0, 0.0], [10.0, 1.0, 1.0], 1.0);
        let far = ([99.0, 0.0, 0.0], [100.0, 1.0, 1.0], 1.0);
        let demand = placements_demand(&view, 1.0, [far, near].into_iter());
        // 500 px per unit at distance 1, nearest box point at 9.
        assert!((demand - 500.0 / 9.0).abs() < 1e-3, "{demand}");
    }

    #[test]
    fn placements_demand_scales_with_the_placement_scale() {
        let view = DemandView::new(
            Some(Vec3::ZERO),
            ScreenView {
                vertical_fov: std::f32::consts::FRAC_PI_2,
                width: 1000,
                height: 1000,
            },
            0.0,
        );
        let placement = ([10.0, 0.0, 0.0], [11.0, 1.0, 1.0], 3.0);
        let demand = placements_demand(&view, 2.0, std::iter::once(placement));
        assert!((demand - 500.0 * 6.0 / 10.0).abs() < 1e-3, "{demand}");
    }

    #[test]
    fn placements_demand_without_placements_is_no_demand() {
        let view = DemandView::default();
        assert_eq!(placements_demand(&view, 1.0, std::iter::empty()), NO_DEMAND);
    }
}
