//! The tonemap pass on the GPU: the scene light it is given against the displayed bytes, under
//! the measured vanilla curve, the image space's cinematic fields and its colour LUT.

use bytemuck::Zeroable;
use glam::{UVec2, Vec3, Vec4};
use wgpu::util::DeviceExt;

use super::TonemapPass;
use super::lut_table::{LutTable, blended_texels};
use crate::rendering::{
    gpu_profiler::GpuProfiler,
    gpu_test::{frame_binding, read_mip, test_device},
    lighting::{
        frame_uniform::FrameUniform,
        lerp::Lerp,
        state::ExposureReadout,
        tonemap_settings::{ImageSpaceCinematic, ImageSpaceLut, TonemapOperator, TonemapSettings},
    },
    post::dof_test_support::{hdr_texture, view_of},
};

const SURFACE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const NEUTRAL_EXPOSURE: f32 = 1.5;
const NEUTRAL_TOE: f32 = 0.02;
/// The dither moves a pixel by up to half a level, the rounding by another half.
const DITHER_LEVELS: f32 = 1.01;
/// A filtered LUT read adds the adapter's filtering precision: CI's software adapter interpolates
/// the table with coarser weights and missed by 1.02 levels.
const LUT_LEVELS: f32 = 1.5;

/// Fog colour bytes and the grey pixels the game showed for them, fully fogged, under an image
/// space with exposure 1.5, `TonemapE` 0.02 and neutral cinematic fields.
const MEASURED_LADDER: [(u8, u8); 11] = [
    (16, 24),
    (32, 47),
    (48, 70),
    (64, 92),
    (96, 132),
    (128, 165),
    (144, 179),
    (160, 190),
    (192, 209),
    (224, 225),
    (255, 236),
];
/// The curve meets every ladder step within 0.95 levels. The game's pixels are rounded, so the
/// shown ones are averaged over enough pixels to cancel the dither and compared just above that.
/// The input scale 4.0 instead of 4.113 misses by 2.4 levels.
const MEASURED_LEVELS: f32 = 1.2;
/// Pixels per light whose dither averages out.
const DITHER_SPAN: usize = 64;

fn fog_byte_light(byte: u8) -> f32 {
    (f32::from(byte) / 255.0).powf(2.2)
}

/// The tone curve of the model, written out independently of the shader.
fn reference_curve(x: f32, toe: f32) -> f32 {
    (x * (0.15 * x + 0.05) + 0.2 * toe) / (x * (0.15 * x + 0.5) + 0.06) - toe / 0.3
}

fn reference_tone_mapped(light: Vec3, exposure: f32, toe: f32) -> Vec3 {
    let white = reference_curve(11.2, toe);
    (light * exposure * 4.113)
        .to_array()
        .map(|x| (reference_curve(x, toe) / white).clamp(0.0, 1.0))
        .into()
}

fn displayed_levels(linear: Vec3) -> Vec3 {
    linear.clamp(Vec3::ZERO, Vec3::ONE).powf(1.0 / 2.2) * 255.0
}

struct Picture {
    exposure: f32,
    toe: f32,
    cinematic: ImageSpaceCinematic,
    operator: TonemapOperator,
    /// The table the pass holds and how much the image space shows of it.
    table: LutTable,
    lut: ImageSpaceLut,
}

impl Picture {
    fn neutral() -> Self {
        Self {
            operator: TonemapOperator::Filmic,
            exposure: NEUTRAL_EXPOSURE,
            toe: NEUTRAL_TOE,
            cinematic: ImageSpaceCinematic::NEUTRAL,
            table: LutTable::identity(),
            lut: ImageSpaceLut::NONE,
        }
    }

    /// The displayed bytes of each scene light, one pixel each; `None` without a GPU.
    fn show(&self, test: &str, lights: &[Vec3]) -> Option<Vec<Vec3>> {
        let (device, queue) = test_device(test)?;
        let size = UVec2::new(lights.len() as u32, 1);
        let texels: Vec<Vec4> = lights.iter().map(|light| light.extend(1.0)).collect();
        let scene = hdr_texture(
            &device,
            &queue,
            size,
            &texels,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let exposure_state = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tonemap_test_exposure"),
            contents: bytemuck::bytes_of(&ExposureReadout {
                adapted_luminance: 1.0,
                exposure: self.exposure,
            }),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let frame = frame_binding(&device);
        let tonemap = TonemapSettings {
            operator: self.operator,
            cinematic: self.cinematic,
            lut: self.lut.clone(),
        }
        .uniform(self.toe);
        frame.write(
            &queue,
            &FrameUniform {
                tonemap: tonemap.tonemap,
                cinematic: tonemap.cinematic,
                tint: tonemap.tint,
                ..FrameUniform::zeroed()
            },
        );
        let pass = TonemapPass::new(
            &device,
            &queue,
            SURFACE_FORMAT,
            &frame.layout,
            &view_of(&scene),
            &exposure_state,
        );
        pass.write_colour_lut(&queue, &blended_texels(&[(Some(&self.table), 1.0)]));
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tonemap_test_target"),
            size: crate::rendering::gpu_test::extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: pass.target_format(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let profiler = GpuProfiler::new(&device, &queue);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        pass.encode(&mut encoder, &view_of(&target), &frame, &profiler);
        queue.submit(std::iter::once(encoder.finish()));
        let bytes = read_mip(&device, &queue, &target, 0, size, 4);
        Some(
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|[r, g, b, _]| Vec3::new(f32::from(*r), f32::from(*g), f32::from(*b)))
                .collect(),
        )
    }
}

fn assert_levels(shown: &[Vec3], expected: &[Vec3], tolerance: f32) {
    for (index, (shown, expected)) in shown.iter().zip(expected).enumerate() {
        assert!(
            shown.abs_diff_eq(*expected, tolerance),
            "pixel {index}: shown {shown}, expected {expected}"
        );
    }
}

fn ladder_lights() -> Vec<Vec3> {
    MEASURED_LADDER
        .iter()
        .map(|&(byte, _)| Vec3::splat(fog_byte_light(byte)))
        .collect()
}

impl Picture {
    /// The displayed levels of each light, each averaged over `DITHER_SPAN` pixels.
    fn show_undithered(&self, test: &str, lights: &[Vec3]) -> Option<Vec<Vec3>> {
        let spread: Vec<Vec3> = lights
            .iter()
            .flat_map(|&light| std::iter::repeat_n(light, DITHER_SPAN))
            .collect();
        let shown = self.show(test, &spread)?;
        Some(
            shown
                .chunks(DITHER_SPAN)
                .map(|pixels| pixels.iter().sum::<Vec3>() / DITHER_SPAN as f32)
                .collect(),
        )
    }
}

#[test]
fn the_neutral_image_space_shows_the_fog_ladder_the_game_shows() {
    let Some(shown) = Picture::neutral().show_undithered("tonemap fog ladder", &ladder_lights())
    else {
        return;
    };
    let measured: Vec<Vec3> = MEASURED_LADDER
        .iter()
        .map(|&(_, pixel)| Vec3::splat(f32::from(pixel)))
        .collect();
    assert_levels(&shown, &measured, MEASURED_LEVELS);
}

#[test]
fn the_channels_are_tone_mapped_apart() {
    let light = Vec3::new(fog_byte_light(192), fog_byte_light(96), fog_byte_light(32));
    let Some(shown) = Picture::neutral().show_undithered("tonemap channels", &[light]) else {
        return;
    };
    assert_levels(&shown, &[Vec3::new(209.0, 132.0, 47.0)], MEASURED_LEVELS);
}

#[test]
fn the_exposure_and_the_toe_shape_the_curve() {
    let lights = ladder_lights();
    for (exposure, toe) in [
        (0.75, NEUTRAL_TOE),
        (3.0, NEUTRAL_TOE),
        (1.5, 0.01),
        (1.5, 0.04),
    ] {
        let picture = Picture {
            exposure,
            toe,
            ..Picture::neutral()
        };
        let Some(shown) = picture.show("tonemap exposure and toe", &lights) else {
            return;
        };
        let expected: Vec<Vec3> = lights
            .iter()
            .map(|&light| displayed_levels(reference_tone_mapped(light, exposure, toe)))
            .collect();
        assert_levels(&shown, &expected, DITHER_LEVELS);
    }
}

fn with_cinematic(cinematic: ImageSpaceCinematic) -> Picture {
    Picture {
        cinematic,
        ..Picture::neutral()
    }
}

fn colour_lights() -> Vec<Vec3> {
    vec![
        Vec3::new(fog_byte_light(192), fog_byte_light(96), fog_byte_light(32)),
        Vec3::new(fog_byte_light(40), fog_byte_light(120), fog_byte_light(200)),
        Vec3::splat(fog_byte_light(64)),
    ]
}

fn neutral_pictures(lights: &[Vec3]) -> Vec<Vec3> {
    lights
        .iter()
        .map(|&light| reference_tone_mapped(light, NEUTRAL_EXPOSURE, NEUTRAL_TOE))
        .collect()
}

fn luminance(picture: Vec3) -> f32 {
    picture.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

#[test]
fn saturation_scales_the_chroma_around_the_luminance_of_the_tone_mapped_picture() {
    let lights = colour_lights();
    for saturation in [0.0, 0.5, 2.0] {
        let picture = with_cinematic(ImageSpaceCinematic {
            saturation,
            ..ImageSpaceCinematic::NEUTRAL
        });
        let Some(shown) = picture.show("tonemap saturation", &lights) else {
            return;
        };
        let expected: Vec<Vec3> = neutral_pictures(&lights)
            .into_iter()
            .map(|p| {
                let grey = Vec3::splat(luminance(p));
                displayed_levels(grey + (p - grey) * saturation)
            })
            .collect();
        assert_levels(&shown, &expected, DITHER_LEVELS);
    }
}

#[test]
fn brightness_scales_the_tone_mapped_picture() {
    let lights = colour_lights();
    for brightness in [0.5, 2.0] {
        let picture = with_cinematic(ImageSpaceCinematic {
            brightness,
            ..ImageSpaceCinematic::NEUTRAL
        });
        let Some(shown) = picture.show("tonemap brightness", &lights) else {
            return;
        };
        let expected: Vec<Vec3> = neutral_pictures(&lights)
            .into_iter()
            .map(|p| displayed_levels(p * brightness))
            .collect();
        assert_levels(&shown, &expected, DITHER_LEVELS);
    }
}

#[test]
fn contrast_pivots_about_the_display_middle() {
    let middle = 0.5_f32.powf(2.2);
    let lights = colour_lights();
    for contrast in [0.5, 2.0] {
        let picture = with_cinematic(ImageSpaceCinematic {
            contrast,
            ..ImageSpaceCinematic::NEUTRAL
        });
        let Some(shown) = picture.show("tonemap contrast", &lights) else {
            return;
        };
        let expected: Vec<Vec3> = neutral_pictures(&lights)
            .into_iter()
            .map(|p| displayed_levels(Vec3::splat(middle) + (p - Vec3::splat(middle)) * contrast))
            .collect();
        assert_levels(&shown, &expected, DITHER_LEVELS);
    }
}

#[test]
fn the_tint_mixes_towards_the_luminance_in_the_tint_colour() {
    let lights = colour_lights();
    for (tint_amount, tint_color) in [
        (0.5, Vec3::new(1.0, 0.0, 0.0)),
        (1.0, Vec3::new(1.0, 0.0, 0.0)),
        (0.5, Vec3::new(0.0, 0.0, 1.0)),
        (0.3, Vec3::new(1.0, 0.39, 0.007)),
    ] {
        let picture = with_cinematic(ImageSpaceCinematic {
            tint_amount,
            tint_color,
            ..ImageSpaceCinematic::NEUTRAL
        });
        let Some(shown) = picture.show("tonemap tint", &lights) else {
            return;
        };
        let expected: Vec<Vec3> = neutral_pictures(&lights)
            .into_iter()
            .map(|p| displayed_levels(p.lerp(luminance(p) * tint_color, tint_amount)))
            .collect();
        assert_levels(&shown, &expected, DITHER_LEVELS);
    }
}

#[test]
fn agx_shows_black_as_black_and_brightens_with_the_light() {
    let picture = Picture {
        operator: TonemapOperator::AgX,
        ..Picture::neutral()
    };
    let lights: Vec<Vec3> = std::iter::once(Vec3::ZERO).chain(ladder_lights()).collect();
    let Some(shown) = picture.show_undithered("tonemap agx", &lights) else {
        return;
    };
    assert!(shown[0].max_element() < 1.0, "black shows {}", shown[0]);
    for pair in shown.windows(2) {
        assert!(pair[1].x > pair[0].x, "{} then {}", pair[0], pair[1]);
    }
}

#[test]
fn agx_passes_through_the_cinematic_stage_and_the_gamma_encoding() {
    let lights = ladder_lights();
    let agx = |brightness| Picture {
        operator: TonemapOperator::AgX,
        cinematic: ImageSpaceCinematic {
            brightness,
            ..ImageSpaceCinematic::NEUTRAL
        },
        ..Picture::neutral()
    };
    let (Some(full), Some(half)) = (
        agx(1.0).show_undithered("tonemap agx cinematic", &lights),
        agx(0.5).show_undithered("tonemap agx cinematic", &lights),
    ) else {
        return;
    };
    let expected: Vec<Vec3> = full
        .iter()
        .map(|levels| displayed_levels((*levels / 255.0).powf(2.2) * 0.5))
        .collect();
    assert_levels(&half, &expected, DITHER_LEVELS);
}

/// A table that darkens the toe, brightens the shoulder and mixes the channels, as the vanilla
/// tables do, with every grid point graded differently.
fn mixing_table() -> LutTable {
    let s_curve = |index: u32| {
        let x = index as f32 / 15.0;
        x * x * (3.0 - 2.0 * x)
    };
    LutTable::from_fn(|r, g, b| {
        let (r, g, b) = (s_curve(r), s_curve(g), s_curve(b));
        [0.8 * r + 0.2 * g, 0.9 * g + 0.1 * b, 0.7 * b + 0.3 * r].map(|x| (x * 255.0).round() as u8)
    })
}

fn graded_by(table: LutTable, amount: f32) -> Picture {
    Picture {
        table,
        lut: ImageSpaceLut::NONE.lerp(&ImageSpaceLut::named(r"Effects\LUTs\Mixing.dds"), amount),
        ..Picture::neutral()
    }
}

fn lut_lights() -> Vec<Vec3> {
    colour_lights()
        .into_iter()
        .chain(ladder_lights())
        .chain([Vec3::new(
            fog_byte_light(250),
            fog_byte_light(5),
            fog_byte_light(130),
        )])
        .collect()
}

#[test]
fn the_lut_grades_the_encoded_picture_between_its_texel_centres() {
    let lights = lut_lights();
    let Some(shown) = graded_by(mixing_table(), 1.0).show("tonemap lut", &lights) else {
        return;
    };
    let expected: Vec<Vec3> = neutral_pictures(&lights)
        .into_iter()
        .map(|p| mixing_table().sample(p.clamp(Vec3::ZERO, Vec3::ONE).powf(1.0 / 2.2)) * 255.0)
        .collect();
    assert_levels(&shown, &expected, LUT_LEVELS);
}

#[test]
fn a_part_of_the_lut_mixes_the_graded_picture_into_the_ungraded_one() {
    let lights = lut_lights();
    let Some(shown) = graded_by(mixing_table(), 0.25).show("tonemap lut amount", &lights) else {
        return;
    };
    let expected: Vec<Vec3> = neutral_pictures(&lights)
        .into_iter()
        .map(|p| {
            let display = p.clamp(Vec3::ZERO, Vec3::ONE).powf(1.0 / 2.2);
            display.lerp(mixing_table().sample(display), 0.25) * 255.0
        })
        .collect();
    assert_levels(&shown, &expected, LUT_LEVELS);
}

#[test]
fn without_a_lut_the_held_table_leaves_the_picture_exactly_as_it_was() {
    let lights = lut_lights();
    let (Some(ungraded), Some(held)) = (
        Picture::neutral().show("tonemap no lut", &lights),
        graded_by(mixing_table(), 0.0).show("tonemap no lut", &lights),
    ) else {
        return;
    };
    assert_eq!(held, ungraded);
}

#[test]
fn the_identity_lut_leaves_the_picture_within_its_filtering() {
    let lights = lut_lights();
    let (Some(ungraded), Some(identity)) = (
        Picture::neutral().show_undithered("tonemap identity lut", &lights),
        graded_by(LutTable::identity(), 1.0).show_undithered("tonemap identity lut", &lights),
    ) else {
        return;
    };
    assert_levels(&identity, &ungraded, 0.2);
}
