//! RADIO: the station list, the tune action and the live waveform of the selected station.

use egui::{Pos2, Rect, Shape, Stroke, Vec2};

use super::radio_wave::{self, WaveParams, Waveform};
use crate::rendering::pipboy::paint::{self, BLACK, fill};
use crate::rendering::pipboy::scrolling_list::{
    ListEvent, ListKeys, ListLayout, ListState, ListView, Row, brightness, list_keys,
};
use crate::rendering::pipboy::shapes::{OutlinedShape, Shade, rect_of};
use crate::rendering::pipboy::tab::{
    BottomBar, ButtonHint, HintKey, InputResult, NavInput, Tab, TabContext,
};
use crate::sim::pipboy::radio::RadioStation;
use crate::sim::pipboy::{PipBoyAction, PipBoySnapshot, StationId};
use crate::ui_layout::{
    LiteralKey, ModuleKeys, PlacementKey, ShapeKey, TranslateKey, UiLayout, UiLayoutError,
    key_check_fields,
};

const MOVIE: &str = "Pipboy_RadioPage";

pub(crate) static LIST: ListKeys = list_keys!(
    movie: "Pipboy_RadioPage",
    list: "List_mc",
    set_prop: "Pipboy_RadioPage.__setProp_List_mc_Scene1_List_0",
    entry: "RadioListEntry",
    border: "@1",
    arrows: "@1",
);

struct RadioKeys {
    frame: ShapeKey,
    dead_line: ShapeKey,
    wave: PlacementKey,
    equip_icon: ShapeKey,
    entry_fade: LiteralKey,
    tune_label: TranslateKey,
    off_label: TranslateKey,
}

static KEYS: RadioKeys = RadioKeys {
    frame: ShapeKey::new(MOVIE, "@1"),
    dead_line: ShapeKey::new(MOVIE, "DeadWave_mc/@1"),
    wave: PlacementKey::new(MOVIE, "RadioWave_mc"),
    equip_icon: ShapeKey::new(MOVIE, "[RadioListEntry]/EquipIcon_mc/@1"),
    entry_fade: LiteralKey::new(MOVIE, "RadioListEntry.<init>#FadedColor.brightness=", 1),
    tune_label: TranslateKey::text("$TuneToStation"),
    off_label: TranslateKey::text("$RadioOff"),
};

key_check_fields!(RadioKeys {
    frame,
    dead_line,
    wave,
    equip_icon,
    entry_fade,
    tune_label,
    off_label
});

fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    RadioLayout::resolve(layout).map(drop)
}

pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[&LIST, &KEYS, &radio_wave::LAYOUT_KEYS],
    resolve: validate,
};

/// The frequency ruler's ticks along the box's right edge: the box shape's own hairlines carry
/// no geometry in the layout, so their rhythm stays this one stand-in.
struct Ruler;

impl Ruler {
    const PITCH: f32 = 6.1;
    const SHORT: f32 = 4.6;
    const LONG: f32 = 7.4;
    const LONG_EVERY: usize = 4;

    /// Offset from the top and length of each tick for a box `height` tall.
    fn ticks(height: f32) -> Vec<(f32, f32)> {
        let count = (height / Self::PITCH).ceil() as usize;
        (0..count)
            .map(|index| {
                let length = if index % Self::LONG_EVERY == 0 {
                    Self::LONG
                } else {
                    Self::SHORT
                };
                (index as f32 * Self::PITCH, length)
            })
            .collect()
    }
}

struct Frame {
    rect: Rect,
    stroke: f32,
    shade: Shade,
}

pub struct RadioLayout {
    list: ListLayout,
    frame: Frame,
    dead_line: OutlinedShape,
    wave_origin: Pos2,
    wave: WaveParams,
    equip_icon: OutlinedShape,
    fade: f32,
    tune_label: String,
    off_label: String,
}

impl RadioLayout {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let frame = layout.get(&KEYS.frame)?;
        let line = frame.lines.first().ok_or_else(|| {
            UiLayoutError::unresolved(MOVIE, KEYS.frame.path, "the box has no line style")
        })?;
        let colour = line.color.ok_or_else(|| {
            UiLayoutError::unresolved(MOVIE, KEYS.frame.path, "the box's line has no colour")
        })?;
        let wave = layout.get(&KEYS.wave)?.world.matrix;
        Ok(Self {
            list: ListLayout::resolve(layout, &LIST)?,
            frame: Frame {
                rect: rect_of(frame.placement.world.matrix.bounds(frame.edge_bounds)),
                stroke: line.width,
                shade: Shade::of(colour, &frame.placement.world.color),
            },
            dead_line: OutlinedShape::resolve(layout, &KEYS.dead_line)?,
            wave_origin: Pos2::new(wave.tx, wave.ty),
            wave: WaveParams::resolve(layout)?,
            equip_icon: OutlinedShape::resolve(layout, &KEYS.equip_icon)?,
            fade: layout.lit_number(&KEYS.entry_fade, 0)? as f32,
            tune_label: layout.get(&KEYS.tune_label)?,
            off_label: layout.get(&KEYS.off_label)?,
        })
    }
}

/// The stations in list order: the ones in range first, each group by name ignoring case.
fn display_order(stations: &[RadioStation]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..stations.len()).collect();
    order.sort_by_cached_key(|&index| {
        (
            !stations[index].in_range,
            stations[index].name.to_lowercase(),
        )
    });
    order
}

/// What activating a station does: switch the radio off when it is the tuned one, tune to it
/// when it is in range.
fn activation(station: &RadioStation, tuned: Option<StationId>) -> Option<PipBoyAction> {
    if tuned == Some(station.id) {
        Some(PipBoyAction::TurnOffRadio)
    } else {
        station
            .in_range
            .then_some(PipBoyAction::TuneRadio(station.id))
    }
}

/// What the wave was last generated for: the selected station and whether it plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TraceSubject {
    station: StationId,
    playing: bool,
}

pub struct RadioPage {
    layout: RadioLayout,
    list: ListState,
    wave: Option<(Waveform, TraceSubject)>,
    sessions: u64,
}

const WAVEFORM_SEED: u64 = 0x05EE_D0F4;
const SEED_STRIDE: u64 = 0x9E37_79B9_7F4A_7C15;

impl RadioPage {
    pub fn new(layout: RadioLayout) -> Self {
        Self {
            layout,
            list: ListState::default(),
            wave: None,
            sessions: 0,
        }
    }

    fn selected<'a>(
        &self,
        stations: &'a [RadioStation],
        order: &[usize],
    ) -> Option<&'a RadioStation> {
        self.list
            .selected()
            .and_then(|position| order.get(position))
            .map(|&index| &stations[index])
    }

    fn draw_frame(&self, cx: &TabContext<'_>) {
        let frame = &self.layout.frame;
        let white = frame.shade.colour(cx.tint);
        paint::outline(cx.painter, frame.rect, frame.stroke, white);
        let inner = frame.rect.shrink(frame.stroke);
        for (offset, length) in Ruler::ticks(inner.height()) {
            fill(
                cx.painter,
                Rect::from_min_size(
                    Pos2::new(inner.max.x - length, inner.min.y + offset),
                    Vec2::new(length, 1.0),
                ),
                white,
            );
        }
    }

    /// Brings the wave to the selected station: a new station or a change of the tuned state
    /// queues a fresh trace of the station's frequency, a not playing station keeps the quiet
    /// frequency.
    fn tune_wave(&mut self, station: &RadioStation, tuned: Option<StationId>) -> &Waveform {
        let params = self.layout.wave;
        let subject = TraceSubject {
            station: station.id,
            playing: tuned == Some(station.id),
        };
        let seed = WAVEFORM_SEED ^ self.sessions.wrapping_mul(SEED_STRIDE);
        let (wave, shown) = self
            .wave
            .get_or_insert_with(|| (Waveform::new(params, seed), subject));
        let first = wave.points().len() == 0;
        if first || *shown != subject {
            *shown = subject;
            wave.init(Self::cycles(&params, station, subject.playing));
        }
        wave
    }

    fn cycles(params: &WaveParams, station: &RadioStation, playing: bool) -> f32 {
        if playing {
            params.cycles_for_frequency(station.frequency_mhz)
        } else {
            params.quiet_amplitude
        }
    }

    fn draw_trace(&mut self, cx: &TabContext<'_>, selected: Option<&RadioStation>) {
        let Some(station) = selected else {
            self.wave = None;
            self.layout.dead_line.paint(cx.painter, cx.tint);
            return;
        };
        let tuned = cx.snapshot.model.radio.tuned;
        let params = self.layout.wave;
        let origin = self.layout.wave_origin;
        let cycles = Self::cycles(&params, station, tuned == Some(station.id));
        self.tune_wave(station, tuned);
        let Some((wave, _)) = self.wave.as_mut() else {
            return;
        };
        wave.advance_to(cx.seconds_open, cycles);
        let points = wave
            .points()
            .enumerate()
            .map(|(index, y)| Pos2::new(origin.x + index as f32, origin.y + y))
            .collect();
        let colour = paint::faded(params.line_shade.colour(cx.tint), params.line_alpha);
        cx.painter
            .add(Shape::line(points, Stroke::new(params.line_width, colour)));
    }

    fn paint_row(
        &self,
        painter: &egui::Painter,
        tint: egui::Color32,
        ui_layout: &UiLayout,
        row: &Row,
        station: &RadioStation,
        tuned: bool,
    ) {
        let layout = &self.layout;
        let fade = if station.in_range { 0.0 } else { layout.fade };
        let dim = |colour| brightness(colour, fade);
        if tuned {
            layout
                .equip_icon
                .translated(row.origin)
                .paint_mapped(painter, tint, |colour| {
                    if row.selected { BLACK } else { dim(colour) }
                });
        }
        let shown = layout.list.entry_text(ui_layout, &station.name);
        let row = Row {
            text_colour: dim(row.text_colour),
            ..*row
        };
        layout.list.paint_text(painter, &row, &shown);
    }
}

impl Tab for RadioPage {
    fn handle_input(&mut self, input: NavInput, cx: &mut TabContext<'_>) -> InputResult {
        let radio = &cx.snapshot.model.radio;
        let order = display_order(&radio.stations);
        let action = |page: &Self, position: Option<usize>| {
            let station = position
                .or_else(|| page.list.selected())
                .and_then(|position| order.get(position))
                .map(|&index| &radio.stations[index]);
            station.and_then(|station| activation(station, radio.tuned))
        };
        let activated = match input {
            NavInput::Hint(HintKey::Enter) => action(self, None),
            NavInput::Up
            | NavInput::Down
            | NavInput::Scrolled(_)
            | NavInput::PointerMoved(_)
            | NavInput::Clicked(_) => {
                let view = ListView::new(&self.layout.list, order.len());
                match self.list.handle(input, &view) {
                    ListEvent::Activated(position) => action(self, Some(position)),
                    ListEvent::None | ListEvent::Moved | ListEvent::Scrolled => None,
                }
            }
            _ => return InputResult::Ignored,
        };
        cx.actions.extend(activated);
        InputResult::Consumed
    }

    fn draw(&mut self, cx: &mut TabContext<'_>) {
        let radio = &cx.snapshot.model.radio;
        let order = display_order(&radio.stations);
        let view = ListView::new(&self.layout.list, order.len());
        self.list.invalidate(&view);
        if self.list.selected().is_none() {
            self.list.select(&view, self.list.scroll());
        }
        let tuned = radio.tuned;
        let (tint, ui_layout) = (cx.tint, cx.layout);
        self.list.paint(cx.painter, tint, &view, |painter, row| {
            let station = &radio.stations[order[row.index]];
            self.paint_row(
                painter,
                tint,
                ui_layout,
                row,
                station,
                tuned == Some(station.id),
            );
        });
        self.list.paint_arrows(cx, &view);
        self.draw_frame(cx);
        let selected = self.selected(&radio.stations, &order);
        self.draw_trace(cx, selected);
    }

    fn hints(&self, snapshot: &PipBoySnapshot<'_>) -> Vec<ButtonHint> {
        let radio = &snapshot.model.radio;
        let order = display_order(&radio.stations);
        let Some(station) = self.selected(&radio.stations, &order) else {
            return Vec::new();
        };
        if radio.tuned == Some(station.id) {
            vec![ButtonHint::new(
                HintKey::Enter,
                self.layout.off_label.clone(),
            )]
        } else {
            vec![
                ButtonHint::new(HintKey::Enter, self.layout.tune_label.clone())
                    .enabled(station.in_range),
            ]
        }
    }

    fn bottom_bar(&self) -> BottomBar {
        BottomBar::Radio
    }

    fn select_row(&mut self, row: usize) {
        let view = ListView::new(&self.layout.list, row + 1);
        self.list.select(&view, row);
    }

    fn on_open(&mut self) {
        self.wave = None;
        self.sessions = self.sessions.wrapping_add(1);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::rendering::pipboy::pages::test_harness::{Harness, vitals};
    use crate::rendering::pipboy::test_layout::pipboy_layout;
    use crate::sim::pipboy::{PipBoyCatalog, PipBoyModel};

    fn model(stations: Vec<RadioStation>, tuned: Option<StationId>) -> PipBoyModel {
        let mut model = PipBoyModel::new(Arc::new(PipBoyCatalog::default()));
        model.radio.stations = stations;
        model.radio.tuned = tuned;
        model
    }

    fn station(id: u16, in_range: bool) -> RadioStation {
        RadioStation {
            id: StationId(id),
            name: format!("Station {id}"),
            frequency_mhz: 90.0,
            in_range,
        }
    }

    fn page(_harness: &Harness) -> RadioPage {
        RadioPage::new(RadioLayout::resolve(&pipboy_layout()).unwrap())
    }

    fn snapshot(model: &PipBoyModel) -> PipBoySnapshot<'_> {
        PipBoySnapshot {
            model,
            vitals: vitals(),
        }
    }

    #[test]
    fn stations_in_range_list_first_and_each_group_by_name_ignoring_case() {
        let named = |id, name: &str, in_range| RadioStation {
            name: name.to_owned(),
            ..station(id, in_range)
        };
        let stations = [
            named(1, "zulu", true),
            named(2, "Alpha", false),
            named(3, "Mike", true),
            named(4, "bravo", false),
        ];
        assert_eq!(display_order(&stations), vec![2, 0, 1, 3]);
    }

    #[test]
    fn an_untuned_in_range_station_is_tuned() {
        assert_eq!(
            activation(&station(1, true), None),
            Some(PipBoyAction::TuneRadio(StationId(1)))
        );
    }

    #[test]
    fn the_tuned_station_switches_the_radio_off_even_out_of_range() {
        assert_eq!(
            activation(&station(1, false), Some(StationId(1))),
            Some(PipBoyAction::TurnOffRadio)
        );
    }

    #[test]
    fn an_out_of_range_station_cannot_be_tuned() {
        assert_eq!(activation(&station(2, false), Some(StationId(1))), None);
    }

    #[test]
    fn the_ruler_has_a_long_tick_every_fourth_and_fits_the_height() {
        let ticks = Ruler::ticks(249.1);
        assert_eq!(ticks.len(), 41);
        assert!((ticks[0].1 - Ruler::LONG).abs() < 1e-6);
        assert!((ticks[1].1 - Ruler::SHORT).abs() < 1e-6);
        assert!((ticks[4].1 - Ruler::LONG).abs() < 1e-6);
        assert!(ticks.iter().all(|(offset, _)| *offset < 249.1));
    }

    #[test]
    fn enter_tunes_the_selected_station() {
        let model = model(vec![station(1, true), station(2, true)], None);
        let mut harness = Harness::new();
        let mut page = page(&harness);
        harness.draw(&mut page, &model);

        harness.send(&mut page, &model, NavInput::Hint(HintKey::Enter));

        assert_eq!(
            harness.take_actions(),
            vec![PipBoyAction::TuneRadio(StationId(1)),]
        );
    }

    #[test]
    fn enter_on_the_tuned_station_turns_the_radio_off_and_an_unreachable_one_does_nothing() {
        let model = model(
            vec![station(1, true), station(2, false)],
            Some(StationId(1)),
        );
        let mut harness = Harness::new();
        let mut page = page(&harness);
        harness.draw(&mut page, &model);

        harness.send(&mut page, &model, NavInput::Hint(HintKey::Enter));
        harness.send(&mut page, &model, NavInput::Down);
        harness.send(&mut page, &model, NavInput::Hint(HintKey::Enter));

        assert_eq!(harness.take_actions(), vec![PipBoyAction::TurnOffRadio]);
    }

    #[test]
    fn the_selected_station_gets_a_trace_and_no_selection_drops_it() {
        let mut harness = Harness::new();
        let mut page = page(&harness);
        harness.draw(&mut page, &model(vec![station(1, true)], None));
        assert!(page.wave.is_some());
        harness.draw(&mut page, &model(Vec::new(), None));
        assert!(page.wave.is_none());
    }

    #[test]
    fn a_new_session_draws_a_trace_of_its_own() {
        let model = model(vec![station(1, true)], Some(StationId(1)));
        let mut harness = Harness::new();
        let mut page = page(&harness);
        let trace = |page: &mut RadioPage, harness: &mut Harness| {
            harness.seconds_open = 2.0;
            harness.draw(page, &model);
            page.wave.as_ref().unwrap().0.points().collect::<Vec<_>>()
        };
        page.on_open();
        let first = trace(&mut page, &mut harness);
        page.on_open();
        let second = trace(&mut page, &mut harness);
        assert_ne!(first, second);
    }

    #[test]
    fn tuning_the_selected_station_regenerates_the_trace_for_its_frequency() {
        let mut harness = Harness::new();
        let mut page = page(&harness);
        harness.draw(&mut page, &model(vec![station(1, true)], None));
        let idle = page.wave.as_ref().unwrap().1;
        harness.draw(
            &mut page,
            &model(vec![station(1, true)], Some(StationId(1))),
        );
        let playing = page.wave.as_ref().unwrap().1;
        assert!(!idle.playing && playing.playing);
    }

    #[test]
    fn hints_follow_the_selected_station() {
        let mut harness = Harness::new();
        let mut page = page(&harness);
        let tuned = model(vec![station(1, true)], Some(StationId(1)));
        harness.draw(&mut page, &tuned);
        assert_eq!(page.hints(&snapshot(&tuned)).len(), 1);
        let empty = model(Vec::new(), None);
        harness.draw(&mut page, &empty);
        assert_eq!(page.hints(&snapshot(&empty)), []);
    }
}
