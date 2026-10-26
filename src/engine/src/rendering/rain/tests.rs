use glam::{UVec2, Vec2, Vec3};

use super::motion::{DropDrift, MAX_DROPS, MAX_SPLASHES, SPLASH_SECONDS};
use super::occlusion::{EXTENT, HEIGHT_RANGE, OcclusionWindow};
use super::uniform::{Fall, RainPlan, RainUniform};
use super::*;
use crate::rendering::shaders;

const EYE: Vec3 = Vec3::new(1234.5, 300.0, -987.25);
const STREAK_TEXTURE: &str = "effects\\test_rain.dds";

/// The rain of the weather the game ships for a downpour.
fn downpour() -> RainInput {
    RainInput {
        intensity: 1.0,
        occludable: true,
        fall_speed: 750.0,
        size: Vec2::new(1.5, 0.35),
        subtextures: UVec2::ZERO,
        density: 7.0,
        box_size: 1200.0,
        texture: STREAK_TEXTURE.into(),
        wind: Vec2::ZERO,
        tint: Vec3::ONE,
    }
}

fn view_of(window: Option<OcclusionWindow>) -> RainView {
    RainView {
        eye: EYE,
        forward: Vec3::NEG_Z,
        time: 10.0,
        window,
    }
}

fn mapped_view() -> RainView {
    view_of(Some(OcclusionWindow::around(EYE)))
}

fn plan_of(input: &RainInput, view: &RainView) -> RainPlan {
    RainPlan::new(input, view, &Fall::of(input), &DropDrift::default())
}

mod plan {
    use super::*;

    #[test]
    fn a_dry_sky_has_no_drops_and_no_splashes() {
        let plan = plan_of(
            &RainInput {
                intensity: 0.0,
                ..downpour()
            },
            &mapped_view(),
        );
        assert_eq!((plan.drops, plan.splashes), (0, 0));
    }

    #[test]
    fn the_drops_follow_the_density_of_the_weather_and_both_follow_the_intensity() {
        let full = plan_of(&downpour(), &mapped_view());
        assert_eq!((full.drops, full.splashes), (7000, MAX_SPLASHES));
        let half = plan_of(
            &RainInput {
                intensity: 0.5,
                ..downpour()
            },
            &mapped_view(),
        );
        assert_eq!((half.drops, half.splashes), (3500, MAX_SPLASHES / 2));
        let dense = plan_of(
            &RainInput {
                density: 100.0,
                ..downpour()
            },
            &mapped_view(),
        );
        assert_eq!(dense.drops, MAX_DROPS);
    }

    #[test]
    fn no_splash_is_drawn_until_the_map_shows_where_the_ground_is() {
        let plan = plan_of(&downpour(), &view_of(None));
        assert_eq!(plan.splashes, 0);
        assert_eq!(plan.drops, 7000, "the drops fall through every roof");
        assert_eq!(plan.uniform.window[3], 0.0);
        assert_eq!(plan.uniform.heights[2], 0.0, "no roof to shelter under");
    }

    #[test]
    fn the_map_is_handed_over_as_the_window_places_it() {
        let window = OcclusionWindow::around(EYE);
        let uniform = plan_of(&downpour(), &view_of(Some(window))).uniform;
        assert_eq!(
            uniform.window,
            [window.origin().x, window.origin().y, EXTENT, 1.0]
        );
        assert_eq!(uniform.heights, [window.bottom(), HEIGHT_RANGE, 1.0, 0.0]);
    }

    #[test]
    fn roofs_shelter_only_when_the_weather_lets_them() {
        let open = RainInput {
            occludable: false,
            ..downpour()
        };
        let plan = plan_of(&open, &mapped_view());
        assert_eq!(plan.uniform.heights[2], 0.0);
        assert_eq!(plan.uniform.window[3], 1.0, "the map still places splashes");
        assert_eq!(plan.splashes, MAX_SPLASHES);
    }

    #[test]
    fn the_units_of_the_weather_become_engine_units() {
        let input = RainInput {
            wind: Vec2::new(100.0, -50.0),
            ..downpour()
        };
        let fall = Fall::of(&input);
        assert!(
            (fall.velocity - Vec3::new(10.0, -75.0, -5.0))
                .abs()
                .max_element()
                < 1e-4
        );
        assert!((fall.edge - 120.0).abs() < 1e-3);

        let uniform = plan_of(&input, &mapped_view()).uniform;
        assert!(
            (uniform.across[3] - 1.5 * ENGINE_UNITS_PER_METRE).abs() < 1e-4,
            "the width"
        );
        assert!(
            (uniform.along[3] - 0.35 * ENGINE_UNITS_PER_METRE).abs() < 1e-4,
            "the length"
        );
    }

    #[test]
    fn a_box_is_never_smaller_than_a_few_metres() {
        let fall = Fall::of(&RainInput {
            box_size: 0.0,
            ..downpour()
        });
        assert!(fall.edge >= 4.0 * ENGINE_UNITS_PER_METRE);
    }

    #[test]
    fn a_texture_of_no_subtextures_is_one_whole_streak() {
        let uniform = plan_of(&downpour(), &mapped_view()).uniform;
        assert_eq!(&uniform.atlas[..2], &[1.0, 1.0]);
        let atlas = plan_of(
            &RainInput {
                subtextures: UVec2::new(4, 2),
                ..downpour()
            },
            &mapped_view(),
        )
        .uniform;
        assert_eq!(&atlas.atlas[..2], &[4.0, 2.0]);
    }

    #[test]
    fn streaks_lean_back_against_the_wind_and_face_the_camera() {
        let leaning = RainInput {
            wind: Vec2::new(250.0, 0.0),
            ..downpour()
        };
        let uniform = plan_of(&leaning, &mapped_view()).uniform;
        let along = Vec3::new(uniform.along[0], uniform.along[1], uniform.along[2]);
        let across = Vec3::new(uniform.across[0], uniform.across[1], uniform.across[2]);
        assert!(along.x < 0.0 && along.y > 0.0, "{along}");
        assert!(across.dot(along).abs() < 1e-5);
        assert!(across.dot(Vec3::NEG_Z).abs() < 1e-5, "{across}");
    }

    #[test]
    fn drops_need_a_size_to_be_drawn_but_splashes_do_not() {
        for size in [Vec2::ZERO, Vec2::new(1.5, 0.0), Vec2::new(-1.0, 0.35)] {
            let plan = plan_of(&RainInput { size, ..downpour() }, &mapped_view());
            assert_eq!(
                (plan.drops, plan.splashes),
                (0, MAX_SPLASHES),
                "size {size}"
            );
        }
    }

    #[test]
    fn a_weather_of_nonsense_still_hands_the_shader_finite_numbers() {
        let nonsense = RainInput {
            intensity: f32::NAN,
            fall_speed: f32::INFINITY,
            size: Vec2::NAN,
            density: f32::NAN,
            box_size: f32::NEG_INFINITY,
            wind: Vec2::new(f32::NAN, f32::INFINITY),
            tint: Vec3::new(f32::NAN, -1.0, f32::INFINITY),
            ..downpour()
        };
        let view = RainView {
            eye: Vec3::NAN,
            forward: Vec3::new(f32::INFINITY, 0.0, 0.0),
            time: f64::NAN,
            window: None,
        };
        let plan = plan_of(&nonsense, &view);
        assert_eq!((plan.drops, plan.splashes), (0, 0));
        let words: &[u32] = bytemuck::cast_slice(std::slice::from_ref(&plan.uniform));
        let floats = &words[..words.len() - 4];
        assert!(
            floats.iter().all(|&bits| f32::from_bits(bits).is_finite()),
            "{:?}",
            plan.uniform
        );
    }

    #[test]
    fn the_splashes_follow_the_clock_of_the_game() {
        let at = |time: f64| {
            plan_of(
                &downpour(),
                &RainView {
                    time,
                    ..mapped_view()
                },
            )
            .uniform
        };
        let start = at(0.0);
        assert_eq!((start.splash[0], start.splash_cycle[0]), (0.0, 0));
        let later = at(SPLASH_SECONDS * 3.25);
        assert_eq!(later.splash_cycle[0], 3);
        assert!((later.splash[0] - 0.25).abs() < 1e-6);
        assert_eq!(
            later.splash[1], start.splash[1],
            "the same circle of splashes"
        );
    }

    #[test]
    fn the_uniform_is_laid_out_as_the_shader_declares_it() {
        let module = naga::front::wgsl::parse_str(shaders::RAIN.wgsl()).expect("the shader parses");
        let span = module
            .types
            .iter()
            .find_map(|(_, ty)| match (&ty.name, &ty.inner) {
                (Some(name), naga::TypeInner::Struct { span, .. }) if name == "RainUniform" => {
                    Some(*span)
                }
                _ => None,
            })
            .expect("the shader declares RainUniform");
        assert_eq!(span as usize, std::mem::size_of::<RainUniform>());
    }

    #[test]
    fn the_shader_has_an_entry_point_for_each_of_the_two_pipelines() {
        let module = naga::front::wgsl::parse_str(shaders::RAIN.wgsl()).expect("the shader parses");
        let entries: Vec<_> = module
            .entry_points
            .iter()
            .map(|entry| (entry.name.as_str(), entry.stage))
            .collect();
        for expected in [
            ("vs_streak", naga::ShaderStage::Vertex),
            ("fs_streak", naga::ShaderStage::Fragment),
            ("vs_splash", naga::ShaderStage::Vertex),
            ("fs_splash", naga::ShaderStage::Fragment),
        ] {
            assert!(entries.contains(&expected), "{expected:?} in {entries:?}");
        }
    }
}

mod gpu {
    use std::path::Path;

    use bytemuck::Zeroable;
    use glam::{Mat4, Vec4};

    use super::*;
    use crate::rendering::lighting::frame_uniform::FrameUniform;
    use crate::rendering::lighting::frame_uniform::MAX_CASCADES;
    use crate::rendering::local_lights::buffers::ClusterBuffers;
    use crate::rendering::post::hdr_target::HDR_FORMAT;
    use crate::rendering::shadows::lamps::atlas_texture::LampAtlasTexture;
    use crate::rendering::shadows::settings::{ShadowConfig, ShadowFilter};
    use crate::rendering::shadows::shadow_maps::ShadowMaps;
    use crate::rendering::shadows::test_rig::{ShadowRig, Slab, rig};
    use crate::rendering::texture::Texture;
    use crate::rendering::water::ssr::tests::harness::{f16_bits_to_f32, read_mip};
    use occlusion::OcclusionMap;

    const TARGET: u32 = 128;
    const BACKGROUND: f32 = 0.5;
    /// How far a pixel must differ from the background to count as rain.
    const LIT: f32 = 0.02;
    const GENERATION: u64 = 3;
    const FOV_Y: f32 = std::f32::consts::FRAC_PI_3;
    const SPLASH_ATLAS: &str = "textures/effects/rainscene/rain_rainsplash_a_16x1_d.dds";

    #[derive(Clone, Copy)]
    struct Look {
        eye: Vec3,
        forward: Vec3,
    }

    impl Look {
        const AHEAD: Self = Self {
            eye: EYE,
            forward: Vec3::NEG_Z,
        };
    }

    #[derive(Clone, Copy)]
    struct Fog {
        colour: Vec3,
        opacity: f32,
    }

    const NO_FOG: Fog = Fog {
        colour: Vec3::ZERO,
        opacity: 0.0,
    };

    fn write_white_texture(root: &Path, relative: &str, size: UVec2) {
        let mut dds = ddsfile::Dds::new_dxgi(ddsfile::NewDxgiParams {
            height: size.y,
            width: size.x,
            depth: None,
            format: ddsfile::DxgiFormat::R8G8B8A8_UNorm,
            mipmap_levels: Some(1),
            array_layers: None,
            caps2: None,
            is_cubemap: false,
            resource_dimension: ddsfile::D3D10ResourceDimension::Texture2D,
            alpha_mode: ddsfile::AlphaMode::Unknown,
        })
        .expect("a valid texture description");
        dds.data.fill(255);
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().expect("a file below the root")).unwrap();
        dds.write(&mut std::fs::File::create(path).unwrap())
            .unwrap();
    }

    /// The renderer in a headless frame: a target to draw into, the real frame binding and
    /// optionally the occlusion map of a world made of `roofs`.
    struct Scene {
        rig: ShadowRig,
        frame: FrameBinding,
        rain: RainRenderer,
        map: OcclusionMap,
        target: wgpu::Texture,
        depth: wgpu::Texture,
        mapped: bool,
        /// The reverse-Z depth of the scene the rain is drawn over: 0 where nothing is.
        backdrop_depth: f32,
        projection: Mat4,
        asset_dir: tempfile::TempDir,
    }

    fn target_texture(device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rain_test_target"),
            size: wgpu::Extent3d {
                width: TARGET,
                height: TARGET,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }

    fn frame_binding(device: &wgpu::Device) -> FrameBinding {
        let config = ShadowConfig {
            resolution: 4,
            distance: 100.0,
            cascade_count: 1,
            split_blend: 0.0,
            filter: ShadowFilter::SingleTap,
            bias_scale: 1.0,
            cascade_min_texels: [0.0; MAX_CASCADES],
            skip_alpha_in_last_cascade: false,
        };
        FrameBinding::new(
            device,
            &FrameUniform::zeroed(),
            &ShadowMaps::new(device, &config),
            LampAtlasTexture::new(device, 4).view(),
            &ClusterBuffers::new(device),
        )
    }

    /// `roofs` of `None` leaves the map undrawn.
    fn scene(test: &str, roofs: Option<&[Slab]>) -> Option<Scene> {
        let mut rig = rig(test)?;
        let assets = tempfile::tempdir().unwrap();
        write_white_texture(
            assets.path(),
            "textures/effects/test_rain.dds",
            UVec2::new(4, 4),
        );
        write_white_texture(assets.path(), SPLASH_ATLAS, UVec2::new(64, 4));
        let mut map = OcclusionMap::new(&rig.device);
        if let Some(roofs) = roofs {
            rig.place(roofs, &[]);
            let mut encoder = rig
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            map.render(
                &rig.renderer,
                &mut encoder,
                &rig.queue,
                &rig.world.scene(),
                EYE,
                GENERATION,
                &rig.profiler,
            );
            rig.queue.submit(std::iter::once(encoder.finish()));
        }
        let frame = frame_binding(&rig.device);
        let rain = RainRenderer::new(
            &rig.device,
            &rig.queue,
            assets.path().to_str().unwrap(),
            &frame.layout,
            map.view(),
        );
        Some(Scene {
            frame,
            rain,
            map,
            target: target_texture(&rig.device, HDR_FORMAT),
            depth: target_texture(&rig.device, Texture::DEPTH_FORMAT),
            mapped: roofs.is_some(),
            backdrop_depth: 0.0,
            projection: Mat4::perspective_infinite_reverse_rh(FOV_Y, 1.0, 0.5),
            rig,
            asset_dir: assets,
        })
    }

    impl Scene {
        fn frame_uniform(&self, look: Look, fog: Fog) -> FrameUniform {
            let view = Mat4::look_to_rh(look.eye, look.forward, Vec3::Y);
            FrameUniform {
                view_proj: self.projection * view,
                camera_pos: look.eye.extend(1.0),
                camera_forward: look.forward.extend(0.0),
                fog_near_color: fog.colour.extend(0.0),
                fog_far_color: fog.colour.extend(1.0),
                fog_params: Vec4::new(1.0, fog.opacity, 0.0, 0.0),
                ..FrameUniform::zeroed()
            }
        }

        fn view(&self, look: Look, time: f64) -> RainView {
            RainView {
                eye: look.eye,
                forward: look.forward,
                time,
                window: self.mapped.then(|| self.map.window()).flatten(),
            }
        }

        fn draw_at(&mut self, input: &RainInput, look: Look, fog: Fog, time: f64) -> Vec<Vec4> {
            let device = &self.rig.device;
            let queue = &self.rig.queue;
            self.frame.write(queue, &self.frame_uniform(look, fog));
            self.rain
                .update(device, queue, input, &self.view(look, time));
            let target = self
                .target
                .create_view(&wgpu::TextureViewDescriptor::default());
            let depth = self
                .depth
                .create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("rain_test_clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(BACKGROUND),
                            g: f64::from(BACKGROUND),
                            b: f64::from(BACKGROUND),
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(self.backdrop_depth),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            }));
            self.rain.encode(
                &mut encoder,
                &target,
                &depth,
                &self.frame,
                &self.rig.profiler,
            );
            queue.submit(std::iter::once(encoder.finish()));
            read_target(device, queue, &self.target)
        }

        fn draw(&mut self, input: &RainInput, look: Look) -> Vec<Vec4> {
            self.draw_at(input, look, NO_FOG, 0.0)
        }
    }

    fn read_target(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
    ) -> Vec<Vec4> {
        read_mip(device, queue, texture, 0, UVec2::splat(TARGET), 8)
            .as_chunks::<8>()
            .0
            .iter()
            .map(|texel| {
                let channel = |index: usize| {
                    f16_bits_to_f32(u16::from_le_bytes([texel[index * 2], texel[index * 2 + 1]]))
                };
                Vec4::new(channel(0), channel(1), channel(2), channel(3))
            })
            .collect()
    }

    /// The (column, row) of every pixel the rain changed.
    fn lit(pixels: &[Vec4]) -> Vec<UVec2> {
        pixels
            .iter()
            .enumerate()
            .filter(|(_, pixel)| {
                (pixel.truncate() - Vec3::splat(BACKGROUND))
                    .abs()
                    .max_element()
                    > LIT
            })
            .map(|(index, _)| UVec2::new(index as u32 % TARGET, index as u32 / TARGET))
            .collect()
    }

    fn roof(height_above_eye: f32) -> Slab {
        Slab {
            centre: EYE + Vec3::new(0.0, height_above_eye, 0.0),
            half_size: 100.0,
            alpha_tested: false,
        }
    }

    #[test]
    fn both_pipelines_are_created_against_the_real_frame_layout() {
        let Some(scene) = scene(
            "both_pipelines_are_created_against_the_real_frame_layout",
            None,
        ) else {
            return;
        };
        assert_eq!(scene.rain.asset_status().splashes, RainTextureState::Loaded);
        assert_eq!(
            scene.rain.asset_status().streaks,
            RainTextureState::NotRequested
        );
    }

    #[test]
    fn a_dry_sky_records_no_draw_and_leaves_the_scene_as_it_is() {
        let Some(mut scene) = scene("a_dry_sky_records_no_draw", None) else {
            return;
        };
        let dry = RainInput {
            intensity: 0.0,
            ..downpour()
        };

        let pixels = scene.draw(&dry, Look::AHEAD);

        assert!(scene.rain.draws.is_none());
        assert_eq!(lit(&pixels).len(), 0, "rain over a clear scene");
        assert_eq!(
            scene.rain.asset_status().streaks,
            RainTextureState::NotRequested,
            "nothing is loaded while it does not rain"
        );
    }

    #[test]
    fn a_downpour_draws_streaks_over_the_scene() {
        let Some(mut scene) = scene("a_downpour_draws_streaks_over_the_scene", None) else {
            return;
        };

        let pixels = scene.draw(&downpour(), Look::AHEAD);

        assert_eq!(
            scene.rain.draws,
            Draws {
                drops: 7000,
                splashes: 0
            }
        );
        let lit = lit(&pixels);
        assert!(lit.len() > 200, "{} pixels", lit.len());
        let brightest = pixels.iter().map(|pixel| pixel.x).fold(0.0, f32::max);
        assert!(brightest > BACKGROUND + 0.1, "{brightest}");
    }

    #[test]
    fn scenery_in_front_hides_the_rain() {
        let Some(mut scene) = scene("scenery_in_front_hides_the_rain", None) else {
            return;
        };
        let in_the_open = lit(&scene.draw(&downpour(), Look::AHEAD)).len();
        scene.backdrop_depth = 1.0;

        let behind_scenery = lit(&scene.draw(&downpour(), Look::AHEAD)).len();

        assert!(in_the_open > 200, "{in_the_open} pixels");
        assert_eq!(behind_scenery, 0);
    }

    #[test]
    fn a_roof_over_the_camera_shelters_the_drops_under_it() {
        let Some(mut scene) = scene(
            "a_roof_over_the_camera_shelters_the_drops",
            Some(&[roof(70.0)]),
        ) else {
            return;
        };

        let sheltered = scene.draw(&downpour(), Look::AHEAD);
        let open = scene.draw(
            &RainInput {
                occludable: false,
                ..downpour()
            },
            Look::AHEAD,
        );

        assert_eq!(lit(&sheltered).len(), 0, "pixels of rain under the roof");
        assert!(lit(&open).len() > 200, "{} pixels", lit(&open).len());
    }

    #[test]
    fn a_roof_at_the_height_of_the_camera_cuts_the_streaks_off_at_the_horizon() {
        let Some(mut scene) = scene(
            "a_roof_at_the_height_of_the_camera_cuts_the_streaks",
            Some(&[roof(0.0)]),
        ) else {
            return;
        };

        let lit = lit(&scene.draw(&downpour(), Look::AHEAD));

        let horizon = TARGET / 2;
        let above = lit.iter().filter(|pixel| pixel.y < horizon).count();
        let below = lit.iter().filter(|pixel| pixel.y > horizon + 1).count();
        assert!(above > 100, "{above} pixels over the horizon");
        assert_eq!(below, 0, "streaks reach under the roof");
    }

    #[test]
    fn a_camera_one_box_edge_further_sees_the_same_rain() {
        let Some(mut scene) = scene("a_camera_one_box_edge_further_sees_the_same_rain", None)
        else {
            return;
        };
        let edge = Fall::of(&downpour()).edge;
        let first = scene.draw(&downpour(), Look::AHEAD);

        let second = scene.draw(
            &downpour(),
            Look {
                eye: EYE + Vec3::new(edge, 0.0, -2.0 * edge),
                ..Look::AHEAD
            },
        );

        let difference: f32 = first
            .iter()
            .zip(&second)
            .map(|(a, b)| (*a - *b).abs().max_element())
            .sum();
        let seen = lit(&first).len() as f32;
        assert!(seen > 200.0);
        assert!(
            difference < 0.02 * seen,
            "{difference} against {seen} pixels"
        );
    }

    #[test]
    fn the_drops_stay_where_they_are_in_the_world_while_the_camera_moves() {
        let Some(mut scene) = scene("the_drops_stay_where_they_are_in_the_world", None) else {
            return;
        };
        // One engine unit to the pixel, so a camera step of whole units shifts the image by
        // whole pixels.
        let half_view = TARGET as f32 / 2.0;
        scene.projection =
            Mat4::orthographic_rh(-half_view, half_view, -half_view, half_view, 2000.0, 0.0);
        let step = 24;
        let few = RainInput {
            density: 0.15,
            ..downpour()
        };
        let before = lit(&scene.draw(&few, Look::AHEAD));
        let after = lit(&scene.draw(
            &few,
            Look {
                eye: EYE + Vec3::new(step as f32, 0.0, 0.0),
                ..Look::AHEAD
            },
        ));

        // The part of the view both cameras see well inside their boxes, where nothing fades.
        let seen_by_both =
            |pixel: &&UVec2| (28..100).contains(&pixel.y) && (52..100).contains(&pixel.x);
        let before: Vec<UVec2> = before.iter().filter(seen_by_both).copied().collect();
        let moved_on = |pixel: &UVec2| UVec2::new(pixel.x - step, pixel.y);
        let missing = before
            .iter()
            .filter(|pixel| !after.contains(&moved_on(pixel)))
            .count();
        assert!(before.len() > 100, "{} pixels", before.len());
        assert!(
            missing * 20 < before.len(),
            "{missing} of {} pixels did not move with the world",
            before.len()
        );
    }

    #[test]
    fn the_rain_falls_as_time_passes() {
        let Some(mut scene) = scene("the_rain_falls_as_time_passes", None) else {
            return;
        };
        scene.draw_at(&downpour(), Look::AHEAD, NO_FOG, 5.0);
        let before = scene.draw_at(&downpour(), Look::AHEAD, NO_FOG, 5.0);

        let after = scene.draw_at(&downpour(), Look::AHEAD, NO_FOG, 5.05);

        let moved = before
            .iter()
            .zip(&after)
            .filter(|(a, b)| (**a - **b).abs().max_element() > LIT)
            .count();
        assert!(moved > 200, "{moved} pixels changed");
    }

    #[test]
    fn splashes_stand_only_on_the_ground_the_map_knows() {
        let ground = Slab {
            centre: EYE + Vec3::new(60.0, -35.0, -60.0),
            half_size: 60.0,
            alpha_tested: false,
        };
        let Some(mut scene) = scene(
            "splashes_stand_only_on_the_ground_the_map_knows",
            Some(&[ground]),
        ) else {
            return;
        };
        let looking_down = Look {
            eye: EYE,
            forward: Vec3::new(0.0, -1.0, -1.0).normalize(),
        };
        let splashes_only = RainInput {
            density: 0.0,
            ..downpour()
        };

        let lit = lit(&scene.draw(&splashes_only, looking_down));

        let right = lit.iter().filter(|pixel| pixel.x >= TARGET / 2).count();
        let far_left = lit.iter().filter(|pixel| pixel.x < TARGET / 2 - 8).count();
        assert!(right > 30, "{right} pixels on the ground");
        assert_eq!(far_left, 0, "splashes where the map holds no height");
    }

    #[test]
    fn splashes_stand_at_the_height_the_map_holds() {
        let Some(mut scene) = scene(
            "splashes_stand_at_the_height_the_map_holds",
            Some(&[roof(0.0)]),
        ) else {
            return;
        };
        let splashes_only = RainInput {
            density: 0.0,
            ..downpour()
        };

        let lit = lit(&scene.draw(&splashes_only, Look::AHEAD));

        let horizon = TARGET / 2;
        let lowest = lit.iter().map(|pixel| pixel.y).max();
        assert!(lit.len() > 20, "{} pixels", lit.len());
        assert!(
            lowest.is_some_and(|row| (horizon - 2..horizon + 2).contains(&row)),
            "the splashes end at row {lowest:?}, the ground at the horizon row {horizon}"
        );
    }

    #[test]
    fn splashes_wait_for_the_map() {
        let Some(mut scene) = scene("splashes_wait_for_the_map", None) else {
            return;
        };
        let splashes_only = RainInput {
            density: 0.0,
            ..downpour()
        };

        let pixels = scene.draw(
            &splashes_only,
            Look {
                eye: EYE,
                forward: Vec3::NEG_Y,
            },
        );

        assert!(scene.rain.draws.is_none());
        assert_eq!(lit(&pixels).len(), 0, "rain over a clear scene");
    }

    #[test]
    fn the_fog_covers_the_rain_like_everything_else() {
        let Some(mut scene) = scene("the_fog_covers_the_rain_like_everything_else", None) else {
            return;
        };
        let clear = scene.draw(&downpour(), Look::AHEAD);
        let fogged = scene.draw_at(
            &downpour(),
            Look::AHEAD,
            Fog {
                colour: Vec3::new(1.0, 0.0, 0.0),
                opacity: 1.0,
            },
            0.0,
        );

        let in_clear = lit(&clear);
        let in_fog = lit(&fogged);
        assert!(in_clear.len() > 200 && in_fog.len() > 200);
        let green_of = |pixels: &[Vec4], lit: &[UVec2]| {
            lit.iter()
                .map(|pixel| pixels[(pixel.y * TARGET + pixel.x) as usize].y)
                .fold(f32::INFINITY, f32::min)
        };
        assert!(
            green_of(&clear, &in_clear) >= BACKGROUND - 1e-3,
            "white rain"
        );
        assert!(green_of(&fogged, &in_fog) < BACKGROUND - 0.02, "red rain");
    }

    #[test]
    fn a_missing_streak_texture_is_reported_and_draws_no_streaks() {
        let Some(mut scene) = scene("a_missing_streak_texture_is_reported", None) else {
            return;
        };
        let missing = RainInput {
            texture: "effects\\nothing_here.dds".into(),
            ..downpour()
        };

        let pixels = scene.draw(&missing, Look::AHEAD);

        assert_eq!(lit(&pixels).len(), 0, "rain over a clear scene");
        match &scene.rain.asset_status().streaks {
            RainTextureState::Failed(message) => {
                assert!(message.contains("nothing_here.dds"), "{message}");
                assert!(message.contains("not in the install"), "{message}");
            }
            other => panic!("expected a failed load, found {other:?}"),
        }
        let found = scene.draw(&downpour(), Look::AHEAD);
        assert!(
            lit(&found).len() > 200,
            "the next weather brings its texture"
        );
        assert_eq!(scene.rain.asset_status().streaks, RainTextureState::Loaded);
    }

    #[test]
    fn a_texture_that_is_still_the_streak_texture_is_not_loaded_again() {
        let Some(mut scene) = scene("a_texture_that_is_still_the_streak_texture", None) else {
            return;
        };
        scene.draw(&downpour(), Look::AHEAD);
        std::fs::remove_file(
            scene
                .asset_dir
                .path()
                .join("textures/effects/test_rain.dds"),
        )
        .unwrap();

        let pixels = scene.draw(&downpour(), Look::AHEAD);

        assert_eq!(scene.rain.asset_status().streaks, RainTextureState::Loaded);
        assert!(lit(&pixels).len() > 200);
    }
}
