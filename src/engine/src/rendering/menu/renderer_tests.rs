//! The menu renderer on the shared headless GPU device: what ends up in the pixels.

use super::layout::{
    RowRect,
    test_support::{standard, ui_layout},
};
use super::loading_icon::test_support::{library, svg};
use super::loading_panel::{PanelContent, PanelText};
use super::renderer::{Backdrop, MenuError, MenuRenderer, MenuScene, OverlayForeground};
use crate::game_flow::{GameFlow, MenuView, StartMode};
use crate::rendering::game_settings::SafeZone;
use crate::rendering::gpu_test::test_device;
use crate::rendering::hud::field_text::TextItem;
use crate::rendering::loading_screen::test_support::{
    cube_art, write_solid_texture, write_two_band_texture,
};
use crate::rendering::loading_screen::{
    BackgroundImages, background_uv, CpuArt, LoadedArt, LoadingArtRenderer, LoadingBackground, graded,
};
use crate::rendering::post::tonemap::ui_gamma_format;
use crate::rendering::ui_colour::UiTint;
use crate::rendering::ui_fonts::{UiFontError, test_support as fonts};

const HUD_GREEN: UiTint = UiTint {
    color: [0.0, 1.0, 0.0],
    opacity: 1.0,
};

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Half the 1280x720 reference stage, so the standard layout's rows sit at known pixels.
const SIZE: (u32, u32) = (640, 360);
/// A pixel inside the standard layout's panel (stage (90, 190), 140 x 150) and left of every row.
const PANEL_ONLY: (u32, u32) = (47, 97);
const GREEN: wgpu::Color = wgpu::Color {
    r: 0.0,
    g: 0.4,
    b: 0.0,
    a: 1.0,
};
const RED: wgpu::Color = wgpu::Color {
    r: 1.0,
    g: 0.0,
    b: 0.0,
    a: 1.0,
};

struct Target {
    texture: wgpu::Texture,
    size: (u32, u32),
}

impl Target {
    fn new(device: &wgpu::Device) -> Self {
        Self::sized(device, SIZE)
    }

    fn sized(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("menu_test_target"),
            size: extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[FORMAT.remove_srgb_suffix()],
        });
        Self { texture, size }
    }

    fn view(&self) -> wgpu::TextureView {
        self.texture
            .create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn fill(&self, device: &wgpu::Device, queue: &wgpu::Queue, color: wgpu::Color) {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
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
        queue.submit(std::iter::once(encoder.finish()));
    }

    fn render(&self, device: &wgpu::Device, queue: &wgpu::Queue, scene: MenuScene<'_>) {
        self.draw(
            &mut renderer(device, queue, HUD_GREEN),
            device,
            queue,
            scene,
        );
    }

    /// Draws `scene` with a renderer that keeps its state from earlier draws.
    fn draw(
        &self,
        renderer: &mut MenuRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: MenuScene<'_>,
    ) {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let view = renderer.target_view(&self.texture);
        renderer.render(device, queue, &mut encoder, &view, self.size, scene);
        queue.submit(std::iter::once(encoder.finish()));
    }

    fn pixels(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Pixels {
        let (width, height) = self.size;
        let bytes_per_row = width * 4;
        assert_eq!(bytes_per_row % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 0);
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("menu_test_readback"),
            size: u64::from(bytes_per_row * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            extent(self.size),
        );
        queue.submit(std::iter::once(encoder.finish()));
        let (sender, receiver) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                sender
                    .send(result)
                    .expect("receiver outlives the map callback");
            });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("menu readback poll failed");
        receiver.recv().unwrap().expect("staging buffer maps");
        let bytes = staging
            .slice(..)
            .get_mapped_range()
            .expect("staging range is mapped")
            .to_vec();
        Pixels {
            bytes,
            size: self.size,
        }
    }
}

fn extent(size: (u32, u32)) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size.0,
        height: size.1,
        depth_or_array_layers: 1,
    }
}

const SAFE_ZONE: SafeZone = SafeZone { x: 64.0, y: 36.0 };

/// A renderer over the standard layout and a two-frame logo, tinted with `tint`.
fn renderer(device: &wgpu::Device, queue: &wgpu::Queue, tint: UiTint) -> MenuRenderer {
    let art_dir = tempfile::tempdir().unwrap();
    let art = library(art_dir.path(), &[svg(0), svg(20)]);
    MenuRenderer::new(
        device,
        queue,
        FORMAT,
        ui_layout(&fonts::family()),
        &fonts::ui_fonts(),
        &art,
        tint,
        SAFE_ZONE,
    )
    .expect("the test font has the layout's family")
}

struct Pixels {
    bytes: Vec<u8>,
    size: (u32, u32),
}

impl Pixels {
    fn at(&self, x: u32, y: u32) -> [u8; 3] {
        let start = ((y * self.size.0 + x) * 4) as usize;
        [
            self.bytes[start],
            self.bytes[start + 1],
            self.bytes[start + 2],
        ]
    }

    /// Every pixel whose colour satisfies `matches`, as `(x, y)`.
    fn positions_where(&self, matches: impl Fn([u8; 3]) -> bool) -> Vec<(u32, u32)> {
        (0..self.size.1)
            .flat_map(|y| (0..self.size.0).map(move |x| (x, y)))
            .filter(|&(x, y)| matches(self.at(x, y)))
            .collect()
    }

    /// The pixel near the right end of a row, past where its text can reach.
    fn at_row_end(&self, row: &RowRect) -> [u8; 3] {
        self.at(
            (row.rect.x + row.rect.w - 4.0) as u32,
            (row.rect.y + row.rect.h / 2.0) as u32,
        )
    }
}

fn menu_flow() -> GameFlow {
    GameFlow::new(Vec::new(), standard().list.visible_rows, StartMode::Menu)
}

#[test]
fn a_list_scene_is_black_with_a_white_bar_behind_only_the_selected_row() {
    let Some((device, queue)) = test_device("menu list scene") else {
        return;
    };
    let target = Target::new(&device);
    target.fill(&device, &queue, GREEN);
    let flow = menu_flow();

    target.render(&device, &queue, MenuScene::List(flow.menu_view().unwrap()));
    let pixels = target.pixels(&device, &queue);

    let rows: Vec<_> = standard().row_rects(SIZE.0, SIZE.1, 0).collect();
    assert_eq!(pixels.at_row_end(&rows[0]), [0, 255, 0], "selected row");
    assert_eq!(
        pixels.at_row_end(&rows[1]),
        pixels.at(PANEL_ONLY.0, PANEL_ONLY.1),
        "an unselected row shows only the panel behind it"
    );
    assert_eq!(
        pixels.at(2, 2),
        [0, 0, 0],
        "the background is cleared to black"
    );
}

#[test]
fn the_selection_bar_follows_the_selected_row() {
    let Some((device, queue)) = test_device("menu selection bar") else {
        return;
    };
    let target = Target::new(&device);
    let mut flow = menu_flow();
    flow.handle(crate::game_flow::FlowEvent::NavDown);

    target.render(&device, &queue, MenuScene::List(flow.menu_view().unwrap()));
    let pixels = target.pixels(&device, &queue);

    let rows: Vec<_> = standard().row_rects(SIZE.0, SIZE.1, 0).collect();
    assert_eq!(
        pixels.at_row_end(&rows[0]),
        pixels.at(PANEL_ONLY.0, PANEL_ONLY.1)
    );
    assert_eq!(pixels.at_row_end(&rows[1]), [0, 255, 0]);
}

#[test]
fn an_overlay_scene_blends_black_over_the_existing_frame() {
    let Some((device, queue)) = test_device("menu overlay scene") else {
        return;
    };
    let sample = |alpha: f32| {
        let target = Target::new(&device);
        target.fill(&device, &queue, RED);
        target.render(&device, &queue, overlay(alpha, None));
        target.pixels(&device, &queue).at(10, 10)
    };

    assert_eq!(
        sample(0.0),
        [255, 0, 0],
        "a transparent overlay leaves the frame"
    );
    let half = sample(0.5);
    assert!(
        (126..=129).contains(&half[0]),
        "half black in gamma space: {half:?}"
    );
    assert_eq!(sample(1.0), [0, 0, 0], "an opaque overlay hides the frame");
}

/// The fixture logo's art is locked to the safe zone's bottom-right corner: with a 64 x 36 safe
/// zone it covers the stage rectangle (1196, 664) to (1216, 684), at half scale the pixels (598, 332)
/// to (608, 342), its left half in the first frame and its right half in the second.
const LOGO_LEFT_HALF: (u32, u32) = (600, 337);
const LOGO_RIGHT_HALF: (u32, u32) = (605, 337);

fn overlay(alpha: f32, loading_seconds: Option<f64>) -> MenuScene<'static> {
    MenuScene::Overlay {
        alpha,
        loading_seconds,
        panel: None,
    }
}

#[test]
fn an_overlay_without_loading_seconds_draws_no_logo() {
    let Some((device, queue)) = test_device("menu overlay without logo") else {
        return;
    };
    let target = Target::new(&device);
    target.render(&device, &queue, overlay(1.0, None));
    let pixels = target.pixels(&device, &queue);
    assert!(
        pixels.positions_where(|rgb| rgb != [0, 0, 0]).is_empty(),
        "nothing but black"
    );
}

#[test]
fn the_loading_logo_renders_without_a_gpu_validation_error() {
    let Some((device, queue)) = test_device("menu loading logo") else {
        return;
    };
    let target = Target::new(&device);
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

    target.render(&device, &queue, overlay(1.0, Some(0.0)));

    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");
    assert_eq!(target.pixels(&device, &queue).at(2, 2), [0, 0, 0]);
}

#[test]
fn the_loading_logo_is_drawn_in_the_hud_colour_on_the_rectangle_the_layout_places() {
    let Some((device, queue)) = test_device("menu loading logo placement") else {
        return;
    };
    let target = Target::new(&device);
    target.render(&device, &queue, overlay(1.0, Some(0.0)));
    let pixels = target.pixels(&device, &queue);

    assert_eq!(pixels.at(LOGO_LEFT_HALF.0, LOGO_LEFT_HALF.1), [0, 255, 0]);
    assert_eq!(pixels.at(LOGO_RIGHT_HALF.0, LOGO_RIGHT_HALF.1), [0, 0, 0]);
    let lit = pixels.positions_where(|rgb| rgb != [0, 0, 0]);
    let (left, right) = (
        lit.iter().map(|&(x, _)| x).min().unwrap(),
        lit.iter().map(|&(x, _)| x).max().unwrap(),
    );
    let (top, bottom) = (
        lit.iter().map(|&(_, y)| y).min().unwrap(),
        lit.iter().map(|&(_, y)| y).max().unwrap(),
    );
    assert_eq!((left, right, top, bottom), (598, 602, 332, 341));
    assert!(
        pixels
            .positions_where(|[r, _, b]| r != 0 || b != 0)
            .is_empty(),
        "every lit pixel is HUD green, none white"
    );
}

#[test]
fn the_logo_frame_follows_the_loading_seconds_and_loops() {
    let Some((device, queue)) = test_device("menu loading logo frames") else {
        return;
    };
    let target = Target::new(&device);
    let lit_half = |seconds: f64| {
        target.render(&device, &queue, overlay(1.0, Some(seconds)));
        let pixels = target.pixels(&device, &queue);
        (
            pixels.at(LOGO_LEFT_HALF.0, LOGO_LEFT_HALF.1) != [0, 0, 0],
            pixels.at(LOGO_RIGHT_HALF.0, LOGO_RIGHT_HALF.1) != [0, 0, 0],
        )
    };

    assert_eq!(lit_half(0.0), (true, false), "loading starts on frame 1");
    assert_eq!(lit_half(0.5), (false, true), "frame 2 at 2 frames a second");
    assert_eq!(lit_half(1.0), (true, false), "the two-frame loop restarts");
    assert_eq!(lit_half(1.5), (false, true));
}

#[test]
fn the_logo_fades_with_the_black_layer() {
    let Some((device, queue)) = test_device("menu loading logo fade") else {
        return;
    };
    let green_at = |alpha: f32| {
        let target = Target::new(&device);
        target.render(&device, &queue, overlay(alpha, Some(0.0)));
        target
            .pixels(&device, &queue)
            .at(LOGO_LEFT_HALF.0, LOGO_LEFT_HALF.1)[1]
    };

    assert_eq!(green_at(1.0), 255);
    let half = green_at(0.5);
    assert!(
        (126..=129).contains(&half),
        "half the HUD green over a black layer of half opacity: {half}"
    );
    assert_eq!(green_at(0.0), 0, "a transparent overlay draws no logo");
}

#[test]
fn the_panel_behind_the_list_is_drawn_faintly_at_its_alpha() {
    let Some((device, queue)) = test_device("menu panel background") else {
        return;
    };
    let target = Target::new(&device);
    let flow = menu_flow();

    target.render(&device, &queue, MenuScene::List(flow.menu_view().unwrap()));
    let pixels = target.pixels(&device, &queue);

    // At half scale the panel spans pixels (45, 95), 70 x 75.
    let inside = pixels.at(PANEL_ONLY.0, PANEL_ONLY.1);
    assert!(
        (1..=2).contains(&inside[1]),
        "alpha 0.008 of #a5a5a5 composited in gamma space is about 1: {inside:?}"
    );
    assert_eq!((inside[0], inside[2]), (0, 0), "tinted HUD green");
    assert_eq!(pixels.at(40, 97), [0, 0, 0], "left of the panel");
    assert_eq!(pixels.at(47, 90), [0, 0, 0], "above the panel");
}

#[test]
fn a_font_family_the_fonts_lack_is_reported_with_its_name() {
    let Some((device, queue)) = test_device("menu missing family") else {
        return;
    };
    let art_dir = tempfile::tempdir().unwrap();
    let art = library(art_dir.path(), &[svg(0)]);
    let result = MenuRenderer::new(
        &device,
        &queue,
        FORMAT,
        ui_layout("Test Family"),
        &fonts::ui_fonts(),
        &art,
        HUD_GREEN,
        SAFE_ZONE,
    );
    assert!(matches!(
        result.err(),
        Some(MenuError::Font(UiFontError::MissingFamily { family, .. })) if family == "Test Family"
    ));
}

#[test]
fn a_label_wider_than_its_row_shrinks_to_fit_instead_of_overflowing() {
    let Some((device, queue)) = test_device("menu long label") else {
        return;
    };
    let target = Target::new(&device);
    let labels = ["SHORT", "SANCTUARY HILLS"];
    let view = MenuView {
        labels: &labels,
        selected: 0,
        first_visible: 0,
    };

    target.render(&device, &queue, MenuScene::List(view));
    let pixels = target.pixels(&device, &queue);

    let rows: Vec<_> = standard().row_rects(SIZE.0, SIZE.1, 0).collect();
    let long_row = rows[1].rect;
    let text = pixels.positions_where(|[_, g, _]| g > 128);
    let in_long_row =
        |&&(_, y): &&(u32, u32)| (y as f32) >= long_row.y && (y as f32) < long_row.y + long_row.h;
    let rightmost = text
        .iter()
        .filter(in_long_row)
        .map(|&(x, _)| x)
        .max()
        .expect("the long label is drawn");
    assert!(
        (rightmost as f32) < long_row.x + long_row.w,
        "text ends at {rightmost}, the row at {}",
        long_row.x + long_row.w
    );
}

#[test]
fn the_list_is_tinted_with_the_hud_colour_throughout() {
    let Some((device, queue)) = test_device("menu tint") else {
        return;
    };
    let target = Target::new(&device);
    let flow = menu_flow();

    target.render(&device, &queue, MenuScene::List(flow.menu_view().unwrap()));
    let pixels = target.pixels(&device, &queue);

    assert!(
        pixels
            .positions_where(|[r, _, b]| r > 0 || b > 0)
            .is_empty(),
        "no red or blue anywhere: every element is multiplied with the HUD green"
    );
    assert_ne!(pixels.positions_where(|[_, g, _]| g == 255), []);
}

/// The extreme pixels `(left, right, top, bottom)` of `lit`.
fn bounds_of(lit: &[(u32, u32)]) -> (u32, u32, u32, u32) {
    let xs = lit.iter().map(|&(x, _)| x);
    let ys = lit.iter().map(|&(_, y)| y);
    (
        xs.clone().min().unwrap(),
        xs.max().unwrap(),
        ys.clone().min().unwrap(),
        ys.max().unwrap(),
    )
}

#[test]
fn the_logo_takes_each_channel_of_a_tint_with_three_distinct_channels() {
    let Some((device, queue)) = test_device("menu loading logo tint") else {
        return;
    };
    let tint = UiTint {
        color: [0.2, 0.5, 0.8],
        opacity: 1.0,
    };
    let [red, green, blue, _] = tint.text_rgba();
    assert!(red != 0 && red != green && green != blue && red != blue);
    let target = Target::new(&device);

    target.draw(
        &mut renderer(&device, &queue, tint),
        &device,
        &queue,
        overlay(1.0, Some(0.0)),
    );

    let pixels = target.pixels(&device, &queue);
    assert_eq!(
        pixels.at(LOGO_LEFT_HALF.0, LOGO_LEFT_HALF.1),
        [red, green, blue]
    );
    let lit = pixels.positions_where(|rgb| rgb != [0, 0, 0]);
    assert!(
        lit.iter()
            .all(|&(x, y)| pixels.at(x, y) == [red, green, blue]),
        "every lit pixel is the tint exactly, with no blurred edge"
    );
}

#[test]
fn a_resize_rasterises_the_logo_again_at_the_new_size() {
    let Some((device, queue)) = test_device("menu loading logo resize") else {
        return;
    };
    let mut renderer = renderer(&device, &queue, HUD_GREEN);
    let small = Target::new(&device);
    let large = Target::sized(&device, (1280, 720));
    let lit_bounds = |target: &Target, renderer: &mut MenuRenderer| {
        target.draw(renderer, &device, &queue, overlay(1.0, Some(0.0)));
        let pixels = target.pixels(&device, &queue);
        let lit = pixels.positions_where(|rgb| rgb != [0, 0, 0]);
        assert!(lit.iter().all(|&(x, y)| pixels.at(x, y) == [0, 255, 0]));
        (bounds_of(&lit), lit.len())
    };

    assert_eq!(
        lit_bounds(&small, &mut renderer),
        ((598, 602, 332, 341), 50)
    );
    assert_eq!(
        lit_bounds(&large, &mut renderer),
        ((1196, 1205, 664, 683), 200)
    );
    assert_eq!(
        lit_bounds(&small, &mut renderer),
        ((598, 602, 332, 341), 50)
    );
}

#[test]
fn a_fractional_placement_is_drawn_on_the_snapped_pixel_rectangle_texel_for_pixel() {
    let Some((device, queue)) = test_device("menu loading logo snapping") else {
        return;
    };
    // The stage fits at 0.6944 here: the logo's rectangle is at (837.67, 461.11), 13.89 wide.
    let target = Target::sized(&device, (896, 500));
    let mut renderer = renderer(&device, &queue, HUD_GREEN);

    target.draw(&mut renderer, &device, &queue, overlay(1.0, Some(0.0)));

    let pixels = target.pixels(&device, &queue);
    let lit = pixels.positions_where(|rgb| rgb != [0, 0, 0]);
    assert_eq!(
        bounds_of(&lit),
        (838, 844, 461, 474),
        "the left half of 14 x 14"
    );
    assert!(
        lit.iter().all(|&(x, y)| pixels.at(x, y) == [0, 255, 0]),
        "no pixel is a blend of two texels"
    );
    assert_eq!(lit.len(), 7 * 14);
}

#[test]
fn a_logo_too_big_to_rasterise_is_hidden_until_the_size_changes_or_a_new_load_begins() {
    let Some((device, queue)) = test_device("menu loading logo too big") else {
        return;
    };
    let target = Target::new(&device);
    let mut renderer = renderer(&device, &queue, HUD_GREEN);
    let huge = (600_000, 337_500);
    let draw_at = |fb: (u32, u32), seconds: f64, renderer: &mut MenuRenderer| {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let view = renderer.target_view(&target.texture);
        renderer.render(
            &device,
            &queue,
            &mut encoder,
            &view,
            fb,
            overlay(1.0, Some(seconds)),
        );
        queue.submit(std::iter::once(encoder.finish()));
    };

    draw_at(huge, 1.0, &mut renderer);
    assert!(renderer.loading_icon_failed());
    draw_at(huge, 1.5, &mut renderer);
    assert!(
        renderer.loading_icon_failed(),
        "still hidden on the same size"
    );
    draw_at(SIZE, 2.0, &mut renderer);
    assert!(!renderer.loading_icon_failed(), "a new size tries again");
    draw_at(huge, 3.0, &mut renderer);
    assert!(renderer.loading_icon_failed());
    draw_at(huge, 0.0, &mut renderer);
    assert!(
        renderer.loading_icon_failed(),
        "a new load tries again, and fails again"
    );
}

#[test]
fn a_new_load_restarts_the_logo_and_the_reveal_continues_it() {
    use crate::game_flow::{Phase, Readiness};

    let Some((device, queue)) = test_device("menu loading logo flow") else {
        return;
    };
    let target = Target::new(&device);
    let mut renderer = renderer(&device, &queue, HUD_GREEN);
    let lit_half = |flow: &GameFlow, renderer: &mut MenuRenderer| {
        target.draw(
            renderer,
            &device,
            &queue,
            overlay(flow.overlay_alpha().max(0.01), flow.loading_seconds()),
        );
        let pixels = target.pixels(&device, &queue);
        (
            pixels.at(LOGO_LEFT_HALF.0, LOGO_LEFT_HALF.1) != [0, 0, 0],
            pixels.at(LOGO_RIGHT_HALF.0, LOGO_RIGHT_HALF.1) != [0, 0, 0],
        )
    };
    let mut flow = GameFlow::new(Vec::new(), standard().list.visible_rows, StartMode::Direct);

    assert_eq!(lit_half(&flow, &mut renderer), (true, false), "frame 1");
    for _ in 0..6 {
        flow.tick(0.1);
    }
    assert_eq!(
        lit_half(&flow, &mut renderer),
        (false, true),
        "frame 2 at 0.6 s"
    );

    flow.hold_at_main_menu();
    flow.release_to_loading();
    assert_eq!(
        lit_half(&flow, &mut renderer),
        (true, false),
        "a new load restarts at frame 1"
    );

    for _ in 0..7 {
        flow.tick(0.1);
    }
    assert_eq!(flow.phase(), Phase::Loading);
    while flow.phase() == Phase::Loading {
        flow.tick(0.1);
        flow.readiness(Readiness {
            full_ring_complete: true,
            player_ready: true,
            ..Readiness::default()
        });
        assert!(flow.loading_seconds().unwrap() < 5.0, "the load ends");
    }
    assert_eq!(flow.phase(), Phase::Revealing);
    let seconds = flow.loading_seconds().unwrap();
    assert!(
        seconds >= 0.7,
        "the reveal did not restart the clock: {seconds}"
    );
    let continued = (seconds * 2.0).floor() as u32 % 2;
    assert_eq!(
        lit_half(&flow, &mut renderer),
        (continued == 0, continued == 1),
        "the reveal shows the frame the load's clock gives, at {seconds} s"
    );
}

const TIP: &str = "A tip is a sentence the loading screen shows while the world loads, and it is \
                   long enough that it cannot fit on one line of the column it is set in.";

fn panel_overlay(alpha: f32, content: PanelContent<'_>) -> MenuScene<'_> {
    MenuScene::Overlay {
        alpha,
        loading_seconds: None,
        panel: Some(content),
    }
}

fn tip_overlay(alpha: f32, tip: &str) -> MenuScene<'_> {
    panel_overlay(alpha, PanelContent::placeholder(Some(tip)))
}

/// The pixels of the panel's HUD green, over the black the overlay leaves.
fn green_pixels(pixels: &Pixels) -> Vec<(u32, u32)> {
    pixels.positions_where(|[red, green, blue]| green > 60 && red < 40 && blue < 40)
}

fn extent_of(positions: &[(u32, u32)]) -> ((u32, u32), (u32, u32)) {
    let xs = positions.iter().map(|&(x, _)| x);
    let ys = positions.iter().map(|&(_, y)| y);
    (
        (xs.clone().min().unwrap(), xs.max().unwrap()),
        (ys.clone().min().unwrap(), ys.max().unwrap()),
    )
}

/// The panel's text items for `SIZE`, to say where its pieces belong on the screen.
fn panel_text(device: &wgpu::Device, queue: &wgpu::Queue, tip: Option<&str>) -> PanelText {
    let renderer = renderer(device, queue, HUD_GREEN);
    let fit = renderer.layout().fit(SIZE.0, SIZE.1);
    renderer.layout().loading_panel.text(
        &fit,
        SAFE_ZONE,
        &HUD_GREEN,
        &PanelContent::placeholder(tip),
    )
}

/// The framebuffer position of the top of the first line of `item`'s field.
fn line_top_px(item: &TextItem) -> f32 {
    item.map.origin.1 + item.geometry.line_top() * item.map.scale
}

fn content_left_px(item: &TextItem) -> f32 {
    item.map.origin.0 + item.geometry.content_left() * item.map.scale
}

/// The four sides of the outline the tip's box draws, found as the long straight runs of green
/// below row `from_row`.
struct BoxEdges {
    left: u32,
    right: u32,
    top: u32,
    bottom: u32,
}

impl BoxEdges {
    fn find(pixels: &Pixels, from_row: u32) -> Self {
        let lit = |x: u32, y: u32| {
            let [red, green, blue] = pixels.at(x, y);
            green > 60 && red < 40 && blue < 40
        };
        let long_runs = |counts: Vec<(u32, u32)>| -> Vec<u32> {
            let longest = counts.iter().map(|&(_, count)| count).max().unwrap_or(0);
            counts
                .into_iter()
                .filter(|&(_, count)| longest > 0 && count * 10 >= longest * 8)
                .map(|(at, _)| at)
                .collect()
        };
        let rows = long_runs(
            (from_row..SIZE.1)
                .map(|y| (y, (0..SIZE.0).filter(|&x| lit(x, y)).count() as u32))
                .collect(),
        );
        let columns = long_runs(
            (0..SIZE.0)
                .map(|x| (x, (from_row..SIZE.1).filter(|&y| lit(x, y)).count() as u32))
                .collect(),
        );
        let (Some(&top), Some(&bottom), Some(&left), Some(&right)) =
            (rows.first(), rows.last(), columns.first(), columns.last())
        else {
            panic!("no box outline below row {from_row}");
        };
        Self {
            left,
            right,
            top,
            bottom,
        }
    }

    /// The green pixels strictly inside the box.
    fn ink(&self, pixels: &Pixels) -> Vec<(u32, u32)> {
        green_pixels(pixels)
            .into_iter()
            .filter(|&(x, y)| x > self.left && x < self.right && y > self.top && y < self.bottom)
            .collect()
    }
}

#[test]
fn the_panel_puts_the_level_and_the_catalogue_number_where_the_layout_does() {
    let Some((device, queue)) = test_device("menu panel placement") else {
        return;
    };
    let target = Target::new(&device);
    let text = panel_text(&device, &queue, None);

    target.render(
        &device,
        &queue,
        panel_overlay(1.0, PanelContent::placeholder(None)),
    );
    let pixels = target.pixels(&device, &queue);

    for (name, item) in [
        ("level", text.level.as_ref().unwrap()),
        ("catalogue number", text.catalogue.as_ref().unwrap()),
    ] {
        let top = line_top_px(item);
        let band: Vec<_> = green_pixels(&pixels)
            .into_iter()
            .filter(|&(_, y)| {
                (y as f32) >= top - 1.0 && (y as f32) < top + item.geometry.line_pitch()
            })
            .collect();
        assert!(!band.is_empty(), "the {name} is drawn at y {top}");
        let ((left, _), (first, _)) = extent_of(&band);
        assert!(
            (left as f32 - content_left_px(item)).abs() <= 3.0,
            "the {name} starts at x {left}, not {}",
            content_left_px(item)
        );
        assert!(
            (first as f32 - top).abs() <= 6.0,
            "the {name} starts at y {first}, not {top}"
        );
    }
}

#[test]
fn the_meter_is_a_frame_with_a_fill_from_the_left_up_to_the_experience() {
    let Some((device, queue)) = test_device("menu panel meter") else {
        return;
    };
    let target = Target::new(&device);
    let content = |experience: f32| PanelContent {
        experience,
        ..PanelContent::placeholder(None)
    };
    // The fixture's meter covers the stage rectangle (142.55, 513.05) to (361.55, 524.05), the
    // pixels (71.3, 256.5) to (180.8, 262).
    let (left, right) = (72, 180);
    let inside_y = 259;
    let quarter = (left + (right - left) / 4, inside_y);
    let three_quarters = (right - (right - left) / 4, inside_y);

    target.render(&device, &queue, panel_overlay(1.0, content(0.5)));
    let half = target.pixels(&device, &queue);
    target.render(&device, &queue, panel_overlay(1.0, content(0.0)));
    let empty = target.pixels(&device, &queue);

    assert_eq!(
        half.at(quarter.0, quarter.1),
        [0, 255, 0],
        "filled up to half"
    );
    assert_eq!(
        half.at(three_quarters.0, three_quarters.1),
        [0, 0, 0],
        "empty beyond"
    );
    assert_eq!(
        empty.at(quarter.0, quarter.1),
        [0, 0, 0],
        "no fill at no experience"
    );
    let frame = empty.at(71, inside_y);
    assert!(
        frame[1] > 100 && frame[0] == 0,
        "the frame stays: {frame:?}"
    );
}

#[test]
fn the_tip_wraps_inside_its_field_and_a_box_surrounds_the_lines_it_wrapped_into() {
    let Some((device, queue)) = test_device("menu tip box") else {
        return;
    };
    let target = Target::new(&device);
    let tip = panel_text(&device, &queue, Some(TIP)).tip.expect("a tip");
    let top = line_top_px(&tip);
    let column_left = content_left_px(&tip);
    let column_right = column_left + tip.wrap_width.expect("a tip wraps");
    let from_row = top as u32 - 4;

    target.render(&device, &queue, tip_overlay(1.0, "Hxg"));
    let one = target.pixels(&device, &queue);
    let one_box = BoxEdges::find(&one, from_row);
    let (_, (one_top, one_bottom)) = extent_of(&one_box.ink(&one));
    let one_line = one_bottom - one_top + 1;
    target.render(&device, &queue, tip_overlay(1.0, TIP));
    let wrapped = target.pixels(&device, &queue);
    let edges = BoxEdges::find(&wrapped, from_row);
    let ink = edges.ink(&wrapped);
    let ((ink_left, ink_right), (ink_top, ink_bottom)) = extent_of(&ink);

    assert!(
        ink_bottom - ink_top + 1 > one_line * 3 / 2,
        "the tip spans several lines: {one_line} px for one, {} for all",
        ink_bottom - ink_top + 1
    );
    assert!(
        ink_left as f32 >= column_left - 2.0 && ink_right as f32 <= column_right + 1.0,
        "the text spans x {ink_left}..={ink_right} in the field {column_left}..{column_right}"
    );
    assert!(
        edges.bottom - edges.top > one_box.bottom - one_box.top,
        "the box grows with the lines"
    );
    // The padding is 8, 3, 5 and 10 stage pixels, half as many here, around the lines.
    assert!((2..=8).contains(&(ink_left - edges.left)), "left");
    assert!(edges.right - ink_right >= 1, "right");
    assert!(ink_top - edges.top >= 1, "top");
    assert!(edges.bottom - ink_bottom >= 4, "bottom");
}

#[test]
fn a_tip_too_long_for_its_field_stops_after_the_last_whole_line_that_fits() {
    let Some((device, queue)) = test_device("menu tip clipping") else {
        return;
    };
    let target = Target::new(&device);
    let tip = panel_text(&device, &queue, Some(TIP)).tip.expect("a tip");
    let field_bottom = tip.map.origin.1
        + (tip.geometry.line_top() + tip.geometry.content_height()) * tip.map.scale;
    let endless = TIP.repeat(8);

    target.render(&device, &queue, tip_overlay(1.0, &endless));
    let pixels = target.pixels(&device, &queue);
    let edges = BoxEdges::find(&pixels, line_top_px(&tip) as u32 - 4);
    let (_, (top, bottom)) = extent_of(&edges.ink(&pixels));

    assert!(
        bottom as f32 <= field_bottom,
        "the tip from y {top} reaches {bottom}, past the field's bottom {field_bottom}"
    );
    assert!(bottom - top > 20, "several lines still show");
}

#[test]
fn a_panel_shows_in_full_strength_and_not_at_all_on_a_transparent_overlay() {
    let Some((device, queue)) = test_device("menu panel strength") else {
        return;
    };
    let target = Target::new(&device);
    let brightest = |alpha: f32| {
        target.render(&device, &queue, tip_overlay(alpha, TIP));
        let pixels = target.pixels(&device, &queue);
        pixels
            .positions_where(|rgb| rgb != [0, 0, 0])
            .iter()
            .map(|&(x, y)| pixels.at(x, y)[1])
            .max()
    };

    assert_eq!(brightest(0.0), None, "a transparent overlay shows no panel");
    let full = brightest(1.0).expect("the panel shows");
    assert!(full >= 250, "full green up to antialiasing: {full}");
}

#[test]
fn an_overlay_without_a_panel_draws_none() {
    let Some((device, queue)) = test_device("menu overlay without panel") else {
        return;
    };
    let target = Target::new(&device);
    target.render(&device, &queue, overlay(1.0, None));
    let lit = green_pixels(&target.pixels(&device, &queue));
    assert_eq!(lit.len(), 0, "{lit:?}");
}

#[test]
fn the_loading_art_sits_above_the_backdrop_and_below_the_logo() {
    let Some((device, queue)) = test_device("menu art between backdrop and foreground") else {
        return;
    };
    let target = Target::new(&device);
    target.fill(&device, &queue, RED);
    let mut menu = renderer(&device, &queue, HUD_GREEN);
    let mut art_renderer = LoadingArtRenderer::new(&device, &queue, ui_gamma_format(FORMAT));
    let dir = tempfile::tempdir().unwrap();
    let mut art = cube_art(dir.path(), "");
    // The cube stands over the logo's corner of the screen.
    art.transform.position = [218.0, 300.0, -122.0];
    let cpu = CpuArt::load(&art).expect("the synthetic cube loads");
    art_renderer.set_art(Some(
        LoadedArt::upload(&device, &queue, cpu).expect("the cube uploads"),
    ));

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let view = menu.target_view(&target.texture);
    let backdrop = Backdrop {
        alpha: 1.0,
        image: None,
    };
    menu.render_backdrop(&device, &queue, &mut encoder, &view, SIZE, backdrop);
    art_renderer.render(&mut encoder, &view, SIZE);
    let foreground = OverlayForeground {
        alpha: 1.0,
        loading_seconds: Some(0.0),
        panel: None,
    };
    menu.render_foreground(&device, &queue, &mut encoder, &view, SIZE, foreground);
    queue.submit(std::iter::once(encoder.finish()));
    let pixels = target.pixels(&device, &queue);

    assert_eq!(pixels.at(2, 2), [0, 0, 0], "the backdrop hid the red frame");
    assert_eq!(
        pixels
            .at(LOGO_LEFT_HALF.0 - 8, LOGO_LEFT_HALF.1 + 8)
            .map(|c| c > 100),
        [true; 3],
        "the cube is drawn over the backdrop"
    );
    assert_eq!(
        pixels.at(LOGO_LEFT_HALF.0, LOGO_LEFT_HALF.1),
        [0, 255, 0],
        "the logo is drawn over the cube"
    );
}

const BACKGROUND_COLOUR: [u8; 3] = [200, 100, 50];

/// A background image of one colour that the install's folder layout names.
fn background_in(dir: &std::path::Path) -> LoadingBackground {
    write_solid_texture(
        dir,
        "Interface/LoadingMenuBG.DDS",
        [
            BACKGROUND_COLOUR[0],
            BACKGROUND_COLOUR[1],
            BACKGROUND_COLOUR[2],
            255,
        ],
    );
    LoadingBackground::new(dir.to_path_buf())
}

fn corners_and_centre(pixels: &Pixels) -> [[u8; 3]; 5] {
    let (width, height) = SIZE;
    [
        pixels.at(0, 0),
        pixels.at(width - 1, 0),
        pixels.at(0, height - 1),
        pixels.at(width - 1, height - 1),
        pixels.at(width / 2, height / 2),
    ]
}

/// What the shader writes for `background` and `overlay` texels of the given bytes.
fn graded_bytes(background: [u8; 3], overlay: [u8; 4]) -> [u8; 3] {
    let unit = |byte: u8| f32::from(byte) / 255.0;
    let out = graded(background.map(unit), overlay.map(unit));
    out.map(|channel| (channel * 255.0).round().clamp(0.0, 255.0) as u8)
}

/// The background of [`background_in`] as drawn without an overlay.
fn graded_background() -> [u8; 3] {
    graded_bytes(BACKGROUND_COLOUR, [0; 4])
}

fn assert_close(actual: [u8; 3], expected: [u8; 3], context: &str) {
    for (a, e) in actual.into_iter().zip(expected) {
        assert!(
            a.abs_diff(e) <= 2,
            "{context}: {actual:?} is not {expected:?}"
        );
    }
}

#[test]
fn the_right_edge_of_the_target_shows_the_image_left_of_its_padding() {
    let Some((device, queue)) = test_device("menu backdrop padding") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let [red, green, blue] = BACKGROUND_COLOUR;
    write_two_band_texture(
        dir.path(),
        "Interface/LoadingMenuBG.DDS",
        [red, green, blue, 255],
        [0, 255, 0, 255],
        background_uv([1.0, 0.5])[0],
    );
    let mut background = LoadingBackground::new(dir.path().to_path_buf());
    let image = background.images(&device, &queue).expect("the image loads");
    let target = Target::new(&device);
    let mut menu = renderer(&device, &queue, HUD_GREEN);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let view = menu.target_view(&target.texture);
    let backdrop = Backdrop {
        alpha: 1.0,
        image: Some(image),
    };
    menu.render_backdrop(&device, &queue, &mut encoder, &view, SIZE, backdrop);
    queue.submit(std::iter::once(encoder.finish()));

    let pixels = target.pixels(&device, &queue);
    let (width, height) = SIZE;
    for x in [0, width / 2, width - 1] {
        assert_close(
            pixels.at(x, height / 2),
            graded_background(),
            "every column shows the left band, the padding never shows",
        );
    }
}

#[test]
fn a_backdrop_image_covers_the_whole_target_graded() {
    let Some((device, queue)) = test_device("menu backdrop image") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut background = background_in(dir.path());
    let image = background.images(&device, &queue).expect("the image loads");
    let target = Target::new(&device);
    target.fill(&device, &queue, RED);
    let mut menu = renderer(&device, &queue, HUD_GREEN);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let view = menu.target_view(&target.texture);
    let backdrop = Backdrop {
        alpha: 1.0,
        image: Some(image),
    };
    menu.render_backdrop(&device, &queue, &mut encoder, &view, SIZE, backdrop);
    queue.submit(std::iter::once(encoder.finish()));

    for pixel in corners_and_centre(&target.pixels(&device, &queue)) {
        assert_close(
            pixel,
            graded_background(),
            "the graded image fills the target",
        );
    }
}

#[test]
fn a_backdrop_overlay_is_added_over_the_image() {
    let Some((device, queue)) = test_device("menu backdrop overlay") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let overlay = [90, 160, 255, 128];
    write_solid_texture(
        dir.path(),
        "Interface/Objects/LoadScreen/Overlay01_d.DDS",
        overlay,
    );
    let mut background = background_in(dir.path());
    let images = background.images(&device, &queue).expect("the images load");
    assert!(images.overlay.is_some());
    let target = Target::new(&device);
    target.fill(&device, &queue, RED);
    let mut menu = renderer(&device, &queue, HUD_GREEN);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let view = menu.target_view(&target.texture);
    let backdrop = Backdrop {
        alpha: 1.0,
        image: Some(images),
    };
    menu.render_backdrop(&device, &queue, &mut encoder, &view, SIZE, backdrop);
    queue.submit(std::iter::once(encoder.finish()));

    let expected = graded_bytes(BACKGROUND_COLOUR, overlay);
    assert_ne!(
        expected,
        graded_background(),
        "the overlay changes the colour"
    );
    for pixel in corners_and_centre(&target.pixels(&device, &queue)) {
        assert_close(pixel, expected, "the overlay is added to the image");
    }
}

#[test]
fn a_backdrop_image_fades_in_with_the_overlay() {
    let Some((device, queue)) = test_device("menu backdrop image opacity") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut background = background_in(dir.path());
    let image = background.images(&device, &queue).expect("the image loads");
    let target = Target::new(&device);
    let mut menu = renderer(&device, &queue, HUD_GREEN);
    let draw_at = |menu: &mut MenuRenderer, alpha: f32| {
        target.fill(&device, &queue, RED);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let view = menu.target_view(&target.texture);
        let backdrop = Backdrop {
            alpha,
            image: Some(image),
        };
        menu.render_backdrop(&device, &queue, &mut encoder, &view, SIZE, backdrop);
        queue.submit(std::iter::once(encoder.finish()));
        target.pixels(&device, &queue).at(SIZE.0 / 2, SIZE.1 / 2)
    };

    let half = draw_at(&mut menu, 0.5);
    let over = draw_at(&mut menu, 7.0);

    let [_, green, blue] = graded_background();
    assert_close(
        half,
        [half[0], green / 2, blue / 2],
        "half of the image over the red frame, which has no green or blue",
    );
    assert_close(
        over,
        graded_background(),
        "an opacity above 1 is fully opaque",
    );
}

#[test]
fn a_backdrop_without_an_image_is_only_the_black_layer() {
    let Some((device, queue)) = test_device("menu backdrop without image") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut background = background_in(dir.path());
    let image = background.images(&device, &queue).expect("the image loads");
    let target = Target::new(&device);
    let mut menu = renderer(&device, &queue, HUD_GREEN);
    let draw = |menu: &mut MenuRenderer, image: Option<BackgroundImages<'_>>| {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let view = menu.target_view(&target.texture);
        let backdrop = Backdrop { alpha: 1.0, image };
        menu.render_backdrop(&device, &queue, &mut encoder, &view, SIZE, backdrop);
        queue.submit(std::iter::once(encoder.finish()));
    };

    draw(&mut menu, Some(image));
    draw(&mut menu, None);

    for pixel in corners_and_centre(&target.pixels(&device, &queue)) {
        assert_eq!(pixel, [0, 0, 0], "the image of an earlier frame is gone");
    }
}

#[test]
fn the_loading_art_is_drawn_over_the_backdrop_image() {
    let Some((device, queue)) = test_device("menu art over backdrop image") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut background = background_in(dir.path());
    let image = background.images(&device, &queue).expect("the image loads");
    let target = Target::new(&device);
    let mut menu = renderer(&device, &queue, HUD_GREEN);
    let mut art_renderer = LoadingArtRenderer::new(&device, &queue, ui_gamma_format(FORMAT));
    let cpu = CpuArt::load(&cube_art(dir.path(), "")).expect("the synthetic cube loads");
    art_renderer.set_art(Some(
        LoadedArt::upload(&device, &queue, cpu).expect("the cube uploads"),
    ));

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let view = menu.target_view(&target.texture);
    let backdrop = Backdrop {
        alpha: 1.0,
        image: Some(image),
    };
    menu.render_backdrop(&device, &queue, &mut encoder, &view, SIZE, backdrop);
    art_renderer.render(&mut encoder, &view, SIZE);
    queue.submit(std::iter::once(encoder.finish()));
    let pixels = target.pixels(&device, &queue);

    assert_close(
        pixels.at(0, 0),
        graded_background(),
        "the corner shows the image",
    );
    let centre = pixels.at(SIZE.0 / 2, SIZE.1 / 2);
    assert!(
        centre
            .iter()
            .zip(graded_background())
            .any(|(a, b)| a.abs_diff(b) > 20),
        "the cube hides the image at the centre: {centre:?}"
    );
}
