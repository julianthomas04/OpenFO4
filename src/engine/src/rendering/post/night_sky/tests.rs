use std::f32::consts::FRAC_PI_2;

use std::sync::Arc;

use glam::{Mat4, UVec2, Vec3, Vec4};
use serde_json::{Value, json};
use wgpu::util::DeviceExt;

use super::assets::{
    AssetFailureKind, AssetState, SkyAssetError, SkyAssetStatus, StarGeometry, load_star_geometry,
    moon_texture_key, star_texture_key,
};
use super::moon::{MoonLayer, MoonSizes, MoonUniform, PhaseTextures};
use super::quad::quad_basis;
use super::stars::{StarLayer, StarUniform};
use super::sun::SunInput;
use super::{MoonInput, NightSky, NightSkyInput, SkySizes, WorldGroupLayouts};
use crate::assets::types::mesh::{CpuMesh, Topology};
use crate::assets::types::texture::TextureError;
use crate::rendering::gpu_test::test_device;
use crate::rendering::lighting::frame_uniform::FrameUniform;
use crate::rendering::lighting::time_of_day::unmeasured::{
    MOON_RADIANCE, STAR_RADIANCE, moon_angular_diameter_deg,
};
use crate::rendering::lighting::time_of_day::{
    ClimateSunTextures, MoonKind, MoonPhase, SkyObjects, StarRotation,
};
use crate::rendering::post::dof_test_support::{read_hdr, view_of};
use crate::rendering::post::hdr_target::HDR_FORMAT;
use crate::rendering::shaders;
use crate::rendering::texture::{Texture, TextureAssetError};
use crate::rendering::vertex::Vertex;

fn sun_input() -> SunInput {
    SunInput {
        direction: Vec3::Y,
        alpha: 1.0,
        disc_color: Vec3::ONE,
        glare_color: Vec3::ONE,
    }
}

fn assert_close(actual: Vec3, expected: Vec3) {
    assert!(
        actual.abs_diff_eq(expected, 1e-5),
        "{actual} is not {expected}"
    );
}

/// A GLB container of the glTF document `json` and the binary buffer `bin`.
pub(super) fn glb(json: &Value, bin: &[u8]) -> Vec<u8> {
    fn chunk(kind: [u8; 4], data: &[u8], pad: u8) -> Vec<u8> {
        let mut data = data.to_vec();
        while !data.len().is_multiple_of(4) {
            data.push(pad);
        }
        let mut out = (data.len() as u32).to_le_bytes().to_vec();
        out.extend(kind);
        out.extend(data);
        out
    }
    let mut body = chunk(*b"JSON", json.to_string().as_bytes(), b' ');
    body.extend(chunk(*b"BIN\0", bin, 0));
    let mut out = b"glTF".to_vec();
    out.extend(2u32.to_le_bytes());
    out.extend(((12 + body.len()) as u32).to_le_bytes());
    out.extend(body);
    out
}

/// An install whose only file is `Meshes\<file_dir>\<file_name>`.
pub(super) fn install_with(file_dir: &str, file_name: &str, bytes: &[u8]) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("Meshes").join(file_dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(file_name), bytes).unwrap();
    root
}

mod keys {
    use super::*;

    #[test]
    fn a_moon_texture_key_names_the_phase_file_below_the_sky_textures() {
        assert_eq!(
            moon_texture_key(MoonKind::Masser, MoonPhase::ThreeWan),
            "textures\\sky\\masser_three_wan.dds"
        );
        assert_eq!(
            moon_texture_key(MoonKind::Secunda, MoonPhase::OneWax),
            "textures\\sky\\secunda_one_wax.dds"
        );
    }

    #[test]
    fn every_moon_and_phase_has_a_key_of_its_own() {
        let mut keys: Vec<String> = [MoonKind::Masser, MoonKind::Secunda]
            .into_iter()
            .flat_map(|kind| MoonPhase::ALL.map(|phase| moon_texture_key(kind, phase)))
            .collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), 16);
    }

    #[test]
    fn the_star_texture_key_is_the_sky_stars_file() {
        assert_eq!(star_texture_key(), "textures\\sky\\skystars.dds");
    }
}

mod moon_maths {
    use super::*;

    #[test]
    fn looking_down_the_view_axis_puts_the_quad_axes_on_the_screen_axes() {
        let (right, up) = quad_basis(Vec3::NEG_Z);
        assert_close(right, Vec3::X);
        assert_close(up, Vec3::Y);
    }

    #[test]
    fn the_basis_is_orthonormal_and_keeps_the_moon_upright_at_every_height() {
        for elevation_deg in [-80.0f32, -10.0, 0.0, 30.0, 85.0] {
            for azimuth_deg in [0.0f32, 90.0, 200.0, 315.0] {
                let (elevation, azimuth) = (elevation_deg.to_radians(), azimuth_deg.to_radians());
                let direction = Vec3::new(
                    elevation.cos() * azimuth.sin(),
                    elevation.sin(),
                    -elevation.cos() * azimuth.cos(),
                );
                let (right, up) = quad_basis(direction);
                assert!((right.length() - 1.0).abs() < 1e-5);
                assert!((up.length() - 1.0).abs() < 1e-5);
                assert!(right.dot(up).abs() < 1e-5);
                assert!(right.dot(direction).abs() < 1e-5);
                assert!(up.dot(direction).abs() < 1e-5);
                assert!(up.y >= 0.0, "{direction} leans {up}");
                assert_close(right.cross(up), -direction);
            }
        }
    }

    #[test]
    fn the_basis_stays_defined_at_the_zenith_and_nadir() {
        for direction in [Vec3::Y, Vec3::NEG_Y] {
            let (right, up) = quad_basis(direction);
            assert!(right.is_finite() && up.is_finite());
            assert!(right.dot(up).abs() < 1e-5);
            assert!((right.length() - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn the_quad_spans_the_tangent_of_half_the_disc_diameter() {
        let uniform = MoonUniform::new(Vec3::Z, 3.0, 1.0, 1.0).unwrap();
        assert!((uniform.half_extent - 1.5f32.to_radians().tan()).abs() < 1e-7);
    }

    #[test]
    fn the_direction_is_normalised_and_visibility_clamped() {
        let uniform = MoonUniform::new(Vec3::new(0.0, 0.0, 5.0), 3.0, 7.0, 2.0).unwrap();
        assert_eq!(uniform.direction, [0.0, 0.0, 1.0]);
        assert_eq!(uniform.visibility, 1.0);
        assert_eq!(uniform.radiance_scale, 2.0 * MOON_RADIANCE);
    }

    #[test]
    fn a_moon_that_cannot_be_seen_gives_no_uniform() {
        let moon = |direction: Vec3, diameter: f32, visibility: f32| {
            MoonUniform::new(direction, diameter, visibility, 1.0)
        };
        assert!(moon(Vec3::ZERO, 3.0, 1.0).is_none(), "no direction");
        assert!(moon(Vec3::NAN, 3.0, 1.0).is_none(), "undefined direction");
        assert!(moon(Vec3::X, 3.0, 0.0).is_none(), "invisible");
        assert!(moon(Vec3::X, 3.0, -1.0).is_none(), "negative visibility");
        assert!(
            moon(Vec3::X, 3.0, f32::NAN).is_none(),
            "undefined visibility"
        );
        assert!(moon(Vec3::X, 0.0, 1.0).is_none(), "no size");
        assert!(moon(Vec3::X, f32::NAN, 1.0).is_none(), "undefined size");
    }

    #[test]
    fn a_negative_radiance_scale_is_clamped_to_zero() {
        let uniform = MoonUniform::new(Vec3::X, 3.0, 1.0, -4.0).unwrap();
        assert_eq!(uniform.radiance_scale, 0.0);
    }

    #[test]
    fn the_vanilla_moons_are_a_few_degrees_wide_and_masser_is_the_larger() {
        let masser = moon_angular_diameter_deg(MoonSizes::VANILLA.of(MoonKind::Masser));
        let secunda = moon_angular_diameter_deg(MoonSizes::VANILLA.of(MoonKind::Secunda));
        assert!((2.0..=5.0).contains(&secunda), "{secunda}");
        assert!(masser > secunda && masser <= 5.0, "{masser}");
    }
}

mod star_maths {
    use super::*;

    fn turn(axis: Vec3, angle_rad: f32) -> StarUniform {
        StarUniform::new(StarRotation { axis, angle_rad }, 1.0, 1.0)
    }

    fn apply(uniform: &StarUniform, point: Vec3) -> Vec3 {
        Mat4::from_cols_array_2d(&uniform.rotation).transform_vector3(point)
    }

    #[test]
    fn a_quarter_turn_about_up_is_right_handed() {
        assert_close(apply(&turn(Vec3::Y, FRAC_PI_2), Vec3::X), Vec3::NEG_Z);
    }

    #[test]
    fn the_axis_length_does_not_change_the_turn() {
        let long = turn(Vec3::Y * 9.0, 1.0);
        let unit = turn(Vec3::Y, 1.0);
        assert_close(
            apply(&long, Vec3::new(1.0, 2.0, 3.0)),
            apply(&unit, Vec3::new(1.0, 2.0, 3.0)),
        );
    }

    #[test]
    fn a_turn_that_cannot_be_made_leaves_the_dome_as_modelled() {
        let point = Vec3::new(1.0, 2.0, 3.0);
        assert_close(apply(&turn(Vec3::ZERO, 1.0), point), point);
        assert_close(apply(&turn(Vec3::Y, f32::NAN), point), point);
        assert_close(apply(&turn(Vec3::Y, 0.0), point), point);
    }

    #[test]
    fn the_opacity_is_clamped_and_the_scale_cannot_go_negative() {
        let rotation = StarRotation {
            axis: Vec3::Y,
            angle_rad: 0.0,
        };
        let uniform = |alpha, scale| StarUniform::new(rotation, alpha, scale).params;
        assert_eq!(uniform(3.0, 2.0)[..2], [1.0, 2.0 * STAR_RADIANCE]);
        assert_eq!(uniform(-1.0, -2.0)[..2], [0.0, 0.0]);
        assert_eq!(uniform(f32::NAN, f32::INFINITY)[..2], [0.0, 0.0]);
    }
}

mod star_geometry {
    use super::*;

    fn mesh(topology: Topology, positions: &[[f32; 3]], indices: &[u32]) -> CpuMesh {
        CpuMesh {
            vertices: positions
                .iter()
                .map(|&position| Vertex {
                    position,
                    tex_coords: [0.0, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    tangent: [0.0; 4],
                })
                .collect(),
            topology,
            indices: indices.to_vec(),
            texture_path: None,
            vertex_colors: None,
            effect: None,
            refraction: None,
        }
    }

    const TRIANGLE: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];

    #[test]
    fn parts_are_joined_with_their_indices_shifted_past_the_earlier_vertices() {
        let geometry = StarGeometry::from_meshes(
            "k",
            vec![
                mesh(Topology::Triangles, &TRIANGLE, &[0, 1, 2]),
                mesh(Topology::Triangles, &TRIANGLE, &[2, 1, 0]),
            ],
        )
        .unwrap();
        assert_eq!(geometry.vertices.len(), 6);
        assert_eq!(geometry.indices, [0, 1, 2, 5, 4, 3]);
    }

    #[test]
    fn point_sets_are_not_part_of_the_dome() {
        let geometry = StarGeometry::from_meshes(
            "k",
            vec![
                mesh(Topology::Points, &TRIANGLE, &[0, 1, 2]),
                mesh(Topology::Triangles, &TRIANGLE, &[0, 1, 2]),
            ],
        )
        .unwrap();
        assert_eq!(geometry.vertices.len(), 3);
        assert_eq!(geometry.indices, [0, 1, 2]);
    }

    #[test]
    fn a_model_without_triangles_is_an_error_naming_the_key() {
        let error = StarGeometry::from_meshes(
            "meshes\\sky\\stars.glb",
            vec![mesh(Topology::Points, &TRIANGLE, &[0, 1, 2])],
        )
        .unwrap_err();
        assert!(
            matches!(error, SkyAssetError::NoTriangles { ref key } if key.ends_with("stars.glb"))
        );
        assert!(matches!(
            StarGeometry::from_meshes("k", Vec::new()),
            Err(SkyAssetError::NoTriangles { .. })
        ));
    }

    #[test]
    fn an_index_past_the_vertices_is_an_error() {
        let error =
            StarGeometry::from_meshes("k", vec![mesh(Topology::Triangles, &TRIANGLE, &[0, 1, 3])])
                .unwrap_err();
        assert!(matches!(error, SkyAssetError::Mesh { .. }), "{error}");
    }

    /// A GLB holding one triangle with texture coordinates, at the identity transform.
    fn triangle_glb() -> Vec<u8> {
        let floats: Vec<u8> = TRIANGLE
            .iter()
            .flatten()
            .chain([0.0, 0.0, 2.0, 0.0, 0.0, 3.0].iter())
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let document: Value = json!({
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [{"mesh": 0}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "TEXCOORD_0": 1}}]}],
            "buffers": [{"byteLength": floats.len()}],
            "bufferViews": [
                {"buffer": 0, "byteOffset": 0, "byteLength": 36},
                {"buffer": 0, "byteOffset": 36, "byteLength": 24}
            ],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                 "min": [0, 0, 0], "max": [1, 1, 0]},
                {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC2"}
            ]
        });
        glb(&document, &floats)
    }

    #[test]
    fn the_dome_of_a_nif_path_loads_from_its_glb_whatever_the_stored_casing() {
        let root = install_with("SKY", "Stars.glb", &triangle_glb());

        let geometry = load_star_geometry(root.path().to_str().unwrap(), "sky\\stars.nif").unwrap();

        assert_eq!(geometry.indices, [0, 1, 2]);
        assert_eq!(geometry.vertices[1].tex_coords, [2.0, 0.0]);
        assert_eq!(geometry.vertices[2].tex_coords, [0.0, 3.0]);
    }

    #[test]
    fn a_missing_dome_is_a_not_found_error() {
        let root = install_with("sky", "other.glb", &triangle_glb());

        let error =
            load_star_geometry(root.path().to_str().unwrap(), "sky\\stars.nif").unwrap_err();

        assert!(matches!(error, SkyAssetError::NotFound { .. }), "{error}");
    }

    #[test]
    fn an_undecodable_dome_is_a_mesh_error() {
        let root = install_with("sky", "stars.glb", b"not a glb");

        let error =
            load_star_geometry(root.path().to_str().unwrap(), "sky\\stars.nif").unwrap_err();

        assert!(matches!(error, SkyAssetError::Mesh { .. }), "{error}");
    }
}

#[test]
fn nothing_is_requested_before_the_first_load() {
    let status = SkyAssetStatus::default();
    assert!(
        status
            .moon
            .iter()
            .all(|state| *state == AssetState::NotRequested)
    );
    assert_eq!(status.star_texture, AssetState::NotRequested);
    assert_eq!(status.star_mesh, AssetState::NotRequested);
}

mod shader_modules {
    use super::*;

    fn entry_points(source: &shaders::ShaderSource) -> Vec<(String, naga::ShaderStage)> {
        let module = naga::front::wgsl::parse_str(source.wgsl()).expect("the shader parses");
        module
            .entry_points
            .iter()
            .map(|entry| (entry.name.clone(), entry.stage))
            .collect()
    }

    #[test]
    fn the_moon_and_star_shaders_have_both_entry_points() {
        for source in [&shaders::SKY_MOON, &shaders::SKY_STARS] {
            let entries = entry_points(source);
            assert!(entries.contains(&("vs_main".to_owned(), naga::ShaderStage::Vertex)));
            assert!(entries.contains(&("fs_main".to_owned(), naga::ShaderStage::Fragment)));
        }
    }
}

pub(super) mod gpu {
    use super::*;

    pub(in super::super) const SIZE: UVec2 = UVec2::new(16, 16);
    const TEXTURE_EDGE: u32 = 16;
    pub(in super::super) const SKY_CLEAR: f32 = 0.0;

    /// What loading the texture `key` fails with in an install that lacks the file.
    pub(in super::super) fn missing_texture(key: &str) -> SkyAssetError {
        TextureAssetError::NotFound {
            key: key.to_owned(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        }
        .into()
    }

    pub(in super::super) struct Rig {
        pub(in super::super) device: wgpu::Device,
        pub(in super::super) queue: wgpu::Queue,
        material_layout: wgpu::BindGroupLayout,
        frame_layout: wgpu::BindGroupLayout,
        instance_layout: wgpu::BindGroupLayout,
        world_bind_groups: [wgpu::BindGroup; 3],
    }

    impl Rig {
        /// Stand-ins of the world pass's material, frame and instance group layouts.
        pub(in super::super) fn world_groups(&self) -> WorldGroupLayouts<'_> {
            [
                &self.material_layout,
                &self.frame_layout,
                &self.instance_layout,
            ]
        }

        /// Binds every world group, as the world pass does once up front.
        fn bind_world_groups(&self, pass: &mut wgpu::RenderPass<'_>) {
            for (index, group) in (0..).zip(&self.world_bind_groups) {
                pass.set_bind_group(index, group, &[]);
            }
        }
    }

    fn uniform_layout(device: &wgpu::Device, label: &str) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        })
    }

    fn uniform_group(
        device: &wgpu::Device,
        label: &str,
        layout: &wgpu::BindGroupLayout,
        contents: &[u8],
    ) -> wgpu::BindGroup {
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        })
    }

    /// A draw of a pipeline with the world pass's layout, as the game issues before the sky. It
    /// covers no pixel.
    pub(in super::super) struct WorldDraw {
        pipeline: wgpu::RenderPipeline,
    }

    impl WorldDraw {
        pub(in super::super) fn new(rig: &Rig) -> Self {
            let device = &rig.device;
            let [material, frame, instance] = rig.world_groups().map(Some);
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("night_sky_test_world_layout"),
                bind_group_layouts: &[material, frame, instance],
                immediate_size: 0,
            });
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("night_sky_test_world_shader"),
                source: wgpu::ShaderSource::Wgsl(WORLD_DRAW_SHADER.into()),
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("night_sky_test_world_pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(HDR_FORMAT.into())],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: Texture::DEPTH_FORMAT,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
            Self { pipeline }
        }

        pub(in super::super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
            pass.set_pipeline(&self.pipeline);
            pass.draw(0..3, 0..1);
        }
    }

    const WORLD_DRAW_SHADER: &str = "
        @group(0) @binding(0) var<uniform> material: vec4<f32>;
        @group(1) @binding(0) var<uniform> frame: vec4<f32>;
        @group(2) @binding(0) var<uniform> instance: vec4<f32>;
        @vertex fn vs_main() -> @builtin(position) vec4<f32> {
            return (material + frame + instance) * 0.0;
        }
        @fragment fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(0.0); }
    ";

    /// A camera at the origin looking down -Z with a 90 degree view and the infinite reverse-Z
    /// projection of the game.
    pub(in super::super) fn frame_uniform() -> FrameUniform {
        let mut uniform: FrameUniform = bytemuck::Zeroable::zeroed();
        uniform.view_proj = Mat4::perspective_infinite_reverse_rh(FRAC_PI_2, 1.0, 0.1);
        uniform
    }

    pub(in super::super) fn rig(test: &str) -> Option<Rig> {
        rig_with_frame(test, &frame_uniform())
    }

    /// A rig whose frame group holds `frame`, for layers that read more of it than the
    /// projection.
    pub(in super::super) fn rig_with_frame(test: &str, frame: &FrameUniform) -> Option<Rig> {
        let (device, queue) = test_device(test)?;
        let material_layout = uniform_layout(&device, "night_sky_test_material_bgl");
        let frame_layout = uniform_layout(&device, "night_sky_test_frame_bgl");
        let instance_layout = uniform_layout(&device, "night_sky_test_instance_bgl");
        let unused = [0; 16];
        let world_bind_groups = [
            uniform_group(
                &device,
                "night_sky_test_material",
                &material_layout,
                &unused,
            ),
            uniform_group(
                &device,
                "night_sky_test_frame",
                &frame_layout,
                bytemuck::bytes_of(frame),
            ),
            uniform_group(
                &device,
                "night_sky_test_instance",
                &instance_layout,
                &unused,
            ),
        ];
        Some(Rig {
            device,
            queue,
            material_layout,
            frame_layout,
            instance_layout,
            world_bind_groups,
        })
    }

    /// A bind group of the shape of the world pass texture feedback group.
    pub(in super::super) fn feedback_group(rig: &Rig) -> wgpu::BindGroup {
        let layout = rig
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("night_sky_test_feedback_bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let buffer = rig.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("night_sky_test_feedback"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        rig.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("night_sky_test_feedback_group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        })
    }

    pub(in super::super) fn white_texture(rig: &Rig, edge: u32) -> wgpu::TextureView {
        pattern_texture(rig, edge, |_, _| [255; 4])
    }

    /// A square sRGB texture whose texel at (column, row) is `texel`.
    pub(in super::super) fn pattern_texture(
        rig: &Rig,
        edge: u32,
        texel: impl Fn(u32, u32) -> [u8; 4],
    ) -> wgpu::TextureView {
        let texels: Vec<u8> = (0..edge)
            .flat_map(|row| (0..edge).map(move |column| (column, row)))
            .flat_map(|(column, row)| texel(column, row))
            .collect();
        let texture = rig.device.create_texture_with_data(
            &rig.queue,
            &wgpu::TextureDescriptor {
                label: Some("night_sky_test_white"),
                size: wgpu::Extent3d {
                    width: edge,
                    height: edge,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &texels,
        );
        view_of(&texture)
    }

    /// Draws into a black target whose depth is `depth` and returns its texels, row by row.
    pub(in super::super) fn render(
        rig: &Rig,
        depth: f32,
        draw: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) -> Vec<Vec4> {
        let target = rig.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("night_sky_test_target"),
            size: wgpu::Extent3d {
                width: SIZE.x,
                height: SIZE.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth_texture = rig.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("night_sky_test_depth"),
            size: wgpu::Extent3d {
                width: SIZE.x,
                height: SIZE.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Texture::DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let target_view = view_of(&target);
        let depth_view = view_of(&depth_texture);
        let mut encoder = rig
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("night_sky_test_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(depth),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rig.bind_world_groups(&mut pass);
            draw(&mut pass);
        }
        rig.queue.submit([encoder.finish()]);
        read_hdr(&rig.device, &rig.queue, &target, SIZE)
    }

    pub(in super::super) fn texel(texels: &[Vec4], column: u32, row: u32) -> Vec4 {
        texels[(row * SIZE.x + column) as usize]
    }

    fn moon_layer(rig: &Rig, phase_textures: PhaseTextures) -> (MoonLayer, [AssetState; 8]) {
        let mut layer = MoonLayer::new(&rig.device, rig.world_groups());
        let states = layer.install(&rig.device, MoonKind::Masser, phase_textures);
        (layer, states)
    }

    fn all_phases(view: &wgpu::TextureView) -> PhaseTextures {
        std::array::from_fn(|_| Ok(view.clone()))
    }

    const MOON_PHASE: MoonPhase = MoonPhase::Full;

    fn moon_pixels(
        rig: &Rig,
        layer: &mut MoonLayer,
        visibility: f32,
        scale: f32,
        depth: f32,
    ) -> Vec<Vec4> {
        let uniform = MoonUniform::new(Vec3::NEG_Z, 30.0, visibility, scale);
        layer.update(&rig.queue, MOON_PHASE, uniform);
        render(rig, depth, |pass| layer.draw(pass))
    }

    const CENTER: (u32, u32) = (8, 8);
    const CORNER: (u32, u32) = (0, 0);

    #[test]
    fn the_moon_covers_the_pixels_around_its_direction_and_nothing_else() {
        let Some(rig) = rig("the_moon_covers_the_pixels_around_its_direction_and_nothing_else")
        else {
            return;
        };
        let (mut layer, _) = moon_layer(&rig, all_phases(&white_texture(&rig, TEXTURE_EDGE)));

        let pixels = moon_pixels(&rig, &mut layer, 1.0, 1.0, SKY_CLEAR);

        let center = texel(&pixels, CENTER.0, CENTER.1);
        assert!(center.truncate().min_element() > 0.0, "{center}");
        assert_eq!(center.w, 1.0, "the target alpha is left alone");
        assert_eq!(
            texel(&pixels, CORNER.0, CORNER.1),
            Vec4::new(0.0, 0.0, 0.0, 1.0)
        );
    }

    #[test]
    fn the_moon_scales_with_the_sky_radiance_and_blends_by_visibility() {
        let Some(rig) = rig("the_moon_scales_with_the_sky_radiance_and_blends_by_visibility")
        else {
            return;
        };
        let (mut layer, _) = moon_layer(&rig, all_phases(&white_texture(&rig, TEXTURE_EDGE)));
        let bright = texel(&moon_pixels(&rig, &mut layer, 1.0, 1.0, SKY_CLEAR), 8, 8).x;

        let doubled = texel(&moon_pixels(&rig, &mut layer, 1.0, 2.0, SKY_CLEAR), 8, 8).x;
        let half_visible = texel(&moon_pixels(&rig, &mut layer, 0.5, 1.0, SKY_CLEAR), 8, 8).x;

        assert!(bright > 0.0);
        assert_eq!(doubled, bright * 2.0);
        assert_eq!(half_visible, bright * 0.5);
    }

    #[test]
    fn the_moon_hides_behind_geometry_that_wrote_depth() {
        let Some(rig) = rig("the_moon_hides_behind_geometry_that_wrote_depth") else {
            return;
        };
        let (mut layer, _) = moon_layer(&rig, all_phases(&white_texture(&rig, TEXTURE_EDGE)));

        let pixels = moon_pixels(&rig, &mut layer, 1.0, 1.0, 0.5);

        assert!(
            pixels
                .iter()
                .all(|texel| *texel == Vec4::new(0.0, 0.0, 0.0, 1.0))
        );
    }

    #[test]
    fn a_moon_behind_the_camera_draws_nothing() {
        let Some(rig) = rig("a_moon_behind_the_camera_draws_nothing") else {
            return;
        };
        let (mut layer, _) = moon_layer(&rig, all_phases(&white_texture(&rig, TEXTURE_EDGE)));
        let uniform = MoonUniform::new(Vec3::Z, 30.0, 1.0, 1.0);
        layer.update(&rig.queue, MOON_PHASE, uniform);

        let pixels = render(&rig, SKY_CLEAR, |pass| layer.draw(pass));

        assert!(
            pixels
                .iter()
                .all(|texel| *texel == Vec4::new(0.0, 0.0, 0.0, 1.0))
        );
    }

    #[test]
    fn a_phase_without_a_usable_texture_is_not_drawn_and_says_why() {
        let Some(rig) = rig("a_phase_without_a_usable_texture_is_not_drawn_and_says_why") else {
            return;
        };
        let white = white_texture(&rig, TEXTURE_EDGE);
        let textures: PhaseTextures = std::array::from_fn(|index| match index {
            i if i == MOON_PHASE.index() => Err(missing_texture("textures\\sky\\masser_full.dds")),
            i if i == MoonPhase::New.index() => Ok(white_texture(&rig, 8)),
            _ => Ok(white.clone()),
        });
        let (mut layer, states) = moon_layer(&rig, textures);

        let pixels = moon_pixels(&rig, &mut layer, 1.0, 1.0, SKY_CLEAR);

        assert!(
            pixels
                .iter()
                .all(|texel| *texel == Vec4::new(0.0, 0.0, 0.0, 1.0))
        );
        assert!(matches!(
            &states[MOON_PHASE.index()],
            AssetState::Failed(failure) if failure.message.contains("masser_full.dds")
        ));
        assert_eq!(states[MoonPhase::New.index()], AssetState::Placeholder);
        assert_eq!(states[MoonPhase::HalfWan.index()], AssetState::Loaded);
    }

    #[test]
    fn a_stand_in_phase_is_not_drawn() {
        let Some(rig) = rig("a_stand_in_phase_is_not_drawn") else {
            return;
        };
        let (mut layer, _) = moon_layer(&rig, all_phases(&white_texture(&rig, 8)));

        let pixels = moon_pixels(&rig, &mut layer, 1.0, 1.0, SKY_CLEAR);

        assert!(
            pixels
                .iter()
                .all(|texel| *texel == Vec4::new(0.0, 0.0, 0.0, 1.0))
        );
    }

    /// A triangle on the z = -1 plane that covers the whole 90 degree view.
    fn covering_dome() -> StarGeometry {
        let vertex = |position: [f32; 3]| Vertex {
            position,
            tex_coords: [0.5, 0.5],
            normal: [0.0, 1.0, 0.0],
            tangent: [0.0; 4],
        };
        StarGeometry {
            vertices: vec![
                vertex([-4.0, -4.0, -1.0]),
                vertex([4.0, -4.0, -1.0]),
                vertex([0.0, 4.0, -1.0]),
            ],
            indices: vec![0, 1, 2],
        }
    }

    pub(in super::super) fn star_layer(rig: &Rig) -> (StarLayer, [AssetState; 2]) {
        let mut layer = StarLayer::new(&rig.device, rig.world_groups());
        let texture = layer.install_texture(&rig.device, Ok(white_texture(rig, TEXTURE_EDGE)));
        let mesh = layer.install_mesh(&rig.device, Some(Ok(covering_dome())));
        (layer, [texture, mesh])
    }

    const UPPER_ROW: u32 = 2;
    const LOWER_ROW: u32 = 13;

    fn star_pixels(rig: &Rig, layer: &mut StarLayer, uniform: &StarUniform) -> Vec<Vec4> {
        layer.update(&rig.queue, uniform);
        render(rig, SKY_CLEAR, |pass| layer.draw(pass))
    }

    pub(in super::super) fn unturned(alpha: f32, scale: f32) -> StarUniform {
        StarUniform::new(
            StarRotation {
                axis: Vec3::Y,
                angle_rad: 0.0,
            },
            alpha,
            scale,
        )
    }

    #[test]
    fn stars_show_above_the_horizon_only() {
        let Some(rig) = rig("stars_show_above_the_horizon_only") else {
            return;
        };
        let (mut layer, states) = star_layer(&rig);
        assert_eq!(states, [AssetState::Loaded, AssetState::Loaded]);

        let pixels = star_pixels(&rig, &mut layer, &unturned(1.0, 1.0));

        assert!(texel(&pixels, 8, UPPER_ROW).truncate().min_element() > 0.0);
        assert_eq!(
            texel(&pixels, 8, UPPER_ROW).w,
            1.0,
            "the target alpha is left alone"
        );
        assert_eq!(texel(&pixels, 8, LOWER_ROW), Vec4::new(0.0, 0.0, 0.0, 1.0));
    }

    #[test]
    fn stars_scale_with_their_opacity_and_the_sky_radiance() {
        let Some(rig) = rig("stars_scale_with_their_opacity_and_the_sky_radiance") else {
            return;
        };
        let (mut layer, _) = star_layer(&rig);
        let full = texel(
            &star_pixels(&rig, &mut layer, &unturned(1.0, 1.0)),
            8,
            UPPER_ROW,
        )
        .x;

        let half = texel(
            &star_pixels(&rig, &mut layer, &unturned(0.5, 1.0)),
            8,
            UPPER_ROW,
        )
        .x;
        let doubled = texel(
            &star_pixels(&rig, &mut layer, &unturned(1.0, 2.0)),
            8,
            UPPER_ROW,
        )
        .x;

        assert!(full > 0.0);
        assert_eq!(half, full * 0.5);
        assert_eq!(doubled, full * 2.0);
    }

    #[test]
    fn fully_transparent_stars_draw_nothing() {
        let Some(rig) = rig("fully_transparent_stars_draw_nothing") else {
            return;
        };
        let (mut layer, _) = star_layer(&rig);

        let pixels = star_pixels(&rig, &mut layer, &unturned(0.0, 1.0));

        assert!(
            pixels
                .iter()
                .all(|texel| *texel == Vec4::new(0.0, 0.0, 0.0, 1.0))
        );
    }

    #[test]
    fn the_dome_turns_with_the_star_rotation() {
        let Some(rig) = rig("the_dome_turns_with_the_star_rotation") else {
            return;
        };
        let (mut layer, _) = star_layer(&rig);
        let behind_the_camera = StarUniform::new(
            StarRotation {
                axis: Vec3::Y,
                angle_rad: std::f32::consts::PI,
            },
            1.0,
            1.0,
        );

        let pixels = star_pixels(&rig, &mut layer, &behind_the_camera);

        assert!(
            pixels
                .iter()
                .all(|texel| *texel == Vec4::new(0.0, 0.0, 0.0, 1.0))
        );
    }

    #[test]
    fn stars_hide_behind_geometry_that_wrote_depth() {
        let Some(rig) = rig("stars_hide_behind_geometry_that_wrote_depth") else {
            return;
        };
        let (mut layer, _) = star_layer(&rig);
        layer.update(&rig.queue, &unturned(1.0, 1.0));

        let pixels = render(&rig, 0.5, |pass| layer.draw(pass));

        assert!(
            pixels
                .iter()
                .all(|texel| *texel == Vec4::new(0.0, 0.0, 0.0, 1.0))
        );
    }

    #[test]
    fn stars_without_a_model_or_a_texture_are_not_drawn_and_say_why() {
        let Some(rig) = rig("stars_without_a_model_or_a_texture_are_not_drawn_and_say_why") else {
            return;
        };
        let mut no_model = StarLayer::new(&rig.device, rig.world_groups());
        no_model.install_texture(&rig.device, Ok(white_texture(&rig, TEXTURE_EDGE)));
        assert_eq!(
            no_model.install_mesh(&rig.device, None),
            AssetState::NotRequested
        );
        let mut no_texture = StarLayer::new(&rig.device, rig.world_groups());
        let state = no_texture.install_texture(
            &rig.device,
            Err(missing_texture("textures\\sky\\skystars.dds")),
        );
        no_texture.install_mesh(&rig.device, Some(Ok(covering_dome())));

        for layer in [&mut no_model, &mut no_texture] {
            let pixels = star_pixels(&rig, layer, &unturned(1.0, 1.0));
            assert!(
                pixels
                    .iter()
                    .all(|texel| *texel == Vec4::new(0.0, 0.0, 0.0, 1.0))
            );
        }
        assert!(
            matches!(state, AssetState::Failed(failure) if failure.message.contains("skystars.dds"))
        );
    }

    fn quadrant_texture(rig: &Rig) -> wgpu::TextureView {
        pattern_texture(rig, TEXTURE_EDGE, |column, row| {
            let lit = column < TEXTURE_EDGE / 2 && row < TEXTURE_EDGE / 2;
            if lit { [255; 4] } else { [0, 0, 0, 255] }
        })
    }

    #[test]
    fn the_moon_texture_appears_upright_and_unmirrored() {
        let Some(rig) = rig("the_moon_texture_appears_upright_and_unmirrored") else {
            return;
        };
        let (mut layer, _) = moon_layer(&rig, all_phases(&quadrant_texture(&rig)));
        let uniform = MoonUniform::new(Vec3::NEG_Z, 60.0, 1.0, 1.0);
        layer.update(&rig.queue, MOON_PHASE, uniform);

        let pixels = render(&rig, SKY_CLEAR, |pass| layer.draw(pass));

        let at = |column, row| texel(&pixels, column, row).x;
        assert!(
            at(6, 6) > 0.0,
            "the lit top left quadrant is at the top left"
        );
        assert_eq!(at(10, 6), 0.0, "top right");
        assert_eq!(at(6, 10), 0.0, "bottom left");
        assert_eq!(at(10, 10), 0.0, "bottom right");
    }

    #[test]
    fn the_moon_texture_alpha_scales_what_is_drawn() {
        let Some(rig) = rig("the_moon_texture_alpha_scales_what_is_drawn") else {
            return;
        };
        let translucent = pattern_texture(&rig, TEXTURE_EDGE, |_, _| [255, 255, 255, HALF_ALPHA]);
        let (mut opaque_layer, _) =
            moon_layer(&rig, all_phases(&white_texture(&rig, TEXTURE_EDGE)));
        let (mut layer, _) = moon_layer(&rig, all_phases(&translucent));
        let opaque = texel(
            &moon_pixels(&rig, &mut opaque_layer, 1.0, 1.0, SKY_CLEAR),
            8,
            8,
        )
        .x;

        let shown = texel(&moon_pixels(&rig, &mut layer, 1.0, 1.0, SKY_CLEAR), 8, 8).x;

        let expected = opaque * f32::from(HALF_ALPHA) / 255.0;
        assert!(
            (shown - expected).abs() < expected * 0.01,
            "{shown} vs {expected}"
        );
    }

    const HALF_ALPHA: u8 = 128;

    #[test]
    fn star_texture_alpha_scales_what_is_drawn() {
        let Some(rig) = rig("star_texture_alpha_scales_what_is_drawn") else {
            return;
        };
        let (mut opaque_layer, _) = star_layer(&rig);
        let translucent = pattern_texture(&rig, TEXTURE_EDGE, |_, _| [255, 255, 255, HALF_ALPHA]);
        let mut layer = StarLayer::new(&rig.device, rig.world_groups());
        layer.install_texture(&rig.device, Ok(translucent));
        layer.install_mesh(&rig.device, Some(Ok(covering_dome())));
        let uniform = unturned(1.0, 1.0);
        let opaque = texel(
            &star_pixels(&rig, &mut opaque_layer, &uniform),
            8,
            UPPER_ROW,
        )
        .x;

        let shown = texel(&star_pixels(&rig, &mut layer, &uniform), 8, UPPER_ROW).x;

        let expected = opaque * f32::from(HALF_ALPHA) / 255.0;
        assert!(
            (shown - expected).abs() < expected * 0.01,
            "{shown} vs {expected}"
        );
    }

    #[test]
    fn the_horizon_fade_is_a_ramp_whose_width_the_uniform_sets() {
        let Some(rig) = rig("the_horizon_fade_is_a_ramp_whose_width_the_uniform_sets") else {
            return;
        };
        let (mut layer, _) = star_layer(&rig);
        let mut wide = unturned(1.0, 1.0);
        wide.params[2] = 1.0;

        let ramp = star_pixels(&rig, &mut layer, &wide);
        let narrow = star_pixels(&rig, &mut layer, &unturned(1.0, 1.0));

        let ramp_at = |row| texel(&ramp, 8, row).x;
        let rising: Vec<f32> = (2..=HORIZON_ROW).rev().map(ramp_at).collect();
        assert!(ramp_at(HORIZON_ROW) > 0.0, "just above the horizon");
        assert_eq!(ramp_at(HORIZON_ROW + 1), 0.0, "just below the horizon");
        assert!(
            rising.windows(2).all(|pair| pair[0] < pair[1]),
            "the stars brighten with height: {rising:?}"
        );
        assert!(
            texel(&narrow, 8, HORIZON_ROW).x > ramp_at(HORIZON_ROW),
            "the default ramp is narrower than the wide one"
        );
        assert_eq!(
            texel(&narrow, 8, HORIZON_ROW).x,
            texel(&narrow, 8, UPPER_ROW).x
        );
    }

    const HORIZON_ROW: u32 = 7;

    /// A small triangle around the direction (1, 0.5, 0): due +X and above the horizon.
    fn off_axis_star() -> StarGeometry {
        let vertex = |position: [f32; 3]| Vertex {
            position,
            tex_coords: [0.5, 0.5],
            normal: [0.0, 1.0, 0.0],
            tangent: [0.0; 4],
        };
        StarGeometry {
            vertices: vec![
                vertex([1.0, 0.1, -0.8]),
                vertex([1.0, 0.9, -0.8]),
                vertex([1.0, 0.5, 0.8]),
            ],
            indices: vec![0, 1, 2],
        }
    }

    fn turned_about_up(rig: &Rig, angle_rad: f32) -> Vec<Vec4> {
        let mut layer = StarLayer::new(&rig.device, rig.world_groups());
        layer.install_texture(&rig.device, Ok(white_texture(rig, TEXTURE_EDGE)));
        layer.install_mesh(&rig.device, Some(Ok(off_axis_star())));
        let rotation = StarRotation {
            axis: Vec3::Y,
            angle_rad,
        };
        star_pixels(rig, &mut layer, &StarUniform::new(rotation, 1.0, 1.0))
    }

    #[test]
    fn a_right_handed_quarter_turn_carries_a_star_from_plus_x_to_the_view_axis() {
        let Some(rig) =
            rig("a_right_handed_quarter_turn_carries_a_star_from_plus_x_to_the_view_axis")
        else {
            return;
        };
        let black = Vec4::new(0.0, 0.0, 0.0, 1.0);

        let unturned = turned_about_up(&rig, 0.0);
        let forwards = turned_about_up(&rig, FRAC_PI_2);
        let backwards = turned_about_up(&rig, -FRAC_PI_2);

        assert!(
            unturned.iter().all(|texel| *texel == black),
            "off to the side"
        );
        assert!(texel(&forwards, 8, 4).x > 0.0, "+X went to -Z, in front");
        assert_eq!(texel(&forwards, 8, 11), black, "nothing below the star");
        assert!(
            backwards.iter().all(|texel| *texel == black),
            "+X went to +Z, behind"
        );
    }

    fn night_sky_input(moon: Option<MoonInput>, sky_model: Option<Arc<str>>) -> NightSkyInput {
        NightSkyInput {
            moon,
            star_rotation: StarRotation {
                axis: Vec3::Y,
                angle_rad: 0.0,
            },
            star_alpha: 1.0,
            sky_model,
            sun: sun_input(),
            sun_textures: ClimateSunTextures::default(),
        }
    }

    fn moon_input(kind: MoonKind) -> MoonInput {
        MoonInput {
            kind,
            phase: MoonPhase::Full,
            direction: Vec3::NEG_Z,
            visibility: 1.0,
        }
    }

    fn model() -> Arc<str> {
        Arc::from("sky\\stars.nif")
    }

    fn new_sky(
        rig: &Rig,
        root: &str,
        moon: Option<MoonKind>,
        sky_model: Option<&Arc<str>>,
    ) -> NightSky {
        NightSky::new(
            &rig.device,
            &rig.queue,
            root,
            rig.world_groups(),
            SkySizes::VANILLA,
            moon,
            sky_model,
        )
    }

    #[test]
    fn a_night_sky_without_an_install_reports_every_asset_failed_and_draws_nothing() {
        let Some(rig) =
            rig("a_night_sky_without_an_install_reports_every_asset_failed_and_draws_nothing")
        else {
            return;
        };
        let root = tempfile::tempdir().unwrap();
        let model = model();
        let mut sky = new_sky(
            &rig,
            root.path().to_str().unwrap(),
            Some(MoonKind::Masser),
            Some(&model),
        );
        let input = night_sky_input(Some(moon_input(MoonKind::Masser)), Some(model));

        sky.update(&rig.device, &rig.queue, &input, 1.0);
        let feedback = feedback_group(&rig);
        let pixels = render(&rig, SKY_CLEAR, |pass| sky.draw(pass, &feedback));

        let status = sky.asset_status();
        let not_found = |state: &AssetState| matches!(state, AssetState::Failed(failure) if failure.kind == AssetFailureKind::NotFound);
        assert!(status.moon.iter().all(not_found));
        assert!(not_found(&status.star_texture));
        assert!(not_found(&status.star_mesh));
        assert!(
            pixels
                .iter()
                .all(|texel| *texel == Vec4::new(0.0, 0.0, 0.0, 1.0))
        );
    }

    #[test]
    fn a_night_sky_without_a_moon_or_a_sky_model_loads_neither() {
        let Some(rig) = rig("a_night_sky_without_a_moon_or_a_sky_model_loads_neither") else {
            return;
        };
        let root = tempfile::tempdir().unwrap();
        let mut sky = new_sky(&rig, root.path().to_str().unwrap(), None, None);

        sky.update(&rig.device, &rig.queue, &night_sky_input(None, None), 1.0);

        let status = sky.asset_status();
        assert!(
            status
                .moon
                .iter()
                .all(|state| *state == AssetState::NotRequested)
        );
        assert_eq!(status.star_mesh, AssetState::NotRequested);
    }

    #[test]
    fn a_later_moon_kind_loads_when_it_first_appears() {
        let Some(rig) = rig("a_later_moon_kind_loads_when_it_first_appears") else {
            return;
        };
        let root = tempfile::tempdir().unwrap();
        let mut sky = new_sky(&rig, root.path().to_str().unwrap(), None, None);
        let input = night_sky_input(Some(moon_input(MoonKind::Secunda)), None);

        sky.update(&rig.device, &rig.queue, &input, 1.0);

        assert!(
            sky.asset_status()
                .moon
                .iter()
                .all(|state| matches!(state, AssetState::Failed(_)))
        );
    }
}

mod error_typing {
    use super::*;

    #[test]
    fn an_error_names_its_kind_in_the_state_it_becomes() {
        let io = || std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let errors = [
            SkyAssetError::NotFound {
                key: "a".to_owned(),
                source: io(),
            },
            SkyAssetError::Unreadable {
                key: "a".to_owned(),
                source: io(),
            },
            SkyAssetError::Mesh {
                key: "a".to_owned(),
                reason: String::new(),
            },
            SkyAssetError::NoTriangles {
                key: "a".to_owned(),
            },
            TextureAssetError::NotFound {
                key: "a".to_owned(),
                source: io(),
            }
            .into(),
            TextureAssetError::Unreadable {
                key: "a".to_owned(),
                source: io(),
            }
            .into(),
            TextureAssetError::Unusable {
                key: "a".to_owned(),
                source: TextureError::BadMagic {
                    path: "a".to_owned(),
                },
            }
            .into(),
        ];
        let kinds = errors.map(|error| match AssetState::failed(&error) {
            AssetState::Failed(failure) => failure.kind,
            other => panic!("{other:?}"),
        });
        assert_eq!(
            kinds,
            [
                AssetFailureKind::NotFound,
                AssetFailureKind::Unreadable,
                AssetFailureKind::Mesh,
                AssetFailureKind::NoTriangles,
                AssetFailureKind::NotFound,
                AssetFailureKind::Unreadable,
                AssetFailureKind::Texture,
            ]
        );
    }

    #[test]
    fn a_path_through_a_file_is_unreadable_not_missing() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("meshes"), b"a file where a folder belongs").unwrap();

        let error =
            load_star_geometry(root.path().to_str().unwrap(), "sky\\stars.nif").unwrap_err();

        assert!(matches!(error, SkyAssetError::Unreadable { .. }), "{error}");
    }
}

mod sky_objects_mapping {
    use super::*;
    use crate::rendering::lighting::time_of_day::MoonObject;

    #[test]
    fn the_input_mirrors_the_sky_objects_and_carries_the_sun() {
        let objects = SkyObjects {
            sun_disc_direction: Vec3::Y,
            moon: Some(MoonObject {
                kind: MoonKind::Secunda,
                phase: MoonPhase::HalfWax,
                direction: Vec3::X,
                visibility: 0.25,
            }),
            star_rotation: StarRotation {
                axis: Vec3::Z,
                angle_rad: 1.5,
            },
            star_alpha: 0.75,
            sky_model_path: Some(Arc::from("sky\\stars.nif")),
            sun_textures: ClimateSunTextures::new(Some("sky\\sun.dds"), None),
        };

        let input = NightSkyInput::new(&objects, sun_input());

        assert_eq!(
            input.moon,
            Some(MoonInput {
                kind: MoonKind::Secunda,
                phase: MoonPhase::HalfWax,
                direction: Vec3::X,
                visibility: 0.25,
            })
        );
        assert_eq!(input.star_rotation, objects.star_rotation);
        assert_eq!(input.star_alpha, 0.75);
        assert_eq!(input.sky_model, objects.sky_model_path);
        assert_eq!(input.sun, sun_input());
        assert_eq!(input.sun_textures, objects.sun_textures);
        let moonless = SkyObjects {
            moon: None,
            ..objects
        };
        assert_eq!(NightSkyInput::new(&moonless, sun_input()).moon, None);
    }
}

/// Developer checks against a real install (`OPENFO4_INSTALL_DIR`), run with `--ignored`.
mod real_install {
    use super::*;

    fn asset_root() -> String {
        let install = std::env::var(crate::assets::INSTALL_DIR_VAR).expect("OPENFO4_INSTALL_DIR");
        format!("{install}/{}", crate::assets::layout::ASSETS_DIR)
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the converted sky meshes"]
    fn the_installed_star_dome_is_a_triangle_mesh_with_repeating_coordinates() {
        let geometry = load_star_geometry(&asset_root(), "sky\\stars.nif").unwrap();

        assert!(!geometry.vertices.is_empty());
        assert_eq!(geometry.indices.len() % 3, 0);
        let count = geometry.vertices.len() as u32;
        assert!(geometry.indices.iter().all(|&index| index < count));
        let beyond_unit = geometry
            .vertices
            .iter()
            .any(|vertex| vertex.tex_coords.iter().any(|uv| !(0.0..=1.0).contains(uv)));
        assert!(beyond_unit, "the dome relies on a repeating sampler");
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the converted sky textures"]
    fn only_the_installed_masser_new_moon_is_a_placeholder() {
        let name = "only_the_installed_masser_new_moon_is_a_placeholder";
        let Some(rig) = super::gpu::rig(name) else {
            return;
        };
        let model: Arc<str> = Arc::from("sky\\stars.nif");
        let mut sky = NightSky::new(
            &rig.device,
            &rig.queue,
            &asset_root(),
            rig.world_groups(),
            SkySizes::VANILLA,
            Some(MoonKind::Masser),
            Some(&model),
        );
        let masser = sky.asset_status().clone();
        let input = NightSkyInput {
            moon: Some(MoonInput {
                kind: MoonKind::Secunda,
                phase: MoonPhase::Full,
                direction: Vec3::Y,
                visibility: 1.0,
            }),
            star_rotation: StarRotation {
                axis: Vec3::Y,
                angle_rad: 0.0,
            },
            star_alpha: 1.0,
            sky_model: Some(model),
            sun: sun_input(),
            sun_textures: ClimateSunTextures::default(),
        };

        sky.update(&rig.device, &rig.queue, &input, 1.0);
        let secunda = sky.asset_status().clone();

        for phase in MoonPhase::ALL {
            let expected = if phase == MoonPhase::New {
                AssetState::Placeholder
            } else {
                AssetState::Loaded
            };
            assert_eq!(masser.moon[phase.index()], expected, "Masser {phase:?}");
            assert_eq!(
                secunda.moon[phase.index()],
                AssetState::Loaded,
                "Secunda {phase:?}"
            );
        }
        assert_eq!(secunda.star_texture, AssetState::Loaded);
        assert_eq!(secunda.star_mesh, AssetState::Loaded);
    }
}
