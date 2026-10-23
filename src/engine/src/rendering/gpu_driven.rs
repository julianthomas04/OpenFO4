use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::{Arc, Mutex};

use glam::Vec3;

use crate::assets::types::effect_params::{
    EffectBlend, EffectKey, EffectParams, EnvKey, EnvParams,
};
use crate::rendering::cube_array::CubeHandle;
use crate::rendering::gpu_slab::{GpuSlab, SlabElement};
use crate::rendering::lighting::cell_lighting::srgb_to_linear;

pub const MAX_TEXTURES: u32 = 8192;

// ---------- GPU-side structs (must match WGSL) ----------

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
#[allow(
    clippy::pub_underscore_fields,
    reason = "explicit padding must stay a public field so the Pod struct matches the WGSL layout"
)]
pub struct GpuInstance {
    pub transform: [[f32; 4]; 4],
    pub aabb_min: [f32; 3],
    pub _pad0: f32,
    pub aabb_max: [f32; 3],
    pub _pad1: f32,
    pub index_offset: u32,
    pub index_count: u32,
    pub vertex_offset: u32,
    pub material_id: u32,
    pub alpha_cutoff: f32,
    pub alpha_mult: f32,
    pub flags: u32,
    pub _pad2: u32,
}

#[repr(C)]
#[derive(Copy, Clone, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[allow(
    clippy::pub_underscore_fields,
    reason = "explicit padding must stay a public field so the Pod struct matches the WGSL layout"
)]
pub struct GpuMaterial {
    pub diffuse_index: u32,
    pub normal_index: u32,
    pub specular_index: u32,
    pub glow_index: u32,
    /// `pack4x8unorm` of the source 8-bit colour; carried for envmaps, not read by
    /// the shaders yet.
    pub specular_color: u32,
    /// Specular multiplier when specular is enabled, 0 when disabled.
    pub specular_scale: f32,
    pub smoothness: f32,
    pub fresnel_power: f32,
    /// rgb: linear emissive colour times multiplier, w: reserved (0).
    pub emissive: [f32; 4],
    /// rgb: linear base colour * scale. w: lighting influence 0..1 (0 = unlit).
    pub effect_color: [f32; 4],
    /// x,y: falloff start/stop facing threshold (dot(n,v) in [-1,1]); z,w: start/stop opacity.
    pub falloff: [f32; 4],
    /// `CubeArray` slot, 0 = none.
    pub env_cube_index: u32,
    /// 2D bindless slot, `WHITE_SLOT` when none.
    pub env_mask_index: u32,
    pub env_scale: f32,
    pub effect_flags: u32,
    /// 2D bindless slot, `WHITE_SLOT` when none.
    pub palette_index: u32,
    pub _pad: [u32; 3],
}

const _: () = assert!(std::mem::size_of::<GpuMaterial>() == 112);

pub const EFFECT_FLAG_EFFECT: u32 = 1;
pub const EFFECT_FLAG_ADDITIVE: u32 = 2;
pub const EFFECT_FLAG_MULTIPLY: u32 = 4;
pub const EFFECT_FLAG_FALLOFF: u32 = 8;
pub const EFFECT_FLAG_FALLOFF_COLOR: u32 = 16;
pub const EFFECT_FLAG_PALETTE_COLOR: u32 = 32;
pub const EFFECT_FLAG_PALETTE_ALPHA: u32 = 64;
pub const EFFECT_FLAG_ENV: u32 = 128;

impl GpuMaterial {
    pub fn new(
        slots: MaterialSlots,
        surface: &SurfaceParams,
        effect: Option<&EffectParams>,
        env: Option<(&EnvParams, CubeHandle)>,
    ) -> Self {
        let mut material = Self {
            diffuse_index: slots.diffuse,
            normal_index: slots.normal,
            specular_index: slots.specular,
            glow_index: slots.glow,
            specular_color: surface.packed_specular_color(),
            specular_scale: surface.specular_scale(),
            smoothness: surface.smoothness,
            fresnel_power: surface.fresnel_power,
            emissive: surface.emissive.extend(0.0).to_array(),
            effect_color: [0.0; 4],
            falloff: [0.0; 4],
            env_cube_index: CubeHandle::NONE.slot(),
            env_mask_index: slots.env_mask,
            env_scale: 1.0,
            effect_flags: 0,
            palette_index: slots.palette,
            _pad: [0; 3],
        };
        if let Some(effect) = effect {
            material.apply_effect(effect);
        }
        if let Some((env, cube)) = env {
            material.env_cube_index = cube.slot();
            material.env_scale = env.mask_scale;
            material.effect_flags |= EFFECT_FLAG_ENV;
        }
        material
    }

    fn apply_effect(&mut self, effect: &EffectParams) {
        let [r, g, b] = effect.base_color_srgb.map(srgb_to_linear);
        let scale = effect.base_color_scale;
        self.effect_color = [
            r * scale,
            g * scale,
            b * scale,
            effect.lighting_influence.unwrap_or(0.0).clamp(0.0, 1.0),
        ];
        self.effect_flags |= EFFECT_FLAG_EFFECT
            | match effect.blend {
                EffectBlend::Alpha => 0,
                EffectBlend::Additive => EFFECT_FLAG_ADDITIVE,
                EffectBlend::Multiply => EFFECT_FLAG_MULTIPLY,
            };
        if let Some(falloff) = &effect.falloff {
            self.falloff = [
                falloff.start_facing,
                falloff.stop_facing,
                falloff.start_opacity,
                falloff.stop_opacity,
            ];
            self.effect_flags |= EFFECT_FLAG_FALLOFF;
            if falloff.scales_color {
                self.effect_flags |= EFFECT_FLAG_FALLOFF_COLOR;
            }
        }
        if let Some(palette) = &effect.palette {
            if palette.recolours_color {
                self.effect_flags |= EFFECT_FLAG_PALETTE_COLOR;
            }
            if palette.drives_alpha {
                self.effect_flags |= EFFECT_FLAG_PALETTE_ALPHA;
            }
        }
    }
}

/// Bindless texture slots of one material.
#[derive(Debug, Copy, Clone)]
pub struct MaterialSlots {
    pub diffuse: u32,
    pub normal: u32,
    pub specular: u32,
    pub glow: u32,
    /// `WHITE_SLOT` when the material has no env mask.
    pub env_mask: u32,
    /// `WHITE_SLOT` when the material has no palette.
    pub palette: u32,
}

/// Scalar/colour lighting parameters of a material (BGSM or inline shader
/// property values).
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct SurfaceParams {
    pub specular_enabled: bool,
    pub specular_color: [u8; 3],
    pub specular_mult: f32,
    pub smoothness: f32,
    pub fresnel_power: f32,
    /// Linear emissive colour times its multiplier.
    pub emissive: Vec3,
}

/// The data-free default: specular disabled and smoothness 0, i.e. fully matte.
pub const LEGACY_SURFACE: SurfaceParams = SurfaceParams {
    specular_enabled: false,
    specular_color: [255, 255, 255],
    specular_mult: 1.0,
    smoothness: 0.0,
    fresnel_power: 5.0,
    emissive: Vec3::ZERO,
};

impl SurfaceParams {
    fn specular_scale(&self) -> f32 {
        if self.specular_enabled {
            self.specular_mult
        } else {
            0.0
        }
    }

    fn packed_specular_color(&self) -> u32 {
        let [r, g, b] = self.specular_color;
        u32::from_le_bytes([r, g, b, 0])
    }

    /// Hashable identity of the parameters; floats compare by bit pattern.
    fn key_bits(&self) -> [u32; 8] {
        [
            u32::from(self.specular_enabled),
            self.packed_specular_color(),
            self.specular_mult.to_bits(),
            self.smoothness.to_bits(),
            self.fresnel_power.to_bits(),
            self.emissive.x.to_bits(),
            self.emissive.y.to_bits(),
            self.emissive.z.to_bits(),
        ]
    }
}

/// Identity of a registered material: every texture path, the surface
/// parameters and the effect/environment parameters, so materials sharing
/// textures never collide.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MaterialKey {
    diffuse: Option<String>,
    normal: Option<String>,
    specular: Option<String>,
    glow: Option<String>,
    surface_bits: [u32; 8],
    effect: Option<EffectKey>,
    env: Option<EnvKey>,
}

impl MaterialKey {
    pub fn new(
        diffuse: Option<&str>,
        normal: Option<&str>,
        specular: Option<&str>,
        glow: Option<&str>,
        surface: &SurfaceParams,
        effect: Option<&EffectParams>,
        env: Option<&EnvParams>,
    ) -> Self {
        Self {
            diffuse: diffuse.map(str::to_string),
            normal: normal.map(str::to_string),
            specular: specular.map(str::to_string),
            glow: glow.map(str::to_string),
            surface_bits: surface.key_bits(),
            effect: effect.map(EffectParams::key),
            env: env.map(EnvParams::key),
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
#[allow(
    clippy::pub_underscore_fields,
    reason = "explicit padding must stay a public field so the Pod struct matches the WGSL layout"
)]
pub struct GpuTerrainInstance {
    pub aabb_min: [f32; 3],
    pub _pad0: f32,
    pub aabb_max: [f32; 3],
    pub _pad1: f32,
    pub index_offset: u32,
    pub index_count: u32,
    pub vertex_offset: u32,
    pub splat_layer: u32,
    pub layer_indices: [u32; 4],
    pub flags: [u32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DrawIndexedIndirect {
    pub index_count: u32,
    pub instance_count: u32,
    pub first_index: u32,
    pub base_vertex: i32,
    pub first_instance: u32,
}

pub const INSTANCE_FLAGS_BLEND: u32 = 1;
pub const INSTANCE_FLAGS_TWO_SIDED: u32 = 2;
/// Editor-only marker (trigger volume, path/effect marker): culled unless the
/// debug "Show markers" toggle is on.
pub const INSTANCE_FLAGS_MARKER: u32 = 4;
/// Effect-shader material (glass, steam, glow cards): drawn by the main cull
/// like any geometry, but never casts a shadow.
pub const INSTANCE_FLAGS_EFFECT: u32 = 8;
/// Decal material (`bDecal`/`bDepthBias` in the source BGSM/BGEM): routed into the
/// decal draw buckets whose pipelines carry a forward depth bias to kill z-fighting
/// against the coplanar surface the decal sits on.
pub const INSTANCE_FLAGS_DECAL: u32 = 16;
/// Water surface (`BSWaterShaderProperty` shape): never routed into an object
/// bucket and never a shadow caster; the water stage draws it from the CPU
/// mirror of the instance (`WorldStreamer::for_each_placed_water`).
pub const INSTANCE_FLAGS_WATER: u32 = 32;
/// Multiplicative blend material, set together with `INSTANCE_FLAGS_BLEND`.
pub const INSTANCE_FLAGS_MULTIPLY: u32 = 64;
/// Effect geometry hidden until effect animation and soft fading exist (FX
/// models, non-BGEM effect shapes): culled unless the debug "Show FX" toggle is
/// on, and never a shadow caster. Distinct from `INSTANCE_FLAGS_EFFECT`, which
/// only means "effect material".
pub const INSTANCE_FLAGS_HIDDEN_FX: u32 = 128;

/// Terrain cell textured by a baked level-4 LOD tile (`flags[0]` bit 0):
/// splat weights are ignored and vertex UVs are tile-space, not cell-space.
pub const TERRAIN_FLAGS_BAKED: u32 = 1;

// ---------- Texture Array ----------

/// 1x1 magenta — failed/missing diffuse loads land here, visibly.
pub const FALLBACK_SLOT: u32 = 0;
/// 1x1 flat normal [128,128,255] — failed/missing normal maps degrade gracefully.
pub const FLAT_NORMAL_SLOT: u32 = 1;
/// 1x1 linear white — a missing `_s` map lets the material scalars drive the result.
pub const WHITE_SLOT: u32 = 2;
/// 1x1 linear black — a missing glow map emits nothing.
pub const BLACK_SLOT: u32 = 3;
const RESERVED_SLOTS: u32 = 4;

fn create_1x1_view(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    format: wgpu::TextureFormat,
    data: [u8; 4],
) -> Arc<wgpu::TextureView> {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    Arc::new(tex.create_view(&wgpu::TextureViewDescriptor::default()))
}

/// Identifies one occupancy of a bindless slot. The generation distinguishes
/// the current occupant from earlier textures that held the same slot, so
/// asynchronous work addressed to a recycled slot can be recognised as stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureHandle {
    pub slot: u32,
    pub generation: u32,
}

struct SlotEntry {
    slot: u32,
    refcount: u32,
    generation: u32,
}

/// What changed in the slot table since the last `take_slot_changes`; decides
/// whether (and how much of) the material bind group and shadow caches must
/// be refreshed.
#[derive(Debug, Default)]
pub struct SlotChanges {
    /// A free slot was reassigned to a different path: draws made before the
    /// next rebind sample the previous occupant's texture, so shadow maps that
    /// sampled them are stale.
    pub recycled_slots: Vec<u32>,
    /// The same texture got a new view (resolution change). Rebind only.
    pub view_swapped_slots: Vec<u32>,
    /// A stand-in view was replaced by the real texture. Rebind, and shadows
    /// that drew alpha-tested casters with the stand-in are stale.
    pub placeholder_resolved_slots: Vec<u32>,
    /// A texture was released. Rebind only, so its VRAM can actually be freed.
    pub vacated_slots: Vec<u32>,
}

impl SlotChanges {
    pub fn is_empty(&self) -> bool {
        self.recycled_slots.is_empty()
            && self.view_swapped_slots.is_empty()
            && self.placeholder_resolved_slots.is_empty()
            && self.vacated_slots.is_empty()
    }

    pub fn merge(&mut self, other: SlotChanges) {
        self.recycled_slots.extend(other.recycled_slots);
        self.view_swapped_slots.extend(other.view_swapped_slots);
        self.placeholder_resolved_slots
            .extend(other.placeholder_resolved_slots);
        self.vacated_slots.extend(other.vacated_slots);
    }
}

/// The bind group holds a multiple of this many views, so slot high-water
/// growth rebuilds it once per step instead of once per new slot.
const BIND_GROUP_GRANULARITY: u32 = 256;

fn create_slot_mips_buffer(device: &wgpu::Device, slot_mips: &[SlotMips]) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("texture_slot_mips"),
        size: std::mem::size_of_val(slot_mips) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: true,
    });
    buffer
        .slice(..)
        .get_mapped_range_mut()
        .expect("freshly created mapped buffer exposes its full range")
        .copy_from_slice(bytemuck::cast_slice(slot_mips));
    buffer.unmap();
    buffer
}

/// First-resident-mip value of a slot bound to a stand-in view; mirrored by
/// `PLACEHOLDER_FIRST_MIP` in `shaders/mip_debug.wgsl`.
pub const PLACEHOLDER_FIRST_MIP: u32 = u32::MAX;

/// `SlotMips::feedback_skip` of a slot with no feedback data; mirrored by
/// `NO_FEEDBACK_SKIP` in `shaders/mip_debug.wgsl`.
pub const NO_FEEDBACK_SKIP: u32 = u32::MAX;

/// Per-slot mip state read by the mip debug views and the feedback pass;
/// mirrors `SlotMips` in `shaders/scene_bindings.wgsl`.
#[derive(Copy, Clone, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct SlotMips {
    /// File mip held by the bound view's mip 0, or `PLACEHOLDER_FIRST_MIP`.
    first_mip: u32,
    /// Skip the held feedback calls for (global skip and block alignment
    /// included, the convention of `first_mip`), or `NO_FEEDBACK_SKIP`.
    feedback_skip: u32,
}

impl SlotMips {
    const UNUSED: Self = Self {
        first_mip: PLACEHOLDER_FIRST_MIP,
        feedback_skip: NO_FEEDBACK_SKIP,
    };
}

pub struct TextureArray {
    pub views: Vec<Option<Arc<wgpu::TextureView>>>,
    next_slot: u32,
    path_to_slot: HashMap<String, SlotEntry>,
    slot_paths: Vec<Option<String>>,
    slot_generations: Vec<u32>,
    free_slots: Vec<u32>,
    fallback_view: Arc<wgpu::TextureView>,
    changes: SlotChanges,
    /// Mirror of the resident path set, shared with the streaming worker as a
    /// dedup hint: paths in here don't need their DDS pre-read. Maintained
    /// exclusively inside acquire/release so it cannot drift from
    /// `path_to_slot`. Readers must tolerate staleness: the main thread falls
    /// back to a placeholder plus an async load for the release race.
    resident: Arc<Mutex<HashSet<String>>>,
    /// Slots that ever served as the diffuse map of an alpha-tested caster.
    /// Only those are sampled by the shadow passes; see
    /// `build_shadow_bind_group_entries`.
    alpha_tested: Vec<bool>,
    new_alpha_slots: Vec<u32>,
    /// Per slot mip state, mirrored to `slot_mips_buffer` for the mip debug
    /// views through one dirty range.
    slot_mips: Vec<SlotMips>,
    slot_mips_dirty: Option<Range<usize>>,
    slot_mips_buffer: wgpu::Buffer,
}

impl TextureArray {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let fallback_view = create_1x1_view(
            device,
            queue,
            "texture_array_fallback",
            wgpu::TextureFormat::Rgba8UnormSrgb,
            [255, 0, 255, 255],
        );
        // Normal maps are data, not color: linear format.
        let flat_normal_view = create_1x1_view(
            device,
            queue,
            "texture_array_flat_normal",
            wgpu::TextureFormat::Rgba8Unorm,
            [128, 128, 255, 255],
        );

        let white_view = create_1x1_view(
            device,
            queue,
            "texture_array_white",
            wgpu::TextureFormat::Rgba8Unorm,
            [255, 255, 255, 255],
        );
        let black_view = create_1x1_view(
            device,
            queue,
            "texture_array_black",
            wgpu::TextureFormat::Rgba8Unorm,
            [0, 0, 0, 255],
        );

        let mut views: Vec<Option<Arc<wgpu::TextureView>>> = vec![None; MAX_TEXTURES as usize];
        views[FALLBACK_SLOT as usize] = Some(fallback_view.clone());
        views[FLAT_NORMAL_SLOT as usize] = Some(flat_normal_view);
        views[WHITE_SLOT as usize] = Some(white_view);
        views[BLACK_SLOT as usize] = Some(black_view);

        let slot_mips = vec![SlotMips::UNUSED; MAX_TEXTURES as usize];
        Self {
            views,
            next_slot: RESERVED_SLOTS,
            path_to_slot: HashMap::new(),
            slot_paths: vec![None; MAX_TEXTURES as usize],
            slot_generations: vec![0; MAX_TEXTURES as usize],
            free_slots: Vec::new(),
            fallback_view,
            changes: SlotChanges::default(),
            resident: Arc::new(Mutex::new(HashSet::new())),
            alpha_tested: vec![false; MAX_TEXTURES as usize],
            new_alpha_slots: Vec::new(),
            slot_mips_dirty: None,
            slot_mips_buffer: create_slot_mips_buffer(device, &slot_mips),
            slot_mips,
        }
    }

    pub fn slot_mips_buffer(&self) -> &wgpu::Buffer {
        &self.slot_mips_buffer
    }

    fn update_slot_mips(&mut self, slot: u32, change: impl FnOnce(&mut SlotMips)) {
        let index = slot as usize;
        let before = self.slot_mips[index];
        change(&mut self.slot_mips[index]);
        if self.slot_mips[index] == before {
            return;
        }
        let dirty = self.slot_mips_dirty.get_or_insert(index..index);
        dirty.start = dirty.start.min(index);
        dirty.end = dirty.end.max(index + 1);
    }

    fn set_first_mip(&mut self, slot: u32, first_mip: u32) {
        self.update_slot_mips(slot, |mips| mips.first_mip = first_mip);
    }

    /// Record the skip the held texture feedback calls for on `slot`, or
    /// `None` when there is no feedback data for it.
    pub fn set_feedback_skip(&mut self, slot: u32, skip: Option<u32>) {
        self.update_slot_mips(slot, |mips| {
            mips.feedback_skip = skip.unwrap_or(NO_FEEDBACK_SKIP);
        });
    }

    /// Upload the slot entries changed since the last call. The material bind
    /// group rebuild calls it so the buffer matches the bound views; the
    /// feedback readback calls it while a mip view is shown.
    pub fn upload_slot_mips(&mut self, queue: &wgpu::Queue) {
        let Some(dirty) = self.slot_mips_dirty.take() else {
            return;
        };
        queue.write_buffer(
            &self.slot_mips_buffer,
            (dirty.start * size_of::<SlotMips>()) as u64,
            bytemuck::cast_slice(&self.slot_mips[dirty]),
        );
    }

    /// Handle for the streaming worker's pre-read dedup (see `resident`).
    pub fn resident_paths(&self) -> Arc<Mutex<HashSet<String>>> {
        self.resident.clone()
    }

    /// Insert (or re-reference) a texture. Each `acquire`/`try_add_ref` must be
    /// balanced by one `release` with the returned handle. `None` when the
    /// array is full; nothing was inserted and there is nothing to release.
    /// `first_mip` is `PLACEHOLDER_FIRST_MIP` for a stand-in view.
    pub fn acquire(
        &mut self,
        path: &str,
        view: Arc<wgpu::TextureView>,
        first_mip: u32,
    ) -> Option<TextureHandle> {
        if let Some(handle) = self.try_add_ref(path) {
            return Some(handle);
        }

        let slot = if let Some(s) = self.free_slots.pop() {
            self.changes.recycled_slots.push(s);
            s
        } else {
            if self.next_slot >= MAX_TEXTURES {
                tracing::warn!("Texture array full, using fallback slot");
                return None;
            }
            let s = self.next_slot;
            self.next_slot += 1;
            s
        };

        let generation = self.slot_generations[slot as usize].wrapping_add(1);
        self.slot_generations[slot as usize] = generation;
        self.views[slot as usize] = Some(view);
        self.set_first_mip(slot, first_mip);
        self.slot_paths[slot as usize] = Some(path.to_string());
        self.path_to_slot.insert(
            path.to_string(),
            SlotEntry {
                slot,
                refcount: 1,
                generation,
            },
        );
        self.resident.lock().unwrap().insert(path.to_string());
        Some(TextureHandle { slot, generation })
    }

    /// Add a reference to an already-resident texture without providing a view.
    pub fn try_add_ref(&mut self, path: &str) -> Option<TextureHandle> {
        let entry = self.path_to_slot.get_mut(path)?;
        entry.refcount += 1;
        Some(TextureHandle {
            slot: entry.slot,
            generation: entry.generation,
        })
    }

    /// Drop one reference. Returns true when the texture was evicted, so the
    /// caller can drop everything it keeps per slot. A handle whose slot has
    /// since been recycled is ignored.
    pub fn release(&mut self, handle: TextureHandle) -> bool {
        if !self.is_current(handle) {
            return false;
        }
        let Some(path) = self.slot_paths[handle.slot as usize].clone() else {
            return false;
        };
        let Some(entry) = self.path_to_slot.get_mut(&path) else {
            return false;
        };
        entry.refcount -= 1;
        if entry.refcount > 0 {
            return false;
        }
        self.path_to_slot.remove(&path);
        self.slot_paths[handle.slot as usize] = None;
        self.views[handle.slot as usize] = None;
        self.update_slot_mips(handle.slot, |mips| *mips = SlotMips::UNUSED);
        self.free_slots.push(handle.slot);
        self.resident.lock().unwrap().remove(&path);
        self.changes.vacated_slots.push(handle.slot);
        true
    }

    /// Whether `handle` still names the texture occupying its slot.
    pub fn is_current(&self, handle: TextureHandle) -> bool {
        self.slot_paths
            .get(handle.slot as usize)
            .is_some_and(Option::is_some)
            && self.slot_generations[handle.slot as usize] == handle.generation
    }

    /// Swap the view bound to a live slot (resolution change). The bind group
    /// keeps the old view until the next rebuild. False for a stale handle.
    pub fn replace_view(
        &mut self,
        handle: TextureHandle,
        view: Arc<wgpu::TextureView>,
        first_mip: u32,
    ) -> bool {
        if !self.is_current(handle) {
            return false;
        }
        self.views[handle.slot as usize] = Some(view);
        self.set_first_mip(handle.slot, first_mip);
        self.changes.view_swapped_slots.push(handle.slot);
        true
    }

    /// Replace a slot's stand-in view with the loaded texture's view; reported
    /// apart from resolution swaps because shadows drawn with the stand-in are
    /// stale. False for a stale handle.
    pub fn resolve_placeholder(
        &mut self,
        handle: TextureHandle,
        view: Arc<wgpu::TextureView>,
        first_mip: u32,
    ) -> bool {
        if !self.is_current(handle) {
            return false;
        }
        self.views[handle.slot as usize] = Some(view);
        self.set_first_mip(handle.slot, first_mip);
        self.changes.placeholder_resolved_slots.push(handle.slot);
        true
    }

    /// Drain the accumulated slot-table changes.
    pub fn take_slot_changes(&mut self) -> SlotChanges {
        std::mem::take(&mut self.changes)
    }

    /// View of a reserved slot, used as the stand-in while a texture loads.
    pub fn reserved_view(&self, slot: u32) -> Arc<wgpu::TextureView> {
        debug_assert!(slot < RESERVED_SLOTS);
        self.views[slot as usize]
            .clone()
            .unwrap_or_else(|| self.fallback_view.clone())
    }

    pub fn fallback_view(&self) -> &Arc<wgpu::TextureView> {
        &self.fallback_view
    }

    /// High-water mark of allocated slots.
    pub fn used_slots(&self) -> u32 {
        self.next_slot
    }

    /// Number of views the material bind group holds: the high-water mark
    /// rounded up to `BIND_GROUP_GRANULARITY`. Growth of this value forces an
    /// immediate rebuild, because indexing past a bound array is UB.
    pub fn bind_group_len(&self) -> u32 {
        self.next_slot
            .next_multiple_of(BIND_GROUP_GRANULARITY)
            .min(MAX_TEXTURES)
    }

    /// Number of currently live (referenced) textures.
    pub fn live_slots(&self) -> u32 {
        RESERVED_SLOTS + self.path_to_slot.len() as u32
    }

    /// Record that `slot` is sampled by alpha-tested shadow casters. Sticky: a
    /// recycled slot merely stays in the set, which costs nothing but a real
    /// view in the shadow bind group.
    pub fn mark_alpha_tested(&mut self, slot: u32) {
        if slot < RESERVED_SLOTS {
            return;
        }
        let marked = &mut self.alpha_tested[slot as usize];
        if !*marked {
            *marked = true;
            self.new_alpha_slots.push(slot);
        }
    }

    /// Slots added to the alpha-tested set since the last call: the shadow
    /// bind group is stale for them.
    pub fn take_new_alpha_slots(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.new_alpha_slots)
    }

    /// Same layout as `build_bind_group_entries`, but slots no alpha-tested
    /// caster samples hold the white stand-in. wgpu-core merges every view of
    /// a bind group into each pass's usage scope at command encoding time, so
    /// a shadow pass binding ~5000 distinct textures pays for all of them; the
    /// repeated stand-in is one cache-hot texture.
    pub fn build_shadow_bind_group_entries(&self) -> Vec<&wgpu::TextureView> {
        let stand_in = self.views[WHITE_SLOT as usize]
            .as_ref()
            .map_or(self.fallback_view.as_ref(), |arc| arc.as_ref());
        self.views[..self.bind_group_len() as usize]
            .iter()
            .enumerate()
            .map(|(slot, view)| {
                if slot as u32 >= RESERVED_SLOTS && !self.alpha_tested[slot] {
                    return stand_in;
                }
                view.as_ref()
                    .map_or(self.fallback_view.as_ref(), |arc| arc.as_ref())
            })
            .collect()
    }

    /// Only the `bind_group_len` prefix is returned; `PARTIALLY_BOUND_BINDING_ARRAY`
    /// (requested in device.rs) lets the bind group hold fewer entries than the
    /// 8192-slot layout. Every index the shaders can produce is < `used_slots`.
    pub fn build_bind_group_entries(&self) -> Vec<&wgpu::TextureView> {
        let stand_in = self.views[WHITE_SLOT as usize]
            .as_ref()
            .map_or(self.fallback_view.as_ref(), |arc| arc.as_ref());
        let bound_len = crate::rendering::debug_skip::material_array_len().unwrap_or(u32::MAX);
        self.views[..self.bind_group_len() as usize]
            .iter()
            .enumerate()
            .map(|(slot, v)| {
                if slot as u32 >= bound_len.max(RESERVED_SLOTS) {
                    return stand_in;
                }
                v.as_ref()
                    .map_or(self.fallback_view.as_ref(), std::convert::AsRef::as_ref)
            })
            .collect()
    }
}

// ---------- Splatmap Array ----------

/// One layer per resident landscape cell. Lives outside the bindless texture
/// array because splat slots scale O(radius^2) with zero cross-cell sharing and
/// would exhaust the 8192 bindless slots at moderate LOD radii.
pub const SPLAT_SIZE: u32 = 128;
/// D3D12 caps 2D array textures at 2048 layers.
pub const SPLAT_LAYERS: u32 = 2048;
/// Solid red = "base layer fully", used when layers run out.
pub const SPLAT_FALLBACK_LAYER: u32 = 0;

pub struct SplatmapArray {
    texture: wgpu::Texture,
    /// `D2Array` view over the whole texture; created once, never invalidated.
    pub view: wgpu::TextureView,
    key_to_layer: HashMap<String, (u32, u32)>, // key -> (layer, refcount)
    free_layers: Vec<u32>,
    next_layer: u32,
}

impl SplatmapArray {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("splatmap_array"),
            size: wgpu::Extent3d {
                width: SPLAT_SIZE,
                height: SPLAT_SIZE,
                depth_or_array_layers: SPLAT_LAYERS,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm, // weights, not color: linear
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let fallback = [255u8, 0, 0, 0].repeat((SPLAT_SIZE * SPLAT_SIZE) as usize);
        Self::write_layer(&texture, queue, SPLAT_FALLBACK_LAYER, &fallback);

        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("splatmap_array_view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        Self {
            texture,
            view,
            key_to_layer: HashMap::new(),
            free_layers: Vec::new(),
            next_layer: 1,
        }
    }

    fn write_layer(texture: &wgpu::Texture, queue: &wgpu::Queue, layer: u32, data: &[u8]) {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: layer,
                },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SPLAT_SIZE * 4),
                rows_per_image: Some(SPLAT_SIZE),
            },
            wgpu::Extent3d {
                width: SPLAT_SIZE,
                height: SPLAT_SIZE,
                depth_or_array_layers: 1,
            },
        );
    }

    /// `data` is `SPLAT_SIZE^2` RGBA8. Keys are shared between full and LOD
    /// variants of a cell (identical splat data) — refcounting covers the
    /// promotion/demotion overlap window.
    pub fn acquire(&mut self, key: &str, data: &[u8], queue: &wgpu::Queue) -> u32 {
        if let Some((layer, refcount)) = self.key_to_layer.get_mut(key) {
            *refcount += 1;
            return *layer;
        }

        let layer = if let Some(l) = self.free_layers.pop() {
            l
        } else {
            if self.next_layer >= SPLAT_LAYERS {
                tracing::warn!("Splatmap array full, returning fallback layer");
                return SPLAT_FALLBACK_LAYER;
            }
            let l = self.next_layer;
            self.next_layer += 1;
            l
        };

        Self::write_layer(&self.texture, queue, layer, data);
        self.key_to_layer.insert(key.to_string(), (layer, 1));
        layer
    }

    pub fn release(&mut self, key: &str) {
        let Some((layer, refcount)) = self.key_to_layer.get_mut(key) else {
            return;
        };
        *refcount -= 1;
        if *refcount == 0 {
            let layer = *layer;
            self.key_to_layer.remove(key);
            self.free_layers.push(layer);
        }
    }

    pub fn live_layers(&self) -> u32 {
        1 + self.key_to_layer.len() as u32
    }
}

// ---------- Material Registry ----------

pub struct MaterialRegistry {
    materials: Vec<GpuMaterial>,
    key_to_id: HashMap<MaterialKey, u32>,
    dirty: bool,
    pub buffer: wgpu::Buffer,
}

/// Drawn by shapes without a diffuse map (marker gizmos): magenta on purpose.
pub const FALLBACK_MATERIAL_ID: u32 = 0;

impl MaterialRegistry {
    pub fn new(device: &wgpu::Device) -> Self {
        let initial_capacity = 1024u64;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("material_buffer"),
            size: initial_capacity * std::mem::size_of::<GpuMaterial>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let fallback = GpuMaterial::new(
            MaterialSlots {
                diffuse: FALLBACK_SLOT,
                normal: FLAT_NORMAL_SLOT,
                specular: WHITE_SLOT,
                glow: BLACK_SLOT,
                env_mask: WHITE_SLOT,
                palette: WHITE_SLOT,
            },
            &LEGACY_SURFACE,
            None,
            None,
        );

        Self {
            materials: vec![fallback],
            key_to_id: HashMap::new(),
            dirty: true,
            buffer,
        }
    }

    /// The registry itself is append-only — bounded by unique material keys
    /// ever seen at 48 B each, which is acceptable. But texture slots can be
    /// freed and reused after cell eviction, so on a key hit with different
    /// slots the entry is updated in place (safe: a stale entry implies no
    /// loaded cell pins its old slots).
    pub fn get_or_insert(&mut self, key: &MaterialKey, material: GpuMaterial) -> u32 {
        if let Some(&id) = self.key_to_id.get(key) {
            let registered = &mut self.materials[id as usize];
            if *registered != material {
                *registered = material;
                self.dirty = true;
            }
            return id;
        }

        let id = self.materials.len() as u32;
        self.materials.push(material);
        self.key_to_id.insert(key.clone(), id);
        self.dirty = true;
        id
    }

    /// Returns true when the underlying buffer was reallocated (bind groups
    /// referencing it must be rebuilt).
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        if !self.dirty {
            return false;
        }

        let mut reallocated = false;
        let required_size = self.materials.len() as u64 * std::mem::size_of::<GpuMaterial>() as u64;
        if required_size > self.buffer.size() {
            let new_size = (required_size * 2).next_power_of_two();
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("material_buffer"),
                size: new_size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            reallocated = true;
        }

        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.materials));
        self.dirty = false;
        reallocated
    }

    pub fn len(&self) -> usize {
        self.materials.len()
    }

    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }
}

// ---------- Instance Buffer (slab) ----------

/// Per-cell slab of object instances. Freed ranges zero-fill, and a zeroed
/// instance has `index_count == 0`, which the cull shader already skips.
pub type InstanceBuffer = GpuSlab<GpuInstance>;

impl SlabElement for GpuInstance {
    /// 64 Ki instances = 8 MB; covers most of a full+LOD ring before the first
    /// growth copy.
    const INITIAL_CAPACITY: u32 = 64 * 1024;
    const COMPACTION_FLOOR: u32 = 16 * 1024;
}

// ---------- Terrain Instance Buffer ----------

pub struct TerrainInstanceBuffer {
    pub instances: Vec<GpuTerrainInstance>,
    pub buffer: wgpu::Buffer,
    capacity: u64,
    pub dirty: bool,
}

impl TerrainInstanceBuffer {
    pub fn new(device: &wgpu::Device) -> Self {
        let capacity = 512u64;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain_instance_buffer"),
            size: capacity * std::mem::size_of::<GpuTerrainInstance>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            instances: Vec::new(),
            buffer,
            capacity,
            dirty: false,
        }
    }

    /// Returns true when the underlying buffer was reallocated.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        if !self.dirty {
            return false;
        }

        let mut reallocated = false;
        let required = self.instances.len().max(1) as u64;
        if required > self.capacity {
            let new_capacity = (required * 2).next_power_of_two();
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("terrain_instance_buffer"),
                size: new_capacity * std::mem::size_of::<GpuTerrainInstance>() as u64,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.capacity = new_capacity;
            reallocated = true;
        }

        if !self.instances.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.instances));
        }
        self.dirty = false;
        reallocated
    }

    pub fn len(&self) -> u32 {
        self.instances.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }
}

// ---------- Indirect Draw Buffers ----------

/// Bucket order is load-bearing: it must match the routing in `gpu_cull.wgsl`,
/// `ObjectPipelines::pipelines_in_bucket_order`, and the count-buffer offsets
/// in the draw loop.
pub const BUCKET_OPAQUE_CULLED: usize = 0;
pub const BUCKET_OPAQUE_TWOSIDED: usize = 1;
pub const BUCKET_ALPHATEST_CULLED: usize = 2;
pub const BUCKET_ALPHATEST_TWOSIDED: usize = 3;
/// Alpha-tested decal geometry: an overlay drawn after terrain with a forward depth
/// bias so it wins the depth TEST against its coplanar host, but with depth WRITE
/// off — a decal must not deposit its pulled-forward depth into the buffer, or that
/// biased depth feeds the `HiZ` pyramid and over-occludes geometry next frame.
pub const BUCKET_DECAL_ALPHATEST_CULLED: usize = 4;
pub const BUCKET_DECAL_ALPHATEST_TWOSIDED: usize = 5;
pub const BUCKET_BLEND_CULLED: usize = 6;
pub const BUCKET_BLEND_TWOSIDED: usize = 7;
/// Alpha-blended decal geometry: blend pipeline (no depth write) plus the forward
/// depth bias, so the decal reliably passes the depth test against its host surface.
pub const BUCKET_DECAL_BLEND_CULLED: usize = 8;
pub const BUCKET_DECAL_BLEND_TWOSIDED: usize = 9;
/// Multiplicative blend overlays (destination colour scaled by the fragment): blend
/// pipelines with depth write off, never decal-biased.
pub const BUCKET_MULTIPLY_CULLED: usize = 10;
pub const BUCKET_MULTIPLY_TWOSIDED: usize = 11;
pub const DRAW_BUCKET_COUNT: usize = 12;
/// Buckets [0, `FIRST_AFTER_TERRAIN_BUCKET`) write depth and draw before terrain.
/// Buckets [`FIRST_AFTER_TERRAIN_BUCKET`, `DRAW_BUCKET_COUNT`) are overlays — decals and
/// alpha-blended geometry — drawn after terrain with depth write OFF (they only
/// depth-test), so they never pollute the depth buffer / `HiZ` occlusion pyramid.
pub const FIRST_AFTER_TERRAIN_BUCKET: usize = BUCKET_DECAL_ALPHATEST_CULLED;
/// The decal buckets, alpha-tested and blended, belong to the opaque scene: they draw
/// straight after terrain, before the water, so water covers and refracts the decals
/// of submerged rock and riverbeds. Fallout 4 is a deferred renderer whose forward
/// water pass neither writes depth above the surface nor runs before its decals, so a
/// decal drawn after the water would paint over it wherever it lies under the surface.
pub const DECAL_BUCKETS: [usize; 4] = [
    BUCKET_DECAL_ALPHATEST_CULLED,
    BUCKET_DECAL_ALPHATEST_TWOSIDED,
    BUCKET_DECAL_BLEND_CULLED,
    BUCKET_DECAL_BLEND_TWOSIDED,
];
/// The blended non-decal buckets: translucent geometry drawn after the water.
pub const TRANSLUCENT_BUCKETS: [usize; 4] = [
    BUCKET_BLEND_CULLED,
    BUCKET_BLEND_TWOSIDED,
    BUCKET_MULTIPLY_CULLED,
    BUCKET_MULTIPLY_TWOSIDED,
];
pub const DRAW_STRIDE: u64 = std::mem::size_of::<DrawIndexedIndirect>() as u64;

/// CPU mirror of the bucket routing in `gpu_cull.wgsl` (`cull_main`): the set of
/// draw buckets an instance with these material properties could be routed into,
/// taken across BOTH `decal_bias_enabled` states. The marker flag bit only
/// *suppresses* a draw in the shader (`show_markers`, `show_fx`), never reroutes it,
/// so it is ignored here — counting a marker instance toward its would-be
/// bucket is a safe over-count. A decal-flagged instance reaches two buckets
/// (its depth-biased decal bucket when the bias is on, its plain bucket when
/// off); everything else reaches exactly one.
///
/// This is the guaranteed upper bound the per-bucket indirect buffers are sized
/// against, so it MUST remain a superset of the shader's routing for every
/// toggle state. The `potential_buckets_mirrors_shader` test pins it to the
/// shader logic; if you change the routing in `gpu_cull.wgsl`, change this too.
pub fn potential_buckets(flags: u32, alpha_cutoff: f32) -> impl Iterator<Item = usize> {
    let two_sided = usize::from(flags & INSTANCE_FLAGS_TWO_SIDED != 0);
    let is_blend = flags & INSTANCE_FLAGS_BLEND != 0;
    let has_decal = flags & INSTANCE_FLAGS_DECAL != 0;
    let is_multiply = flags & INSTANCE_FLAGS_MULTIPLY != 0;
    let is_alphatest = !is_blend && alpha_cutoff > 0.0;

    // (plain bucket when decal bias is off, decal bucket when it is on). A
    // non-blend decal always routes to the alpha-test decal bucket when the bias
    // is on, matching the shader's `else if (is_decal)` arm that precedes the
    // alpha-test arm.
    let (plain, decal) = if is_blend && is_multiply {
        (BUCKET_MULTIPLY_CULLED, BUCKET_MULTIPLY_CULLED)
    } else if is_blend {
        (BUCKET_BLEND_CULLED, BUCKET_DECAL_BLEND_CULLED)
    } else if is_alphatest {
        (BUCKET_ALPHATEST_CULLED, BUCKET_DECAL_ALPHATEST_CULLED)
    } else {
        (BUCKET_OPAQUE_CULLED, BUCKET_DECAL_ALPHATEST_CULLED)
    };

    let mut buckets = [usize::MAX; 2];
    buckets[0] = plain + two_sided;
    let mut n = 1;
    if has_decal && decal != plain {
        buckets[1] = decal + two_sided;
        n = 2;
    }
    buckets.into_iter().take(n)
}

/// Largest draw count that fits one buffer under the device's
/// `max_buffer_size`. Mirrors `InstanceBuffer`'s `max_capacity` clamp so a
/// pathological potential degrades to a device-capped buffer (the shader's
/// arrayLength guard then drops the overflow) instead of a failed allocation.
pub fn max_draws_per_buffer(device: &wgpu::Device) -> u32 {
    (device.limits().max_buffer_size / DRAW_STRIDE).min(u64::from(u32::MAX)) as u32
}

/// Draw capacity to size an indirect buffer for, given its resident potential
/// draw count: the potential plus a 1.5x growth headroom, floored, then clamped
/// to `max_draws`. The potential is already a hard upper bound (see
/// [`potential_buckets`]), so the headroom only amortizes reallocations — it
/// is not a correctness margin. u64 math avoids overflow before the clamp.
pub fn target_capacity(potential: u32, floor: u32, max_draws: u32) -> u32 {
    let with_headroom = u64::from(potential) * 3 / 2;
    with_headroom
        .max(u64::from(floor))
        .min(u64::from(max_draws)) as u32
}

pub struct IndirectDrawBuffers {
    pub draw_buffers: [wgpu::Buffer; DRAW_BUCKET_COUNT],
    pub count_buffer: wgpu::Buffer,
    /// Per-bucket capacity in draws. Each `draw_buffers[i]` holds `capacities[i]`
    /// `DrawIndexedIndirect` commands; the cull shader's atomic append into a
    /// bucket is bounded by the resident per-bucket potential kept ≤ this value.
    capacities: [u32; DRAW_BUCKET_COUNT],
}

impl IndirectDrawBuffers {
    /// Boot with every bucket at its floor (the empty-world potential of 0).
    pub fn new(device: &wgpu::Device, label: &str) -> Self {
        let max_per_buffer = max_draws_per_buffer(device);
        let capacities: [u32; DRAW_BUCKET_COUNT] =
            std::array::from_fn(|b| target_capacity(0, Self::bucket_floor(b), max_per_buffer));
        let draw_buffers =
            std::array::from_fn(|b| Self::create_draw_buffer(device, capacities[b], label, b));

        let count_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label}_draw_counts")),
            size: (DRAW_BUCKET_COUNT * std::mem::size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        Self {
            draw_buffers,
            count_buffer,
            capacities,
        }
    }

    /// Per-bucket floor (in draws) so a near-empty bucket isn't reallocated for
    /// every streamed instance: depth-writing buckets
    /// (`0..FIRST_AFTER_TERRAIN_BUCKET`) carry the bulk of the scene, while the
    /// overlay buckets (decals + blend) realistically hold few draws.
    fn bucket_floor(bucket: usize) -> u32 {
        if bucket < FIRST_AFTER_TERRAIN_BUCKET {
            4096
        } else {
            256
        }
    }

    fn create_draw_buffer(
        device: &wgpu::Device,
        capacity: u32,
        label: &str,
        bucket: usize,
    ) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("{label}_indirect_{bucket}")),
            size: (u64::from(capacity) * DRAW_STRIDE).max(DRAW_STRIDE),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Grow each bucket to fit its resident `potentials` draw count (with
    /// headroom + floor), reallocating only the buckets that need it — never
    /// shrinks. Returns true if any buffer was reallocated, so the caller
    /// rebuilds the cull bind group that references them.
    pub fn resize(
        &mut self,
        device: &wgpu::Device,
        potentials: [u32; DRAW_BUCKET_COUNT],
        label: &str,
    ) -> bool {
        let max_per_buffer = max_draws_per_buffer(device);
        let mut grew = false;
        #[allow(
            clippy::needless_range_loop,
            reason = "bucket indexes several parallel per-bucket arrays and is passed to bucket_floor"
        )]
        for bucket in 0..DRAW_BUCKET_COUNT {
            let target = target_capacity(
                potentials[bucket],
                Self::bucket_floor(bucket),
                max_per_buffer,
            );
            if target > self.capacities[bucket] {
                self.draw_buffers[bucket] = Self::create_draw_buffer(device, target, label, bucket);
                self.capacities[bucket] = target;
                grew = true;
            }
        }
        grew
    }

    /// Must run every frame before the indirect draws, even when the cull
    /// dispatch is skipped — otherwise stale counts from the previous frame
    /// draw phantom geometry after an N>0 -> 0 instance transition.
    pub fn clear_counts(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.clear_buffer(&self.count_buffer, 0, None);
    }

    pub fn capacity(&self, bucket: usize) -> u32 {
        self.capacities[bucket]
    }

    pub fn capacities(&self) -> [u32; DRAW_BUCKET_COUNT] {
        self.capacities
    }
}

// ---------- Terrain Indirect (single bucket) ----------

pub struct TerrainIndirectBuffer {
    pub draw_buffer: wgpu::Buffer,
    pub count_buffer: wgpu::Buffer,
    max_draws: u32,
}

impl TerrainIndirectBuffer {
    pub fn new(device: &wgpu::Device, max_draws: u32) -> Self {
        let buf_size = u64::from(max_draws) * DRAW_STRIDE;

        let draw_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain_indirect"),
            size: buf_size.max(DRAW_STRIDE),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let count_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain_draw_count"),
            size: std::mem::size_of::<u32>() as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        Self {
            draw_buffer,
            count_buffer,
            max_draws,
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, new_max: u32) {
        if new_max <= self.max_draws {
            return;
        }
        *self = Self::new(device, new_max);
    }

    /// See [`IndirectDrawBuffers::clear_counts`].
    pub fn clear_counts(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.clear_buffer(&self.count_buffer, 0, None);
    }

    pub fn max_draws(&self) -> u32 {
        self.max_draws
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::gpu_test::test_device;
    use bytemuck::Zeroable;

    const INSTANCE_SIZE: u64 = std::mem::size_of::<GpuInstance>() as u64;

    const INSTANCE_WGSL: &str = include_str!("shaders/instance.wgsl");

    /// Fields of the WGSL struct `name` as (field, type), in declaration order.
    fn wgsl_struct_fields(source: &str, name: &str) -> Vec<(String, String)> {
        let header = format!("struct {name} {{");
        let start = source.find(&header).expect("struct present") + header.len();
        let body = &source[start..start + source[start..].find('}').expect("struct closed")];
        body.lines()
            .filter_map(|line| line.trim().trim_end_matches(',').split_once(':'))
            .map(|(field, ty)| (field.trim().to_owned(), ty.trim().to_owned()))
            .collect()
    }

    /// (size, alignment) of the WGSL scalar and vec4 types used by `Material`.
    fn wgsl_type_layout(ty: &str) -> (usize, usize) {
        match ty {
            "u32" | "f32" => (4, 4),
            "vec4<f32>" => (16, 16),
            other => panic!("unsupported WGSL field type {other}"),
        }
    }

    #[test]
    fn specular_colour_reaches_the_gpu_as_the_authored_srgb_bytes() {
        let slots = MaterialSlots {
            diffuse: 0,
            normal: 0,
            specular: 0,
            glow: 0,
            env_mask: 0,
            palette: 0,
        };
        let surface = SurfaceParams {
            specular_color: [255, 128, 7],
            ..LEGACY_SURFACE
        };
        let material = GpuMaterial::new(slots, &surface, None, None);
        assert_eq!(material.specular_color.to_le_bytes(), [255, 128, 7, 0]);
    }

    #[test]
    fn gpu_material_matches_the_wgsl_material_struct() {
        macro_rules! offset {
            ($field:ident) => {
                std::mem::offset_of!(GpuMaterial, $field)
            };
        }
        let pad = offset!(_pad);
        let rust_layout = [
            ("diffuse_index", offset!(diffuse_index)),
            ("normal_index", offset!(normal_index)),
            ("specular_index", offset!(specular_index)),
            ("glow_index", offset!(glow_index)),
            ("specular_color", offset!(specular_color)),
            ("specular_scale", offset!(specular_scale)),
            ("smoothness", offset!(smoothness)),
            ("fresnel_power", offset!(fresnel_power)),
            ("emissive", offset!(emissive)),
            ("effect_color", offset!(effect_color)),
            ("falloff", offset!(falloff)),
            ("env_cube_index", offset!(env_cube_index)),
            ("env_mask_index", offset!(env_mask_index)),
            ("env_scale", offset!(env_scale)),
            ("effect_flags", offset!(effect_flags)),
            ("palette_index", offset!(palette_index)),
            ("_pad0", pad),
            ("_pad1", pad + 4),
            ("_pad2", pad + 8),
        ];

        let wgsl = wgsl_struct_fields(INSTANCE_WGSL, "Material");
        assert_eq!(
            wgsl.iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            rust_layout
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
        );

        let mut offset = 0usize;
        let mut max_align = 1;
        for ((name, ty), (_, rust_offset)) in wgsl.iter().zip(rust_layout) {
            let (size, align) = wgsl_type_layout(ty);
            offset = offset.next_multiple_of(align);
            assert_eq!(offset, rust_offset, "field {name} offset");
            offset += size;
            max_align = max_align.max(align);
        }
        assert_eq!(
            offset.next_multiple_of(max_align),
            std::mem::size_of::<GpuMaterial>()
        );
    }

    #[test]
    fn effect_flag_constants_match_between_rust_and_wgsl() {
        for (name, value) in [
            ("EFFECT_FLAG_EFFECT", EFFECT_FLAG_EFFECT),
            ("EFFECT_FLAG_ADDITIVE", EFFECT_FLAG_ADDITIVE),
            ("EFFECT_FLAG_MULTIPLY", EFFECT_FLAG_MULTIPLY),
            ("EFFECT_FLAG_FALLOFF", EFFECT_FLAG_FALLOFF),
            ("EFFECT_FLAG_FALLOFF_COLOR", EFFECT_FLAG_FALLOFF_COLOR),
            ("EFFECT_FLAG_PALETTE_COLOR", EFFECT_FLAG_PALETTE_COLOR),
            ("EFFECT_FLAG_PALETTE_ALPHA", EFFECT_FLAG_PALETTE_ALPHA),
            ("EFFECT_FLAG_ENV", EFFECT_FLAG_ENV),
        ] {
            let declaration = format!("const {name}: u32 = {value}u;");
            assert!(
                INSTANCE_WGSL.contains(&declaration),
                "missing `{declaration}`"
            );
        }
    }

    fn inst(index_count: u32) -> GpuInstance {
        GpuInstance {
            index_count,
            ..GpuInstance::zeroed()
        }
    }

    /// Blocking readback of `count` instances at `offset`.
    fn read_back(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        slab: &InstanceBuffer,
        offset: u32,
        count: u32,
    ) -> Vec<GpuInstance> {
        let bytes = u64::from(count) * INSTANCE_SIZE;
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("slab_test_staging"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_buffer_to_buffer(
            &slab.buffer,
            u64::from(offset) * INSTANCE_SIZE,
            &staging,
            0,
            bytes,
        );
        queue.submit(std::iter::once(encoder.finish()));
        let (map_sender, map_outcome) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                map_sender
                    .send(result)
                    .expect("readback outlives its map callback");
            });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("slab readback poll failed");
        map_outcome
            .recv()
            .expect("wait_indefinitely runs the map callback")
            .expect("slab readback map failed");
        let data = staging
            .slice(..)
            .get_mapped_range()
            .expect("mapped staging buffer exposes its full range");
        let out: Vec<GpuInstance> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        out
    }

    #[test]
    #[ignore = "needs a GPU adapter"]
    fn slab_alloc_free_reuse() {
        let (device, queue) = test_device("slab alloc free reuse").expect("a GPU adapter");
        let mut sb = InstanceBuffer::new(&device, "test_slab");

        let a = sb.allocate(100, &device, &queue).unwrap();
        let b = sb.allocate(50, &device, &queue).unwrap();
        let c = sb.allocate(25, &device, &queue).unwrap();
        assert_eq!((a, b, c), (0, 100, 150));
        assert_eq!(sb.dispatch_count(), 175);
        assert_eq!(sb.live_count(), 175);
        assert_eq!(sb.hole_count(), 0);

        // Free the back two; their ranges coalesce into 100..175.
        sb.free(b, 50, &queue);
        sb.free(c, 25, &queue);
        assert_eq!(sb.live_count(), 100);
        assert_eq!(sb.hole_count(), 75);

        // Best-fit reuse of the coalesced hole; cursor must not advance.
        let d = sb.allocate(60, &device, &queue).unwrap();
        assert_eq!(d, 100);
        assert_eq!(sb.dispatch_count(), 175);
        assert_eq!(sb.hole_count(), 15);
    }

    #[test]
    #[ignore = "needs a GPU adapter"]
    fn slab_free_zeroes_gpu_range() {
        let (device, queue) = test_device("slab free zeroes gpu range").expect("a GPU adapter");
        let mut sb = InstanceBuffer::new(&device, "test_slab");

        let a = sb.allocate(4, &device, &queue).unwrap();
        sb.write(a, &[inst(7); 4], &queue);
        let live = read_back(&device, &queue, &sb, a, 4);
        assert!(live.iter().all(|i| i.index_count == 7));

        // Freed range must read back zeroed: a zeroed instance has
        // index_count == 0, which is exactly what the cull shader skips.
        sb.free(a, 4, &queue);
        let dead = read_back(&device, &queue, &sb, a, 4);
        assert!(dead.iter().all(|i| i.index_count == 0));
    }

    #[test]
    #[ignore = "needs a GPU adapter"]
    fn slab_reset_packed() {
        let (device, queue) = test_device("slab reset packed").expect("a GPU adapter");
        let mut sb = InstanceBuffer::new(&device, "test_slab");

        let a = sb.allocate(10, &device, &queue).unwrap();
        let _b = sb.allocate(10, &device, &queue).unwrap();
        sb.free(a, 10, &queue);
        assert_eq!(sb.hole_count(), 10);

        let packed = vec![inst(9); 10];
        let n = sb.reset_packed(&packed, &device, &queue);
        assert_eq!(n, 10);
        assert_eq!(sb.dispatch_count(), 10);
        assert_eq!(sb.live_count(), 10);
        assert_eq!(sb.hole_count(), 0);

        let back = read_back(&device, &queue, &sb, 0, 10);
        assert!(back.iter().all(|i| i.index_count == 9));
    }

    /// Reference port of the bucket routing in `gpu_cull.wgsl` (`cull_main`),
    /// for one set of toggle states. `None` = the instance is suppressed and
    /// emits no draw. Kept deliberately verbatim so the oracle test below
    /// catches any drift between the shader and `potential_buckets`.
    fn shader_route(
        flags: u32,
        alpha_cutoff: f32,
        decal_bias: bool,
        show_markers: bool,
        show_fx: bool,
    ) -> Option<usize> {
        if flags & INSTANCE_FLAGS_WATER != 0 {
            return None;
        }
        if flags & INSTANCE_FLAGS_MARKER != 0 && !show_markers {
            return None;
        }
        if flags & INSTANCE_FLAGS_HIDDEN_FX != 0 && !show_fx {
            return None;
        }
        let is_blend = flags & INSTANCE_FLAGS_BLEND != 0;
        let is_two_sided = flags & INSTANCE_FLAGS_TWO_SIDED != 0;
        let is_decal = flags & INSTANCE_FLAGS_DECAL != 0 && decal_bias;
        let is_multiply = flags & INSTANCE_FLAGS_MULTIPLY != 0;
        let is_alphatest = !is_blend && alpha_cutoff > 0.0;
        let mut bucket = if is_blend {
            if is_multiply {
                10
            } else if is_decal {
                8
            } else {
                6
            }
        } else if is_decal {
            4
        } else if is_alphatest {
            2
        } else {
            0
        };
        if is_two_sided {
            bucket += 1;
        }
        Some(bucket)
    }

    /// `potential_buckets` must be a superset of the shader's routing for every
    /// material-flag / alpha-cutoff / toggle combination — that superset is the
    /// guaranteed bound the per-bucket buffers are sized against.
    #[test]
    fn potential_buckets_mirrors_shader() {
        // bits 0..8 cover BLEND|TWO_SIDED|MARKER|EFFECT|DECAL|WATER|MULTIPLY|HIDDEN_FX (values 0..=255).
        for flags in 0u32..256 {
            for &alpha_cutoff in &[0.0_f32, 0.5] {
                let superset: Vec<usize> = potential_buckets(flags, alpha_cutoff).collect();
                assert!(
                    superset.iter().all(|&b| b < DRAW_BUCKET_COUNT),
                    "flags={flags:#x}: bucket out of range in {superset:?}"
                );
                for decal_bias in [false, true] {
                    for (show_markers, show_fx) in
                        [(false, false), (true, false), (false, true), (true, true)]
                    {
                        if let Some(b) =
                            shader_route(flags, alpha_cutoff, decal_bias, show_markers, show_fx)
                        {
                            assert!(
                                superset.contains(&b),
                                "flags={flags:#x} cutoff={alpha_cutoff} decal_bias={decal_bias} routed \
                                 to bucket {b}, not in potential set {superset:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "needs a GPU adapter"]
    fn indirect_resize_grows_only_changed_buckets() {
        let (device, _queue) =
            test_device("indirect resize grows only changed buckets").expect("a GPU adapter");
        let mut indirect = IndirectDrawBuffers::new(&device, "test");
        let boot: Vec<u32> = (0..DRAW_BUCKET_COUNT)
            .map(|b| indirect.capacity(b))
            .collect();

        // Zero potential keeps every bucket at its floor: nothing grows.
        assert!(!indirect.resize(&device, [0; DRAW_BUCKET_COUNT], "test"));
        for (b, &expected) in boot.iter().enumerate() {
            assert_eq!(indirect.capacity(b), expected);
        }

        // A large potential in one bucket grows only that buffer, to at least
        // the potential (the guaranteed bound the buffer must cover).
        let grown = BUCKET_DECAL_BLEND_CULLED;
        let mut potentials = [0u32; DRAW_BUCKET_COUNT];
        potentials[grown] = 100_000;
        let before: Vec<u64> = indirect
            .draw_buffers
            .iter()
            .map(wgpu::Buffer::size)
            .collect();
        assert!(indirect.resize(&device, potentials, "test"));
        assert!(indirect.capacity(grown) >= potentials[grown]);
        for (b, &old_size) in before.iter().enumerate() {
            let changed = indirect.draw_buffers[b].size() != old_size;
            assert_eq!(changed, b == grown, "bucket {b} reallocation mismatch");
        }

        // Lower potential never shrinks (grow-only high-water mark).
        let high = indirect.capacity(grown);
        assert!(!indirect.resize(&device, [0; DRAW_BUCKET_COUNT], "test"));
        assert_eq!(indirect.capacity(grown), high);
    }

    #[test]
    fn potential_buckets_of_a_plain_opaque_instance_is_the_opaque_bucket() {
        let buckets: Vec<usize> = potential_buckets(0, 0.0).collect();
        assert_eq!(buckets, vec![BUCKET_OPAQUE_CULLED]);
    }

    #[test]
    fn potential_buckets_two_sided_moves_to_the_twosided_variant() {
        let buckets: Vec<usize> = potential_buckets(INSTANCE_FLAGS_TWO_SIDED, 0.5).collect();
        assert_eq!(buckets, vec![BUCKET_ALPHATEST_TWOSIDED]);
    }

    #[test]
    fn potential_buckets_of_a_decal_covers_both_decal_bias_states() {
        let opaque: Vec<usize> = potential_buckets(INSTANCE_FLAGS_DECAL, 0.0).collect();
        assert_eq!(
            opaque,
            vec![BUCKET_OPAQUE_CULLED, BUCKET_DECAL_ALPHATEST_CULLED]
        );
        let blend: Vec<usize> = potential_buckets(
            INSTANCE_FLAGS_DECAL | INSTANCE_FLAGS_BLEND | INSTANCE_FLAGS_TWO_SIDED,
            0.5,
        )
        .collect();
        assert_eq!(
            blend,
            vec![BUCKET_BLEND_TWOSIDED, BUCKET_DECAL_BLEND_TWOSIDED]
        );
    }

    #[test]
    fn shader_oracle_routes_multiply_to_the_multiply_buckets() {
        let multiply = INSTANCE_FLAGS_BLEND | INSTANCE_FLAGS_MULTIPLY;
        for decal_bias in [false, true] {
            assert_eq!(
                shader_route(
                    multiply | INSTANCE_FLAGS_TWO_SIDED,
                    0.0,
                    decal_bias,
                    false,
                    false
                ),
                Some(BUCKET_MULTIPLY_TWOSIDED)
            );
            assert_eq!(
                shader_route(
                    multiply | INSTANCE_FLAGS_DECAL,
                    0.0,
                    decal_bias,
                    false,
                    false
                ),
                Some(BUCKET_MULTIPLY_CULLED)
            );
        }
    }

    #[test]
    fn potential_buckets_routes_multiply_once_whatever_the_decal_flag() {
        let multiply = INSTANCE_FLAGS_BLEND | INSTANCE_FLAGS_MULTIPLY;
        let plain: Vec<usize> =
            potential_buckets(multiply | INSTANCE_FLAGS_TWO_SIDED, 0.0).collect();
        assert_eq!(plain, vec![BUCKET_MULTIPLY_TWOSIDED]);
        let decal: Vec<usize> = potential_buckets(multiply | INSTANCE_FLAGS_DECAL, 0.0).collect();
        assert_eq!(decal, vec![BUCKET_MULTIPLY_CULLED]);
    }

    #[test]
    fn potential_buckets_ignores_alpha_cutoff_for_blended_instances() {
        let buckets: Vec<usize> = potential_buckets(INSTANCE_FLAGS_BLEND, 0.9).collect();
        assert_eq!(buckets, vec![BUCKET_BLEND_CULLED]);
    }

    #[test]
    fn potential_buckets_over_counts_markers_instead_of_dropping_them() {
        let plain: Vec<usize> = potential_buckets(0, 0.0).collect();
        let marked: Vec<usize> = potential_buckets(INSTANCE_FLAGS_MARKER, 0.0).collect();
        assert_eq!(plain, marked);
    }

    #[test]
    fn potential_buckets_over_counts_hidden_fx_instead_of_dropping_them() {
        let plain: Vec<usize> = potential_buckets(0, 0.0).collect();
        let hidden: Vec<usize> = potential_buckets(INSTANCE_FLAGS_HIDDEN_FX, 0.0).collect();
        assert_eq!(plain, hidden);
    }

    #[test]
    fn target_capacity_adds_half_again_as_headroom() {
        assert_eq!(target_capacity(10_000, 0, u32::MAX), 15_000);
    }

    #[test]
    fn target_capacity_never_drops_below_the_floor() {
        assert_eq!(target_capacity(0, 4096, u32::MAX), 4096);
        assert_eq!(target_capacity(100, 4096, u32::MAX), 4096);
    }

    #[test]
    fn target_capacity_is_clamped_to_the_device_maximum() {
        assert_eq!(target_capacity(1_000_000, 4096, 50_000), 50_000);
        assert_eq!(target_capacity(0, 4096, 1000), 1000);
    }

    #[test]
    fn target_capacity_does_not_overflow_for_a_huge_potential() {
        assert_eq!(target_capacity(u32::MAX, 0, u32::MAX), u32::MAX);
    }

    #[test]
    fn every_overlay_bucket_is_either_a_decal_or_translucent_bucket_exactly_once() {
        let mut overlay: Vec<usize> = DECAL_BUCKETS
            .iter()
            .chain(TRANSLUCENT_BUCKETS.iter())
            .copied()
            .collect();
        overlay.sort_unstable();
        assert_eq!(
            overlay,
            (FIRST_AFTER_TERRAIN_BUCKET..DRAW_BUCKET_COUNT).collect::<Vec<_>>()
        );
    }

    #[test]
    fn blended_decals_draw_with_the_decals_before_the_water() {
        assert!(DECAL_BUCKETS.contains(&BUCKET_DECAL_BLEND_CULLED));
        assert!(DECAL_BUCKETS.contains(&BUCKET_DECAL_BLEND_TWOSIDED));
        assert!(!TRANSLUCENT_BUCKETS.contains(&BUCKET_DECAL_BLEND_CULLED));
    }

    #[test]
    fn bucket_floors_are_4096_before_terrain_and_256_for_the_overlay_buckets() {
        assert_eq!(IndirectDrawBuffers::bucket_floor(0), 4096);
        assert_eq!(
            IndirectDrawBuffers::bucket_floor(FIRST_AFTER_TERRAIN_BUCKET - 1),
            4096
        );
        assert_eq!(
            IndirectDrawBuffers::bucket_floor(FIRST_AFTER_TERRAIN_BUCKET),
            256
        );
        assert_eq!(
            IndirectDrawBuffers::bucket_floor(DRAW_BUCKET_COUNT - 1),
            256
        );
    }
}
