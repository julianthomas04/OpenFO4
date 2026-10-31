//! Procedural grass: one instanced draw per (cell, grass type), drawn inside the
//! opaque world pass. The cell loader places the blades (`CellGrass`) and the
//! streaming workers parse the models (`GrassModelCache`); this stage owns their
//! GPU buffers, the grass materials, and the per-frame culling and draw
//! submission. Models and grass types are reference counted by the resident
//! draws that use them, so a model, its bind groups and its texture references
//! live exactly as long as some cell draws it.

mod cell_book;
mod fade;
mod model;
mod panel;
mod pipelines;
mod registry;
mod uniforms;
mod visibility;
mod wind;

use wgpu::util::DeviceExt;

use self::cell_book::{CellBook, CellKey};
pub use self::fade::FadeLimits;
use self::fade::{FadeOverride, GrassFade};
use self::model::{GrassMeshCpu, GrassModelCpu};
pub use self::model::{GrassModelCache, GrassModels};
pub use self::panel::{GrassPanelState, grass_panel};
use self::pipelines::{GRASS_GROUP, GrassPipelines};
use self::registry::RefRegistry;
use self::uniforms::{GrassBatchUniform, GrassFrameUniform};
pub use self::visibility::GrassView;
use self::visibility::{blade_reach, blades_bounds};
use self::wind::GrassWind;
pub use self::wind::commonwealth_clear_wind;
use crate::assets::types::grass::{CellGrass, GrassFlags};
use crate::rendering::frustum::Aabb;
use crate::rendering::render_pipelines::world_pass_layout::WorldPassLayout;
use crate::rendering::skinning::materials::{
    CharacterGpuContext, ResolvedMaterial, resolve_material,
};
use crate::rendering::texture_streaming::residency::TextureResidency;
use crate::rendering::weather::WindFrame;
use crate::utils::{env_flag, error_once, warn_once};

const GRASS_ENV: &str = "OPENFO4_GRASS";

/// What the last frame's grass pass submitted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GrassStats {
    pub draws: u32,
    pub instances: u32,
}

/// Resident totals of the stage, for the debug panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GrassResidency {
    pub cells: usize,
    pub types: usize,
    pub models: usize,
    pub blades: u64,
    pub instance_bytes: u64,
}

/// The panel's live controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrassControls {
    pub draw: bool,
    pub fade: FadeOverride,
}

impl GrassControls {
    fn new(limits: &FadeLimits) -> Self {
        Self {
            draw: true,
            fade: FadeOverride::configured(limits),
        }
    }
}

/// What integrating one cell's grass did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellGrassAdded {
    /// Whether the stage holds grass for the cell afterwards.
    pub holds_grass: bool,
    /// Buffer bytes created for the cell, for the streamer's upload budget.
    pub uploaded_bytes: u64,
}

struct ModelSubmesh {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    material: ResolvedMaterial,
    has_color_stream: bool,
}

/// A grass model on the GPU, shared by every grass type that uses it. It holds
/// the texture references of its materials until it is released.
struct GrassModel {
    submeshes: Vec<ModelSubmesh>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct TypeKey {
    model_path: String,
    flags: u8,
    wave_period_bits: u32,
}

/// A grass type: a model with the flags and wave period of one GRAS record. The
/// shader parameters of each submesh live in one bind group per submesh.
struct GrassType {
    id: u64,
    batches: Vec<wgpu::BindGroup>,
}

/// The blades of one grass type in one cell.
struct CellDraw {
    type_key: TypeKey,
    type_id: u64,
    instances: wgpu::Buffer,
    count: u32,
    bounds: Aabb,
}

pub struct GrassStage {
    enabled: bool,
    pipelines: GrassPipelines,
    frame_buffer: wgpu::Buffer,
    wind: GrassWind,
    fade: GrassFade,
    /// How far a blade may lean in the culling bounds: that of the strongest wind the stage may be
    /// blown with, so no wind it is blown with leans a blade out of its bounds.
    max_lean: f32,
    controls: GrassControls,
    applied_fade: FadeOverride,
    fade_limits: FadeLimits,
    draw_distance: f32,
    models: RefRegistry<String, GrassModel>,
    types: RefRegistry<TypeKey, GrassType>,
    next_type_id: u64,
    cells: CellBook<CellDraw>,
    stats: GrassStats,
}

impl GrassStage {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        world: &WorldPassLayout,
        frame_layout: &wgpu::BindGroupLayout,
        fade_limits: FadeLimits,
        wind_limit: WindFrame,
    ) -> Self {
        let wind = GrassWind::default_weather();
        let fade = GrassFade::configured(&fade_limits);
        let frame_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("grass_frame"),
            contents: bytemuck::bytes_of(&GrassFrameUniform::new(wind, fade)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        Self {
            enabled: env_flag(GRASS_ENV, true),
            pipelines: GrassPipelines::new(device, format, world, frame_layout),
            frame_buffer,
            wind,
            fade,
            max_lean: GrassWind::from_wind(wind_limit).max_lean(),
            controls: GrassControls::new(&fade_limits),
            applied_fade: FadeOverride::configured(&fade_limits),
            fade_limits,
            draw_distance: fade.draw_distance(&fade_limits),
            models: RefRegistry::default(),
            types: RefRegistry::default(),
            next_type_id: 0,
            cells: CellBook::default(),
            stats: GrassStats::default(),
        }
    }

    pub fn stats(&self) -> GrassStats {
        self.stats
    }

    /// Whether `OPENFO4_GRASS` allows the stage to hold grass at all.
    pub fn available(&self) -> bool {
        self.enabled
    }

    pub fn fade_limits(&self) -> FadeLimits {
        self.fade_limits
    }

    pub fn controls_mut(&mut self) -> &mut GrassControls {
        &mut self.controls
    }

    pub fn residency(&self) -> GrassResidency {
        let draws = || self.cells.entries();
        GrassResidency {
            cells: self.cells.cell_count(),
            types: self.types.len(),
            models: self.models.len(),
            blades: draws().map(|draw| u64::from(draw.count)).sum(),
            instance_bytes: draws().map(|draw| draw.instances.size()).sum(),
        }
    }

    /// Applies the panel's fade override to the culling distance and the shader
    /// uniform; a no-op while it is unchanged.
    pub fn sync_controls(&mut self, queue: &wgpu::Queue) {
        if self.controls.fade == self.applied_fade {
            return;
        }
        let resolved = self.controls.fade.resolve(&self.fade_limits);
        self.fade = resolved.fade;
        self.write_frame_uniform(queue);
        self.draw_distance = resolved.fade.draw_distance(&self.fade_limits);
        self.applied_fade = self.controls.fade;
    }

    /// Blows the grass in `wind` from now on; a no-op while it is unchanged. The culling bounds
    /// hold for any wind up to the limit the stage was made with.
    pub fn set_wind(&mut self, queue: &wgpu::Queue, wind: WindFrame) {
        let wind = GrassWind::from_wind(wind);
        if wind == self.wind {
            return;
        }
        self.wind = wind;
        self.write_frame_uniform(queue);
    }

    fn write_frame_uniform(&self, queue: &wgpu::Queue) {
        queue.write_buffer(
            &self.frame_buffer,
            0,
            bytemuck::bytes_of(&GrassFrameUniform::new(self.wind, self.fade)),
        );
    }

    /// Takes over the grass of a freshly integrated cell, replacing any `key`
    /// held. `models` are the parsed models the cell's worker prepared.
    pub fn add_cell(
        &mut self,
        key: CellKey,
        grass: &[CellGrass],
        models: &GrassModels,
        gpu: &mut CharacterGpuContext,
    ) -> CellGrassAdded {
        if !self.enabled {
            return CellGrassAdded::default();
        }
        let mut uploaded_bytes = 0;
        let draws: Vec<CellDraw> = grass
            .iter()
            .filter_map(|grass| self.cell_draw(grass, models, gpu, &mut uploaded_bytes))
            .collect();
        let holds_grass = !draws.is_empty();
        for replaced in self.cells.insert(key, draws).into_iter().flatten() {
            self.release_draw(&replaced, gpu);
        }
        CellGrassAdded {
            holds_grass,
            uploaded_bytes,
        }
    }

    /// Releases the grass of a retired cell; returns whether it held any.
    pub fn remove_cell(&mut self, key: CellKey, gpu: &mut CharacterGpuContext) -> bool {
        let Some(draws) = self.cells.remove(key) else {
            return false;
        };
        for draw in &draws {
            self.release_draw(draw, gpu);
        }
        true
    }

    /// Drops every cell and every model, releasing their texture references.
    pub fn clear_cells(&mut self, gpu: &mut CharacterGpuContext) {
        self.cells.clear();
        self.types.drain().for_each(drop);
        for model in self.models.drain() {
            release_model_textures(&model, gpu);
        }
    }

    /// Grass textures stay at full detail while grass is resident: the streamer's
    /// demand sweep knows nothing of them. Only models some resident cell draws
    /// are held, so only their textures are pinned.
    pub fn note_texture_demand(&self, residency: &mut TextureResidency) {
        for submesh in self.models.values().flat_map(|model| &model.submeshes) {
            for handle in &submesh.material.texture_refs {
                residency.note_demand(handle.slot, f32::INFINITY);
            }
        }
    }

    /// Submits the draws of every cell the view sees, batched by grass type so a
    /// type's bind group and mesh buffers are bound once for all its cells. The
    /// caller keeps the world pass bind groups 0 to 2 bound and rebinds whatever
    /// it had on `GRASS_GROUP` afterwards.
    pub fn draw(&mut self, pass: &mut wgpu::RenderPass<'_>, view: &GrassView) {
        self.stats = GrassStats::default();
        if !self.enabled || !self.controls.draw {
            return;
        }
        let mut visible: Vec<&CellDraw> = self
            .cells
            .entries()
            .filter(|draw| view.sees(&draw.bounds, self.draw_distance))
            .collect();
        let mut stats = GrassStats::default();
        let mut bound_two_sided = None;
        for run in batch_by_type(&mut visible, |draw| draw.type_id) {
            let key = &run[0].type_key;
            let (Some(grass_type), Some(model)) =
                (self.types.get(key), self.models.get(&key.model_path))
            else {
                error_once(&format!("grass-draw-unheld:{}", key.model_path), || {
                    format!(
                        "grass type of '{}' is drawn but not held; its cells are skipped",
                        key.model_path
                    )
                });
                continue;
            };
            for (submesh, batch) in model.submeshes.iter().zip(&grass_type.batches) {
                let two_sided = submesh.material.two_sided;
                if bound_two_sided != Some(two_sided) {
                    pass.set_pipeline(self.pipelines.pipeline(two_sided));
                    bound_two_sided = Some(two_sided);
                }
                pass.set_bind_group(GRASS_GROUP, batch, &[]);
                pass.set_vertex_buffer(0, submesh.vertex_buffer.slice(..));
                pass.set_index_buffer(submesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                for draw in run {
                    pass.set_vertex_buffer(1, draw.instances.slice(..));
                    pass.draw_indexed(0..submesh.index_count, 0, 0..draw.count);
                    stats.draws += 1;
                    stats.instances += draw.count;
                }
            }
        }
        self.stats = stats;
    }

    fn cell_draw(
        &mut self,
        grass: &CellGrass,
        models: &GrassModels,
        gpu: &mut CharacterGpuContext,
        uploaded_bytes: &mut u64,
    ) -> Option<CellDraw> {
        if grass.blades.is_empty() {
            return None;
        }
        let Some(cpu) = models.get(&grass.model_path) else {
            warn_once(
                &format!("grass-model-unprepared:{}", grass.model_path),
                || {
                    format!(
                        "grass model '{}' was not prepared for its cell; its grass is not drawn",
                        grass.model_path
                    )
                },
            );
            return None;
        };
        let reach = blade_reach(cpu.radius, &grass.blades, self.max_lean);
        let bounds = blades_bounds(&grass.blades, reach)?;
        let (type_key, type_id) = self.acquire_type(grass, cpu, gpu, uploaded_bytes)?;
        let instances = gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("grass_blades"),
                contents: bytemuck::cast_slice(&grass.blades),
                usage: wgpu::BufferUsages::VERTEX,
            });
        *uploaded_bytes += instances.size();
        Some(CellDraw {
            type_key,
            type_id,
            instances,
            count: grass.blades.len() as u32,
            bounds,
        })
    }

    /// Takes a reference to the grass type of `grass`, creating it (and uploading
    /// its model) on first use.
    fn acquire_type(
        &mut self,
        grass: &CellGrass,
        cpu: &GrassModelCpu,
        gpu: &mut CharacterGpuContext,
        uploaded_bytes: &mut u64,
    ) -> Option<(TypeKey, u64)> {
        let key = TypeKey {
            model_path: grass.model_path.clone(),
            flags: grass.flags.0,
            wave_period_bits: grass.wave_period.to_bits(),
        };
        let Self {
            models,
            types,
            pipelines,
            frame_buffer,
            next_type_id,
            ..
        } = self;
        let held = types.acquire(key.clone(), || {
            let model = models.acquire(key.model_path.clone(), || {
                Some(upload_model(cpu, gpu, uploaded_bytes))
            })?;
            let batches = model
                .submeshes
                .iter()
                .map(|submesh| {
                    batch_bind_group(
                        gpu.device,
                        pipelines,
                        frame_buffer,
                        submesh,
                        grass.flags,
                        grass.wave_period,
                    )
                })
                .collect();
            let id = *next_type_id;
            *next_type_id += 1;
            Some(GrassType { id, batches })
        })?;
        Some((key, held.id))
    }

    fn release_draw(&mut self, draw: &CellDraw, gpu: &mut CharacterGpuContext) {
        if self.types.release(&draw.type_key).is_none() {
            return;
        }
        if let Some(model) = self.models.release(&draw.type_key.model_path) {
            release_model_textures(&model, gpu);
        }
    }
}

/// Sorts `draws` by grass type and splits them into one run per type.
fn batch_by_type<T>(draws: &mut [T], type_id: impl Fn(&T) -> u64) -> impl Iterator<Item = &[T]> {
    draws.sort_unstable_by_key(&type_id);
    draws.chunk_by(move |a, b| type_id(a) == type_id(b))
}

fn release_model_textures(model: &GrassModel, gpu: &mut CharacterGpuContext) {
    for submesh in &model.submeshes {
        gpu.release_textures(&submesh.material.texture_refs);
    }
}

fn batch_bind_group(
    device: &wgpu::Device,
    pipelines: &GrassPipelines,
    frame_buffer: &wgpu::Buffer,
    submesh: &ModelSubmesh,
    flags: GrassFlags,
    wave_period: f32,
) -> wgpu::BindGroup {
    let batch = GrassBatchUniform::new(
        submesh.material.material_id,
        submesh.material.alpha_cutoff,
        flags,
        wave_period,
        submesh.has_color_stream,
    );
    let batch_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("grass_batch"),
        contents: bytemuck::bytes_of(&batch),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("grass_batch"),
        layout: &pipelines.batch_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: frame_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: batch_buffer.as_entire_binding(),
            },
        ],
    })
}

fn upload_model(
    cpu: &GrassModelCpu,
    gpu: &mut CharacterGpuContext,
    uploaded_bytes: &mut u64,
) -> GrassModel {
    let submeshes = cpu
        .meshes
        .iter()
        .map(|mesh| upload_submesh(mesh, gpu, uploaded_bytes))
        .collect();
    GrassModel { submeshes }
}

fn upload_submesh(
    mesh: &GrassMeshCpu,
    gpu: &mut CharacterGpuContext,
    uploaded_bytes: &mut u64,
) -> ModelSubmesh {
    let mut buffer = |label, contents: &[u8], usage| {
        *uploaded_bytes += contents.len() as u64;
        gpu.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            })
    };
    let vertex_buffer = buffer(
        "grass_vertices",
        bytemuck::cast_slice(&mesh.vertices),
        wgpu::BufferUsages::VERTEX,
    );
    let index_buffer = buffer(
        "grass_indices",
        bytemuck::cast_slice(&mesh.indices),
        wgpu::BufferUsages::INDEX,
    );
    ModelSubmesh {
        vertex_buffer,
        index_buffer,
        index_count: mesh.indices.len() as u32,
        material: resolve_material(mesh.material_name.as_deref(), gpu),
        has_color_stream: mesh.has_color_stream,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_are_grouped_into_one_run_per_type() {
        let mut draws = [(2, 'a'), (1, 'b'), (2, 'c'), (3, 'd'), (1, 'e')];
        let runs: Vec<Vec<char>> = batch_by_type(&mut draws, |draw| draw.0)
            .map(|run| {
                let mut letters: Vec<char> = run.iter().map(|draw| draw.1).collect();
                letters.sort_unstable();
                letters
            })
            .collect();
        assert_eq!(runs, vec![vec!['b', 'e'], vec!['a', 'c'], vec!['d']]);
    }

    #[test]
    fn no_draws_make_no_runs() {
        let mut draws: [(u64, char); 0] = [];
        assert_eq!(batch_by_type(&mut draws, |draw| draw.0).count(), 0);
    }
}
