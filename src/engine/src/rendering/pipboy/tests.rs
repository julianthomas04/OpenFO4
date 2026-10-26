//! Shell tests: every screen paints with an empty and a populated model, input routes through the
//! navigation state machine, and a frame reaches pixels on a real device.

use std::sync::Arc;

use egui::{Pos2, Vec2};

use super::fonts::PipBoyFonts;
use super::input::ShellInput;
use super::nav::{Page, Screen, SubTab, TabId};
use super::shapes::OutlinedShape;
use super::tab::{HintKey, NavInput};
use super::{PipBoyScreen, stage::StageFit};
use crate::assets::layout::{INTERFACE_DIR, PIPBOY_ART_MANIFEST_FILE, PIPBOY_DIR};
use crate::assets::pipboy_art::{ArtLibrary, FixedArt};
use crate::rendering::ui_colour::{UiColourSet, UiColours, UiTint};
use crate::sim::pipboy::inventory::{
    ApparelStats, ComponentAmount, DamageValue, InventoryCategory, InventoryItem, ItemDetails,
    ItemId, WeaponStats,
};
use crate::sim::pipboy::perks::PerkRank;
use crate::sim::pipboy::{
    DamageType, PerkDefinition, PerkId, PipBoyAction, PipBoyCatalog, PipBoyModel, PipBoySnapshot,
    PlayerVitals,
};

const WINDOW: Vec2 = Vec2::new(1920.0, 1080.0);
const TINY_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 10 10"><rect width="10" height="10" fill="white"/></svg>"#;

fn art_fixture(dir: &std::path::Path) -> ArtLibrary {
    let root = dir.join(INTERFACE_DIR).join(PIPBOY_DIR);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("frame.svg"), TINY_SVG).unwrap();
    let clips: Vec<_> = FixedArt::all()
        .map(|art| {
            serde_json::json!({
                "id": art.clip_id(), "source": "Test.swf", "colour": "mask", "fps": 30.0,
                "script": [], "bounds": [-5.0, -5.0, 5.0, 5.0], "frames": ["frame.svg"],
            })
        })
        .collect();
    let manifest =
        serde_json::json!({ "format": crate::assets::pipboy_art::MANIFEST_FORMAT, "clips": clips });
    std::fs::write(root.join(PIPBOY_ART_MANIFEST_FILE), manifest.to_string()).unwrap();
    ArtLibrary::load(dir).unwrap()
}

fn fonts() -> PipBoyFonts {
    let defaults = egui::FontDefinitions::default();
    let bytes = defaults.font_data["Ubuntu-Light"].font.to_vec();
    PipBoyFonts::new(bytes.clone(), bytes)
}

fn colours() -> UiColours {
    UiColours::new(
        UiTint {
            color: [0.9, 0.2, 0.3],
            opacity: 1.0,
        },
        UiTint {
            color: [0.2, 0.9, 0.3],
            opacity: 1.0,
        },
    )
}

fn screen(dir: &std::path::Path) -> PipBoyScreen {
    PipBoyScreen::new(
        fonts(),
        Arc::new(art_fixture(dir)),
        crate::rendering::pipboy::test_layout::pipboy_layout(),
        &colours(),
        crate::rendering::game_settings::SafeZone { x: 64.0, y: 36.0 },
    )
    .expect("the test layout resolves")
}

fn item(name: &str, category: InventoryCategory, details: ItemDetails) -> InventoryItem {
    InventoryItem {
        id: ItemId(0),
        form_key: None,
        name: name.to_owned(),
        category,
        count: 2,
        weight: 3.5,
        value: 40,
        equipped: false,
        favourite: None,
        legendary: false,
        details,
    }
}

fn populated_model() -> PipBoyModel {
    let perk = PerkDefinition {
        id: PerkId("p".into()),
        name: "Iron Fist".into(),
        ranks: (1..=3)
            .map(|n| PerkRank {
                form_key: format!("p{n}"),
                description: "Punches hit harder and the description wraps over lines.".into(),
                required_level: n,
            })
            .collect(),
        vault_boy_swf: None,
        chart: None,
        sex: None,
    };
    let mut model = PipBoyModel::new(Arc::new(PipBoyCatalog {
        perks: vec![perk],
        ..PipBoyCatalog::default()
    }));
    model.perks.0.insert(PerkId("p".into()), 2);
    model.character.name = "Nate".into();
    model.character.progression.unspent_perk_points = 1;
    let damage = |kind, amount| DamageValue { kind, amount };
    let mut weapon = item(
        "10mm Pistol",
        InventoryCategory::Weapons,
        ItemDetails::Weapon(WeaponStats {
            damage: vec![
                damage(DamageType::Physical, 18.0),
                damage(DamageType::Energy, 4.0),
            ],
            fire_rate: 3.0,
            range: 120.0,
            accuracy: 70.0,
            ammo_name: Some("10mm Round".into()),
        }),
    );
    weapon.equipped = true;
    weapon.legendary = true;
    model.inventory.insert(weapon);
    model.inventory.insert(item(
        "Vault Suit",
        InventoryCategory::Apparel,
        ItemDetails::Apparel(ApparelStats {
            resistances: vec![damage(DamageType::Physical, 5.0)],
            covers: vec![
                crate::sim::pipboy::Limb::Torso,
                crate::sim::pipboy::Limb::Head,
            ],
            slot_mask: 1,
        }),
    ));
    model.inventory.insert(item(
        "Stimpak",
        InventoryCategory::Aid,
        ItemDetails::Aid(Vec::new()),
    ));
    model.inventory.insert(item(
        "Wrench",
        InventoryCategory::Junk,
        ItemDetails::Junk(vec![ComponentAmount {
            component: "Steel".into(),
            count: 2,
        }]),
    ));
    model
        .inventory
        .insert(item("Teddy", InventoryCategory::Misc, ItemDetails::Misc));
    model
        .inventory
        .insert(item("Barrel", InventoryCategory::Mods, ItemDetails::Mod));
    model
        .inventory
        .insert(item("10mm", InventoryCategory::Ammo, ItemDetails::Ammo));
    model
}

fn vitals() -> PlayerVitals {
    PlayerVitals {
        hp: 80.0,
        hp_max: 100.0,
        ap: 0.0,
        ap_max: 0.0,
        position: None,
        heading_yaw: 0.0,
    }
}

/// Paints without a renderer: the texture deltas have nowhere to go, so they are discarded.
fn paint(
    screen: &mut PipBoyScreen,
    snapshot: &PipBoySnapshot<'_>,
    seconds: f64,
) -> super::PaintedFrame {
    let mut frame = screen.paint_frame(WINDOW, snapshot, seconds);
    frame.output.textures_delta.clear();
    frame
}

fn all_screens() -> Vec<Screen> {
    let mut screens = vec![
        Screen::PerkChart,
        Screen::Shell(TabId::Map),
        Screen::Shell(TabId::Radio),
    ];
    for page in [Page::Stat, Page::Inv, Page::Data] {
        screens.extend(
            page.sub_tabs()
                .iter()
                .map(|sub| Screen::Shell(TabId::Sub(*sub))),
        );
    }
    screens
}

#[test]
fn every_screen_paints_something_with_an_empty_and_a_populated_model() {
    let dir = tempfile::tempdir().unwrap();
    let mut screen = screen(dir.path());
    for model in [PipBoyModel::new(Arc::default()), populated_model()] {
        let snapshot = PipBoySnapshot {
            model: &model,
            clock: crate::sim::WorldClock::fixture().date(),
            vitals: vitals(),
        };
        for target in all_screens() {
            screen.show(target, Some(1));
            let frame = paint(&mut screen, &snapshot, 0.5);
            if matches!(target, Screen::Shell(_)) {
                assert!(
                    !frame.output.shapes.is_empty(),
                    "{target:?} painted nothing"
                );
            }
        }
    }
}

#[test]
fn the_painted_frame_is_laid_out_for_the_stage_of_the_screen() {
    let dir = tempfile::tempdir().unwrap();
    let mut screen = screen(dir.path());
    let model = PipBoyModel::new(Arc::default());
    let snapshot = PipBoySnapshot {
        model: &model,
        clock: crate::sim::WorldClock::fixture().date(),
        vitals: vitals(),
    };

    let shell = paint(&mut screen, &snapshot, 0.0);
    screen.show(Screen::PerkChart, None);
    let chart = paint(&mut screen, &snapshot, 0.0);

    for (frame, screen_shown) in [
        (&shell, Screen::Shell(TabId::Map)),
        (&chart, Screen::PerkChart),
    ] {
        let (stage, fraction) = screen.stages.of(screen_shown);
        let expected = StageFit::new(stage, WINDOW, fraction);
        assert!((frame.fit.scale - expected.scale).abs() < 1e-4);
    }
    assert!((shell.fit.scale - chart.fit.scale).abs() > 1e-4);
}

#[test]
fn closing_from_a_page_asks_the_sim_to_close_and_from_the_perk_chart_returns_to_the_shell() {
    let dir = tempfile::tempdir().unwrap();
    let mut screen = screen(dir.path());
    let model = populated_model();
    let snapshot = PipBoySnapshot {
        model: &model,
        clock: crate::sim::WorldClock::fixture().date(),
        vitals: vitals(),
    };

    screen.show(Screen::PerkChart, None);
    screen.dispatch(ShellInput::Close, &snapshot);
    assert_eq!(screen.take_actions(), []);
    assert!(matches!(screen.nav().screen(), Screen::Shell(_)));

    screen.dispatch(ShellInput::Close, &snapshot);
    assert_eq!(screen.take_actions(), vec![PipBoyAction::Close]);
}

#[test]
fn page_and_sub_tab_keys_move_through_the_navigation_and_stop_at_the_ends() {
    let dir = tempfile::tempdir().unwrap();
    let mut screen = screen(dir.path());
    let model = populated_model();
    let snapshot = PipBoySnapshot {
        model: &model,
        clock: crate::sim::WorldClock::fixture().date(),
        vitals: vitals(),
    };

    screen.dispatch(ShellInput::NextPage, &snapshot);
    assert_eq!(screen.nav().page(), Page::Inv);
    screen.dispatch(ShellInput::Nav(NavInput::NextSubTab), &snapshot);
    assert_eq!(screen.nav().tab(), TabId::Sub(SubTab::Apparel));
    screen.dispatch(ShellInput::PrevPage, &snapshot);
    screen.dispatch(ShellInput::PrevPage, &snapshot);
    assert_eq!(screen.nav().page(), Page::Stat);
}

fn text_colours(frame: &super::PaintedFrame) -> Vec<egui::Color32> {
    let mut colours = Vec::new();
    for clipped in &frame.output.shapes {
        if let egui::Shape::Text(text) = &clipped.shape {
            colours.extend(
                text.galley
                    .job
                    .sections
                    .iter()
                    .map(|section| section.format.color),
            );
        }
    }
    colours
}

/// Whether `colour` is `tint` scaled by one positive fraction (to the 8-bit rounding), so black
/// and any other hue fail.
fn is_shade_of(colour: egui::Color32, tint: egui::Color32) -> bool {
    let channels = [
        (colour.r(), tint.r()),
        (colour.g(), tint.g()),
        (colour.b(), tint.b()),
    ];
    let Some(&(strongest, full)) = channels.iter().max_by_key(|(_, full)| *full) else {
        return false;
    };
    if strongest == 0 || full == 0 {
        return false;
    }
    let fraction = f32::from(strongest) / f32::from(full);
    channels
        .iter()
        .all(|&(value, full)| (f32::from(value) - f32::from(full) * fraction).abs() <= 1.5)
}

fn filled_colours(frame: &super::PaintedFrame) -> Vec<egui::Color32> {
    fn collect(shape: &egui::Shape, out: &mut Vec<egui::Color32>) {
        match shape {
            egui::Shape::Rect(rect) => out.push(rect.fill),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
            _ => {}
        }
    }
    let mut colours = Vec::new();
    for clipped in &frame.output.shapes {
        collect(&clipped.shape, &mut colours);
    }
    colours
}

#[test]
fn the_perk_chart_uses_the_hud_colour_and_the_pages_the_pipboy_colour() {
    let colours = colours();
    let hud = colours.tint_for(UiColourSet::Hud).srgb_color32();
    let pipboy = colours.tint_for(UiColourSet::Pipboy).srgb_color32();
    let dir = tempfile::tempdir().unwrap();
    let mut screen = screen(dir.path());
    let model = populated_model();
    let snapshot = PipBoySnapshot {
        model: &model,
        clock: crate::sim::WorldClock::fixture().date(),
        vitals: vitals(),
    };

    screen.show(Screen::PerkChart, None);
    let chart_frame = paint(&mut screen, &snapshot, 0.5);
    let chart = filled_colours(&chart_frame);
    let chart_text = text_colours(&chart_frame);
    screen.show(Screen::Shell(TabId::Sub(SubTab::Perks)), None);
    let shell = filled_colours(&paint(&mut screen, &snapshot, 0.5));

    assert!(
        chart.contains(&hud),
        "the level box is drawn in the HUD colour"
    );
    assert!(
        !chart_text.is_empty() && chart_text.iter().all(|colour| is_shade_of(*colour, hud)),
        "every chart text is a visible shade of the HUD colour"
    );
    assert!(
        !chart.contains(&pipboy),
        "nothing on the chart is Pip-Boy coloured"
    );
    assert!(
        shell.contains(&pipboy),
        "the page is drawn in the Pip-Boy colour"
    );
}

#[test]
fn the_perk_chart_key_on_perks_opens_the_chart() {
    let dir = tempfile::tempdir().unwrap();
    let mut screen = screen(dir.path());
    let model = populated_model();
    let snapshot = PipBoySnapshot {
        model: &model,
        clock: crate::sim::WorldClock::fixture().date(),
        vitals: vitals(),
    };

    screen.show(Screen::Shell(TabId::Sub(SubTab::Perks)), None);
    screen.dispatch(ShellInput::Nav(NavInput::Hint(HintKey::T)), &snapshot);

    assert_eq!(screen.nav().screen(), Screen::PerkChart);
}

#[test]
fn inventory_keys_emit_the_matching_actions() {
    let dir = tempfile::tempdir().unwrap();
    let mut screen = screen(dir.path());
    let model = populated_model();
    let snapshot = PipBoySnapshot {
        model: &model,
        clock: crate::sim::WorldClock::fixture().date(),
        vitals: vitals(),
    };
    let weapon = model.inventory.listed(InventoryCategory::Weapons)[0].id;

    screen.show(Screen::Shell(TabId::Sub(SubTab::Weapons)), None);
    screen.dispatch(ShellInput::Nav(NavInput::Hint(HintKey::E)), &snapshot);
    screen.dispatch(ShellInput::Nav(NavInput::Hint(HintKey::R)), &snapshot);
    screen.dispatch(ShellInput::Nav(NavInput::Hint(HintKey::Tab)), &snapshot);
    screen.dispatch(ShellInput::Nav(NavInput::Hint(HintKey::C)), &snapshot);

    let actions = screen.take_actions();
    assert_eq!(actions[0], PipBoyAction::Unequip(weapon));
    assert!(matches!(
        actions.last(),
        Some(PipBoyAction::SetSortMode {
            category: InventoryCategory::Weapons,
            ..
        })
    ));
    assert!(!actions.iter().any(|a| matches!(a, PipBoyAction::Close)));
}

/// A point inside the INV title of the test layout.
fn inv_title() -> Pos2 {
    let x = crate::rendering::pipboy::test_layout::TITLE_PITCH as f32 + 50.0;
    Pos2::new(x, 10.0)
}

#[test]
fn a_click_on_a_page_title_selects_the_page() {
    let dir = tempfile::tempdir().unwrap();
    let mut screen = screen(dir.path());
    let model = populated_model();
    let snapshot = PipBoySnapshot {
        model: &model,
        clock: crate::sim::WorldClock::fixture().date(),
        vitals: vitals(),
    };
    paint(&mut screen, &snapshot, 0.0);
    let before = screen.nav().sub_index();

    // The page title row is hit-tested before the sub-tab strip: STAT is already selected.
    screen.dispatch(ShellInput::Nav(NavInput::Clicked(inv_title())), &snapshot);

    assert_eq!(before, 0);
    assert_eq!(screen.nav().page(), Page::Inv);
}

#[test]
fn a_notify_request_shows_on_the_message_line_and_fades_after_three_seconds() {
    let dir = tempfile::tempdir().unwrap();
    let mut screen = screen(dir.path());
    let model = populated_model();
    let snapshot = PipBoySnapshot {
        model: &model,
        clock: crate::sim::WorldClock::fixture().date(),
        vitals: vitals(),
    };

    screen
        .requests
        .push(super::tab::ShellRequest::Notify("Nothing to show".into()));
    screen.apply_requests();
    assert_eq!(screen.notice.text(), Some("Nothing to show"));

    paint(&mut screen, &snapshot, 1.0);
    assert_eq!(screen.notice.text(), Some("Nothing to show"));
    paint(&mut screen, &snapshot, 3.5);
    assert_eq!(screen.notice.text(), None);
}

#[test]
fn a_new_sim_rejection_is_shown_as_the_games_text_once_per_change() {
    let dir = tempfile::tempdir().unwrap();
    let mut screen = screen(dir.path());
    let mut model = populated_model();
    model.last_rejection = Some(crate::sim::pipboy::PipBoyActionError::PerkSpecialTooLow {
        perk: crate::sim::pipboy::PerkId("perk".to_owned()),
    });
    let snapshot = PipBoySnapshot {
        model: &model,
        clock: crate::sim::WorldClock::fixture().date(),
        vitals: vitals(),
    };

    paint(&mut screen, &snapshot, 0.0);
    assert_eq!(screen.notice.text(), Some("Notice refused"));

    screen.notice.clear();
    paint(&mut screen, &snapshot, 0.1);
    assert_eq!(screen.notice.text(), None);
}

#[test]
fn a_rejection_the_game_has_no_text_for_is_not_shown_to_the_player() {
    let dir = tempfile::tempdir().unwrap();
    let mut screen = screen(dir.path());
    let mut model = populated_model();
    model.last_rejection = Some(crate::sim::pipboy::PipBoyActionError::UnknownItem(ItemId(
        9,
    )));
    let snapshot = PipBoySnapshot {
        model: &model,
        clock: crate::sim::WorldClock::fixture().date(),
        vitals: vitals(),
    };

    paint(&mut screen, &snapshot, 0.0);
    assert_eq!(screen.notice.text(), None);
}

#[test]
fn page_and_chrome_input_is_swallowed_while_the_tab_is_modal() {
    let dir = tempfile::tempdir().unwrap();
    let mut screen = screen(dir.path());
    let model = populated_model();
    let snapshot = PipBoySnapshot {
        model: &model,
        clock: crate::sim::WorldClock::fixture().date(),
        vitals: vitals(),
    };
    screen.show(Screen::Shell(TabId::Sub(SubTab::Weapons)), None);
    paint(&mut screen, &snapshot, 0.0);
    screen.dispatch(ShellInput::Nav(NavInput::Hint(HintKey::Q)), &snapshot);
    assert!(screen.active_tab_is_modal());

    screen.dispatch(ShellInput::NextPage, &snapshot);
    screen.dispatch(ShellInput::PrevPage, &snapshot);
    screen.dispatch(ShellInput::Nav(NavInput::Clicked(inv_title())), &snapshot);
    screen.dispatch(ShellInput::Nav(NavInput::NextSubTab), &snapshot);

    assert_eq!(screen.nav().tab(), TabId::Sub(SubTab::Weapons));
    assert!(screen.active_tab_is_modal());

    screen.dispatch(ShellInput::Close, &snapshot);
    assert!(!screen.active_tab_is_modal());
    assert_eq!(screen.take_actions(), []);
}

#[test]
fn opening_the_pip_boy_drops_a_prompt_left_open() {
    let dir = tempfile::tempdir().unwrap();
    let mut screen = screen(dir.path());
    let model = populated_model();
    let snapshot = PipBoySnapshot {
        model: &model,
        clock: crate::sim::WorldClock::fixture().date(),
        vitals: vitals(),
    };
    screen.show(Screen::Shell(TabId::Sub(SubTab::Weapons)), None);
    screen.dispatch(ShellInput::Nav(NavInput::Hint(HintKey::Q)), &snapshot);
    assert!(screen.active_tab_is_modal());

    screen.reset_session();

    assert!(!screen.active_tab_is_modal());
}

#[test]
fn the_cursor_seen_while_closed_hovers_and_clicks_on_the_first_open_frame() {
    let dir = tempfile::tempdir().unwrap();
    let mut screen = screen(dir.path());
    let model = populated_model();
    let snapshot = PipBoySnapshot {
        model: &model,
        clock: crate::sim::WorldClock::fixture().date(),
        vitals: vitals(),
    };
    let (stage, fraction) = screen.stages.of(Screen::Shell(TabId::Map));
    let fit = StageFit::new(stage, WINDOW, fraction);
    let title = inv_title();
    let window = fit.origin_px + title.to_vec2() * fit.scale;
    let moved = winit::event::WindowEvent::CursorMoved {
        device_id: winit::event::DeviceId::dummy(),
        position: winit::dpi::PhysicalPosition::new(f64::from(window.x), f64::from(window.y)),
    };

    screen.observe_window_event(&moved);
    screen.reset_session();
    paint(&mut screen, &snapshot, 0.0);
    let press = winit::event::WindowEvent::MouseInput {
        device_id: winit::event::DeviceId::dummy(),
        state: winit::event::ElementState::Pressed,
        button: winit::event::MouseButton::Left,
    };
    screen.handle_window_event(&press, &snapshot);

    assert_eq!(screen.nav().page(), Page::Inv);
}

#[test]
fn rank_keys_on_a_perk_without_rank_data_do_not_panic() {
    let dir = tempfile::tempdir().unwrap();
    let mut screen = screen(dir.path());
    let mut model = populated_model();
    let rankless = PerkDefinition {
        id: PerkId("empty".into()),
        name: "Rankless".into(),
        ranks: Vec::new(),
        vault_boy_swf: None,
        chart: None,
        sex: None,
    };
    let mut catalog = (*model.catalog).clone();
    catalog.perks = vec![rankless];
    model.catalog = Arc::new(catalog);
    model.perks.0.clear();
    model.perks.0.insert(PerkId("empty".into()), 1);
    let snapshot = PipBoySnapshot {
        model: &model,
        clock: crate::sim::WorldClock::fixture().date(),
        vitals: vitals(),
    };

    screen.show(Screen::Shell(TabId::Sub(SubTab::Perks)), None);
    screen.dispatch(ShellInput::Nav(NavInput::Hint(HintKey::Ctrl)), &snapshot);
    screen.dispatch(ShellInput::Nav(NavInput::Hint(HintKey::Alt)), &snapshot);
    paint(&mut screen, &snapshot, 0.0);
}

#[test]
fn stimpak_and_radaway_keys_do_nothing_without_the_item() {
    let dir = tempfile::tempdir().unwrap();
    let mut screen = screen(dir.path());
    let model = PipBoyModel::new(Arc::default());
    let snapshot = PipBoySnapshot {
        model: &model,
        clock: crate::sim::WorldClock::fixture().date(),
        vitals: vitals(),
    };

    screen.show(Screen::Shell(TabId::Sub(SubTab::Status)), None);
    screen.dispatch(ShellInput::Nav(NavInput::Hint(HintKey::E)), &snapshot);
    screen.dispatch(ShellInput::Nav(NavInput::Hint(HintKey::R)), &snapshot);

    assert_eq!(screen.take_actions(), []);
}

fn read_pixels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    size: [u32; 2],
) -> Vec<u8> {
    let padded = (size[0] * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pipboy readback"),
        size: u64::from(padded * size[1]),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(size[1]),
            },
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            sender
                .send(result)
                .expect("the test outlives its map callback");
        });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("readback poll");
    receiver.recv().unwrap().expect("readback map");
    let data = buffer
        .slice(..)
        .get_mapped_range()
        .expect("mapped")
        .to_vec();
    buffer.unmap();
    let row_bytes = (size[0] * 4) as usize;
    (0..size[1] as usize)
        .flat_map(|row| data[row * padded as usize..][..row_bytes].to_vec())
        .collect()
}

#[test]
fn a_frame_reaches_the_target_inside_the_stage_and_leaves_the_bars_untouched() {
    use crate::rendering::egui_pass::{EguiPass, EguiTarget};
    use crate::rendering::gpu_profiler::{GpuPass, GpuProfiler};

    let Some((device, queue)) = crate::rendering::gpu_test::test_device("pipboy frame") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let mut screen = screen(dir.path());
    let model = populated_model();
    let snapshot = PipBoySnapshot {
        model: &model,
        clock: crate::sim::WorldClock::fixture().date(),
        vitals: vitals(),
    };
    let size = [1920_u32, 1080];
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pipboy target"),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut pass = EguiPass::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    let mut profiler = GpuProfiler::new(&device, &queue);

    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    screen.show(Screen::Shell(TabId::Sub(SubTab::Weapons)), None);
    profiler.begin_frame();
    let frame = screen.paint_frame(Vec2::new(size[0] as f32, size[1] as f32), &snapshot, 0.5);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    let target = EguiTarget {
        label: "pipboy test",
        view: &view,
        size_in_pixels: size,
        pixels_per_point: frame.fit.scale,
        profiled_as: GpuPass::Hud,
    };
    pass.draw(
        &device,
        &queue,
        &mut encoder,
        &profiler,
        &target,
        &screen.ctx,
        frame.output,
    );
    queue.submit(std::iter::once(encoder.finish()));
    let error = pollster::block_on(scope.pop());
    assert!(error.is_none(), "{error:?}");

    let pixels = read_pixels(&device, &queue, &texture, size);
    let lit = |x0: u32, x1: u32, y0: u32, y1: u32| {
        (y0..y1).any(|y| {
            (x0..x1).any(|x| {
                let at = ((y * size[0] + x) * 4) as usize;
                pixels[at..at + 3].iter().any(|channel| *channel != 0)
            })
        })
    };
    let (stage, fraction) = screen.stages.of(Screen::Shell(TabId::Map));
    let fit = StageFit::new(stage, Vec2::new(size[0] as f32, size[1] as f32), fraction);
    let header_x = (fit.origin_px.x + 33.0 * fit.scale) as u32;
    let header_end = (fit.origin_px.x + 862.0 * fit.scale) as u32;
    assert!(lit(
        header_x,
        header_end,
        (fit.origin_px.y + 15.0 * fit.scale) as u32,
        (fit.origin_px.y + 60.0 * fit.scale) as u32
    ));
    assert!(
        !lit(0, fit.origin_px.x as u32 - 2, 0, size[1] / 2),
        "the left bar holds nothing above the screen-space hint bar"
    );

    let stage_pixel = |x: f32, y: f32| {
        let px = (fit.origin_px.x + x * fit.scale) as u32;
        let py = (fit.origin_px.y + y * fit.scale) as u32;
        let at = ((py * size[0] + px) * 4) as usize;
        [pixels[at], pixels[at + 1], pixels[at + 2]]
    };
    let installed = crate::rendering::pipboy::test_layout::pipboy_layout();
    let list = &crate::rendering::pipboy::pages::inv::LIST;
    let origin = installed.get(&list.list).unwrap().world.matrix;
    let border = installed.get(&list.border).unwrap().local.matrix;
    let bar = OutlinedShape::resolve(&installed, &list.entry_border)
        .unwrap()
        .bounds()
        .translate(Vec2::new(origin.tx, origin.ty + border.ty));
    let selection_bar = stage_pixel(bar.max.x - 1.0, bar.max.y - 1.0);
    assert!(
        selection_bar[1] > 150,
        "the first list row is selected: {selection_bar:?}"
    );
    assert_eq!(
        stage_pixel(bar.min.x - 5.0, bar.center().y),
        [0, 0, 0],
        "nothing lies left of the list bar"
    );
}

#[test]
fn the_pipboys_declared_layout_keys_resolve_against_a_complete_layout() {
    let layout = crate::rendering::pipboy::test_layout::pipboy_layout();
    layout.validate(&[&super::LAYOUT_KEYS]).unwrap();
}

#[test]
fn a_layout_missing_the_hp_text_aborts_with_its_key() {
    let layout = crate::ui_layout::test_support::layout_with_movie(
        "PipboyMenu",
        &crate::ui_layout::test_support::set_text_movie(),
    );
    let error = layout.validate(&[&super::LAYOUT_KEYS]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Menu_mc/BottomBar_mc/Info_mc:StatsPage/HP_tf")
    );
}
