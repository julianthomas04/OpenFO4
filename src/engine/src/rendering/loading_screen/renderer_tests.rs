//! The loading art renderer on the shared headless GPU device: what ends up in the pixels.

use glam::UVec2;

use super::test_support::{cube_art, layered_quads_art, write_texture};
use super::{CpuArt, LoadedArt, LoadingArt, LoadingArtRenderer, PanKey};
use crate::rendering::gpu_test::{linear_to_srgb_byte, read_mip, test_device};
use crate::rendering::post::tonemap::ui_gamma_format;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const SIZE: (u32, u32) = (320, 180);
const BLACK: wgpu::Color = wgpu::Color::BLACK;
/// The cube's front face is lit by the ambient term and the light at an angle of
/// `cos = 0.7 / |(-0.4, 0.6, 0.7)|`; a white texel shows `0.35 + 0.75 * cos` encoded to sRGB.
const FRONT_FACE_LINEAR: f32 = 0.35 + 0.75 * 0.696_5;

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
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[ui_gamma_format(FORMAT)],
        });
        let target = Self { texture, size };
        target.clear(scene, clear);
        target
    }

    /// The view the menu draws through: the swapchain without its sRGB encoding.
    fn view(&self) -> wgpu::TextureView {
        self.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(ui_gamma_format(FORMAT)),
            ..Default::default()
        })
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

struct Pixels {
    bytes: Vec<u8>,
    size: (u32, u32),
}

impl Pixels {
    fn read(device: &wgpu::Device, queue: &wgpu::Queue, target: &Target) -> Self {
        Self {
            bytes: read_mip(
                device,
                queue,
                &target.texture,
                0,
                UVec2::new(target.size.0, target.size.1),
                4,
            ),
            size: target.size,
        }
    }

    fn at(&self, x: u32, y: u32) -> [u8; 3] {
        let start = ((y * self.size.0 + x) * 4) as usize;
        [
            self.bytes[start],
            self.bytes[start + 1],
            self.bytes[start + 2],
        ]
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
        let (device, queue) = test_device(test)?;
        let renderer = LoadingArtRenderer::new(&device, &queue, ui_gamma_format(FORMAT));
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
        self.renderer
            .render(&mut encoder, &target.view(), target.size);
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
fn a_cube_draws_in_the_centre_lit_and_encoded_and_leaves_the_corners_black() {
    let Some(mut scene) = Scene::new("loading art cube") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    scene.show(&cube_art(dir.path(), ""));
    let target = Target::new(&scene, SIZE, BLACK);

    let pixels = scene.draw(&target);

    let expected = linear_to_srgb_byte(FRONT_FACE_LINEAR);
    for channel in pixels.centre() {
        assert_close(channel, expected, 4);
    }
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

    let expected = linear_to_srgb_byte(FRONT_FACE_LINEAR);
    for channel in pixels.centre() {
        assert_close(channel, expected, 4);
    }
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

    let near = linear_to_srgb_byte(FRONT_FACE_LINEAR);
    for channel in pixels.centre() {
        assert_close(channel, near, 3);
    }
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
