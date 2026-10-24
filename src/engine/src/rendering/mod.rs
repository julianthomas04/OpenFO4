mod activation;
pub mod adapter;
pub mod addon_nodes;
pub mod anim_debug_panel;
pub mod animated_objects;
mod auto_anim;
mod auto_character;
pub mod boot;
pub mod camera;
pub mod cube_array;
pub mod cull_stats;
pub mod debug_skip;
pub mod debug_ui;
pub mod debug_view;
pub mod device;
pub mod device_loss;
pub mod effects;
pub mod egui_pass;
pub mod fatal_runtime_error;
mod flow_input;
pub mod foot_surface;
pub mod fps;
pub mod free_list;
pub mod frustum;
pub mod game_frame;
pub mod game_settings;
pub mod gpu_cull;
pub mod gpu_driven;
pub mod gpu_profiler;
pub mod gpu_slab;
#[cfg(test)]
pub(crate) mod gpu_test;
pub mod grass;
pub mod grass_streaming;
pub mod hiz;
pub mod hud;
pub mod instance;
pub mod light_anim;
pub mod lighting;
mod line_buffer;
pub mod local_lights;
pub mod mega_buffer;
pub mod menu;
mod menu_frame;
pub mod mouse_capture;
pub mod navmesh_overlay;
pub mod particles;
pub mod perf_log;
pub mod pipboy;
pub mod pipboy_capture;
pub mod placed_light;
pub mod post;
pub mod ragdoll_debug_panel;
pub mod ragdoll_objects;
mod ragdoll_sim_link;
pub mod readback_ring;
pub mod refraction;
pub mod render_pipelines;
pub(crate) mod row_pose;
pub mod scene_copy;
pub mod shaders;
pub mod shadows;
pub mod skeleton_overlay;
pub mod skinning;
mod startup_reads;
mod stream_settle;
pub(crate) mod svg_raster;
pub mod texture;
pub mod texture_streaming;
pub mod ui_colour;
pub mod ui_fonts;
pub mod version_overlay;
pub mod vertex;
pub mod water;
pub mod window_surface;
pub mod world_streamer;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::{Mat4, UVec2, UVec4, Vec3};
use tracing::instrument;
use winit::{keyboard::KeyCode, window::Window};

use crate::anim::marker_timeline::{FIRST_PERSON_ASSETS_DIR, load_gait_timings};
use crate::assets::AssetServer;
use crate::assets::layout;
use crate::assets::pipboy_art::ArtLibrary;
use crate::audio::world_music_loader::{load_world_music, seed_from_clock};
use crate::audio::{
    AudioError, AudioFrame, AudioSystem, Footsteps, PLACE_POLLS_PER_SECOND, PlacePoll,
};
use crate::bethesda_space::UNITS_FROM_ENGINE;
use crate::game_flow::{GameFlow, SpawnLocation, SpawnPlace, StartMode};
use crate::rendering::water::ssr::constants::Surroundings;
use crate::rendering::{
    anim_debug_panel::{AnimDebugPanel, characters_section},
    camera::{Camera, CameraController},
    cull_stats::CullStatsReadback,
    debug_skip::{DebugPass, DebugSkip},
    debug_ui::{DebugUi, ToastType},
    device_loss::DeviceLossMonitor,
    fatal_runtime_error::FatalRuntimeError,
    fps::FpsUtils,
    gpu_cull::{CullUniforms, ObjectCullPipeline, TerrainCullPipeline},
    gpu_driven::{
        IndirectDrawBuffers, InstanceBuffer, MAX_TEXTURES, MaterialRegistry, SlotChanges,
        SplatmapArray, TerrainIndirectBuffer, TerrainInstanceBuffer, TextureArray,
    },
    gpu_profiler::{GpuPass, GpuProfiler, SpanPart, panel::gpu_profiler_section},
    gpu_slab::GpuSlab,
    grass::{GrassPanelState, GrassStage, GrassView, grass_panel},
    hiz::HizPipeline,
    lighting::{
        FrameBinding, LightingState, debug_panel::lighting_panel,
        exterior_weather::ExteriorWeather, interior_catalog::InteriorLightingCatalog,
        state::FrameInputs,
    },
    line_buffer::LineBuffer,
    local_lights::{
        ClusterBuffers, LocalLightFrame, LocalLightRenderer, LocalLightSettings,
        debug_panel::local_lights_panel,
    },
    mega_buffer::MegaBuffer,
    mouse_capture::MouseCapture,
    navmesh_overlay::{NavmeshOverlay, navmesh_section},
    perf_log::{PerfFrame, PerfLog},
    pipboy::{PipBoy, PipBoyEnv, input::OpenKey},
    pipboy_capture::{CaptureConfig, PipBoyCapture},
    placed_light::GpuPlacedLight,
    post::{
        PostChain,
        depth_of_field::{BokehGate, DofCamera, select_frame},
        hdr_target::{HDR_FORMAT, clear_color},
        tonemap,
    },
    ragdoll_debug_panel::ragdoll_settings_section,
    readback_ring::{ReadbackError, escalate_device_loss},
    render_pipelines::{
        debug_lines::{DebugLinePipeline, LineVertex},
        object::ObjectPipelines,
        terrain::TerrainPipeline,
        world_pass_layout::{FEEDBACK_GROUP, WorldPassLayout},
    },
    scene_copy::SceneCopy,
    shadows::{
        ShadowRenderer, ShadowScene, ShadowState,
        lamps::{
            LampShadowCache, LampShadowState, atlas_texture::LampAtlasTexture,
            debug_panel::lamp_shadows_panel, selection::SelectionView, source::LampCandidate,
        },
        shadow_cull::caster_potentials,
        shadow_maps::ShadowMaps,
    },
    skeleton_overlay::{OverlayView, SkeletonOverlay},
    skinning::{
        CharacterSystem, materials::CharacterGpuContext, pipelines::SkinnedPipelines,
        scene::Visibility,
    },
    texture::Texture,
    texture_streaming::{ResidencyPolicy, TextureFeedback, TextureResidency, demand::ScreenView},
    water::{CameraWater, WaterStage, water_panel},
    window_surface::WindowSurface,
    world_streamer::{InteriorRequestError, LoadMode, StreamTargets, UploadBudget, WorldStreamer},
};
use crate::sim::pipboy::PipBoyAction;
use crate::sim::{
    DynamicPose, MaterialBuoyancy, PlayerInput, PlayerMovement, SimCommand, SimWorld, water_physics,
};
use crate::spawn_cell::SpawnCellTarget;
use crate::startup::{StartupError, StartupTimer};
use crate::utils;
use crate::video::{MenuVideo, VideoCommand, VideoError};
use crate::water_world::{Scope as WaterScope, SharedWaterWorld, WaterWorld, physics_query};
use activation::{Activation, TeleportRefusal, teleport_gate};
use addon_nodes::AddonFx;
use animated_objects::AnimatedObjects;
use auto_anim::AutoAnim;
use auto_character::AutoCharacters;
use boot::{BootParts, GpuContext, StartupInputs, StateInputs};
use cube_array::CubeArray;
use effects::stage::{EffectFrameInputs, EffectStage};
use light_anim::LightAnimator;
use lighting::frame_uniform::FrameUniform;
use menu::{MenuRenderer, MenuScene};
use particles::CameraBasis;
use ragdoll_objects::RagdollObjects;
use refraction::{RefractionFrameInputs, RefractionStage};
use shaders::startup::EffectValidation;
use startup_reads::StartupDbReads;
use ui_colour::UiColourSet;
use ui_fonts::UiFonts;

// 00003C:Fallout4.esm // Commonwealth
const WORLDSPACE: &str = crate::game_flow::spawn_locations::COMMONWEALTH;

/// One entry in the debug worldspace dropdown: the `form_key` the streamer
/// switches to, the "<pretty name> (<`form_key`>)" menu text, and the player
/// start cell (resolved once at boot so a switch needs no per-switch DB query).
struct WorldspaceOption {
    form_key: String,
    label: String,
    start_cell: (i32, i32),
}

impl WorldspaceOption {
    /// Build a dropdown entry from a WRLD row, resolving its player start cell
    /// up front (so a switch needs no DB query). `None` for rows without a
    /// `form_key` (nothing to switch to). The label prefers the readable FULL
    /// name, falling back to the editor id, then a placeholder.
    fn from_worldspace(
        w: crate::assets::db::worldspaces::Worldspace,
        db: &crate::assets::db::DbClient,
    ) -> Option<Self> {
        let form_key = w.form_key?;
        let pretty = w
            .name
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(w.editor_id.as_deref())
            .unwrap_or("(unnamed)");
        let start_cell = WorldStreamer::resolve_start_cell(db, &form_key);
        Some(Self {
            label: format!("{pretty} ({form_key})"),
            form_key,
            start_cell,
        })
    }
}

/// One entry in the debug interior dropdown: the cell `form_key` the streamer
/// loads, the "<editor id> (<`form_key`>)" (or bare `form_key`) menu text, the
/// synthetic grid the interior is keyed at, and the player start position — the
/// grid and start are derived from the cell's placed-object anchor, computed
/// once at boot so a load runs no DB query.
struct InteriorOption {
    form_key: String,
    label: String,
    grid: (i32, i32),
    start_pos: Vec3,
}

/// How far (engine units) the activation ray reaches when looking for a load
/// door under the crosshair. ~150 Bethesda units (SCALE 0.1), near vanilla
/// `fActivatePickLength`; the debug inspector ray (2000) is far longer.
const ACTIVATE_REACH: f32 = 15.0;

/// Frames the `OPENFO4_AUTO_ACTIVATE` dev hook waits between hops (~3s at 60fps)
/// as a floor; the real gate is `player_eye().is_some()` (the prior teleport's
/// player has respawned). Dev-only; never affects normal play.
const AUTO_ACTIVATE_COOLDOWN_FRAMES: u32 = 180;

/// A resolved load-door teleport: everything needed to enter the destination,
/// precomputed at boot from `placed_teleports`. `start_pos`/`grid`/`yaw` are in
/// engine space. For an interior destination `worldspace` is unused; for an
/// exterior destination `cell_form_key` is unused.
#[derive(Clone)]
struct ResolvedTarget {
    is_interior: bool,
    worldspace: String,
    cell_form_key: String,
    start_pos: Vec3,
    grid: (i32, i32),
    /// `None` keeps the current look direction: spawns without a stored heading,
    /// and the initial spawn when `OPENFO4_SPAWN_LOOK` is set.
    yaw: Option<f32>,
}

impl ResolvedTarget {
    /// The initial spawn into an interior cell. An explicit `spawn_look` wins
    /// over the door heading of the arrival.
    fn interior_spawn(target: SpawnCellTarget, spawn_look: Option<(f32, f32)>) -> Self {
        let (start_pos, grid) = Self::start_at(target.position);
        Self {
            is_interior: true,
            worldspace: String::new(),
            cell_form_key: target.cell_form_key,
            start_pos,
            grid,
            yaw: if spawn_look.is_some() {
                None
            } else {
                target
                    .heading
                    .map(|heading| crate::bethesda_space::yaw_from_rotation_z(heading as f32))
            },
        }
    }

    /// The spawn picker's choice: a position in a worldspace or in one interior cell.
    fn picked_spawn(location: &SpawnLocation) -> Self {
        match &location.place {
            SpawnPlace::Interior { cell_form_key } => Self::interior_spawn(
                SpawnCellTarget {
                    cell_form_key: cell_form_key.clone(),
                    position: location.position,
                    heading: location.heading,
                },
                None,
            ),
            SpawnPlace::Exterior { worldspace } => {
                let (start_pos, grid) = Self::start_at(location.position);
                Self {
                    is_interior: false,
                    worldspace: worldspace.clone(),
                    cell_form_key: String::new(),
                    start_pos,
                    grid,
                    yaw: location
                        .heading
                        .map(|heading| crate::bethesda_space::yaw_from_rotation_z(heading as f32)),
                }
            }
        }
    }

    /// The engine-space start position and its cell grid for a spawn at `position`, given in
    /// Bethesda units.
    fn start_at(position: [f64; 3]) -> (Vec3, (i32, i32)) {
        let [x, y, z] = position.map(|c| c as f32);
        let start_pos = WorldStreamer::beth_to_engine(x, y, z);
        (start_pos, WorldStreamer::camera_to_grid(start_pos))
    }
}

/// The MATT buoyancy table of the water simulation, from the `material_types`
/// rows. `None` (logged) leaves every body at the default buoyancy.
fn material_buoyancy_table(db_path: &str) -> Option<MaterialBuoyancy> {
    let rows = match crate::assets::db::DbClient::new(db_path)
        .and_then(|db| db.get_material_buoyancies())
    {
        Ok(Some(rows)) => rows,
        Ok(None) => return None,
        Err(e) => {
            tracing::warn!(
                "material buoyancy table not loaded ({e}); dynamic bodies use the default buoyancy"
            );
            return None;
        }
    };
    let table = MaterialBuoyancy::new(rows.iter().map(|row| (row.material_hash, row.buoyancy)));
    tracing::info!("material buoyancy: {} materials", table.len());
    Some(table)
}

/// Which list the developer menu's cell picker draws from. A UI-only selector — the
/// space actually loaded is owned by the streamer ([`WorldStreamer::in_interior`]).
#[derive(Clone, Copy, PartialEq, Eq)]
enum DebugWorldMode {
    Worldspace,
    Interior,
}

/// Which controller drives the camera: the walking player (default) or the
/// free-fly debug camera. Toggled with V or the debug-UI checkbox.
#[derive(Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Walk,
    Fly,
}

/// Sprint widens the FOV. The magnitude and ramp rate are feel-tuned against
/// footage, in Beth FOV degrees.
const SPRINT_FOV_ADJUST: f32 = 10.0;
const FOV_ADJUST_PER_SEC: f32 = 45.0;

/// Bethesda INI FOV (horizontal at a 16:9 base, Hor+ scaling) -> the vertical
/// FOV the projection wants. fDefaultWorldFOV=70 comes out at 43.0° vertical.
fn beth_fov_to_vertical(fov_deg: f32) -> f32 {
    2.0 * ((fov_deg.to_radians() / 2.0).tan() * (9.0 / 16.0))
        .atan()
        .to_degrees()
}

/// Pressed-state of the walk-mode movement keys plus the FO4-style toggle
/// states, snapshotted into a [`PlayerInput`] each frame.
///
/// Vanilla control semantics: sprint and crouch are toggles, not holds.
/// Shift arms sprint only while W alone of WASD is down; sprint drops on the
/// next Shift press, on W release, when any strafe/backpedal key is pressed,
/// or when sneak is entered (AP exhaustion will join once AP exists). Ctrl
/// toggles crouch; starting a sprint stands the player up, jumping does not.
#[derive(Default)]
struct WalkKeys {
    forward: bool,
    backward: bool,
    left: bool,
    right: bool,
    jump: bool,
    /// Raw Shift/Ctrl pressed-state, kept only for key-down edge detection.
    sprint_held: bool,
    crouch_held: bool,
    /// The toggle states the sim actually consumes.
    sprint_active: bool,
    crouch_active: bool,
}

impl WalkKeys {
    /// Drops the keys the player was holding; the sprint and crouch toggles keep their state.
    fn release_held(&mut self) {
        *self = Self {
            sprint_active: self.sprint_active,
            crouch_active: self.crouch_active,
            ..Self::default()
        };
    }
}

/// Which GPU buffers were reallocated this frame; every bind group that
/// references one must be rebuilt before the next draw.
struct BufferReallocations {
    object_instances: bool,
    terrain_instances: bool,
    object_indirect: bool,
    terrain_indirect: bool,
    shadow_draws: bool,
    placed_lights: bool,
}

/// The keys the free-fly camera moves with; released when the Pip-Boy opens so none stays held.
const FLY_CAMERA_KEYS: [KeyCode; 7] = [
    KeyCode::KeyW,
    KeyCode::KeyS,
    KeyCode::KeyA,
    KeyCode::KeyD,
    KeyCode::Space,
    KeyCode::ShiftLeft,
    KeyCode::ControlLeft,
];

const SHADOW_ALPHA_MASK_ENV: &str = "OPENFO4_SHADOW_ALPHA_MASK";
/// Dev override: park the camera at its spawn position. The walking player and
/// its gravity are bypassed, so a spawn below the sea, or in the air, stays put.
/// The mouse capture starts released (see [`MouseCapture`]), so mouse motion
/// does not turn the view until Esc locks it.
const FREEZE_CAMERA_ENV: &str = "OPENFO4_FREEZE_CAMERA";
const NAVMESH_OVERLAY_ENV: &str = "OPENFO4_NAVMESH_OVERLAY";
/// Light green, the colour the collision wireframe has always had.
const COLLISION_WIREFRAME_COLOR: [u8; 4] = [115, 255, 140, 255];

/// Whether the camera is parked, from the flag lookup `read_flag(name, default)`
/// (`utils::env_flag` in production).
pub(crate) fn camera_frozen(read_flag: impl Fn(&str, bool) -> bool) -> bool {
    read_flag(FREEZE_CAMERA_ENV, false)
}

/// `OPENFO4_SHADOW_ALPHA_MASK=0` makes the shadow passes bind the full
/// material bind group again (no stand-in textures, no extra rebuilds).
fn shadow_alpha_mask_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| crate::utils::env_flag(SHADOW_ALPHA_MASK_ENV, true))
}

pub struct State {
    surface: Arc<WindowSurface>,
    device: wgpu::Device,
    device_loss: DeviceLossMonitor,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    is_surface_configured: bool,
    pub window: Arc<Window>,

    // Pipelines
    world_pass_layout: WorldPassLayout,
    object_pipelines: ObjectPipelines,
    terrain_pipeline: TerrainPipeline,
    debug_line_pipeline: DebugLinePipeline,
    skeleton_overlay: SkeletonOverlay,

    // Camera
    camera: Camera,
    pub camera_controller: CameraController,
    cached_vp: Mat4,

    // Lighting
    lighting: LightingState,
    interior_lighting: Option<InteriorLightingCatalog>,
    frame_binding: FrameBinding,
    shadow_renderer: ShadowRenderer,
    lamp_shadow_cache: LampShadowCache,
    /// Reused every frame for the streamer's resident shadow lamps.
    lamp_candidates: Vec<LampCandidate>,
    /// Reused per-frame buffer for the sim's dynamic-body pose readback.
    dynamic_poses: Vec<DynamicPose>,
    /// (placed-light slab slot, lamp-shadow table slot) for the light cull.
    lamp_cull_slots: Vec<UVec2>,
    lamp_atlas_preview: egui::TextureId,
    local_lights: LocalLightRenderer,
    /// Origin of the frame uniform's time.
    /// Seconds the world has run: the frame uniform's time, frozen while the Pip-Boy is open.
    world_seconds: f32,

    // Post
    post: PostChain,
    scene_copy: SceneCopy,
    water_stage: WaterStage,
    effect_stage: EffectStage,
    refraction_stage: RefractionStage,
    /// The latest frame uniform, which the effect stage prepares its draws from at render time.
    latest_frame: Option<FrameUniform>,
    /// Particle systems of addon attachments and placed models.
    addon_fx: AddonFx,
    light_anim: LightAnimator,
    grass: GrassStage,
    /// The depth of field gate in normal gameplay, with the INI's bokeh setting.
    bokeh_gate: BokehGate,

    // Depth
    depth_texture: Texture,
    hiz_pipeline: HizPipeline,
    /// False until the pyramid holds a rendered frame (startup, resize); the
    /// cull shader skips the occlusion test while false.
    hiz_valid: bool,
    debug_skip: DebugSkip,

    // GPU-driven: geometry
    object_mega: MegaBuffer,
    terrain_mega: MegaBuffer,

    // GPU-driven: instances
    object_instances: InstanceBuffer,
    /// Placed LIGH references of the resident full-detail cells, read by the
    /// light cull.
    placed_lights: GpuSlab<GpuPlacedLight>,
    terrain_instances: TerrainInstanceBuffer,

    // GPU-driven: materials & textures
    texture_array: TextureArray,
    cube_array: CubeArray,
    texture_residency: TextureResidency,
    texture_feedback: TextureFeedback,
    /// Anisotropy clamp of the material sampler.
    max_anisotropy: u32,
    splatmaps: SplatmapArray,
    material_registry: MaterialRegistry,

    // Skinned characters
    characters: CharacterSystem,
    skinned_pipelines: SkinnedPipelines,
    anim_panel: AnimDebugPanel,

    // GPU-driven: indirect draws
    object_indirect: IndirectDrawBuffers,
    terrain_indirect: TerrainIndirectBuffer,

    // GPU-driven: culling
    object_cull: ObjectCullPipeline,
    terrain_cull: TerrainCullPipeline,
    object_cull_bg: wgpu::BindGroup,
    terrain_cull_bg: wgpu::BindGroup,

    // GPU-driven: bind groups for rendering
    material_bind_group: wgpu::BindGroup,
    /// `None` when `SHADOW_ALPHA_MASK_ENV` disables masking: the shadow
    /// passes then bind `material_bind_group`.
    shadow_material_bind_group: Option<wgpu::BindGroup>,
    instance_bind_group: wgpu::BindGroup,
    material_sampler: wgpu::Sampler,
    terrain_sampler: wgpu::Sampler,
    shadow_sampler: wgpu::Sampler,
    /// Texture slots changed but the material bind group hasn't been rebuilt
    /// yet (rebuilds are debounced; see `update()`).
    material_bg_dirty: bool,
    material_bg_last_rebuild: Instant,
    /// Slot-table changes accumulated since the last rebuild.
    pending_slot_changes: SlotChanges,
    /// Slots that became alpha-tested casters' diffuse since the last rebuild.
    pending_alpha_slots: Vec<u32>,
    /// `TextureArray::bind_group_len` at the last rebuild. Growth past it forces
    /// an immediate rebuild: the live bind group is shorter than the new slot
    /// indices, and indexing an unbound entry of a partially-bound binding
    /// array is undefined behavior — never debounce that case.
    material_bg_built_len: u32,

    // Streaming
    streamer: WorldStreamer,
    worldspaces: Vec<WorldspaceOption>,
    interiors: Vec<InteriorOption>,
    /// A worldspace picked in the debug UI this frame, applied at the top of
    /// the next `update` (the egui closure only records intent; the streamer
    /// teardown + camera teleport need direct field access, not the closure's
    /// capture).
    pending_worldspace: Option<String>,
    /// An interior cell picked in the debug UI this frame, applied next
    /// `update` like `pending_worldspace`.
    pending_interior: Option<String>,
    /// Which picker the developer menu shows (UI-only; the loaded space is the
    /// streamer's `in_interior`).
    world_mode: DebugWorldMode,
    /// Case-insensitive substring filter for the interior dropdown (~1195
    /// entries make a flat combo unusable).
    interior_filter: String,
    /// Load-door teleports keyed by the source door's REFR `form_key`, resolved at
    /// boot. Looked up O(1) by the activation raycast (E) and the debug
    /// `is_interactible` readout — no DB query at runtime.
    teleports: std::collections::HashMap<String, ResolvedTarget>,
    /// A load door activated this frame (E), applied next `update` like
    /// `pending_interior`.
    pending_teleport: Option<ResolvedTarget>,
    /// Dev override (`OPENFO4_AUTO_ACTIVATE="ref1,ref2,..."`): teleport through the
    /// listed load doors in sequence, one hop per readiness window. Lets the
    /// activation dispatch (both interior and worldspace branches, and the round
    /// trip) be exercised in headless/automated runs where OS input can't be
    /// injected into the window.
    auto_activate: Vec<String>,
    auto_idx: usize,
    auto_cooldown: u32,
    /// Resident animated placed objects (doors, containers, ...) and their playback.
    animated_objects: AnimatedObjects,
    /// Skinned replacements of ragdolls that moved (`OPENFO4_RAGDOLL_POKE` pokes one for checks).
    ragdoll_objects: RagdollObjects,
    /// Dev override (`OPENFO4_AUTO_ANIM="ref1,ref2,..."`): toggle those objects' animations.
    auto_anim: AutoAnim,
    /// Dev override (`OPENFO4_AUTO_CHARACTER`): spawn skinned characters playing given clips.
    auto_characters: AutoCharacters,

    // Simulation (sim/render split: commands in, poses out)
    sim: SimWorld,
    input_mode: InputMode,
    freeze_camera: bool,
    walk_keys: WalkKeys,
    /// Base world FOV in Bethesda convention (horizontal at a 16:9 base),
    /// from the INI cascade (`fDefaultWorldFOV`).
    world_fov_beth: f32,
    /// Additive FOV adjust in Beth degrees, ramped toward its target each
    /// frame, like the vanilla camera's FOV adjust. Sprint widens through this
    /// channel.
    fov_adjust: f32,

    // Collision wireframe overlay (debug; off by default). The line buffer
    // is rebuilt from the sim when the collider set changes (debounced —
    // streaming bursts bump the generation every frame) or the camera strays
    // from the last build point. The wgpu buffer is persistent and grows
    // geometrically; rebuilds rewrite it in place. The radius is fixed far
    // beyond the collision ring (`collision_radius` cells around the player),
    // so the overlay always covers the entire live collider set.
    collision_wireframe: bool,
    /// Navmesh wireframe (debug; `OPENFO4_NAVMESH_OVERLAY=1` turns it on at start). Rebuilt
    /// whenever the resident nav world changes.
    navmesh_overlay: NavmeshOverlay,
    collision_wire_radius: f32,
    collision_lines: LineBuffer,
    collision_wire_gen: u64,
    collision_wire_center: Vec3,
    collision_wire_built_radius: f32,
    collision_wire_last_rebuild: Instant,
    collision_wire_truncated: bool,

    // Native HUD overlay (crosshair + HP/AP meters). Color-only screen-space
    // pass, composited after HiZ generation and before the egui debug pass.
    hud: hud::HudRenderer,
    version_overlay: version_overlay::VersionOverlay,

    // Pip-Boy screen: replaces the HUD while the model says it is open.
    pipboy: PipBoy,
    /// Capture hooks not yet applied (they wait for the player to spawn).
    pipboy_env: PipBoyEnv,
    /// The screenshot session when `OPENFO4_PIPBOY_CAPTURE` is set.
    capture: Option<PipBoyCapture>,
    pipboy_was_open: bool,
    /// Whether the mouse was captured when the Pip-Boy opened, to restore on close.
    capture_before_pipboy: bool,
    // Game flow: boot, main menu, spawn picker, loading overlay, reveal.
    flow: GameFlow,
    // Silent (`None`) when the audio device cannot be opened.
    audio: Option<AudioSystem>,
    foot_surface: foot_surface::FootSurface<crate::assets::db::DbClient>,
    /// Paces how often the player's place is looked up for the world music.
    music_poll: PlacePoll,
    menu_video: MenuVideo,
    menu: MenuRenderer,
    /// Pointer position in framebuffer pixels, for menu hit-testing.
    pointer_px: (f32, f32),
    /// The grid cell the load in progress spawns the player in; only the stall report reads it.
    load_spawn_cell: (i32, i32),
    quit_requested: bool,

    // Debug UI
    pub debug_ui: DebugUi,
    mouse_capture: MouseCapture,
    fps_utils: FpsUtils,
    perf_log: Option<PerfLog>,
    fps_limit: u32,
    target_delta: f32,
    /// Debug toggle: render editor-marker instances (trigger volumes, path
    /// and effect markers). Off by default — the game never shows them.
    show_markers: bool,
    /// Debug toggle: render effect geometry hidden until effect animation and
    /// soft fading exist (inline non-BGEM effect shapes that the effect stage
    /// does not own, non-effect parts of Effects\ models). Off by default;
    /// shapes the effect stage draws and BGEM effects such as glass always render.
    show_fx: bool,
    /// Debug toggle: route decal-flagged geometry into the depth-biased decal
    /// buckets (the z-fighting fix). On by default; turning it off renders decals
    /// as ordinary coplanar geometry, for A/B comparison of the fix.
    decal_bias_enabled: bool,
    toast_queue: Vec<(String, ToastType)>,
    process_memory: utils::ProcessMemory,
    vram_text: String,
    memory_section_open: bool,
    aimed_door: bool,
    vram_last_refresh: Instant,

    // Stats
    cull_stats: CullStatsReadback,
    gpu_profiler: GpuProfiler,
}

impl State {
    pub fn queue_toast(&mut self, message: String, toast_type: ToastType) {
        self.toast_queue.push((message, toast_type));
    }

    /// Build the debug interior dropdown: every interior cell, each with a start
    /// position derived from its placed-object anchor (Bethesda -> engine
    /// space) and the grid it loads at. DB open/query failures are logged and
    /// degrade to an empty (or anchor-less) list — the engine still runs.
    fn build_interiors(db_path: &str) -> Vec<InteriorOption> {
        let db = match crate::assets::db::DbClient::new(db_path) {
            Ok(db) => db,
            Err(e) => {
                tracing::warn!("interior dropdown DB open failed: {e}");
                return Vec::new();
            }
        };
        let cells = match db.get_interior_cells() {
            Ok(cells) => cells,
            Err(e) => {
                tracing::warn!("interior dropdown query failed: {e}");
                return Vec::new();
            }
        };
        // Per-cell anchors (Bethesda units) -> engine-space start positions.
        // A query failure just drops every anchor; interiors then fall back
        // to the origin start below.
        let starts: std::collections::HashMap<String, Vec3> = match db.get_interior_anchors() {
            Ok(rows) => rows
                .into_iter()
                .filter_map(|(fk, bx, by, bz)| {
                    // Same Bethesda->engine transform as OPENFO4_SPAWN and the
                    // per-object placement: (x, z, -y) * SCALE. Drop a non-finite
                    // anchor (NaN/inf from corrupt position data) so it can't
                    // poison the player teleport — it falls back to the origin.
                    let world = WorldStreamer::beth_to_engine(bx as f32, by as f32, bz as f32);
                    world.is_finite().then_some((fk, world))
                })
                .collect(),
            Err(e) => {
                tracing::warn!("interior anchor query failed: {e}");
                std::collections::HashMap::new()
            }
        };

        cells
            .into_iter()
            .filter_map(|c| {
                let form_key = c.form_key?;
                let label = match c.editor_id.as_deref().filter(|s| !s.is_empty()) {
                    Some(eid) => format!("{eid} ({form_key})"),
                    None => form_key.clone(),
                };
                // Object-less interiors have no anchor (nothing to stand on);
                // fall back to the origin — the player falls / can free-fly.
                let start_pos = starts.get(&form_key).copied().unwrap_or(Vec3::ZERO);
                let grid = WorldStreamer::camera_to_grid(start_pos);
                Some(InteriorOption {
                    form_key,
                    label,
                    grid,
                    start_pos,
                })
            })
            .collect()
    }

    /// Build the load-door activation map: every teleport door resolved to its
    /// destination (interior cell or exterior worldspace) with the arrival
    /// position, grid and facing precomputed in engine space. DB open/query
    /// failures degrade to an empty map — doors are then simply non-interactible
    /// and the engine still runs.
    fn build_teleports(db_path: &str) -> std::collections::HashMap<String, ResolvedTarget> {
        let db = match crate::assets::db::DbClient::new(db_path) {
            Ok(db) => db,
            Err(e) => {
                tracing::warn!("teleport map DB open failed: {e}");
                return std::collections::HashMap::new();
            }
        };
        let records = match db.get_placed_teleports() {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("teleport map query failed: {e}");
                return std::collections::HashMap::new();
            }
        };

        let mut map = std::collections::HashMap::with_capacity(records.len());
        let mut invalid_pos = 0usize;
        for r in records {
            // Drop a non-finite destination (corrupt position) so it can never
            // poison the player teleport.
            let start_pos =
                WorldStreamer::beth_to_engine(r.dest_x as f32, r.dest_y as f32, r.dest_z as f32);
            if !start_pos.is_finite() {
                invalid_pos += 1;
                continue;
            }
            // Stream the ring at the cell containing the arrival position, for
            // both interior (synthetic key, like build_interiors) and exterior.
            // Deriving from the position — not the destination cell's DB grid —
            // is load-bearing: a teleport's destination ref is often a *persistent*
            // ref living in the worldspace TopCell, whose grid the installer spoofs
            // to (0,0); trusting that would stream the wrong ring and drop the
            // player through unloaded terrain far from the origin.
            let grid = WorldStreamer::camera_to_grid(start_pos);
            map.insert(
                r.source_ref,
                ResolvedTarget {
                    is_interior: r.is_interior,
                    worldspace: r.worldspace.unwrap_or_default(),
                    cell_form_key: r.dest_cell,
                    start_pos,
                    grid,
                    yaw: Some(crate::bethesda_space::yaw_from_rotation_z(
                        r.dest_rot_z as f32,
                    )),
                },
            );
        }

        if let Ok(total) = db.count_placed_teleports() {
            tracing::info!(
                "teleport map: {} doors resolved ({} unresolved destination, {} invalid position)",
                map.len(),
                total.saturating_sub(map.len()).saturating_sub(invalid_pos),
                invalid_pos
            );
        } else {
            tracing::info!("teleport map: {} doors resolved", map.len());
        }
        map
    }

    /// What the player is currently aiming at within activation reach, if it can
    /// be activated: a load door or an animated object. Shared by the E-press
    /// activation and (via a precomputed bool) the crosshair and the debug
    /// `is_interactible` readout, so all agree exactly.
    fn aimed_activation(&self) -> Option<Activation> {
        activation::resolve(&self.aimed_ref()?, &self.teleports, &self.animated_objects)
    }

    /// The ref id of the object under the crosshair within activation reach.
    fn aimed_ref(&self) -> Option<String> {
        self.sim
            .debug_ray_hit(
                self.camera.eye,
                self.camera_controller.forward(),
                ACTIVATE_REACH,
            )?
            .ref_id
    }

    /// The one entry for every request to teleport through a load door, by the door's ref id.
    /// Only a spawned player with no teleport already pending may queue one; the teleport
    /// itself runs in `update`. The menu spawn path (`begin_play`) bypasses this gate: the
    /// player is not spawned there by design.
    fn queue_teleport(&mut self, source_ref: &str) -> Result<(), TeleportRefusal> {
        teleport_gate(
            self.sim.player_eye().is_some(),
            self.pending_teleport.is_some(),
        )?;
        let target = self
            .teleports
            .get(source_ref)
            .ok_or_else(|| TeleportRefusal::UnknownSource(source_ref.to_string()))?;
        self.pending_teleport = Some(target.clone());
        Ok(())
    }

    /// E-press: queues the aimed load door's teleport or toggles the aimed animated object.
    fn activate_aimed(&mut self) {
        let Some(ref_id) = self.aimed_ref() else {
            return;
        };
        match activation::resolve(&ref_id, &self.teleports, &self.animated_objects) {
            Some(Activation::Teleport) => {
                if let Err(refusal) = self.queue_teleport(&ref_id) {
                    tracing::debug!("activation of {ref_id} refused: {refusal}");
                }
            }
            Some(Activation::Animate(entity)) => {
                if let Err(error) = self.animated_objects.toggle(entity) {
                    tracing::warn!("activation: {error}");
                }
            }
            None => {}
        }
    }

    /// Assembles the renderer state on the startup worker from the main thread's boot parts.
    #[instrument(skip_all)]
    pub fn build(
        parts: BootParts,
        inputs: StateInputs,
        timer: &mut StartupTimer,
    ) -> Result<State, StartupError> {
        let BootParts {
            window,
            gpu,
            mouse_capture,
            freeze_camera,
        } = parts;
        let GpuContext {
            surface,
            device,
            queue,
            device_loss,
            mut config,
            ..
        } = gpu;
        let StateInputs {
            startup:
                StartupInputs {
                    asset_server,
                    perf_log: perf_log_enabled,
                    start_mode,
                    ..
                },
            settings,
            spawn_cell,
        } = inputs;
        let surface_format = config.format;

        let db_path = asset_server.db_path();
        let db_reads = std::thread::Builder::new()
            .name("startup-db-reads".to_owned())
            .spawn({
                let db_path = db_path.clone();
                move || StartupDbReads::load(&db_path, start_mode)
            })
            .map_err(StartupError::WorkerSpawn)?;
        timer.mark("gpu_context")?;

        let depth_texture = Texture::create_depth_texture(&device, &config, "depth_texture");

        // Camera. Base FOV comes from the game INI cascade the installer
        // copies (step 2d), validated at startup.
        let world_fov_beth = settings.world_fov_beth;
        tracing::info!(
            "world FOV {world_fov_beth} ({:.1}° vertical)",
            beth_fov_to_vertical(world_fov_beth),
        );
        let mut camera = Camera {
            eye: (0.0, 100.0, 0.0).into(),
            target: (0.0, 100.0, -1.0).into(),
            up: Vec3::Y,
            aspect: Camera::aspect_of(config.width, config.height),
            fovy: beth_fov_to_vertical(world_fov_beth),
            znear: 0.5,
            zfar: 100_000.0,
        };

        let texture_streaming = settings.texture_streaming;
        let mut lighting = LightingState::new(
            ShadowState::new(settings.shadows),
            LampShadowState::new(settings.lamp_shadows),
            LocalLightSettings::new(settings.light_fade),
        );
        if let Some(weather) = ExteriorWeather::load_or_report(&asset_server.db_path(), WORLDSPACE)
        {
            lighting.set_exterior_weather(weather);
        }
        let shadow_maps = ShadowMaps::new(&device, &lighting.shadows.config);
        let lamp_atlas_resolution = lighting.lamp_shadows.config.atlas_resolution;
        let lamp_atlas = LampAtlasTexture::new(&device, lamp_atlas_resolution);
        let cluster_buffers = ClusterBuffers::new(&device);
        let frame_binding = FrameBinding::new(
            &device,
            &lighting.frame_uniform(&FrameInputs {
                camera: &camera,
                cascades: None,
                viewport: UVec2::new(config.width, config.height),
                max_anisotropy: u32::from(texture_streaming.max_anisotropy),
                texture_feedback: UVec4::ZERO,
                elapsed_seconds: 0.0,
                camera_water: CameraWater::NONE,
            }),
            &shadow_maps,
            lamp_atlas.view(),
            &cluster_buffers,
        );
        let scene_copy = SceneCopy::new(&device, config.width, config.height);
        let world_pass_layout = WorldPassLayout::new(&device, &frame_binding.layout);
        let post = PostChain::new(
            &device,
            config.format,
            config.width,
            config.height,
            &frame_binding.layout,
            &world_pass_layout.pipeline_layout,
        );
        timer.mark("frame_resources")?;

        // GPU-driven: textures & materials
        let texture_array = TextureArray::new(&device, &queue);
        let mut cube_array =
            CubeArray::new(&device, &queue, Path::new(asset_server.asset_location()));
        cube_array.set_forced_cube(utils::env_opt(cube_array::FORCED_CUBE_ENV));
        let texture_policy = ResidencyPolicy::new(&texture_streaming);
        let texture_residency =
            TextureResidency::new(texture_policy, asset_server.asset_location());
        let splatmaps = SplatmapArray::new(&device, &queue);
        let material_registry = MaterialRegistry::new(&device);

        // Samplers
        let material_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: texture_streaming.max_anisotropy,
            ..Default::default()
        });

        // Splatmap sampler: a cell's splatmap spans exactly 0..1 UV, so Repeat
        // would blend opposite edges together at every cell border.
        let terrain_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        // Alpha-tested shadow casters sample their diffuse with Repeat.
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });

        // Pipelines
        let texture_feedback =
            TextureFeedback::new(&device, &world_pass_layout.feedback_bind_group_layout);
        let object_pipelines = ObjectPipelines::new(
            &device,
            HDR_FORMAT,
            &world_pass_layout.feedback_pipeline_layout,
            texture_feedback.enabled(),
        );

        let terrain_pipeline = TerrainPipeline::new(
            &device,
            HDR_FORMAT,
            &world_pass_layout.feedback_pipeline_layout,
            texture_feedback.enabled(),
        );

        timer.mark("texture_arrays")?;

        let characters = CharacterSystem::new(&device, asset_server.asset_location());
        let skinned_pipelines = SkinnedPipelines::new(
            &device,
            HDR_FORMAT,
            &world_pass_layout,
            &frame_binding.layout,
            characters.bind_layout(),
            texture_feedback.enabled(),
        );

        let skeleton_overlay = SkeletonOverlay::new(
            &device,
            HDR_FORMAT,
            &frame_binding.layout,
            &depth_texture.view,
        );
        let debug_line_pipeline =
            DebugLinePipeline::new(&device, HDR_FORMAT, &world_pass_layout.pipeline_layout);

        // Mega-buffers
        let object_mega = MegaBuffer::new(&device, "object");
        let terrain_mega = MegaBuffer::new(&device, "terrain");

        // Instance buffers
        let object_instances = InstanceBuffer::new(&device, "object_instances");
        let placed_lights = GpuSlab::new(&device, "placed_lights");
        let terrain_instances = TerrainInstanceBuffer::new(&device);
        let lamp_shadow_cache = LampShadowCache::new(lamp_atlas_resolution);
        let shadow_renderer = ShadowRenderer::new(
            &device,
            shadow_maps,
            lamp_atlas,
            &lighting.shadows.config,
            &world_pass_layout,
            characters.bind_layout(),
            &object_instances.buffer,
            &terrain_instances.buffer,
        );

        // Indirect draw buffers
        let object_indirect = IndirectDrawBuffers::new(&device, "object");
        let terrain_indirect = TerrainIndirectBuffer::new(&device, 512);

        // HiZ pyramid (input to the object cull's occlusion test)
        let hiz_pipeline = HizPipeline::new(
            &device,
            &depth_texture.view,
            config.width.max(1),
            config.height.max(1),
        );

        let local_lights = LocalLightRenderer::new(
            &device,
            cluster_buffers,
            &placed_lights.buffer,
            hiz_pipeline.pyramid_view(),
            HDR_FORMAT,
            &world_pass_layout.pipeline_layout,
        );

        // Cull pipelines
        let object_cull = ObjectCullPipeline::new(&device);
        let terrain_cull = TerrainCullPipeline::new(&device);

        let object_cull_bg = object_cull.create_bind_group(
            &device,
            &object_instances.buffer,
            &object_indirect,
            hiz_pipeline.pyramid_view(),
        );
        let terrain_cull_bg =
            terrain_cull.create_bind_group(&device, &terrain_instances.buffer, &terrain_indirect);

        // Material bind group (texture array + samplers + material buffer + splatmaps)
        let (material_bind_group, shadow_material_bind_group) = Self::build_material_bind_groups(
            &device,
            &world_pass_layout.material_bind_group_layout,
            &texture_array,
            &cube_array,
            &material_sampler,
            &terrain_sampler,
            &shadow_sampler,
            &material_registry.buffer,
            &splatmaps.view,
        );

        // Instance bind group (object + terrain instances)
        let instance_bind_group = Self::build_instance_bind_group(
            &device,
            &world_pass_layout.instance_bind_group_layout,
            &object_instances.buffer,
            &terrain_instances.buffer,
        );

        timer.mark("pipelines_and_buffers")?;

        let mut debug_ui = DebugUi::new(&device, &window, surface_format);
        let lamp_atlas_preview =
            debug_ui.register_texture(&device, shadow_renderer.atlas_preview_view());
        let ui_fonts = UiFonts::load(&layout::fonts_dir(asset_server.asset_location()))?;
        let ui_layout = std::sync::Arc::new(crate::ui_layout::UiLayout::open(Path::new(
            &layout::ui_layout_dir(asset_server.asset_location()),
        ))?);
        ui_layout.validate(&[&pipboy::LAYOUT_KEYS, &hud::LAYOUT_KEYS, &menu::LAYOUT_KEYS])?;
        let hud = hud::HudRenderer::new(
            &device,
            &queue,
            surface_format,
            &ui_fonts,
            &ui_layout,
            settings.ui_colours.tint_for(UiColourSet::HudOverlay),
        )
        .map_err(StartupError::Hud)?;
        let version_overlay = version_overlay::VersionOverlay::new(
            &device,
            &queue,
            surface_format,
            &ui_fonts,
            &ui_layout,
            hud.stage(),
        )?;
        let art_library = Arc::new(
            ArtLibrary::load(Path::new(asset_server.asset_location()))
                .map_err(|error| StartupError::Pipboy(error.into()))?,
        );
        let menu = MenuRenderer::new(
            &device,
            &queue,
            surface_format,
            ui_layout.clone(),
            &ui_fonts,
            &art_library,
            settings.ui_colours.tint_for(UiColourSet::Hud),
            settings.safe_zone,
        )?;
        let menu_visible_rows = menu.layout().list.visible_rows;
        let mut pipboy = PipBoy::new(
            &device,
            surface_format,
            asset_server.asset_location(),
            art_library,
            ui_layout,
            &settings.ui_colours,
            settings.safe_zone,
        )
        .map_err(StartupError::Pipboy)?;
        let pipboy_env = PipBoyEnv::from_env().map_err(StartupError::Pipboy)?;
        let capture = CaptureConfig::from_env()
            .map_err(|error| StartupError::Pipboy(error.into()))?
            .map(|capture_config| PipBoyCapture::new(&capture_config))
            .transpose()
            .map_err(|error| StartupError::Pipboy(error.into()))?;
        if capture.is_some() {
            config.usage |= wgpu::TextureUsages::COPY_SRC;
        }
        let fps_utils = FpsUtils::new(64);
        let fps_limit = 0;
        let target_delta = 1.0 / fps_limit as f32;
        let mut camera_controller = CameraController::new(500.0, 0.0025);

        timer.mark("ui")?;

        let loaded_grid = settings.loaded_grid;
        let mut streamer = WorldStreamer::new(
            &db_path,
            asset_server.asset_location(),
            WORLDSPACE,
            texture_array.resident_paths(),
            texture_policy,
            loaded_grid,
            settings.grass_placement,
        )
        .map_err(StartupError::Streamer)?;
        streamer.set_screen(ScreenView {
            vertical_fov: beth_fov_to_vertical(world_fov_beth).to_radians(),
            width: config.width,
            height: config.height,
        });

        timer.mark("streamer")?;

        let water_world = SharedWaterWorld::new(WaterWorld::new(&db_path));
        let water_stage = WaterStage::new(
            &device,
            &queue,
            scene_copy.layout(),
            asset_server.asset_location(),
            &db_path,
            loaded_grid,
            settings.water,
            water_world.clone(),
        );
        timer.mark("water")?;

        let effect_stage = EffectStage::new(
            &device,
            &queue,
            scene_copy.layout(),
            asset_server.asset_location(),
        );

        let refraction_stage = RefractionStage::new(
            &device,
            &queue,
            scene_copy.layout(),
            asset_server.asset_location(),
        );

        let grass = GrassStage::new(
            &device,
            HDR_FORMAT,
            &world_pass_layout,
            &frame_binding.layout,
            settings.grass_fade_limits,
        );
        timer.mark("effects_and_grass")?;

        // Dev override: OPENFO4_SPAWN_LOOK="yaw_deg,pitch_deg" aims the camera at
        // launch (yaw 0 = +X/east, pitch -90 = straight down). The controller drives
        // the look direction every frame in both walk and fly modes, so set it there;
        // the first-frame camera.target is updated to match so frame 0 already aims.
        let spawn_look = std::env::var("OPENFO4_SPAWN_LOOK").ok().and_then(|raw| {
            let parsed = camera::parse_spawn_look(&raw);
            if parsed.is_none() {
                tracing::warn!(
                    "OPENFO4_SPAWN_LOOK={raw:?} is not `yaw_deg,pitch_deg` with finite numbers; ignoring it"
                );
            }
            parsed
        });
        let spawn_target =
            spawn_cell.map(|target| ResolvedTarget::interior_spawn(target, spawn_look));
        // Dev override: OPENFO4_SPAWN="x,y,z" in Bethesda world units (the
        // placed_objects coordinate space), e.g. straight from a DB query.
        // The default camera faces north (+Y Bethesda), so place the
        // override a little south of whatever should be inspected.
        // With OPENFO4_SPAWN_CELL set, spawn_cell already folded the override
        // into the target position.
        let start_pos = match spawn_target.as_ref() {
            Some(target) => target.start_pos,
            None => match crate::spawn_cell::requested_position()? {
                Some([x, y, z]) => WorldStreamer::beth_to_engine(x as f32, y as f32, z as f32),
                None => streamer.starting_position(),
            },
        };
        camera.eye = start_pos;
        camera.target = start_pos + Vec3::new(0.0, 0.0, -1.0);
        if let Some((yaw, pitch)) = spawn_look {
            camera_controller.set_look(yaw, pitch);
            camera.target = camera.eye + camera_controller.forward();
        }
        // A spawn cell is entered through the door-teleport path on the first
        // update, so the default worldspace ring would only be torn down again.
        // The menu start loads nothing until the player picks a spawn.
        if spawn_target.is_none() && start_mode == StartMode::Direct {
            streamer.initial_load(camera.eye);
        }

        timer.mark("spawn_position")?;

        let StartupDbReads {
            spawn_locations,
            worldspaces,
            interior_lighting,
            interiors,
            teleports,
            player_movement,
            buoyancy_table,
        } = match db_reads.join() {
            Ok(reads) => reads?,
            Err(panic) => std::panic::resume_unwind(panic),
        };
        timer.mark("db_reads_wait")?;

        // The player spawns at the camera start and snaps to the ground as
        // soon as the cell's terrain collision streams in.
        let mut sim = SimWorld::new(player_movement);
        if let Some(table) = buoyancy_table {
            sim.set_material_buoyancy_table(std::sync::Arc::new(table));
        }
        if !water_physics::test_plane_requested() {
            sim.set_water_query(Some(physics_query(water_world)));
        }
        if start_mode == StartMode::Direct {
            sim.apply(SimCommand::SpawnPlayer { pos: start_pos });
            sim.set_fold_hold(true);
        }
        let mut audio = AudioSystem::open_default(
            Path::new(asset_server.asset_location()),
            settings.menu_music,
        )?;
        if let Some(audio) = &mut audio {
            let world_music = load_world_music(
                &db_path,
                Path::new(&layout::audio_dir(asset_server.asset_location())),
            )
            .map_err(AudioError::from)?;
            audio.set_world_music(world_music);
        }
        let footstep_catalog = crate::assets::db::DbClient::new(&db_path)
            .and_then(|db| db.get_footstep_catalog())
            .map_err(StartupError::Footsteps)?;
        let gait_timings = load_gait_timings(&format!(
            "{}/{FIRST_PERSON_ASSETS_DIR}",
            asset_server.asset_location()
        ))
        .map_err(|error| StartupError::Footsteps(error.into()))?;
        if let Some(mismatch) = crate::sim::jump_windup_mismatch(&gait_timings) {
            tracing::warn!("{mismatch}");
        }
        sim.install_gait_timings(gait_timings);
        let foot_surface = foot_surface::FootSurface::new(
            crate::assets::db::DbClient::new(&db_path).map_err(StartupError::Footsteps)?,
            footstep_catalog.landscape_materials.clone(),
        );
        if let Some(audio) = &mut audio {
            let footsteps = Footsteps::new(
                footstep_catalog,
                Path::new(&layout::audio_dir(asset_server.asset_location())),
                seed_from_clock(),
            )
            .map_err(AudioError::from)?;
            audio.set_footsteps(footsteps);
        }
        let menu_video = MenuVideo::new(
            settings.menu_video,
            Path::new(asset_server.asset_location()),
        );
        let mut flow = GameFlow::new(spawn_locations, menu_visible_rows, start_mode);
        if capture.is_some() {
            flow.hold_at_main_menu();
        }
        let pipboy_catalog = crate::assets::db::DbClient::new(&db_path)
            .and_then(|db| crate::assets::pipboy_catalog::load_pipboy_catalog(&db))
            .map_err(StartupError::Pipboy)?;
        if let Some(map) = &pipboy_catalog.world_map {
            pipboy.load_world_map(&device, &queue, &map.texture_key)?;
            pipboy.require_marker_art(map.markers.iter().map(|marker| marker.marker_type))?;
        }
        pipboy.load_perk_grid(&device, &queue, surface_format)?;
        sim.set_pipboy_catalog(std::sync::Arc::new(pipboy_catalog));

        let cull_stats = CullStatsReadback::new(&device);
        let gpu_profiler = GpuProfiler::new(&device, &queue);
        timer.mark("sim_and_profiler")?;

        let material_bg_built_len = texture_array.bind_group_len();
        Ok(Self {
            surface,
            device,
            device_loss,
            queue,
            config,
            is_surface_configured: false,
            window,
            world_pass_layout,
            object_pipelines,
            terrain_pipeline,
            debug_line_pipeline,
            skeleton_overlay,
            camera,
            camera_controller,
            lighting,
            interior_lighting,
            frame_binding,
            shadow_renderer,
            lamp_shadow_cache,
            lamp_candidates: Vec::new(),
            dynamic_poses: Vec::new(),
            lamp_cull_slots: Vec::new(),
            lamp_atlas_preview,
            local_lights,
            world_seconds: 0.0,
            post,
            scene_copy,
            water_stage,
            effect_stage,
            refraction_stage,
            latest_frame: None,
            addon_fx: AddonFx::new(asset_server.asset_location()),
            light_anim: LightAnimator::new(asset_server.asset_location()),
            grass,
            bokeh_gate: settings.bokeh_gate,
            cached_vp: Mat4::IDENTITY,
            depth_texture,
            hiz_pipeline,
            hiz_valid: false,
            debug_skip: DebugSkip::from_env(),
            object_mega,
            terrain_mega,
            object_instances,
            placed_lights,
            terrain_instances,
            texture_array,
            cube_array,
            texture_residency,
            texture_feedback,
            max_anisotropy: u32::from(texture_streaming.max_anisotropy),
            splatmaps,
            material_registry,
            characters,
            skinned_pipelines,
            anim_panel: AnimDebugPanel::default(),
            object_indirect,
            terrain_indirect,
            object_cull,
            terrain_cull,
            object_cull_bg,
            terrain_cull_bg,
            material_bind_group,
            shadow_material_bind_group,
            instance_bind_group,
            material_sampler,
            terrain_sampler,
            shadow_sampler,
            material_bg_dirty: false,
            material_bg_last_rebuild: Instant::now(),
            pending_slot_changes: SlotChanges::default(),
            pending_alpha_slots: Vec::new(),
            material_bg_built_len,
            streamer,
            worldspaces,
            interiors,
            pending_worldspace: None,
            pending_interior: None,
            world_mode: DebugWorldMode::Worldspace,
            interior_filter: String::new(),
            teleports,
            pending_teleport: spawn_target,
            auto_activate: std::env::var("OPENFO4_AUTO_ACTIVATE")
                .ok()
                .map(|s| {
                    s.split(',')
                        .map(|x| x.trim().to_string())
                        .filter(|x| !x.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            auto_idx: 0,
            auto_cooldown: 0,
            animated_objects: AnimatedObjects::default(),
            ragdoll_objects: RagdollObjects::default(),
            auto_anim: AutoAnim::from_env(),
            auto_characters: AutoCharacters::from_env(),
            sim,
            input_mode: InputMode::Walk,
            freeze_camera,
            walk_keys: WalkKeys::default(),
            world_fov_beth,
            fov_adjust: 0.0,
            collision_wireframe: false,
            navmesh_overlay: NavmeshOverlay::new(crate::utils::env_flag(
                NAVMESH_OVERLAY_ENV,
                false,
            )),
            collision_wire_radius: 10000.0,
            collision_lines: LineBuffer::new("collision_wireframe_lines"),
            collision_wire_gen: 0,
            collision_wire_center: Vec3::ZERO,
            collision_wire_built_radius: 0.0,
            collision_wire_last_rebuild: Instant::now(),
            collision_wire_truncated: false,
            hud,
            version_overlay,
            pipboy,
            pipboy_env,
            capture,
            pipboy_was_open: false,
            capture_before_pipboy: false,
            flow,
            audio,
            foot_surface,
            music_poll: PlacePoll::new(PLACE_POLLS_PER_SECOND),
            menu_video,
            menu,
            pointer_px: (0.0, 0.0),
            load_spawn_cell: WorldStreamer::camera_to_grid(start_pos),
            quit_requested: false,
            debug_ui,
            mouse_capture,
            fps_utils,
            perf_log: perf_log_enabled.then(PerfLog::new),
            fps_limit,
            target_delta,
            show_markers: false,
            show_fx: false,
            decal_bias_enabled: true,
            toast_queue: Vec::new(),
            process_memory: utils::ProcessMemory::new(),
            vram_text: String::new(),
            memory_section_open: false,
            aimed_door: false,
            vram_last_refresh: Instant::now(),
            cull_stats,
            gpu_profiler,
        })
    }

    /// The world-pass material bind group plus, unless masking is disabled
    /// (`SHADOW_ALPHA_MASK_ENV`), the one the shadow passes bind: identical
    /// except that textures no alpha-tested caster samples are stand-ins.
    fn build_material_bind_groups(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        texture_array: &TextureArray,
        cube_array: &CubeArray,
        material_sampler: &wgpu::Sampler,
        terrain_sampler: &wgpu::Sampler,
        shadow_sampler: &wgpu::Sampler,
        material_buffer: &wgpu::Buffer,
        splatmap_view: &wgpu::TextureView,
    ) -> (wgpu::BindGroup, Option<wgpu::BindGroup>) {
        let slot_mips_buffer = texture_array.slot_mips_buffer();
        let cube_views = cube_array.views();
        let build = |label: &str, views: &[&wgpu::TextureView]| {
            Self::build_material_bind_group(
                device,
                layout,
                label,
                views,
                &cube_views,
                material_sampler,
                terrain_sampler,
                shadow_sampler,
                material_buffer,
                splatmap_view,
                slot_mips_buffer,
            )
        };
        let world = build(
            "material_bind_group",
            &texture_array.build_bind_group_entries(),
        );
        let shadow = shadow_alpha_mask_enabled().then(|| {
            build(
                "shadow_material_bind_group",
                &texture_array.build_shadow_bind_group_entries(),
            )
        });
        (world, shadow)
    }

    fn build_material_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        label: &str,
        view_refs: &[&wgpu::TextureView],
        cube_view_refs: &[&wgpu::TextureView],
        material_sampler: &wgpu::Sampler,
        terrain_sampler: &wgpu::Sampler,
        shadow_sampler: &wgpu::Sampler,
        material_buffer: &wgpu::Buffer,
        splatmap_view: &wgpu::TextureView,
        slot_mips_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureViewArray(view_refs),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(material_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: material_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(terrain_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(splatmap_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(shadow_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: slot_mips_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureViewArray(cube_view_refs),
                },
            ],
        })
    }

    fn build_instance_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        object_buffer: &wgpu::Buffer,
        terrain_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("instance_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: object_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: terrain_buffer.as_entire_binding(),
                },
            ],
        })
    }

    fn extract_frustum_planes(frustum_mat: &Mat4) -> [[f32; 4]; 5] {
        let [left, right, bottom, top, _infinity, near] = frustum::clip_planes(frustum_mat);
        // With the infinite reverse-Z projection the depth-0 plane lies at
        // infinity (zero normal, culls nothing); the real near plane is depth 1.
        [left, right, bottom, top, near].map(|plane| plane.to_array())
    }

    #[tracing::instrument(skip_all, name = "render")]
    pub fn render(&mut self, _asset_server: &mut AssetServer) -> anyhow::Result<()> {
        self.device_loss.check()?;
        let frame_start = Instant::now();

        self.window.request_redraw();

        if !self.is_surface_configured {
            return Ok(());
        }

        // Sub-spans split the frame cost: swapchain back-pressure shows up in
        // `acquire`/`present`, staging-flush cost in `submit`.
        let output = {
            let _span = tracing::info_span!("acquire").entered();
            match self.surface.acquire_frame(&self.device, &self.config) {
                Some(frame) => frame,
                None => return Ok(()),
            }
        };

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        if self.flow.phase().shows_menu() {
            self.render_menu_frame(output, &view, frame_start);
            return Ok(());
        }

        self.prepare_effects()?;

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        let encode_cull_span = tracing::info_span!("encode_cull").entered();
        // Clear draw counts unconditionally: the cull dispatches below are skipped
        // at zero instances, and stale counts would replay last frame's draws.
        self.object_indirect.clear_counts(&mut encoder);
        self.terrain_indirect.clear_counts(&mut encoder);
        self.gpu_profiler.begin_frame();

        let frustum_planes = Self::extract_frustum_planes(&self.cached_vp);
        let base_uniforms = CullUniforms {
            frustum_planes,
            total_instances: 0,
            show_markers: u32::from(self.show_markers),
            decal_bias_enabled: u32::from(self.decal_bias_enabled),
            show_fx: u32::from(self.show_fx),
            view_proj: self.cached_vp.to_cols_array_2d(),
            screen_size: [self.config.width as f32, self.config.height as f32],
            hiz_mip_count: self.hiz_pipeline.mip_count(),
            hiz_valid: u32::from(self.hiz_valid),
        };

        // GPU cull: objects. The dispatch covers the slab's high-water cursor
        // (holes self-cull via index_count == 0), but only while anything is
        // actually live.
        let skip = self.debug_skip;
        let object_count = self.object_instances.dispatch_count();
        if !skip.skips(DebugPass::Cull)
            && object_count > 0
            && self.object_instances.live_count() > 0
        {
            let cull_uniforms = CullUniforms {
                total_instances: object_count,
                ..base_uniforms
            };
            self.object_cull.dispatch(
                &mut encoder,
                &self.object_cull_bg,
                &cull_uniforms,
                &self.queue,
                &self.gpu_profiler,
            );
        }

        // GPU cull: terrain
        let terrain_count = self.terrain_instances.len();
        if !skip.skips(DebugPass::Cull) && terrain_count > 0 {
            let cull_uniforms = CullUniforms {
                total_instances: terrain_count,
                ..base_uniforms
            };
            self.terrain_cull.dispatch(
                &mut encoder,
                &self.terrain_cull_bg,
                &cull_uniforms,
                &self.queue,
                &self.gpu_profiler,
            );
        }

        if !skip.skips(DebugPass::Lights) {
            self.local_lights.encode(
                &mut encoder,
                &self.queue,
                &LocalLightFrame {
                    camera: &self.camera,
                    view_proj: self.cached_vp,
                    frustum_planes,
                    screen_size: base_uniforms.screen_size,
                    hiz_mip_count: base_uniforms.hiz_mip_count,
                    hiz_valid: self.hiz_valid,
                    slot_count: self.placed_lights.dispatch_count(),
                    lights_loaded: self.placed_lights.live_count(),
                    settings: &self.lighting.local_lights,
                    shadow_lamps: &self.lamp_cull_slots,
                },
                &self.gpu_profiler,
            );
        }

        // Copy post-cull counts for the debug-UI stats (encoded after the cull
        // dispatches, so the copies observe this frame's counts).
        self.cull_stats.encode_copy(
            &mut encoder,
            &self.object_indirect.count_buffer,
            &self.terrain_indirect.count_buffer,
        );

        drop(encode_cull_span);

        {
            let _span = tracing::info_span!("shadows").entered();
            if !skip.skips(DebugPass::Shadows) {
                self.shadow_renderer.encode(
                    &mut encoder,
                    &self.queue,
                    &ShadowScene {
                        material_bind_group: self
                            .shadow_material_bind_group
                            .as_ref()
                            .unwrap_or(&self.material_bind_group),
                        instance_bind_group: &self.instance_bind_group,
                        object_mega: &self.object_mega,
                        terrain_mega: &self.terrain_mega,
                        object_count: self.object_instances.dispatch_count(),
                        terrain_count: self.terrain_instances.len(),
                        skinned: self.characters.scene(),
                    },
                    &self.gpu_profiler,
                );
                if self.lighting.lamp_shadows.settings.show_atlas {
                    self.shadow_renderer.encode_atlas_preview(&mut encoder);
                }
            }
        }

        let encode_world_span = tracing::info_span!("encode_world_pass").entered();
        let world_skipped = skip.skips(DebugPass::World);
        let pipelines = self.object_pipelines.pipelines_in_bucket_order();
        let skinned = self.characters.scene();
        let skinned_pipelines = &self.skinned_pipelines;

        // Every pipeline of both world passes shares the world pass layout, so
        // the groups are bound once per pass.
        let bind_world_groups = |pass: &mut wgpu::RenderPass| {
            pass.set_bind_group(0, &self.material_bind_group, &[]);
            self.frame_binding.bind(pass);
            pass.set_bind_group(2, &self.instance_bind_group, &[]);
            pass.set_bind_group(FEEDBACK_GROUP, self.texture_feedback.bind_group(), &[]);
        };
        let bind_object_mega = |pass: &mut wgpu::RenderPass, mega: &'_ MegaBuffer| {
            pass.set_vertex_buffer(0, mega.vertex_buffer.slice(..));
            pass.set_index_buffer(mega.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        };
        let draw_bucket = |pass: &mut wgpu::RenderPass, bucket: usize| {
            pass.set_pipeline(pipelines[bucket]);
            // Cap executed draws at this bucket's capacity — safe even if the
            // GPU count overran, since the shader dropped writes past it.
            pass.multi_draw_indexed_indirect_count(
                &self.object_indirect.draw_buffers[bucket],
                0,
                &self.object_indirect.count_buffer,
                (bucket * std::mem::size_of::<u32>()) as u64,
                self.object_indirect.capacity(bucket),
            );
        };
        // Characters swap the instance group for their own; the static draws
        // after them need theirs back.
        let show_fx = self.show_fx;
        let draw_skinned = |pass: &mut wgpu::RenderPass, blended: bool| {
            let drew = skinned.draw(pass, Visibility::Camera, |part| {
                let material = &part.material;
                (material.kind.is_blended() == blended && (show_fx || !part.hidden_by_default))
                    .then(|| skinned_pipelines.pipeline(material.kind, material.two_sided))
            });
            if drew {
                pass.set_bind_group(2, &self.instance_bind_group, &[]);
            }
        };

        // Opaque world pass: clears colour and depth, draws the objects, terrain,
        // decals and sky the water stage may sample through the scene copy. Grass
        // follows in its own pass; `world` timing excludes it.
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.post.hdr_view(),
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color(self.lighting.background_color())),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                occlusion_query_set: None,
                timestamp_writes: self
                    .gpu_profiler
                    .render_writes(GpuPass::World, SpanPart::Whole),
                multiview_mask: None,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_texture.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
            });

            if !world_skipped {
                bind_world_groups(&mut render_pass);

                // Draw order matters: depth-writing object buckets first, then terrain,
                // then the decals, and the translucent blend buckets in the second pass.
                // Overlays don't write depth, so terrain drawn after them would overwrite
                // their pixels wherever it lies behind; equally, drawing decals before
                // terrain would let terrain cover ground decals (they no longer write
                // depth to win against it).
                bind_object_mega(&mut render_pass, &self.object_mega);
                for bucket in 0..gpu_driven::FIRST_AFTER_TERRAIN_BUCKET {
                    draw_bucket(&mut render_pass, bucket);
                }
                draw_skinned(&mut render_pass, false);

                // Terrain
                render_pass.set_pipeline(&self.terrain_pipeline.pipeline);
                render_pass.set_vertex_buffer(0, self.terrain_mega.vertex_buffer.slice(..));
                render_pass.set_index_buffer(
                    self.terrain_mega.index_buffer.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                render_pass.multi_draw_indexed_indirect_count(
                    &self.terrain_indirect.draw_buffer,
                    0,
                    &self.terrain_indirect.count_buffer,
                    0,
                    self.terrain_indirect.max_draws(),
                );

                // Decals belong to the opaque scene the water covers: drawn after the
                // water they would show through it (submerged rock and riverbed decals).
                bind_object_mega(&mut render_pass, &self.object_mega);
                for bucket in gpu_driven::DECAL_BUCKETS {
                    draw_bucket(&mut render_pass, bucket);
                }

                // Sky fills every pixel no geometry touched, so the blend buckets of
                // the second pass composite over it. Interiors show the clear colour
                // instead.
                if self.lighting.draws_sky() {
                    self.post.draw_sky(&mut render_pass);
                }
            }
        }

        // Grass gets its own pass so it is timed apart from `world`. It loads the
        // world pass's colour and depth and writes depth like the opaque objects, so
        // the water passes and the next frame's HiZ see it. The sky test only passes
        // where nothing wrote depth, so drawing grass after it changes no pixel.
        if !world_skipped {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Grass Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.post.hdr_view(),
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                occlusion_query_set: None,
                timestamp_writes: self
                    .gpu_profiler
                    .render_writes(GpuPass::Grass, SpanPart::Whole),
                multiview_mask: None,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_texture.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
            });
            // Grass owns bind group 3, which the object pipelines use for the
            // texture feedback.
            bind_world_groups(&mut render_pass);
            self.grass.sync_controls(&self.queue);
            self.grass.draw(
                &mut render_pass,
                &GrassView {
                    eye: self.camera.eye,
                    planes: frustum::clip_planes(&self.cached_vp),
                },
            );
        }

        if !skip.skips(DebugPass::WaterDisplacement) {
            self.water_stage
                .encode_displacement(&mut encoder, &self.gpu_profiler);
        }

        if !world_skipped && !skip.skips(DebugPass::Water) {
            self.water_stage.encode_prepasses(
                &self.device,
                &mut encoder,
                &self.scene_copy,
                self.post.hdr_view(),
                &self.depth_texture.view,
                &self.object_mega,
            );
        }

        if !world_skipped
            && self.scene_copy.wanted(
                self.water_stage.has_work()
                    || self.effect_stage.needs_scene_copy()
                    || self.refraction_stage.has_work(),
            )
        {
            self.scene_copy.encode(
                &mut encoder,
                self.post.hdr_texture(),
                &self.depth_texture.texture,
            );
        }

        if !world_skipped && !skip.skips(DebugPass::Water) && !skip.skips(DebugPass::WaterSsr) {
            self.water_stage.encode_ssr(
                &self.device,
                &mut encoder,
                &self.scene_copy,
                &self.depth_texture.view,
                &self.object_mega,
                &self.gpu_profiler,
            );
        }

        // Translucent pass: loads the opaque result, then the water stage, the
        // overlay buckets and the debug lines. Timed as `water`, so it includes
        // the overlay buckets.
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Translucent Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: self.post.hdr_view(),
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                occlusion_query_set: None,
                timestamp_writes: self
                    .gpu_profiler
                    .render_writes(GpuPass::Water, SpanPart::Whole),
                multiview_mask: None,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_texture.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
            });

            if !world_skipped {
                bind_world_groups(&mut render_pass);

                if !skip.skips(DebugPass::Water)
                    && self
                        .water_stage
                        .draw(&mut render_pass, &self.scene_copy, &self.object_mega)
                {
                    bind_world_groups(&mut render_pass);
                }

                // Blended buckets (the water stage may have switched the
                // vertex/index buffers — rebind). These depth-test but don't
                // depth-write.
                bind_object_mega(&mut render_pass, &self.object_mega);
                for bucket in gpu_driven::TRANSLUCENT_BUCKETS {
                    draw_bucket(&mut render_pass, bucket);
                }
                draw_skinned(&mut render_pass, true);

                // Heat haze: distorts the opaque scene copy, before the effects.
                if self
                    .refraction_stage
                    .draw(&mut render_pass, &self.scene_copy)
                {
                    bind_world_groups(&mut render_pass);
                }

                // Effect shapes and particles: after every other translucent
                // draw, sorted among themselves.
                if self.effect_stage.draw(&mut render_pass, &self.scene_copy) {
                    bind_world_groups(&mut render_pass);
                }

                // Collision wireframe overlay.
                if self.collision_wireframe {
                    self.collision_lines
                        .draw(&mut render_pass, &self.debug_line_pipeline.pipeline);
                }
                self.navmesh_overlay
                    .draw(&mut render_pass, &self.debug_line_pipeline.pipeline);

                self.local_lights
                    .draw_spheres(&mut render_pass, &self.lighting.local_lights);
            }
        }
        if !world_skipped && !skip.skips(DebugPass::DepthOfField) {
            let underwater = self
                .water_stage
                .underwater_dof()
                .filter(|_| !skip.skips(DebugPass::Water));
            let camera = DofCamera {
                near: self.camera.znear * UNITS_FROM_ENGINE,
                far: self.camera.zfar * UNITS_FROM_ENGINE,
            };
            if let Some(frame) = select_frame(
                self.lighting.depth_of_field(),
                underwater.as_ref().map(|inputs| inputs.image_space),
                self.bokeh_gate,
                camera,
                self.post.target_size(),
            ) {
                self.post.encode_depth_of_field(
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    &self.depth_texture.view,
                    underwater.and_then(|inputs| inputs.surface_depth),
                    &frame,
                );
            }
        }
        drop(encode_world_span);

        self.skeleton_overlay
            .encode(&mut encoder, self.post.hdr_view(), &self.frame_binding);

        let encode_post_span = tracing::info_span!("encode_post").entered();
        // HiZ pyramid for next frame's occlusion culling.
        if !skip.skips(DebugPass::Hiz) {
            self.hiz_pipeline.generate(&mut encoder, &self.gpu_profiler);
        }

        if !skip.skips(DebugPass::Exposure) {
            self.post
                .encode_auto_exposure(&mut encoder, &self.gpu_profiler);
        }

        let tonemap_view = output.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.post.tonemap_target_format()),
            ..Default::default()
        });
        if !skip.skips(DebugPass::Tonemap) {
            self.post.encode_tonemap(
                &mut encoder,
                &tonemap_view,
                &self.frame_binding,
                &self.gpu_profiler,
            );
        }

        // Native HUD overlay. Color-only, LoadOp::Load, no depth: the HiZ
        // pyramid above already consumed the depth buffer, so the HUD must not
        // touch it. Drawn before egui so the developer menu paints over the HUD.
        // The Pip-Boy replaces the HUD while it is open.
        let pipboy_open = self.sim.pipboy_snapshot().model.open;
        self.pipboy.sync_open(pipboy_open);
        if pipboy_open {
            if !skip.skips(DebugPass::PipBoy) {
                let snapshot = self.sim.pipboy_snapshot();
                self.pipboy.render(
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    &view,
                    [self.config.width, self.config.height],
                    &self.gpu_profiler,
                    &snapshot,
                );
            }
        } else if !skip.skips(DebugPass::Hud) {
            self.hud.render(
                &self.device,
                &self.queue,
                &mut encoder,
                &view,
                &self.gpu_profiler,
            );
        }
        self.flush_pipboy_actions();
        drop(encode_post_span);

        // The black layer of the loading screen and the reveal fade, over the world and
        // the HUD and under the debug UI.
        let overlay_alpha = self.flow.overlay_alpha();
        if overlay_alpha > 0.0 {
            let menu_view = self.menu.target_view(&output.texture);
            self.menu.render(
                &self.device,
                &self.queue,
                &mut encoder,
                &menu_view,
                (self.config.width, self.config.height),
                MenuScene::Overlay {
                    alpha: overlay_alpha,
                    loading_seconds: self.flow.loading_seconds(),
                },
            );
        }

        self.render_version_overlay(&mut encoder, &view);

        // Queued toasts must register before debug_ui.render so they paint
        // inside this frame's egui pass.
        self.show_queued_toasts();

        // Refresh the (expensive) allocator report at most twice a second.
        if self.memory_section_open
            && (self.vram_text.is_empty()
                || self.vram_last_refresh.elapsed() > Duration::from_millis(500))
        {
            let _span = tracing::info_span!("vram_report").entered();
            self.vram_last_refresh = Instant::now();
            self.vram_text = match self.device.generate_allocator_report() {
                Some(report) => format!(
                    "VRAM allocated: {}\nVRAM reserved: {}",
                    utils::format_bytes(report.total_allocated_bytes),
                    utils::format_bytes(report.total_reserved_bytes)
                ),
                None => String::from("VRAM: unavailable"),
            };
        }

        // Compute the load-door activation state up front: a &self method can't
        // be called inside the egui closure once `self.debug_ui` is borrowed
        // mutably by render, so capture the plain bool here.
        let dbg_interactible = self.aimed_door;
        let developer_window_visible =
            self.flow.developer_window_visible() && !self.capture_active();
        let mut memory_section_open = false;

        // Debug UI
        let egui_span = tracing::info_span!("egui").entered();
        if !skip.skips(DebugPass::Egui) {
            self.debug_ui.render(
                &self.window,
                &self.device,
                &self.queue,
                &self.config,
                &mut encoder,
                &view,
                &self.gpu_profiler,
                |ctx| {
                    if !developer_window_visible {
                        return;
                    }
                    egui::Window::new("Developer Menu")
                        .default_open(false)
                        .show(ctx, |ui| {
                        let total_objects = self.object_instances.live_count();
                        let total_terrain = self.terrain_instances.len();

                        egui::CollapsingHeader::new("Performance")
                            .default_open(true)
                            .show(ui, |ui| {
                                ui.label(format!("FPS: {:.1}", self.fps_utils.fps()));
                                let response = ui.add(
                                    egui::DragValue::new(&mut self.fps_limit)
                                        .speed(1)
                                        .prefix("FPS Limit: "),
                                );
                                if response.changed() {
                                    self.fps_utils.clear_history();
                                    self.target_delta = 1.0 / self.fps_limit as f32;
                                }
                                egui::CollapsingHeader::new("GPU timings")
                                    .default_open(false)
                                    .show(ui, |ui| gpu_profiler_section(ui, &self.gpu_profiler));
                            });

                        egui::CollapsingHeader::new("Debug view")
                            .default_open(false)
                            .show(ui, |ui| {
                                debug_view::debug_view_section(
                                    ui,
                                    &mut self.lighting.debug_view,
                                    self.texture_feedback.enabled(),
                                );
                            });

                        let memory = egui::CollapsingHeader::new("Memory")
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.label(format!(
                                    "RAM allocated: {}",
                                    utils::format_bytes(self.process_memory.bytes())
                                ));
                                ui.label(&self.vram_text);
                            });
                        memory_section_open = memory.body_returned.is_some();

                        egui::CollapsingHeader::new("Textures & Materials")
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.label(format!(
                                    "Textures: {} live / {} peak / {}",
                                    self.texture_array.live_slots(),
                                    self.texture_array.used_slots(),
                                    MAX_TEXTURES
                                ));
                                ui.label(format!(
                                    "Texture memory: {} resident, {} reads pending",
                                    utils::format_bytes(
                                        self.texture_residency.stats.resident_bytes
                                    ),
                                    self.texture_residency.stats.pending,
                                ));
                                ui.label(format!(
                                    "Splat layers: {} live",
                                    self.splatmaps.live_layers()
                                ));
                                ui.label(format!("Materials: {}", self.material_registry.len()));
                            });

                        egui::CollapsingHeader::new("Instances")
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.label(format!(
                                    "Object instances: {} (slab {}/{}, {} holes)",
                                    total_objects,
                                    self.object_instances.dispatch_count(),
                                    self.object_instances.capacity(),
                                    self.object_instances.hole_count(),
                                ));
                                ui.label(format!(
                                    "Placed lights: {} (slab {}/{}, {} holes)",
                                    self.placed_lights.live_count(),
                                    self.placed_lights.dispatch_count(),
                                    self.placed_lights.capacity(),
                                    self.placed_lights.hole_count(),
                                ));
                                ui.label(format!("Terrain instances: {total_terrain}"));
                            });

                        egui::CollapsingHeader::new("Culling & Meshes")
                            .default_open(false)
                            .show(ui, |ui| {
                                // GPU cull results, ~2 frames stale.
                                let stats = &self.cull_stats.latest;
                                let visible = stats.objects_total();
                                let culled_pct = if total_objects > 0 {
                                    100.0 * (1.0 - visible as f32 / total_objects as f32)
                                } else {
                                    0.0
                                };
                                ui.label(format!(
                                    "Visible objects: {visible} / {total_objects} ({culled_pct:.0}% culled)"
                                ));
                                ui.label(format!(
                                    "  opaque {} / atest {} / blend {} / decal {}",
                                    stats.object_buckets[0] + stats.object_buckets[1],
                                    stats.object_buckets[2] + stats.object_buckets[3],
                                    stats.object_buckets[6] + stats.object_buckets[7],
                                    stats.object_buckets[4]
                                        + stats.object_buckets[5]
                                        + stats.object_buckets[8]
                                        + stats.object_buckets[9],
                                ));
                                ui.label(format!(
                                    "Visible terrain: {} / {}",
                                    stats.terrain, total_terrain
                                ));
                                ui.label(format!(
                                    "Unique meshes (objects): {}",
                                    self.object_mega.mesh_count()
                                ));
                                ui.label(format!(
                                    "Unique meshes (terrain): {}",
                                    self.terrain_mega.mesh_count()
                                ));
                                ui.label(format!(
                                    "Object mega: {:.0}M / {:.0}M verts",
                                    self.object_mega.vertex_used() as f32 / 1e6,
                                    self.object_mega.vertex_capacity() as f32 / 1e6,
                                ));
                                ui.label(format!(
                                    "Terrain mega: {:.1}M / {:.1}M verts",
                                    self.terrain_mega.vertex_used() as f32 / 1e6,
                                    self.terrain_mega.vertex_capacity() as f32 / 1e6,
                                ));
                            });

                        egui::CollapsingHeader::new("World")
                            .default_open(false)
                            .show(ui, |ui| {
                                // World mode: Worldspace streams the open world; Interior
                                // drops it and loads one interior cell as a walkable unit.
                                // Flipping the toggle only swaps which picker shows — nothing
                                // loads until a row is picked.
                                ui.horizontal(|ui| {
                                    ui.label("World mode:");
                                    ui.selectable_value(
                                        &mut self.world_mode,
                                        DebugWorldMode::Worldspace,
                                        "Worldspace",
                                    );
                                    ui.selectable_value(
                                        &mut self.world_mode,
                                        DebugWorldMode::Interior,
                                        "Interior",
                                    );
                                });
                                match self.world_mode {
                                    DebugWorldMode::Worldspace => {
                                        ui.horizontal(|ui| {
                                            ui.label("Worldspace:");
                                            let current = self.streamer.worldspace.clone();
                                            let selected_text = self
                                                .worldspaces
                                                .iter()
                                                .find(|w| w.form_key == current)
                                                .map_or(current.as_str(), |w| w.label.as_str());
                                            let mut chosen = current.clone();
                                            egui::ComboBox::from_id_salt("worldspace_select")
                                                .selected_text(selected_text)
                                                .show_ui(ui, |ui| {
                                                    for opt in &self.worldspaces {
                                                        ui.selectable_value(
                                                            &mut chosen,
                                                            opt.form_key.clone(),
                                                            &opt.label,
                                                        );
                                                    }
                                                });
                                            // Re-picking the current worldspace while inside an
                                            // interior is the way back out, so also fire when
                                            // in an interior even if the key is unchanged.
                                            if chosen != current || self.streamer.in_interior() {
                                                self.pending_worldspace = Some(chosen);
                                            }
                                        });
                                    }
                                    DebugWorldMode::Interior => {
                                        ui.horizontal(|ui| {
                                            ui.label("Filter:");
                                            ui.text_edit_singleline(&mut self.interior_filter);
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label("Interior:");
                                            let needle = self.interior_filter.to_ascii_lowercase();
                                            // The streamer's loaded interior is the single
                                            // source of truth for the current selection, so the
                                            // button text clears itself the moment we leave one.
                                            let current = self.streamer.current_interior();
                                            let selected_text = current
                                                .and_then(|fk| {
                                                    self.interiors.iter().find(|i| i.form_key == fk)
                                                })
                                                .map_or("(pick interior)", |i| i.label.as_str());
                                            // Collect the click after the combo closure so the
                                            // borrow of self.interiors ends before we write
                                            // pending_interior.
                                            let mut picked: Option<String> = None;
                                            egui::ComboBox::from_id_salt("interior_select")
                                                .selected_text(selected_text)
                                                .show_ui(ui, |ui| {
                                                    for opt in &self.interiors {
                                                        if !needle.is_empty()
                                                            && !opt
                                                                .label
                                                                .to_ascii_lowercase()
                                                                .contains(&needle)
                                                        {
                                                            continue;
                                                        }
                                                        let selected =
                                                            current == Some(opt.form_key.as_str());
                                                        if ui
                                                            .selectable_label(selected, &opt.label)
                                                            .clicked()
                                                        {
                                                            picked = Some(opt.form_key.clone());
                                                        }
                                                    }
                                                });
                                            if let Some(fk) = picked {
                                                self.pending_interior = Some(fk);
                                            }
                                        });
                                    }
                                }
                            });

                        egui::CollapsingHeader::new("Streaming")
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.label(format!(
                                    "Loaded cells: {} ({} full + {} LOD)",
                                    self.streamer.loaded_cell_count()
                                        + self.streamer.lod_loaded_cell_count(),
                                    self.streamer.loaded_cell_count(),
                                    self.streamer.lod_loaded_cell_count(),
                                ));
                                ui.label(format!(
                                    "Pending: {} full + {} LOD",
                                    self.streamer.pending_cell_count(),
                                    self.streamer.lod_pending_cell_count(),
                                ));
                                let (full_rate, lod_rate) = self.streamer.spawn_rates();
                                ui.label(format!(
                                    "Spawn rate: {full_rate:.0} full/s, {lod_rate:.0} LOD/s",
                                ));
                                // Spawn mode: Loading floors the integrate/upload
                                // budgets and wakes the whole worker pool; Streaming
                                // (gameplay default) keeps one worker and the
                                // hitch-free budgets. Picking either pins it — the
                                // SPAWN_COMPLETE auto-drop only moves the boot default.
                                ui.horizontal(|ui| {
                                    ui.label("Spawn mode:");
                                    let mut mode = self.streamer.mode();
                                    let changed = ui
                                        .selectable_value(
                                            &mut mode,
                                            LoadMode::Streaming,
                                            "Streaming",
                                        )
                                        .changed()
                                        | ui.selectable_value(
                                            &mut mode,
                                            LoadMode::Loading,
                                            "Loading",
                                        )
                                        .changed();
                                    if changed {
                                        self.streamer.set_mode(mode);
                                    }
                                });
                                ui.label(format!(
                                    "Workers: {}/{} active",
                                    self.streamer.active_worker_count(),
                                    self.streamer.pool_size(),
                                ));
                                ui.add(
                                    egui::DragValue::new(self.streamer.max_cells_per_frame_mut())
                                        .range(1..=100)
                                        .prefix("Cells/frame"),
                                );
                                // Ranges keep (2*(stream+lod)+1)^2 within the 2048-layer
                                // splatmap array; beyond it splats degrade to the fallback.
                                ui.add(
                                    egui::DragValue::new(self.streamer.stream_radius_mut())
                                        .speed(1)
                                        .range(1..=20)
                                        .prefix("High Detail Radius: "),
                                );
                                ui.add(
                                    egui::DragValue::new(self.streamer.lod_radius_mut())
                                        .speed(1)
                                        .range(0..=50)
                                        .prefix("LOD Radius: "),
                                );
                            });

                        egui::CollapsingHeader::new("Markers & FX")
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.label(format!(
                                    "Marker instances ({}): {}",
                                    if self.show_markers { "shown" } else { "hidden" },
                                    self.streamer.marker_instance_count(),
                                ));
                                ui.checkbox(&mut self.show_markers, "Show markers (M)");
                                ui.label(format!(
                                    "Hidden FX instances ({}): {}",
                                    if self.show_fx { "shown" } else { "hidden" },
                                    self.streamer.fx_instance_count(),
                                ));
                                ui.checkbox(&mut self.show_fx, "Show FX (F)");
                                ui.checkbox(&mut self.decal_bias_enabled, "Decal depth bias (B)");
                            });

                        egui::CollapsingHeader::new("Player")
                            .default_open(false)
                            .show(ui, |ui| {
                                let pd = self.sim.player_debug();
                                let player_state = if !pd.spawned && pd.waiting_for_ground {
                                    "waiting for ground"
                                } else if !pd.spawned {
                                    "unspawned"
                                } else if pd.waiting_for_ground {
                                    "teleporting"
                                } else if pd.grounded {
                                    "grounded"
                                } else {
                                    "airborne"
                                };
                                ui.label(format!("Player State: {player_state}"));
                                ui.label(format!(
                                    "Position: ({:.1}, {:.1}, {:.1}) Cell: ({}, {})",
                                    pd.pos.x, pd.pos.y, pd.pos.z, pd.cell.0, pd.cell.1
                                ));
                                ui.label(format!("Vertical velocity: {:.2}", pd.vertical_vel));
                                ui.label(format!(
                                    "Heading: {:.1}\u{b0}",
                                    crate::bethesda_space::compass_heading_degrees(
                                        self.camera_controller.yaw()
                                    )
                                ));
                                if let Some(hit) = self.sim.debug_ray_hit(
                                    self.camera.eye,
                                    self.camera_controller.forward(),
                                    2000.0,
                                ) {
                                    // Every mesh path starts with the meshes
                                    // folder — constant, so drop it.
                                    let path = layout::below_meshes(&hit.mesh_path);
                                    ui.label(format!("Aim: {} @ {:.1}", path, hit.dist));
                                    ui.label(format!(
                                        "Ref: {}",
                                        hit.ref_id.as_deref().unwrap_or("-")
                                    ));
                                } else {
                                    ui.label("Aim: -");
                                    ui.label("Ref: -");
                                }
                                ui.label(format!("is_interactible: {dbg_interactible}"));
                            });

                        egui::CollapsingHeader::new("Physics")
                            .default_open(false)
                            .show(ui, |ui| {
                                let st = self.sim.stats();
                                ui.label(format!(
                                    "Colliders: {} across {} cells",
                                    st.collider_count, st.cells_with_colliders
                                ));
                                ui.label(format!(
                                    "Physics step: {} µs, {} steps/frame",
                                    st.last_step_micros, st.steps_last_tick
                                ));
                                ui.label(format!(
                                    "Dynamic bodies: {} active / {} resident",
                                    st.active_dynamic, st.dyn_total
                                ));
                                ui.add(
                                    egui::DragValue::new(self.streamer.physics_radius_mut())
                                        .speed(1)
                                        .range(1..=8)
                                        .prefix("Physics Radius: "),
                                );
                                ui.checkbox(
                                    &mut self.collision_wireframe,
                                    "Collision wireframe (C)",
                                );
                                if let Some(settings) = ragdoll_settings_section(
                                    ui,
                                    self.sim.ragdoll_settings(),
                                    &mut self.skeleton_overlay,
                                ) {
                                    self.sim.apply(SimCommand::SetRagdollSettings(settings));
                                }
                            });

                        egui::CollapsingHeader::new("Navmesh")
                            .default_open(false)
                            .show(ui, |ui| {
                                navmesh_section(
                                    ui,
                                    &mut self.navmesh_overlay,
                                    self.streamer.nav_world().stats(),
                                );
                            });

                        egui::CollapsingHeader::new("Camera")
                            .default_open(false)
                            .show(ui, |ui| {
                                let mut fly = self.input_mode == InputMode::Fly;
                                if ui.checkbox(&mut fly, "Free-fly camera (V)").changed() {
                                    if fly {
                                        self.input_mode = InputMode::Fly;
                                    } else {
                                        self.sim.apply(SimCommand::TeleportPlayer {
                                            pos: self.camera.eye,
                                        });
                                        self.input_mode = InputMode::Walk;
                                    }
                                }
                                ui.add(
                                    egui::DragValue::new(&mut self.camera_controller.speed)
                                        .speed(1)
                                        .prefix("Camera Speed: "),
                                );
                            });

                        egui::CollapsingHeader::new("Lighting")
                            .default_open(false)
                            .show(ui, |ui| {
                                lighting_panel(ui, &mut self.lighting);
                                local_lights_panel(
                                    ui,
                                    &mut self.lighting.local_lights,
                                    self.local_lights.counters(),
                                    self.placed_lights.live_count(),
                                );
                                lamp_shadows_panel(
                                    ui,
                                    &mut self.lighting.lamp_shadows,
                                    &self.lamp_shadow_cache.stats,
                                    self.lamp_atlas_preview,
                                );
                            });

                        egui::CollapsingHeader::new("Water")
                            .default_open(false)
                            .show(ui, |ui| {
                                water_panel(ui, self.water_stage.settings_mut());
                            });

                        egui::CollapsingHeader::new("Grass")
                            .default_open(false)
                            .show(ui, |ui| {
                                let residency = self.grass.residency();
                                grass_panel(
                                    ui,
                                    &GrassPanelState {
                                        available: self.grass.available(),
                                        stats: self.grass.stats(),
                                        residency,
                                        fade_limits: self.grass.fade_limits(),
                                    },
                                    self.grass.controls_mut(),
                                );
                            });

                        characters_section(
                            ui,
                            &mut self.anim_panel,
                            &mut self.characters,
                            &self.sim,
                            &self.camera,
                        );
                    });
                },
            );
        }

        drop(egui_span);
        if !memory_section_open {
            // Drop the cached report so a reopened header refreshes at once
            // instead of showing a stale reading.
            self.vram_text.clear();
        }
        self.memory_section_open = memory_section_open;
        let frame_capture = self.begin_frame_capture(&mut encoder, &output.texture);
        self.texture_feedback.encode_readback(
            &mut encoder,
            Instant::now(),
            self.texture_residency.recycle_epoch(),
        );
        self.gpu_profiler.end_frame(&mut encoder);

        // Split so the profile shows whether command validation/encoding
        // (`finish`) or staging flush + resource tracking (`submit`) dominates.
        let command_buffer = {
            let _span = tracing::info_span!("encoder_finish").entered();
            encoder.finish()
        };
        {
            let _span = tracing::info_span!("submit").entered();
            self.queue.submit(std::iter::once(command_buffer));
        }
        self.finish_frame_capture(frame_capture);
        let post_submit_span = tracing::info_span!("post_submit").entered();
        self.cull_stats.post_submit();
        self.texture_feedback.post_submit();
        self.post.post_submit();
        self.shadow_renderer.post_submit();
        self.local_lights.post_submit();
        self.gpu_profiler.post_submit();
        drop(post_submit_span);
        if let Some(perf_log) = self.perf_log.as_mut().filter(|log| log.is_due()) {
            let _span = tracing::info_span!("perf_log").entered();
            perf_log.emit_if_due(&PerfFrame {
                fps: self.fps_utils.fps(),
                profiler: &self.gpu_profiler,
                lights_loaded: self.placed_lights.live_count(),
                local_lights: *self.local_lights.counters(),
                shadow_draws: self.shadow_renderer.draw_stats(),
                cascade_refreshes: self.lighting.shadows.cascade_refreshes(),
                lamp_shadows: &self.lamp_shadow_cache.stats,
                exposure: self.lighting.exposure.readout,
                textures: {
                    let mut textures = self.texture_residency.stats;
                    textures.live_slots = self.texture_array.live_slots();
                    textures.feedback_age_ms = self.texture_feedback.age_ms();
                    textures
                },
                world_draws: self.cull_stats.latest.objects_total()
                    + self.cull_stats.latest.terrain,
                object_slots: self.object_instances.dispatch_count(),
                object_live: self.object_instances.live_count(),
                grass: self.grass.stats(),
                navmesh: self.streamer.nav_world().stats(),
                device: &self.device,
            });
        }
        // The pyramid now holds this frame's depth: next frame may occlusion-cull.
        self.hiz_valid = !skip.skips(DebugPass::Hiz);
        {
            let _span = tracing::info_span!("present").entered();
            self.queue.present(output);
        }

        self.limit_frame_rate(frame_start);

        Ok(())
    }

    /// Build the GPU-resource bundle the streamer needs (with a fresh per-frame
    /// upload budget) and run `f` against it. A closure rather than a returned
    /// `StreamTargets` because the bundle borrows many `self` fields mutably
    /// while the streamer is borrowed mutably alongside it — the borrow checker
    /// permits that only as disjoint field borrows within one body, which a
    /// returned struct cannot express. Both streaming entry points (the
    /// worldspace switch and the per-frame update) route through here so the
    /// bundle is constructed in exactly one place.
    fn with_stream_targets<R>(
        &mut self,
        asset_server: &mut AssetServer,
        f: impl FnOnce(&mut WorldStreamer, &mut StreamTargets) -> R,
    ) -> R {
        let limit = self.streamer.upload_budget_limit();
        let mut targets = StreamTargets {
            object_instances: &mut self.object_instances,
            placed_lights: &mut self.placed_lights,
            terrain_instances: &mut self.terrain_instances,
            object_mega: &mut self.object_mega,
            terrain_mega: &mut self.terrain_mega,
            texture_array: &mut self.texture_array,
            residency: &mut self.texture_residency,
            splatmaps: &mut self.splatmaps,
            material_registry: &mut self.material_registry,
            cube_array: &mut self.cube_array,
            grass: &mut self.grass,
            asset_server,
            device: &self.device,
            queue: &self.queue,
            upload_budget: UploadBudget::new(limit),
        };
        f(&mut self.streamer, &mut targets)
    }

    /// The interior cell request failed after the streamer tore the old content down: drops
    /// the queued collider removals so physics matches the empty world, and leaves the player
    /// where it was instead of teleporting into a cell that will never load.
    fn abandon_failed_interior_entry(&mut self, error: &InteriorRequestError) {
        tracing::error!("interior entry abandoned, the world stays empty: {error}");
        stream_settle::apply_teardown(&mut self.sim, self.streamer.take_sim_commands());
    }

    /// Finish a streamer teardown+load (worldspace switch or interior enter):
    /// apply the queued `RemoveCellColliders` and fold the broad phase BEFORE
    /// teleporting, then move the camera and request the player spawn at
    /// `start`. The remove-before-teleport order is load-bearing — otherwise the
    /// spawn's ground-snap can latch onto the old content's still-resident
    /// terrain at a grid that numerically aliases the new start.
    fn apply_stream_teleport(&mut self, start: Vec3) {
        stream_settle::apply_teardown(&mut self.sim, self.streamer.take_sim_commands());
        self.camera.eye = start;
        self.sim.apply(SimCommand::SpawnPlayer { pos: start });
    }

    /// Prepares the effect stage's draws for the frame `update` computed. A translated effect
    /// shader missing from the install is fatal, and so is a failed background validation of the
    /// effect shaders; draws that merely lack a texture or a compiled pipeline are skipped by
    /// the stage.
    fn prepare_effects(&mut self) -> anyhow::Result<()> {
        Self::check_effect_validation()?;
        let Some(frame) = self.latest_frame else {
            return Ok(());
        };
        let streamer = &self.streamer;
        let light_anim = &self.light_anim;
        let elapsed_seconds = self.world_seconds;
        let result = self.effect_stage.prepare(
            &EffectFrameInputs {
                frame: &frame,
                near_plane: self.camera.znear,
                elapsed_seconds,
            },
            &mut self.cube_array,
            |visit| streamer.for_each_effect_instance(visit),
            |lights| {
                streamer.for_each_placed_light(|key, light| {
                    lights.push(light_anim.pickable(key, light));
                });
            },
        );
        self.refraction_stage.prepare(
            &RefractionFrameInputs {
                frame: &frame,
                near_plane: self.camera.znear,
                elapsed_seconds,
            },
            |visit| streamer.for_each_refraction_instance(visit),
        );
        result.map_err(anyhow::Error::from)
    }

    /// Fails with the validation error once the background validation of the effect shader
    /// family has finished and found a module the GPU compiler would reject.
    fn check_effect_validation() -> anyhow::Result<()> {
        match shaders::startup::effect_validation().and_then(EffectValidation::failure) {
            Some(error) => Err(anyhow::anyhow!("effect shader validation failed: {error}")),
            None => Ok(()),
        }
    }

    /// Animates the model-driven lights and runs the addon and placed-model particle systems.
    fn update_fire_effects(&mut self, dt: f32) {
        let elapsed = self.world_seconds;
        self.light_anim.update(
            &self.streamer,
            elapsed,
            self.camera.eye,
            &self.placed_lights,
            &self.queue,
        );
        self.addon_fx.sync(&self.streamer, self.camera.eye);
        self.addon_fx.update(dt, self.camera.eye);
        let camera = CameraBasis::from_camera_to_world(self.camera.view_matrix().inverse());
        let planes = frustum::clip_planes(&self.cached_vp);
        self.addon_fx
            .submit(&camera, &planes, &mut self.effect_stage);
    }

    /// Register newly streamed animated objects, advance the playing clips and
    /// aim their kinematic bodies for this frame's physics step. Does nothing
    /// (no allocation, no sim command) while no clip plays.
    fn tick_animated_objects(&mut self, dt: f32) {
        self.animated_objects
            .apply_events(self.streamer.take_animated_events());
        for command in self.animated_objects.take_kinematic_releases() {
            self.sim.apply(command);
        }
        self.auto_anim.tick(
            dt,
            || self.sim.player_eye().is_some(),
            &mut self.animated_objects,
        );
        self.animated_objects.tick(dt);
        let kinematic = self.animated_objects.kinematic_poses();
        if !kinematic.is_empty() {
            self.sim.set_kinematic_poses(kinematic);
        }
    }

    /// Poses the skinned ragdolls from the sim and swaps their static rows out.
    fn update_ragdolls(&mut self, asset_server: &mut AssetServer) {
        let events = self.streamer.take_ragdoll_events();
        let row_ops = self.ragdoll_objects.update(
            events,
            &mut self.sim,
            &mut self.characters,
            &mut CharacterGpuContext {
                device: &self.device,
                queue: &self.queue,
                texture_array: &mut self.texture_array,
                residency: &mut self.texture_residency,
                material_registry: &mut self.material_registry,
                cube_array: &mut self.cube_array,
            },
        );
        if !row_ops.is_empty() {
            self.with_stream_targets(asset_server, |streamer, targets| {
                streamer.apply_animated_poses(&row_ops, targets);
            });
        }
    }

    /// Write the poses `tick_animated_objects` produced into the instance rows.
    /// Runs after the streamer update so a repack already ran.
    fn write_animated_rows(&mut self, asset_server: &mut AssetServer) {
        if !self.animated_objects.has_updates() {
            return;
        }
        let objects = std::mem::take(&mut self.animated_objects);
        self.with_stream_targets(asset_server, |streamer, targets| {
            streamer.apply_animated_poses(&objects.poses(), targets);
        });
        self.animated_objects = objects;
    }

    fn play_footsteps(&mut self) -> Result<(), AudioError> {
        let events = self.sim.drain_footsteps();
        if self.input_mode == InputMode::Fly {
            return Ok(());
        }
        let Some(audio) = &mut self.audio else {
            return Ok(());
        };
        let worldspace =
            (!self.streamer.in_interior()).then_some(self.streamer.worldspace.as_str());
        for event in events {
            let material = self
                .foot_surface
                .material_hash(worldspace, event.surface)
                .map_err(AudioError::FootSurface)?;
            audio.play_footstep(event, material)?;
        }
        Ok(())
    }

    /// Starts or stops the menu video with the menu and uploads the newest frame. A video that
    /// fails is given up for the run and the menu stays black; the game goes on.
    fn sync_menu_video(&mut self) {
        if let Err(error) = self.try_sync_menu_video() {
            self.menu_video.abandon(&error);
            self.menu.clear_video();
        }
    }

    fn try_sync_menu_video(&mut self) -> Result<(), VideoError> {
        let command = self.menu_video.update(self.flow.phase().shows_menu())?;
        if command == Some(VideoCommand::Stop) {
            self.menu.clear_video();
        }
        if let Some(frame) = self.menu_video.take_frame() {
            self.menu.upload_video(&self.device, &self.queue, &frame);
        }
        Ok(())
    }

    fn sync_audio(&mut self, dt: f32) -> Result<(), AudioError> {
        self.play_footsteps()?;
        if self.audio.is_none() {
            return Ok(());
        }
        let phase = self.flow.phase();
        let dt = Duration::from_secs_f32(dt.max(0.0));
        let place = (!phase.shows_menu() && self.music_poll.due(dt))
            .then(|| self.streamer.music_place(self.camera.eye));
        let frame = AudioFrame {
            phase,
            dt,
            hour: self.sim.pipboy_snapshot().model.clock.hour(),
            place,
        };
        match &mut self.audio {
            Some(audio) => audio.sync(frame),
            None => Ok(()),
        }
    }

    pub fn record_perf_frame(&mut self, update: std::time::Duration, render: std::time::Duration) {
        if let Some(perf_log) = &mut self.perf_log {
            perf_log.record_frame(update, render);
        }
    }

    #[tracing::instrument(skip_all, name = "update")]
    pub fn update(&mut self, asset_server: &mut AssetServer) -> Result<(), FatalRuntimeError> {
        self.device_loss.check()?;
        self.fps_utils.tick();
        let dt = self.fps_utils.dt().unwrap_or(0.0);
        self.flow.tick(dt);
        self.sync_audio(dt)?;
        self.sync_menu_video();
        // Nothing is loaded and nothing simulates until the player picks a spawn.
        if self.flow.phase().shows_menu() {
            self.drive_pipboy_capture(dt);
            self.streamer.suspend_frame_meter();
            return Ok(());
        }

        self.apply_pipboy_env();
        self.drive_pipboy_capture(dt);
        let pipboy_open = self.sim.pipboy_snapshot().model.open;
        self.sync_pipboy_session(pipboy_open);
        // The world stands still behind the Pip-Boy: render-side clocks get no time.
        let world_dt = if pipboy_open { 0.0 } else { dt };
        self.world_seconds += world_dt;

        // Apply a debug-UI worldspace selection before anything reads the
        // camera this frame: the streamer tears the old content down and reports
        // the new start. The input-mode block below recomputes camera.target
        // from the controller, so only eye is set here (in apply_stream_teleport).
        if let Some(ws) = self.pending_worldspace.take() {
            // Re-picking the current worldspace is a no-op — unless we're in an
            // interior, where it is the way back out and must reload.
            if ws != self.streamer.worldspace || self.streamer.in_interior() {
                let leaving_interior = self.streamer.in_interior();
                // Start cell was resolved at boot (no DB query on this frame).
                let start_cell = self
                    .worldspaces
                    .iter()
                    .find(|w| w.form_key == ws)
                    .map_or((0, 0), |w| w.start_cell);
                let start = self.with_stream_targets(asset_server, |streamer, targets| {
                    streamer.switch_worldspace(&ws, start_cell, targets)
                });
                if leaving_interior {
                    // Restore terrain-gated walking for the worldspace spawn.
                    self.sim.apply(SimCommand::SetInteriorMode(false));
                    self.input_mode = InputMode::Walk;
                }
                self.apply_stream_teleport(start);
            }
        }

        // Apply a debug-UI interior selection: drop the worldspace cells and
        // load the single interior cell as a walkable unit (sim interior mode on
        // so the player grounds on its object-mesh floors).
        if let Some(fk) = self.pending_interior.take() {
            // Copy out grid + start (Copy) so the &self.interiors borrow ends
            // before with_stream_targets takes &mut self.
            let target = self
                .interiors
                .iter()
                .find(|i| i.form_key == fk)
                .map(|i| (i.grid, i.start_pos));
            if let Some((grid, start_pos)) = target {
                let entered = self.with_stream_targets(asset_server, |streamer, targets| {
                    streamer.enter_interior(&fk, grid, start_pos, targets)
                });
                match entered {
                    Ok(start) => {
                        self.sim.apply(SimCommand::SetInteriorMode(true));
                        self.input_mode = InputMode::Walk;
                        self.apply_stream_teleport(start);
                    }
                    Err(error) => self.abandon_failed_interior_entry(&error),
                }
            } else {
                tracing::warn!("interior {fk} not in dropdown list; ignoring");
            }
        }

        // Dev override: teleport through OPENFO4_AUTO_ACTIVATE's door sequence, one
        // hop per readiness window (player spawned + a cooldown). Drives the
        // activation dispatch directly by ref where OS input can't be injected;
        // the raycast path itself is covered by the is_interactible readout.
        if self.auto_idx < self.auto_activate.len() {
            if self.auto_cooldown > 0 {
                self.auto_cooldown -= 1;
            } else if self.sim.player_eye().is_some() {
                let r = self.auto_activate[self.auto_idx].clone();
                let hop_consumed = match self.queue_teleport(&r) {
                    Ok(()) => {
                        tracing::info!("OPENFO4_AUTO_ACTIVATE[{}]: {r}", self.auto_idx);
                        true
                    }
                    Err(refusal @ TeleportRefusal::UnknownSource(_)) => {
                        tracing::warn!("OPENFO4_AUTO_ACTIVATE: {refusal}");
                        true
                    }
                    Err(refusal) => {
                        tracing::debug!("OPENFO4_AUTO_ACTIVATE: {r} deferred: {refusal}");
                        false
                    }
                };
                if hop_consumed {
                    self.auto_idx += 1;
                    self.auto_cooldown = AUTO_ACTIVATE_COOLDOWN_FRAMES;
                }
            }
        }

        // Apply a load-door activation (E): the same teardown + teleport path as
        // the debug pickers, but the arrival position, grid and facing all come
        // from the door's XTEL. `switch_worldspace`'s returned default start is
        // intentionally discarded — we always arrive at the door's destination.
        if let Some(t) = self.pending_teleport.take() {
            let entered = self.with_stream_targets(asset_server, |streamer, targets| {
                if t.is_interior {
                    streamer
                        .enter_interior(&t.cell_form_key, t.grid, t.start_pos, targets)
                        .map(drop)
                } else {
                    streamer.switch_worldspace(&t.worldspace, t.grid, targets);
                    Ok(())
                }
            });
            if let Err(error) = entered {
                self.abandon_failed_interior_entry(&error);
            } else {
                self.sim.apply(SimCommand::SetInteriorMode(t.is_interior));
                self.input_mode = InputMode::Walk;
                if let Some(yaw) = t.yaw {
                    self.camera_controller.set_look(yaw, 0.0);
                }
                self.apply_stream_teleport(t.start_pos);
                // Keep the debug picker in sync with where we now are. Crucially, the
                // Worldspace dropdown auto-arms `pending_worldspace` while `in_interior`
                // (its "flip to exit" affordance); leaving world_mode on Worldspace
                // after entering an interior would bounce the player straight back out.
                self.world_mode = if t.is_interior {
                    DebugWorldMode::Interior
                } else {
                    DebugWorldMode::Worldspace
                };
            }
        }

        self.tick_animated_objects(world_dt);

        let sim_tick_span = tracing::info_span!("sim_tick").entered();
        if self.freeze_camera {
            self.input_mode = InputMode::Fly;
        }
        match self.input_mode {
            InputMode::Fly => {
                if self.freeze_camera {
                    self.camera.target = self.camera.eye + self.camera_controller.forward();
                } else {
                    self.camera_controller.update_camera(&mut self.camera, dt);
                }
                // Keep dynamic clutter simulating around the free camera. Neutral
                // input so stale held keys don't drift the parked player (it is
                // teleported back to the camera on the fly->walk handoff anyway).
                self.sim
                    .apply(SimCommand::SetPlayerInput(PlayerInput::default()));
                self.sim.tick(dt);
            }
            InputMode::Walk => {
                self.sim
                    .apply(SimCommand::SetPlayerInput(self.player_input()));
                self.sim.tick(dt);
                // Until the player has spawned onto the ground the camera
                // holds its last position (initial load, teleports).
                if let Some(eye) = self.sim.player_eye() {
                    self.camera.eye = eye;
                }
                self.camera.target = self.camera.eye + self.camera_controller.forward();
            }
        }

        drop(sim_tick_span);
        // After the camera follows the player: placement needs this frame's eye and look direction.
        self.auto_characters.tick(
            || self.sim.player_eye().is_some(),
            &mut self.characters,
            &self.sim,
            &self.camera,
        );
        let hud_span = tracing::info_span!("hud").entered();
        // Build the HUD overlay for this frame. The crosshair state is selected
        // here (render-side): door-aim depends on the camera, which the sim does
        // not own — the sim only supplies the Standard spread radius.
        self.aimed_door = self.aimed_activation().is_some();
        let hud_activate = self.aimed_door;
        let mut hud_snapshot = self.sim.hud_snapshot();
        hud_snapshot.visible &= self.flow.hud_visible();
        self.hud.update(
            &hud_snapshot,
            hud_activate,
            world_dt,
            self.config.width,
            self.config.height,
        );

        drop(hud_span);
        let lighting_frame_span = tracing::info_span!("lighting_frame").entered();
        // Sprint FOV widening: ramp the additive adjust toward its target.
        // Gated on the sprint toggle actually moving (W held), the proxy for
        // FO4's sprint camera state.
        let sprinting = self.input_mode == InputMode::Walk
            && self.walk_keys.sprint_active
            && self.walk_keys.forward;
        let target = if sprinting { SPRINT_FOV_ADJUST } else { 0.0 };
        let max_step = FOV_ADJUST_PER_SEC * dt;
        self.fov_adjust = if (target - self.fov_adjust).abs() <= max_step {
            target
        } else {
            self.fov_adjust + max_step * (target - self.fov_adjust).signum()
        };
        self.camera.fovy = beth_fov_to_vertical(self.world_fov_beth + self.fov_adjust);
        self.lighting.sync_environment(
            self.streamer.current_interior(),
            self.interior_lighting.as_ref(),
        );
        let mut water_entries = Vec::new();
        if self.water_stage.wants_water_entries() {
            self.sim.drain_water_entries(&mut water_entries);
        }
        let wading_player = matches!(self.input_mode, InputMode::Walk)
            .then(|| self.sim.player_body())
            .flatten();
        self.water_stage.update_displacement(
            &self.queue,
            world_dt,
            self.camera.eye,
            wading_player,
            &water_entries,
        );
        self.water_stage
            .set_camera_surroundings(match self.streamer.current_interior() {
                Some(_) => Surroundings::Interior {
                    clip_distance: self
                        .lighting
                        .interior_clip_distance()
                        .map(|distance| distance * crate::bethesda_space::UNITS_FROM_ENGINE),
                },
                None => Surroundings::Exterior,
            });
        let water_scope = match self.streamer.current_interior() {
            Some(cell) => WaterScope::Interior(cell),
            None => WaterScope::Exterior(&self.streamer.worldspace),
        };
        self.water_stage.collect(
            &self.camera,
            water_scope,
            |visit| self.streamer.for_each_placed_water(visit),
            |visit| self.streamer.for_each_loaded_cell(visit),
        );
        let rendered_cascades = self.shadow_renderer.take_rendered_cascades();
        let mut shadow_cascades = self
            .lighting
            .shadow_cascades(&self.camera, rendered_cascades);
        let frame_uniform = self.lighting.frame_uniform(&FrameInputs {
            camera: &self.camera,
            cascades: shadow_cascades.as_ref(),
            viewport: UVec2::new(self.config.width, self.config.height),
            max_anisotropy: self.max_anisotropy,
            texture_feedback: self
                .texture_feedback
                .frame_params(self.texture_residency.policy().global_skip),
            elapsed_seconds: self.world_seconds,
            camera_water: self.water_stage.camera_water(),
        });
        self.cached_vp = frame_uniform.view_proj;
        self.latest_frame = Some(frame_uniform);
        self.water_stage
            .set_debug_output(&self.device, self.lighting.debug_view.water_output());
        self.water_stage.prepare(
            &self.device,
            &self.queue,
            &frame_uniform,
            self.streamer.in_interior(),
        );
        drop(lighting_frame_span);
        let gpu_poll_span = tracing::info_span!("gpu_poll").entered();
        escalate_device_loss(self.cull_stats.poll(&self.device, &self.device_loss))?;
        let mip_view_shown = self.lighting.debug_view.is_mip_view();
        escalate_device_loss(self.texture_feedback.poll(
            &self.device,
            &self.device_loss,
            &mut self.texture_residency,
            &mut self.texture_array,
            mip_view_shown,
        ))?;
        if mip_view_shown {
            self.texture_array.upload_slot_mips(&self.queue);
        }
        escalate_device_loss(self.gpu_profiler.poll(&self.device, &self.device_loss))?;
        match self.post.poll_exposure(&self.device, &self.device_loss) {
            Ok(Some(readout)) => self.lighting.exposure.readout = readout,
            Ok(None) => {}
            Err(error) => escalate_device_loss(Err(error))?,
        }
        escalate_device_loss(self.local_lights.poll(&self.device, &self.device_loss))?;
        drop(gpu_poll_span);
        // The resident potential must bound every bucket's actual draw count.
        // The readback is a few frames stale, but capacities only ever grow, so
        // a past count was produced under an equal-or-smaller cap — a clean
        // check. Trips loudly if the CPU routing mirror ever drifts from the
        // shader (the shader's arrayLength guard is the production fail-safe).
        #[cfg(debug_assertions)]
        for bucket in 0..gpu_driven::DRAW_BUCKET_COUNT {
            debug_assert!(
                self.cull_stats.latest.object_buckets[bucket]
                    <= self.object_indirect.capacity(bucket),
                "bucket {bucket} drew {} > capacity {} — per-bucket potential bound violated",
                self.cull_stats.latest.object_buckets[bucket],
                self.object_indirect.capacity(bucket),
            );
        }
        let frame_uniform_write_span = tracing::info_span!("frame_uniform_write").entered();
        self.frame_binding.write(&self.queue, &frame_uniform);
        self.post
            .write_exposure_params(&self.queue, &self.lighting.exposure.gpu_params(dt));

        drop(frame_uniform_write_span);
        let eye = self.camera.eye;
        // Without the sprint widening: the narrowest field of view the camera
        // takes is the one that shows textures largest.
        self.streamer.set_screen(ScreenView {
            vertical_fov: beth_fov_to_vertical(self.world_fov_beth).to_radians(),
            width: self.config.width,
            height: self.config.height,
        });
        let stream = self.with_stream_targets(asset_server, |streamer, targets| {
            streamer.update_gpu_driven(eye, targets)
        });
        self.update_fire_effects(dt);

        let full_ring_complete = stream.full_ring_complete;
        if let Some(report) = &stream.spawn_complete {
            self.toast_queue.push((
                format!(
                    "Initial spawn: {:.1}s total — detail {:.1}s ({} cells), LOD {:.1}s ({} cells)",
                    report.total.as_secs_f32(),
                    report.full.as_secs_f32(),
                    report.full_cells,
                    report.lod.as_secs_f32(),
                    report.lod_cells,
                ),
                ToastType::Info,
            ));
        }

        // Collider lifecycle follows the streamer's collision ring. The fold
        // into the broad phase is deferred to the next fixed tick (see
        // `flush_colliders_per_frame`). No-op (and no span) on clean frames. The load
        // readiness follows the settled colliders and is fed here, nowhere else.
        let settled = stream_settle::settle_colliders(
            &mut self.sim,
            stream.sim_commands,
            crate::sim::COLLIDER_APPLY_BUDGET,
        );
        self.feed_load_readiness(settled, full_ring_complete);

        // Rewrite moving dynamic clutter into the instance buffer. Runs after
        // the streamer's update_gpu_driven (so any repack already ran) and after
        // the sim-command fold (so newly-promoted bodies are live). The CPU
        // mirror is updated too, so a later repack preserves the moved pose.
        let dynamic_poses_span = tracing::info_span!("dynamic_poses").entered();
        let mut dyn_poses = std::mem::take(&mut self.dynamic_poses);
        self.sim.dynamic_poses(&mut dyn_poses);
        if !dyn_poses.is_empty() {
            self.with_stream_targets(asset_server, |streamer, targets| {
                streamer.apply_dynamic_poses(&dyn_poses, targets);
            });
        }
        self.dynamic_poses = dyn_poses;
        self.write_animated_rows(asset_server);
        self.update_ragdolls(asset_server);

        drop(dynamic_poses_span);
        let characters_span = tracing::info_span!("characters").entered();
        self.characters.update(
            world_dt,
            &self.cached_vp,
            &mut CharacterGpuContext {
                device: &self.device,
                queue: &self.queue,
                texture_array: &mut self.texture_array,
                residency: &mut self.texture_residency,
                material_registry: &mut self.material_registry,
                cube_array: &mut self.cube_array,
            },
        );
        self.characters
            .note_texture_demand(&mut self.texture_residency);
        self.grass.note_texture_demand(&mut self.texture_residency);
        drop(characters_span);
        let collision_wireframe_span = tracing::info_span!("collision_wireframe").entered();
        if self.collision_wireframe {
            self.refresh_collision_wireframe();
        } else if self.collision_lines.is_allocated() {
            // Toggled off (key or checkbox): release the line buffer.
            self.collision_lines.release();
        }
        self.navmesh_overlay
            .update(&self.device, &self.queue, self.streamer.nav_world());

        drop(collision_wireframe_span);
        self.skeleton_overlay.update(
            &self.device,
            &self.queue,
            || OverlayView {
                eye: self.camera.eye,
                planes: frustum::clip_planes(&self.cached_vp),
            },
            &self.animated_objects,
            &self.characters,
        );
        // The object slab uploads per-cell ranges inside the streamer; here we
        // only learn whether its buffer handle moved (growth). Terrain and
        // materials keep the whole-buffer upload path (tiny).
        let buffer_uploads_span = tracing::info_span!("buffer_uploads").entered();
        let object_realloc = self.object_instances.take_reallocated();
        let placed_lights_realloc = self.placed_lights.take_reallocated();
        let terrain_realloc = self.terrain_instances.upload(&self.device, &self.queue);
        let material_realloc = self.material_registry.upload(&self.device, &self.queue);

        drop(buffer_uploads_span);
        // Resize the per-bucket indirect draw buffers to the resident per-bucket
        // potential — a hard upper bound on the draws the cull shader can append
        // to each bucket (see potential_buckets). Each bucket is sized
        // independently from its own potential, so a near-empty decal bucket no
        // longer mirrors the whole object pool; only the buckets that grew are
        // reallocated.
        let indirect_resize_span = tracing::info_span!("indirect_resize").entered();
        let object_indirect_resized =
            self.object_indirect
                .resize(&self.device, self.streamer.bucket_potential(), "object");
        if object_indirect_resized {
            let caps = self.object_indirect.capacities();
            let total_kib =
                caps.iter().map(|&c| u64::from(c)).sum::<u64>() * gpu_driven::DRAW_STRIDE / 1024;
            tracing::info!(
                "object indirect resized: live={} caps={:?} total={} KiB",
                self.object_instances.live_count(),
                caps,
                total_kib,
            );
        }
        let mut terrain_indirect_resized = false;
        let terrain_count = self.terrain_instances.len();
        if terrain_count > self.terrain_indirect.max_draws() {
            self.terrain_indirect
                .resize(&self.device, terrain_count * 2);
            terrain_indirect_resized = true;
        }

        drop(indirect_resize_span);
        // Rebuild only the bind groups whose resources actually changed; the
        // material bind group in particular holds up to 8192 texture views,
        // and beyond its creation cost every rebuilt bind group pays a
        // first-use re-track of all those views in that frame's queue.submit.
        // During streaming bursts the slot table changes on nearly every
        // integrate frame, so rebuilds are debounced: at most one per
        // MATERIAL_BG_DEBOUNCE while dirt accumulates. The visible artifact
        // is bounded and benign — a texture acquired into a *recycled* slot
        // samples the slot's previous texture for up to the debounce window,
        // on geometry that just streamed in at the ring edge. Two cases are
        // never debounced: a material-buffer realloc (the live bind group
        // references a dead buffer) and slot high-water growth (indexing an
        // unbound entry of a partially-bound array is UB).
        let material_bind_group_span = tracing::info_span!("material_bind_group").entered();
        const MATERIAL_BG_DEBOUNCE: Duration = Duration::from_millis(150);
        self.pending_slot_changes.merge(stream.slot_changes);
        let new_alpha_slots = self.texture_array.take_new_alpha_slots();
        if self.shadow_material_bind_group.is_some() {
            self.pending_alpha_slots.extend(new_alpha_slots);
        }
        let cube_added = self.cube_array.take_changed();
        self.material_bg_dirty |= !self.pending_slot_changes.is_empty()
            || !self.pending_alpha_slots.is_empty()
            || material_realloc
            || cube_added;
        let must_rebuild_now =
            material_realloc || self.texture_array.bind_group_len() > self.material_bg_built_len;
        if self.material_bg_dirty
            && (must_rebuild_now || self.material_bg_last_rebuild.elapsed() >= MATERIAL_BG_DEBOUNCE)
        {
            (self.material_bind_group, self.shadow_material_bind_group) =
                Self::build_material_bind_groups(
                    &self.device,
                    &self.world_pass_layout.material_bind_group_layout,
                    &self.texture_array,
                    &self.cube_array,
                    &self.material_sampler,
                    &self.terrain_sampler,
                    &self.shadow_sampler,
                    &self.material_registry.buffer,
                    &self.splatmaps.view,
                );
            self.texture_array.upload_slot_mips(&self.queue);
            self.material_bg_dirty = false;
            self.material_bg_last_rebuild = Instant::now();
            self.material_bg_built_len = self.texture_array.bind_group_len();

            // Recycled slots showed another texture and resolved placeholders
            // showed a stand-in (white alpha) in shadows drawn before this
            // rebind; resolution swaps and releases cannot have.
            let mut changes = std::mem::take(&mut self.pending_slot_changes);
            // Shadows drawn before this rebuild sampled the stand-in for
            // these newly alpha-tested slots, exactly like a placeholder.
            changes
                .placeholder_resolved_slots
                .append(&mut self.pending_alpha_slots);
            let stats = &mut self.texture_residency.stats;
            stats.bind_group_rebuilds += 1;
            stats.slots_recycled += changes.recycled_slots.len() as u64;
            self.streamer.on_materials_rebound(
                !changes.recycled_slots.is_empty(),
                &changes.placeholder_resolved_slots,
            );
        }

        drop(material_bind_group_span);
        let lamp_shadows_span = tracing::info_span!("lamp_shadows").entered();
        let mut shadow_invalidations = self.streamer.take_shadow_invalidations();
        self.characters
            .invalidate_shadows(&mut shadow_invalidations);
        self.lighting
            .shadows
            .note_invalidations(&shadow_invalidations, shadow_cascades.as_mut());
        if self.lighting.local_lights.enabled {
            self.streamer
                .shadow_lamp_candidates(&mut self.lamp_candidates);
        } else {
            self.lamp_candidates.clear();
            self.streamer.invalidate_lamp_candidates();
        }
        let camera_planes = frustum::clip_planes(&frame_uniform.view_proj);
        let (overflowed_lamps, lamp_regions_grew) = match self
            .shadow_renderer
            .poll_overflow(&self.device, &self.device_loss)
        {
            Ok(polled) => polled,
            Err(ReadbackError::DeviceLost(lost)) => return Err(lost.into()),
            Err(recoverable) => {
                tracing::error!("{recoverable}");
                Default::default()
            }
        };
        self.lamp_shadow_cache.redirty(&overflowed_lamps.tiles);
        let lamp_shadows = &mut self.lighting.lamp_shadows;
        let lamp_frame = self.lamp_shadow_cache.update(
            &self.lamp_candidates,
            &SelectionView {
                eye: self.camera.eye,
                planes: &camera_planes,
                light_fade: self.lighting.local_lights.fade,
                shadow_fade_end: lamp_shadows.config.fade_end,
            },
            &shadow_invalidations,
            &lamp_shadows.config,
            &mut lamp_shadows.settings,
            self.world_seconds,
            world_dt,
        );
        self.shadow_renderer
            .prepare(&self.queue, shadow_cascades.as_ref(), &lamp_frame.views);
        self.frame_binding
            .write_lamp_shadows(&self.queue, &lamp_frame.table);
        self.lamp_cull_slots = lamp_frame.cull_slots;

        drop(lamp_shadows_span);
        let rebind_buffers_span = tracing::info_span!("rebind_buffers").entered();
        let shadow_draws_resized = self.shadow_renderer.resize_draws(
            &self.device,
            caster_potentials(self.streamer.bucket_potential(), terrain_count),
        ) | lamp_regions_grew;

        self.rebind_reallocated_buffers(&BufferReallocations {
            object_instances: object_realloc,
            terrain_instances: terrain_realloc,
            object_indirect: object_indirect_resized,
            terrain_indirect: terrain_indirect_resized,
            shadow_draws: shadow_draws_resized,
            placed_lights: placed_lights_realloc,
        });
        drop(rebind_buffers_span);
        Ok(())
    }

    /// Rebuilds exactly the bind groups that reference a reallocated buffer.
    fn rebind_reallocated_buffers(&mut self, reallocated: &BufferReallocations) {
        let instances = reallocated.object_instances || reallocated.terrain_instances;
        if instances {
            self.instance_bind_group = Self::build_instance_bind_group(
                &self.device,
                &self.world_pass_layout.instance_bind_group_layout,
                &self.object_instances.buffer,
                &self.terrain_instances.buffer,
            );
        }

        if reallocated.object_instances || reallocated.object_indirect {
            self.object_cull_bg = self.object_cull.create_bind_group(
                &self.device,
                &self.object_instances.buffer,
                &self.object_indirect,
                self.hiz_pipeline.pyramid_view(),
            );
        }

        if reallocated.terrain_instances || reallocated.terrain_indirect {
            self.terrain_cull_bg = self.terrain_cull.create_bind_group(
                &self.device,
                &self.terrain_instances.buffer,
                &self.terrain_indirect,
            );
        }

        if instances || reallocated.shadow_draws {
            self.shadow_renderer.rebind(
                &self.device,
                &self.object_instances.buffer,
                &self.terrain_instances.buffer,
            );
        }

        if reallocated.placed_lights {
            self.local_lights.rebind_cull(
                &self.device,
                &self.placed_lights.buffer,
                self.hiz_pipeline.pyramid_view(),
            );
        }
    }

    pub fn is_surface_configured(&self) -> bool {
        self.is_surface_configured
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.config.width = width;
            self.config.height = height;
            self.camera.fit_viewport(width, height);

            self.surface.configure(&self.device, &self.config);
            self.is_surface_configured = true;

            self.depth_texture =
                Texture::create_depth_texture(&self.device, &self.config, "depth_texture");

            self.post.resize(&self.device, width, height);
            self.pipboy.set_window_size(width, height);
            self.scene_copy.resize(&self.device, width, height);
            self.skeleton_overlay
                .resize(&self.device, &self.depth_texture.view);

            self.hiz_pipeline
                .resize(&self.device, &self.depth_texture.view, width, height);

            // The pyramid was recreated: rebind it and skip occlusion until it
            // holds a rendered frame again.
            self.hiz_valid = false;
            self.object_cull_bg = self.object_cull.create_bind_group(
                &self.device,
                &self.object_instances.buffer,
                &self.object_indirect,
                self.hiz_pipeline.pyramid_view(),
            );
            self.local_lights.rebind_cull(
                &self.device,
                &self.placed_lights.buffer,
                self.hiz_pipeline.pyramid_view(),
            );
        }
    }

    /// Whether the Pip-Boy is up and owns the keyboard and mouse.
    pub fn pipboy_open(&self) -> bool {
        self.sim.pipboy_snapshot().model.open
    }

    /// Offers a window event to the Pip-Boy; `true` when it consumed the event.
    /// Lets the Pip-Boy track the cursor while it is closed, so it opens knowing the pointer.
    pub fn observe_pipboy_event(&mut self, event: &winit::event::WindowEvent) {
        self.pipboy.observe_window_event(event);
    }

    pub fn handle_pipboy_event(&mut self, event: &winit::event::WindowEvent) -> bool {
        let consumed = self
            .pipboy
            .handle_window_event(event, &self.sim.pipboy_snapshot());
        self.flush_pipboy_actions();
        consumed
    }

    fn flush_pipboy_actions(&mut self) {
        for action in self.pipboy.take_actions() {
            self.sim.apply(SimCommand::PipBoy(action));
        }
    }

    fn open_pipboy(&mut self, key: OpenKey) {
        if let OpenKey::Page(page) = key {
            self.pipboy.screen_mut().show_page(page);
        }
        self.sim.apply(SimCommand::PipBoy(PipBoyAction::Open));
    }

    /// Opens the screen named by the capture hooks once the player exists.
    fn apply_pipboy_env(&mut self) {
        if self.pipboy_env.open.is_none() || self.sim.player_eye().is_none() {
            return;
        }
        let env = std::mem::take(&mut self.pipboy_env);
        if let Some(screen) = env.open {
            self.pipboy.screen_mut().show(screen, env.select_row);
            self.sim.apply(SimCommand::PipBoy(PipBoyAction::Open));
        }
    }

    /// Follows the model's open flag: opening frees the mouse, drops held movement keys and the
    /// developer menu's keyboard focus,
    /// closing restores the capture the player had.
    fn sync_pipboy_session(&mut self, open: bool) {
        if open == self.pipboy_was_open {
            return;
        }
        self.pipboy_was_open = open;
        if open {
            self.capture_before_pipboy = self.mouse_capture.is_locked();
            self.mouse_capture.set_locked(false);
            self.debug_ui.release_keyboard_focus();
            self.walk_keys.release_held();
            for code in FLY_CAMERA_KEYS {
                self.camera_controller.handle_key(code, false);
            }
        } else {
            self.mouse_capture.set_locked(self.capture_before_pipboy);
        }
        self.mouse_capture.apply(&self.window);
    }

    pub fn handle_key(&mut self, code: KeyCode, is_pressed: bool, is_repeat: bool) {
        if !self.flow.gameplay_input_enabled() {
            self.handle_menu_key(code, is_pressed, is_repeat);
            return;
        }
        if let Some(open) = pipboy::input::open_key_press(code, is_pressed, is_repeat)
            .filter(|_| !self.pipboy_open())
        {
            self.open_pipboy(open);
        } else if code == KeyCode::Escape && is_pressed && !is_repeat {
            self.mouse_capture.toggle();
            self.mouse_capture.apply(&self.window);
        } else if code == KeyCode::KeyF && is_pressed {
            // Hotkey twin of the debug-UI "Show FX" checkbox.
            self.show_fx = !self.show_fx;
        } else if code == KeyCode::KeyB && is_pressed {
            // Hotkey twin of the debug-UI "Decal depth bias" checkbox.
            self.decal_bias_enabled = !self.decal_bias_enabled;
        } else if code == KeyCode::KeyC && is_pressed {
            // Hotkey twin of the debug-UI "Collision wireframe" checkbox;
            // update() handles buffer build/release on the transition.
            self.collision_wireframe = !self.collision_wireframe;
        } else if code == KeyCode::KeyN && is_pressed {
            // Hotkey twin of the debug-UI "Navmesh overlay" checkbox.
            self.navmesh_overlay.enabled = !self.navmesh_overlay.enabled;
        } else if code == KeyCode::KeyV && is_pressed {
            // Hotkey twin of the debug-UI "Free-fly camera" checkbox.
            self.toggle_input_mode();
        } else if code == KeyCode::KeyE && is_pressed && self.input_mode == InputMode::Walk {
            // Activate the load door under the crosshair. Record intent only; the
            // teleport (streamer teardown + player move) needs `asset_server` and
            // runs in `update`, like the debug pickers.
            self.activate_aimed();
        } else {
            // Both key maps stay fresh regardless of mode (same physical
            // keys, different meanings), so neither holds stale pressed
            // state across a toggle. Only the active mode's map drives.
            self.camera_controller.handle_key(code, is_pressed);
            match code {
                KeyCode::KeyW => {
                    self.walk_keys.forward = is_pressed;
                    if !is_pressed {
                        // Sprint lives and dies with W.
                        self.walk_keys.sprint_active = false;
                    }
                }
                KeyCode::KeyS => {
                    self.walk_keys.backward = is_pressed;
                    if is_pressed {
                        self.walk_keys.sprint_active = false;
                    }
                }
                KeyCode::KeyA => {
                    self.walk_keys.left = is_pressed;
                    if is_pressed {
                        self.walk_keys.sprint_active = false;
                    }
                }
                KeyCode::KeyD => {
                    self.walk_keys.right = is_pressed;
                    if is_pressed {
                        self.walk_keys.sprint_active = false;
                    }
                }
                KeyCode::Space => self.walk_keys.jump = is_pressed,
                KeyCode::ShiftLeft => {
                    if is_pressed && !self.walk_keys.sprint_held {
                        if self.walk_keys.sprint_active {
                            self.walk_keys.sprint_active = false;
                        } else if self.walk_keys.forward
                            && !(self.walk_keys.backward
                                || self.walk_keys.left
                                || self.walk_keys.right)
                        {
                            self.walk_keys.sprint_active = true;
                            // Sprinting from sneak stands the player up (the
                            // sim's headroom probe may delay the actual stand
                            // under a low ceiling).
                            self.walk_keys.crouch_active = false;
                        }
                    }
                    self.walk_keys.sprint_held = is_pressed;
                }
                KeyCode::ControlLeft => {
                    if is_pressed && !self.walk_keys.crouch_held {
                        self.walk_keys.crouch_active = !self.walk_keys.crouch_active;
                        // Sneaking exits sprint (sprint had cleared crouch, so
                        // this toggle always lands on crouched-not-sprinting).
                        self.walk_keys.sprint_active = false;
                    }
                    self.walk_keys.crouch_held = is_pressed;
                }
                _ => {}
            }
        }
    }

    /// Switch between the walking player and the free-fly debug camera.
    /// Fly -> walk hands the camera position to the sim, which re-snaps the
    /// capsule to the ground below it.
    fn toggle_input_mode(&mut self) {
        self.input_mode = match self.input_mode {
            InputMode::Walk => InputMode::Fly,
            InputMode::Fly => {
                self.sim.apply(SimCommand::TeleportPlayer {
                    pos: self.camera.eye,
                });
                InputMode::Walk
            }
        };
    }

    /// Rebuild the collision wireframe line buffer when stale: the collider
    /// set changed, the camera strayed from the last build point, or the
    /// radius was adjusted. Generation-triggered rebuilds are debounced —
    /// streaming bursts bump the generation every frame for seconds at a
    /// time, and tessellation + upload per frame on top of streaming load
    /// would turn the overlay into a slideshow.
    fn refresh_collision_wireframe(&mut self) {
        const GENERATION_DEBOUNCE: Duration = Duration::from_millis(300);

        // The built region is an AABB of half-extent = radius around the
        // build point; the staleness slop must stay well inside it or the
        // camera can exit the region while the cache still reads fresh.
        let rebuild_distance = (self.collision_wire_radius * 0.25).min(60.0);

        let generation = self.sim.collider_generation();
        let gen_changed = generation != self.collision_wire_gen
            && self.collision_wire_last_rebuild.elapsed() >= GENERATION_DEBOUNCE;
        let moved = self.camera.eye.distance(self.collision_wire_center) >= rebuild_distance;
        #[allow(
            clippy::float_cmp,
            reason = "change detection: the built radius is a verbatim copy of the setting"
        )]
        let radius_changed = self.collision_wire_built_radius != self.collision_wire_radius;
        if self.collision_lines.is_allocated() && !gen_changed && !moved && !radius_changed {
            return;
        }

        let (lines, truncated) = self
            .sim
            .debug_wireframe(self.camera.eye, self.collision_wire_radius);
        self.collision_wire_truncated = truncated;
        self.collision_wire_gen = generation;
        self.collision_wire_center = self.camera.eye;
        self.collision_wire_built_radius = self.collision_wire_radius;
        self.collision_wire_last_rebuild = Instant::now();

        let vertices: Vec<LineVertex> = lines
            .into_iter()
            .map(|position| LineVertex::new(position, COLLISION_WIREFRAME_COLOR))
            .collect();
        self.collision_lines
            .upload(&self.device, &self.queue, &vertices);
    }

    /// Snapshot the walk keys + camera yaw into the sim's input format.
    fn player_input(&self) -> PlayerInput {
        let move_dir = glam::Vec2::new(
            (i32::from(self.walk_keys.right) - i32::from(self.walk_keys.left)) as f32,
            (i32::from(self.walk_keys.forward) - i32::from(self.walk_keys.backward)) as f32,
        );
        PlayerInput {
            move_dir: move_dir.normalize_or_zero(),
            yaw: self.camera_controller.yaw(),
            jump: self.walk_keys.jump,
            sprint: self.walk_keys.sprint_active,
            crouch: self.walk_keys.crouch_active,
        }
    }
}

#[cfg(test)]
mod walk_keys_tests {
    use super::WalkKeys;

    #[test]
    fn releasing_held_keys_keeps_the_sprint_and_crouch_toggles() {
        let mut keys = WalkKeys {
            forward: true,
            jump: true,
            sprint_held: true,
            crouch_held: true,
            sprint_active: true,
            crouch_active: true,
            ..WalkKeys::default()
        };

        keys.release_held();

        assert!(!keys.forward && !keys.jump && !keys.sprint_held && !keys.crouch_held);
        assert!(keys.sprint_active && keys.crouch_active);
    }
}

#[cfg(test)]
mod spawn_target_tests {
    use super::*;

    const DOOR_HEADING: f64 = 1.25;

    fn arrival(heading: Option<f64>) -> SpawnCellTarget {
        SpawnCellTarget {
            cell_form_key: "0001A2:Fallout4.esm".to_owned(),
            position: [10.0, 20.0, 30.0],
            heading,
        }
    }

    #[test]
    fn a_door_arrival_faces_the_door_heading_without_an_explicit_spawn_look() {
        let target = ResolvedTarget::interior_spawn(arrival(Some(DOOR_HEADING)), None);
        assert_eq!(
            target.yaw,
            Some(crate::bethesda_space::yaw_from_rotation_z(
                DOOR_HEADING as f32
            ))
        );
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "the heading is rounded to one decimal, so the expected literals are exact"
    )]
    fn a_spawn_at_a_heading_marker_reads_that_heading_on_the_compass() {
        for degrees in [0.0_f32, 90.0, 180.0, 270.0, 273.4] {
            let yaw = crate::bethesda_space::yaw_from_rotation_z(degrees.to_radians());
            assert_eq!(crate::bethesda_space::compass_heading_degrees(yaw), degrees);
        }
    }

    #[test]
    fn an_explicit_spawn_look_wins_over_the_door_heading() {
        let target = ResolvedTarget::interior_spawn(arrival(Some(DOOR_HEADING)), Some((0.5, -0.3)));
        assert_eq!(target.yaw, None);
    }

    #[test]
    fn a_spawn_without_a_stored_heading_keeps_the_current_look() {
        assert_eq!(
            ResolvedTarget::interior_spawn(arrival(None), None).yaw,
            None
        );
        assert_eq!(
            ResolvedTarget::interior_spawn(arrival(None), Some((0.5, -0.3))).yaw,
            None
        );
    }
}

#[cfg(test)]
mod freeze_camera_tests {
    use super::*;

    fn lookup_with(set: &'static str, value: bool) -> impl Fn(&str, bool) -> bool {
        move |name, default| if name == set { value } else { default }
    }

    #[test]
    fn the_camera_is_frozen_when_the_freeze_variable_is_set() {
        assert!(camera_frozen(lookup_with("OPENFO4_FREEZE_CAMERA", true)));
    }

    #[test]
    fn the_camera_is_not_frozen_when_the_freeze_variable_is_unset_or_off() {
        assert!(!camera_frozen(|_, default| default));
        assert!(!camera_frozen(lookup_with("OPENFO4_FREEZE_CAMERA", false)));
    }

    #[test]
    fn only_the_freeze_variable_freezes_the_camera() {
        assert!(!camera_frozen(lookup_with("OPENFO4_SPAWN_LOOK", true)));
    }

    #[test]
    fn a_frozen_camera_starts_with_the_mouse_capture_released() {
        let frozen = MouseCapture::new(camera_frozen(lookup_with("OPENFO4_FREEZE_CAMERA", true)));
        let free = MouseCapture::new(camera_frozen(|_, default| default));
        assert!(!frozen.is_locked());
        assert!(free.is_locked());
    }
}

#[cfg(test)]
mod test_support {
    use crate::assets::ini::GameIni;

    /// Every INI setting the engine reads, with values unlike any vanilla default so a test sees
    /// a setting that was not read from the INI.
    pub const FULL_INI: &str = "\
[Display]
iShadowMapResolution=2048
fDirShadowDistance=12000.0
iDirShadowSplits=2
fBlendSplitDirShadow=30.0
uiOrthoShadowFilter=2
fShadowBiasScale=0.75
iMaxFocusShadows=6
uiShadowFilter=1
fLocalShadowMapHalveEveryXUnit=900.0
fShadowLODMaxStartFade=1200.0
fLightLODMaxStartFade=4000.0
iTexMipMapSkip=1
iMaxAnisotropy=8
[Imagespace]
bScreenSpaceBokeh=0
[Water]
bUseWaterReflections=0
[TerrainManager]
fBlockMaximumDistance=120000.0
[Interface]
fDefaultWorldFOV=75
fSafeZoneXWide=50.0
iHUDColorR=51
iHUDColorG=229
iHUDColorB=77
[Pipboy]
fPipboyEffectColorR=0.2
fPipboyEffectColorG=0.9
fPipboyEffectColorB=0.3
[MAIN]
fHUDOpacity=0.8
[General]
uGridsToLoad=7
[Grass]
iMinGrassSize=30
fGrassStartFadeDistance=3000.0
fGrassMinStartFadeDistance=500.0
fGrassMaxStartFadeDistance=8000.0
";

    /// [`FULL_INI`] with the settings in `overrides` (INI text) replaced.
    pub fn ini_with(overrides: &str) -> GameIni {
        GameIni::from_texts([FULL_INI, overrides])
    }
}
