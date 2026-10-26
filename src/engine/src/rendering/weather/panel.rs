//! The debug panel of the weather cycle: what the cycle is doing, the controls to steer it and the
//! controls to look at the clouds one layer at a time.

use super::CLOUD_LAYER_COUNT;
use super::frame::WeatherFrame;
use super::library::WeatherLibrary;
use crate::rendering::rain::{RainAssetStatus, RainTextureState};
use crate::sim::WeatherSnapshot;
use crate::sim::commands::WeatherEdit;
use crate::weather::WeatherId;

const LAST_CLOUD_LAYER: u8 = (CLOUD_LAYER_COUNT - 1) as u8;

/// What the panel shows of the weather this frame.
pub struct WeatherView<'a> {
    pub library: &'a WeatherLibrary,
    pub snapshot: WeatherSnapshot<'a>,
    /// `None` when the frame could not be made.
    pub frame: Option<&'a WeatherFrame>,
}

/// How the clouds are drawn, for looking at them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CloudControls {
    pub draw: bool,
    solo_enabled: bool,
    solo_layer: u8,
}

impl Default for CloudControls {
    fn default() -> Self {
        Self {
            draw: true,
            solo_enabled: false,
            solo_layer: 0,
        }
    }
}

impl CloudControls {
    /// The one layer to draw, if only one is.
    pub fn solo(&self) -> Option<u8> {
        self.solo_enabled.then_some(self.solo_layer)
    }
}

#[cfg(test)]
impl CloudControls {
    pub(super) fn hidden() -> Self {
        Self {
            draw: false,
            ..Self::default()
        }
    }

    pub(super) fn soloing(layer: u8) -> Self {
        Self {
            solo_enabled: true,
            solo_layer: layer,
            ..Self::default()
        }
    }
}

/// How the rain is drawn, for looking at it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RainControls {
    pub draw: bool,
    /// Whether the rain stirs ripples in the water around the camera.
    pub ripples: bool,
}

impl Default for RainControls {
    fn default() -> Self {
        Self {
            draw: true,
            ripples: true,
        }
    }
}

#[derive(Debug, Default)]
pub struct WeatherPanelState {
    /// The weather the user picked for the Force and Transition buttons; `None` until one is.
    picked: Option<WeatherId>,
    pub clouds: CloudControls,
    pub rain: RainControls,
}

impl WeatherPanelState {
    /// The weather the Force and Transition buttons lead to: the one picked, while the library
    /// has it, else the current weather, so the target follows the cycle until another is picked.
    fn target(&self, weather_count: usize, current: WeatherId) -> WeatherId {
        self.picked
            .filter(|picked| picked.index() < weather_count)
            .unwrap_or(current)
    }

    /// Takes what the picker settled on after showing `shown`: only a change is a pick.
    fn settle(&mut self, shown: WeatherId, settled: WeatherId) -> WeatherId {
        if settled != shown {
            self.picked = Some(settled);
        }
        settled
    }
}

/// What the user did to the weather controls in one frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WeatherActions {
    pub force: bool,
    pub transition: bool,
    pub lightning: bool,
    /// The progress the slider was moved to.
    pub progress: Option<f32>,
}

/// The edits for `actions` on the weather `target`; an action the user did not take produces
/// none, so the running cycle is never overwritten by its own readout.
pub fn weather_edits(target: WeatherId, actions: &WeatherActions) -> Vec<WeatherEdit> {
    let mut edits = Vec::new();
    if actions.force {
        edits.push(WeatherEdit::Force(target));
    }
    if actions.transition {
        edits.push(WeatherEdit::Transition(target));
    }
    if let Some(progress) = actions.progress {
        edits.push(WeatherEdit::SetProgress(progress));
    }
    if actions.lightning {
        edits.push(WeatherEdit::TriggerLightning);
    }
    edits
}

/// `view` is `None` while the world has no weather cycle.
pub fn weather_panel(
    ui: &mut egui::Ui,
    state: &mut WeatherPanelState,
    view: Option<&WeatherView<'_>>,
    rain: &RainAssetStatus,
    edits: &mut Vec<WeatherEdit>,
) {
    let Some(view) = view else {
        ui.label("Weather cycle: none, the exterior keeps its single weather");
        return;
    };
    for line in readouts(view) {
        ui.label(line);
    }
    let target = picker(ui, state, view);
    let mut actions = WeatherActions::default();
    ui.horizontal(|ui| {
        actions.force = ui.button("Force").clicked();
        actions.transition = ui.button("Transition").clicked();
    });
    if view.snapshot.previous.is_some() {
        let mut progress = view.snapshot.progress;
        let slider = egui::Slider::new(&mut progress, 0.0..=1.0).text("progress");
        if ui.add(slider).changed() {
            actions.progress = Some(progress);
        }
    }
    actions.lightning = ui.button("Trigger lightning").clicked();
    edits.extend(weather_edits(target, &actions));
    cloud_controls(ui, &mut state.clouds);
    rain_controls(ui, &mut state.rain, rain);
}

fn picker(ui: &mut egui::Ui, state: &mut WeatherPanelState, view: &WeatherView<'_>) -> WeatherId {
    let count = view.library.weathers().len();
    let shown = state.target(count, view.snapshot.current);
    let mut picked = shown;
    egui::ComboBox::from_label("weather")
        .selected_text(weather_name(view.library, picked))
        .show_ui(ui, |ui| {
            for index in 0..count {
                let id = WeatherId::new(index);
                ui.selectable_value(&mut picked, id, weather_name(view.library, id));
            }
        });
    state.settle(shown, picked)
}

fn cloud_controls(ui: &mut egui::Ui, controls: &mut CloudControls) {
    ui.checkbox(&mut controls.draw, "Draw clouds");
    ui.horizontal(|ui| {
        ui.checkbox(&mut controls.solo_enabled, "Solo layer");
        ui.add_enabled(
            controls.solo_enabled,
            egui::Slider::new(&mut controls.solo_layer, 0..=LAST_CLOUD_LAYER),
        );
    });
}

fn rain_controls(ui: &mut egui::Ui, controls: &mut RainControls, status: &RainAssetStatus) {
    ui.checkbox(&mut controls.draw, "Draw rain");
    ui.checkbox(&mut controls.ripples, "Rain ripples in water");
    for line in rain_status_lines(status) {
        ui.label(line);
    }
}

fn rain_status_lines(status: &RainAssetStatus) -> [String; 2] {
    [
        texture_line("Rain streaks", &status.streaks),
        texture_line("Rain splashes", &status.splashes),
    ]
}

fn texture_line(name: &str, state: &RainTextureState) -> String {
    match state {
        RainTextureState::NotRequested => format!("{name}: not requested"),
        RainTextureState::Loaded => format!("{name}: loaded"),
        RainTextureState::Failed(message) => format!("{name}: failed, {message}"),
    }
}

fn readouts(view: &WeatherView<'_>) -> Vec<String> {
    let WeatherView {
        library,
        snapshot,
        frame,
    } = view;
    vec![
        format!("Current: {}", weather_name(library, snapshot.current)),
        previous_label(library, snapshot),
        progress_label(snapshot),
        next_roll_label(snapshot),
        precipitation_label(library, *frame),
        thunder_label(*frame),
        wind_label(*frame),
    ]
}

/// The editor id of the weather, else its form key.
fn weather_name(library: &WeatherLibrary, id: WeatherId) -> String {
    match library.get(id) {
        Some(weather) => weather
            .editor_id
            .as_deref()
            .unwrap_or(&weather.form_key)
            .to_owned(),
        None => format!("{id:?} (not in the library)"),
    }
}

fn previous_label(library: &WeatherLibrary, snapshot: &WeatherSnapshot<'_>) -> String {
    match snapshot.previous {
        Some(previous) => format!("Previous: {}", weather_name(library, previous)),
        None => "Previous: -".to_owned(),
    }
}

fn progress_label(snapshot: &WeatherSnapshot<'_>) -> String {
    match snapshot.previous {
        Some(_) => format!("Progress: {}%", (snapshot.progress * 100.0).round()),
        None => "Progress: settled".to_owned(),
    }
}

fn next_roll_label(snapshot: &WeatherSnapshot<'_>) -> String {
    if snapshot.hours_to_next_roll > 0.0 {
        format!(
            "Next roll in: {:.1} game hours",
            snapshot.hours_to_next_roll
        )
    } else {
        "Next roll: due".to_owned()
    }
}

fn precipitation_label(library: &WeatherLibrary, frame: Option<&WeatherFrame>) -> String {
    match frame.map(|frame| frame.precipitation) {
        None => "Precipitation: -".to_owned(),
        Some(None) => "Precipitation: none".to_owned(),
        Some(Some(precipitation)) => format!(
            "Precipitation: {:.2} ({})",
            precipitation.intensity,
            weather_name(library, precipitation.weather)
        ),
    }
}

fn thunder_label(frame: Option<&WeatherFrame>) -> String {
    frame.map_or_else(
        || "Thunder: -".to_owned(),
        |frame| format!("Thunder: {:.2}", frame.thunder_intensity),
    )
}

fn wind_label(frame: Option<&WeatherFrame>) -> String {
    frame.map_or_else(
        || "Wind: -".to_owned(),
        |frame| {
            format!(
                "Wind: speed {:.2}, turbulence {:.2}, heading {:.0} deg",
                frame.wind.speed,
                frame.wind.turbulence,
                frame.wind.heading.to_degrees()
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::lighting::time_of_day::{SlotBlend, TimeSlot};
    use crate::rendering::weather::definition::fixtures::weather_def;
    use crate::rendering::weather::library::fixtures::library_of;
    use crate::rendering::weather::{PrecipitationFrame, WindFrame};
    use crate::weather::WeatherFlags;

    const A: WeatherId = WeatherId::new(0);
    const B: WeatherId = WeatherId::new(1);

    fn library() -> WeatherLibrary {
        let mut unnamed = weather_def("0012AB:Fallout4.esm");
        unnamed.editor_id = None;
        library_of(vec![weather_def("A"), unnamed])
    }

    fn settled(current: WeatherId) -> WeatherSnapshot<'static> {
        WeatherSnapshot {
            current,
            previous: None,
            progress: 1.0,
            strikes: &[],
            hours_to_next_roll: 3.5,
        }
    }

    fn changing() -> WeatherSnapshot<'static> {
        WeatherSnapshot {
            current: B,
            previous: Some(A),
            progress: 0.42,
            strikes: &[],
            hours_to_next_roll: 9.5,
        }
    }

    fn frame(library: &WeatherLibrary) -> WeatherFrame {
        let day = SlotBlend {
            from: TimeSlot::Day,
            to: TimeSlot::Day,
            t: 0.0,
        };
        WeatherFrame {
            sky: library.get(A).unwrap().slots.sample(day),
            cloud_layers: Vec::new(),
            precipitation: Some(PrecipitationFrame {
                weather: B,
                intensity: 0.5,
                flags: WeatherFlags::default(),
            }),
            thunder_intensity: 0.2,
            wind: WindFrame {
                speed: 0.47,
                turbulence: 0.19,
                heading: std::f32::consts::FRAC_PI_2,
            },
            strikes: Vec::new(),
        }
    }

    fn loaded() -> RainAssetStatus {
        RainAssetStatus {
            streaks: RainTextureState::Loaded,
            splashes: RainTextureState::Loaded,
        }
    }

    fn show(state: &mut WeatherPanelState, view: &WeatherView<'_>) -> Vec<WeatherEdit> {
        let ctx = egui::Context::default();
        let mut edits = Vec::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            weather_panel(ui, state, Some(view), &loaded(), &mut edits);
        });
        output.textures_delta.clear();
        edits
    }

    #[test]
    fn each_texture_of_the_rain_says_what_became_of_it() {
        let status = RainAssetStatus {
            streaks: RainTextureState::Failed("file missing".to_owned()),
            splashes: RainTextureState::NotRequested,
        };

        assert_eq!(
            rain_status_lines(&status),
            [
                "Rain streaks: failed, file missing",
                "Rain splashes: not requested"
            ]
        );
        assert_eq!(
            rain_status_lines(&loaded()),
            ["Rain streaks: loaded", "Rain splashes: loaded"]
        );
    }

    #[test]
    fn controls_left_alone_produce_no_edit() {
        assert_eq!(weather_edits(A, &WeatherActions::default()), []);
    }

    #[test]
    fn the_target_follows_the_current_weather_until_another_is_picked() {
        let mut state = WeatherPanelState::default();
        assert_eq!(state.target(2, A), A);
        assert_eq!(state.target(2, B), B);
        assert_eq!(state.settle(B, A), A);
        assert_eq!(state.target(2, B), A);
        assert_eq!(state.target(2, A), A);
    }

    #[test]
    fn showing_a_weather_is_no_pick() {
        let mut state = WeatherPanelState::default();
        assert_eq!(state.settle(A, A), A);
        assert_eq!(state.target(2, B), B);
    }

    #[test]
    fn a_picked_weather_the_library_lacks_gives_way_to_the_current_one() {
        let mut state = WeatherPanelState::default();
        state.settle(A, WeatherId::new(5));
        assert_eq!(state.target(5, B), B);
        assert_eq!(state.target(6, B), WeatherId::new(5));
    }

    #[test]
    fn the_panel_alone_neither_picks_a_weather_nor_edits_the_cycle() {
        let library = library();
        let mut state = WeatherPanelState::default();
        for snapshot in [settled(A), changing(), settled(B)] {
            let view = WeatherView {
                library: &library,
                snapshot,
                frame: None,
            };
            assert_eq!(show(&mut state, &view), []);
            assert_eq!(state.picked, None);
        }
    }

    #[test]
    fn each_action_produces_exactly_its_own_edit() {
        let only = |actions: WeatherActions| weather_edits(B, &actions);
        assert_eq!(
            only(WeatherActions {
                force: true,
                ..Default::default()
            }),
            [WeatherEdit::Force(B)]
        );
        assert_eq!(
            only(WeatherActions {
                transition: true,
                ..Default::default()
            }),
            [WeatherEdit::Transition(B)]
        );
        assert_eq!(
            only(WeatherActions {
                lightning: true,
                ..Default::default()
            }),
            [WeatherEdit::TriggerLightning]
        );
        assert_eq!(
            only(WeatherActions {
                progress: Some(0.75),
                ..Default::default()
            }),
            [WeatherEdit::SetProgress(0.75)]
        );
    }

    #[test]
    fn several_actions_at_once_produce_one_edit_each_with_the_switch_first() {
        let edits = weather_edits(
            B,
            &WeatherActions {
                force: true,
                transition: true,
                lightning: true,
                progress: Some(0.5),
            },
        );
        assert_eq!(
            edits,
            [
                WeatherEdit::Force(B),
                WeatherEdit::Transition(B),
                WeatherEdit::SetProgress(0.5),
                WeatherEdit::TriggerLightning,
            ]
        );
    }

    #[test]
    fn the_clouds_draw_in_full_until_the_controls_say_otherwise() {
        let controls = CloudControls::default();
        assert!(controls.draw);
        assert_eq!(controls.solo(), None);
    }

    #[test]
    fn a_solo_layer_is_only_one_while_the_solo_switch_is_on() {
        let mut controls = CloudControls {
            solo_layer: 7,
            ..CloudControls::default()
        };
        assert_eq!(controls.solo(), None);
        controls.solo_enabled = true;
        assert_eq!(controls.solo(), Some(7));
    }

    #[test]
    fn the_slider_reaches_every_cloud_layer_and_no_more() {
        assert_eq!(usize::from(LAST_CLOUD_LAYER) + 1, CLOUD_LAYER_COUNT);
    }

    #[test]
    fn a_weather_is_named_by_its_editor_id_then_its_form_key() {
        let library = library();
        assert_eq!(weather_name(&library, A), "TestWeather");
        assert_eq!(weather_name(&library, B), "0012AB:Fallout4.esm");
    }

    #[test]
    fn a_weather_the_library_lacks_is_named_as_such() {
        let name = weather_name(&library(), WeatherId::new(5));
        assert!(name.contains("not in the library"), "{name}");
    }

    #[test]
    fn a_settled_weather_reads_as_current_without_a_previous_one() {
        let library = library();
        let view = WeatherView {
            library: &library,
            snapshot: settled(A),
            frame: None,
        };
        let lines = readouts(&view);
        assert_eq!(lines[0], "Current: TestWeather");
        assert_eq!(lines[1], "Previous: -");
        assert_eq!(lines[2], "Progress: settled");
    }

    #[test]
    fn a_transition_reads_with_both_weathers_and_its_progress() {
        let library = library();
        let view = WeatherView {
            library: &library,
            snapshot: changing(),
            frame: None,
        };
        let lines = readouts(&view);
        assert_eq!(lines[0], "Current: 0012AB:Fallout4.esm");
        assert_eq!(lines[1], "Previous: TestWeather");
        assert_eq!(lines[2], "Progress: 42%");
    }

    #[test]
    fn the_next_roll_is_counted_in_game_hours_whether_the_weather_is_settled_or_changing() {
        assert_eq!(next_roll_label(&settled(A)), "Next roll in: 3.5 game hours");
        assert_eq!(next_roll_label(&changing()), "Next roll in: 9.5 game hours");
    }

    #[test]
    fn a_roll_that_is_due_reads_as_due() {
        let due = WeatherSnapshot {
            hours_to_next_roll: 0.0,
            ..settled(A)
        };
        assert_eq!(next_roll_label(&due), "Next roll: due");
    }

    #[test]
    fn without_a_frame_the_frame_readouts_are_dashes() {
        let library = library();
        let view = WeatherView {
            library: &library,
            snapshot: settled(A),
            frame: None,
        };
        assert_eq!(
            readouts(&view)[4..],
            ["Precipitation: -", "Thunder: -", "Wind: -"]
        );
    }

    #[test]
    fn precipitation_reads_its_intensity_and_the_weather_it_falls_from() {
        let library = library();
        assert_eq!(
            precipitation_label(&library, Some(&frame(&library))),
            "Precipitation: 0.50 (0012AB:Fallout4.esm)"
        );
    }

    #[test]
    fn thunder_reads_its_intensity() {
        assert_eq!(thunder_label(Some(&frame(&library()))), "Thunder: 0.20");
    }

    #[test]
    fn wind_reads_its_speed_its_turbulence_and_its_heading_in_degrees() {
        assert_eq!(
            wind_label(Some(&frame(&library()))),
            "Wind: speed 0.47, turbulence 0.19, heading 90 deg"
        );
    }

    #[test]
    fn the_readouts_run_from_the_weathers_over_the_cycle_to_the_frame() {
        let library = library();
        let frame = frame(&library);
        let snapshot = changing();
        let view = WeatherView {
            library: &library,
            snapshot,
            frame: Some(&frame),
        };
        assert_eq!(
            readouts(&view),
            [
                format!("Current: {}", weather_name(&library, B)),
                previous_label(&library, &snapshot),
                progress_label(&snapshot),
                next_roll_label(&snapshot),
                precipitation_label(&library, Some(&frame)),
                thunder_label(Some(&frame)),
                wind_label(Some(&frame)),
            ]
        );
    }

    #[test]
    fn a_frame_without_precipitation_reads_none() {
        let library = library();
        let frame = WeatherFrame {
            precipitation: None,
            ..frame(&library)
        };
        assert_eq!(
            precipitation_label(&library, Some(&frame)),
            "Precipitation: none"
        );
    }
}
