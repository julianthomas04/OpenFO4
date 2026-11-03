//! The loading art renderer on the shared headless GPU device: what ends up in the pixels.

use glam::{UVec2, Vec3};

use super::stage_framing::test_support::vanilla_framing;
use super::test_support::{cube_art, layered_quads_art, write_solid_texture, write_texture};
use super::{
    CpuArt, LoadedArt, LoadingArt, LoadingArtRenderer, PICTURE_FORMAT, PanKey, SceneLights,
};
use crate::bethesda_space::{UNITS_TO_ENGINE, position_to_engine};
use crate::rendering::gpu_test::{f16_bits_to_f32, linear_to_srgb_byte, read_mip, test_device};
use crate::rendering::lighting::cell_lighting::srgb_to_linear;
use crate::rendering::placed_light::linear_colour;

const SIZE: (u32, u32) = (320, 180);
const BLACK: wgpu::Color = wgpu::Color::BLACK;
/// How far the cube's front face is from the camera, in engine units: the cube's centre is at
/// 30 and its half edge 5.
const FRONT_FACE_DEPTH: f32 = 25.0;
/// Half the cube's edge in engine units.
const CUBE_HALF_EDGE: f32 = 5.0;
/// The diffuse of a fully rough surface seen along its normal, relative to Lambert:
/// `1 - 0.5 * 1 / (1 + 0.57)` from the world's roughness coefficients.
const ROUGH_FLATTEN: f32 = 0.681_5;

/// The linear colour of the cube's front face lit by `lights`' third light alone: the other two
/// stand behind the face. The world's local light on a matte white texel seen head-on: the colour
/// times the falloff `(1 - (distance / radius)^2)^2.2` times the rough diffuse times the cosine.
fn front_face_linear(lights: &SceneLights) -> [f32; 3] {
    let fill = lights.lights[2];
    let [x, y, z] = fill.position.to_array();
    let to_light = position_to_engine(x, y, z) - Vec3::new(0.0, 0.0, -FRONT_FACE_DEPTH);
    let distance = to_light.length();
    let normalised = distance / (fill.radius * UNITS_TO_ENGINE);
    let falloff = (1.0 - normalised * normalised).max(0.0).powf(2.2);
    let cosine = to_light.z / distance;
    let cone = spot_cone(fill.position, fill.spot_fov, to_light);
    (linear_colour(fill.diffuse, fill.dimmer) * (falloff * cone * ROUGH_FLATTEN * cosine))
        .to_array()
}

/// The share of a spotlight's light that reaches the front face's centre: the light aims at the
/// cube's centre, and the world's cone falloff (exponent 1) rescales the cosine off the axis from
/// the cone's edge to its axis.
fn spot_cone(position: Vec3, full_angle_degrees: f32, to_light: Vec3) -> f32 {
    let [x, y, z] = position.to_array();
    let light = position_to_engine(x, y, z);
    let axis = (Vec3::new(0.0, 0.0, -FRONT_FACE_DEPTH - CUBE_HALF_EDGE) - light).normalize();
    let cos_axis = -to_light.normalize().dot(axis);
    let cos_outer = (full_angle_degrees.to_radians() * 0.5).cos();
    (1.0 - (1.0 - cos_axis) / (1.0 - cos_outer)).clamp(0.0, 1.0)
}

fn front_face_bytes() -> [u8; 3] {
    front_face_linear(&SceneLights::VANILLA).map(linear_to_srgb_byte)
}

fn assert_close_colour(actual: [u8; 3], expected: [u8; 3], tolerance: u8) {
    for (a, e) in actual.into_iter().zip(expected) {
        assert_close(a, e, tolerance);
    }
}

struct Target {
    texture: wgpu::Texture,
    size: (u32, u32),
}

impl Target {
    fn new(scene: &Scene, size: (u32, u32), clear: wgpu::Color) -> Self {
        let texture = scene.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("loading_art_test_target"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: PICTURE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target = Self { texture, size };
        target.clear(scene, clear);
        target
    }

    /// The view the loading screen draws its linear picture through.
    fn view(&self) -> wgpu::TextureView {
        self.texture
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn clear(&self, scene: &Scene, color: wgpu::Color) {
        let mut encoder = scene
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let view = self.view();
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(color),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        }));
        scene.queue.submit(std::iter::once(encoder.finish()));
    }
}

/// The picture read back, each channel sRGB-encoded to a byte so that it compares like the
/// display's colour.
struct Pixels {
    bytes: Vec<u8>,
    size: (u32, u32),
}

const BYTES_PER_TEXEL: usize = 8;

impl Pixels {
    fn read(device: &wgpu::Device, queue: &wgpu::Queue, target: &Target) -> Self {
        Self {
            bytes: read_mip(
                device,
                queue,
                &target.texture,
                0,
                UVec2::new(target.size.0, target.size.1),
                BYTES_PER_TEXEL as u32,
            ),
            size: target.size,
        }
    }

    fn at(&self, x: u32, y: u32) -> [u8; 3] {
        let start = (y * self.size.0 + x) as usize * BYTES_PER_TEXEL;
        [0, 1, 2].map(|channel| {
            let at = start + channel * 2;
            let half = u16::from_le_bytes([self.bytes[at], self.bytes[at + 1]]);
            linear_to_srgb_byte(f16_bits_to_f32(half))
        })
    }

    fn centre(&self) -> [u8; 3] {
        self.at(self.size.0 / 2, self.size.1 / 2)
    }

    /// The width in pixels of the columns that hold anything but black.
    fn lit_width(&self) -> u32 {
        let lit_columns: Vec<u32> = (0..self.size.0)
            .filter(|&x| (0..self.size.1).any(|y| self.at(x, y) != [0, 0, 0]))
            .collect();
        match (lit_columns.first(), lit_columns.last()) {
            (Some(first), Some(last)) => last - first + 1,
            _ => 0,
        }
    }

    fn lit_count(&self) -> usize {
        (0..self.size.1)
            .flat_map(|y| (0..self.size.0).map(move |x| (x, y)))
            .filter(|&(x, y)| self.at(x, y) != [0, 0, 0])
            .count()
    }
}

struct Scene {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: LoadingArtRenderer,
}

impl Scene {
    fn new(test: &str) -> Option<Self> {
        Self::lit_by(test, &SceneLights::VANILLA)
    }

    fn lit_by(test: &str, lights: &SceneLights) -> Option<Self> {
        let (device, queue) = test_device(test)?;
        let renderer = LoadingArtRenderer::new(&device, &queue, PICTURE_FORMAT, lights);
        Some(Self {
            device,
            queue,
            renderer,
        })
    }

    fn show(&mut self, art: &LoadingArt) {
        let cpu = CpuArt::load(art).expect("the synthetic cube loads");
        let loaded = LoadedArt::upload(&self.device, &self.queue, cpu).expect("the cube uploads");
        self.renderer.set_art(Some(loaded));
    }

    fn draw(&mut self, target: &Target) -> Pixels {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.renderer.render(
            &mut encoder,
            &target.view(),
            target.size,
            vanilla_framing().vertical_fov_radians,
        );
        self.queue.submit(std::iter::once(encoder.finish()));
        Pixels::read(&self.device, &self.queue, target)
    }
}

fn assert_close(actual: u8, expected: u8, tolerance: u8) {
    assert!(
        actual.abs_diff(expected) <= tolerance,
        "expected about {expected}, got {actual}"
    );
}

#[test]
fn a_cube_draws_in_the_centre_lit_and_leaves_the_corners_black() {
    let Some(mut scene) = Scene::new("loading art cube") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    scene.show(&cube_art(dir.path(), ""));
    let target = Target::new(&scene, SIZE, BLACK);

    let pixels = scene.draw(&target);

    assert_close_colour(pixels.centre(), front_face_bytes(), 4);
    let (width, height) = SIZE;
    for corner in [
        (0, 0),
        (width - 1, 0),
        (0, height - 1),
        (width - 1, height - 1),
    ] {
        assert_eq!(pixels.at(corner.0, corner.1), [0, 0, 0], "{corner:?}");
    }
}

#[test]
fn a_part_whose_texture_is_missing_draws_white() {
    let Some(mut scene) = Scene::new("loading art missing texture") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    scene.show(&cube_art(dir.path(), "no_such_folder\\no_such_texture.dds"));
    let target = Target::new(&scene, SIZE, BLACK);

    let pixels = scene.draw(&target);

    assert_close_colour(pixels.centre(), front_face_bytes(), 4);
}

#[test]
fn nothing_draws_without_art_or_after_it_is_cleared() {
    let Some(mut scene) = Scene::new("loading art cleared") else {
        return;
    };
    let target = Target::new(&scene, SIZE, BLACK);
    assert_eq!(scene.draw(&target).lit_count(), 0, "no art yet");

    let dir = tempfile::tempdir().unwrap();
    scene.show(&cube_art(dir.path(), ""));
    assert!(scene.draw(&target).lit_count() > 0);
    scene.renderer.set_art(None);
    target.clear(&scene, BLACK);
    assert_eq!(scene.draw(&target).lit_count(), 0, "art cleared");
}

#[test]
fn dragging_turns_the_cube_within_its_limits() {
    let Some(mut scene) = Scene::new("loading art drag") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut art = cube_art(dir.path(), "");
    art.rotation_limits_deg = (-45, 45);
    scene.show(&art);
    let target = Target::new(&scene, SIZE, BLACK);
    let head_on = scene.draw(&target).lit_width();

    scene.renderer.drag(100_000.0, 0.0);
    target.clear(&scene, BLACK);
    let turned = scene.draw(&target).lit_width();

    assert_eq!(scene.renderer.camera().unwrap().yaw_degrees().round(), 45.0);
    assert!(
        turned * 10 > head_on * 11,
        "a cube turned by 45 degrees shows its diagonal: {head_on} -> {turned} px wide"
    );
}

#[test]
fn a_model_that_cannot_be_zoomed_in_on_keeps_its_size() {
    let Some(mut scene) = Scene::new("loading art zoom") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut art = cube_art(dir.path(), "");
    art.zoom_limits = (0.0, 1.0);
    // Far enough that the cube still fits the frame when zoomed in to half the distance.
    art.transform.position = [0.0, 660.0, 0.0];
    scene.show(&art);
    let target = Target::new(&scene, SIZE, BLACK);
    let before = scene.draw(&target).lit_count();

    scene.renderer.drag(0.0, 10_000.0);
    target.clear(&scene, BLACK);
    let zoomed_out = scene.draw(&target).lit_count();
    scene.renderer.drag(0.0, -1_000.0);
    target.clear(&scene, BLACK);
    let zoomed_in = scene.draw(&target).lit_count();

    assert_eq!(
        zoomed_out, before,
        "the lower limit is the stored placement"
    );
    assert!(zoomed_in > before * 2, "{before} -> {zoomed_in}");
}

#[test]
fn resizing_the_target_recreates_the_depth_buffer_without_a_validation_error() {
    let Some(mut scene) = Scene::new("loading art resize") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    scene.show(&cube_art(dir.path(), ""));
    let error_scope = scene.device.push_error_scope(wgpu::ErrorFilter::Validation);

    for size in [SIZE, (200, 200), SIZE] {
        let target = Target::new(&scene, size, BLACK);
        assert!(scene.draw(&target).lit_count() > 0, "{size:?}");
    }

    let error = pollster::block_on(error_scope.pop());
    assert!(error.is_none(), "{error:?}");
}

#[test]
fn overlapping_surfaces_show_the_nearest_whatever_the_draw_order() {
    let Some(mut scene) = Scene::new("loading art layered surfaces") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    scene.show(&layered_quads_art(dir.path()));
    let target = Target::new(&scene, SIZE, BLACK);

    let pixels = scene.draw(&target);

    assert_close_colour(pixels.centre(), front_face_bytes(), 3);
}

/// How many pixels the cube lights when its texture has `alpha` and its material is `tokens`;
/// `None` without a GPU.
fn lit_pixels_with(test: &str, alpha: u8, tokens: &str) -> Option<usize> {
    let mut scene = Scene::new(test)?;
    let dir = tempfile::tempdir().unwrap();
    write_texture(dir.path(), "leaf.dds", alpha);
    scene.show(&cube_art(dir.path(), &format!("leaf.dds{tokens}")));
    let target = Target::new(&scene, SIZE, BLACK);
    Some(scene.draw(&target).lit_count())
}

#[test]
fn a_material_with_an_alpha_test_cuts_out_texels_below_its_cutoff() {
    // 100/255 is below 128/255.
    let Some(lit) = lit_pixels_with("loading art alpha cut", 100, "|test:128") else {
        return;
    };
    assert_eq!(lit, 0, "the whole cube is below its cutoff");
}

#[test]
fn a_material_with_an_alpha_test_keeps_texels_above_its_cutoff() {
    // 200/255 and 100/255 are both above 50/255; 200/255 is above 128/255.
    let (Some(high), Some(low)) = (
        lit_pixels_with("loading art alpha keep high", 200, "|test:128"),
        lit_pixels_with("loading art alpha keep low", 100, "|test:50"),
    ) else {
        return;
    };
    assert!(high > 0 && low > 0, "{high} {low}");
}

#[test]
fn a_texel_exactly_at_the_cutoff_is_kept() {
    let Some(lit) = lit_pixels_with("loading art alpha boundary", 51, "|test:51") else {
        return;
    };
    assert!(lit > 0, "only texels below the cutoff are cut out");
}

#[test]
fn a_material_without_an_alpha_test_never_discards() {
    let Some(lit) = lit_pixels_with("loading art no alpha test", 0, "") else {
        return;
    };
    assert!(
        lit > 0,
        "a fully transparent texture still draws without a test token"
    );
}

impl Pixels {
    /// The mean column of the pixels that hold anything but black.
    fn lit_centre_x(&self) -> f32 {
        let (sum, count) = (0..self.size.1)
            .flat_map(|y| (0..self.size.0).map(move |x| (x, y)))
            .filter(|&(x, y)| self.at(x, y) != [0, 0, 0])
            .fold((0.0_f32, 0.0_f32), |(sum, count), (x, _)| {
                (sum + x as f32, count + 1.0)
            });
        sum / count
    }
}

#[test]
fn a_held_pan_key_moves_the_drawn_model_across_the_screen() {
    let Some(mut scene) = Scene::new("loading art pan") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    scene.show(&cube_art(dir.path(), ""));
    let target = Target::new(&scene, SIZE, BLACK);
    let before = scene.draw(&target).lit_centre_x();

    scene.renderer.set_pan_key(PanKey::Right, true);
    scene.renderer.tick(0.25);
    scene.renderer.set_pan_key(PanKey::Right, false);
    target.clear(&scene, BLACK);
    let right = scene.draw(&target).lit_centre_x();
    scene.renderer.set_pan_key(PanKey::Left, true);
    scene.renderer.tick(0.25);
    scene.renderer.tick(0.25);
    target.clear(&scene, BLACK);
    let left = scene.draw(&target).lit_centre_x();

    assert!(right > before + 5.0, "{before} -> {right}");
    assert!(left < before - 5.0, "{before} -> {left}");
}

/// The centre pixel of the cube whose material is `material`, below `textures` (name and
/// texel); `None` without a GPU.
fn centre_with(test: &str, material: &str, textures: &[(&str, [u8; 4])]) -> Option<[u8; 3]> {
    let mut scene = Scene::new(test)?;
    let dir = tempfile::tempdir().unwrap();
    for (name, texel) in textures {
        write_solid_texture(dir.path(), name, *texel);
    }
    scene.show(&cube_art(dir.path(), material));
    let target = Target::new(&scene, SIZE, BLACK);
    Some(scene.draw(&target).centre())
}

const MID_GREY: [u8; 4] = [128, 128, 128, 255];
const SPECULAR_TOKENS: &str = "|spec:spec.dds|sm:1|sg:0.0";

fn assert_same_colour(actual: [u8; 3], expected: [u8; 3]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert_close(actual, expected, 1);
    }
}

#[test]
fn a_specular_map_adds_a_highlight_in_the_colour_of_the_light() {
    let (Some(matte), Some(shiny)) = (
        centre_with("loading art matte", "grey.dds", &[("grey.dds", MID_GREY)]),
        centre_with(
            "loading art specular",
            &format!("grey.dds{SPECULAR_TOKENS}"),
            &[("grey.dds", MID_GREY), ("spec.dds", [255; 4])],
        ),
    ) else {
        return;
    };
    assert!(
        shiny[0] > matte[0] + 3 && shiny[2] > matte[2] + 3,
        "{matte:?} -> {shiny:?}"
    );
    let added = |channel: usize| srgb_to_linear(shiny[channel]) - srgb_to_linear(matte[channel]);
    let fill = SceneLights::VANILLA.lights[2];
    let light = linear_colour(fill.diffuse, fill.dimmer);
    let (added_ratio, light_ratio) = (added(0) / added(2), light.x / light.z);
    assert!(
        (added_ratio - light_ratio).abs() < 0.05 * light_ratio,
        "{matte:?} -> {shiny:?}: the highlight has the light's red to blue {light_ratio}, not {added_ratio}"
    );
}

#[test]
fn a_specular_map_without_intensity_adds_nothing() {
    let (Some(matte), Some(dull)) = (
        centre_with(
            "loading art matte again",
            "grey.dds",
            &[("grey.dds", MID_GREY)],
        ),
        centre_with(
            "loading art black specular",
            &format!("grey.dds{SPECULAR_TOKENS}"),
            &[("grey.dds", MID_GREY), ("spec.dds", [0, 255, 0, 255])],
        ),
    ) else {
        return;
    };
    assert_same_colour(dull, matte);
}

#[test]
fn a_specular_map_that_cannot_be_loaded_switches_the_specular_off() {
    let (Some(matte), Some(unloaded)) = (
        centre_with(
            "loading art matte third",
            "grey.dds",
            &[("grey.dds", MID_GREY)],
        ),
        centre_with(
            "loading art missing specular",
            &format!("grey.dds{SPECULAR_TOKENS}"),
            &[("grey.dds", MID_GREY)],
        ),
    ) else {
        return;
    };
    assert_same_colour(unloaded, matte);
}

#[test]
fn a_material_without_specular_parameters_stays_matte_whatever_its_specular_map() {
    let (Some(matte), Some(unmarked)) = (
        centre_with(
            "loading art matte fourth",
            "grey.dds",
            &[("grey.dds", MID_GREY)],
        ),
        centre_with(
            "loading art legacy specular",
            "grey.dds|spec:spec.dds",
            &[("grey.dds", MID_GREY), ("spec.dds", [255; 4])],
        ),
    ) else {
        return;
    };
    assert_same_colour(unmarked, matte);
}

/// A normal map texel whose normal leans by `lean` along the surface's U axis.
fn leaning_normal(lean: f32) -> [u8; 4] {
    let encode = |component: f32| ((component * 0.5 + 0.5) * 255.0).round() as u8;
    [encode(lean), encode(0.0), 255, 255]
}

#[test]
fn a_normal_map_changes_how_the_surface_catches_the_light() {
    let lit = |test: &str, lean: Option<f32>| {
        let material = if lean.is_some() {
            "grey.dds|normal.dds"
        } else {
            "grey.dds"
        };
        let mut textures = vec![("grey.dds", MID_GREY)];
        if let Some(lean) = lean {
            textures.push(("normal.dds", leaning_normal(lean)));
        }
        centre_with(test, material, &textures)
    };
    let (Some(flat), Some(one_way), Some(other_way)) = (
        lit("loading art flat", None),
        lit("loading art lean one way", Some(0.6)),
        lit("loading art lean other way", Some(-0.6)),
    ) else {
        return;
    };
    let (brighter, darker) = if one_way[0] > other_way[0] {
        (one_way, other_way)
    } else {
        (other_way, one_way)
    };
    // The one light that reaches the face stands a little to one side of the view axis, so
    // leaning towards it brightens less than leaning away darkens.
    assert!(
        brighter[0] > flat[0] && darker[0] + 10 < flat[0],
        "flat {flat:?}, leaning {one_way:?} and {other_way:?}"
    );
}

#[test]
fn a_normal_map_that_cannot_be_loaded_leaves_the_surface_flat() {
    let (Some(flat), Some(unloaded)) = (
        centre_with(
            "loading art flat again",
            "grey.dds",
            &[("grey.dds", MID_GREY)],
        ),
        centre_with(
            "loading art missing normal",
            "grey.dds|normal.dds",
            &[("grey.dds", MID_GREY)],
        ),
    ) else {
        return;
    };
    assert_same_colour(unloaded, flat);
}

fn centre_lit_by(test: &str, lights: &SceneLights) -> Option<[u8; 3]> {
    let mut scene = Scene::lit_by(test, lights)?;
    let dir = tempfile::tempdir().unwrap();
    scene.show(&cube_art(dir.path(), ""));
    let target = Target::new(&scene, SIZE, BLACK);
    Some(scene.draw(&target).centre())
}

fn with_fill(change: impl FnOnce(&mut super::scene_lights::SceneLight)) -> SceneLights {
    let mut lights = SceneLights::VANILLA;
    change(&mut lights.lights[2]);
    lights
}

fn linear_centre(test: &str, lights: &SceneLights) -> Option<[f32; 3]> {
    centre_lit_by(test, lights).map(|bytes| bytes.map(srgb_to_linear))
}

#[test]
fn a_head_on_face_shows_the_worlds_local_light_of_the_fill_light() {
    let Some(centre) = linear_centre("loading art fill light", &SceneLights::VANILLA) else {
        return;
    };

    for (channel, (actual, expected)) in centre
        .into_iter()
        .zip(front_face_linear(&SceneLights::VANILLA))
        .enumerate()
    {
        assert!(
            (actual - expected).abs() < 0.01,
            "channel {channel}: {actual} is not {expected}"
        );
    }
}

#[test]
fn doubling_the_dimmer_doubles_the_radiance() {
    let doubled = with_fill(|fill| fill.dimmer *= 2.0);
    let (Some(single), Some(double)) = (
        linear_centre("loading art dimmer one", &SceneLights::VANILLA),
        linear_centre("loading art dimmer two", &doubled),
    ) else {
        return;
    };

    for channel in 0..3 {
        let ratio = double[channel] / single[channel];
        assert!(
            (ratio - 2.0).abs() < 0.05,
            "channel {channel}: {single:?} -> {double:?}"
        );
    }
}

#[test]
fn a_light_whose_radius_ends_before_the_model_lights_nothing() {
    let short = with_fill(|fill| fill.radius = 50.0);

    let Some(centre) = centre_lit_by("loading art short radius", &short) else {
        return;
    };

    assert_eq!(centre, [0, 0, 0]);
}

#[test]
fn a_light_behind_the_face_does_not_light_it() {
    let behind = with_fill(|fill| fill.position = Vec3::new(400.0, 1000.0, 220.0));

    let Some(centre) = centre_lit_by("loading art light behind", &behind) else {
        return;
    };

    assert_eq!(centre, [0, 0, 0]);
}

#[test]
fn a_nearer_light_is_brighter_than_a_farther_one() {
    let near = with_fill(|fill| fill.position = Vec3::new(0.0, -300.0, 0.0));
    let far = with_fill(|fill| fill.position = Vec3::new(0.0, -2500.0, 0.0));
    let (Some(near), Some(far)) = (
        linear_centre("loading art near light", &near),
        linear_centre("loading art far light", &far),
    ) else {
        return;
    };

    assert!(near[1] > far[1] + 0.05, "{near:?} against {far:?}");
}
