//! The game's flow from the first frame to play: boot, main menu, spawn picker, loading
//! screen, reveal. It consumes abstract input and readiness signals and answers with the
//! action the caller has to perform; it knows neither the GPU nor the window.

mod fade;
mod load_gate;
mod menu_list;
pub mod spawn_locations;
mod start_mode;
mod wheel;

use fade::Fade;
use load_gate::LoadGate;
pub use load_gate::{PlayerWait, ProgressCounts, Readiness, StreamCounts};
use menu_list::MenuList;
pub use menu_list::MenuView;
pub use spawn_locations::{SpawnLocation, SpawnPlace};
pub use start_mode::StartMode;
use wheel::WheelAccumulator;

/// The labels are translate keys: the menu shows their text by the game's text-setting rule.
const PLAY_LABEL: &str = "$PLAY";
const QUIT_LABEL: &str = "$QUIT";
const BACK_LABEL: &str = "$BACK";
const PLAY_ROW: usize = 0;
const QUIT_ROW: usize = 1;
/// After a menu opens, activation is ignored for this long, so the second click of a double
/// click or a held key cannot carry through into the screen that opened.
const ACTIVATION_DEBOUNCE_SECONDS: f32 = 0.25;
/// The largest frame time one step of the flow's timers may consume, so a stalled or suspended
/// frame cannot skip the fade, the debounce or push the load towards its stall timeout.
const MAX_FRAME_STEP_SECONDS: f32 = 0.05;

#[allow(
    clippy::manual_clamp,
    reason = "f32::clamp passes NaN through, max/min turn a NaN frame time into no progress"
)]
fn capped_step(dt: f32) -> f32 {
    dt.max(0.0).min(MAX_FRAME_STEP_SECONDS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    MainMenu,
    SpawnSelect,
    Loading,
    Revealing,
    InGame,
}

impl Phase {
    pub fn shows_menu(self) -> bool {
        matches!(self, Self::MainMenu | Self::SpawnSelect)
    }

    /// Whether the phase shows a list the player picks a row of.
    fn shows_list(self) -> bool {
        matches!(self, Self::MainMenu | Self::SpawnSelect)
    }
}

/// Input in terms of the menu, already decoded from keys and pointer positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowEvent {
    NavUp,
    NavDown,
    Activate,
    Back,
    /// The pointer is over the list row with this index.
    RowHovered(usize),
    /// The primary pointer button was pressed over the list row with this index.
    RowPressed(usize),
    /// The primary pointer button was released, over the list row with this index if the
    /// pointer was over a row. A row activates when it is released after being pressed.
    RowReleased(Option<usize>),
    /// Mouse wheel steps; positive scrolls towards the first row.
    Wheel(i32),
}

/// What the caller has to do after an input.
#[derive(Debug, Clone, PartialEq)]
pub enum FlowAction {
    None,
    BeginPlay(SpawnLocation),
    ReleaseFoldHold,
    LockCursor,
    Quit,
}

pub struct GameFlow {
    phase: Phase,
    main_menu: MenuList,
    spawn_menu: MenuList,
    locations: Vec<SpawnLocation>,
    gate: LoadGate,
    fade: Fade,
    wheel: WheelAccumulator,
    /// The list row the pointer button went down on in the current phase.
    pressed_row: Option<usize>,
    since_phase_change: f32,
    /// Seconds since the current load began; the reveal that ends it continues the count.
    loading_seconds: f64,
}

impl GameFlow {
    /// A flow that starts at the main menu, or with `StartMode::Direct` already loading (the
    /// caller has started the load). `visible_rows` is how many rows of a list fit on screen.
    pub fn new(locations: Vec<SpawnLocation>, visible_rows: usize, mode: StartMode) -> Self {
        let spawn_labels = locations
            .iter()
            .map(|location| location.name)
            .chain([BACK_LABEL])
            .collect();
        let mut flow = Self {
            phase: Phase::MainMenu,
            main_menu: MenuList::new(vec![PLAY_LABEL, QUIT_LABEL], visible_rows),
            spawn_menu: MenuList::new(spawn_labels, visible_rows),
            locations,
            gate: LoadGate::default(),
            fade: Fade::default(),
            wheel: WheelAccumulator::default(),
            pressed_row: None,
            since_phase_change: 0.0,
            loading_seconds: 0.0,
        };
        if mode == StartMode::Direct {
            flow.begin_loading();
        }
        flow
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Shows the main menu in place of a direct start's load; `release_to_loading` resumes it.
    pub fn hold_at_main_menu(&mut self) {
        self.enter(Phase::MainMenu);
    }

    /// Leaves the main menu for the load a direct start deferred; does nothing elsewhere.
    pub fn release_to_loading(&mut self) {
        if self.phase == Phase::MainMenu {
            self.begin_loading();
        }
    }

    /// Every phase change goes through here: a new phase starts its debounce and forgets the
    /// pointer press and the wheel remainder of the one before.
    fn enter(&mut self, phase: Phase) {
        self.phase = phase;
        self.since_phase_change = 0.0;
        self.pressed_row = None;
        self.wheel.reset();
    }

    pub fn handle(&mut self, event: FlowEvent) -> FlowAction {
        if !self.phase.shows_list() {
            return FlowAction::None;
        }
        match event {
            FlowEvent::Activate if self.accepts_activation() => {
                self.activate_row(self.active_list().selected())
            }
            FlowEvent::Activate => FlowAction::None,
            FlowEvent::Back => self.go_back(),
            FlowEvent::RowPressed(row) => {
                self.press_row(row);
                FlowAction::None
            }
            FlowEvent::RowReleased(row) => self.release_pointer(row),
            FlowEvent::Wheel(steps) => {
                self.scroll_steps(steps);
                FlowAction::None
            }
            FlowEvent::NavUp => {
                self.active_list_mut().select_previous();
                FlowAction::None
            }
            FlowEvent::NavDown => {
                self.active_list_mut().select_next();
                FlowAction::None
            }
            FlowEvent::RowHovered(row) => {
                self.active_list_mut().select(row);
                FlowAction::None
            }
        }
    }

    /// Feeds the mouse wheel's travel in lines (positive away from the user). Only whole lines
    /// scroll a list; the fractions of a touchpad's small deltas add up until they do.
    pub fn scroll(&mut self, lines: f32) {
        if !self.phase.shows_list() {
            return;
        }
        if let Some(steps) = self.wheel.push(lines) {
            self.scroll_steps(steps);
        }
    }

    /// Feeds one frame's readiness signals; only a load in progress reacts.
    pub fn readiness(&mut self, readiness: Readiness) -> FlowAction {
        if self.phase != Phase::Loading {
            return FlowAction::None;
        }
        let step = self.gate.update(readiness);
        if let Some(stall) = step.stalled {
            tracing::error!(
                "loading made no progress for {}s: {stall}",
                load_gate::NO_PROGRESS_TIMEOUT_SECONDS,
            );
        }
        if step.reveal {
            self.enter(Phase::Revealing);
            self.fade = Fade::default();
            FlowAction::LockCursor
        } else if step.release_fold_hold {
            FlowAction::ReleaseFoldHold
        } else {
            FlowAction::None
        }
    }

    /// Advances the load timers and the reveal fade by one frame of `dt` seconds.
    pub fn tick(&mut self, dt: f32) {
        self.since_phase_change += capped_step(dt);
        if self.loading_seconds().is_some() {
            self.loading_seconds += f64::from(dt.max(0.0));
        }
        match self.phase {
            Phase::Loading => self.gate.tick(dt),
            Phase::Revealing => {
                self.fade.advance(dt);
                if self.fade.is_finished() {
                    self.enter(Phase::InGame);
                }
            }
            Phase::MainMenu | Phase::SpawnSelect | Phase::InGame => {}
        }
    }

    /// The list to draw, in the menu phases.
    pub fn menu_view(&self) -> Option<MenuView<'_>> {
        match self.phase {
            Phase::MainMenu => Some(self.main_menu.view()),
            Phase::SpawnSelect => Some(self.spawn_menu.view()),
            Phase::Loading | Phase::Revealing | Phase::InGame => None,
        }
    }

    /// Opacity of the black layer over the world.
    pub fn overlay_alpha(&self) -> f32 {
        match self.phase {
            Phase::MainMenu | Phase::SpawnSelect | Phase::Loading => 1.0,
            Phase::Revealing => self.fade.alpha(),
            Phase::InGame => 0.0,
        }
    }

    /// Seconds since the current load began while the loading screen is up, including the
    /// reveal that fades it out; `None` otherwise. Drives the loading screen's animation.
    pub fn loading_seconds(&self) -> Option<f64> {
        matches!(self.phase, Phase::Loading | Phase::Revealing).then_some(self.loading_seconds)
    }

    pub fn hud_visible(&self) -> bool {
        self.gameplay_input_enabled()
    }

    /// Whether keys and mouse motion reach the camera and the player.
    pub fn gameplay_input_enabled(&self) -> bool {
        matches!(self.phase, Phase::Revealing | Phase::InGame)
    }

    pub fn developer_window_visible(&self) -> bool {
        self.phase == Phase::InGame
    }

    /// Whether queued toasts may be shown. They wait out the black loading screen and the
    /// reveal fade, which would otherwise draw them over a black screen.
    pub fn toasts_visible(&self) -> bool {
        matches!(
            self.phase,
            Phase::MainMenu | Phase::SpawnSelect | Phase::InGame
        )
    }

    fn begin_loading(&mut self) {
        self.enter(Phase::Loading);
        self.gate = LoadGate::default();
        self.loading_seconds = 0.0;
    }

    /// The list of the current phase; only called in a phase that shows one.
    fn active_list(&self) -> &MenuList {
        match self.phase {
            Phase::SpawnSelect => &self.spawn_menu,
            _ => &self.main_menu,
        }
    }

    fn active_list_mut(&mut self) -> &mut MenuList {
        match self.phase {
            Phase::SpawnSelect => &mut self.spawn_menu,
            _ => &mut self.main_menu,
        }
    }

    fn accepts_activation(&self) -> bool {
        self.since_phase_change >= ACTIVATION_DEBOUNCE_SECONDS
    }

    /// Records the press when `row` exists and activation is accepted; the row activates on
    /// release.
    fn press_row(&mut self, row: usize) {
        if self.active_list_mut().select(row) && self.accepts_activation() {
            self.pressed_row = Some(row);
        }
    }

    /// Activates the row that received the press in this phase if the button came up over it.
    fn release_pointer(&mut self, released_over: Option<usize>) -> FlowAction {
        match (self.pressed_row.take(), released_over) {
            (Some(pressed), Some(released)) if pressed == released => self.activate_row(pressed),
            _ => FlowAction::None,
        }
    }

    fn scroll_steps(&mut self, steps: i32) {
        self.active_list_mut().scroll(steps.saturating_neg());
    }

    fn activate_row(&mut self, row: usize) -> FlowAction {
        match self.phase {
            Phase::MainMenu => self.activate_main_menu_row(row),
            Phase::SpawnSelect => self.activate_spawn_row(row),
            Phase::Loading | Phase::Revealing | Phase::InGame => FlowAction::None,
        }
    }

    fn activate_main_menu_row(&mut self, row: usize) -> FlowAction {
        match row {
            PLAY_ROW => {
                self.spawn_menu.reset();
                self.enter(Phase::SpawnSelect);
                FlowAction::None
            }
            QUIT_ROW => FlowAction::Quit,
            _ => FlowAction::None,
        }
    }

    fn activate_spawn_row(&mut self, row: usize) -> FlowAction {
        let back_row = self.locations.len();
        if row == back_row {
            return self.go_back();
        }
        let Some(location) = self.locations.get(row).cloned() else {
            return FlowAction::None;
        };
        self.begin_loading();
        FlowAction::BeginPlay(location)
    }

    fn go_back(&mut self) -> FlowAction {
        if self.phase == Phase::SpawnSelect {
            self.enter(Phase::MainMenu);
        }
        FlowAction::None
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the overlay opacity at the start and end of a load is exactly 1 and 0"
)]
mod tests {
    use super::*;

    const VISIBLE_ROWS: usize = 3;

    fn location(name: &'static str) -> SpawnLocation {
        SpawnLocation {
            name,
            place: SpawnPlace::Exterior {
                worldspace: spawn_locations::COMMONWEALTH.to_owned(),
            },
            position: [1.0, 2.0, 3.0],
            heading: None,
        }
    }

    fn locations() -> Vec<SpawnLocation> {
        ["ONE", "TWO", "THREE", "FOUR", "FIVE"].map(location).into()
    }

    /// Lets the debounce of the phase the flow just entered run out.
    fn settle(flow: &mut GameFlow) {
        let frames = (ACTIVATION_DEBOUNCE_SECONDS / MAX_FRAME_STEP_SECONDS).ceil() as u32 + 1;
        for _ in 0..frames {
            flow.tick(MAX_FRAME_STEP_SECONDS);
        }
    }

    /// A full click: the button goes down and comes up over `row`.
    fn click(flow: &mut GameFlow, row: usize) -> FlowAction {
        flow.handle(FlowEvent::RowPressed(row));
        flow.handle(FlowEvent::RowReleased(Some(row)))
    }

    fn menu_flow() -> GameFlow {
        let mut flow = GameFlow::new(locations(), VISIBLE_ROWS, StartMode::Menu);
        settle(&mut flow);
        flow
    }

    fn spawn_select_flow() -> GameFlow {
        let mut flow = menu_flow();
        flow.handle(FlowEvent::Activate);
        assert_eq!(flow.phase(), Phase::SpawnSelect);
        settle(&mut flow);
        flow
    }

    fn loading_flow() -> GameFlow {
        let mut flow = spawn_select_flow();
        assert!(matches!(
            flow.handle(FlowEvent::Activate),
            FlowAction::BeginPlay(_)
        ));
        flow
    }

    fn streamed_with_player() -> Readiness {
        Readiness {
            full_ring_complete: true,
            player_ready: true,
            ..Readiness::default()
        }
    }

    /// Runs up to `frames` loading frames of 0.1 s and stops when the load ends, returning the
    /// actions other than `None`.
    fn run_load(flow: &mut GameFlow, frames: u32, readiness: Readiness) -> Vec<FlowAction> {
        let mut actions = Vec::new();
        for _ in 0..frames {
            flow.tick(0.1);
            let action = flow.readiness(readiness);
            if action != FlowAction::None {
                actions.push(action);
            }
            if flow.phase() != Phase::Loading {
                break;
            }
        }
        actions
    }

    fn revealing_flow() -> GameFlow {
        let mut flow = loading_flow();
        run_load(&mut flow, 20, streamed_with_player());
        assert_eq!(flow.phase(), Phase::Revealing);
        flow
    }

    fn labels(flow: &GameFlow) -> Vec<&'static str> {
        flow.menu_view().expect("a menu is shown").labels.to_vec()
    }

    #[test]
    fn a_direct_start_can_be_held_at_the_menu_and_released_into_its_load() {
        let mut flow = GameFlow::new(Vec::new(), VISIBLE_ROWS, StartMode::Direct);
        flow.hold_at_main_menu();
        assert_eq!(flow.phase(), Phase::MainMenu);
        assert!(flow.menu_view().is_some());
        flow.release_to_loading();
        assert_eq!(flow.phase(), Phase::Loading);
    }

    #[test]
    fn releasing_outside_the_menu_changes_nothing() {
        let mut flow = loading_flow();
        flow.release_to_loading();
        assert_eq!(flow.phase(), Phase::Loading);
        let mut flow = spawn_select_flow();
        flow.release_to_loading();
        assert_eq!(flow.phase(), Phase::SpawnSelect);
    }

    #[test]
    fn a_menu_start_opens_the_main_menu() {
        let flow = menu_flow();
        assert_eq!(flow.phase(), Phase::MainMenu);
        assert_eq!(labels(&flow), ["$PLAY", "$QUIT"]);
    }

    #[test]
    fn a_direct_start_goes_straight_to_loading() {
        let flow = GameFlow::new(Vec::new(), VISIBLE_ROWS, StartMode::Direct);
        assert_eq!(flow.phase(), Phase::Loading);
        assert_eq!(flow.loading_seconds(), Some(0.0));
    }

    #[test]
    fn play_opens_the_spawn_picker_with_the_locations_and_a_final_back() {
        let flow = spawn_select_flow();
        assert_eq!(
            labels(&flow),
            ["ONE", "TWO", "THREE", "FOUR", "FIVE", "$BACK"]
        );
    }

    #[test]
    fn quit_asks_the_caller_to_quit() {
        let mut flow = menu_flow();
        flow.handle(FlowEvent::NavDown);
        assert_eq!(flow.handle(FlowEvent::Activate), FlowAction::Quit);
    }

    #[test]
    fn escape_in_the_main_menu_does_nothing() {
        let mut flow = menu_flow();
        assert_eq!(flow.handle(FlowEvent::Back), FlowAction::None);
        assert_eq!(flow.phase(), Phase::MainMenu);
    }

    #[test]
    fn back_in_the_spawn_picker_returns_to_the_main_menu() {
        let mut flow = spawn_select_flow();
        assert_eq!(flow.handle(FlowEvent::Back), FlowAction::None);
        assert_eq!(flow.phase(), Phase::MainMenu);
    }

    #[test]
    fn the_back_row_returns_to_the_main_menu() {
        let mut flow = spawn_select_flow();
        flow.handle(FlowEvent::NavUp);
        let view = flow.menu_view().unwrap();
        assert_eq!(view.labels[view.selected], "$BACK");
        assert_eq!(flow.handle(FlowEvent::Activate), FlowAction::None);
        assert_eq!(flow.phase(), Phase::MainMenu);
    }

    #[test]
    fn the_spawn_picker_opens_at_its_first_row_every_time() {
        let mut flow = spawn_select_flow();
        flow.handle(FlowEvent::NavDown);
        flow.handle(FlowEvent::NavDown);
        flow.handle(FlowEvent::Back);
        settle(&mut flow);
        flow.handle(FlowEvent::Activate);
        let view = flow.menu_view().unwrap();
        assert_eq!((view.selected, view.first_visible), (0, 0));
    }

    #[test]
    fn activating_a_location_begins_play_there_and_starts_loading() {
        let mut flow = spawn_select_flow();
        flow.handle(FlowEvent::NavDown);
        let action = flow.handle(FlowEvent::Activate);
        assert_eq!(action, FlowAction::BeginPlay(location("TWO")));
        assert_eq!(flow.phase(), Phase::Loading);
        assert_eq!(flow.loading_seconds(), Some(0.0));
        assert_eq!(flow.overlay_alpha(), 1.0);
    }

    #[test]
    fn navigation_wraps_and_scrolls_the_spawn_picker_window() {
        let mut flow = spawn_select_flow();
        flow.handle(FlowEvent::NavUp);
        let view = flow.menu_view().unwrap();
        assert_eq!((view.selected, view.first_visible), (5, 3));
        flow.handle(FlowEvent::NavDown);
        let view = flow.menu_view().unwrap();
        assert_eq!((view.selected, view.first_visible), (0, 0));
    }

    #[test]
    fn hovering_a_row_selects_it_and_a_missing_row_changes_nothing() {
        let mut flow = spawn_select_flow();
        flow.handle(FlowEvent::RowHovered(2));
        assert_eq!(flow.menu_view().unwrap().selected, 2);
        flow.handle(FlowEvent::RowHovered(99));
        assert_eq!(flow.menu_view().unwrap().selected, 2);
    }

    #[test]
    fn a_click_on_a_row_activates_it() {
        let mut flow = spawn_select_flow();
        assert_eq!(click(&mut flow, 1), FlowAction::BeginPlay(location("TWO")));
    }

    #[test]
    fn clicking_play_opens_the_spawn_picker() {
        let mut flow = menu_flow();
        click(&mut flow, 0);
        assert_eq!(flow.phase(), Phase::SpawnSelect);
    }

    #[test]
    fn pressing_a_row_alone_selects_it_but_activates_nothing() {
        let mut flow = spawn_select_flow();
        assert_eq!(flow.handle(FlowEvent::RowPressed(1)), FlowAction::None);
        assert_eq!(flow.phase(), Phase::SpawnSelect);
        assert_eq!(flow.menu_view().unwrap().selected, 1);
    }

    #[test]
    fn releasing_over_another_row_than_the_pressed_one_activates_nothing() {
        let mut flow = spawn_select_flow();
        flow.handle(FlowEvent::RowPressed(1));
        let action = flow.handle(FlowEvent::RowReleased(Some(2)));
        assert_eq!(action, FlowAction::None);
        assert_eq!(flow.phase(), Phase::SpawnSelect);
    }

    #[test]
    fn releasing_off_the_list_cancels_the_press() {
        let mut flow = spawn_select_flow();
        flow.handle(FlowEvent::RowPressed(1));
        flow.handle(FlowEvent::RowReleased(None));
        assert_eq!(
            flow.handle(FlowEvent::RowReleased(Some(1))),
            FlowAction::None
        );
    }

    #[test]
    fn a_release_without_a_press_in_this_phase_activates_nothing() {
        let mut flow = menu_flow();
        flow.handle(FlowEvent::RowPressed(0));
        flow.handle(FlowEvent::Activate);
        assert_eq!(flow.phase(), Phase::SpawnSelect);
        settle(&mut flow);

        let action = flow.handle(FlowEvent::RowReleased(Some(0)));

        assert_eq!(action, FlowAction::None);
        assert_eq!(flow.phase(), Phase::SpawnSelect);
    }

    #[test]
    fn the_second_click_of_a_double_click_on_play_does_not_pick_the_first_location() {
        let mut flow = menu_flow();
        click(&mut flow, 0);
        assert_eq!(flow.phase(), Phase::SpawnSelect);
        flow.tick(0.016);

        let second = click(&mut flow, 0);

        assert_eq!(second, FlowAction::None);
        assert_eq!(flow.phase(), Phase::SpawnSelect);
    }

    #[test]
    fn a_press_inside_the_debounce_is_dropped_even_when_released_after_it() {
        let mut flow = menu_flow();
        click(&mut flow, 0);
        flow.handle(FlowEvent::RowPressed(0));
        settle(&mut flow);
        assert_eq!(
            flow.handle(FlowEvent::RowReleased(Some(0))),
            FlowAction::None
        );
        assert_eq!(flow.phase(), Phase::SpawnSelect);
    }

    #[test]
    fn a_click_after_the_debounce_activates() {
        let mut flow = menu_flow();
        click(&mut flow, 0);
        settle(&mut flow);
        assert!(matches!(click(&mut flow, 0), FlowAction::BeginPlay(_)));
    }

    #[test]
    fn activation_is_ignored_until_the_debounce_after_a_phase_change_has_passed() {
        let mut flow = GameFlow::new(locations(), VISIBLE_ROWS, StartMode::Menu);
        assert_eq!(flow.handle(FlowEvent::Activate), FlowAction::None);
        assert_eq!(flow.phase(), Phase::MainMenu);

        let frames_to_wait = (ACTIVATION_DEBOUNCE_SECONDS / MAX_FRAME_STEP_SECONDS) as u32 - 1;
        for _ in 0..frames_to_wait {
            flow.tick(MAX_FRAME_STEP_SECONDS);
        }
        assert_eq!(flow.handle(FlowEvent::Activate), FlowAction::None);
        assert_eq!(flow.phase(), Phase::MainMenu);

        settle(&mut flow);
        flow.handle(FlowEvent::Activate);
        assert_eq!(flow.phase(), Phase::SpawnSelect);
    }

    #[test]
    fn a_held_enter_cannot_activate_through_the_screen_it_opened() {
        let mut flow = menu_flow();
        flow.handle(FlowEvent::Activate);
        assert_eq!(flow.phase(), Phase::SpawnSelect);
        flow.tick(0.016);
        assert_eq!(flow.handle(FlowEvent::Activate), FlowAction::None);
        assert_eq!(flow.phase(), Phase::SpawnSelect);
    }

    #[test]
    fn a_long_frame_does_not_use_up_more_than_one_capped_step_of_the_debounce() {
        let mut flow = GameFlow::new(locations(), VISIBLE_ROWS, StartMode::Menu);
        flow.tick(30.0);
        assert_eq!(flow.handle(FlowEvent::Activate), FlowAction::None);
    }

    #[test]
    fn navigation_is_not_debounced() {
        let mut flow = GameFlow::new(locations(), VISIBLE_ROWS, StartMode::Menu);
        flow.handle(FlowEvent::NavDown);
        assert_eq!(flow.menu_view().unwrap().selected, 1);
        assert_eq!(flow.handle(FlowEvent::Back), FlowAction::None);
    }

    #[test]
    fn pressing_a_missing_row_does_nothing() {
        let mut flow = menu_flow();
        assert_eq!(flow.handle(FlowEvent::RowPressed(7)), FlowAction::None);
        assert_eq!(
            flow.handle(FlowEvent::RowReleased(Some(7))),
            FlowAction::None
        );
        assert_eq!(flow.phase(), Phase::MainMenu);
    }

    #[test]
    fn a_row_beyond_the_back_row_does_nothing_and_the_back_row_goes_back() {
        let mut flow = spawn_select_flow();
        assert_eq!(flow.activate_spawn_row(99), FlowAction::None);
        assert_eq!(flow.phase(), Phase::SpawnSelect);
        assert_eq!(click(&mut flow, locations().len()), FlowAction::None);
        assert_eq!(flow.phase(), Phase::MainMenu);
    }

    #[test]
    fn the_wheel_scrolls_the_window_and_clamps_at_both_ends() {
        let mut flow = spawn_select_flow();
        flow.handle(FlowEvent::Wheel(-100));
        let view = flow.menu_view().unwrap();
        assert_eq!((view.first_visible, view.selected), (3, 3));
        flow.handle(FlowEvent::Wheel(100));
        assert_eq!(flow.menu_view().unwrap().first_visible, 0);
    }

    fn full_picker_flow() -> GameFlow {
        let names = ["LOCATION"; 13].map(location);
        let mut flow = GameFlow::new(names.into(), 9, StartMode::Menu);
        settle(&mut flow);
        flow.handle(FlowEvent::Activate);
        settle(&mut flow);
        flow
    }

    #[test]
    fn the_window_follows_the_selection_down_to_back_in_a_full_picker() {
        let mut flow = full_picker_flow();
        for _ in 0..13 {
            flow.handle(FlowEvent::NavDown);
        }
        let view = flow.menu_view().unwrap();
        assert_eq!((view.selected, view.first_visible), (13, 5));
        assert_eq!(view.labels[view.selected], "$BACK");
        assert!(view.selected < view.first_visible + 9);
    }

    #[test]
    fn the_wheel_reaches_back_in_a_full_picker() {
        let mut flow = full_picker_flow();
        flow.scroll(-5.0);
        let view = flow.menu_view().unwrap();
        assert_eq!(view.first_visible, 5);
        assert_eq!(view.labels[view.first_visible + 8], "$BACK");
    }

    #[test]
    fn the_most_negative_wheel_step_scrolls_without_overflowing() {
        let mut flow = spawn_select_flow();
        flow.handle(FlowEvent::Wheel(i32::MIN));
        assert_eq!(flow.menu_view().unwrap().first_visible, 3);
        flow.handle(FlowEvent::Wheel(i32::MAX));
        assert_eq!(flow.menu_view().unwrap().first_visible, 0);
    }

    #[test]
    fn small_wheel_deltas_scroll_only_once_they_add_up_to_a_whole_line() {
        let mut flow = spawn_select_flow();
        for _ in 0..3 {
            flow.scroll(-0.25);
            assert_eq!(flow.menu_view().unwrap().first_visible, 0);
        }
        flow.scroll(-0.25);
        assert_eq!(flow.menu_view().unwrap().first_visible, 1);
        flow.scroll(-0.25);
        assert_eq!(flow.menu_view().unwrap().first_visible, 1);
    }

    #[test]
    fn a_wheel_remainder_does_not_carry_into_the_next_phase() {
        let mut flow = spawn_select_flow();
        flow.scroll(-0.75);
        flow.handle(FlowEvent::Back);
        settle(&mut flow);
        flow.handle(FlowEvent::Activate);
        assert_eq!(flow.phase(), Phase::SpawnSelect);
        flow.scroll(-0.5);
        assert_eq!(flow.menu_view().unwrap().first_visible, 0);
    }

    #[test]
    fn the_wheel_outside_a_list_does_nothing() {
        let mut flow = loading_flow();
        flow.scroll(-5.0);
        assert_eq!(flow.phase(), Phase::Loading);
    }

    #[test]
    fn menu_input_during_loading_is_ignored() {
        let mut flow = loading_flow();
        for event in [FlowEvent::Back, FlowEvent::Activate, FlowEvent::NavDown] {
            assert_eq!(flow.handle(event), FlowAction::None);
            assert_eq!(flow.phase(), Phase::Loading);
        }
    }

    #[test]
    fn loading_releases_the_fold_hold_once_then_reveals_and_locks_the_cursor() {
        let mut flow = loading_flow();
        let actions = run_load(&mut flow, 10, streamed_with_player());
        assert_eq!(
            actions,
            [FlowAction::ReleaseFoldHold, FlowAction::LockCursor]
        );
        assert_eq!(flow.phase(), Phase::Revealing);
    }

    #[test]
    fn loading_waits_for_the_stream_even_when_the_player_is_ready() {
        let mut flow = loading_flow();
        let player_only = Readiness {
            player_ready: true,
            ..Readiness::default()
        };
        assert!(run_load(&mut flow, 50, player_only).is_empty());
        assert_eq!(flow.phase(), Phase::Loading);
    }

    #[test]
    fn readiness_outside_loading_is_ignored() {
        let mut flow = spawn_select_flow();
        assert_eq!(flow.readiness(streamed_with_player()), FlowAction::None);
        assert_eq!(flow.phase(), Phase::SpawnSelect);
    }

    #[test]
    fn the_overlay_stays_opaque_until_revealing_then_fades_to_the_game() {
        let mut flow = revealing_flow();
        assert_eq!(flow.overlay_alpha(), 1.0);
        let mut previous = 1.0;
        for _ in 0..200 {
            flow.tick(0.016);
            let alpha = flow.overlay_alpha();
            assert!((0.0..=1.0).contains(&alpha));
            assert!(alpha <= previous);
            previous = alpha;
            if flow.phase() == Phase::InGame {
                break;
            }
        }
        assert_eq!(flow.phase(), Phase::InGame);
        assert_eq!(flow.overlay_alpha(), 0.0);
        assert_eq!(flow.loading_seconds(), None);
    }

    #[test]
    fn the_loading_clock_counts_from_the_load_and_runs_on_through_the_reveal() {
        let mut flow = loading_flow();
        let mut frames = 0.0;
        while flow.phase() == Phase::Loading && frames < 200.0 {
            flow.tick(0.1);
            flow.readiness(if frames < 5.0 {
                Readiness::default()
            } else {
                streamed_with_player()
            });
            frames += 1.0;
        }
        assert_eq!(flow.phase(), Phase::Revealing);
        let at_reveal = flow.loading_seconds().unwrap();
        assert!(
            (at_reveal - 0.1 * frames).abs() < 1e-4,
            "{frames} frames of 0.1 s, not restarted by the reveal: {at_reveal}"
        );
        flow.tick(0.25);
        assert!((flow.loading_seconds().unwrap() - at_reveal - 0.25).abs() < 1e-4);
    }

    #[test]
    fn the_loading_clock_is_not_slowed_by_the_frame_step_cap() {
        let mut flow = loading_flow();
        flow.tick(MAX_FRAME_STEP_SECONDS * 10.0);
        let seconds = flow.loading_seconds().unwrap();
        assert!((seconds - f64::from(MAX_FRAME_STEP_SECONDS * 10.0)).abs() < 1e-6);
    }

    #[test]
    fn a_negative_or_nan_frame_time_does_not_move_the_loading_clock() {
        let mut flow = loading_flow();
        flow.tick(-1.0);
        flow.tick(f32::NAN);
        assert_eq!(flow.loading_seconds(), Some(0.0));
    }

    #[test]
    fn the_loading_clock_stops_after_the_reveal_and_restarts_with_the_next_load() {
        let mut flow = GameFlow::new(Vec::new(), VISIBLE_ROWS, StartMode::Direct);
        flow.tick(1.5);
        assert_eq!(flow.loading_seconds(), Some(1.5));
        flow.hold_at_main_menu();
        assert_eq!(
            flow.loading_seconds(),
            None,
            "the menu is not the loading screen"
        );
        flow.tick(2.0);
        flow.release_to_loading();
        assert_eq!(
            flow.loading_seconds(),
            Some(0.0),
            "a new load starts at zero"
        );
    }

    #[test]
    fn gameplay_input_the_hud_and_the_debug_window_follow_the_phase() {
        let flow = menu_flow();
        let gating = |flow: &GameFlow| {
            (
                flow.gameplay_input_enabled(),
                flow.hud_visible(),
                flow.developer_window_visible(),
            )
        };
        assert_eq!(gating(&flow), (false, false, false));
        let mut flow = revealing_flow();
        assert_eq!(gating(&flow), (true, true, false));
        for _ in 0..100 {
            flow.tick(0.05);
        }
        assert_eq!(gating(&flow), (true, true, true));
    }

    #[test]
    fn toasts_wait_out_the_loading_screen_and_the_reveal() {
        let toasts = |flow: &GameFlow| flow.toasts_visible();
        let flow = menu_flow();
        assert!(toasts(&flow), "main menu");
        let mut flow = spawn_select_flow();
        assert!(toasts(&flow), "spawn picker");
        flow.handle(FlowEvent::Activate);
        assert_eq!(flow.phase(), Phase::Loading);
        assert!(!toasts(&flow), "loading");
        let mut flow = revealing_flow();
        assert!(!toasts(&flow), "revealing");
        for _ in 0..100 {
            flow.tick(0.05);
        }
        assert!(toasts(&flow), "in game");
    }

    #[test]
    fn a_direct_start_loads_without_a_menu_and_reveals() {
        let mut flow = GameFlow::new(Vec::new(), VISIBLE_ROWS, StartMode::Direct);
        let actions = run_load(&mut flow, 20, streamed_with_player());
        assert_eq!(
            actions,
            [FlowAction::ReleaseFoldHold, FlowAction::LockCursor]
        );
    }
}
