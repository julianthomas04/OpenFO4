//! The loading art renderer on the shared headless GPU device: what ends up in the pixels.

use glam::UVec2;

use super::test_support::cube_art;
use super::{CpuArt, LoadedArt, LoadingArt, LoadingArtRenderer};
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

    fn draw(&mut self, target: &Target, alpha: f32) -> Pixels {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        self.renderer
            .render(&mut encoder, &target.view(), target.size, alpha);
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

    let pixels = scene.draw(&target, 1.0);

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

    let pixels = scene.draw(&target, 1.0);

    let expected = linear_to_srgb_byte(FRONT_FACE_LINEAR);
    for channel in pixels.centre() {
        assert_close(channel, expected, 4);
    }
}

#[test]
fn the_model_fades_with_alpha_and_alpha_zero_draws_nothing() {
    let Some(mut scene) = Scene::new("loading art alpha") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    scene.show(&cube_art(dir.path(), ""));
    let green = wgpu::Color {
        r: 0.0,
        g: 0.4,
        b: 0.0,
        a: 1.0,
    };
    let target = Target::new(&scene, SIZE, green);
    let background = scene.draw(&target, 0.0);
    assert_eq!(
        background.centre(),
        background.at(0, 0),
        "alpha 0 changes nothing"
    );

    let faded = scene.draw(&target, 0.5);

    let face = linear_to_srgb_byte(FRONT_FACE_LINEAR);
    let [red, green_channel, _] = faded.centre();
    assert_close(red, face / 2, 4);
    assert!(
        green_channel > red,
        "half of the green background still shows: {green_channel}"
    );
}

#[test]
fn nothing_draws_without_art_or_after_it_is_cleared() {
    let Some(mut scene) = Scene::new("loading art cleared") else {
        return;
    };
    let target = Target::new(&scene, SIZE, BLACK);
    assert_eq!(scene.draw(&target, 1.0).lit_count(), 0, "no art yet");

    let dir = tempfile::tempdir().unwrap();
    scene.show(&cube_art(dir.path(), ""));
    assert!(scene.draw(&target, 1.0).lit_count() > 0);
    scene.renderer.set_art(None);
    target.clear(&scene, BLACK);
    assert_eq!(scene.draw(&target, 1.0).lit_count(), 0, "art cleared");
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
    let head_on = scene.draw(&target, 1.0).lit_width();

    scene.renderer.drag(100_000.0);
    target.clear(&scene, BLACK);
    let turned = scene.draw(&target, 1.0).lit_width();

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
    let before = scene.draw(&target, 1.0).lit_count();

    scene.renderer.wheel(-50.0);
    target.clear(&scene, BLACK);
    let zoomed_out = scene.draw(&target, 1.0).lit_count();
    scene.renderer.wheel(5.0);
    target.clear(&scene, BLACK);
    let zoomed_in = scene.draw(&target, 1.0).lit_count();

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
        assert!(scene.draw(&target, 1.0).lit_count() > 0, "{size:?}");
    }

    let error = pollster::block_on(error_scope.pop());
    assert!(error.is_none(), "{error:?}");
}
