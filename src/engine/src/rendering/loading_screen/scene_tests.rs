//! The loading screen's stage and colour pass on the shared headless GPU device: what ends up in
//! the pixels.

use std::path::Path;

use glam::{UVec2, Vec3};

use super::grade::{GradeLook, INFERRED_LIFT, LoadScreenLut};
use super::grade_pass::GradePass;
use super::test_support::{cube_art, graded, install_stage, lamp_shadow_config};
use super::{
    CpuArt, GradeSource, LoadedArt, LoadingArtRenderer, LoadingScene, ModelControls,
    PICTURE_FORMAT, SceneLights,
};
use crate::assets::ini::GameIni;
use crate::rendering::gpu_test::{read_mip, test_device};
use crate::rendering::lighting::cell_lighting::srgb_unit_to_linear;
use crate::rendering::lighting::tonemap_settings::ImageSpaceCinematic;
use crate::rendering::post::tonemap::lut_table::{LUT_SIZE, LutTable, strip_dds};

/// The menu's target: the plain view of an 8 bit swapchain, holding encoded colour.
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const SIZE: (u32, u32) = (64, 36);
const RED: [u8; 4] = [255, 0, 0, 255];

struct Target {
    texture: wgpu::Texture,
    size: (u32, u32),
}

impl Target {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, size: (u32, u32)) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("loading_scene_test_target"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let target = Self { texture, size };
        let texels: Vec<u8> = RED.repeat((size.0 * size.1) as usize);
        queue.write_texture(
            target.texture.as_image_copy(),
            &texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.0 * 4),
                rows_per_image: Some(size.1),
            },
            target.texture.size(),
        );
        target
    }

    fn view(&self) -> wgpu::TextureView {
        self.texture
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn pixel(&self, device: &wgpu::Device, queue: &wgpu::Queue, x: u32, y: u32) -> [u8; 3] {
        let bytes = read_mip(
            device,
            queue,
            &self.texture,
            0,
            UVec2::new(self.size.0, self.size.1),
            4,
        );
        let at = ((y * self.size.0 + x) * 4) as usize;
        [bytes[at], bytes[at + 1], bytes[at + 2]]
    }

    fn corners_and_centre(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> [[u8; 3]; 5] {
        let (width, height) = self.size;
        [
            (0, 0),
            (width - 1, 0),
            (0, height - 1),
            (width - 1, height - 1),
            (width / 2, height / 2),
        ]
        .map(|(x, y)| self.pixel(device, queue, x, y))
    }
}

fn bytes(colour: Vec3) -> [u8; 3] {
    colour
        .to_array()
        .map(|channel| (channel * 255.0).round().clamp(0.0, 255.0) as u8)
}

fn decoded(rgb: [u8; 3]) -> Vec3 {
    Vec3::from(rgb.map(|byte| srgb_unit_to_linear(f32::from(byte) / 255.0)))
}

fn assert_close(actual: [u8; 3], expected: [u8; 3], context: &str) {
    for (a, e) in actual.into_iter().zip(expected) {
        assert!(
            a.abs_diff(e) <= 2,
            "{context}: {actual:?} is not {expected:?}"
        );
    }
}

/// The scene's grade without a database: no cinematic stage and no LUT file.
fn neutral_look() -> GradeLook {
    GradeLook {
        cinematic: ImageSpaceCinematic::NEUTRAL,
    }
}

/// The stage's background in the picture: its colour plus the inferred lift.
fn lifted_background() -> Vec3 {
    decoded(BACKGROUND) + Vec3::splat(INFERRED_LIFT)
}

fn scene(device: &wgpu::Device, queue: &wgpu::Queue, root: &Path) -> LoadingScene {
    let mut scene = LoadingScene::new(
        device,
        queue,
        TARGET_FORMAT,
        root,
        GradeSource {
            lut: LoadScreenLut::from_ini(&GameIni::from_texts([""])).unwrap(),
            db_path: String::new(),
        },
    );
    scene.prepare().expect("the stage loads");
    scene
}

fn empty_art_renderer(device: &wgpu::Device, queue: &wgpu::Queue) -> LoadingArtRenderer {
    LoadingArtRenderer::new(
        device,
        queue,
        PICTURE_FORMAT,
        &SceneLights::VANILLA,
        &lamp_shadow_config(),
    )
}

fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scene: &mut LoadingScene,
    art: &mut LoadingArtRenderer,
    target: &Target,
    fade: f32,
) {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    scene.render(&mut encoder, art, &target.view(), target.size, fade);
    queue.submit(std::iter::once(encoder.finish()));
}

const BACKGROUND: [u8; 3] = [200, 100, 50];

#[test]
fn the_stage_background_fills_the_target_graded() {
    let Some((device, queue)) = test_device("loading scene background") else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    install_stage(
        root.path(),
        [BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255],
        [0; 4],
    );
    let mut scene = scene(&device, &queue, root.path());
    let target = Target::new(&device, &queue, SIZE);

    render(
        &device,
        &queue,
        &mut scene,
        &mut empty_art_renderer(&device, &queue),
        &target,
        1.0,
    );

    let expected = bytes(graded(lifted_background(), &neutral_look(), |display| {
        display
    }));
    for pixel in target.corners_and_centre(&device, &queue) {
        assert_close(pixel, expected, "the background covers the whole target");
    }
}

#[test]
fn every_width_of_screen_is_covered_by_the_stage_of_its_shape() {
    let Some((device, queue)) = test_device("loading scene widths") else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    install_stage(
        root.path(),
        [BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255],
        [0; 4],
    );
    let mut scene = scene(&device, &queue, root.path());
    let expected = bytes(graded(lifted_background(), &neutral_look(), |display| {
        display
    }));

    for size in [(48, 36), (82, 36), (124, 36)] {
        let target = Target::new(&device, &queue, size);
        render(
            &device,
            &queue,
            &mut scene,
            &mut empty_art_renderer(&device, &queue),
            &target,
            1.0,
        );
        for pixel in target.corners_and_centre(&device, &queue) {
            assert_close(pixel, expected, &format!("{size:?} is covered"));
        }
    }
}

#[test]
fn the_overlay_adds_its_colour_at_its_alpha_times_the_base_alpha_and_the_materials() {
    let Some((device, queue)) = test_device("loading scene overlay") else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let overlay = [90, 160, 255, 128];
    install_stage(
        root.path(),
        [BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255],
        overlay,
    );
    let mut scene = scene(&device, &queue, root.path());
    let target = Target::new(&device, &queue, SIZE);

    render(
        &device,
        &queue,
        &mut scene,
        &mut empty_art_renderer(&device, &queue),
        &target,
        1.0,
    );

    let added = decoded([overlay[0], overlay[1], overlay[2]]) * (0.6 * 0.6) * (128.0 / 255.0);
    let expected = bytes(graded(
        lifted_background() + added,
        &neutral_look(),
        |display| display,
    ));
    let plain = bytes(graded(lifted_background(), &neutral_look(), |display| {
        display
    }));
    assert_ne!(expected, plain, "the overlay changes the colour");
    for pixel in target.corners_and_centre(&device, &queue) {
        assert_close(pixel, expected, "the overlay is added in linear light");
    }
}

#[test]
fn the_scene_fades_with_the_overlay() {
    let Some((device, queue)) = test_device("loading scene fade") else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    install_stage(
        root.path(),
        [BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255],
        [0; 4],
    );
    let mut scene = scene(&device, &queue, root.path());
    let target = Target::new(&device, &queue, SIZE);

    render(
        &device,
        &queue,
        &mut scene,
        &mut empty_art_renderer(&device, &queue),
        &target,
        0.5,
    );

    let [red, green, blue] = bytes(graded(lifted_background(), &neutral_look(), |display| {
        display
    }));
    let half = |channel: u8, under: u8| u8::midpoint(channel, under);
    assert_close(
        target.pixel(&device, &queue, SIZE.0 / 2, SIZE.1 / 2),
        [half(red, 255), half(green, 0), half(blue, 0)],
        "half of the scene over the red target",
    );
}

#[test]
fn the_model_is_drawn_in_front_of_the_stage() {
    let Some((device, queue)) = test_device("loading scene model") else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    install_stage(
        root.path(),
        [BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255],
        [0; 4],
    );
    let mut scene = scene(&device, &queue, root.path());
    let mut art = empty_art_renderer(&device, &queue);
    let cpu = CpuArt::load(&cube_art(root.path(), ""), ModelControls::VANILLA)
        .expect("the synthetic cube loads");
    art.set_art(Some(
        LoadedArt::upload(&device, &queue, cpu).expect("the cube uploads"),
    ));
    let target = Target::new(&device, &queue, SIZE);

    render(&device, &queue, &mut scene, &mut art, &target, 1.0);

    let background = bytes(graded(lifted_background(), &neutral_look(), |display| {
        display
    }));
    assert_close(
        target.pixel(&device, &queue, 0, 0),
        background,
        "the corner shows the stage",
    );
    let centre = target.pixel(&device, &queue, SIZE.0 / 2, SIZE.1 / 2);
    assert!(
        centre
            .iter()
            .zip(background)
            .any(|(a, b)| a.abs_diff(b) > 20),
        "the cube hides the stage at the centre: {centre:?}"
    );
}

#[test]
fn the_scene_grades_with_the_lut_the_setting_names() {
    let Some((device, queue)) = test_device("loading scene lut") else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    install_stage(
        root.path(),
        [BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255],
        [0; 4],
    );
    let inverted = LutTable::from_fn(|r, g, b| [r, g, b].map(|level| (255 - level * 17) as u8));
    let lut_path = root
        .path()
        .join("textures/effects/luts/loadingmenucolorlut.dds");
    std::fs::create_dir_all(lut_path.parent().unwrap()).unwrap();
    std::fs::write(&lut_path, strip_dds(&inverted, 0)).unwrap();
    let mut scene = scene(&device, &queue, root.path());
    let target = Target::new(&device, &queue, SIZE);

    render(
        &device,
        &queue,
        &mut scene,
        &mut empty_art_renderer(&device, &queue),
        &target,
        1.0,
    );

    let expected = bytes(graded(lifted_background(), &neutral_look(), |display| {
        Vec3::ONE - display
    }));
    assert_close(
        target.pixel(&device, &queue, SIZE.0 / 2, SIZE.1 / 2),
        expected,
        "the LUT inverts the picture",
    );
}

/// Grades one linear colour with `pass` onto a 1 x 1 target and reads the encoded result.
fn grade_one(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pass: &mut GradePass,
    linear: Vec3,
) -> [u8; 3] {
    let picture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("loading_grade_test_picture"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        picture.as_image_copy(),
        bytemuck::cast_slice(&[linear.x, linear.y, linear.z, 1.0]),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(16),
            rows_per_image: Some(1),
        },
        picture.size(),
    );
    let target = Target::new(device, queue, (1, 1));
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    pass.draw(
        device,
        queue,
        &mut encoder,
        &picture.create_view(&wgpu::TextureViewDescriptor::default()),
        &target.view(),
        1.0,
    );
    queue.submit(std::iter::once(encoder.finish()));
    target.pixel(device, queue, 0, 0)
}

/// Every grid point graded to a colour that tells its three coordinates apart.
fn distinct() -> LutTable {
    LutTable::from_fn(|r, g, b| [(r * 16 + g) as u8, (g * 16 + b) as u8, (b * 16 + r) as u8])
}

/// The linear colour whose encoding is exactly the grid point `(r, g, b)`.
fn grid_point(r: u32, g: u32, b: u32) -> Vec3 {
    let level = |index: u32| srgb_unit_to_linear(index as f32 / (LUT_SIZE - 1) as f32);
    Vec3::new(level(r), level(g), level(b))
}

fn plain_pass(device: &wgpu::Device, queue: &wgpu::Queue, saturation: f32) -> GradePass {
    let mut pass = GradePass::new(device, queue, TARGET_FORMAT);
    pass.set_look(GradeLook {
        cinematic: ImageSpaceCinematic {
            saturation,
            ..ImageSpaceCinematic::NEUTRAL
        },
    });
    pass
}

#[test]
fn a_grid_point_reads_exactly_its_own_lut_entry() {
    let Some((device, queue)) = test_device("loading grade lut grid point") else {
        return;
    };
    let mut pass = plain_pass(&device, &queue, 1.0);
    pass.set_lut(&queue, &distinct());

    for (r, g, b) in [(0, 0, 0), (15, 15, 15), (3, 7, 11), (14, 1, 9)] {
        let expected = [(r * 16 + g) as u8, (g * 16 + b) as u8, (b * 16 + r) as u8];
        assert_close(
            grade_one(&device, &queue, &mut pass, grid_point(r, g, b)),
            expected,
            &format!("grid point {r} {g} {b}"),
        );
    }
}

#[test]
fn a_colour_between_two_grid_points_reads_their_mean() {
    let Some((device, queue)) = test_device("loading grade lut between points") else {
        return;
    };
    let mut pass = plain_pass(&device, &queue, 1.0);
    let ramp = LutTable::from_fn(|r, _, _| [(r * 16) as u8, 0, 0]);
    pass.set_lut(&queue, &ramp);
    let halfway = srgb_unit_to_linear(4.5 / (LUT_SIZE - 1) as f32);

    let graded = grade_one(&device, &queue, &mut pass, Vec3::new(halfway, 0.0, 0.0));

    assert_close(graded, [72, 0, 0], "halfway between entries 64 and 80");
}

#[test]
fn saturation_is_applied_in_linear_light_before_the_lut() {
    let Some((device, queue)) = test_device("loading grade saturation") else {
        return;
    };
    let mut pass = plain_pass(&device, &queue, 0.5);
    pass.set_lut(&queue, &LutTable::identity());
    let colour = Vec3::new(0.5, 0.1, 0.02);
    let look = GradeLook {
        cinematic: ImageSpaceCinematic {
            saturation: 0.5,
            ..ImageSpaceCinematic::NEUTRAL
        },
    };

    let expected = bytes(graded(colour, &look, |display| display));
    assert_close(
        grade_one(&device, &queue, &mut pass, colour),
        expected,
        "half saturated",
    );
}

#[test]
fn an_infinite_picture_colour_is_graded_as_the_largest_finite_one() {
    let Some((device, queue)) = test_device("loading grade infinite colour") else {
        return;
    };
    let mut pass = plain_pass(&device, &queue, 1.0);
    pass.set_lut(&queue, &LutTable::identity());
    let look = GradeLook {
        cinematic: ImageSpaceCinematic::NEUTRAL,
    };

    let graded_infinity = grade_one(
        &device,
        &queue,
        &mut pass,
        Vec3::new(f32::INFINITY, 0.5, 0.5),
    );

    let expected = bytes(graded(Vec3::new(65_000.0, 0.5, 0.5), &look, |display| {
        display
    }));
    assert_close(graded_infinity, expected, "infinity clamps to 65000");
}

#[test]
fn a_nan_picture_colour_is_graded_as_black() {
    let Some((device, queue)) = test_device("loading grade nan colour") else {
        return;
    };
    let mut pass = plain_pass(&device, &queue, 1.0);
    pass.set_lut(&queue, &LutTable::identity());
    let look = GradeLook {
        cinematic: ImageSpaceCinematic::NEUTRAL,
    };

    let graded_nan = grade_one(&device, &queue, &mut pass, Vec3::new(f32::NAN, 0.5, 0.5));

    let expected = bytes(graded(Vec3::new(0.0, 0.5, 0.5), &look, |display| display));
    assert_close(graded_nan, expected, "NaN grades as 0");
}

#[test]
#[ignore = "needs OPENFO4_INSTALL_DIR with the installed loading screen stage and LUT"]
fn the_installed_stage_frames_24_degrees_and_its_lut_loads() {
    use super::stage::{CpuStage, stage_paths};
    let install = std::env::var(crate::assets::INSTALL_DIR_VAR).expect("OPENFO4_INSTALL_DIR");
    let root = Path::new(&install).join(crate::assets::layout::ASSETS_DIR);

    let aspects: Vec<f32> = stage_paths(&root)
        .iter()
        .map(|path| {
            let stage = CpuStage::load(path).expect("the installed stage loads");
            let fov = stage.framing.vertical_fov_radians.to_degrees();
            assert!((fov - 23.98).abs() < 0.05, "{}: {fov}", path.display());
            assert_eq!(stage.layers.len(), 2, "{}", path.display());
            stage.aspect
        })
        .collect();
    assert!(
        aspects.windows(2).all(|pair| pair[0] < pair[1]),
        "{aspects:?}"
    );
    assert!((aspects[0] - 16.0 / 9.0).abs() < 0.01, "{aspects:?}");

    let lut = LoadScreenLut::from_ini(&GameIni::from_texts([""])).unwrap();
    crate::rendering::post::tonemap::lut_library::load_table(
        &root.to_string_lossy(),
        &lut.key.as_str().into(),
    )
    .expect("the installed loading screen LUT loads");
}

#[test]
fn a_stage_with_a_missing_plane_texture_is_unusable() {
    let Some((device, queue)) = test_device("loading scene missing texture") else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    install_stage(
        root.path(),
        [BACKGROUND[0], BACKGROUND[1], BACKGROUND[2], 255],
        [0; 4],
    );
    std::fs::remove_file(
        root.path()
            .join("textures")
            .join(super::test_support::STAGE_OVERLAY_TEXTURE),
    )
    .unwrap();
    let mut scene = LoadingScene::new(
        &device,
        &queue,
        TARGET_FORMAT,
        root.path(),
        GradeSource {
            lut: LoadScreenLut::from_ini(&GameIni::from_texts([""])).unwrap(),
            db_path: String::new(),
        },
    );

    let error = scene.prepare().expect_err("the overlay is missing");

    assert!(matches!(error, super::StageError::NoneUsable(_)), "{error}");
    assert!(scene.prepare().is_err(), "the failure is not retried away");
}
