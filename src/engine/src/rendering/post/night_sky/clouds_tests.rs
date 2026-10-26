use std::ops::Range;
use std::sync::Arc;

use glam::{Vec2, Vec3, Vec4};
use serde_json::{Value, json};

use super::assets::{AssetFailureKind, AssetState, SkyAssetError, load_texture, named_texture_key};
use super::cloud_geometry::{CloudGeometry, CloudVertex, DEFAULT_MODEL, load_cloud_geometry};
use super::cloud_scroll::{ScrollOffsets, wrap_scroll};
use super::clouds::{CloudLayer, CloudLayerDraw, CloudUniform, CloudsInput, MAX_DRAWS, plan_draws};
use super::tests::gpu::{
    Rig, SKY_CLEAR, WorldDraw, feedback_group, frame_uniform, pattern_texture, render, rig,
    rig_with_frame, star_layer, texel, unturned, white_texture,
};
use super::tests::{glb, install_with};
use super::{NightSky, NightSkyInput, SkySizes, SunInput};
use crate::assets::types::mesh::{CpuMesh, Topology};
use crate::rendering::lighting::time_of_day::unmeasured::{
    CLOUD_HORIZON_HAZE_SINE, CLOUD_SCROLL_SCALE,
};
use crate::rendering::lighting::time_of_day::{ClimateSunTextures, StarRotation};
use crate::rendering::shaders;
use crate::rendering::vertex::Vertex;

const TEXTURE: &str = "Sky\\Test_d.dds";

fn layer_draw(index: u8, alpha: f32) -> CloudLayerDraw {
    CloudLayerDraw {
        layer: index,
        texture: Arc::from(TEXTURE),
        color: Vec3::ONE,
        alpha,
        scroll_speed: Vec2::ZERO,
    }
}

fn input(layers: Vec<CloudLayerDraw>) -> CloudsInput {
    CloudsInput {
        layers,
        ..CloudsInput::default()
    }
}

fn new_sky(rig: &Rig, root: &str) -> NightSky {
    NightSky::new(
        &rig.device,
        &rig.queue,
        root,
        rig.world_groups(),
        SkySizes::VANILLA,
        None,
        None,
    )
}

/// The pixels of the whole sky drawn after the clouds of `clouds` are readied.
fn night_sky_frame(rig: &Rig, sky: &mut NightSky, clouds: &CloudsInput) -> Vec<Vec4> {
    sky.update_clouds(&rig.device, &rig.queue, clouds, 1.0, 0.0);
    let feedback = feedback_group(rig);
    render(rig, SKY_CLEAR, |pass| sky.draw(pass, &feedback))
}

mod scroll {
    use super::*;
    use proptest::prelude::*;

    fn assert_close(actual: Vec2, expected: Vec2) {
        assert!(
            actual.abs_diff_eq(expected, 1e-5),
            "{actual} is not {expected}"
        );
    }

    #[test]
    fn the_offset_moves_by_the_velocity_over_the_time_step() {
        let moved = wrap_scroll(Vec2::new(0.25, 0.5), Vec2::new(0.1, -0.2), 2.0);

        assert_close(moved, Vec2::new(0.45, 0.1));
    }

    #[test]
    fn the_offset_wraps_in_both_directions_on_both_axes() {
        assert_close(
            wrap_scroll(Vec2::new(0.9, 0.1), Vec2::new(0.2, -0.2), 1.0),
            Vec2::new(0.1, 0.9),
        );
        assert_close(
            wrap_scroll(Vec2::ZERO, Vec2::new(3.25, -4.25), 1.0),
            Vec2::new(0.25, 0.75),
        );
    }

    #[test]
    fn a_whole_turn_lands_on_zero_not_on_one() {
        let moved = wrap_scroll(Vec2::new(0.5, 0.0), Vec2::new(0.5, 1.0), 1.0);

        assert_eq!(moved, Vec2::ZERO);
    }

    #[test]
    fn a_backward_step_too_small_for_the_float_stays_below_one() {
        let moved = wrap_scroll(Vec2::ZERO, Vec2::splat(-1e-9), 1.0);

        assert!((0.0..1.0).contains(&moved.x), "{moved}");
        assert!((0.0..1.0).contains(&moved.y), "{moved}");
    }

    #[test]
    fn a_time_step_that_is_negative_or_not_a_number_moves_nothing() {
        let offset = Vec2::new(0.3, 0.6);
        for dt in [-1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(wrap_scroll(offset, Vec2::ONE, dt), offset, "{dt}");
        }
    }

    #[test]
    fn a_velocity_or_offset_that_is_not_a_number_counts_as_zero() {
        assert_eq!(
            wrap_scroll(Vec2::new(0.3, 0.6), Vec2::NAN, 1.0),
            Vec2::new(0.3, 0.6)
        );
        assert_eq!(wrap_scroll(Vec2::NAN, Vec2::ZERO, 1.0), Vec2::ZERO);
    }

    proptest! {
        #[test]
        fn the_offset_always_stays_in_the_unit_square(
            offset in (-4.0f32..4.0, -4.0f32..4.0),
            velocity in (-2.0f32..2.0, -2.0f32..2.0),
            dt in 0.0f32..600.0,
        ) {
            let moved = wrap_scroll(Vec2::from(offset), Vec2::from(velocity), dt);

            prop_assert!((0.0..1.0).contains(&moved.x), "{moved}");
            prop_assert!((0.0..1.0).contains(&moved.y), "{moved}");
        }
    }

    fn moving(index: u8, alpha: f32, speed: f32) -> CloudLayerDraw {
        CloudLayerDraw {
            scroll_speed: Vec2::new(speed, 0.0),
            ..layer_draw(index, alpha)
        }
    }

    fn after(speed: f32, seconds: f32) -> Vec2 {
        wrap_scroll(
            Vec2::ZERO,
            Vec2::new(speed, 0.0) * CLOUD_SCROLL_SCALE,
            seconds,
        )
    }

    #[test]
    fn each_layer_scrolls_at_its_own_speed() {
        let mut offsets = ScrollOffsets::new();

        offsets.advance(&[moving(2, 1.0, 0.01), moving(5, 1.0, -0.02)], 2.0);

        assert_close(offsets.offset(2), after(0.01, 2.0));
        assert_close(offsets.offset(5), after(-0.02, 2.0));
        assert_eq!(
            offsets.offset(3),
            Vec2::ZERO,
            "a layer nobody shows stays put"
        );
    }

    #[test]
    fn the_offset_goes_on_from_where_it_was() {
        let mut offsets = ScrollOffsets::new();
        let layers = [moving(1, 1.0, 0.004)];

        offsets.advance(&layers, 0.5);
        offsets.advance(&layers, 0.5);

        assert_close(offsets.offset(1), after(0.004, 1.0));
    }

    #[test]
    fn a_layer_two_weathers_show_moves_once_at_the_speed_weighted_by_alpha() {
        let mut offsets = ScrollOffsets::new();
        let layers = [moving(4, 0.75, 0.04), moving(4, 0.25, 0.0)];

        offsets.advance(&layers, 1.0);

        assert_close(offsets.offset(4), after(0.03, 1.0));
    }

    #[test]
    fn a_layer_that_shows_in_neither_weather_moves_at_the_plain_mean_speed() {
        let mut offsets = ScrollOffsets::new();
        let layers = [moving(4, 0.0, 0.04), moving(4, 0.0, 0.0)];

        offsets.advance(&layers, 1.0);

        assert_close(offsets.offset(4), after(0.02, 1.0));
    }

    #[test]
    fn a_cross_fade_does_not_make_the_layer_jump() {
        let mut offsets = ScrollOffsets::new();
        let outgoing = moving(4, 1.0, 0.01);
        let incoming = CloudLayerDraw {
            texture: Arc::from("Sky\\Other_d.dds"),
            ..moving(4, 1.0, 0.01)
        };
        let frames = [
            vec![outgoing.clone()],
            vec![outgoing, incoming.clone()],
            vec![incoming],
        ];

        let seen: Vec<Vec2> = frames
            .iter()
            .map(|layers| {
                offsets.advance(layers, 0.1);
                offsets.offset(4)
            })
            .collect();

        for (frame, offset) in (1..).zip(seen) {
            assert_close(offset, after(0.01, 0.1 * frame as f32));
        }
    }
}

mod geometry {
    use super::*;

    fn mesh(
        topology: Topology,
        positions: &[[f32; 3]],
        indices: &[u32],
        colors: Option<&[[u8; 4]]>,
    ) -> CpuMesh {
        CpuMesh {
            vertices: positions
                .iter()
                .enumerate()
                .map(|(index, &position)| Vertex {
                    position,
                    tex_coords: [index as f32, 0.5],
                    normal: [0.0, 1.0, 0.0],
                    tangent: [0.0; 4],
                })
                .collect(),
            topology,
            indices: indices.to_vec(),
            texture_path: None,
            vertex_colors: colors.map(<[_]>::to_vec),
            effect: None,
            refraction: None,
        }
    }

    const TRIANGLE: [[f32; 3]; 3] = [[0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [0.0, 1.0, 1.0]];
    const QUAD: [[f32; 3]; 4] = [
        [0.0, 0.0, 2.0],
        [1.0, 0.0, 2.0],
        [1.0, 1.0, 2.0],
        [0.0, 1.0, 2.0],
    ];

    fn triangle(colors: Option<&[[u8; 4]]>) -> CpuMesh {
        mesh(Topology::Triangles, &TRIANGLE, &[0, 1, 2], colors)
    }

    fn quad() -> CpuMesh {
        mesh(Topology::Triangles, &QUAD, &[0, 1, 2, 0, 2, 3], None)
    }

    #[test]
    fn every_mesh_gets_its_index_range_in_the_order_of_the_file() {
        let meshes = vec![triangle(None), quad(), triangle(None)];

        let geometry = CloudGeometry::from_meshes("k", meshes).unwrap();

        assert_eq!(geometry.ranges, [0..3, 3..9, 9..12]);
        assert_eq!(geometry.vertices.len(), 3 + 4 + 3);
    }

    #[test]
    fn the_indices_of_a_mesh_are_shifted_past_the_vertices_before_it() {
        let geometry = CloudGeometry::from_meshes("k", vec![triangle(None), quad()]).unwrap();

        assert_eq!(geometry.indices, [0, 1, 2, 3, 4, 5, 3, 5, 6]);
    }

    #[test]
    fn a_mesh_that_is_not_a_triangle_list_keeps_its_place_with_an_empty_range() {
        let points = mesh(Topology::Points, &TRIANGLE, &[0, 1, 2], None);

        let geometry =
            CloudGeometry::from_meshes("k", vec![triangle(None), points, quad()]).unwrap();

        assert_eq!(geometry.ranges, [0..3, 3..3, 3..9]);
        assert_eq!(
            geometry.vertices.len(),
            3 + 4,
            "the points are not uploaded"
        );
        assert_eq!(geometry.indices[3..], [3, 4, 5, 3, 5, 6]);
    }

    #[test]
    fn positions_and_texture_coordinates_come_from_the_vertices() {
        let geometry = CloudGeometry::from_meshes("k", vec![quad()]).unwrap();

        let second = geometry.vertices[1];
        assert_eq!(second.position, [1.0, 0.0, 2.0]);
        assert_eq!(second.uv, [1.0, 0.5]);
    }

    #[test]
    fn vertex_colours_are_kept_and_a_mesh_without_them_is_opaque_white() {
        let colors = [[10, 20, 30, 0], [11, 21, 31, 127], [12, 22, 32, 255]];

        let geometry =
            CloudGeometry::from_meshes("k", vec![triangle(Some(&colors)), quad()]).unwrap();

        let stored: Vec<[u8; 4]> = geometry
            .vertices
            .iter()
            .map(|vertex| vertex.color)
            .collect();
        assert_eq!(stored[..3], colors);
        assert_eq!(stored[3..], [[255; 4]; 4]);
    }

    #[test]
    fn a_vertex_beyond_the_colours_the_mesh_has_is_opaque_white() {
        let colors = [[1, 2, 3, 4]];

        let geometry = CloudGeometry::from_meshes("k", vec![triangle(Some(&colors))]).unwrap();

        assert_eq!(geometry.vertices[0].color, [1, 2, 3, 4]);
        assert_eq!(geometry.vertices[1].color, [255; 4]);
    }

    #[test]
    fn a_model_without_triangles_is_an_error_naming_the_key() {
        let points = mesh(Topology::Points, &TRIANGLE, &[0, 1, 2], None);

        let error =
            CloudGeometry::from_meshes("meshes\\sky\\clouds.glb", vec![points]).unwrap_err();

        assert!(
            matches!(error, SkyAssetError::NoTriangles { ref key } if key.ends_with("clouds.glb")),
            "{error}"
        );
        assert!(matches!(
            CloudGeometry::from_meshes("k", Vec::new()),
            Err(SkyAssetError::NoTriangles { .. })
        ));
    }

    #[test]
    fn an_index_past_the_vertices_is_an_error() {
        let broken = mesh(Topology::Triangles, &TRIANGLE, &[0, 1, 3], None);

        let error = CloudGeometry::from_meshes("k", vec![triangle(None), broken]).unwrap_err();

        assert!(matches!(error, SkyAssetError::Mesh { .. }), "{error}");
    }

    #[test]
    fn the_vertex_layout_matches_the_inputs_of_the_shader() {
        let layout = CloudVertex::layout();

        assert_eq!(layout.array_stride, 24);
        assert_eq!(std::mem::size_of::<CloudVertex>(), 24);
        let attributes: Vec<_> = layout
            .attributes
            .iter()
            .map(|attribute| {
                (
                    attribute.shader_location,
                    attribute.offset,
                    attribute.format,
                )
            })
            .collect();
        assert_eq!(
            attributes,
            [
                (0, 0, wgpu::VertexFormat::Float32x3),
                (1, 12, wgpu::VertexFormat::Float32x2),
                (2, 20, wgpu::VertexFormat::Unorm8x4),
            ]
        );
    }

    /// The bytes of one mesh of [`clouds_glb`]: a triangle that covers the view, with the
    /// alpha of its vertex colours given.
    fn triangle_bytes(alpha: u8) -> Vec<u8> {
        let positions = [-4.0f32, -4.0, -1.0, 4.0, -4.0, -1.0, 0.0, 4.0, -1.0];
        let uvs = [-1.5f32, 2.5, 2.5, 2.5, 0.5, -1.5];
        let mut bytes: Vec<u8> = positions
            .iter()
            .chain(&uvs)
            .flat_map(|value| value.to_le_bytes())
            .collect();
        bytes.extend([255, 255, 255, alpha].repeat(3));
        bytes
    }

    /// A GLB of two one-triangle meshes under a root node, the first one moved up by one unit.
    pub(super) fn clouds_glb() -> Vec<u8> {
        const MESH_BYTES: usize = 72;
        let bin = [triangle_bytes(255), triangle_bytes(128)].concat();
        let views: Vec<Value> = (0..2)
            .flat_map(|mesh| {
                [(0, 36), (36, 24), (60, 12)].map(|(offset, length)| {
                    json!({"buffer": 0, "byteOffset": mesh * MESH_BYTES + offset, "byteLength": length})
                })
            })
            .collect();
        let accessors: Vec<Value> = (0..2)
            .flat_map(|mesh| {
                let view = mesh * 3;
                [
                    json!({"bufferView": view, "componentType": 5126, "count": 3,
                           "type": "VEC3", "min": [-4, -4, -1], "max": [4, 4, -1]}),
                    json!({"bufferView": view + 1, "componentType": 5126, "count": 3,
                           "type": "VEC2"}),
                    json!({"bufferView": view + 2, "componentType": 5121, "normalized": true,
                           "count": 3, "type": "VEC4"}),
                ]
            })
            .collect();
        let attributes = |first: usize| json!({"POSITION": first, "TEXCOORD_0": first + 1, "COLOR_0": first + 2});
        let document = json!({
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [
                {"name": "Clouds", "children": [1, 2]},
                {"mesh": 0, "translation": [0, 1, 0]},
                {"mesh": 1}
            ],
            "meshes": [
                {"primitives": [{"attributes": attributes(0)}]},
                {"primitives": [{"attributes": attributes(3)}]}
            ],
            "buffers": [{"byteLength": bin.len()}],
            "bufferViews": views,
            "accessors": accessors
        });
        glb(&document, &bin)
    }

    /// An install whose `Meshes\<dir>\<file>` is the model of [`clouds_glb`].
    pub(super) fn install_with_clouds(dir: &str, file: &str) -> tempfile::TempDir {
        install_with(dir, file, &clouds_glb())
    }

    #[test]
    fn the_cloud_model_of_a_nif_path_loads_from_its_glb_whatever_the_stored_casing() {
        let root = install_with_clouds("SKY", "Clouds.glb");

        let geometry =
            load_cloud_geometry(root.path().to_str().unwrap(), "sky\\clouds.nif").unwrap();

        assert_eq!(geometry.ranges, [0..3, 3..6]);
        assert_eq!(
            geometry.vertices[0].position,
            [-4.0, -3.0, -1.0],
            "the node moved the first mesh up"
        );
        assert_eq!(geometry.vertices[3].position, [-4.0, -4.0, -1.0]);
        assert_eq!(geometry.vertices[1].uv, [2.5, 2.5]);
        assert_eq!(geometry.vertices[0].color, [255; 4]);
        assert_eq!(geometry.vertices[3].color, [255, 255, 255, 128]);
    }

    #[test]
    fn a_missing_cloud_model_is_a_not_found_error() {
        let root = install_with_clouds("sky", "other.glb");

        let error =
            load_cloud_geometry(root.path().to_str().unwrap(), "sky\\clouds.nif").unwrap_err();

        assert!(matches!(error, SkyAssetError::NotFound { .. }), "{error}");
    }

    #[test]
    fn an_undecodable_cloud_model_is_a_mesh_error() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("meshes").join("sky");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("clouds.glb"), b"not a glb").unwrap();

        let error =
            load_cloud_geometry(root.path().to_str().unwrap(), "sky\\clouds.nif").unwrap_err();

        assert!(matches!(error, SkyAssetError::Mesh { .. }), "{error}");
    }
}

mod planning {
    use super::*;

    /// A model of three meshes, the second of which has nothing to draw.
    const RANGES: [Range<u32>; 3] = [0..3, 3..3, 3..9];

    /// What a plan holds, without the layers themselves.
    #[derive(Debug, Default, PartialEq)]
    struct Outcome {
        /// The layer index and the mesh range of each draw.
        drawn: Vec<(u8, Range<u32>)>,
        without_mesh: Vec<u8>,
        truncated: bool,
    }

    impl Outcome {
        fn drawing(drawn: Vec<(u8, Range<u32>)>) -> Self {
            Self {
                drawn,
                ..Self::default()
            }
        }
    }

    fn outcome(input: &CloudsInput, ranges: &[Range<u32>]) -> Outcome {
        let plan = plan_draws(input, ranges);
        Outcome {
            drawn: plan
                .draws
                .iter()
                .map(|planned| (planned.layer.layer, planned.range.clone()))
                .collect(),
            without_mesh: plan.without_mesh,
            truncated: plan.truncated,
        }
    }

    #[test]
    fn a_layer_draws_the_mesh_of_its_index_in_the_order_of_the_input() {
        let input = input(vec![layer_draw(2, 1.0), layer_draw(0, 0.5)]);

        let plan = plan_draws(&input, &RANGES);

        assert_eq!(
            outcome(&input, &RANGES),
            Outcome::drawing(vec![(2, 3..9), (0, 0..3)])
        );
        assert_eq!(plan.draws[0].layer, &input.layers[0]);
        assert_eq!(plan.draws[1].layer, &input.layers[1]);
    }

    #[test]
    fn a_layer_that_two_weathers_show_is_drawn_twice_in_the_order_of_the_input() {
        let mut incoming = layer_draw(0, 0.25);
        incoming.texture = Arc::from("Sky\\Other_d.dds");
        let input = input(vec![
            layer_draw(0, 0.75),
            layer_draw(2, 1.0),
            incoming.clone(),
        ]);

        let plan = plan_draws(&input, &RANGES);

        assert_eq!(
            outcome(&input, &RANGES),
            Outcome::drawing(vec![(0, 0..3), (2, 3..9), (0, 0..3)])
        );
        assert_eq!(plan.draws[2].layer, &incoming);
    }

    #[test]
    fn a_layer_less_opaque_than_one_step_of_a_byte_is_not_drawn() {
        let step = 1.0 / 255.0;
        let input = input(vec![
            layer_draw(0, 0.0),
            layer_draw(2, step * 0.99),
            layer_draw(0, step),
            layer_draw(2, 1.0),
        ]);

        assert_eq!(
            outcome(&input, &RANGES),
            Outcome::drawing(vec![(0, 0..3), (2, 3..9)])
        );
    }

    #[test]
    fn an_alpha_that_is_negative_or_not_a_number_is_not_drawn_and_one_above_one_is() {
        let input = input(vec![
            layer_draw(0, -0.5),
            layer_draw(0, f32::NAN),
            layer_draw(0, f32::NEG_INFINITY),
            layer_draw(2, 3.0),
        ]);

        assert_eq!(outcome(&input, &RANGES), Outcome::drawing(vec![(2, 3..9)]));
    }

    #[test]
    fn solo_draws_the_chosen_layer_alone() {
        let mut input = input(vec![
            layer_draw(0, 1.0),
            layer_draw(2, 1.0),
            layer_draw(0, 0.5),
        ]);

        input.solo = Some(0);
        assert_eq!(
            outcome(&input, &RANGES),
            Outcome::drawing(vec![(0, 0..3), (0, 0..3)])
        );
        input.solo = Some(7);
        assert_eq!(outcome(&input, &RANGES), Outcome::default());
    }

    #[test]
    fn solo_does_not_bring_back_a_layer_that_is_too_faint() {
        let mut input = input(vec![layer_draw(0, 0.0)]);
        input.solo = Some(0);

        assert_eq!(outcome(&input, &RANGES), Outcome::default());
    }

    #[test]
    fn a_layer_beyond_the_meshes_is_reported_once_and_not_drawn() {
        let input = input(vec![
            layer_draw(5, 1.0),
            layer_draw(0, 1.0),
            layer_draw(5, 0.5),
            layer_draw(9, 1.0),
        ]);

        assert_eq!(
            outcome(&input, &RANGES),
            Outcome {
                drawn: vec![(0, 0..3)],
                without_mesh: vec![5, 9],
                truncated: false,
            }
        );
    }

    #[test]
    fn a_layer_beyond_the_meshes_that_cannot_be_seen_is_not_reported() {
        let faint = input(vec![layer_draw(5, 0.0)]);
        let mut soloed = input(vec![layer_draw(6, 1.0)]);
        soloed.solo = Some(0);

        assert_eq!(outcome(&faint, &RANGES), Outcome::default());
        assert_eq!(outcome(&soloed, &RANGES), Outcome::default());
    }

    #[test]
    fn a_mesh_without_triangles_is_neither_drawn_nor_reported() {
        let input = input(vec![layer_draw(1, 1.0)]);

        assert_eq!(outcome(&input, &RANGES), Outcome::default());
    }

    #[test]
    fn no_layers_or_no_meshes_plan_nothing() {
        let layers = input(vec![layer_draw(0, 1.0)]);

        assert_eq!(
            outcome(&CloudsInput::default(), &RANGES),
            Outcome::default()
        );
        assert_eq!(
            outcome(&layers, &[]),
            Outcome {
                without_mesh: vec![0],
                ..Outcome::default()
            }
        );
    }

    #[test]
    fn layers_past_the_limit_are_left_out_and_the_plan_says_so() {
        let too_many = input(vec![layer_draw(0, 1.0); MAX_DRAWS + 3]);
        let just_enough = input(vec![layer_draw(0, 1.0); MAX_DRAWS]);

        let over = outcome(&too_many, &RANGES);
        let exactly = outcome(&just_enough, &RANGES);

        let all = vec![(0, 0..3); MAX_DRAWS];
        assert_eq!(
            over,
            Outcome {
                drawn: all.clone(),
                without_mesh: Vec::new(),
                truncated: true,
            }
        );
        assert_eq!(exactly, Outcome::drawing(all));
    }
}

mod uniform {
    use super::*;

    fn colored(color: Vec3, alpha: f32) -> CloudLayerDraw {
        CloudLayerDraw {
            color,
            ..layer_draw(0, alpha)
        }
    }

    #[test]
    fn the_uniform_has_the_layout_the_shader_declares() {
        assert_eq!(std::mem::size_of::<CloudUniform>(), 32);
    }

    #[test]
    fn the_radiance_is_the_colour_times_the_sky_scale_and_one_plus_the_flash() {
        let layer = colored(Vec3::new(0.5, 0.25, 1.0), 0.75);

        let uniform = CloudUniform::new(&layer, Vec2::ZERO, 0.5, 2.0);

        assert_eq!(uniform.radiance_opacity, [1.5, 0.75, 3.0, 0.75]);
    }

    #[test]
    fn the_scroll_offset_and_the_haze_width_ride_in_the_second_vector() {
        let uniform = CloudUniform::new(&layer_draw(0, 1.0), Vec2::new(0.25, 0.75), 0.0, 1.0);

        assert_eq!(
            uniform.offset_haze,
            [0.25, 0.75, CLOUD_HORIZON_HAZE_SINE, 0.0]
        );
    }

    #[test]
    fn the_opacity_is_clamped_to_one() {
        let uniform = CloudUniform::new(&layer_draw(0, 3.0), Vec2::ZERO, 0.0, 1.0);

        assert_eq!(uniform.radiance_opacity[3], 1.0);
    }

    #[test]
    fn values_that_are_negative_or_not_numbers_cannot_darken_or_corrupt_the_layer() {
        let layer = colored(Vec3::new(-1.0, f32::NAN, 0.5), 1.0);

        let negative_flash = CloudUniform::new(&layer, Vec2::ZERO, -3.0, 1.0);
        let undefined_flash = CloudUniform::new(&layer, Vec2::ZERO, f32::NAN, 1.0);
        let negative_scale = CloudUniform::new(&layer, Vec2::ZERO, 0.0, -2.0);
        let undefined_scale = CloudUniform::new(&layer, Vec2::ZERO, 0.0, f32::NAN);

        assert_eq!(negative_flash.radiance_opacity, [0.0, 0.0, 0.5, 1.0]);
        assert_eq!(undefined_flash.radiance_opacity, [0.0, 0.0, 0.5, 1.0]);
        assert_eq!(negative_scale.radiance_opacity, [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(undefined_scale.radiance_opacity, [0.0, 0.0, 0.0, 1.0]);
    }
}

mod shader_module {
    use super::*;

    #[test]
    fn the_cloud_shader_has_both_entry_points() {
        let module = naga::front::wgsl::parse_str(shaders::SKY_CLOUDS.wgsl()).expect("it parses");
        let entries: Vec<_> = module
            .entry_points
            .iter()
            .map(|entry| (entry.name.as_str(), entry.stage))
            .collect();
        assert!(entries.contains(&("vs_main", naga::ShaderStage::Vertex)));
        assert!(entries.contains(&("fs_main", naga::ShaderStage::Fragment)));
    }
}

mod gpu {
    use super::*;
    use crate::rendering::lighting::frame_uniform::FrameUniform;

    const BLACK: Vec4 = Vec4::new(0.0, 0.0, 0.0, 1.0);
    const TEXTURE_EDGE: u32 = 16;
    const HALF_ALPHA: u8 = 128;
    const RED_TEXTURE: &str = "Sky\\Red_d.dds";
    const BLUE_TEXTURE: &str = "Sky\\Blue_d.dds";
    /// Pixels of the 16 x 16 target more than 30 degrees above the horizon, where the horizon
    /// haze has cleared: left of the view axis, right of it and on it.
    const SKY_LEFT: (u32, u32) = (3, 2);
    const SKY_RIGHT: (u32, u32) = (12, 2);
    const SKY_CENTER: (u32, u32) = (8, 2);
    /// A row 4 degrees above the horizon, inside the haze, and one below the horizon.
    const HORIZON_ROW: u32 = 7;
    const BELOW_HORIZON_ROW: u32 = 13;
    /// Where the sun, 30 degrees up due north, appears.
    const SUN_PIXEL: (u32, u32) = (8, 3);

    const _: () = assert!(
        CLOUD_HORIZON_HAZE_SINE > 0.1 && CLOUD_HORIZON_HAZE_SINE < 0.4,
        "the probe pixels assume that the haze ends between 6 and 24 degrees above the horizon"
    );

    fn assert_color(actual: Vec4, expected: Vec3) {
        assert!(
            actual.truncate().abs_diff_eq(expected, 3e-3),
            "{actual} is not {expected}"
        );
    }

    fn at(pixels: &[Vec4], (column, row): (u32, u32)) -> Vec4 {
        texel(pixels, column, row)
    }

    fn assert_black(pixels: &[Vec4]) {
        assert!(pixels.iter().all(|texel| *texel == BLACK), "{pixels:?}");
    }

    fn not_found(key: &str) -> SkyAssetError {
        SkyAssetError::NotFound {
            key: key.to_owned(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        }
    }

    type Mesh = (Vec<CloudVertex>, Vec<u32>);

    /// A point of the plane z = -1, which the 90 degree view of the rig spans from -1 to 1 in x
    /// and y. The texture coordinates run from the left (u 0) to the right (u 1) and from the
    /// top (v 0) down, so a texture appears upright.
    fn vertex(x: f32, y: f32, alpha: u8) -> CloudVertex {
        CloudVertex {
            position: [x, y, -1.0],
            uv: [f32::midpoint(x, 1.0), f32::midpoint(1.0, -y)],
            color: [255, 255, 255, alpha],
        }
    }

    /// A triangle that covers the whole view, with the vertex alpha of its base and its tip.
    fn covering_with_tip(base: u8, tip: u8) -> Mesh {
        (
            vec![
                vertex(-4.0, -4.0, base),
                vertex(4.0, -4.0, base),
                vertex(0.0, 4.0, tip),
            ],
            vec![0, 1, 2],
        )
    }

    fn covering() -> Mesh {
        covering_with_tip(255, 255)
    }

    /// The left or the right half of the view and a margin.
    fn half(right: bool) -> Mesh {
        let (near, far) = if right { (0.0, 4.0) } else { (-4.0, 0.0) };
        (
            vec![
                vertex(near, -4.0, 255),
                vertex(far, -4.0, 255),
                vertex(far, 4.0, 255),
                vertex(near, 4.0, 255),
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
    }

    /// A model whose mesh `i` is `meshes[i]`.
    fn model(meshes: Vec<Mesh>) -> CloudGeometry {
        let mut geometry = CloudGeometry {
            vertices: Vec::new(),
            indices: Vec::new(),
            ranges: Vec::new(),
        };
        for (vertices, indices) in meshes {
            let first = geometry.vertices.len() as u32;
            let begin = geometry.indices.len() as u32;
            geometry
                .indices
                .extend(indices.iter().map(|index| index + first));
            geometry.vertices.extend(vertices);
            geometry.ranges.push(begin..geometry.indices.len() as u32);
        }
        geometry
    }

    /// The textures a test offers by path; every request is recorded.
    struct Textures {
        known: Vec<(&'static str, wgpu::TextureView)>,
        requested: Vec<String>,
    }

    impl Textures {
        fn new(known: Vec<(&'static str, wgpu::TextureView)>) -> Self {
            Self {
                known,
                requested: Vec::new(),
            }
        }

        /// Only `TEXTURE`, which is white.
        fn white(rig: &Rig) -> Self {
            Self::new(vec![(TEXTURE, white_texture(rig, TEXTURE_EDGE))])
        }

        fn load(&mut self, path: &str) -> Result<wgpu::TextureView, SkyAssetError> {
            self.requested.push(path.to_owned());
            self.known
                .iter()
                .find(|(known, _)| *known == path)
                .map(|(_, view)| view.clone())
                .ok_or_else(|| not_found(&named_texture_key(path)))
        }
    }

    /// A cloud layer holding a model, the textures a test offers and the rig it draws on.
    struct Scene {
        rig: Rig,
        clouds: CloudLayer,
        textures: Textures,
    }

    impl Scene {
        fn install(
            rig: Rig,
            geometry: Result<CloudGeometry, SkyAssetError>,
            textures: Textures,
        ) -> (Self, AssetState) {
            let mut clouds = CloudLayer::new(&rig.device, rig.world_groups());
            let state = clouds.install_mesh(&rig.device, geometry);
            let scene = Self {
                rig,
                clouds,
                textures,
            };
            (scene, state)
        }

        fn new(rig: Rig, meshes: Vec<Mesh>, textures: Textures) -> Self {
            let (scene, state) = Self::install(rig, Ok(model(meshes)), textures);
            assert_eq!(state, AssetState::Loaded);
            scene
        }

        fn white(rig: Rig, meshes: Vec<Mesh>) -> Self {
            let textures = Textures::white(&rig);
            Self::new(rig, meshes, textures)
        }

        fn update(&mut self, input: &CloudsInput, radiance_scale: f32, dt: f32) {
            let textures = &mut self.textures;
            self.clouds.update(
                &self.rig.device,
                &self.rig.queue,
                input,
                radiance_scale,
                dt,
                |path| textures.load(path),
            );
        }

        /// The pixels of the last update drawn with `depth` in the depth buffer.
        fn draw(&self, depth: f32) -> Vec<Vec4> {
            render(&self.rig, depth, |pass| self.clouds.draw(pass))
        }

        fn scaled_frame(&mut self, input: &CloudsInput, radiance_scale: f32, dt: f32) -> Vec<Vec4> {
            self.update(input, radiance_scale, dt);
            self.draw(SKY_CLEAR)
        }

        /// `input` at the sky radiance scale 1 with no time passing.
        fn frame(&mut self, input: &CloudsInput) -> Vec<Vec4> {
            self.scaled_frame(input, 1.0, 0.0)
        }
    }

    fn colored(index: u8, color: Vec3, alpha: f32) -> CloudLayerDraw {
        CloudLayerDraw {
            color,
            ..layer_draw(index, alpha)
        }
    }

    fn textured(index: u8, texture: &str) -> CloudLayerDraw {
        CloudLayerDraw {
            texture: Arc::from(texture),
            ..layer_draw(index, 1.0)
        }
    }

    #[test]
    fn a_layer_paints_its_colour_over_the_pixels_of_its_mesh_and_nothing_else() {
        let name = "a_layer_paints_its_colour_over_the_pixels_of_its_mesh_and_nothing_else";
        let Some(rig) = rig(name) else {
            return;
        };
        let mut scene = Scene::white(rig, vec![half(false)]);

        let drawn = scene.frame(&input(vec![colored(0, Vec3::new(0.5, 0.25, 1.0), 1.0)]));

        assert_color(at(&drawn, SKY_LEFT), Vec3::new(0.5, 0.25, 1.0));
        assert_eq!(
            at(&drawn, SKY_LEFT).w,
            1.0,
            "the target alpha is left alone"
        );
        assert_eq!(at(&drawn, SKY_RIGHT), BLACK);
    }

    #[test]
    fn the_layer_alpha_blends_the_layer_over_what_is_behind_it() {
        let Some(rig) = rig("the_layer_alpha_blends_the_layer_over_what_is_behind_it") else {
            return;
        };
        let mut scene = Scene::white(rig, vec![covering()]);

        let drawn = scene.frame(&input(vec![colored(0, Vec3::new(0.0, 1.0, 0.5), 0.5)]));

        assert_color(at(&drawn, SKY_CENTER), Vec3::new(0.0, 0.5, 0.25));
    }

    #[test]
    fn the_texture_alpha_and_the_vertex_alpha_scale_the_opacity() {
        let Some(rig) = rig("the_texture_alpha_and_the_vertex_alpha_scale_the_opacity") else {
            return;
        };
        let translucent = pattern_texture(&rig, TEXTURE_EDGE, |_, _| [255, 255, 255, HALF_ALPHA]);
        let textures = Textures::new(vec![(TEXTURE, translucent)]);
        let meshes = vec![covering(), covering_with_tip(HALF_ALPHA, HALF_ALPHA)];
        let mut scene = Scene::new(rig, meshes, textures);
        let half_alpha = f32::from(HALF_ALPHA) / 255.0;

        let texture_only = scene.frame(&input(vec![layer_draw(0, 1.0)]));
        let both = scene.frame(&input(vec![layer_draw(1, 1.0)]));

        assert_color(at(&texture_only, SKY_CENTER), Vec3::splat(half_alpha));
        assert_color(at(&both, SKY_CENTER), Vec3::splat(half_alpha * half_alpha));
    }

    #[test]
    fn the_vertex_alpha_fades_a_mesh_out_towards_its_tip() {
        let Some(rig) = rig("the_vertex_alpha_fades_a_mesh_out_towards_its_tip") else {
            return;
        };
        let mut scene = Scene::white(rig, vec![covering_with_tip(255, 0)]);

        let drawn = scene.frame(&input(vec![layer_draw(0, 1.0)]));

        let brightness: Vec<f32> = (0..=2).map(|row| texel(&drawn, 8, row).x).collect();
        assert!(
            brightness.windows(2).all(|pair| pair[0] < pair[1]),
            "the mesh thins out towards the tip, the top row: {brightness:?}"
        );
        assert!(brightness[0] > 0.0);
    }

    #[test]
    fn the_layer_colour_multiplies_the_texels_per_channel() {
        let Some(rig) = rig("the_layer_colour_multiplies_the_texels_per_channel") else {
            return;
        };
        let orange = pattern_texture(&rig, TEXTURE_EDGE, |_, _| [255, 128, 0, 255]);
        let mut scene = Scene::new(
            rig,
            vec![covering()],
            Textures::new(vec![(TEXTURE, orange)]),
        );

        let drawn = scene.frame(&input(vec![colored(0, Vec3::new(1.0, 0.5, 1.0), 1.0)]));

        let linear_green = 0.215_860_5;
        assert_color(
            at(&drawn, SKY_CENTER),
            Vec3::new(1.0, 0.5 * linear_green, 0.0),
        );
    }

    #[test]
    fn the_sky_radiance_scale_and_the_flash_brighten_the_clouds() {
        let Some(rig) = rig("the_sky_radiance_scale_and_the_flash_brighten_the_clouds") else {
            return;
        };
        let mut scene = Scene::white(rig, vec![covering()]);
        let mut cloud = input(vec![colored(0, Vec3::splat(0.25), 1.0)]);

        let plain = scene.scaled_frame(&cloud, 1.0, 0.0);
        let doubled = scene.scaled_frame(&cloud, 2.0, 0.0);
        cloud.flash = 1.0;
        let flashed = scene.scaled_frame(&cloud, 1.0, 0.0);
        let both = scene.scaled_frame(&cloud, 2.0, 0.0);

        assert_color(at(&plain, SKY_CENTER), Vec3::splat(0.25));
        assert_color(at(&doubled, SKY_CENTER), Vec3::splat(0.5));
        assert_color(at(&flashed, SKY_CENTER), Vec3::splat(0.5));
        assert_color(at(&both, SKY_CENTER), Vec3::splat(1.0));
    }

    #[test]
    fn layers_draw_in_the_order_of_the_input() {
        let Some(rig) = rig("layers_draw_in_the_order_of_the_input") else {
            return;
        };
        let mut scene = Scene::white(rig, vec![covering()]);
        let red = colored(0, Vec3::X, 1.0);
        let green = colored(0, Vec3::Y, 0.5);

        let green_on_red = scene.frame(&input(vec![red.clone(), green.clone()]));
        let red_on_green = scene.frame(&input(vec![green, red]));

        assert_color(at(&green_on_red, SKY_CENTER), Vec3::new(0.5, 0.5, 0.0));
        assert_color(at(&red_on_green, SKY_CENTER), Vec3::X);
    }

    #[test]
    fn a_layer_draws_the_mesh_of_its_index() {
        let Some(rig) = rig("a_layer_draws_the_mesh_of_its_index") else {
            return;
        };
        let mut scene = Scene::white(rig, vec![half(false), half(true)]);

        let first = scene.frame(&input(vec![layer_draw(0, 1.0)]));
        let second = scene.frame(&input(vec![layer_draw(1, 1.0)]));
        let beyond = scene.frame(&input(vec![layer_draw(2, 1.0)]));

        assert_color(at(&first, SKY_LEFT), Vec3::ONE);
        assert_eq!(at(&first, SKY_RIGHT), BLACK);
        assert_eq!(at(&second, SKY_LEFT), BLACK);
        assert_color(at(&second, SKY_RIGHT), Vec3::ONE);
        assert_black(&beyond);
    }

    #[test]
    fn solo_draws_the_chosen_layer_alone() {
        let Some(rig) = rig("solo_draws_the_chosen_layer_alone") else {
            return;
        };
        let mut scene = Scene::white(rig, vec![half(false), half(true)]);
        let mut both = input(vec![layer_draw(0, 1.0), layer_draw(1, 1.0)]);
        both.solo = Some(1);

        let drawn = scene.frame(&both);

        assert_eq!(at(&drawn, SKY_LEFT), BLACK);
        assert_color(at(&drawn, SKY_RIGHT), Vec3::ONE);
    }

    #[test]
    fn a_layer_without_opacity_draws_nothing_and_loads_nothing() {
        let Some(rig) = rig("a_layer_without_opacity_draws_nothing_and_loads_nothing") else {
            return;
        };
        let mut scene = Scene::white(rig, vec![covering()]);

        let drawn = scene.frame(&input(vec![layer_draw(0, 0.0), layer_draw(0, 0.001)]));

        assert_black(&drawn);
        assert!(
            scene.textures.requested.is_empty(),
            "{:?}",
            scene.textures.requested
        );
    }

    #[test]
    fn a_layer_that_is_left_out_of_the_next_frame_is_not_drawn_again() {
        let Some(rig) = rig("a_layer_that_is_left_out_of_the_next_frame_is_not_drawn_again") else {
            return;
        };
        let mut scene = Scene::white(rig, vec![covering()]);
        let shown = scene.frame(&input(vec![layer_draw(0, 1.0)]));

        let gone = scene.frame(&CloudsInput::default());

        assert_color(at(&shown, SKY_CENTER), Vec3::ONE);
        assert_black(&gone);
    }

    #[test]
    fn the_texture_of_a_layer_is_the_one_its_input_names_and_it_loads_once() {
        let name = "the_texture_of_a_layer_is_the_one_its_input_names_and_it_loads_once";
        let Some(rig) = rig(name) else {
            return;
        };
        let red = pattern_texture(&rig, TEXTURE_EDGE, |_, _| [255, 0, 0, 255]);
        let textures = Textures::new(vec![
            (TEXTURE, white_texture(&rig, TEXTURE_EDGE)),
            (RED_TEXTURE, red),
        ]);
        let mut scene = Scene::new(rig, vec![covering()], textures);

        let white = scene.frame(&input(vec![textured(0, TEXTURE)]));
        let red = scene.frame(&input(vec![textured(0, RED_TEXTURE)]));
        let white_again = scene.frame(&input(vec![textured(0, TEXTURE)]));

        assert_color(at(&white, SKY_CENTER), Vec3::ONE);
        assert_color(at(&red, SKY_CENTER), Vec3::X);
        assert_color(at(&white_again, SKY_CENTER), Vec3::ONE);
        assert_eq!(scene.textures.requested, [TEXTURE, RED_TEXTURE]);
    }

    #[test]
    fn layers_that_trade_textures_or_places_keep_their_own_texture() {
        let name = "layers_that_trade_textures_or_places_keep_their_own_texture";
        let Some(rig) = rig(name) else {
            return;
        };
        let red = pattern_texture(&rig, TEXTURE_EDGE, |_, _| [255, 0, 0, 255]);
        let blue = pattern_texture(&rig, TEXTURE_EDGE, |_, _| [0, 0, 255, 255]);
        let textures = Textures::new(vec![(RED_TEXTURE, red), (BLUE_TEXTURE, blue)]);
        let mut scene = Scene::new(rig, vec![half(false), half(true)], textures);
        let left = |texture| textured(0, texture);
        let right = |texture| textured(1, texture);

        let first = scene.frame(&input(vec![left(RED_TEXTURE), right(BLUE_TEXTURE)]));
        let traded = scene.frame(&input(vec![left(BLUE_TEXTURE), right(RED_TEXTURE)]));
        let reordered = scene.frame(&input(vec![right(RED_TEXTURE), left(BLUE_TEXTURE)]));

        assert_color(at(&first, SKY_LEFT), Vec3::X);
        assert_color(at(&first, SKY_RIGHT), Vec3::Z);
        for pixels in [&traded, &reordered] {
            assert_color(at(pixels, SKY_LEFT), Vec3::Z);
            assert_color(at(pixels, SKY_RIGHT), Vec3::X);
        }
        assert_eq!(scene.textures.requested, [RED_TEXTURE, BLUE_TEXTURE]);
    }

    #[test]
    fn two_layers_with_one_texture_load_it_once() {
        let Some(rig) = rig("two_layers_with_one_texture_load_it_once") else {
            return;
        };
        let mut scene = Scene::white(rig, vec![half(false), half(true)]);

        let drawn = scene.frame(&input(vec![layer_draw(0, 1.0), layer_draw(1, 1.0)]));

        assert_color(at(&drawn, SKY_LEFT), Vec3::ONE);
        assert_color(at(&drawn, SKY_RIGHT), Vec3::ONE);
        assert_eq!(scene.textures.requested, [TEXTURE]);
    }

    #[test]
    fn a_layer_whose_texture_is_missing_is_skipped_and_not_asked_for_again() {
        let name = "a_layer_whose_texture_is_missing_is_skipped_and_not_asked_for_again";
        let Some(rig) = rig(name) else {
            return;
        };
        let mut scene = Scene::white(rig, vec![half(false), half(true)]);
        let mistyped = "SkySky\\GS01_d.dds";
        let layers = input(vec![textured(0, mistyped), layer_draw(1, 1.0)]);

        let drawn = scene.frame(&layers);
        let again = scene.frame(&layers);

        for pixels in [&drawn, &again] {
            assert_eq!(at(pixels, SKY_LEFT), BLACK, "the layer with the typo");
            assert_color(at(pixels, SKY_RIGHT), Vec3::ONE);
        }
        assert_eq!(scene.textures.requested, [mistyped, TEXTURE]);
    }

    #[test]
    fn the_texture_scrolls_by_the_offset_and_wraps() {
        let Some(rig) = rig("the_texture_scrolls_by_the_offset_and_wraps") else {
            return;
        };
        let left_lit = pattern_texture(&rig, TEXTURE_EDGE, |column, _| {
            if column < TEXTURE_EDGE / 2 {
                [255; 4]
            } else {
                [0, 0, 0, 255]
            }
        });
        let textures = Textures::new(vec![(TEXTURE, left_lit)]);
        let mut scene = Scene::new(rig, vec![covering()], textures);
        let drifting = input(vec![CloudLayerDraw {
            scroll_speed: Vec2::new(0.5 / CLOUD_SCROLL_SCALE, 0.0),
            ..layer_draw(0, 1.0)
        }]);

        let unmoved = scene.scaled_frame(&drifting, 1.0, 0.0);
        let half_way = scene.scaled_frame(&drifting, 1.0, 1.0);
        let all_the_way = scene.scaled_frame(&drifting, 1.0, 1.0);

        assert_color(at(&unmoved, SKY_LEFT), Vec3::ONE);
        assert_color(at(&unmoved, SKY_RIGHT), Vec3::ZERO);
        assert_color(at(&half_way, SKY_LEFT), Vec3::ZERO);
        assert_color(at(&half_way, SKY_RIGHT), Vec3::ONE);
        assert_color(at(&all_the_way, SKY_LEFT), Vec3::ONE);
        assert_color(at(&all_the_way, SKY_RIGHT), Vec3::ZERO);
    }

    fn horizon_frame(color: Vec3) -> FrameUniform {
        let mut frame = frame_uniform();
        frame.sky_horizon = color.extend(1.0);
        frame
    }

    #[test]
    fn the_clouds_take_the_horizon_colour_towards_the_horizon_and_below_it() {
        let name = "the_clouds_take_the_horizon_colour_towards_the_horizon_and_below_it";
        let horizon = Vec3::new(0.2, 0.4, 0.6);
        let Some(rig) = rig_with_frame(name, &horizon_frame(horizon)) else {
            return;
        };
        let mut scene = Scene::white(rig, vec![covering()]);

        let drawn = scene.frame(&input(vec![colored(0, Vec3::X, 1.0)]));

        assert_color(at(&drawn, SKY_CENTER), Vec3::X);
        assert_color(texel(&drawn, 8, BELOW_HORIZON_ROW), horizon);
        let hazy = texel(&drawn, 8, HORIZON_ROW).truncate();
        assert!(
            hazy.x > horizon.x && hazy.x < 1.0,
            "red fades towards the horizon colour: {hazy}"
        );
        assert!(hazy.y > 0.0 && hazy.y < horizon.y, "{hazy}");
    }

    #[test]
    fn clouds_cover_what_is_behind_them_but_only_where_they_are_opaque() {
        let name = "clouds_cover_what_is_behind_them_but_only_where_they_are_opaque";
        let Some(rig) = rig(name) else {
            return;
        };
        let (mut stars, _) = star_layer(&rig);
        stars.update(&rig.queue, &unturned(1.0, 1.0));
        let mut scene = Scene::white(rig, vec![covering()]);
        let mut over_the_stars = |layers: Vec<CloudLayerDraw>| {
            scene.update(&input(layers), 1.0, 0.0);
            render(&scene.rig, SKY_CLEAR, |pass| {
                stars.draw(pass);
                scene.clouds.draw(pass);
            })
        };

        let stars_alone = over_the_stars(Vec::new());
        let hidden = over_the_stars(vec![colored(0, Vec3::Y, 1.0)]);
        let shining_through = over_the_stars(vec![colored(0, Vec3::Y, 0.5)]);
        let invisible = over_the_stars(vec![colored(0, Vec3::Y, 0.0)]);

        let star = at(&stars_alone, SKY_CENTER).truncate();
        assert!(star.min_element() > 0.0, "{star}");
        assert_color(at(&hidden, SKY_CENTER), Vec3::Y);
        assert_color(at(&shining_through, SKY_CENTER), star * 0.5 + Vec3::Y * 0.5);
        assert_eq!(at(&invisible, SKY_CENTER), at(&stars_alone, SKY_CENTER));
    }

    #[test]
    fn clouds_hide_behind_geometry_that_wrote_depth() {
        let Some(rig) = rig("clouds_hide_behind_geometry_that_wrote_depth") else {
            return;
        };
        let mut scene = Scene::white(rig, vec![covering()]);
        scene.update(&input(vec![layer_draw(0, 1.0)]), 1.0, 0.0);

        let drawn = scene.draw(0.5);

        assert_black(&drawn);
    }

    #[test]
    fn clouds_show_after_a_world_pass_draw() {
        let Some(rig) = rig("clouds_show_after_a_world_pass_draw") else {
            return;
        };
        let mut scene = Scene::white(rig, vec![covering()]);
        scene.update(&input(vec![layer_draw(0, 1.0)]), 1.0, 0.0);
        let world = WorldDraw::new(&scene.rig);

        let drawn = render(&scene.rig, SKY_CLEAR, |pass| {
            world.draw(pass);
            scene.clouds.draw(pass);
        });

        assert_color(at(&drawn, SKY_CENTER), Vec3::ONE);
    }

    #[test]
    fn a_cloud_model_that_failed_to_load_draws_nothing_and_says_why() {
        let Some(rig) = rig("a_cloud_model_that_failed_to_load_draws_nothing_and_says_why") else {
            return;
        };
        let textures = Textures::white(&rig);
        let failure = Err(not_found("meshes\\sky\\clouds.glb"));
        let (mut scene, state) = Scene::install(rig, failure, textures);

        let drawn = scene.frame(&input(vec![layer_draw(0, 1.0)]));

        assert_black(&drawn);
        assert!(
            scene.textures.requested.is_empty(),
            "nothing to put a texture on"
        );
        assert!(matches!(
            state,
            AssetState::Failed(failure)
                if failure.kind == AssetFailureKind::NotFound
                    && failure.message.contains("clouds.glb")
        ));
    }

    fn write_white_texture(path: &std::path::Path) {
        let mut dds = ddsfile::Dds::new_dxgi(ddsfile::NewDxgiParams {
            height: 4,
            width: 4,
            depth: None,
            format: ddsfile::DxgiFormat::R8G8B8A8_UNorm,
            mipmap_levels: Some(1),
            array_layers: None,
            caps2: None,
            is_cubemap: false,
            resource_dimension: ddsfile::D3D10ResourceDimension::Texture2D,
            alpha_mode: ddsfile::AlphaMode::Unknown,
        })
        .unwrap();
        dds.data.fill(255);
        let mut file = std::fs::File::create(path).unwrap();
        dds.write(&mut file).unwrap();
    }

    /// An install with the model of [`geometry::clouds_glb`] below `Meshes\Sky` and the white
    /// textures `CloudsTest_d.DDS` and `Sun_d.DDS` below `Textures\Sky`.
    fn install() -> tempfile::TempDir {
        let root = geometry::install_with_clouds("Sky", "Clouds.glb");
        let textures = root.path().join("Textures").join("Sky");
        std::fs::create_dir_all(&textures).unwrap();
        write_white_texture(&textures.join("CloudsTest_d.DDS"));
        write_white_texture(&textures.join("Sun_d.DDS"));
        root
    }

    const INSTALLED_COLOR: Vec3 = Vec3::new(0.5, 0.25, 1.0);

    /// A layer of the texture `CloudsTest_d.DDS` of [`install`], whatever the case of `path`.
    fn installed_layer(path: &str) -> CloudLayerDraw {
        CloudLayerDraw {
            color: INSTALLED_COLOR,
            ..textured(0, path)
        }
    }

    fn sunny_sky() -> NightSkyInput {
        NightSkyInput {
            moon: None,
            star_rotation: StarRotation {
                axis: Vec3::Y,
                angle_rad: 0.0,
            },
            star_alpha: 0.0,
            sky_model: None,
            sun: SunInput {
                direction: Vec3::new(0.0, 0.5, -0.866_025_4),
                alpha: 1.0,
                disc_color: Vec3::ONE,
                glare_color: Vec3::ONE,
            },
            sun_textures: ClimateSunTextures::new(Some("Sky\\Sun_d.dds"), None),
        }
    }

    #[test]
    fn a_night_sky_loads_the_model_and_textures_of_an_install_and_draws_the_layers() {
        let name = "a_night_sky_loads_the_model_and_textures_of_an_install_and_draws_the_layers";
        let Some(rig) = rig(name) else {
            return;
        };
        let root = install();
        let mut sky = new_sky(&rig, root.path().to_str().unwrap());
        assert_eq!(sky.asset_status().cloud_mesh, AssetState::NotRequested);

        let drawn = night_sky_frame(
            &rig,
            &mut sky,
            &input(vec![installed_layer("SKY\\cloudstest_D.dds")]),
        );

        assert_eq!(sky.asset_status().cloud_mesh, AssetState::Loaded);
        assert_color(at(&drawn, SKY_CENTER), INSTALLED_COLOR);
    }

    #[test]
    fn a_night_sky_loads_no_cloud_model_until_a_layer_shows() {
        let Some(rig) = rig("a_night_sky_loads_no_cloud_model_until_a_layer_shows") else {
            return;
        };
        let root = install();
        let mut sky = new_sky(&rig, root.path().to_str().unwrap());

        let nothing = night_sky_frame(&rig, &mut sky, &CloudsInput::default());
        assert_eq!(sky.asset_status().cloud_mesh, AssetState::NotRequested);
        let shown = night_sky_frame(
            &rig,
            &mut sky,
            &input(vec![installed_layer("Sky\\CloudsTest_d.dds")]),
        );
        let gone = night_sky_frame(&rig, &mut sky, &CloudsInput::default());

        assert_black(&nothing);
        assert_color(at(&shown, SKY_CENTER), INSTALLED_COLOR);
        assert_black(&gone);
    }

    #[test]
    fn a_night_sky_without_the_cloud_model_in_its_install_says_so_and_draws_no_clouds() {
        let name = "a_night_sky_without_the_cloud_model_in_its_install_says_so_and_draws_no_clouds";
        let Some(rig) = rig(name) else {
            return;
        };
        let root = tempfile::tempdir().unwrap();
        let mut sky = new_sky(&rig, root.path().to_str().unwrap());

        let drawn = night_sky_frame(
            &rig,
            &mut sky,
            &input(vec![installed_layer("Sky\\CloudsTest_d.dds")]),
        );

        assert_black(&drawn);
        assert!(matches!(
            &sky.asset_status().cloud_mesh,
            AssetState::Failed(failure) if failure.kind == AssetFailureKind::NotFound
        ));
    }

    #[test]
    fn a_night_sky_loads_the_model_a_weather_names_when_it_changes() {
        let Some(rig) = rig("a_night_sky_loads_the_model_a_weather_names_when_it_changes") else {
            return;
        };
        let root = install();
        let mut sky = new_sky(&rig, root.path().to_str().unwrap());
        let mut weather = input(vec![installed_layer("Sky\\CloudsTest_d.dds")]);
        night_sky_frame(&rig, &mut sky, &weather);
        assert_eq!(sky.asset_status().cloud_mesh, AssetState::Loaded);

        weather.model = Some(Arc::from("Sky\\Other.nif"));
        let other = night_sky_frame(&rig, &mut sky, &weather);
        let missing = sky.asset_status().cloud_mesh.clone();
        weather.model = Some(Arc::from("sky\\clouds.nif"));
        let back = night_sky_frame(&rig, &mut sky, &weather);

        assert_black(&other);
        assert!(matches!(
            missing,
            AssetState::Failed(failure) if failure.kind == AssetFailureKind::NotFound
        ));
        assert_eq!(sky.asset_status().cloud_mesh, AssetState::Loaded);
        assert_color(at(&back, SKY_CENTER), INSTALLED_COLOR);
    }

    #[test]
    fn a_night_sky_draws_the_clouds_over_the_sun() {
        let Some(rig) = rig("a_night_sky_draws_the_clouds_over_the_sun") else {
            return;
        };
        let root = install();
        let mut sky = new_sky(&rig, root.path().to_str().unwrap());
        sky.update(&rig.device, &rig.queue, &sunny_sky(), 1.0);
        let feedback = feedback_group(&rig);
        let sun_alone = render(&rig, SKY_CLEAR, |pass| sky.draw(pass, &feedback));
        assert!(at(&sun_alone, SUN_PIXEL).truncate().min_element() > 0.0);

        let covered = night_sky_frame(
            &rig,
            &mut sky,
            &input(vec![installed_layer("Sky\\CloudsTest_d.dds")]),
        );

        assert_color(at(&covered, SUN_PIXEL), INSTALLED_COLOR);
    }
}

/// Developer checks against a real install (`OPENFO4_INSTALL_DIR`), run with `--ignored`.
mod real_install {
    use super::*;

    const LAYER_COUNT: usize = 16;

    fn asset_root() -> String {
        let install = std::env::var(crate::assets::INSTALL_DIR_VAR).expect("OPENFO4_INSTALL_DIR");
        format!("{install}/{}", crate::assets::layout::ASSETS_DIR)
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the converted sky meshes"]
    fn the_installed_cloud_model_has_one_triangle_mesh_for_each_of_the_sixteen_layers() {
        let geometry = load_cloud_geometry(&asset_root(), DEFAULT_MODEL).unwrap();

        assert_eq!(geometry.ranges.len(), LAYER_COUNT);
        assert_eq!(geometry.ranges[0].start, 0);
        assert!(
            geometry
                .ranges
                .windows(2)
                .all(|pair| pair[0].end == pair[1].start)
        );
        assert_eq!(
            geometry.ranges[LAYER_COUNT - 1].end as usize,
            geometry.indices.len()
        );
        let vertex_count = geometry.vertices.len() as u32;
        assert!(geometry.indices.iter().all(|&index| index < vertex_count));
        for (layer, range) in geometry.ranges.iter().enumerate() {
            assert!(
                !range.is_empty() && range.len() % 3 == 0,
                "mesh {layer}: {range:?}"
            );
        }
        let faded = geometry
            .vertices
            .iter()
            .filter(|vertex| vertex.color[3] < 255)
            .count();
        assert!(faded > 0, "the meshes fade out at their edges");
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the converted sky textures"]
    fn every_installed_cloud_texture_loads_and_a_mistyped_path_is_not_found() {
        let name = "every_installed_cloud_texture_loads_and_a_mistyped_path_is_not_found";
        let Some(rig) = rig(name) else {
            return;
        };
        let root = asset_root();
        let load =
            |path: &str| load_texture(&rig.device, &rig.queue, &root, &named_texture_key(path));
        let folder = std::fs::read_dir(format!("{root}/textures/sky")).unwrap();
        let cloud_files: Vec<String> = folder
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|file| {
                let lower = file.to_ascii_lowercase();
                let is_dds = std::path::Path::new(&lower)
                    .extension()
                    .is_some_and(|extension| extension == "dds");
                is_dds && (lower.contains("cloud") || lower.starts_with("gs0"))
            })
            .collect();

        assert!(cloud_files.len() > 10, "{cloud_files:?}");
        for file in &cloud_files {
            let loaded = load(&format!("Sky\\{file}"));
            assert!(loaded.is_ok(), "{file}: {:?}", loaded.err());
        }
        let error = load("SkySky\\GS01_d.dds").unwrap_err();
        assert!(matches!(error, SkyAssetError::NotFound { .. }), "{error}");
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the converted sky meshes and textures"]
    fn the_installed_clouds_are_drawn_over_the_view() {
        let name = "the_installed_clouds_are_drawn_over_the_view";
        let Some(rig) = rig(name) else {
            return;
        };
        let mut sky = new_sky(&rig, &asset_root());
        let layers = (0..LAYER_COUNT as u8)
            .map(|layer| CloudLayerDraw {
                texture: Arc::from("Sky\\CloudsLower01_d.dds"),
                ..layer_draw(layer, 1.0)
            })
            .collect();

        let drawn = night_sky_frame(&rig, &mut sky, &input(layers));

        assert_eq!(sky.asset_status().cloud_mesh, AssetState::Loaded);
        let lit = drawn.iter().filter(|texel| texel.x > 0.0).count();
        assert!(lit > 0, "no pixel of {} is lit", drawn.len());
    }
}
