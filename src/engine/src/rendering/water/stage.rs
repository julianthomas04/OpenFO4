use std::collections::HashMap;
use std::time::{Duration, Instant};

use glam::{Mat4, UVec2, Vec3};
use wgpu::util::DeviceExt;

use super::{
    config::WaterConfig,
    constants::{
        DRAW_CONSTANTS_STRIDE, FarField, GeometryInputs, WaterDrawConstants, WaterEnvironment,
        WaterFrameConstants, WaterMaterial, WaterSetting, draw_constants, frame_constants,
    },
    debug_output::{DebugSelection, DrawPipeline, WaterDebugOutput},
    displacement::{
        DISPLACEMENT_QUAD_DRAW_REGISTER, EngineFrame, QUAD_REGISTER, WaterDisplacement,
    },
    permutation::{self, MaskTag, PassContext, SurfacePasses},
    pipelines::{
        DRAW_GROUP, FRAME_GROUP, FrameBindings, NORMALS_GROUP, SCENE_GROUP, WaterLayouts,
        WaterPipelines,
    },
    placement::{PlacedWater, WaterPlane},
    settings::{CellSurfaceRing, WaterSettings},
    ssr::{
        self, RaySetupKind, SsrCamera, SsrChain,
        constants::{Surroundings, half_extent, ray_setup_frame_constants, world_camera_far},
    },
    textures::{NormalTextures, WaterSurfaceDepth},
    underwater_fog::{UnderwaterFog, load_image_space_dofs, underwater_override_dof},
};
use crate::assets::db::DbClient;
use crate::assets::db::waters::Water;
use crate::assets::types::water_shader::{WaterShaderToken, water_flags};
use crate::assets::world_settings::LoadedGrid;
use crate::bethesda_space::{UNITS_FROM_ENGINE, position_from_engine};
use crate::rendering::{
    camera::Camera,
    frustum::{Aabb, clip_planes},
    gpu_profiler::GpuProfiler,
    lighting::frame_uniform::FrameUniform,
    mega_buffer::MegaBuffer,
    post::depth_of_field::{ImageSpaceDepthOfField, UnderwaterOverride},
    scene_copy::SceneCopy,
    vertex::Vertex,
};
use crate::sim::PlayerBody;
use crate::utils::{env_flag, env_opt, warn_once};
use crate::water_world::height::DEFAULT_WATER_FORM_KEY;
use crate::water_world::{
    CameraWater, LoadedCell, NearPlane, PlacedWaterVolume, Scope, SharedWaterWorld, SurfacePlane,
    VIRTUAL_NEAR_DISTANCE, WaterWorld, camera_water,
};

const WATER_ENV: &str = "OPENFO4_WATER";
const TRACE_ENV: &str = "OPENFO4_WATER_TRACE";
const DISPLACEMENT_QUAD_ENV: &str = "OPENFO4_WATER_DISPLACEMENT_QUAD";
/// Diagnostic override of the SSR far plane (Bethesda units); unset keeps the
/// world camera's.
const SSR_FAR_ENV: &str = "OPENFO4_WATER_SSR_FAR";
/// Water objects that get a displacement quad per frame.
const MAX_DISPLACEMENT_QUADS: usize = 7;
/// Height difference under which two surfaces of one water share a quad.
/// The value only merges coplanar duplicates.
const QUAD_HEIGHT_TOLERANCE: f32 = 0.01;
/// Smallest upward component of a surface normal that counts as horizontal.
const HORIZONTAL_NORMAL_Y: f32 = 0.999;
const TRACE_INTERVAL: Duration = Duration::from_secs(1);
const INITIAL_DRAW_CAPACITY: usize = 64;

/// The WATR records of the database, resolved to shader constants.
struct WaterEntry {
    material: WaterMaterial,
    /// The WATR's Screen Space Reflections flag.
    screen_space_reflections: bool,
    normal_textures: [Option<String>; 3],
    /// Built on first use: it owns the texture loads.
    normals: Option<wgpu::BindGroup>,
    underwater_fog: UnderwaterFog,
    /// The depth of field of the WATR's image space (`INAM`), which gates the underwater
    /// fog; `None` without an image space or without exported depth of field data.
    depth_of_field: Option<ImageSpaceDepthOfField>,
}

#[derive(Default)]
struct WaterTable {
    entries: Vec<WaterEntry>,
    by_form_key: HashMap<String, usize>,
}

impl WaterTable {
    fn from_waters(
        waters: Vec<Water>,
        depth_of_fields: &HashMap<String, ImageSpaceDepthOfField>,
    ) -> Self {
        let mut table = Self::default();
        for water in waters {
            let Some(data) = water.data else {
                warn_once(&format!("water-no-data:{}", water.form_key), || {
                    format!(
                        "WATR {} has no extracted data (database predates water extraction); it will not draw",
                        water.form_key
                    )
                });
                continue;
            };
            let layers = data.noise_layers();
            let depth_of_field = depth_of_fields.get(&water.form_key).copied();
            table
                .by_form_key
                .insert(water.form_key, table.entries.len());
            table.entries.push(WaterEntry {
                material: WaterMaterial::from_watr(&data),
                screen_space_reflections: data.screen_space_reflections,
                normal_textures: layers.map(|layer| layer.texture.clone()),
                normals: None,
                underwater_fog: UnderwaterFog::of_water(&data),
                depth_of_field,
            });
        }
        table
    }

    fn index_of(&self, form_key: &str) -> Option<usize> {
        self.by_form_key.get(form_key).copied()
    }
}

#[derive(Debug, Clone, Copy)]
enum Geometry {
    Mesh {
        index_offset: u32,
        index_count: u32,
        vertex_offset: u32,
    },
    Quad,
}

/// One water surface collected for this frame.
struct Surface {
    world: Mat4,
    bounds: Aabb,
    geometry: Geometry,
    water: usize,
    shader: WaterShaderToken,
    /// A point on the plane the shader measures the camera height against.
    plane_point: Vec3,
    kind: SurfaceKind,
}

/// Where a surface comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfaceKind {
    /// A placed water mesh.
    Placed,
    /// The full-detail surface of a loaded exterior cell.
    Cell,
    /// A distant water quad of the terrain LOD.
    Lod,
}

impl Surface {
    fn is_lod(&self) -> bool {
        self.kind == SurfaceKind::Lod
    }
}

/// What a camera fully under water gives the depth of field effect, from
/// [`WaterStage::underwater_dof`].
pub struct UnderwaterDofInputs<'a> {
    pub image_space: &'a UnderwaterOverride,
    /// The underwater surface depth of this frame, `None` when no surface was drawn.
    pub surface_depth: Option<&'a wgpu::TextureView>,
}

/// What the last frames drew, for `OPENFO4_WATER_TRACE`.
#[derive(Default)]
struct DrawTally {
    placed: usize,
    cell: usize,
    lod: usize,
    quads: usize,
    /// Placed surfaces the frustum test dropped this frame.
    placed_culled: usize,
    /// The placed surface nearest the camera, culled or not.
    nearest_placed: Option<NearestPlaced>,
}

/// The placed water mesh nearest the camera, in Bethesda units.
#[derive(Debug, Clone, Copy, PartialEq)]
struct NearestPlaced {
    distance: f32,
    plane_height: f32,
    culled: bool,
}

/// One draw of the frame: a surface that survived culling, in draw order.
#[derive(Debug, Clone, Copy)]
struct Draw {
    constants_offset: u32,
    water: usize,
    geometry: Geometry,
    passes: SurfacePasses,
    /// The SSR ray setup this surface draws, when it needs one.
    ray_setup: Option<RaySetupKind>,
}

struct DrawConstantsBuffer {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    capacity: usize,
}

impl DrawConstantsBuffer {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, capacity: usize) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("water_draw_constants"),
            size: capacity as u64 * DRAW_CONSTANTS_STRIDE,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("water_draw_constants_group"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(DRAW_CONSTANTS_STRIDE),
                }),
            }],
        });
        Self {
            buffer,
            bind_group,
            capacity,
        }
    }
}

/// The unit quad (`[-0.5, 0.5]` on X and Z) water planes are drawn with.
struct QuadMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
}

impl QuadMesh {
    const INDICES: [u32; 6] = [0, 2, 1, 0, 3, 2];

    fn new(device: &wgpu::Device) -> Self {
        let corner = |x: f32, z: f32| Vertex {
            position: [x, 0.0, z],
            tex_coords: [x + 0.5, z + 0.5],
            normal: [0.0, 1.0, 0.0],
            tangent: [1.0, 0.0, 0.0, 1.0],
        };
        let vertices = [
            corner(-0.5, -0.5),
            corner(0.5, -0.5),
            corner(0.5, 0.5),
            corner(-0.5, 0.5),
        ];
        Self {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("water_quad_vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("water_quad_indices"),
                contents: bytemuck::cast_slice(&Self::INDICES),
                usage: wgpu::BufferUsages::INDEX,
            }),
        }
    }
}

/// The water draw stage between the opaque world and the blend/decal buckets.
///
/// Per frame: [`Self::collect`] gathers the resident water meshes and the
/// plane source's planes, [`Self::prepare`] culls, sorts them back to front
/// and writes their constants, [`Self::encode_prepasses`] writes the refraction mask and tints the opaque
/// scene by the water body (before the scene copy is taken) and
/// [`Self::draw`] shades the surfaces from the scene copy.
pub struct WaterStage {
    enabled: bool,
    settings: WaterSettings,
    waters: WaterTable,
    layouts: WaterLayouts,
    pipelines: WaterPipelines,
    repeat_sampler: wgpu::Sampler,
    normal_textures: NormalTextures,
    frame: FrameBindings,
    displacement: WaterDisplacement,
    /// Whether the displacement quad is drawn over the water near the player.
    displacement_quad: bool,
    ssr_far_override: Option<f32>,
    worldspace_water: Option<usize>,
    draw_constants: DrawConstantsBuffer,
    quad: QuadMesh,
    ssr: SsrChain,
    ssr_camera: Option<SsrCamera>,
    surroundings: Surroundings,
    /// Whether this frame's draws include a surface that needs the SSR chain.
    ssr_active: bool,
    world: SharedWaterWorld,
    /// `uGridsToLoad`: the cells around the camera that draw as full cell surfaces.
    cell_surface_radius: i32,
    config: WaterConfig,
    surfaces: Vec<Surface>,
    draws: Vec<Draw>,
    constants_scratch: Vec<WaterDrawConstants>,
    camera_water: CameraWater,
    camera_eye: Vec3,
    surface_depth: WaterSurfaceDepth,
    /// The depth of field this frame replaces the weather's with, when the camera is
    /// fully under water.
    underwater_dof: Option<UnderwaterOverride>,
    trace: bool,
    last_trace: Instant,
    tally: DrawTally,
    last_frame_constants: WaterFrameConstants,
    /// What the draws show instead of their shading while a water debug view is selected.
    debug: DebugSelection,
}

impl WaterStage {
    /// Loads the WATR table from the database. A table that cannot be read
    /// leaves the stage without water types, which is logged: every water
    /// shape is then skipped.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene_layout: &wgpu::BindGroupLayout,
        asset_root: &str,
        db_path: &str,
        loaded_grid: LoadedGrid,
        config: WaterConfig,
        world: SharedWaterWorld,
    ) -> Self {
        let (waters, depth_of_fields) = DbClient::new(db_path)
            .and_then(|db| {
                let waters = db.get_waters()?;
                let depth_of_fields = load_image_space_dofs(&db, &waters);
                Ok((waters, depth_of_fields))
            })
            .unwrap_or_else(|error| {
                tracing::error!("WATR table unavailable, water will not draw: {error:#}");
                (Vec::new(), HashMap::new())
            });
        Self::with_waters(
            device,
            queue,
            scene_layout,
            asset_root,
            waters,
            &depth_of_fields,
            loaded_grid,
            config,
            world,
        )
    }

    pub fn with_waters(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene_layout: &wgpu::BindGroupLayout,
        asset_root: &str,
        waters: Vec<Water>,
        depth_of_fields: &HashMap<String, ImageSpaceDepthOfField>,
        loaded_grid: LoadedGrid,
        config: WaterConfig,
        world: SharedWaterWorld,
    ) -> Self {
        let layouts = WaterLayouts::new(device);
        let pipelines = WaterPipelines::new(device, scene_layout, &layouts);
        // Repeat, trilinear.
        let repeat_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("water_repeat_sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let trace = env_flag(TRACE_ENV, false);
        let displacement = WaterDisplacement::new(device, trace);
        let frame = FrameBindings::new(device, queue, &layouts, 1, 1, displacement.targets());
        let draw_constants = DrawConstantsBuffer::new(device, &layouts.draw, INITIAL_DRAW_CAPACITY);
        let ssr = SsrChain::new(device, scene_layout, &layouts);
        Self {
            enabled: env_flag(WATER_ENV, true),
            settings: WaterSettings::default(),
            waters: WaterTable::from_waters(waters, depth_of_fields),
            layouts,
            pipelines,
            repeat_sampler,
            normal_textures: NormalTextures::new(device, queue, asset_root),
            frame,
            displacement,
            displacement_quad: env_flag(DISPLACEMENT_QUAD_ENV, true),
            ssr_far_override: env_opt(SSR_FAR_ENV),
            worldspace_water: None,
            draw_constants,
            quad: QuadMesh::new(device),
            ssr,
            ssr_camera: None,
            surroundings: Surroundings::Exterior,
            ssr_active: false,
            world,
            cell_surface_radius: loaded_grid.radius(),
            config,
            surfaces: Vec::new(),
            draws: Vec::new(),
            constants_scratch: Vec::new(),
            camera_water: CameraWater::NONE,
            camera_eye: Vec3::ZERO,
            surface_depth: WaterSurfaceDepth::new(device, 1, 1),
            underwater_dof: None,
            trace,
            last_trace: Instant::now(),
            tally: DrawTally::default(),
            last_frame_constants: bytemuck::Zeroable::zeroed(),
            debug: DebugSelection::default(),
        }
    }

    /// The `cb1[2]` source for a surface of `surface_water`: the worldspace
    /// water, else `DefaultWater`. Only when neither is in the table does the
    /// surface's own water stand in, which is reported.
    fn environment_for(
        &self,
        surface_water: usize,
        water_multiplier: Option<Vec3>,
    ) -> WaterEnvironment {
        let far_field: FarField = self
            .worldspace_water
            .or_else(|| self.waters.index_of(DEFAULT_WATER_FORM_KEY))
            .map_or_else(
                || {
                    warn_once("water-no-default-water", || {
                        format!(
                            "{DEFAULT_WATER_FORM_KEY} is not in the WATR table; surfaces use their own water as the far-field water"
                        )
                    });
                    self.waters.entries[surface_water].material.far_field()
                },
                |index| self.waters.entries[index].material.far_field(),
            );
        WaterEnvironment::under(far_field, water_multiplier)
    }

    /// Selects what the draws show instead of their shading (`None` for the normal shading),
    /// creating the pipelines of the output the first time it is selected.
    pub fn set_debug_output(&mut self, device: &wgpu::Device, output: Option<WaterDebugOutput>) {
        if let Some(output) = self.debug.select(output) {
            self.pipelines.ensure_debug(device, output);
        }
    }

    /// Whether the stage will draw this frame. Gates the scene colour and
    /// depth copies, which only water consumes.
    pub fn has_work(&self) -> bool {
        !self.draws.is_empty()
    }

    /// The height of the water surface at the engine-space `point`; `None` where there is no
    /// water.
    pub fn surface_height_at(&self, point: Vec3) -> Option<f32> {
        self.world.read().surface_at(point).map(|hit| hit.height)
    }

    /// The camera's relation to the water, for the frame uniform: Fallout 4's
    /// near-plane test against the water world's water height query.
    pub fn camera_water(&self) -> CameraWater {
        self.camera_water
    }

    pub fn settings_mut(&mut self) -> &mut WaterSettings {
        &mut self.settings
    }

    /// Brings the water world up to date with the streamed cells (`scope`
    /// and the resident placed water that `visit_placed` hands over one by
    /// one), then gathers this frame's surfaces from it and works out where
    /// the camera is in the water.
    pub fn collect(
        &mut self,
        camera: &Camera,
        scope: Scope<'_>,
        visit_placed: impl FnOnce(&mut dyn FnMut(PlacedWater<'_>)),
        visit_loaded: impl FnOnce(&mut dyn FnMut(LoadedCell)),
    ) {
        self.surfaces.clear();
        let mut volumes = Vec::new();
        let waters = &self.waters;
        let surfaces = &mut self.surfaces;
        let draw_surfaces = self.enabled;
        visit_placed(&mut |placed: PlacedWater<'_>| {
            volumes.push(PlacedWaterVolume::new(
                placed.transform,
                placed.water_type.map(std::sync::Arc::from),
                placed.volume.cloned(),
            ));
            if !draw_surfaces {
                return;
            }
            let Some(water) = resolve_water(waters, placed.water_type) else {
                return;
            };
            surfaces.push(Surface {
                world: placed.transform,
                bounds: Aabb {
                    min: placed.aabb_min,
                    max: placed.aabb_max,
                },
                geometry: Geometry::Mesh {
                    index_offset: placed.index_offset,
                    index_count: placed.index_count,
                    vertex_offset: placed.vertex_offset,
                },
                water,
                shader: placed.shader,
                plane_point: placed.transform.w_axis.truncate(),
                kind: SurfaceKind::Placed,
            });
        });
        {
            let mut world = self.world.write();
            world.set_scope(scope);
            world.set_placed(volumes);
            let mut loaded = Vec::new();
            visit_loaded(&mut |cell| loaded.push(cell));
            world.set_loaded(loaded);
        }
        let world = self.world.read();
        self.worldspace_water = world
            .worldspace_water_form()
            .and_then(|key| self.waters.index_of(key));
        if world.water_is_cleared() {
            self.surfaces.clear();
        } else if self.enabled {
            let ring = CellSurfaceRing::around(camera.eye, self.cell_surface_radius);
            self.surfaces
                .extend(cell_surfaces(&world, &self.waters, ring));
            self.surfaces.extend(lod_surfaces(
                &world,
                &self.waters,
                camera.eye,
                ring,
                self.config,
            ));
        }
        let ssr_far = self
            .ssr_far_override
            .unwrap_or_else(|| world_camera_far(self.surroundings));
        self.camera_eye = camera.eye;
        self.ssr_camera = Some(SsrCamera::of(camera).with_far(ssr_far));
        let near = NearPlane::new(
            camera.eye,
            camera.forward(),
            camera.up,
            camera.fovy.to_radians(),
            camera.aspect,
            VIRTUAL_NEAR_DISTANCE,
        );
        self.camera_water = if self.settings.underwater_effect {
            camera_water(near, |point| world.surface_at(point).map(|hit| hit.height))
        } else {
            CameraWater::NONE
        };
        self.underwater_dof = self
            .camera_water
            .underwater
            .then(|| world.surface_at(self.camera_water.probe))
            .flatten()
            .and_then(|hit| self.underwater_override_in(Some(hit.water_type)));
    }

    /// The depth of field of a camera whose near plane is fully under the water
    /// `water_type` (state 3 selects the water's image space and the fogged final pass).
    fn underwater_override_in(&self, water_type: Option<&str>) -> Option<UnderwaterOverride> {
        let entry = &self.waters.entries[self.waters.index_of(water_type?)?];
        let dof = underwater_override_dof(self.camera_water.immersion, entry.depth_of_field)?;
        Some(UnderwaterOverride {
            dof,
            fog: entry.underwater_fog.registers(),
        })
    }

    /// What the depth of field effect of this frame takes from the water: the
    /// replacement of the weather's depth of field and the underwater surface
    /// depth the water prepasses wrote.
    pub fn underwater_dof(&self) -> Option<UnderwaterDofInputs<'_>> {
        Some(UnderwaterDofInputs {
            image_space: self.underwater_dof.as_ref()?,
            surface_depth: (!self.draws.is_empty()).then(|| self.surface_depth.view()),
        })
    }

    /// Culls and sorts this frame's surfaces and writes their constants for `setting`. A
    /// camera under water draws every surface with the underwater shader (LOD water has no
    /// underwater variant and keeps its own).
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &FrameUniform,
        setting: WaterSetting,
    ) {
        let interior = setting.is_interior();
        self.draws.clear();
        self.constants_scratch.clear();
        self.tally = DrawTally::default();
        self.ssr_active = false;
        if !self.enabled || self.surfaces.is_empty() {
            self.trace_summary();
            return;
        }
        let planes = clip_planes(&frame.view_proj);
        let camera = frame.camera_pos.truncate();
        let culled: Vec<bool> = self
            .surfaces
            .iter()
            .map(|surface| surface.bounds.is_outside(&planes))
            .collect();
        if self.trace {
            self.tally.placed_culled = self
                .surfaces
                .iter()
                .zip(&culled)
                .filter(|(surface, culled)| surface.kind == SurfaceKind::Placed && **culled)
                .count();
            self.tally.nearest_placed = nearest_placed(&self.surfaces, &culled, camera);
        }
        let order =
            back_to_front(self.surfaces.iter().zip(&culled).map(|(surface, culled)| {
                (!culled).then(|| plane_distance(&surface.world, camera))
            }));
        if order.is_empty() {
            self.trace_summary();
            return;
        }
        let quad_sources = self.displacement_quad_sources(&order);
        let draw_count = order.len() + quad_sources.len();
        if draw_count > self.draw_constants.capacity {
            let capacity = draw_count.next_power_of_two();
            self.draw_constants = DrawConstantsBuffer::new(device, &self.layouts.draw, capacity);
        }
        self.frame.ensure_target_size(
            device,
            &self.layouts,
            frame.viewport.x as u32,
            frame.viewport.y as u32,
        );
        self.surface_depth
            .ensure_size(device, frame.viewport.x as u32, frame.viewport.y as u32);
        let time = frame.time_water.x;
        let ssr_enabled = self.ssr.enabled();
        let quad_register = self
            .displacement
            .quad_draw_register(position_from_engine(camera).truncate());
        let jobs = order
            .into_iter()
            .map(|surface_index| (surface_index, false))
            .chain(
                quad_sources
                    .into_iter()
                    .map(|surface_index| (surface_index, true)),
            );
        for (slot, (surface_index, is_quad)) in jobs.enumerate() {
            let surface = &self.surfaces[surface_index];
            match (is_quad, surface.kind) {
                (true, _) => self.tally.quads += 1,
                (false, SurfaceKind::Lod) => self.tally.lod += 1,
                (false, SurfaceKind::Cell) => self.tally.cell += 1,
                (false, SurfaceKind::Placed) => self.tally.placed += 1,
            }
            let context = PassContext {
                interior,
                underwater: self.camera_water.underwater,
                screen_space_reflections: ssr_enabled
                    && self.waters.entries[surface.water].screen_space_reflections,
            };
            let flags = if is_quad {
                permutation::displacement_quad_flags(surface.shader.water_flags)
            } else {
                surface.shader.water_flags
            };
            let passes = permutation::select(flags, context);
            if passes.unsupported != 0 {
                warn_once(
                    &format!("water-unsupported:{:#x}", passes.unsupported),
                    || {
                        format!(
                            "water surfaces use permutation features the engine lacks ({}); they draw with the nearest permutation",
                            permutation::describe_unsupported(passes.unsupported)
                        )
                    },
                );
            }
            let environment = self.environment_for(surface.water, setting.water_multiplier());
            let geometry = GeometryInputs {
                world: surface.world,
                view_projection: frame.view_proj,
                camera,
                plane_point: surface.plane_point,
                mesh_uv: flags & water_flags::VERTEX_UV != 0,
                interior,
            };
            let entry = &mut self.waters.entries[surface.water];
            let ray_setup = ssr_enabled.then(|| {
                ssr::ray_setup_kind(&ssr::SurfaceSsr {
                    lod: surface.is_lod(),
                    wading: passes.mask_tag == Some(MaskTag::Wading),
                    water_flag: entry.screen_space_reflections,
                    camera_underwater: context.underwater,
                    shading_samples_ssr: passes.variant.depth || passes.variant.ssr,
                })
            });
            entry.material.advance_scroll(time);
            let mut constants = draw_constants(&entry.material, frame, &geometry, &environment);
            if is_quad {
                constants.ps_cb2[DISPLACEMENT_QUAD_DRAW_REGISTER] = quad_register;
            }
            self.constants_scratch.push(constants);
            if entry.normals.is_none() {
                entry.normals = Some(build_normals_group(
                    device,
                    queue,
                    &self.layouts,
                    &self.repeat_sampler,
                    &mut self.normal_textures,
                    &entry.normal_textures,
                ));
            }
            self.draws.push(Draw {
                constants_offset: (slot as u64 * DRAW_CONSTANTS_STRIDE) as u32,
                water: surface.water,
                geometry: surface.geometry,
                passes,
                ray_setup: ray_setup.flatten(),
            });
        }
        self.prepare_ssr(device, queue, frame);
        let mut constants = frame_constants(frame);
        constants.cb12[QUAD_REGISTER] = self.displacement.quad_constants();
        self.last_frame_constants = constants;
        self.frame.write(queue, &self.last_frame_constants);
        queue.write_buffer(
            &self.draw_constants.buffer,
            0,
            bytemuck::cast_slice(&self.constants_scratch),
        );
        self.trace_summary();
    }

    /// The visible surfaces (indices into `self.surfaces`, in draw order) that
    /// get a displacement quad of their own: none while the field is idle.
    fn displacement_quad_sources(&self, visible: &[usize]) -> Vec<usize> {
        if !self.displacement_quad || self.displacement.visible_target().is_none() {
            return Vec::new();
        }
        let candidates = visible.iter().filter_map(|&surface_index| {
            let surface = &self.surfaces[surface_index];
            let covered = !surface.is_lod() && !surface.shader.has_flag(water_flags::DISPLACEMENT);
            let plane = SurfacePlane::of_transform(surface.world);
            let height = plane.height_at(surface.plane_point.x, surface.plane_point.z)?;
            (covered && plane.normal.y.abs() > HORIZONTAL_NORMAL_Y).then_some(QuadCandidate {
                surface: surface_index,
                water: surface.water,
                height,
            })
        });
        choose_quad_sources(candidates)
    }

    /// Runs the wading displacement driver for this frame and uploads its
    /// pass constants: the player (or the camera when none is spawned) is the
    /// origin of the field, `water_entries` are where bodies entered the water.
    pub fn update_displacement(
        &mut self,
        queue: &wgpu::Queue,
        dt: f32,
        reference: Vec3,
        player: Option<PlayerBody>,
        water_entries: &[Vec3],
    ) {
        let world = self.world.read();
        self.displacement.update(
            &EngineFrame {
                dt,
                reference,
                player,
                water_entries,
            },
            |point| world.surface_at(point).map(|hit| hit.height),
        );
        self.displacement.upload(queue);
    }

    /// What surrounds the camera this frame: it decides the far plane the SSR
    /// raytracer reads.
    pub fn set_camera_surroundings(&mut self, surroundings: Surroundings) {
        self.surroundings = surroundings;
    }

    /// Whether the displacement consumes the water entries of the bodies; when
    /// it does not, the caller leaves them undrained (the producer bounds them).
    pub fn wants_water_entries(&self) -> bool {
        self.displacement.enabled()
    }

    /// The displacement passes of this frame. They run whether or not water is
    /// in view: the field's state advances every frame.
    pub fn encode_displacement(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        profiler: &GpuProfiler,
    ) {
        self.displacement.encode(encoder, profiler);
    }

    /// Sizes the SSR targets and writes the ray-setup constants when a draw
    /// needs the chain; the water shader samples the empty stand-ins otherwise,
    /// so a surface that does not use SSR never reads a stale result.
    fn prepare_ssr(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &FrameUniform) {
        self.ssr_active = self.draws.iter().any(|draw| draw.ray_setup.is_some());
        let camera = self.ssr_camera.filter(|_| self.ssr_active);
        let views = camera.map(|camera| {
            let viewport = UVec2::new(frame.viewport.x as u32, frame.viewport.y as u32);
            let views = self.ssr.prepare(device, queue, viewport, &camera);
            let mut constants =
                ray_setup_frame_constants(frame, &camera, half_extent(viewport.x, viewport.y));
            constants.cb12[QUAD_REGISTER] = self.displacement.quad_constants();
            self.frame.write_ray_setup(queue, &constants);
            views
        });
        self.ssr_active = views.is_some();
        self.frame.bind_reflection(device, &self.layouts, views);
    }

    /// `OPENFO4_WATER_TRACE`: once a second, what was drawn and where the
    /// camera is in the water.
    fn trace_summary(&mut self) {
        if !self.trace || self.last_trace.elapsed() < TRACE_INTERVAL {
            return;
        }
        self.last_trace = Instant::now();
        let world = self.world.read();
        tracing::info!(
            "water-trace: surfaces drawn={} (placed={}, cell={}, lod={}, displacement quads={}) of {} collected (placed frustum-culled={}, nearest placed={:?}), ray setups per kind={:?}, ssr far={:?}, resident placed volumes={}, camera underwater={} immersion={:?} fraction={:.2} camera_bu={:?} surface_height={:.2} engine units = {:.0} BU, underwater_fog={:?}",
            self.draws.len(),
            self.tally.placed,
            self.tally.cell,
            self.tally.lod,
            self.tally.quads,
            self.surfaces.len(),
            self.tally.placed_culled,
            self.tally.nearest_placed,
            RaySetupKind::ALL.map(|kind| {
                self.draws
                    .iter()
                    .filter(|draw| draw.ray_setup == Some(kind))
                    .count()
            }),
            self.ssr_camera.map(|camera| camera.far()),
            world.placed_count(),
            self.camera_water.underwater,
            self.camera_water.immersion,
            self.camera_water.fraction,
            position_from_engine(self.camera_eye).to_array(),
            self.camera_water.surface_height,
            self.camera_water.surface_height * UNITS_FROM_ENGINE,
            self.underwater_dof
                .as_ref()
                .map(|replacement| replacement.fog),
        );
        drop(world);
        self.trace_registers();
    }

    /// `OPENFO4_WATER_TRACE`: the shader registers of the first placed and the
    /// first LOD draw, so a wrong colour can be traced to a zero or implausible
    /// register without a GPU capture.
    fn trace_registers(&self) {
        let frame = &self.last_frame_constants;
        for (label, lod) in [("placed", false), ("lod", true)] {
            let Some(index) = self
                .draws
                .iter()
                .position(|draw| draw.passes.variant.lod == lod)
            else {
                continue;
            };
            let draw = &self.draws[index];
            let constants = &self.constants_scratch[index];
            let (cb1, cb2) = (&constants.ps_cb1, &constants.ps_cb2);
            tracing::info!(
                "water-trace {label}: descriptor={:#x} variant={:?} depth_fog={:?} mask={:?} | cb1[0] shallow={} [1] deep={} [2] far={} [3] horizon={} [4] lower={} [5] upper={} [6] silt={} [8]={} [10]={} [11]={} [12]={} [13]={} | cb2[4] plane={} | cb0[2] sun_dir={} [3] sun={} | cb12[35] cam={} [41]={} [42]={} [43]={} [44]={}",
                draw.passes.main_descriptor,
                draw.passes.variant,
                draw.passes.depth_fog,
                draw.passes.mask_tag,
                cb1[0],
                cb1[1],
                cb1[2],
                cb1[3],
                cb1[4],
                cb1[5],
                cb1[6],
                cb1[8],
                cb1[10],
                cb1[11],
                cb1[12],
                cb1[13],
                cb2[4],
                frame.cb0[2],
                frame.cb0[3],
                frame.cb12[35],
                frame.cb12[41],
                frame.cb12[42],
                frame.cb12[43],
                frame.cb12[44],
            );
        }
    }

    /// The passes that run before the scene copy: the mask tags of the
    /// refraction test and the depth fog, which tints the opaque scene so the
    /// main pass refracts the tinted scene.
    pub fn encode_prepasses(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene: &SceneCopy,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        mega: &MegaBuffer,
    ) {
        if self.draws.is_empty() {
            return;
        }
        self.encode_mask(encoder, depth, mega);
        self.encode_depth_fog(device, encoder, scene, color, depth, mega);
        if self.underwater_dof.is_some() {
            self.encode_surface_depth(encoder, depth, mega);
        }
    }

    /// Writes the depth of the underwater surfaces into [`WaterSurfaceDepth`]
    /// (cleared to 0, no surface), depth-tested against the opaque scene held
    /// read-only.
    fn encode_surface_depth(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        mega: &MegaBuffer,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Water Surface Depth Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: self.surface_depth.view(),
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: None,
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(self.pipelines.surface_depth());
        for draw in self
            .draws
            .iter()
            .filter(|draw| draw.passes.variant.underwater)
        {
            pass.set_bind_group(
                DRAW_GROUP,
                &self.draw_constants.bind_group,
                &[draw.constants_offset],
            );
            self.draw_geometry(&mut pass, draw.geometry, mega);
        }
    }

    /// Clears the `t11` mask and lets every water surface tag the pixels it
    /// covers (the constant-colour passes `0x12xxx` / `0x14xxx`), depth-tested
    /// against the opaque scene held read-only.
    fn encode_mask(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        depth: &wgpu::TextureView,
        mega: &MegaBuffer,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Water Mask Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: self.frame.mask_view(),
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: None,
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let mut bound_tag = None;
        for draw in &self.draws {
            let Some(tag) = draw.passes.mask_tag else {
                continue;
            };
            if bound_tag != Some(tag) {
                pass.set_pipeline(self.pipelines.mask(tag));
                bound_tag = Some(tag);
            }
            pass.set_bind_group(
                DRAW_GROUP,
                &self.draw_constants.bind_group,
                &[draw.constants_offset],
            );
            self.draw_geometry(&mut pass, draw.geometry, mega);
        }
    }

    /// The depth-fog pass: blends the water body's colour over the opaque
    /// scene, holding the depth buffer read-only.
    fn encode_depth_fog(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene: &SceneCopy,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        mega: &MegaBuffer,
    ) {
        if !self
            .draws
            .iter()
            .any(|draw| draw.passes.depth_fog.is_some())
        {
            return;
        }
        let scene_group = scene.bind_group_with_live_depth(device, depth);
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Water Depth Fog Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: None,
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(self.pipelines.depth_fog());
        pass.set_bind_group(SCENE_GROUP, &scene_group, &[]);
        pass.set_bind_group(FRAME_GROUP, self.frame.group(), &[]);
        for draw in self
            .draws
            .iter()
            .filter(|draw| draw.passes.depth_fog.is_some())
        {
            pass.set_bind_group(
                DRAW_GROUP,
                &self.draw_constants.bind_group,
                &[draw.constants_offset],
            );
            self.draw_geometry(&mut pass, draw.geometry, mega);
        }
    }

    /// The SSR chain, after the scene copy: ray setup of the SSR surfaces at half
    /// resolution, the Hi-Z pyramid of `depth`, the raytrace over the scene
    /// colour copy and the blurs. Does nothing when no surface needs it.
    pub fn encode_ssr(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        scene: &SceneCopy,
        depth: &wgpu::TextureView,
        mega: &MegaBuffer,
        profiler: &GpuProfiler,
    ) {
        if !self.ssr_active {
            return;
        }
        {
            let Some(mut pass) = self.ssr.begin_ray_setup(encoder, profiler) else {
                return;
            };
            pass.set_bind_group(SCENE_GROUP, scene.bind_group(), &[]);
            let visible_field = self.displacement.visible_target();
            for kind in RaySetupKind::ALL {
                let mut draws = self
                    .draws
                    .iter()
                    .filter(|draw| draw.ray_setup == Some(kind))
                    .peekable();
                if draws.peek().is_none() {
                    continue;
                }
                let frame_group = match visible_field {
                    Some(target) if kind == RaySetupKind::Wading => {
                        self.frame.ray_setup_wading_group(target)
                    }
                    _ => self.frame.ray_setup_group(),
                };
                pass.set_bind_group(FRAME_GROUP, frame_group, &[]);
                pass.set_pipeline(self.ssr.ray_setup_pipeline(kind));
                for draw in draws {
                    let normals = self.waters.entries[draw.water]
                        .normals
                        .as_ref()
                        .expect("prepare builds the normal layers of every drawn water");
                    pass.set_bind_group(NORMALS_GROUP, normals, &[]);
                    pass.set_bind_group(
                        DRAW_GROUP,
                        &self.draw_constants.bind_group,
                        &[draw.constants_offset],
                    );
                    self.draw_geometry(&mut pass, draw.geometry, mega);
                }
            }
        }
        self.ssr
            .encode_resolve(device, encoder, scene, depth, profiler);
    }

    /// Shades the surfaces from the scene copy, or draws the selected debug output in place of
    /// the shading. Returns whether anything was drawn: the pass's bind groups of the object
    /// pipelines are then stale and the caller rebinds them.
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        scene: &SceneCopy,
        mega: &MegaBuffer,
    ) -> bool {
        if self.draws.is_empty() {
            return false;
        }
        pass.set_bind_group(SCENE_GROUP, scene.bind_group(), &[]);
        let visible_field = self.displacement.visible_target();
        let debug = self.debug.output();
        let mut bound_pipeline = None;
        for draw in &self.draws {
            let Some(pipeline) = DrawPipeline::select(debug, &draw.passes) else {
                continue;
            };
            let frame_group = match visible_field {
                Some(target) if draw.passes.reads_displacement() => self.frame.wading_group(target),
                _ => self.frame.group(),
            };
            pass.set_bind_group(FRAME_GROUP, frame_group, &[]);
            if bound_pipeline != Some(pipeline) {
                pass.set_pipeline(self.pipelines.pipeline(pipeline));
                bound_pipeline = Some(pipeline);
            }
            let normals = self.waters.entries[draw.water]
                .normals
                .as_ref()
                .expect("prepare builds the normal layers of every drawn water");
            pass.set_bind_group(NORMALS_GROUP, normals, &[]);
            pass.set_bind_group(
                DRAW_GROUP,
                &self.draw_constants.bind_group,
                &[draw.constants_offset],
            );
            self.draw_geometry(pass, draw.geometry, mega);
        }
        true
    }

    fn draw_geometry(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        geometry: Geometry,
        mega: &MegaBuffer,
    ) {
        match geometry {
            Geometry::Mesh {
                index_offset,
                index_count,
                vertex_offset,
            } => {
                pass.set_vertex_buffer(0, mega.vertex_buffer.slice(..));
                pass.set_index_buffer(mega.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(
                    index_offset..index_offset + index_count,
                    vertex_offset as i32,
                    0..1,
                );
            }
            Geometry::Quad => {
                pass.set_vertex_buffer(0, self.quad.vertices.slice(..));
                pass.set_index_buffer(self.quad.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..QuadMesh::INDICES.len() as u32, 0, 0..1);
            }
        }
    }
}

fn resolve_water(waters: &WaterTable, water_type: Option<&str>) -> Option<usize> {
    let Some(form_key) = water_type else {
        warn_once("water-shape-without-watr", || {
            "a water shape is placed without a WATR (its base object is no water activator); it will not draw".to_owned()
        });
        return None;
    };
    let found = waters.index_of(form_key);
    if found.is_none() {
        warn_once(&format!("water-unknown-watr:{form_key}"), || {
            format!("water type {form_key} is not in the WATR table; its surfaces will not draw")
        });
    }
    found
}

/// The surfaces of the loaded cells, as placed water.
fn cell_surfaces(world: &WaterWorld, waters: &WaterTable, ring: CellSurfaceRing) -> Vec<Surface> {
    let shader = WaterPlane::cell_shader();
    world
        .cell_surfaces()
        .iter()
        .filter(|surface| ring.contains(surface.plane.cell))
        .filter_map(|surface| {
            let water = resolve_water(waters, Some(&surface.water_type))?;
            let quad = WaterPlane::from(&surface.plane);
            let center = (quad.min + quad.max) * 0.5;
            Some(Surface {
                world: quad.transform(),
                bounds: quad.bounds(),
                geometry: Geometry::Quad,
                water,
                shader,
                plane_point: Vec3::new(center.x, quad.height, center.y),
                kind: SurfaceKind::Cell,
            })
        })
        .collect()
}

/// The distant quads within the configured LOD range that no loaded cell surface replaces.
fn lod_surfaces(
    world: &WaterWorld,
    waters: &WaterTable,
    eye: Vec3,
    ring: CellSurfaceRing,
    config: WaterConfig,
) -> Vec<Surface> {
    let Some(lod) = world.lod_water() else {
        return Vec::new();
    };
    let Some(water) = resolve_water(waters, Some(&lod.water_type)) else {
        return Vec::new();
    };
    let shader = WaterPlane::lod_shader(config.reflections);
    lod.within_range(eye, config.lod_range_game_units)
        .filter(|plane| !(ring.contains(plane.cell) && world.has_cell_surface(plane.cell)))
        .map(|plane| lod_surface(plane, lod.plane_height, water, shader))
        .collect()
}

fn lod_surface(
    plane: &crate::water_world::CellPlane,
    plane_height: Option<f32>,
    water: usize,
    shader: WaterShaderToken,
) -> Surface {
    let quad = WaterPlane::from(plane);
    let center = (plane.min + plane.max) * 0.5;
    Surface {
        world: quad.transform(),
        bounds: quad.bounds(),
        geometry: Geometry::Quad,
        water,
        shader,
        // The game gives every LOD water shape the worldspace's LOD height as
        // its plane, whatever height the quad's vertices carry.
        plane_point: Vec3::new(center.x, plane_height.unwrap_or(plane.height), center.y),
        kind: SurfaceKind::Lod,
    }
}

fn build_normals_group(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layouts: &WaterLayouts,
    sampler: &wgpu::Sampler,
    textures: &mut NormalTextures,
    paths: &[Option<String>; 3],
) -> wgpu::BindGroup {
    let views = paths
        .each_ref()
        .map(|path| textures.view(device, queue, path.as_deref()));
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("water_normals_group"),
        layout: &layouts.normals,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&views[0]),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&views[1]),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&views[2]),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

/// A surface that may carry a displacement quad.
struct QuadCandidate {
    surface: usize,
    water: usize,
    /// Plane height under the surface's reference point.
    height: f32,
}

/// The first [`MAX_DISPLACEMENT_QUADS`] candidates, one per water and height.
/// Water objects have no defined order; draw order stands in.
fn choose_quad_sources(candidates: impl Iterator<Item = QuadCandidate>) -> Vec<usize> {
    let mut chosen: Vec<QuadCandidate> = Vec::new();
    for candidate in candidates {
        let duplicate = chosen.iter().any(|kept| {
            kept.water == candidate.water
                && (kept.height - candidate.height).abs() <= QUAD_HEIGHT_TOLERANCE
        });
        if !duplicate && chosen.len() < MAX_DISPLACEMENT_QUADS {
            chosen.push(candidate);
        }
    }
    chosen.into_iter().map(|quad| quad.surface).collect()
}

/// The placed surface whose bounds are nearest the camera (engine space in,
/// Bethesda units out); `None` when no placed surface is collected.
fn nearest_placed(surfaces: &[Surface], culled: &[bool], camera: Vec3) -> Option<NearestPlaced> {
    surfaces
        .iter()
        .zip(culled)
        .filter(|(surface, _)| surface.kind == SurfaceKind::Placed)
        .map(|(surface, culled)| {
            let nearest = camera.clamp(surface.bounds.min, surface.bounds.max);
            NearestPlaced {
                distance: nearest.distance(camera) * UNITS_FROM_ENGINE,
                plane_height: surface.plane_point.y * UNITS_FROM_ENGINE,
                culled: *culled,
            }
        })
        .min_by(|a, b| a.distance.total_cmp(&b.distance))
}

/// How far the camera is from the plane a surface lies in. Sorting water by
/// this orders overlapping surfaces correctly whatever their size: of two
/// parallel surfaces the camera sees the nearer plane in front, so a pond over
/// a distant-water quad draws after it even when the quad's centre is closer.
fn plane_distance(world: &Mat4, camera: Vec3) -> f32 {
    SurfacePlane::of_transform(*world)
        .signed_distance(camera)
        .abs()
}

/// Indices of the visible surfaces, farthest plane first; equal distances
/// keep their collection order. `distances` holds one entry per surface, or
/// `None` when it is culled.
fn back_to_front(distances: impl Iterator<Item = Option<f32>>) -> Vec<usize> {
    let mut visible: Vec<(f32, usize)> = distances
        .enumerate()
        .filter_map(|(index, distance)| Some((distance?, index)))
        .collect();
    visible.sort_by(|a, b| b.0.total_cmp(&a.0));
    visible.into_iter().map(|(_, index)| index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::db::water_placement::LodWaterQuad;
    use crate::water_world::LodWater;
    use crate::water_world::height::{WorldspaceChain, WorldspaceWaterRecord};

    fn at_height(height: f32) -> Mat4 {
        Mat4::from_translation(Vec3::new(0.0, height, 0.0))
    }

    #[test]
    fn surfaces_draw_farthest_first_and_culled_ones_are_dropped() {
        let distances = [Some(1.0), Some(10.0), None, Some(5.0)];
        assert_eq!(back_to_front(distances.into_iter()), [1, 3, 0]);
    }

    fn surface_at(center: Vec3, half: f32, lod: bool) -> Surface {
        Surface {
            world: Mat4::from_translation(center),
            bounds: Aabb {
                min: center - Vec3::splat(half),
                max: center + Vec3::splat(half),
            },
            geometry: Geometry::Quad,
            water: 0,
            shader: WaterShaderToken {
                water_flags: 0,
                shader_flags_1: 0,
                shader_flags_2: 0,
                uv_offset: glam::Vec2::ZERO,
                uv_scale: glam::Vec2::ONE,
            },
            plane_point: center,
            kind: if lod {
                SurfaceKind::Lod
            } else {
                SurfaceKind::Placed
            },
        }
    }

    #[test]
    fn the_nearest_placed_surface_is_reported_in_bethesda_units_and_ignores_lod() {
        let surfaces = [
            surface_at(Vec3::new(1.0, 5.0, 0.0), 0.5, true),
            surface_at(Vec3::new(30.0, 67.0, 0.0), 10.0, false),
            surface_at(Vec3::new(200.0, 10.0, 0.0), 10.0, false),
        ];
        let nearest = nearest_placed(&surfaces, &[false, true, false], Vec3::ZERO).unwrap();
        assert!((nearest.distance - 10.0 * 20.0_f32.hypot(57.0)).abs() < 1e-2);
        assert!((nearest.plane_height - 670.0).abs() < 1e-3);
        assert!(nearest.culled);
    }

    #[test]
    fn no_placed_surface_means_no_nearest() {
        let surfaces = [surface_at(Vec3::ZERO, 1.0, true)];
        assert_eq!(nearest_placed(&surfaces, &[false], Vec3::ZERO), None);
    }

    fn candidate(surface: usize, water: usize, height: f32) -> QuadCandidate {
        QuadCandidate {
            surface,
            water,
            height,
        }
    }

    #[test]
    fn coplanar_surfaces_of_one_water_share_a_quad() {
        let candidates = [
            candidate(0, 3, 10.0),
            candidate(1, 3, 10.005),
            candidate(2, 4, 10.0),
        ];
        assert_eq!(choose_quad_sources(candidates.into_iter()), [0, 2]);
    }

    #[test]
    fn surfaces_of_one_water_at_different_heights_each_get_a_quad() {
        let candidates = [candidate(0, 3, 10.0), candidate(1, 3, 40.0)];
        assert_eq!(choose_quad_sources(candidates.into_iter()), [0, 1]);
    }

    #[test]
    fn at_most_seven_quads_are_drawn() {
        let candidates = (0..12).map(|index| candidate(index, index, 0.0));
        assert_eq!(
            choose_quad_sources(candidates),
            (0..MAX_DISPLACEMENT_QUADS).collect::<Vec<_>>()
        );
    }

    #[test]
    fn equal_distances_keep_their_collection_order() {
        let distances = [Some(2.0), Some(2.0), Some(2.0)];
        assert_eq!(back_to_front(distances.into_iter()), [0, 1, 2]);
    }

    #[test]
    fn the_distance_is_to_the_plane_not_to_the_centre() {
        let camera = Vec3::new(0.0, 100.0, 0.0);
        assert!((plane_distance(&at_height(45.0), camera) - 55.0).abs() < 1e-4);
    }

    #[test]
    fn the_distance_is_the_same_below_the_plane() {
        let camera = Vec3::new(0.0, 10.0, 0.0);
        assert!((plane_distance(&at_height(45.0), camera) - 35.0).abs() < 1e-4);
    }

    #[test]
    fn a_pond_above_a_huge_quad_draws_after_it_seen_from_above() {
        // The quad's centre is under the camera, the pond's is far to the side:
        // nearest-centre sorting would draw the quad last and cover the pond.
        let camera = Vec3::new(0.0, 80.0, 0.0);
        let quad = at_height(45.0);
        let pond = Mat4::from_translation(Vec3::new(500.0, 46.0, 0.0));
        let order = back_to_front(
            [quad, pond]
                .iter()
                .map(|world| Some(plane_distance(world, camera))),
        );
        assert_eq!(order, [0, 1]);
    }

    #[test]
    fn seen_from_below_the_lower_plane_is_nearer_and_draws_last() {
        let camera = Vec3::new(0.0, 10.0, 0.0);
        let order = back_to_front(
            [at_height(45.0), at_height(46.0)]
                .iter()
                .map(|world| Some(plane_distance(world, camera))),
        );
        assert_eq!(order, [1, 0]);
    }

    fn lod_config(reflections: bool, lod_range_game_units: f32) -> WaterConfig {
        WaterConfig {
            reflections,
            lod_range_game_units,
        }
    }

    fn lod_water_table() -> WaterTable {
        let water = Water {
            form_key: "lod".to_owned(),
            editor_id: None,
            name: None,
            material: None,
            data: Some(crate::rendering::water::test_fixtures::synthetic_water()),
        };
        WaterTable::from_waters(vec![water], &HashMap::new())
    }

    /// A worldspace whose LOD water has one quad at each of `cells`.
    fn lod_world(cells: &[(i32, i32)]) -> WaterWorld {
        let quads: Vec<_> = cells
            .iter()
            .map(|&(cell_x, cell_y)| LodWaterQuad {
                cell_x,
                cell_y,
                height: 0.0,
            })
            .collect();
        let chain = WorldspaceChain::new(vec![WorldspaceWaterRecord {
            form_key: "ws".to_owned(),
            parent: None,
            parent_flags: 0,
            water_form: None,
            lod_water_form: Some("lod".to_owned()),
            lod_water_height: None,
            default_water_height: 0.0,
            flags: 0,
        }]);
        WaterWorld::with_exterior(chain, HashMap::new(), LodWater::new(&quads, "lod", None))
    }

    fn surfaces_around_the_origin(world: &WaterWorld, config: WaterConfig) -> Vec<Surface> {
        let ring = CellSurfaceRing::around(Vec3::ZERO, 0);
        lod_surfaces(world, &lod_water_table(), Vec3::ZERO, ring, config)
    }

    #[test]
    fn the_configured_range_decides_which_lod_quads_draw() {
        // Cell 30 starts 122880 game units east of the camera.
        let world = lod_world(&[(0, 0), (30, 0)]);
        let near_only = surfaces_around_the_origin(&world, lod_config(false, 100_000.0));
        let both = surfaces_around_the_origin(&world, lod_config(false, 150_000.0));
        assert_eq!(near_only.len(), 1);
        assert_eq!(both.len(), 2);
    }

    #[test]
    fn lod_surfaces_reflect_exactly_when_the_config_says_so() {
        let world = lod_world(&[(0, 0)]);
        for reflections in [false, true] {
            let surfaces = surfaces_around_the_origin(&world, lod_config(reflections, 100_000.0));
            assert_eq!(surfaces.len(), 1);
            let shader = &surfaces[0].shader;
            assert!(shader.has_flag(water_flags::LOD));
            assert_eq!(shader.has_flag(water_flags::REFLECTIONS), reflections);
            assert_eq!(surfaces[0].kind, SurfaceKind::Lod);
        }
    }

    #[test]
    fn a_tilted_surface_is_measured_against_its_own_plane() {
        let world = Mat4::from_rotation_translation(
            glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Vec3::new(10.0, 0.0, 0.0),
        );
        // The surface plane is vertical through x = 10.
        assert!((plane_distance(&world, Vec3::new(3.0, 50.0, 0.0)) - 7.0).abs() < 1e-4);
    }
}
