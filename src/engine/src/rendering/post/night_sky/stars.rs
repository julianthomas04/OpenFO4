//! The stars: the sky model dome of the climate, centred on the camera at infinite depth,
//! turned by the star rotation and added over the sky gradient.

use glam::{Mat4, Quat};
use wgpu::util::DeviceExt;

use super::assets::{AssetState, SkyAssetError, StarGeometry, report};
use super::layer::{self, LAYER_GROUP, LayerPipeline, WorldGroupLayouts};
use crate::rendering::lighting::time_of_day::StarRotation;
use crate::rendering::lighting::time_of_day::unmeasured::{STAR_HORIZON_FADE_SINE, STAR_RADIANCE};
use crate::rendering::shaders;
use crate::rendering::vertex::Vertex;
use crate::utils::finite_or_zero;

/// Mirrors `StarUniform` in `sky_stars.wgsl`.
#[repr(C)]
#[derive(Debug, Copy, Clone, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct StarUniform {
    pub rotation: [[f32; 4]; 4],
    /// x: opacity of the stars, y: the sky radiance scale times the radiance of a star texel,
    /// z: the sine of the elevation at which the stars are fully faded in.
    pub params: [f32; 4],
}

impl StarUniform {
    /// An axis without length cannot turn the dome, which then stays as modelled.
    pub(super) fn new(rotation: StarRotation, alpha: f32, radiance_scale: f32) -> Self {
        let turn = rotation
            .axis
            .try_normalize()
            .map_or(Quat::IDENTITY, |axis| {
                Quat::from_axis_angle(axis, finite_or_zero(rotation.angle_rad))
            });
        Self {
            rotation: Mat4::from_quat(turn).to_cols_array_2d(),
            params: [
                finite_or_zero(alpha).clamp(0.0, 1.0),
                finite_or_zero(radiance_scale).max(0.0) * STAR_RADIANCE,
                STAR_HORIZON_FADE_SINE,
                0.0,
            ],
        }
    }

    pub(super) fn opacity(&self) -> f32 {
        self.params[0]
    }
}

struct StarMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

impl StarMesh {
    fn new(device: &wgpu::Device, geometry: &StarGeometry) -> Self {
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("night_sky_star_vertices"),
            contents: bytemuck::cast_slice(&geometry.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("night_sky_star_indices"),
            contents: bytemuck::cast_slice(&geometry.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        Self {
            vertices,
            indices,
            index_count: geometry.indices.len() as u32,
        }
    }
}

pub(super) struct StarLayer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bind_group: Option<wgpu::BindGroup>,
    mesh: Option<StarMesh>,
    active: bool,
}

impl StarLayer {
    pub(super) fn new(device: &wgpu::Device, world_groups: WorldGroupLayouts<'_>) -> Self {
        let bind_group_layout = layer::bind_group_layout(device, "night_sky_stars_bgl");
        let pipeline = layer::create_pipeline(
            device,
            &LayerPipeline {
                label: "Night Sky Stars Pipeline",
                shader: &shaders::SKY_STARS,
                world_groups,
                layer_layout: &bind_group_layout,
                vertex_buffers: &[Some(Vertex::desc())],
                topology: wgpu::PrimitiveTopology::TriangleList,
                color_blend: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            },
        );
        Self {
            pipeline,
            uniform: layer::uniform_buffer::<StarUniform>(device, "night_sky_stars_uniform"),
            sampler: layer::sampler(device, "night_sky_stars_sampler", wgpu::AddressMode::Repeat),
            bind_group_layout,
            bind_group: None,
            mesh: None,
            active: false,
        }
    }

    pub(super) fn install_texture(
        &mut self,
        device: &wgpu::Device,
        texture: Result<wgpu::TextureView, SkyAssetError>,
    ) -> AssetState {
        match texture {
            Ok(view) => {
                self.bind_group = Some(layer::bind_group(
                    device,
                    "night_sky_stars_bind_group",
                    &self.bind_group_layout,
                    &self.uniform,
                    &view,
                    &self.sampler,
                ));
                AssetState::Loaded
            }
            Err(error) => {
                report(&error);
                self.bind_group = None;
                AssetState::failed(&error)
            }
        }
    }

    /// `None` is a climate without a sky model: no dome, nothing drawn.
    pub(super) fn install_mesh(
        &mut self,
        device: &wgpu::Device,
        geometry: Option<Result<StarGeometry, SkyAssetError>>,
    ) -> AssetState {
        match geometry {
            None => {
                self.mesh = None;
                AssetState::NotRequested
            }
            Some(Ok(geometry)) => {
                self.mesh = Some(StarMesh::new(device, &geometry));
                AssetState::Loaded
            }
            Some(Err(error)) => {
                report(&error);
                self.mesh = None;
                AssetState::failed(&error)
            }
        }
    }

    pub(super) fn update(&mut self, queue: &wgpu::Queue, uniform: &StarUniform) {
        self.active = uniform.opacity() > 0.0 && self.bind_group.is_some() && self.mesh.is_some();
        if self.active {
            queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(uniform));
        }
    }

    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let (true, Some(bind_group), Some(mesh)) = (self.active, &self.bind_group, &self.mesh)
        else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(LAYER_GROUP, bind_group, &[]);
        pass.set_vertex_buffer(0, mesh.vertices.slice(..));
        pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..mesh.index_count, 0, 0..1);
    }
}
