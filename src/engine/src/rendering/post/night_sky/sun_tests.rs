use glam::{Vec3, Vec4};

use super::assets::{AssetFailureKind, AssetState, SkyAssetError, named_texture_key};
use super::sun::{SunInput, SunLayer, SunSizes, SunUniform, SunUniforms};
use super::tests::gpu::{
    Rig, SKY_CLEAR, WorldDraw, feedback_group, pattern_texture, render, rig, texel, white_texture,
};
use super::{NightSky, NightSkyInput, SkySizes};
use crate::assets::ini::{GameIni, IniError};
use crate::rendering::lighting::time_of_day::unmeasured::{
    MOON_SIZE_UNITS_PER_DEGREE, SUN_DISC_RADIANCE, SUN_GLARE_RADIANCE, SUN_HORIZON_FADE_SINE,
    SUN_SIZE_UNITS_PER_DEGREE, sun_sprite_edge_deg,
};
use crate::rendering::lighting::time_of_day::{ClimateSunTextures, StarRotation};
use crate::rendering::shaders;

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= expected.abs() * 1e-3 + 1e-6,
        "{actual} is not {expected}"
    );
}

fn sun_input() -> SunInput {
    SunInput {
        direction: Vec3::new(0.0, 0.5, -0.866_025_4),
        alpha: 1.0,
        disc_color: Vec3::ONE,
        glare_color: Vec3::ONE,
    }
}

fn night_sky_input(textures: ClimateSunTextures) -> NightSkyInput {
    NightSkyInput {
        moon: None,
        star_rotation: StarRotation {
            axis: Vec3::Y,
            angle_rad: 0.0,
        },
        star_alpha: 0.0,
        sky_model: None,
        sun: sun_input(),
        sun_textures: textures,
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

mod sizes {
    use super::*;

    fn ini(text: &str) -> GameIni {
        GameIni::from_texts([text])
    }

    #[test]
    fn the_sizes_come_from_the_ini() {
        let sizes = SunSizes::from_ini(&ini("fSunBaseSize=300.5\nfSunGlareSize=450\n")).unwrap();
        assert_eq!(
            sizes,
            SunSizes {
                base: 300.5,
                glare: 450.0
            }
        );
    }

    #[test]
    fn sizes_no_ini_sets_take_the_vanilla_defaults() {
        assert_eq!(SunSizes::from_ini(&ini("")).unwrap(), SunSizes::VANILLA);
        let only_base = SunSizes::from_ini(&ini("fSunBaseSize=100\n")).unwrap();
        assert_eq!(only_base.base, 100.0);
        assert_eq!(only_base.glare, SunSizes::VANILLA.glare);
        let only_glare = SunSizes::from_ini(&ini("fSunGlareSize=100\n")).unwrap();
        assert_eq!(only_glare.base, SunSizes::VANILLA.base);
        assert_eq!(only_glare.glare, 100.0);
    }

    #[test]
    fn the_vanilla_defaults_are_the_games() {
        assert_eq!(SunSizes::VANILLA.base, 425.0);
        assert_eq!(SunSizes::VANILLA.glare, 600.0);
    }

    #[test]
    fn a_size_that_is_set_must_be_a_positive_number() {
        for (text, unparsable) in [
            ("fSunBaseSize=0\n", false),
            ("fSunBaseSize=-5\n", false),
            ("fSunGlareSize=inf\n", false),
            ("fSunGlareSize=large\n", true),
        ] {
            let error = SunSizes::from_ini(&ini(text)).unwrap_err();
            let expected_kind = if unparsable {
                matches!(error, IniError::Unparsable { .. })
            } else {
                matches!(error, IniError::Invalid { .. })
            };
            assert!(expected_kind, "{text}: {error}");
        }
    }

    #[test]
    fn a_size_is_converted_to_the_edge_of_its_sprite_in_the_moons_unit() {
        assert_eq!(SUN_SIZE_UNITS_PER_DEGREE, MOON_SIZE_UNITS_PER_DEGREE);
        assert_close(sun_sprite_edge_deg(SunSizes::VANILLA.base), 425.0 / 30.0);
        assert_close(sun_sprite_edge_deg(SunSizes::VANILLA.glare), 20.0);
        assert!(
            sun_sprite_edge_deg(SunSizes::VANILLA.glare)
                > sun_sprite_edge_deg(SunSizes::VANILLA.base)
        );
    }
}

mod keys {
    use super::*;

    #[test]
    fn a_named_texture_key_names_the_file_below_the_textures_in_lower_case() {
        assert_eq!(
            named_texture_key("Sky\\Sun_d.dds"),
            "textures\\sky\\sun_d.dds"
        );
        assert_eq!(
            named_texture_key("Sky/SunGlare.DDS"),
            "textures\\sky\\sunglare.dds"
        );
    }
}

mod uniforms {
    use super::*;

    fn input() -> SunInput {
        SunInput {
            direction: Vec3::new(0.0, 0.0, -2.0),
            alpha: 0.5,
            disc_color: Vec3::new(1.0, 0.5, 0.25),
            glare_color: Vec3::new(0.2, 0.4, 0.8),
        }
    }

    #[test]
    fn the_uniform_has_the_layout_the_shader_declares() {
        assert_eq!(std::mem::size_of::<SunUniform>(), 64);
    }

    #[test]
    fn the_quad_spans_the_tangent_of_half_the_sprite_edge() {
        let uniform = SunUniform::new(Vec3::Z, 20.0, 1.0, Vec3::ONE).unwrap();
        assert!((uniform.half_extent - 10.0f32.to_radians().tan()).abs() < 1e-7);
        assert_eq!(uniform.horizon_fade_sine, SUN_HORIZON_FADE_SINE);
    }

    #[test]
    fn the_direction_is_normalised_and_the_alpha_clamped() {
        let uniform = SunUniform::new(Vec3::new(0.0, 0.0, 5.0), 3.0, 7.0, Vec3::ONE).unwrap();
        assert_eq!(uniform.direction, [0.0, 0.0, 1.0]);
        assert_eq!(uniform.alpha, 1.0);
    }

    #[test]
    fn a_sprite_that_cannot_be_seen_gives_no_uniform() {
        let sun = |direction: Vec3, edge: f32, alpha: f32, radiance: Vec3| {
            SunUniform::new(direction, edge, alpha, radiance)
        };
        assert!(
            sun(Vec3::ZERO, 3.0, 1.0, Vec3::ONE).is_none(),
            "no direction"
        );
        assert!(
            sun(Vec3::NAN, 3.0, 1.0, Vec3::ONE).is_none(),
            "bad direction"
        );
        assert!(sun(Vec3::X, 0.0, 1.0, Vec3::ONE).is_none(), "no size");
        assert!(sun(Vec3::X, f32::NAN, 1.0, Vec3::ONE).is_none(), "bad size");
        assert!(sun(Vec3::X, 3.0, 0.0, Vec3::ONE).is_none(), "no alpha");
        assert!(
            sun(Vec3::X, 3.0, -1.0, Vec3::ONE).is_none(),
            "negative alpha"
        );
        assert!(
            sun(Vec3::X, 3.0, f32::NAN, Vec3::ONE).is_none(),
            "bad alpha"
        );
        assert!(sun(Vec3::X, 3.0, 1.0, Vec3::ZERO).is_none(), "no radiance");
        assert!(sun(Vec3::X, 3.0, 1.0, Vec3::NAN).is_none(), "bad radiance");
    }

    #[test]
    fn radiance_that_is_negative_or_undefined_in_one_channel_is_zero_there() {
        let uniform = SunUniform::new(Vec3::X, 3.0, 1.0, Vec3::new(2.0, -1.0, f32::NAN)).unwrap();
        assert_eq!(uniform.radiance, [2.0, 0.0, 0.0]);
    }

    #[test]
    fn each_sprite_takes_its_size_colour_and_brightness() {
        let sizes = SunSizes {
            base: 300.0,
            glare: 600.0,
        };

        let uniforms = SunUniforms::new(&input(), sizes, 2.0);

        let disc = uniforms.disc.unwrap();
        let glare = uniforms.glare.unwrap();
        assert_eq!(disc.direction, [0.0, 0.0, -1.0]);
        assert_eq!(glare.direction, [0.0, 0.0, -1.0]);
        assert_eq!(disc.alpha, 0.5);
        assert_eq!(glare.alpha, 0.5);
        assert_close(disc.half_extent, 5.0f32.to_radians().tan());
        assert_close(glare.half_extent, 10.0f32.to_radians().tan());
        let radiance = |uniform: SunUniform, color: Vec3, brightness: f32| {
            Vec3::from(uniform.radiance).abs_diff_eq(color * 2.0 * brightness, 1e-4)
        };
        assert!(radiance(disc, input().disc_color, SUN_DISC_RADIANCE));
        assert!(radiance(glare, input().glare_color, SUN_GLARE_RADIANCE));
    }

    #[test]
    fn a_sun_without_alpha_or_sky_radiance_draws_neither_sprite() {
        let sizes = SunSizes::VANILLA;
        let dark = SunInput {
            alpha: 0.0,
            ..input()
        };
        for uniforms in [
            SunUniforms::new(&dark, sizes, 1.0),
            SunUniforms::new(&input(), sizes, 0.0),
            SunUniforms::new(&input(), sizes, -3.0),
            SunUniforms::new(&input(), sizes, f32::NAN),
        ] {
            assert_eq!((uniforms.disc, uniforms.glare), (None, None));
        }
    }

    #[test]
    fn a_black_glare_colour_leaves_the_disc_alone() {
        let input = SunInput {
            glare_color: Vec3::ZERO,
            ..input()
        };

        let uniforms = SunUniforms::new(&input, SunSizes::VANILLA, 1.0);

        assert!(uniforms.disc.is_some());
        assert!(uniforms.glare.is_none());
    }
}

mod shader_module {
    use super::*;

    #[test]
    fn the_sun_shader_has_both_entry_points() {
        let module = naga::front::wgsl::parse_str(shaders::SKY_SUN.wgsl()).expect("it parses");
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

    const BLACK: Vec4 = Vec4::new(0.0, 0.0, 0.0, 1.0);
    const TEXTURE_EDGE: u32 = 16;
    const HALF_ALPHA: u8 = 128;
    /// 20 degrees across: a disc around the sun that leaves the left of the view alone.
    const SMALL: f32 = 600.0;
    /// 90 degrees across: a glare that covers the view around the sun.
    const LARGE: f32 = 2700.0;
    const SIZES: SunSizes = SunSizes {
        base: SMALL,
        glare: LARGE,
    };
    /// Where the sun, 30 degrees up due north, and its glare appear on the 16 x 16 target.
    const SUN_PIXEL: (u32, u32) = (8, 3);
    const GLARE_ONLY_PIXEL: (u32, u32) = (3, 4);
    const BELOW_HORIZON_ROW: u32 = 12;

    type Texture = Option<Result<wgpu::TextureView, SkyAssetError>>;

    fn not_found(key: &str) -> Result<wgpu::TextureView, SkyAssetError> {
        Err(SkyAssetError::NotFound {
            key: key.to_owned(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        })
    }

    fn sun_layer(rig: &Rig, disc: Texture, glare: Texture) -> (SunLayer, [AssetState; 2]) {
        let mut layer = SunLayer::new(&rig.device, rig.world_groups());
        let states = layer.install(&rig.device, disc, glare);
        (layer, states)
    }

    fn white_sun(rig: &Rig) -> SunLayer {
        let white = || Some(Ok(white_texture(rig, TEXTURE_EDGE)));
        sun_layer(rig, white(), white()).0
    }

    fn pixels(
        rig: &Rig,
        layer: &mut SunLayer,
        input: &SunInput,
        radiance_scale: f32,
        depth: f32,
    ) -> Vec<Vec4> {
        layer.update(&rig.queue, &SunUniforms::new(input, SIZES, radiance_scale));
        render(rig, depth, |pass| layer.draw(pass))
    }

    fn at(pixels: &[Vec4], (column, row): (u32, u32)) -> Vec4 {
        texel(pixels, column, row)
    }

    fn assert_black(pixels: &[Vec4]) {
        assert!(pixels.iter().all(|texel| *texel == BLACK), "{pixels:?}");
    }

    #[test]
    fn the_disc_and_the_glare_cover_the_pixels_around_the_sun() {
        let Some(rig) = rig("the_disc_and_the_glare_cover_the_pixels_around_the_sun") else {
            return;
        };
        let mut layer = white_sun(&rig);

        let drawn = pixels(&rig, &mut layer, &sun_input(), 1.0, SKY_CLEAR);

        let sun = at(&drawn, SUN_PIXEL);
        assert!(sun.truncate().min_element() > 0.0, "{sun}");
        assert_eq!(sun.w, 1.0, "the target alpha is left alone");
        let glare_only = at(&drawn, GLARE_ONLY_PIXEL);
        assert_close(glare_only.x, SUN_GLARE_RADIANCE);
        assert_close(sun.x, SUN_DISC_RADIANCE + SUN_GLARE_RADIANCE);
        assert_eq!(
            at(&drawn, (0, BELOW_HORIZON_ROW)),
            BLACK,
            "nothing below the horizon"
        );
    }

    #[test]
    fn the_disc_and_the_glare_have_the_brightness_of_their_sprites_at_full_texture() {
        let Some(rig) =
            rig("the_disc_and_the_glare_have_the_brightness_of_their_sprites_at_full_texture")
        else {
            return;
        };
        let white = || Some(Ok(white_texture(&rig, TEXTURE_EDGE)));
        let (mut glare_layer, _) = sun_layer(&rig, None, white());
        let (mut disc_layer, _) = sun_layer(&rig, white(), None);

        let glare = pixels(&rig, &mut glare_layer, &sun_input(), 1.0, SKY_CLEAR);
        let disc = pixels(&rig, &mut disc_layer, &sun_input(), 1.0, SKY_CLEAR);

        assert_close(at(&glare, GLARE_ONLY_PIXEL).x, SUN_GLARE_RADIANCE);
        assert_close(at(&glare, SUN_PIXEL).x, SUN_GLARE_RADIANCE);
        assert_close(at(&disc, SUN_PIXEL).x, SUN_DISC_RADIANCE);
        assert_eq!(
            at(&disc, GLARE_ONLY_PIXEL),
            BLACK,
            "the disc is the small one"
        );
    }

    #[test]
    fn the_sprites_scale_with_the_sky_radiance_the_sun_alpha_and_their_colours() {
        let Some(rig) =
            rig("the_sprites_scale_with_the_sky_radiance_the_sun_alpha_and_their_colours")
        else {
            return;
        };
        let mut layer = white_sun(&rig);
        let base = pixels(&rig, &mut layer, &sun_input(), 1.0, SKY_CLEAR);

        let doubled = pixels(&rig, &mut layer, &sun_input(), 2.0, SKY_CLEAR);
        let half_alpha = pixels(
            &rig,
            &mut layer,
            &SunInput {
                alpha: 0.5,
                ..sun_input()
            },
            1.0,
            SKY_CLEAR,
        );
        let tinted = pixels(
            &rig,
            &mut layer,
            &SunInput {
                disc_color: Vec3::new(1.0, 0.5, 0.0),
                glare_color: Vec3::new(0.0, 0.0, 1.0),
                ..sun_input()
            },
            1.0,
            SKY_CLEAR,
        );

        for probe in [SUN_PIXEL, GLARE_ONLY_PIXEL] {
            assert_close(at(&doubled, probe).x, at(&base, probe).x * 2.0);
            assert_close(at(&half_alpha, probe).x, at(&base, probe).x * 0.5);
        }
        let disc = at(&tinted, SUN_PIXEL);
        let glare = at(&tinted, GLARE_ONLY_PIXEL);
        assert_close(disc.x, SUN_DISC_RADIANCE);
        assert_close(disc.y, SUN_DISC_RADIANCE * 0.5);
        assert_close(disc.z, SUN_GLARE_RADIANCE);
        assert_eq!(
            (glare.x, glare.y),
            (0.0, 0.0),
            "the glare has its own colour"
        );
        assert_close(glare.z, SUN_GLARE_RADIANCE);
    }

    #[test]
    fn the_sprites_add_to_what_is_behind_them() {
        let Some(rig) = rig("the_sprites_add_to_what_is_behind_them") else {
            return;
        };
        let mut layer = white_sun(&rig);
        layer.update(&rig.queue, &SunUniforms::new(&sun_input(), SIZES, 1.0));

        let once = render(&rig, SKY_CLEAR, |pass| layer.draw(pass));
        let twice = render(&rig, SKY_CLEAR, |pass| {
            layer.draw(pass);
            layer.draw(pass);
        });

        assert_close(at(&twice, SUN_PIXEL).x, at(&once, SUN_PIXEL).x * 2.0);
    }

    #[test]
    fn the_texture_alpha_weights_what_is_drawn() {
        let Some(rig) = rig("the_texture_alpha_weights_what_is_drawn") else {
            return;
        };
        let translucent = pattern_texture(&rig, TEXTURE_EDGE, |_, _| [255, 255, 255, HALF_ALPHA]);
        let clear = pattern_texture(&rig, TEXTURE_EDGE, |_, _| [255, 255, 255, 0]);
        let mut opaque_layer = white_sun(&rig);
        let (mut translucent_layer, _) = sun_layer(&rig, Some(Ok(translucent)), None);
        let (mut clear_layer, _) = sun_layer(&rig, Some(Ok(clear)), None);
        let opaque = pixels(&rig, &mut opaque_layer, &sun_input(), 1.0, SKY_CLEAR);

        let shown = pixels(&rig, &mut translucent_layer, &sun_input(), 1.0, SKY_CLEAR);
        let hidden = pixels(&rig, &mut clear_layer, &sun_input(), 1.0, SKY_CLEAR);

        assert_close(
            at(&shown, SUN_PIXEL).x,
            SUN_DISC_RADIANCE * f32::from(HALF_ALPHA) / 255.0,
        );
        assert!(at(&opaque, SUN_PIXEL).x > at(&shown, SUN_PIXEL).x);
        assert_black(&hidden);
    }

    #[test]
    fn the_texture_appears_upright_and_unmirrored() {
        let Some(rig) = rig("the_texture_appears_upright_and_unmirrored") else {
            return;
        };
        let quadrant = pattern_texture(&rig, TEXTURE_EDGE, |column, row| {
            if column < TEXTURE_EDGE / 2 && row < TEXTURE_EDGE / 2 {
                [255; 4]
            } else {
                [0, 0, 0, 255]
            }
        });
        let (mut layer, _) = sun_layer(&rig, Some(Ok(quadrant)), None);
        let up_front = SunInput {
            direction: Vec3::new(0.0, 0.5, -0.866_025_4),
            ..sun_input()
        };
        layer.update(
            &rig.queue,
            &SunUniforms::new(
                &up_front,
                SunSizes {
                    base: 1800.0,
                    glare: SMALL,
                },
                1.0,
            ),
        );

        let drawn = render(&rig, SKY_CLEAR, |pass| layer.draw(pass));

        let lit = |column, row| texel(&drawn, column, row).x;
        assert!(
            lit(6, 2) > 0.0,
            "the lit top left quadrant is at the top left"
        );
        assert_eq!(lit(10, 2), 0.0, "top right");
        assert_eq!(lit(6, 6), 0.0, "bottom left");
        assert_eq!(lit(10, 6), 0.0, "bottom right");
    }

    #[test]
    fn the_part_below_the_horizon_is_cut() {
        let Some(rig) = rig("the_part_below_the_horizon_is_cut") else {
            return;
        };
        let mut layer = white_sun(&rig);
        let on_the_horizon = SunInput {
            direction: Vec3::NEG_Z,
            ..sun_input()
        };

        let drawn = pixels(&rig, &mut layer, &on_the_horizon, 1.0, SKY_CLEAR);

        assert!(at(&drawn, (8, 5)).x > 0.0, "above the horizon");
        assert_eq!(
            at(&drawn, (8, BELOW_HORIZON_ROW)),
            BLACK,
            "below the horizon"
        );
    }

    #[test]
    fn a_sun_below_the_horizon_is_not_seen() {
        let Some(rig) = rig("a_sun_below_the_horizon_is_not_seen") else {
            return;
        };
        let mut layer = white_sun(&rig);
        let set = SunInput {
            direction: Vec3::new(0.0, -0.5, -0.866_025_4),
            ..sun_input()
        };
        let small = SunSizes {
            base: SMALL,
            glare: SMALL,
        };
        layer.update(&rig.queue, &SunUniforms::new(&set, small, 1.0));

        assert_black(&render(&rig, SKY_CLEAR, |pass| layer.draw(pass)));
    }

    #[test]
    fn the_sun_hides_behind_geometry_that_wrote_depth() {
        let Some(rig) = rig("the_sun_hides_behind_geometry_that_wrote_depth") else {
            return;
        };
        let mut layer = white_sun(&rig);

        assert_black(&pixels(&rig, &mut layer, &sun_input(), 1.0, 0.5));
    }

    #[test]
    fn a_sun_behind_the_camera_draws_nothing() {
        let Some(rig) = rig("a_sun_behind_the_camera_draws_nothing") else {
            return;
        };
        let mut layer = white_sun(&rig);
        let behind = SunInput {
            direction: Vec3::new(0.0, 0.5, 0.866_025_4),
            ..sun_input()
        };

        assert_black(&pixels(&rig, &mut layer, &behind, 1.0, SKY_CLEAR));
    }

    #[test]
    fn a_sprite_that_is_not_updated_this_frame_stays_hidden() {
        let Some(rig) = rig("a_sprite_that_is_not_updated_this_frame_stays_hidden") else {
            return;
        };
        let mut layer = white_sun(&rig);
        pixels(&rig, &mut layer, &sun_input(), 1.0, SKY_CLEAR);
        let night = SunInput {
            alpha: 0.0,
            ..sun_input()
        };

        assert_black(&pixels(&rig, &mut layer, &night, 1.0, SKY_CLEAR));
    }

    #[test]
    fn a_sprite_without_a_usable_texture_is_not_drawn_and_says_why() {
        let Some(rig) = rig("a_sprite_without_a_usable_texture_is_not_drawn_and_says_why") else {
            return;
        };
        let glare = Some(Ok(white_texture(&rig, TEXTURE_EDGE)));
        let (mut layer, states) =
            sun_layer(&rig, Some(not_found("textures\\sky\\sun_d.dds")), glare);

        let drawn = pixels(&rig, &mut layer, &sun_input(), 1.0, SKY_CLEAR);

        assert!(matches!(
            &states[0],
            AssetState::Failed(failure)
                if failure.kind == AssetFailureKind::NotFound
                    && failure.message.contains("sun_d.dds")
        ));
        assert_eq!(states[1], AssetState::Loaded);
        assert_close(at(&drawn, SUN_PIXEL).x, SUN_GLARE_RADIANCE);
        let (mut neither, states) = sun_layer(&rig, None, None);
        assert_eq!(states, [AssetState::NotRequested, AssetState::NotRequested]);
        assert_black(&pixels(&rig, &mut neither, &sun_input(), 1.0, SKY_CLEAR));
    }

    #[test]
    fn a_climate_that_names_a_texture_and_a_failed_one_replaces_the_earlier_state() {
        let Some(rig) =
            rig("a_climate_that_names_a_texture_and_a_failed_one_replaces_the_earlier_state")
        else {
            return;
        };
        let (mut layer, _) = sun_layer(&rig, Some(not_found("a")), Some(not_found("b")));
        let states = layer.install(
            &rig.device,
            Some(Ok(white_texture(&rig, TEXTURE_EDGE))),
            None,
        );

        assert_eq!(states, [AssetState::Loaded, AssetState::NotRequested]);
        let drawn = pixels(&rig, &mut layer, &sun_input(), 1.0, SKY_CLEAR);
        assert_close(at(&drawn, SUN_PIXEL).x, SUN_DISC_RADIANCE);
    }

    #[test]
    fn a_night_sky_without_the_sun_textures_in_the_install_reports_them_failed_and_draws_no_sun() {
        let Some(rig) = rig(
            "a_night_sky_without_the_sun_textures_in_the_install_reports_them_failed_and_draws_no_sun",
        ) else {
            return;
        };
        let root = tempfile::tempdir().unwrap();
        let mut sky = new_sky(&rig, root.path().to_str().unwrap());
        assert_eq!(sky.asset_status().sun_texture, AssetState::NotRequested);
        let textures = ClimateSunTextures::new(Some("Sky\\Sun_d.dds"), Some("Sky\\SunGlare.dds"));

        sky.update(&rig.device, &rig.queue, &night_sky_input(textures), 1.0);
        let feedback = feedback_group(&rig);
        let drawn = render(&rig, SKY_CLEAR, |pass| sky.draw(pass, &feedback));

        let status = sky.asset_status();
        for state in [&status.sun_texture, &status.sun_glare_texture] {
            assert!(
                matches!(state, AssetState::Failed(failure) if failure.kind == AssetFailureKind::NotFound),
                "{state:?}"
            );
        }
        assert_black(&drawn);
    }

    #[test]
    fn a_night_sky_loads_the_sun_textures_a_later_climate_names() {
        let Some(rig) = rig("a_night_sky_loads_the_sun_textures_a_later_climate_names") else {
            return;
        };
        let root = tempfile::tempdir().unwrap();
        let mut sky = new_sky(&rig, root.path().to_str().unwrap());
        let nothing = night_sky_input(ClimateSunTextures::default());
        sky.update(&rig.device, &rig.queue, &nothing, 1.0);
        assert_eq!(sky.asset_status().sun_texture, AssetState::NotRequested);
        assert_eq!(
            sky.asset_status().sun_glare_texture,
            AssetState::NotRequested
        );

        let glare_only = ClimateSunTextures::new(None, Some("Sky\\SunGlare.dds"));
        sky.update(&rig.device, &rig.queue, &night_sky_input(glare_only), 1.0);

        assert_eq!(sky.asset_status().sun_texture, AssetState::NotRequested);
        assert!(matches!(
            sky.asset_status().sun_glare_texture,
            AssetState::Failed(_)
        ));
    }

    #[test]
    fn the_sun_shows_after_a_world_pass_draw() {
        let Some(rig) = rig("the_sun_shows_after_a_world_pass_draw") else {
            return;
        };
        let mut layer = white_sun(&rig);
        layer.update(&rig.queue, &SunUniforms::new(&sun_input(), SIZES, 1.0));
        let world = WorldDraw::new(&rig);

        let drawn = render(&rig, SKY_CLEAR, |pass| {
            world.draw(pass);
            layer.draw(pass);
        });

        let sun = at(&drawn, SUN_PIXEL);
        assert!(sun.truncate().min_element() > 0.0, "{sun}");
    }
}

/// Developer checks against a real install (`OPENFO4_INSTALL_DIR`), run with `--ignored`.
mod real_install {
    use super::*;
    use crate::assets::db::DbClient;

    const DEFAULT_CLIMATE: &str = "00015F:Fallout4.esm";
    const GOODNEIGHBOR_CLIMATE: &str = "115C69:Fallout4.esm";

    fn asset_root() -> String {
        let install = std::env::var(crate::assets::INSTALL_DIR_VAR).expect("OPENFO4_INSTALL_DIR");
        format!("{install}/{}", crate::assets::layout::ASSETS_DIR)
    }

    fn climate_textures(climate: &str) -> ClimateSunTextures {
        let db = DbClient::new(&format!("{}/data/data.db", asset_root())).unwrap();
        let (sun, glare) = db.get_climate_sun_textures(climate).unwrap();
        ClimateSunTextures::new(sun.as_deref(), glare.as_deref())
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the converted sky textures"]
    fn the_sun_textures_of_the_vanilla_climates_load() {
        let name = "the_sun_textures_of_the_vanilla_climates_load";
        let Some(rig) = rig(name) else {
            return;
        };
        for (climate, sun_name) in [
            (DEFAULT_CLIMATE, "Sky\\Sun_d.dds"),
            (GOODNEIGHBOR_CLIMATE, "Sky\\Sun.dds"),
        ] {
            let textures = climate_textures(climate);
            assert_eq!(textures.sun.as_deref(), Some(sun_name), "{climate}");
            assert_eq!(
                textures.glare.as_deref(),
                Some("Sky\\SunGlare.dds"),
                "{climate}"
            );
            let mut sky = new_sky(&rig, &asset_root());

            sky.update(&rig.device, &rig.queue, &night_sky_input(textures), 1.0);

            let status = sky.asset_status();
            assert_eq!(status.sun_texture, AssetState::Loaded, "{climate}");
            assert_eq!(status.sun_glare_texture, AssetState::Loaded, "{climate}");
        }
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the converted sky textures"]
    fn the_installed_sun_is_drawn_where_it_stands() {
        let name = "the_installed_sun_is_drawn_where_it_stands";
        let Some(rig) = rig(name) else {
            return;
        };
        let mut sky = new_sky(&rig, &asset_root());
        let textures = climate_textures(DEFAULT_CLIMATE);
        sky.update(&rig.device, &rig.queue, &night_sky_input(textures), 1.0);
        let feedback = feedback_group(&rig);

        let drawn = render(&rig, SKY_CLEAR, |pass| sky.draw(pass, &feedback));

        assert!(texel(&drawn, 8, 3).truncate().min_element() > 0.0);
        assert_eq!(texel(&drawn, 0, 15), Vec4::new(0.0, 0.0, 0.0, 1.0));
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR"]
    fn the_installed_inis_leave_the_sun_sizes_to_the_vanilla_defaults() {
        let ini = GameIni::load(std::path::Path::new(&format!("{}/data", asset_root()))).unwrap();

        let sizes = SunSizes::from_ini(&ini).unwrap();

        assert_eq!(sizes, SunSizes::VANILLA);
    }
}
