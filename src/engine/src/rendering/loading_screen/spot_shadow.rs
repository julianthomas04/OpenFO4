//! The shadows of the loading menu's three spotlights: the model's depth seen from each light,
//! drawn into tiles of the world's lamp-shadow atlas and looked up by the world's lamp-shadow
//! code (`lamp_shadow.wgsl`), with the world's lamp-shadow table, bias and filter. Only the
//! caster pass is the loading art's own, since the world's casters draw its own instance layout.

use bytemuck::Zeroable;
use glam::{Mat4, Vec3, Vec4};

use super::model::Draw;
use super::scene_lights::{GpuSceneLight, LIGHT_COUNT};
use crate::bethesda_space::UNITS_TO_ENGINE;
use crate::rendering::placed_light::defaults;
use crate::rendering::shaders;
use crate::rendering::shadows::lamps::atlas::Tile;
use crate::rendering::shadows::lamps::atlas_texture::LampAtlasTexture;
use crate::rendering::shadows::lamps::geometry::{CUBE_FACES, LampFrustums, LampProjection};
use crate::rendering::shadows::lamps::table::{GpuShadowLamp, GpuShadowLampTable};
use crate::rendering::shadows::lamps::{LampShadowConfig, LampShadowSettings};
use crate::rendering::shadows::shadow_maps::{SHADOW_MAP_FORMAT, comparison_sampler};
use crate::rendering::vertex::Vertex;

/// The side of each light's tile. The world sizes its tiles by distance; these three stay put.
pub const TILE_SIZE: u32 = 1024;
/// Two tiles by two hold the three lights'.
const ATLAS_SIZE: u32 = 2 * TILE_SIZE;
/// The receiver-distance fade of the world's lookup is out of reach: the viewer is the origin and
/// the model a few tens of engine units away.
const FADE_END: f32 = 1.0e15;
/// The world's lamp-shadow biases unscaled: the INI's `fShadowBiasScale` is tuned for the world's
/// tiles, which the model's 1024 tile on a 10 degree cone are not.
const BIAS_SCALE: f32 = 1.0;
/// The strength of a lamp's shadow in the table: fully dark where occluded.
const STRENGTH: f32 = 1.0;

/// The tile of each light in the atlas.
fn tile_of(light: usize) -> Tile {
    Tile {
        x: (light as u32 % 2) * TILE_SIZE,
        y: (light as u32 / 2) * TILE_SIZE,
        size: TILE_SIZE,
    }
}

/// What the lights' shadow maps are made of this frame.
pub struct LightShadowFrame {
    pub table: GpuShadowLampTable,
    /// Each light's view-projection for its caster pass; `None` where it has no shadow.
    pub view_proj: [Option<Mat4>; LIGHT_COUNT],
}

impl LightShadowFrame {
    /// The table of the lights seen from where they stand towards `aim`, filtered and biased
    /// like the world's lamp shadows of `config`. A light whose cone is too wide for one tile
    /// has no shadow.
    pub fn of(lights: &[GpuSceneLight; LIGHT_COUNT], aim: Vec3, config: &LampShadowConfig) -> Self {
        let near_clip = defaults::NEAR_CLIP * UNITS_TO_ENGINE;
        let mut table = GpuShadowLampTable::zeroed();
        let mut view_proj = [None; LIGHT_COUNT];
        for (index, light) in lights.iter().enumerate() {
            let Some(projection) = LampProjection::single_spot(light.half_angle_cosine()) else {
                crate::utils::warn_once(&format!("loading-spot-too-wide:{index}"), || {
                    format!(
                        "loading menu scene light {} has a cone too wide for one shadow tile; it casts no shadow",
                        index + 1
                    )
                });
                continue;
            };
            let position = light.position();
            let frustums = LampFrustums::from_parts(
                position,
                aim - position,
                near_clip,
                projection,
                TILE_SIZE,
            );
            let tile = tile_of(index);
            let mut tiles = [Vec4::ZERO; CUBE_FACES];
            tiles[0] =
                Vec4::new(tile.x as f32, tile.y as f32, tile.size as f32, 0.0) / ATLAS_SIZE as f32;
            table.lamps[index] =
                GpuShadowLamp::new(&frustums, tiles, position, near_clip, STRENGTH);
            view_proj[index] = Some(frustums.view_projs[0]);
        }
        let settings = LampShadowSettings::new(config);
        table.header(
            &LampShadowConfig {
                fade_end: FADE_END,
                bias_scale: BIAS_SCALE,
                ..*config
            },
            &settings,
        );
        Self { table, view_proj }
    }
}

pub struct SpotShadows {
    atlas: LampAtlasTexture,
    table_buffer: wgpu::Buffer,
    /// The receiver group of `loading_art.wgsl`: the table, the atlas and the comparison sampler.
    pub receiver_group: wgpu::BindGroup,
    caster_pipeline: wgpu::RenderPipeline,
    caster_buffers: Vec<wgpu::Buffer>,
    caster_groups: Vec<wgpu::BindGroup>,
}

impl SpotShadows {
    pub fn new(
        device: &wgpu::Device,
        receiver_layout: &wgpu::BindGroupLayout,
        scene_layout: &wgpu::BindGroupLayout,
        texture_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let atlas = LampAtlasTexture::new(device, ATLAS_SIZE);
        let table_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("loading_spot_shadow_table"),
            size: std::mem::size_of::<GpuShadowLampTable>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = comparison_sampler(device);
        let receiver_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("loading_spot_shadow_receiver_group"),
            layout: receiver_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: table_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(atlas.view()),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let caster_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("loading_spot_caster_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: CASTER_BINDING,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let caster_buffers: Vec<wgpu::Buffer> = (0..LIGHT_COUNT)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("loading_spot_caster_view"),
                    size: std::mem::size_of::<Mat4>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            })
            .collect();
        let caster_groups = caster_buffers
            .iter()
            .map(|buffer| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("loading_spot_caster_group"),
                    layout: &caster_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: CASTER_BINDING,
                        resource: buffer.as_entire_binding(),
                    }],
                })
            })
            .collect();
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("loading_spot_caster_layout"),
            bind_group_layouts: &[
                Some(scene_layout),
                Some(texture_layout),
                Some(&caster_layout),
            ],
            immediate_size: 0,
        });
        let shader = shaders::LOADING_ART.compile(device);
        // Depth-only and both faces, like the world's casters; cut-out texels cast nothing.
        let caster_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Loading Spot Caster Pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_shadow"),
                buffers: &[Some(Vertex::desc())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_shadow"),
                targets: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_MAP_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self {
            atlas,
            table_buffer,
            receiver_group,
            caster_pipeline,
            caster_buffers,
            caster_groups,
        }
    }

    /// Uploads `frame`'s table and draws the model's depth from each light into its tile. The
    /// scene group holds the model's matrix; `draws` and `texture_groups` pair up.
    pub fn render(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &LightShadowFrame,
        casters: &Casters<'_>,
    ) {
        queue.write_buffer(&self.table_buffer, 0, bytemuck::bytes_of(&frame.table));
        for (buffer, view_proj) in self.caster_buffers.iter().zip(&frame.view_proj) {
            if let Some(view_proj) = view_proj {
                queue.write_buffer(buffer, 0, bytemuck::bytes_of(view_proj));
            }
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Loading Spot Shadow Pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: self.atlas.view(),
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            occlusion_query_set: None,
            timestamp_writes: None,
            multiview_mask: None,
        });
        for (index, view_proj) in frame.view_proj.iter().enumerate() {
            if view_proj.is_none() {
                continue;
            }
            self.atlas.begin_tile(&mut pass, tile_of(index));
            pass.set_pipeline(&self.caster_pipeline);
            pass.set_bind_group(0, casters.scene_group, &[]);
            pass.set_bind_group(2, &self.caster_groups[index], &[]);
            pass.set_vertex_buffer(0, casters.vertices.slice(..));
            pass.set_index_buffer(casters.indices.slice(..), wgpu::IndexFormat::Uint32);
            for (draw, group) in casters.draws.iter().zip(casters.texture_groups) {
                pass.set_bind_group(1, group, &[]);
                pass.draw_indexed(draw.indices.clone(), draw.base_vertex, 0..1);
            }
        }
    }
}

/// The binding of the light's view in the caster group, as `loading_art.wgsl` declares it.
const CASTER_BINDING: u32 = 3;

/// The model as the caster pass draws it.
pub struct Casters<'a> {
    pub scene_group: &'a wgpu::BindGroup,
    pub vertices: &'a wgpu::Buffer,
    pub indices: &'a wgpu::Buffer,
    pub draws: &'a [Draw],
    pub texture_groups: &'a [wgpu::BindGroup],
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::loading_screen::scene_lights::SceneLights;
    use crate::rendering::loading_screen::test_support::lamp_shadow_config;

    fn frame(aim: Vec3) -> LightShadowFrame {
        LightShadowFrame::of(&SceneLights::VANILLA.to_gpu(), aim, &lamp_shadow_config())
    }

    #[test]
    fn the_three_lights_get_distinct_tiles_inside_the_atlas() {
        let tiles: Vec<Tile> = (0..LIGHT_COUNT).map(tile_of).collect();

        let origins: Vec<(u32, u32)> = tiles.iter().map(|tile| (tile.x, tile.y)).collect();
        assert_eq!(origins, [(0, 0), (1024, 0), (0, 1024)]);
        assert!(
            tiles
                .iter()
                .all(|tile| tile.x + tile.size <= ATLAS_SIZE && tile.y + tile.size <= ATLAS_SIZE)
        );
    }

    #[test]
    fn every_vanilla_light_casts_a_shadow() {
        let frame = frame(Vec3::new(0.0, 0.0, -30.0));

        assert!(frame.view_proj.iter().all(Option::is_some));
    }

    #[test]
    fn a_light_with_a_cone_too_wide_for_a_tile_casts_no_shadow() {
        let mut lights = SceneLights::VANILLA;
        lights.lights[1].spot_fov = 150.0;

        let frame = LightShadowFrame::of(
            &lights.to_gpu(),
            Vec3::new(0.0, 0.0, -30.0),
            &lamp_shadow_config(),
        );

        assert!(frame.view_proj[0].is_some());
        assert!(frame.view_proj[1].is_none());
        assert!(frame.view_proj[2].is_some());
    }

    #[test]
    fn the_table_header_carries_the_worlds_filter_and_unscaled_bias_and_no_distance_fade() {
        let config = lamp_shadow_config();
        assert_ne!(
            config.bias_scale, 1.0,
            "the INI's scale must not leak into the bias"
        );

        let frame = frame(Vec3::new(0.0, 0.0, -30.0));

        let settings = LampShadowSettings::new(&config);
        assert_eq!(
            frame.table.bias.truncate(),
            Vec3::new(
                settings.constant_bias,
                settings.slope_bias,
                settings.normal_offset
            )
        );
        assert_eq!(frame.table.filter_taps.x, config.filter.taps_per_axis());
        assert!(frame.table.bias.w > 1.0e29, "{}", frame.table.bias.w);
    }

    #[test]
    fn a_lights_view_looks_from_it_towards_the_aim() {
        let aim = Vec3::new(0.0, 0.0, -30.0);
        let lights = SceneLights::VANILLA.to_gpu();
        let frame = LightShadowFrame::of(&lights, aim, &lamp_shadow_config());

        for (light, view_proj) in lights.iter().zip(frame.view_proj) {
            let clip = view_proj.unwrap() * aim.extend(1.0);
            let ndc = clip.truncate() / clip.w;
            assert!(
                ndc.x.abs() < 1e-4 && ndc.y.abs() < 1e-4,
                "{ndc} from {}",
                light.position()
            );
        }
    }
}
