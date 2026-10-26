//! Runs the lighting functions of the object shader on the GPU, one compute thread per case.

use std::sync::OnceLock;
use std::sync::mpsc;

use bytemuck::{Pod, Zeroable};
use glam::{Vec3, Vec4};

use crate::rendering::device::REQUIRED_FEATURES;
use crate::rendering::gpu_cull::{storage_entry, uniform_entry};
use crate::rendering::gpu_test::test_device;
use crate::rendering::lighting::ambient::AMBIENT_CUBE_FACES;
use crate::rendering::lighting::fog::{Fog, FogUniformFields};
use crate::rendering::lighting::frame_uniform::FrameUniform;
use crate::rendering::lighting::god_rays::InScatterUniformFields;
use crate::rendering::shaders::OBJECT;
use crate::rendering::shadows::lamps::table::SHADOW_SLOT_NONE;

const ENTRY_SOURCE: &str = include_str!("entry.wgsl");
const SKY_FOG_SOURCE: &str = include_str!("../sky_fog.wgsl");
const ENTRY_POINT: &str = "lighting_test_main";
const MAX_SPECULAR_INTENSITY_OVERRIDE: &str = "MAX_SPECULAR_INTENSITY";
const FRAME_GROUP: u32 = 1;
const IO_GROUP: u32 = 3;
const WORKGROUP_SIZE: usize = 64;
const CASES_PER_DISPATCH: usize = 200_000;

const KIND_DIRECT: u32 = 0;
const KIND_LOCAL: u32 = 1;
const KIND_AMBIENT: u32 = 2;
const KIND_ATTENUATION: u32 = 3;
const KIND_CONE: u32 = 4;
const KIND_AMBIENT_TOWARD: u32 = 5;
const KIND_FALLOFF: u32 = 6;
const KIND_COMPOSE_BLEND: u32 = 7;
const KIND_COMPOSE_MULTIPLY: u32 = 8;
const KIND_ENV_TERM: u32 = 9;
const KIND_ENV_EXPOSURE: u32 = 10;
const KIND_ENV_ENABLED: u32 = 11;
const KIND_ENV_DIRECTION: u32 = 12;
const KIND_PALETTE_COORDINATES: u32 = 13;
const KIND_APPLY_PALETTE: u32 = 14;
const KIND_EFFECT_LAYER: u32 = 15;
const KIND_EFFECT_FACING: u32 = 16;
const KIND_LIT_ENV_F0: u32 = 17;
const KIND_FOG: u32 = 18;
const KIND_SKY_FOG: u32 = 19;
const KIND_APPLY_FOG: u32 = 20;

/// Mirrors `TestCase` in `entry.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct Case {
    albedo_smoothness: Vec4,
    normal_intensity: Vec4,
    to_light: Vec4,
    to_viewer: Vec4,
    light_color: Vec4,
    light_axis_cos_outer: Vec4,
    attenuation_exponent: Vec4,
    position_radius: Vec4,
    ids: [u32; 4],
}

const _: () = assert!(std::mem::size_of::<Case>() == 144);

/// Mirrors `TestResult` in `entry.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct RawResult {
    first: Vec4,
    second: Vec4,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Output {
    pub diffuse: Vec3,
    pub specular: Vec3,
}

impl Output {
    pub fn scalar(&self) -> f32 {
        self.diffuse.x
    }

    /// The colour and the alpha of a blend composition.
    pub fn color_alpha(&self) -> (Vec3, f32) {
        (self.diffuse, self.specular.x)
    }
}

/// What a blend composition shades: the colour for the classes that fade out with distance,
/// the one that mixes towards the fog colour, the reflection, and the fog transmittance.
#[derive(Debug, Copy, Clone)]
pub struct Shaded {
    pub body: Vec3,
    pub fogged_body: Vec3,
    pub reflection: Vec3,
    pub transmittance: f32,
}

#[derive(Debug, Copy, Clone)]
pub struct Surface {
    pub albedo: Vec3,
    pub normal: Vec3,
    pub smoothness: f32,
    pub spec_intensity: f32,
}

/// Inputs of the effect layer shaping.
#[derive(Debug, Copy, Clone)]
pub struct EffectLayer {
    pub flags: u32,
    pub falloff: Vec4,
    pub tint: Vec3,
    pub texel: Vec4,
    pub alpha_mult: f32,
    pub facing: f32,
}

/// Inputs of the environment reflection term.
#[derive(Debug, Copy, Clone)]
pub struct EnvTerm {
    pub radiance: Vec3,
    pub f0: f32,
    pub n_dot_v: f32,
    pub mask: f32,
    pub mask_scale: f32,
    pub exposure: f32,
    pub roughness_lod: f32,
    pub level_count: u32,
}

impl Default for EnvTerm {
    fn default() -> Self {
        Self {
            radiance: Vec3::ONE,
            f0: 0.04,
            n_dot_v: 1.0,
            mask: 1.0,
            mask_scale: 1.0,
            exposure: 1.0,
            roughness_lod: 0.0,
            level_count: 1,
        }
    }
}

impl Default for Surface {
    fn default() -> Self {
        Self {
            albedo: Vec3::ONE,
            normal: Vec3::Z,
            smoothness: 0.5,
            spec_intensity: 1.0,
        }
    }
}

impl Surface {
    pub fn glossy(smoothness: f32) -> Self {
        Self {
            smoothness,
            ..Self::default()
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Light {
    pub position: Vec3,
    pub radius: f32,
    pub flags: u32,
    pub attenuation: Vec3,
    pub spot_exponent: f32,
    pub direction: Vec3,
    pub spot_cos_outer: f32,
    pub shadow_slot: u32,
}

impl Default for Light {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            radius: 100.0,
            flags: 0,
            attenuation: Vec3::new(0.0, 1.0, 2.0),
            spot_exponent: 1.0,
            direction: Vec3::NEG_Z,
            spot_cos_outer: -1.0,
            shadow_slot: SHADOW_SLOT_NONE,
        }
    }
}

impl Light {
    pub fn with_flags(flags: u32) -> Self {
        Self {
            flags,
            ..Self::default()
        }
    }

    /// A spot light shining along -Z with the given half-angle.
    pub fn spot(flags: u32, half_angle_degrees: f32, exponent: f32, shadow_slot: u32) -> Self {
        Self {
            flags,
            spot_exponent: exponent,
            spot_cos_outer: half_angle_degrees.to_radians().cos(),
            shadow_slot,
            ..Self::default()
        }
    }

    pub fn attenuated(constant: f32, scalar: f32, exponent: f32) -> Self {
        Self {
            attenuation: Vec3::new(constant, scalar, exponent),
            ..Self::default()
        }
    }
}

impl Case {
    fn blank(kind: u32) -> Self {
        Self {
            ids: [kind, 0, SHADOW_SLOT_NONE, 0],
            ..Zeroable::zeroed()
        }
    }

    fn with_surface(mut self, surface: &Surface) -> Self {
        self.albedo_smoothness = surface.albedo.extend(surface.smoothness);
        self.normal_intensity = surface.normal.extend(surface.spec_intensity);
        self
    }

    fn with_light(mut self, light: &Light) -> Self {
        self.light_axis_cos_outer = light.direction.extend(light.spot_cos_outer);
        self.attenuation_exponent = light.attenuation.extend(light.spot_exponent);
        self.position_radius = light.position.extend(light.radius);
        self.ids[1] = light.flags;
        self.ids[2] = light.shadow_slot;
        self
    }

    pub fn direct(surface: &Surface, l: Vec3, v: Vec3, color: Vec3) -> Self {
        let mut case = Self::blank(KIND_DIRECT).with_surface(surface);
        case.to_light = l.extend(0.0);
        case.to_viewer = v.extend(0.0);
        case.light_color = color.extend(0.0);
        case
    }

    pub fn local(surface: &Surface, light: &Light, l: Vec3, v: Vec3, color: Vec3) -> Self {
        let mut case = Self::blank(KIND_LOCAL)
            .with_surface(surface)
            .with_light(light);
        case.to_light = l.extend(0.0);
        case.to_viewer = v.extend(0.0);
        case.light_color = color.extend(0.0);
        case
    }

    pub fn ambient(surface: &Surface, v: Vec3) -> Self {
        let mut case = Self::blank(KIND_AMBIENT).with_surface(surface);
        case.to_viewer = v.extend(0.0);
        case
    }

    pub fn attenuation(light: &Light, distance: f32) -> Self {
        let mut case = Self::blank(KIND_ATTENUATION).with_light(light);
        case.to_light = Vec4::new(0.0, 0.0, 0.0, distance);
        case
    }

    pub fn cone(light: &Light, l: Vec3) -> Self {
        let mut case = Self::blank(KIND_CONE).with_light(light);
        case.to_light = l.extend(0.0);
        case
    }

    /// `falloff` is start and stop facing, then start and stop opacity.
    pub fn falloff(falloff: Vec4, facing: f32) -> Self {
        let mut case = Self::blank(KIND_FALLOFF);
        case.attenuation_exponent = falloff;
        case.to_light = Vec4::new(0.0, 0.0, 0.0, facing);
        case
    }

    fn composition(kind: u32, shaded: &Shaded, alpha: f32) -> Self {
        let mut case = Self::blank(kind);
        case.albedo_smoothness = shaded.body.extend(0.0);
        case.normal_intensity = shaded.fogged_body.extend(0.0);
        case.to_light = shaded.reflection.extend(0.0);
        case.to_viewer = Vec4::new(alpha, shaded.transmittance, 0.0, 0.0);
        case
    }

    pub fn compose_blend(flags: u32, shaded: &Shaded, alpha: f32) -> Self {
        let mut case = Self::composition(KIND_COMPOSE_BLEND, shaded, alpha);
        case.ids[1] = flags;
        case
    }

    pub fn compose_multiply(shaded: &Shaded, alpha: f32) -> Self {
        Self::composition(KIND_COMPOSE_MULTIPLY, shaded, alpha)
    }

    /// The reflection term for a cube sample of `radiance`; the weight comes back in the
    /// second output's x, the mip level in its y.
    pub fn env_term(term: &EnvTerm) -> Self {
        let mut case = Self::blank(KIND_ENV_TERM);
        case.light_color = term.radiance.extend(0.0);
        case.to_viewer = Vec4::new(term.f0, term.n_dot_v, term.mask, term.mask_scale);
        case.position_radius = Vec4::new(term.exposure, term.roughness_lod, 0.0, 0.0);
        case.ids[3] = term.level_count;
        case
    }

    pub fn env_exposure(direction: Vec3) -> Self {
        let mut case = Self::blank(KIND_ENV_EXPOSURE);
        case.to_light = direction.extend(0.0);
        case
    }

    pub fn env_enabled(effect_flags: u32, env_cube_index: u32) -> Self {
        let mut case = Self::blank(KIND_ENV_ENABLED);
        case.ids[1] = effect_flags;
        case.ids[2] = env_cube_index;
        case
    }

    /// The cube sampling direction for an engine-space reflection direction.
    pub fn env_direction(direction: Vec3) -> Self {
        let mut case = Self::blank(KIND_ENV_DIRECTION);
        case.to_light = direction.extend(0.0);
        case
    }

    /// The palette coordinates of `base` (diffuse rgba) as (colour, alpha) in the output's
    /// diffuse x and y, and the uv for `coordinate` in diffuse z and specular x.
    pub fn palette_coordinates(base: Vec4, coordinate: f32) -> Self {
        let mut case = Self::blank(KIND_PALETTE_COORDINATES);
        case.albedo_smoothness = base;
        case.to_light = Vec4::new(coordinate, 0.0, 0.0, 0.0);
        case
    }

    /// Palette resolution of `base` given what the palette returns for the luminance and
    /// for the alpha coordinate.
    pub fn apply_palette(flags: u32, base: Vec4, by_luminance: Vec4, by_alpha: Vec4) -> Self {
        let mut case = Self::blank(KIND_APPLY_PALETTE);
        case.ids[1] = flags;
        case.albedo_smoothness = base;
        case.normal_intensity = by_luminance;
        case.to_light = by_alpha;
        case
    }

    /// Tinted, faded effect layer; the colour is the output's diffuse, the alpha its
    /// specular x and the pre-falloff albedo its specular yz (red and green).
    pub fn effect_layer(layer: &EffectLayer) -> Self {
        let mut case = Self::blank(KIND_EFFECT_LAYER);
        case.ids[1] = layer.flags;
        case.attenuation_exponent = layer.falloff;
        case.albedo_smoothness = layer.tint.extend(0.0);
        case.normal_intensity = layer.texel;
        case.to_viewer = Vec4::new(layer.alpha_mult, layer.facing, 0.0, 0.0);
        case
    }

    /// Reflectance at normal incidence of a lit material whose specular colour holds the
    /// packed bytes `specular_rgb` (red in the lowest byte).
    pub fn lit_env_f0(specular_rgb: [u8; 3], spec_intensity: f32) -> Self {
        let mut case = Self::blank(KIND_LIT_ENV_F0);
        case.ids[1] = u32::from_le_bytes([specular_rgb[0], specular_rgb[1], specular_rgb[2], 0]);
        case.to_light = Vec4::new(spec_intensity, 0.0, 0.0, 0.0);
        case
    }

    pub fn effect_facing(geometric_normal: Vec3, view: Vec3) -> Self {
        let mut case = Self::blank(KIND_EFFECT_FACING);
        case.to_light = geometric_normal.extend(0.0);
        case.to_viewer = view.extend(0.0);
        case
    }

    pub fn ambient_toward(direction: Vec3) -> Self {
        let mut case = Self::blank(KIND_AMBIENT_TOWARD);
        case.to_light = direction.extend(0.0);
        case
    }

    /// The fog colour (diffuse) and opacity (specular x) at `world_pos`.
    pub fn fog(world_pos: Vec3) -> Self {
        let mut case = Self::blank(KIND_FOG);
        case.to_light = world_pos.extend(0.0);
        case
    }

    /// The sky `radiance` in the unit `direction` under the fog (diffuse), and the sky's share of
    /// the fog there (specular x).
    pub fn sky_fog(radiance: Vec3, direction: Vec3) -> Self {
        let mut case = Self::blank(KIND_SKY_FOG);
        case.light_color = radiance.extend(0.0);
        case.to_light = direction.extend(0.0);
        case
    }

    /// A surface of `color` at `world_pos` under the fog (diffuse).
    pub fn apply_fog(color: Vec3, world_pos: Vec3) -> Self {
        let mut case = Self::blank(KIND_APPLY_FOG);
        case.light_color = color.extend(0.0);
        case.to_light = world_pos.extend(0.0);
        case
    }
}

/// Run-wide inputs shared by every case of one dispatch.
#[derive(Debug, Copy, Clone)]
pub struct Environment {
    /// The `MAX_SPECULAR_INTENSITY` override of the shader; `None` builds the pipeline without
    /// any override, as the production pipelines do.
    pub max_specular_intensity: Option<f32>,
    /// Linear ambient colour of each cube face in the frame uniform's order.
    pub ambient_cube: [Vec3; AMBIENT_CUBE_FACES],
    pub camera: Vec3,
    /// The fog with the sun's in-scatter.
    pub fog: FogUniformFields,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            max_specular_intensity: None,
            ambient_cube: [Vec3::ZERO; AMBIENT_CUBE_FACES],
            camera: Vec3::ZERO,
            fog: Fog::Disabled.uniform_fields(),
        }
    }
}

impl Environment {
    pub fn with_fog(camera: Vec3, fog: &Fog) -> Self {
        Self {
            camera,
            fog: fog.uniform_fields(),
            ..Self::default()
        }
    }

    pub fn with_fog_and_in_scatter(
        camera: Vec3,
        fog: &Fog,
        in_scatter: InScatterUniformFields,
    ) -> Self {
        Self {
            camera,
            fog: fog.uniform_fields().with_in_scatter(in_scatter),
            ..Self::default()
        }
    }

    pub fn with_specular_ceiling(ceiling: f32) -> Self {
        Self {
            max_specular_intensity: Some(ceiling),
            ..Self::default()
        }
    }
}

pub struct LightingHarness {
    device: wgpu::Device,
    queue: wgpu::Queue,
    module: wgpu::ShaderModule,
    frame_layout: wgpu::BindGroupLayout,
    io_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
}

impl LightingHarness {
    /// The process-wide harness, or why no usable GPU could be set up.
    pub fn shared() -> Result<&'static Self, &'static str> {
        static SHARED: OnceLock<Result<LightingHarness, String>> = OnceLock::new();
        SHARED
            .get_or_init(Self::create)
            .as_ref()
            .map_err(String::as_str)
    }

    fn create() -> Result<Self, String> {
        let (device, queue) =
            test_device("lighting tests").ok_or_else(|| "no shared GPU test device".to_string())?;
        let missing = REQUIRED_FEATURES - device.features();
        if !missing.is_empty() {
            return Err(format!(
                "the object shader needs device features the adapter lacks: {missing:?}"
            ));
        }
        let source = format!("{}{SKY_FOG_SOURCE}{ENTRY_SOURCE}", OBJECT.wgsl());
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lighting_test_shader"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lighting_test_frame_bgl"),
            entries: &[uniform_entry(0)],
        });
        let io_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lighting_test_io_bgl"),
            entries: &[storage_entry(0, true), storage_entry(1, false)],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lighting_test_layout"),
            bind_group_layouts: &[None, Some(&frame_layout), None, Some(&io_layout)],
            immediate_size: 0,
        });
        Ok(Self {
            device,
            queue,
            module,
            frame_layout,
            io_layout,
            pipeline_layout,
        })
    }

    fn pipeline(&self, max_specular_intensity: Option<f32>) -> wgpu::ComputePipeline {
        let constants: Vec<(&str, f64)> = max_specular_intensity
            .map(|ceiling| (MAX_SPECULAR_INTENSITY_OVERRIDE, f64::from(ceiling)))
            .into_iter()
            .collect();
        self.device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("lighting_test_pipeline"),
                layout: Some(&self.pipeline_layout),
                module: &self.module,
                entry_point: Some(ENTRY_POINT),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants: &constants,
                    ..Default::default()
                },
                cache: None,
            })
    }

    fn frame_bind_group(&self, environment: &Environment) -> wgpu::BindGroup {
        let mut frame = FrameUniform::zeroed();
        frame.ambient_cube = environment.ambient_cube.map(|face| face.extend(0.0));
        frame.camera_pos = environment.camera.extend(1.0);
        environment.fog.write_to(&mut frame);
        let buffer = self.buffer_with(
            "lighting_test_frame",
            wgpu::BufferUsages::UNIFORM,
            bytemuck::bytes_of(&frame),
        );
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lighting_test_frame_bg"),
            layout: &self.frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        })
    }

    fn buffer_with(&self, label: &str, usage: wgpu::BufferUsages, data: &[u8]) -> wgpu::Buffer {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: data.len() as u64,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buffer, 0, data);
        buffer
    }

    pub fn run(&self, environment: &Environment, cases: &[Case]) -> Vec<Output> {
        let pipeline = self.pipeline(environment.max_specular_intensity);
        let frame_group = self.frame_bind_group(environment);
        cases
            .chunks(CASES_PER_DISPATCH)
            .flat_map(|chunk| self.dispatch(&pipeline, &frame_group, chunk))
            .collect()
    }

    fn dispatch(
        &self,
        pipeline: &wgpu::ComputePipeline,
        frame_group: &wgpu::BindGroup,
        cases: &[Case],
    ) -> Vec<Output> {
        let result_bytes = (cases.len() * std::mem::size_of::<RawResult>()) as u64;
        let case_buffer = self.buffer_with(
            "lighting_test_cases",
            wgpu::BufferUsages::STORAGE,
            bytemuck::cast_slice(cases),
        );
        let result_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lighting_test_results"),
            size: result_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lighting_test_staging"),
            size: result_bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let io_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lighting_test_io_bg"),
            layout: &self.io_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: case_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: result_buffer.as_entire_binding(),
                },
            ],
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
            pass.set_pipeline(pipeline);
            pass.set_bind_group(FRAME_GROUP, frame_group, &[]);
            pass.set_bind_group(IO_GROUP, &io_group, &[]);
            pass.dispatch_workgroups(cases.len().div_ceil(WORKGROUP_SIZE) as u32, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&result_buffer, 0, &staging, 0, result_bytes);
        self.queue.submit(std::iter::once(encoder.finish()));
        self.read_back(&staging)
    }

    fn read_back(&self, staging: &wgpu::Buffer) -> Vec<Output> {
        let (sender, outcome) = mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                sender
                    .send(result)
                    .expect("readback outlives its map callback");
            });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("lighting test poll failed");
        outcome
            .recv()
            .expect("wait_indefinitely runs the map callback")
            .expect("lighting test readback map failed");
        let data = staging
            .slice(..)
            .get_mapped_range()
            .expect("mapped staging buffer exposes its full range");
        let results: Vec<RawResult> = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        staging.unmap();
        results
            .into_iter()
            .map(|raw| Output {
                diffuse: raw.first.truncate(),
                specular: raw.second.truncate(),
            })
            .collect()
    }
}

/// Unit vector at `polar_degrees` from +Z and `azimuth_degrees` about it.
pub fn polar(polar_degrees: f64, azimuth_degrees: f64) -> Vec3 {
    let (theta, phi) = (polar_degrees.to_radians(), azimuth_degrees.to_radians());
    Vec3::new(
        (theta.sin() * phi.cos()) as f32,
        (theta.sin() * phi.sin()) as f32,
        theta.cos() as f32,
    )
}
