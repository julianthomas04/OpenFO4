use glam::UVec4;

use super::views::{GpuViewCull, MAX_SHADOW_VIEWS, ShadowViewSet};
use crate::rendering::{
    device::STORAGE_BUFFERS_PER_STAGE,
    gpu_cull::{storage_entry, uniform_entry},
    gpu_driven::{
        BUCKET_ALPHATEST_CULLED, BUCKET_ALPHATEST_TWOSIDED, BUCKET_OPAQUE_CULLED,
        BUCKET_OPAQUE_TWOSIDED, DRAW_BUCKET_COUNT, DRAW_STRIDE, max_draws_per_buffer,
        target_capacity,
    },
    shaders,
};
use crate::utils::env_flag;

/// Object instances, terrain instances, one draw buffer per caster kind and
/// the counts, shared by both entry points' layout.
const STORAGE_BUFFERS: u32 = 2 + CASTER_KINDS as u32 + 1;
const _: () = assert!(STORAGE_BUFFERS <= STORAGE_BUFFERS_PER_STAGE);

const OBJECT_WORKGROUP_SIZE: u32 = 256;
const TERRAIN_WORKGROUP_SIZE: u32 = 64;
const COUNT_STRIDE: u64 = std::mem::size_of::<u32>() as u64;

pub const CASTER_KINDS: usize = 3;

/// `shadow_cull.wgsl` hardcodes `MAX_SHADOW_VIEWS` and `COUNTER_COUNT`
/// (`CASTER_KINDS * MAX_SHADOW_VIEWS`); there is no constant injection into
/// WGSL, so these fail the build when the Rust side drifts from them.
const WGSL_MAX_SHADOW_VIEWS: usize = 16;
const WGSL_COUNTER_COUNT: usize = 48;
const _: () = assert!(MAX_SHADOW_VIEWS == WGSL_MAX_SHADOW_VIEWS);
const _: () = assert!(CASTER_KINDS * MAX_SHADOW_VIEWS == WGSL_COUNTER_COUNT);

/// Discriminants are mirrored by the `KIND_*` constants in `shadow_cull.wgsl`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CasterKind {
    Opaque = 0,
    AlphaTest = 1,
    Terrain = 2,
}

impl CasterKind {
    pub const ALL: [CasterKind; CASTER_KINDS] = [
        CasterKind::Opaque,
        CasterKind::AlphaTest,
        CasterKind::Terrain,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    /// Per-cascade floor so sparse kinds are not reallocated for every
    /// streamed instance.
    fn cascade_capacity_floor(self) -> u32 {
        match self {
            Self::Opaque | Self::AlphaTest => 4096,
            Self::Terrain => 512,
        }
    }

    /// Per-lamp-view starting capacity: a lamp's range sphere reaches a small
    /// part of the scene. Grows when a view overflows.
    fn lamp_capacity_floor(self) -> u32 {
        match self {
            Self::Opaque => 2048,
            Self::AlphaTest => 1024,
            Self::Terrain => 64,
        }
    }
}

/// Upper bound of the draws each caster kind can append per cascade. The
/// shadow cull only accepts instances the main cull routes to the opaque and
/// alpha-test buckets, so their resident potentials bound it.
pub fn caster_potentials(
    bucket_potential: [u32; DRAW_BUCKET_COUNT],
    terrain_instances: u32,
) -> [u32; CASTER_KINDS] {
    [
        bucket_potential[BUCKET_OPAQUE_CULLED] + bucket_potential[BUCKET_OPAQUE_TWOSIDED],
        bucket_potential[BUCKET_ALPHATEST_CULLED] + bucket_potential[BUCKET_ALPHATEST_TWOSIDED],
        terrain_instances,
    ]
}

/// Mirrors `ShadowCullUniforms` in `shadow_cull.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ShadowCullUniforms {
    views: [GpuViewCull; MAX_SHADOW_VIEWS],
    object_count: u32,
    terrain_count: u32,
    active_count: u32,
    cascade_slots: u32,
    cascade_capacities: UVec4,
    lamp_capacities: UVec4,
    /// x: `DEBUG_*` bits.
    debug: UVec4,
    /// The slots of the views in use, packed four per element.
    active_slots: [UVec4; MAX_SHADOW_VIEWS / 4],
}

const _: () = assert!(std::mem::size_of::<ShadowCullUniforms>() == 2688);

/// Mirror the `DEBUG_*` constants in `shadow_cull.wgsl`.
const DEBUG_ONE_TRIANGLE: u32 = 1;
const DEBUG_SKIP_ALPHA: u32 = 2;

/// Diagnostic switches for isolating shadow cost, read once at startup.
fn debug_flags() -> u32 {
    let toggles = [
        ("OPENFO4_SHADOW_DEBUG_ONE_TRIANGLE", DEBUG_ONE_TRIANGLE),
        ("OPENFO4_SHADOW_DEBUG_SKIP_ALPHA", DEBUG_SKIP_ALPHA),
    ];
    toggles.iter().fold(0, |flags, (name, bit)| {
        if !env_flag(name, false) {
            return flags;
        }
        tracing::warn!("{name}=1: shadow debug toggle active, shadows are intentionally incorrect");
        flags | bit
    })
}

/// Per-kind draw capacities of one view class.
type Capacities = [u32; CASTER_KINDS];

fn capacities_uniform(capacities: Capacities) -> UVec4 {
    UVec4::new(capacities[0], capacities[1], capacities[2], 0)
}

/// Index of the first draw of view `slot` in a kind's indirect buffer: the
/// cascade regions come first, then the lamp regions.
fn first_draw(cascade_slots: u32, cascade_capacity: u32, lamp_capacity: u32, slot: usize) -> u64 {
    if slot < cascade_slots as usize {
        slot as u64 * u64::from(cascade_capacity)
    } else {
        let lamp_slot = (slot - cascade_slots as usize) as u64;
        u64::from(cascade_slots) * u64::from(cascade_capacity)
            + lamp_slot * u64::from(lamp_capacity)
    }
}

/// How many views of each class a cull gives draw regions to. The cascades take the first view
/// slots and the lamp views the ones after them, so together they fit the `MAX_SHADOW_VIEWS` of
/// the cull's uniforms and counts, and stop short of the view slot of the ortho map.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct CullSlots {
    cascades: u32,
    lamps: u32,
}

impl CullSlots {
    /// Panics when the views would not fit the `MAX_SHADOW_VIEWS` slots.
    pub const fn new(cascades: u32, lamps: u32) -> Self {
        assert!(
            cascades as usize + lamps as usize <= MAX_SHADOW_VIEWS,
            "the cascade and lamp views need more view slots than a cull has"
        );
        Self { cascades, lamps }
    }

    pub const fn cascades(self) -> u32 {
        self.cascades
    }
}

/// One indirect buffer per caster kind, split into one region per view slot:
/// `slots.cascades` regions of the cascade capacity, then `slots.lamps`
/// regions of the lamp capacity. The counts are kind-major with a
/// `MAX_SHADOW_VIEWS` stride.
pub struct ShadowDraws {
    buffers: [wgpu::Buffer; CASTER_KINDS],
    pub counts: wgpu::Buffer,
    cascade_capacities: Capacities,
    lamp_capacities: Capacities,
    slots: CullSlots,
}

impl ShadowDraws {
    fn new(device: &wgpu::Device, slots: CullSlots) -> Self {
        let lamp_capacities = CasterKind::ALL.map(CasterKind::lamp_capacity_floor);
        let cascade_capacities = CasterKind::ALL.map(|kind| {
            target_capacity(
                0,
                kind.cascade_capacity_floor(),
                Self::max_per_cascade(device, slots, lamp_capacities[kind.index()]),
            )
        });
        let buffers = CasterKind::ALL.map(|kind| {
            let i = kind.index();
            Self::create_buffer(
                device,
                kind,
                slots,
                cascade_capacities[i],
                lamp_capacities[i],
            )
        });
        let counts = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow_draw_counts"),
            size: (CASTER_KINDS * MAX_SHADOW_VIEWS) as u64 * COUNT_STRIDE,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            buffers,
            counts,
            cascade_capacities,
            lamp_capacities,
            slots,
        }
    }

    /// What is left of the per-buffer draw limit after the lamp regions,
    /// shared evenly by the cascades.
    fn max_per_cascade(device: &wgpu::Device, slots: CullSlots, lamp_capacity: u32) -> u32 {
        let lamp_draws = slots.lamps * lamp_capacity;
        max_draws_per_buffer(device).saturating_sub(lamp_draws) / slots.cascades.max(1)
    }

    fn create_buffer(
        device: &wgpu::Device,
        kind: CasterKind,
        slots: CullSlots,
        cascade_capacity: u32,
        lamp_capacity: u32,
    ) -> wgpu::Buffer {
        let draws = u64::from(slots.cascades) * u64::from(cascade_capacity)
            + u64::from(slots.lamps) * u64::from(lamp_capacity);
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("shadow_indirect_{kind:?}")),
            size: (draws * DRAW_STRIDE).max(DRAW_STRIDE),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Grows the cascade regions of the kinds whose potential outgrew them;
    /// never shrinks. Returns true when any buffer was reallocated.
    fn resize(&mut self, device: &wgpu::Device, potentials: [u32; CASTER_KINDS]) -> bool {
        let mut grew = false;
        for kind in CasterKind::ALL {
            let i = kind.index();
            let max_per_cascade =
                Self::max_per_cascade(device, self.slots, self.lamp_capacities[i]);
            let target = target_capacity(
                potentials[i],
                kind.cascade_capacity_floor(),
                max_per_cascade,
            );
            if target > self.cascade_capacities[i] {
                self.cascade_capacities[i] = target;
                self.reallocate(device, kind);
                grew = true;
            }
        }
        grew
    }

    /// Grows the lamp regions of the kinds whose `needed` draw count outgrew
    /// them; never shrinks. Returns true when any buffer was reallocated. A
    /// cull without lamp regions has none to grow.
    fn grow_lamp_regions(&mut self, device: &wgpu::Device, needed: Capacities) -> bool {
        if self.slots.lamps == 0 {
            return false;
        }
        let mut grew = false;
        for kind in CasterKind::ALL {
            let i = kind.index();
            let cascade_draws =
                u64::from(self.slots.cascades) * u64::from(self.cascade_capacities[i]);
            let max_per_lamp = u64::from(max_draws_per_buffer(device))
                .saturating_sub(cascade_draws)
                / u64::from(self.slots.lamps);
            let target = target_capacity(needed[i], self.lamp_capacities[i], max_per_lamp as u32);
            if target > self.lamp_capacities[i] {
                self.lamp_capacities[i] = target;
                self.reallocate(device, kind);
                grew = true;
            }
        }
        grew
    }

    pub fn cascade_capacities(&self) -> Capacities {
        self.cascade_capacities
    }

    pub fn lamp_capacities(&self) -> Capacities {
        self.lamp_capacities
    }

    fn reallocate(&mut self, device: &wgpu::Device, kind: CasterKind) {
        let i = kind.index();
        self.buffers[i] = Self::create_buffer(
            device,
            kind,
            self.slots,
            self.cascade_capacities[i],
            self.lamp_capacities[i],
        );
    }

    pub fn buffer(&self, kind: CasterKind) -> &wgpu::Buffer {
        &self.buffers[kind.index()]
    }

    fn is_cascade_slot(&self, slot: usize) -> bool {
        slot < self.slots.cascades as usize
    }

    pub fn capacity(&self, kind: CasterKind, slot: usize) -> u32 {
        if self.is_cascade_slot(slot) {
            self.cascade_capacities[kind.index()]
        } else {
            self.lamp_capacities[kind.index()]
        }
    }

    /// Mirrors `region_base` in `shadow_cull.wgsl`.
    pub fn draw_offset(&self, kind: CasterKind, slot: usize) -> u64 {
        let i = kind.index();
        first_draw(
            self.slots.cascades,
            self.cascade_capacities[i],
            self.lamp_capacities[i],
            slot,
        ) * DRAW_STRIDE
    }

    pub fn count_offset(kind: CasterKind, slot: usize) -> u64 {
        (kind.index() * MAX_SHADOW_VIEWS + slot) as u64 * COUNT_STRIDE
    }
}

/// The compute pipelines of the cull, the layout of what they bind and the
/// diagnostic switches they run with. Culls with draw regions of their own
/// share one.
#[derive(Clone)]
struct CullPipelines {
    objects: wgpu::ComputePipeline,
    terrain: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    debug_flags: u32,
}

impl CullPipelines {
    fn new(device: &wgpu::Device) -> Self {
        let shader = shaders::SHADOW_CULL.compile(device);

        let mut entries: Vec<_> = (0..2).map(|binding| storage_entry(binding, true)).collect();
        entries.extend((2..STORAGE_BUFFERS).map(|binding| storage_entry(binding, false)));
        entries.push(uniform_entry(STORAGE_BUFFERS));
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow_cull_bgl"),
            entries: &entries,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow_cull_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let create_pipeline = |entry_point: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry_point),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry_point),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };

        Self {
            objects: create_pipeline("shadow_cull_objects"),
            terrain: create_pipeline("shadow_cull_terrain"),
            bind_group_layout,
            debug_flags: debug_flags(),
        }
    }
}

/// Per-view caster culling into `ShadowDraws`: one thread per object or
/// terrain instance, tested against every view of the frame; no frustum or
/// occlusion test against the camera.
pub struct ShadowCull {
    pipelines: CullPipelines,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    draws: ShadowDraws,
}

impl ShadowCull {
    pub fn new(
        device: &wgpu::Device,
        slots: CullSlots,
        object_instances: &wgpu::Buffer,
        terrain_instances: &wgpu::Buffer,
    ) -> Self {
        Self::with_pipelines(
            CullPipelines::new(device),
            device,
            slots,
            object_instances,
            terrain_instances,
        )
    }

    /// A cull that runs this one's pipelines over draw regions of its own,
    /// for views that must not take slots and regions from the ones this
    /// cull serves.
    #[must_use]
    pub fn sibling(
        &self,
        device: &wgpu::Device,
        slots: CullSlots,
        object_instances: &wgpu::Buffer,
        terrain_instances: &wgpu::Buffer,
    ) -> Self {
        Self::with_pipelines(
            self.pipelines.clone(),
            device,
            slots,
            object_instances,
            terrain_instances,
        )
    }

    fn with_pipelines(
        pipelines: CullPipelines,
        device: &wgpu::Device,
        slots: CullSlots,
        object_instances: &wgpu::Buffer,
        terrain_instances: &wgpu::Buffer,
    ) -> Self {
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shadow_cull_uniforms"),
            size: std::mem::size_of::<ShadowCullUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let draws = ShadowDraws::new(device, slots);
        let bind_group = Self::create_bind_group(
            device,
            &pipelines.bind_group_layout,
            object_instances,
            terrain_instances,
            &draws,
            &uniform_buffer,
        );
        Self {
            pipelines,
            uniform_buffer,
            bind_group,
            draws,
        }
    }

    fn create_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        object_instances: &wgpu::Buffer,
        terrain_instances: &wgpu::Buffer,
        draws: &ShadowDraws,
        uniform_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        let resources = [object_instances, terrain_instances]
            .into_iter()
            .chain(draws.buffers.iter())
            .chain([&draws.counts, uniform_buffer]);
        let entries: Vec<_> = resources
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow_cull_bg"),
            layout,
            entries: &entries,
        })
    }

    pub fn draws(&self) -> &ShadowDraws {
        &self.draws
    }

    /// Returns true when a draw buffer was reallocated; the caller then rebinds.
    pub fn resize_draws(&mut self, device: &wgpu::Device, potentials: [u32; CASTER_KINDS]) -> bool {
        self.draws.resize(device, potentials)
    }

    /// Returns true when a draw buffer was reallocated; the caller then rebinds.
    pub fn grow_lamp_regions(
        &mut self,
        device: &wgpu::Device,
        needed: [u32; CASTER_KINDS],
    ) -> bool {
        self.draws.grow_lamp_regions(device, needed)
    }

    pub fn rebind(
        &mut self,
        device: &wgpu::Device,
        object_instances: &wgpu::Buffer,
        terrain_instances: &wgpu::Buffer,
    ) {
        self.bind_group = Self::create_bind_group(
            device,
            &self.pipelines.bind_group_layout,
            object_instances,
            terrain_instances,
            &self.draws,
            &self.uniform_buffer,
        );
    }

    /// Clears the counts unconditionally so a skipped dispatch never replays
    /// last frame's draws. A cull dispatches at most once per submission: its
    /// uniforms are written through the queue, ahead of every pass.
    pub fn dispatch(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        views: &ShadowViewSet,
        object_count: u32,
        terrain_count: u32,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        encoder.clear_buffer(&self.draws.counts, 0, None);
        let uniforms = self.uniforms(views, object_count, terrain_count);
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("shadow_cull"),
            timestamp_writes,
        });
        pass.set_bind_group(0, &self.bind_group, &[]);
        if object_count > 0 {
            pass.set_pipeline(&self.pipelines.objects);
            pass.dispatch_workgroups(object_count.div_ceil(OBJECT_WORKGROUP_SIZE), 1, 1);
        }
        if terrain_count > 0 {
            pass.set_pipeline(&self.pipelines.terrain);
            pass.dispatch_workgroups(terrain_count.div_ceil(TERRAIN_WORKGROUP_SIZE), 1, 1);
        }
    }

    fn uniforms(
        &self,
        views: &ShadowViewSet,
        object_count: u32,
        terrain_count: u32,
    ) -> ShadowCullUniforms {
        let slots = self.draws.slots;
        debug_assert!(
            views.cascades.len() <= slots.cascades as usize
                && views.lamps.len() <= slots.lamps as usize,
            "{} cascade and {} lamp views for the draw regions {slots:?}",
            views.cascades.len(),
            views.lamps.len()
        );
        let cascade_slots = slots.cascades as usize;
        let mut culls = [GpuViewCull::UNUSED; MAX_SHADOW_VIEWS];
        let mut active = [0u32; MAX_SHADOW_VIEWS];
        let mut active_count = 0;
        for (slot, view) in views.slotted(cascade_slots) {
            culls[slot] = GpuViewCull::from(view);
            active[active_count] = slot as u32;
            active_count += 1;
        }
        ShadowCullUniforms {
            views: culls,
            object_count,
            terrain_count,
            active_count: active_count as u32,
            cascade_slots: cascade_slots as u32,
            cascade_capacities: capacities_uniform(self.draws.cascade_capacities),
            lamp_capacities: capacities_uniform(self.draws.lamp_capacities),
            debug: UVec4::new(self.pipelines.debug_flags, 0, 0, 0),
            active_slots: bytemuck::cast(active),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::gpu_driven::{
        INSTANCE_FLAGS_BLEND, INSTANCE_FLAGS_DECAL, INSTANCE_FLAGS_EFFECT,
        INSTANCE_FLAGS_HIDDEN_FX, INSTANCE_FLAGS_MARKER, INSTANCE_FLAGS_WATER,
    };
    use crate::rendering::lighting::frame_uniform::MAX_CASCADES;
    use crate::rendering::shadows::test_rig::rig;
    use crate::rendering::shadows::views::MAX_LAMP_VIEWS_PER_FRAME;

    const SHADOW_CULL_WGSL: &str = include_str!("../shaders/shadow_cull.wgsl");

    fn wgsl_const(name: &str) -> &'static str {
        let prefix = format!("const {name} = ");
        SHADOW_CULL_WGSL
            .lines()
            .find_map(|line| line.strip_prefix(prefix.as_str()))
            .unwrap_or_else(|| panic!("{name} declared"))
            .trim_end_matches(';')
    }

    #[test]
    fn instance_flag_constants_mirror_the_rust_values() {
        for (name, value) in [
            ("INSTANCE_FLAGS_BLEND", INSTANCE_FLAGS_BLEND),
            ("INSTANCE_FLAGS_MARKER", INSTANCE_FLAGS_MARKER),
            ("INSTANCE_FLAGS_EFFECT", INSTANCE_FLAGS_EFFECT),
            ("INSTANCE_FLAGS_DECAL", INSTANCE_FLAGS_DECAL),
            ("INSTANCE_FLAGS_WATER", INSTANCE_FLAGS_WATER),
            ("INSTANCE_FLAGS_HIDDEN_FX", INSTANCE_FLAGS_HIDDEN_FX),
        ] {
            assert_eq!(wgsl_const(name), format!("{value}u"), "{name}");
        }
    }

    #[test]
    fn effect_instances_are_excluded_from_shadow_casting() {
        assert!(wgsl_const("NON_CASTER_FLAGS").contains("INSTANCE_FLAGS_EFFECT"));
    }

    #[test]
    fn hidden_fx_instances_are_excluded_from_shadow_casting() {
        assert!(wgsl_const("NON_CASTER_FLAGS").contains("INSTANCE_FLAGS_HIDDEN_FX"));
    }

    #[test]
    fn cascade_regions_come_first_then_the_lamp_regions_of_their_own_capacity() {
        // (cascade_slots, cascade_capacity, lamp_capacity, slot, expected first draw)
        let cases = [
            (3, 100, 20, 0, 0),
            (3, 100, 20, 2, 200),
            (3, 100, 20, 3, 300),
            (3, 100, 20, 5, 340),
            (3, 100, 64, 3, 300),
            (3, 100, 64, 4, 364),
        ];
        for (slots, cascade_capacity, lamp_capacity, slot, expected) in cases {
            assert_eq!(
                first_draw(slots, cascade_capacity, lamp_capacity, slot),
                expected,
                "slot {slot}, lamp capacity {lamp_capacity}"
            );
        }
    }

    #[test]
    fn a_cull_can_serve_every_cascade_and_lamp_view_there_is_a_slot_for() {
        let slots = CullSlots::new(MAX_CASCADES as u32, MAX_LAMP_VIEWS_PER_FRAME as u32);

        assert_eq!(slots.cascades as usize, MAX_CASCADES);
        assert_eq!(slots.lamps as usize, MAX_LAMP_VIEWS_PER_FRAME);
        assert_eq!(MAX_CASCADES + MAX_LAMP_VIEWS_PER_FRAME, MAX_SHADOW_VIEWS);
    }

    #[test]
    #[should_panic(expected = "view slots")]
    fn a_cull_cannot_serve_a_view_that_has_no_slot() {
        let _ = CullSlots::new(MAX_CASCADES as u32, MAX_LAMP_VIEWS_PER_FRAME as u32 + 1);
    }

    #[test]
    fn a_sibling_shares_the_pipelines_and_holds_only_the_regions_it_was_asked_for() {
        let Some(rig) = rig("a_sibling_shares_the_pipelines") else {
            return;
        };
        let cull = &rig.renderer.cull;
        let sibling = &rig.renderer.ortho_cull;

        assert_eq!(sibling.pipelines.objects, cull.pipelines.objects);
        assert_eq!(sibling.pipelines.terrain, cull.pipelines.terrain);
        assert_ne!(sibling.draws().counts, cull.draws().counts);
        for kind in CasterKind::ALL {
            let i = kind.index();
            let cascade_draws = u64::from(cull.draws().cascade_capacities()[i]);
            let lamp_draws =
                MAX_LAMP_VIEWS_PER_FRAME as u64 * u64::from(cull.draws().lamp_capacities()[i]);
            assert_eq!(
                cull.draws().buffer(kind).size(),
                (cascade_draws + lamp_draws) * DRAW_STRIDE,
                "{kind:?}: the sun's cascade and the lamp views"
            );
            assert_eq!(
                sibling.draws().buffer(kind).size(),
                u64::from(sibling.draws().cascade_capacities()[i]) * DRAW_STRIDE,
                "{kind:?}: the one view of the map"
            );
        }
    }

    #[test]
    fn a_cull_without_lamp_regions_has_none_to_grow() {
        let Some(mut rig) = rig("a_cull_without_lamp_regions_has_none_to_grow") else {
            return;
        };
        let before = rig
            .renderer
            .ortho_cull
            .draws()
            .buffer(CasterKind::Opaque)
            .size();

        let grew = rig
            .renderer
            .ortho_cull
            .grow_lamp_regions(&rig.device, [1_000_000; CASTER_KINDS]);

        assert!(!grew);
        let after = rig
            .renderer
            .ortho_cull
            .draws()
            .buffer(CasterKind::Opaque)
            .size();
        assert_eq!(after, before);
    }

    #[test]
    fn counts_are_kind_major_with_the_view_stride() {
        assert_eq!(ShadowDraws::count_offset(CasterKind::Opaque, 0), 0);
        assert_eq!(ShadowDraws::count_offset(CasterKind::Opaque, 5), 5 * 4);
        assert_eq!(
            ShadowDraws::count_offset(CasterKind::AlphaTest, 0),
            (MAX_SHADOW_VIEWS * 4) as u64
        );
        assert_eq!(
            ShadowDraws::count_offset(CasterKind::Terrain, 2),
            ((2 * MAX_SHADOW_VIEWS + 2) * 4) as u64
        );
    }

    #[test]
    fn caster_potentials_sum_the_culled_and_two_sided_buckets_of_each_kind() {
        let mut buckets = [0; DRAW_BUCKET_COUNT];
        buckets[BUCKET_OPAQUE_CULLED] = 1;
        buckets[BUCKET_OPAQUE_TWOSIDED] = 10;
        buckets[BUCKET_ALPHATEST_CULLED] = 100;
        buckets[BUCKET_ALPHATEST_TWOSIDED] = 1000;
        assert_eq!(caster_potentials(buckets, 7), [11, 1100, 7]);
    }

    #[test]
    fn caster_potentials_ignore_buckets_the_shadow_cull_never_accepts() {
        let mut buckets = [0; DRAW_BUCKET_COUNT];
        let counted = [
            BUCKET_OPAQUE_CULLED,
            BUCKET_OPAQUE_TWOSIDED,
            BUCKET_ALPHATEST_CULLED,
            BUCKET_ALPHATEST_TWOSIDED,
        ];
        for (bucket, slot) in buckets.iter_mut().enumerate() {
            if !counted.contains(&bucket) {
                *slot = 999;
            }
        }
        assert_eq!(caster_potentials(buckets, 0), [0, 0, 0]);
    }
}
