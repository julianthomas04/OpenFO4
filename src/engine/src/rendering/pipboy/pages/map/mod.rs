//! MAP: the world map and the local map (the same image, zoomed around the player), its markers,
//! the player arrow, a player-placed marker and the fast-travel request.

mod click;
mod layout;
mod markers;
mod stand_ins;
mod tooltip;
mod view;

use std::collections::HashSet;

use egui::{Pos2, Rect, Stroke, Vec2};
use glam::Vec3;

use self::click::ClickTracker;
use self::markers::{arrow_rotation, engine_to_map, map_to_engine};
use self::stand_ins::{
    FOCUS_RING_RADIUS, FOCUS_RING_SECONDS, FOCUS_RING_STROKE, KEY_PAN_STEP, KEY_ZOOM_DELTA,
    NO_MAP_DATA_TEXT, NO_MAP_IMAGE_TEXT, NO_MAP_LOCATION_NOTICE, NOTICE_TEXT_SIZE, ZOOM_IN_SLOT,
    ZOOM_OUT_SLOT, fast_travel_notice, notice_field,
};
use self::view::{MapMode, MapView};
use super::glyph::{draw_clip_registered, draw_sprite_rotated};
use crate::assets::pipboy_art::{FixedArt, Icon, marker_clip_id};
use crate::rendering::pipboy::paint::{self, TextField, TextStyle, text_field};
use crate::rendering::pipboy::tab::{
    BottomBar, ButtonHint, HintKey, InputResult, NavInput, PointerButton, ShellRequest, Tab,
    TabContext,
};
use crate::sim::pipboy::{MapMarker, PipBoyAction, PipBoyModel, PipBoySnapshot, WorldMap};

pub use self::layout::{LAYOUT_KEYS, MapLayout};
pub use self::markers::{PULSE_KEYS, PlayerPulse};

const MODE_ENV: &str = "OPENFO4_PIPBOY_MAP";
const FULL_UV: Rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));

pub fn parse_mode(text: &str) -> Option<MapMode> {
    match text.trim().to_ascii_lowercase().as_str() {
        "world" => Some(MapMode::World),
        "local" => Some(MapMode::Local),
        _ => None,
    }
}

/// The ring that marks the place a "show on map" request centred on; it goes away on the next
/// pan, zoom or click, or after [`FOCUS_RING_SECONDS`].
#[derive(Clone, Copy, Debug, PartialEq)]
struct FocusRing {
    position: Vec3,
    since: f64,
}

/// What the pointer is over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hover {
    Marker(usize),
    Custom,
}

pub struct MapPage {
    layout: MapLayout,
    pulse: PlayerPulse,
    view: MapView,
    pointer: Option<Pos2>,
    clicks: ClickTracker,
    hovered: Option<Hover>,
    pending_show: bool,
    pending_focus: Option<Vec3>,
    focus: Option<FocusRing>,
    startup_applied: bool,
    texture_missing_reported: bool,
    marker_types_without_art: HashSet<u16>,
}

impl MapPage {
    pub fn new(layout: MapLayout, pulse: PlayerPulse) -> Self {
        let view = MapView::new(layout.world, layout.local, layout.rules);
        Self {
            layout,
            pulse,
            view,
            pointer: None,
            clicks: ClickTracker::default(),
            hovered: None,
            pending_show: true,
            pending_focus: None,
            focus: None,
            startup_applied: false,
            texture_missing_reported: false,
            marker_types_without_art: HashSet::new(),
        }
    }

    /// Centres the map on an engine-space position (DATA "show on map").
    pub fn focus_on(&mut self, position: Vec3) {
        self.pending_focus = Some(position);
    }

    fn map_position(&self, map: &WorldMap, position: Vec3) -> Vec2 {
        engine_to_map(&map.projection, self.view.texture(), position)
    }

    /// Shows `mode` the way the menu does: its zoom restarts and the map centres on the player.
    fn show(&mut self, mode: MapMode, map: &WorldMap, snapshot: &PipBoySnapshot<'_>) {
        self.view.show(mode);
        if let Some(position) = snapshot.vitals.position {
            let at = self.map_position(map, position);
            self.view.centre_on(at);
        }
        self.focus = None;
    }

    fn apply_startup_mode(&mut self, map: &WorldMap, snapshot: &PipBoySnapshot<'_>) {
        if std::mem::replace(&mut self.startup_applied, true) {
            return;
        }
        let Some(text) = crate::utils::optional_env_var(MODE_ENV) else {
            return;
        };
        if let Some(mode) = parse_mode(&text) {
            self.show(mode, map, snapshot);
        } else {
            tracing::error!("{MODE_ENV}: '{text}' is neither 'world' nor 'local'");
        }
    }

    fn pan(&mut self, delta: Vec2) {
        self.view.move_map(delta);
        self.focus = None;
    }

    fn zoom(&mut self, delta: f32) {
        self.view.wheel(delta, self.clicks.moving());
        self.focus = None;
    }

    /// The position the custom marker is drawn and picked at: held inside the border.
    fn custom_marker_map(&self, map: &WorldMap, position: Vec3) -> Vec2 {
        self.view.clamped(self.map_position(map, position))
    }

    /// The discovered marker or the placed marker nearest to `at` inside the pick square.
    fn pick(&self, map: &WorldMap, model: &PipBoyModel, at: Pos2) -> Option<Hover> {
        let custom_index = map.markers.len();
        let markers = map
            .markers
            .iter()
            .enumerate()
            .filter(|(_, marker)| model.is_marker_discovered(marker))
            .map(|(index, marker)| (index, self.map_position(map, marker.position)));
        let custom = model
            .custom_marker
            .map(|position| (custom_index, self.custom_marker_map(map, position)));
        self.view.nearest(at, markers.chain(custom)).map(|index| {
            if index == custom_index {
                Hover::Custom
            } else {
                Hover::Marker(index)
            }
        })
    }

    fn on_click(&mut self, at: Pos2, cx: &mut TabContext<'_>) {
        self.focus = None;
        let snapshot = *cx.snapshot;
        let Some(map) = snapshot.model.catalog.world_map.as_ref() else {
            return;
        };
        if !self.view.window().contains(at) {
            return;
        }
        match self.pick(map, snapshot.model, at) {
            Some(Hover::Marker(index)) => {
                let marker = &map.markers[index];
                if marker.can_fast_travel {
                    cx.actions
                        .push(PipBoyAction::RequestFastTravel(marker.id.clone()));
                    cx.requests.push(ShellRequest::Notify(fast_travel_notice(
                        &self.layout.labels.fast_travel,
                        &marker.name,
                    )));
                }
            }
            Some(Hover::Custom) => cx.actions.push(PipBoyAction::RemoveCustomMarker),
            None => {
                let height = snapshot.vitals.position.map_or(0.0, |position| position.y);
                let target = map_to_engine(
                    &map.projection,
                    self.view.texture(),
                    self.view.stage_to_marker(at),
                    height,
                );
                cx.actions.push(PipBoyAction::PlaceCustomMarker(target));
            }
        }
    }

    fn update_hover(&mut self, map: &WorldMap, model: &PipBoyModel) {
        let window = self.view.window();
        self.hovered = self
            .pointer
            .filter(|at| window.contains(*at))
            .and_then(|at| self.pick(map, model, at));
    }

    fn apply_pending_show(&mut self, map: &WorldMap, snapshot: &PipBoySnapshot<'_>) {
        if std::mem::replace(&mut self.pending_show, false) {
            self.show(self.view.mode(), map, snapshot);
        }
    }

    fn apply_pending_focus(&mut self, map: &WorldMap, cx: &TabContext<'_>) {
        if let Some(position) = self.pending_focus.take() {
            let at = self.map_position(map, position);
            self.view.centre_on(at);
            self.focus = Some(FocusRing {
                position,
                since: cx.seconds_open,
            });
        }
    }
}

impl Tab for MapPage {
    fn handle_input(&mut self, input: NavInput, cx: &mut TabContext<'_>) -> InputResult {
        let snapshot = *cx.snapshot;
        if let Some(image) = cx.world_map_texture {
            self.view.set_texture(image.size);
        }
        match input {
            NavInput::Hint(HintKey::R) => {
                if let Some(map) = snapshot.model.catalog.world_map.as_ref() {
                    self.show(self.view.mode().toggled(), map, &snapshot);
                }
            }
            NavInput::Scrolled(delta) => self.zoom(delta),
            NavInput::Slot(ZOOM_IN_SLOT) => self.zoom(KEY_ZOOM_DELTA),
            NavInput::Slot(ZOOM_OUT_SLOT) => self.zoom(-KEY_ZOOM_DELTA),
            NavInput::Up => self.pan(Vec2::new(0.0, KEY_PAN_STEP)),
            NavInput::Down => self.pan(Vec2::new(0.0, -KEY_PAN_STEP)),
            NavInput::PrevSubTab => self.pan(Vec2::new(KEY_PAN_STEP, 0.0)),
            NavInput::NextSubTab => self.pan(Vec2::new(-KEY_PAN_STEP, 0.0)),
            NavInput::PointerMoved(at) => self.pointer = Some(at),
            NavInput::Clicked(at) => {
                self.pointer = Some(at);
                self.focus = None;
                self.clicks.press(at, self.view.window().contains(at));
            }
            NavInput::Dragged {
                delta,
                button: PointerButton::Primary,
            } => {
                if self.clicks.drag(delta) {
                    self.view.drag(delta);
                    self.focus = None;
                }
            }
            NavInput::Released(at) => {
                self.pointer = Some(at);
                if let Some(click) = self.clicks.release() {
                    self.on_click(click, cx);
                }
            }
            _ => return InputResult::Ignored,
        }
        InputResult::Consumed
    }

    fn draw(&mut self, cx: &mut TabContext<'_>) {
        let snapshot = *cx.snapshot;
        let white = paint::shade(cx.tint, 1.0);
        let Some(map) = snapshot.model.catalog.world_map.as_ref() else {
            if self.pending_focus.take().is_some() {
                cx.requests
                    .push(ShellRequest::Notify(NO_MAP_LOCATION_NOTICE.to_owned()));
            }
            let painter = cx.painter.with_clip_rect(self.view.window());
            self.draw_notice(&painter, white, NO_MAP_DATA_TEXT);
            return;
        };
        let painter = cx.painter.with_clip_rect(self.view.window());
        let Some(image) = cx.world_map_texture else {
            if !std::mem::replace(&mut self.texture_missing_reported, true) {
                tracing::error!(
                    "the world map texture was not registered before the map page drew"
                );
            }
            self.draw_notice(&painter, white, NO_MAP_IMAGE_TEXT);
            return;
        };
        self.view.set_texture(image.size);
        self.apply_startup_mode(map, &snapshot);
        self.apply_pending_show(map, &snapshot);
        self.apply_pending_focus(map, cx);
        self.expire_focus(cx.seconds_open);
        self.update_hover(map, snapshot.model);

        let painter = cx.painter.with_clip_rect(self.view.window());
        painter.image(image.id, self.view.texture_rect(), FULL_UV, white);
        self.draw_focus_ring(cx, &painter, map);
        for (index, marker) in map.markers.iter().enumerate() {
            if snapshot.model.is_marker_discovered(marker)
                && self.hovered != Some(Hover::Marker(index))
            {
                self.draw_marker(cx, &painter, map, marker, false);
            }
        }
        self.draw_custom_marker(cx, &painter, map, &snapshot);
        self.draw_player(cx, &painter, map, &snapshot);
        if let Some(Hover::Marker(index)) = self.hovered {
            let marker = &map.markers[index];
            let anchor = self.draw_marker(cx, &painter, map, marker, true);
            tooltip::draw(cx, &self.layout.tooltip, marker, anchor);
        }
    }

    fn hints(&self, snapshot: &PipBoySnapshot<'_>) -> Vec<ButtonHint> {
        let map = snapshot.model.catalog.world_map.as_ref();
        let labels = &self.layout.labels;
        let target = match self.view.mode() {
            MapMode::World => &labels.local_map,
            MapMode::Local => &labels.world_map,
        };
        let pointer_hint = match self.hovered {
            Some(Hover::Marker(index)) => map
                .and_then(|map| map.markers.get(index))
                .filter(|marker| marker.can_fast_travel)
                .map(|_| ButtonHint::new(HintKey::Mouse1, labels.fast_travel.clone())),
            Some(Hover::Custom) => Some(ButtonHint::new(
                HintKey::Mouse1,
                labels.remove_marker.clone(),
            )),
            None => map.map(|_| ButtonHint::new(HintKey::Mouse1, labels.place_marker.clone())),
        };
        pointer_hint
            .into_iter()
            .chain([ButtonHint::new(HintKey::R, target.clone()).enabled(map.is_some())])
            .collect()
    }

    fn bottom_bar(&self) -> BottomBar {
        BottomBar::DateTime {
            with_location: true,
        }
    }

    fn on_open(&mut self) {
        self.clicks.cancel();
        self.pointer = None;
        self.hovered = None;
        self.pending_focus = None;
        self.pending_show = true;
        self.focus = None;
    }
}

impl MapPage {
    fn draw_notice(&self, painter: &egui::Painter, colour: egui::Color32, text: &str) {
        let (origin, width) = notice_field(self.view.window().center());
        text_field(
            painter,
            TextField::centre(origin, width),
            text,
            TextStyle::bold(NOTICE_TEXT_SIZE, colour),
        );
    }

    fn expire_focus(&mut self, seconds: f64) {
        if self
            .focus
            .is_some_and(|ring| seconds - ring.since >= FOCUS_RING_SECONDS)
        {
            self.focus = None;
        }
    }

    /// Draws `marker` and returns the point its tooltip hangs from: the marker's bottom.
    fn draw_marker(
        &mut self,
        cx: &mut TabContext<'_>,
        painter: &egui::Painter,
        map: &WorldMap,
        marker: &MapMarker,
        selected: bool,
    ) -> Pos2 {
        let at = self
            .view
            .marker_to_stage(self.map_position(map, marker.position));
        let scale = self.view.marker_scale(selected);
        let colour = paint::shade(cx.tint, 1.0);
        let library = cx.art_library;
        let Some(clip) = library.clip(&marker_clip_id(marker.marker_type)) else {
            if self.marker_types_without_art.insert(marker.marker_type) {
                tracing::error!(
                    "no Pip-Boy art for map marker type {}; the art manifest was not checked against the catalog at load",
                    marker.marker_type
                );
            }
            return at;
        };
        let height = clip.bounds[3] - clip.bounds[1];
        let extent = clip
            .bounds
            .iter()
            .fold(0.0_f32, |far, edge| far.max(edge.abs()))
            * scale;
        if self.view.window().expand(extent).contains(at) {
            draw_clip_registered(cx, painter, clip, at, scale, colour);
        }
        at + Vec2::new(0.0, height * scale)
    }

    fn draw_custom_marker(
        &self,
        cx: &mut TabContext<'_>,
        painter: &egui::Painter,
        map: &WorldMap,
        snapshot: &PipBoySnapshot<'_>,
    ) {
        let Some(position) = snapshot.model.custom_marker else {
            return;
        };
        let at = self
            .view
            .marker_to_stage(self.custom_marker_map(map, position));
        let selected = self.hovered == Some(Hover::Custom);
        let scale = self.view.marker_scale(selected);
        let colour = paint::shade(cx.tint, 1.0);
        let library = cx.art_library;
        let clip = library.fixed(FixedArt::Icon(Icon::CustomMarker));
        draw_clip_registered(cx, painter, clip, at, scale, colour);
    }

    fn draw_focus_ring(&self, cx: &TabContext<'_>, painter: &egui::Painter, map: &WorldMap) {
        if let Some(ring) = self.focus {
            let at = self
                .view
                .marker_to_stage(self.map_position(map, ring.position));
            let alpha = self.pulse.alpha(cx.seconds_open);
            let colour = paint::faded(paint::shade(cx.tint, 1.0), alpha);
            painter.circle_stroke(
                at,
                FOCUS_RING_RADIUS,
                Stroke::new(FOCUS_RING_STROKE, colour),
            );
        }
    }

    fn draw_player(
        &self,
        cx: &mut TabContext<'_>,
        painter: &egui::Painter,
        map: &WorldMap,
        snapshot: &PipBoySnapshot<'_>,
    ) {
        let Some(position) = snapshot.vitals.position else {
            return;
        };
        let at = self.view.marker_to_stage(self.map_position(map, position));
        let colour = paint::faded(
            paint::shade(cx.tint, 1.0),
            self.pulse.alpha(cx.seconds_open),
        );
        let sprite = cx.art.fixed_scaled(
            cx.painter.ctx(),
            cx.art_library,
            FixedArt::Icon(Icon::PlayerArrow),
            0,
            self.view.marker_scale(false),
        );
        if let Some(sprite) = sprite {
            draw_sprite_rotated(
                painter,
                sprite,
                at,
                arrow_rotation(snapshot.vitals.heading_yaw),
                colour,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::layout::HintLabels;
    use super::view::tests::{TEXTURE, params, rules};
    use super::*;
    use crate::rendering::pipboy::input::{InputTranslator, ShellInput};
    use crate::rendering::pipboy::pages::test_harness::Harness;
    use crate::rendering::pipboy::stage::StageFit;
    use crate::rendering::pipboy::test_layout::pipboy_layout;
    use crate::rendering::pipboy::world_map_texture::WorldMapImage;
    use crate::sim::pipboy::{MapMarkerId, PipBoyCatalog, WorldMapProjection};
    use winit::dpi::PhysicalPosition;
    use winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};

    const PROJECTION: WorldMapProjection = WorldMapProjection {
        origin_u: 0.5,
        origin_v: 0.5,
        u_per_engine_x: 1.0e-3,
        v_per_engine_z: 1.0e-3,
    };

    fn pulse() -> PlayerPulse {
        PlayerPulse::resolve(&pipboy_layout()).unwrap()
    }

    fn layout() -> MapLayout {
        MapLayout {
            world: params(),
            local: params(),
            rules: rules(),
            tooltip: super::tooltip::TooltipLayout::resolve(&pipboy_layout()).unwrap(),
            labels: HintLabels {
                fast_travel: "travel".to_owned(),
                place_marker: "place".to_owned(),
                remove_marker: "remove".to_owned(),
                world_map: "world".to_owned(),
                local_map: "local".to_owned(),
            },
        }
    }

    fn marker(id: &str, position: Vec3, discovered: bool, can_fast_travel: bool) -> MapMarker {
        MapMarker {
            id: MapMarkerId(id.to_owned()),
            name: id.to_owned(),
            marker_type: 1,
            flags: 0,
            position,
            discovered,
            can_fast_travel,
        }
    }

    /// Positions that land well inside the synthetic texture, away from each other.
    fn near() -> Vec3 {
        Vec3::new(-20.0, 0.0, -10.0)
    }

    fn far() -> Vec3 {
        Vec3::new(60.0, 0.0, 40.0)
    }

    /// `near` discovered and travellable, `far` undiscovered.
    fn model_with_markers() -> PipBoyModel {
        let catalog = PipBoyCatalog {
            world_map: Some(WorldMap {
                worldspace: "ws".to_owned(),
                texture_key: "textures/map.dds".to_owned(),
                projection: PROJECTION,
                markers: vec![
                    marker("near", near(), true, true),
                    marker("far", far(), false, true),
                ],
            }),
            ..PipBoyCatalog::default()
        };
        let mut model = PipBoyModel::new(Arc::new(catalog));
        model
            .discovered_markers
            .remove(&MapMarkerId("far".to_owned()));
        model
    }

    fn harness() -> Harness {
        let mut harness = Harness::new();
        harness.world_map = Some(WorldMapImage {
            id: egui::TextureId::default(),
            size: TEXTURE,
        });
        harness
    }

    /// A page that has drawn once, so the texture size is known and the view is placed.
    fn ready(harness: &mut Harness, model: &PipBoyModel) -> MapPage {
        let mut page = MapPage::new(layout(), pulse());
        harness.draw(&mut page, model);
        page
    }

    fn stage_of(page: &MapPage, position: Vec3) -> Pos2 {
        let map = engine_to_map(&PROJECTION, TEXTURE, position);
        page.view.marker_to_stage(map)
    }

    fn click(harness: &mut Harness, page: &mut MapPage, model: &PipBoyModel, at: Pos2) {
        harness.send(page, model, NavInput::Clicked(at));
        harness.send(page, model, NavInput::Released(at));
    }

    /// A spot inside the window, away from every marker.
    fn open_ground(page: &MapPage) -> Pos2 {
        let window = page.view.window();
        window.min + Vec2::new(5.0, 5.0)
    }

    #[test]
    fn the_startup_mode_accepts_world_and_local_in_any_case() {
        assert_eq!(parse_mode("world"), Some(MapMode::World));
        assert_eq!(parse_mode(" Local "), Some(MapMode::Local));
        assert_eq!(parse_mode("regional"), None);
    }

    #[test]
    fn a_click_on_a_discovered_marker_requests_fast_travel_and_says_so() {
        let model = model_with_markers();
        let mut harness = harness();
        let mut page = ready(&mut harness, &model);

        let at = stage_of(&page, near());
        click(&mut harness, &mut page, &model, at);

        assert_eq!(
            harness.take_actions(),
            vec![PipBoyAction::RequestFastTravel(MapMarkerId(
                "near".to_owned()
            ))]
        );
        assert_eq!(
            harness.take_requests(),
            vec![ShellRequest::Notify("travel: near".to_owned())]
        );
    }

    #[test]
    fn a_click_on_an_undiscovered_marker_places_a_marker_instead() {
        let model = model_with_markers();
        let mut harness = harness();
        let mut page = ready(&mut harness, &model);

        let at = stage_of(&page, far());
        click(&mut harness, &mut page, &model, at);

        assert!(matches!(
            harness.take_actions().as_slice(),
            [PipBoyAction::PlaceCustomMarker(_)]
        ));
    }

    #[test]
    fn a_click_on_open_ground_places_the_marker_where_the_pointer_is() {
        let model = model_with_markers();
        let mut harness = harness();
        let mut page = ready(&mut harness, &model);
        let at = open_ground(&page);

        click(&mut harness, &mut page, &model, at);

        let actions = harness.take_actions();
        let [PipBoyAction::PlaceCustomMarker(position)] = actions.as_slice() else {
            panic!("expected one placement, got {actions:?}");
        };
        assert!(stage_of(&page, *position).distance(at) < 0.5);
    }

    #[test]
    fn a_click_on_the_placed_marker_removes_it() {
        let mut model = model_with_markers();
        let mut harness = harness();
        let mut page = ready(&mut harness, &model);
        let at = open_ground(&page);
        click(&mut harness, &mut page, &model, at);
        for action in harness.take_actions() {
            model.submit(action);
        }
        let placed = model.custom_marker.expect("the placement stays");
        harness.draw(&mut page, &model);

        let at = stage_of(&page, placed);
        click(&mut harness, &mut page, &model, at);

        assert_eq!(
            harness.take_actions(),
            vec![PipBoyAction::RemoveCustomMarker]
        );
    }

    #[test]
    fn a_press_released_outside_the_window_or_never_pressed_does_nothing() {
        let model = model_with_markers();
        let mut harness = harness();
        let mut page = ready(&mut harness, &model);
        let outside = page.view.window().max + Vec2::splat(30.0);

        click(&mut harness, &mut page, &model, outside);
        let ground = open_ground(&page);
        harness.send(&mut page, &model, NavInput::Released(ground));

        assert_eq!(harness.take_actions(), []);
    }

    #[test]
    fn dragging_pans_the_map_by_the_drag_speed_and_the_release_after_a_drag_is_no_click() {
        let model = model_with_markers();
        let mut harness = harness();
        let mut page = ready(&mut harness, &model);
        let at = open_ground(&page);
        let before = stage_of(&page, near());

        harness.send(&mut page, &model, NavInput::Clicked(at));
        harness.send(
            &mut page,
            &model,
            NavInput::Dragged {
                delta: Vec2::new(-4.0, -3.0),
                button: PointerButton::Primary,
            },
        );
        harness.send(&mut page, &model, NavInput::Released(at));

        let moved = stage_of(&page, near()) - before;
        let speed = page.view.params().drag_speed;
        assert!((moved.x - -4.0 * speed).abs() < 1e-2);
        assert!((moved.y - -3.0 * speed).abs() < 1e-2);
        assert_eq!(harness.take_actions(), []);
    }

    #[test]
    fn the_wheel_zooms_but_not_while_the_map_is_being_dragged() {
        let model = model_with_markers();
        let mut harness = harness();
        let mut page = ready(&mut harness, &model);
        let scale = page.view.scale();
        let at = open_ground(&page);

        harness.send(&mut page, &model, NavInput::Clicked(at));
        harness.send(
            &mut page,
            &model,
            NavInput::Dragged {
                delta: Vec2::new(1.0, 0.0),
                button: PointerButton::Primary,
            },
        );
        harness.send(&mut page, &model, NavInput::Scrolled(1.0));
        assert!((page.view.scale() - scale).abs() < 1e-6);

        harness.send(&mut page, &model, NavInput::Released(at));
        harness.send(&mut page, &model, NavInput::Scrolled(1.0));
        assert!(page.view.scale() > scale);
    }

    #[test]
    fn a_press_left_over_from_a_closed_session_does_not_click_after_reopening() {
        let model = model_with_markers();
        let mut harness = harness();
        let mut page = ready(&mut harness, &model);
        let at = stage_of(&page, near());
        harness.send(&mut page, &model, NavInput::Clicked(at));

        page.on_open();
        harness.send(&mut page, &model, NavInput::Released(at));

        assert_eq!(harness.take_actions(), []);
    }

    #[test]
    fn show_on_map_centres_the_position_and_rings_it_until_the_view_changes_or_time_runs_out() {
        let model = model_with_markers();
        let mut harness = harness();
        let mut page = ready(&mut harness, &model);

        page.focus_on(near());
        harness.draw(&mut page, &model);

        assert!(page.focus.is_some());
        let border = page.view.params().border;
        let centre = stage_of(&page, near());
        assert!((centre.x - border.center().x).abs() < 1e-2);

        harness.send(&mut page, &model, NavInput::Down);
        assert!(page.focus.is_none());

        page.focus_on(near());
        harness.draw(&mut page, &model);
        harness.seconds_open += FOCUS_RING_SECONDS + 1.0;
        harness.draw(&mut page, &model);
        assert!(page.focus.is_none());
    }

    #[test]
    fn show_on_map_without_a_map_tells_the_player_instead_of_silently_dropping_it() {
        let model = PipBoyModel::new(Arc::new(PipBoyCatalog::default()));
        let mut harness = harness();
        let mut page = MapPage::new(layout(), pulse());

        page.focus_on(near());
        harness.draw(&mut page, &model);

        assert_eq!(
            harness.take_requests(),
            vec![ShellRequest::Notify(NO_MAP_LOCATION_NOTICE.to_owned())]
        );
    }

    #[test]
    fn hovering_a_discovered_marker_selects_it_and_an_undiscovered_one_stays_hidden() {
        let model = model_with_markers();
        let mut harness = harness();
        let mut page = ready(&mut harness, &model);

        let at = stage_of(&page, near());
        harness.send(&mut page, &model, NavInput::PointerMoved(at));
        harness.draw(&mut page, &model);
        assert_eq!(page.hovered, Some(Hover::Marker(0)));

        let at = stage_of(&page, far());
        harness.send(&mut page, &model, NavInput::PointerMoved(at));
        harness.draw(&mut page, &model);
        assert_eq!(page.hovered, None);
    }

    #[test]
    fn the_pointer_hint_follows_what_the_pointer_is_over() {
        let mut model = model_with_markers();
        let mut harness = harness();
        let mut page = ready(&mut harness, &model);
        let snapshot_hints = |page: &MapPage, model: &PipBoyModel| {
            let snapshot = PipBoySnapshot {
                model,
                clock: crate::sim::WorldClock::default().date(),
                vitals: crate::rendering::pipboy::pages::test_harness::vitals(),
            };
            page.hints(&snapshot)
        };
        assert_eq!(snapshot_hints(&page, &model)[0].label, "place");

        let at = stage_of(&page, near());
        harness.send(&mut page, &model, NavInput::PointerMoved(at));
        harness.draw(&mut page, &model);
        assert_eq!(snapshot_hints(&page, &model)[0].label, "travel");

        let at = open_ground(&page);
        click(&mut harness, &mut page, &model, at);
        for action in harness.take_actions() {
            model.submit(action);
        }
        harness.draw(&mut page, &model);
        let placed = stage_of(&page, model.custom_marker.unwrap());
        harness.send(&mut page, &model, NavInput::PointerMoved(placed));
        harness.draw(&mut page, &model);
        assert_eq!(snapshot_hints(&page, &model)[0].label, "remove");
    }

    #[test]
    fn a_marker_that_cannot_fast_travel_offers_no_pointer_hint_and_places_nothing() {
        let catalog = PipBoyCatalog {
            world_map: Some(WorldMap {
                worldspace: "ws".to_owned(),
                texture_key: "textures/map.dds".to_owned(),
                projection: PROJECTION,
                markers: vec![marker("door", near(), true, false)],
            }),
            ..PipBoyCatalog::default()
        };
        let model = PipBoyModel::new(Arc::new(catalog));
        let mut harness = harness();
        let mut page = ready(&mut harness, &model);
        let at = stage_of(&page, near());

        harness.send(&mut page, &model, NavInput::PointerMoved(at));
        harness.draw(&mut page, &model);
        let snapshot = PipBoySnapshot {
            model: &model,
            clock: crate::sim::WorldClock::default().date(),
            vitals: crate::rendering::pipboy::pages::test_harness::vitals(),
        };
        assert_eq!(page.hints(&snapshot).len(), 1);
        click(&mut harness, &mut page, &model, at);
        assert_eq!(harness.take_actions(), []);
    }

    #[test]
    fn the_keys_pan_and_zoom_and_r_switches_between_world_and_local() {
        let model = model_with_markers();
        let mut harness = harness();
        let mut page = ready(&mut harness, &model);
        let scale = page.view.scale();
        let before = stage_of(&page, near());

        harness.send(&mut page, &model, NavInput::Down);
        assert!(stage_of(&page, near()).y < before.y);
        harness.send(&mut page, &model, NavInput::Slot(ZOOM_IN_SLOT));
        assert!(page.view.scale() > scale);
        harness.send(&mut page, &model, NavInput::Slot(ZOOM_OUT_SLOT));
        assert!((page.view.scale() - scale).abs() < 1e-5);

        harness.send(&mut page, &model, NavInput::Hint(HintKey::R));
        assert_eq!(page.view.mode(), MapMode::Local);
        harness.send(&mut page, &model, NavInput::Hint(HintKey::R));
        assert_eq!(page.view.mode(), MapMode::World);
    }

    #[test]
    fn switching_modes_centres_the_player_and_restarts_the_zoom() {
        let model = model_with_markers();
        let mut harness = harness();
        harness.vitals.position = Some(near());
        let mut page = ready(&mut harness, &model);
        harness.send(&mut page, &model, NavInput::Scrolled(1.0));

        harness.send(&mut page, &model, NavInput::Hint(HintKey::R));

        let border = page.view.params().border;
        assert!((stage_of(&page, near()).x - border.center().x).abs() < 1e-2);
        assert!((page.view.scale() - 1.5).abs() < 1e-5);
    }

    #[test]
    fn the_translator_and_the_page_turn_a_physical_click_into_one_action_and_a_drag_into_none() {
        let model = model_with_markers();
        let mut harness = harness();
        let mut page = ready(&mut harness, &model);
        let fit = StageFit {
            scale: 1.5,
            origin_px: Pos2::new(100.0, 40.0),
        };
        let mut translator = InputTranslator::default();
        let cursor = |at: Pos2| {
            let px = fit.origin_px + at.to_vec2() * fit.scale;
            WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: PhysicalPosition::new(f64::from(px.x), f64::from(px.y)),
            }
        };
        let button = |state| WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Left,
        };
        let mut feed = |event: &WindowEvent, page: &mut MapPage, harness: &mut Harness| {
            for input in translator.translate(event, fit).inputs {
                if let ShellInput::Nav(nav) = input {
                    harness.send(page, &model, nav);
                }
            }
        };
        let ground = open_ground(&page);

        feed(&cursor(ground), &mut page, &mut harness);
        feed(&button(ElementState::Pressed), &mut page, &mut harness);
        feed(&button(ElementState::Released), &mut page, &mut harness);
        assert!(matches!(
            harness.take_actions().as_slice(),
            [PipBoyAction::PlaceCustomMarker(_)]
        ));

        feed(&button(ElementState::Pressed), &mut page, &mut harness);
        feed(
            &cursor(ground + Vec2::new(40.0, 0.0)),
            &mut page,
            &mut harness,
        );
        feed(&button(ElementState::Released), &mut page, &mut harness);
        assert_eq!(harness.take_actions(), []);
    }

    #[test]
    fn the_page_draws_with_and_without_a_map_in_both_modes() {
        let model = model_with_markers();
        let mut harness = harness();
        let mut page = ready(&mut harness, &model);
        harness.send(&mut page, &model, NavInput::Hint(HintKey::R));
        harness.draw(&mut page, &model);
        harness.draw(
            &mut page,
            &PipBoyModel::new(Arc::new(PipBoyCatalog::default())),
        );
        let mut without_texture = Harness::new();
        without_texture.draw(&mut MapPage::new(layout(), pulse()), &model);
    }

    #[test]
    fn the_declared_layout_resolves_against_a_synthesized_layout() {
        assert!(MapLayout::resolve(&pipboy_layout()).is_ok());
    }
}
