//! The loading screen on demand: a screen picked from the debug menu covers the world and its
//! model, pointer and keys work as during a load, until it is closed. The world underneath keeps
//! running and nothing of it changes, so closing returns to exactly where the player was.

use winit::keyboard::KeyCode;

use super::host::ArtStatus;
use super::keys::art_key;
use crate::rendering::loading_screen::PanKey;
use crate::assets::db::load_screens::{LoadScreen, LoadScreenCatalog};
use crate::game_flow::{OverlayLayers, Phase};
use crate::random::RandomSource;
use crate::rendering::State;

/// Where the logo's animation stands in a pinned preview, which holds still for comparisons.
const PINNED_LOGO_SECONDS: f64 = 0.0;
const PREVIEW_LAYERS: OverlayLayers = OverlayLayers {
    art: true,
    panel: true,
};

/// Which screen a preview shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::rendering) enum ScreenRequest {
    /// The screen with this form key or editor id, whatever its case.
    Named(String),
    Random,
}

#[derive(Debug, thiserror::Error)]
pub(in crate::rendering) enum PreviewError {
    #[error("the loading screens are unavailable: {0}")]
    CatalogUnavailable(String),
    #[error("no loading screen is named {0}")]
    UnknownScreen(String),
    #[error("the database holds no loading screen")]
    EmptyCatalog,
    #[error("the loading screen can only be opened while the world is shown")]
    WorldNotShown,
    #[error("a loading screen is already open")]
    AlreadyOpen,
    #[error("loading screen {label} cannot be shown: {error}")]
    Load { label: String, error: String },
}

/// A screen as the debug menu lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::rendering) struct ScreenEntry {
    pub form_key: String,
    /// What the list shows and the filter searches.
    pub text: String,
}

impl ScreenEntry {
    pub fn of(screen: &LoadScreen) -> Self {
        let text = match &screen.editor_id {
            Some(editor_id) => format!("{editor_id} ({})", screen.form_key),
            None => screen.form_key.clone(),
        };
        Self {
            form_key: screen.form_key.clone(),
            text,
        }
    }
}

/// The entries whose text contains `filter`, whatever its case; all of them for a blank filter.
pub(in crate::rendering) fn matching_entries<'a>(
    entries: &'a [ScreenEntry],
    filter: &str,
) -> Vec<&'a ScreenEntry> {
    let needle = filter.trim().to_lowercase();
    entries
        .iter()
        .filter(|entry| entry.text.to_lowercase().contains(&needle))
        .collect()
}

/// Whether a preview may open now: in play, with none open.
fn check_can_open(phase: Phase, preview_open: bool) -> Result<(), PreviewError> {
    if preview_open {
        Err(PreviewError::AlreadyOpen)
    } else if phase != Phase::InGame {
        Err(PreviewError::WorldNotShown)
    } else {
        Ok(())
    }
}

/// What a key does while a preview is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreviewKey {
    Close,
    Pan(PanKey),
    Ignore,
}

/// Esc, pressed and not repeated, closes the preview; the pan keys move the model; every other key
/// does nothing.
fn preview_key(code: KeyCode, is_pressed: bool, is_repeat: bool) -> PreviewKey {
    if code == KeyCode::Escape {
        if is_pressed && !is_repeat {
            PreviewKey::Close
        } else {
            PreviewKey::Ignore
        }
    } else {
        art_key(code).map_or(PreviewKey::Ignore, PreviewKey::Pan)
    }
}

/// The screen `request` asks for.
pub(super) fn requested_screen<'a>(
    catalog: &'a LoadScreenCatalog,
    request: &ScreenRequest,
    rng: &mut impl RandomSource,
) -> Result<&'a LoadScreen, PreviewError> {
    match request {
        ScreenRequest::Named(name) => {
            find_screen(catalog, name).ok_or_else(|| PreviewError::UnknownScreen(name.clone()))
        }
        ScreenRequest::Random => {
            if catalog.screens.is_empty() {
                return Err(PreviewError::EmptyCatalog);
            }
            Ok(&catalog.screens[rng.below(catalog.screens.len())])
        }
    }
}

fn find_screen<'a>(catalog: &'a LoadScreenCatalog, name: &str) -> Option<&'a LoadScreen> {
    catalog.screens.iter().find(|screen| {
        screen.form_key.eq_ignore_ascii_case(name)
            || screen
                .editor_id
                .as_deref()
                .is_some_and(|editor_id| editor_id.eq_ignore_ascii_case(name))
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Open {
    seconds: f64,
    /// The model's camera is put at its start once the model is shown, and the logo holds still.
    pinned: bool,
    pin_due: bool,
}

/// What the debug menu keeps of the loading screen preview.
#[derive(Debug, Default)]
pub(in crate::rendering) struct LoadScreenPreview {
    open: Option<Open>,
    pub filter: String,
    /// Whether the next preview is pinned.
    pub pin_camera: bool,
    pub entries: Option<Vec<ScreenEntry>>,
    pub error: Option<String>,
    /// Whether the cursor was locked when the preview opened, to restore on close.
    cursor_was_locked: bool,
}

impl LoadScreenPreview {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    fn begin(&mut self, pinned: bool, cursor_was_locked: bool) {
        self.open = Some(Open {
            seconds: 0.0,
            pinned,
            pin_due: pinned,
        });
        self.error = None;
        self.cursor_was_locked = cursor_was_locked;
    }

    /// Ends the preview; answers whether the cursor was locked before it, `None` if none was open.
    fn end(&mut self) -> Option<bool> {
        self.open.take().map(|_| self.cursor_was_locked)
    }

    /// Lets `seconds` pass; a pinned preview's clock holds still.
    fn advance(&mut self, seconds: f32) {
        if let Some(open) = &mut self.open
            && !open.pinned
            && seconds.is_finite()
            && seconds > 0.0
        {
            open.seconds += f64::from(seconds);
        }
    }

    /// Seconds on the logo's clock while a preview is open.
    pub fn loading_seconds(&self) -> Option<f64> {
        self.open.map(|open| {
            if open.pinned {
                PINNED_LOGO_SECONDS
            } else {
                open.seconds
            }
        })
    }

    /// Whether the camera is to be pinned now: once, as soon as the model is shown.
    fn take_pin(&mut self, model_shown: bool) -> bool {
        match &mut self.open {
            Some(open) if open.pin_due && model_shown => {
                open.pin_due = false;
                true
            }
            _ => false,
        }
    }
}

/// What the loading overlay draws this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::rendering) struct OverlayView {
    pub alpha: f32,
    pub layers: OverlayLayers,
    pub loading_seconds: Option<f64>,
}

impl State {
    /// The overlay of the preview while one is open, else the flow's.
    pub(in crate::rendering) fn overlay_view(&self) -> OverlayView {
        match self.load_screens.preview.loading_seconds() {
            Some(seconds) => OverlayView {
                alpha: 1.0,
                layers: PREVIEW_LAYERS,
                loading_seconds: Some(seconds),
            },
            None => OverlayView {
                alpha: self.flow.overlay_alpha(),
                layers: self.flow.overlay_layers(),
                loading_seconds: self.flow.loading_seconds(),
            },
        }
    }

    pub(in crate::rendering) fn loading_screen_preview_open(&self) -> bool {
        self.load_screens.preview.is_open()
    }

    /// Shows the screen `request` names over the world, replacing a preview already open.
    pub(in crate::rendering) fn open_loading_screen_preview(
        &mut self,
        request: &ScreenRequest,
        pinned: bool,
    ) -> Result<(), PreviewError> {
        check_can_open(self.flow.phase(), self.load_screens.preview.is_open())?;
        let chosen = self.load_screens.host.choose_requested(request)?;
        let label = chosen.label.clone();
        self.load_screens
            .host
            .show(chosen)
            .map_err(|error| PreviewError::Load {
                label,
                error: error.to_string(),
            })?;
        let was_locked = self.mouse_capture.is_locked();
        self.load_screens.preview.begin(pinned, was_locked);
        self.mouse_capture.set_locked(false);
        self.mouse_capture.apply(&self.window);
        self.debug_ui.release_keyboard_focus();
        self.release_held_input();
        Ok(())
    }

    /// Opens what the debug menu asked for, with the camera pinned as its checkbox says; a
    /// failure stays on the menu.
    pub(in crate::rendering) fn open_requested_loading_screen(&mut self, request: &ScreenRequest) {
        let pinned = self.load_screens.preview.pin_camera;
        if let Err(error) = self.open_loading_screen_preview(request, pinned) {
            tracing::error!("{error}");
            self.load_screens.preview.error = Some(error.to_string());
        }
    }

    /// Takes the preview down and gives the cursor back as it was.
    pub(in crate::rendering) fn close_loading_screen_preview(&mut self) {
        let Some(was_locked) = self.load_screens.preview.end() else {
            return;
        };
        self.load_screens.host.clear();
        self.load_screens.drag.release();
        self.mouse_capture.set_locked(was_locked);
        self.mouse_capture.apply(&self.window);
    }

    /// Follows the preview's model load and lets `seconds` pass for it.
    pub(super) fn sync_preview(&mut self, seconds: f32) {
        self.load_screens.host.tick(seconds);
        self.load_screens.preview.advance(seconds);
        if let ArtStatus::Failed { label, error } = self.load_screens.host.poll() {
            let message = PreviewError::Load {
                label,
                error: error.to_string(),
            }
            .to_string();
            tracing::error!("{message}");
            self.close_loading_screen_preview();
            self.load_screens.preview.error = Some(message);
        }
        let model_shown = self.load_screens.host.has_art();
        if self.load_screens.preview.take_pin(model_shown) {
            self.load_screens.host.pin_camera();
        }
    }

    /// Applies what `preview_key` makes of the key.
    pub(in crate::rendering) fn handle_preview_key(
        &mut self,
        code: KeyCode,
        is_pressed: bool,
        is_repeat: bool,
    ) {
        match preview_key(code, is_pressed, is_repeat) {
            PreviewKey::Close => self.close_loading_screen_preview(),
            PreviewKey::Pan(key) => self.load_screens.host.key(key, is_pressed, is_repeat),
            PreviewKey::Ignore => {}
        }
    }

    /// Whether the model, its background and the panel are all on the screen.
    pub(in crate::rendering) fn loading_screen_fully_shown(&mut self) -> bool {
        self.art_shown() && self.load_screens.host.background().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::db::load_screens::ModelTransform;
    use crate::random::SplitMix64;

    fn screen(editor_id: Option<&str>, form_key: &str) -> LoadScreen {
        LoadScreen {
            form_key: form_key.to_string(),
            editor_id: editor_id.map(str::to_string),
            tip: None,
            model_path: String::new(),
            displays_in_main_menu: false,
            no_rotation: false,
            transform: ModelTransform {
                position: [0.0; 3],
                rotation_radians: [0.0; 3],
                scale: 1.0,
                around_origin: false,
                zoom_min: 0.0,
                zoom_max: 0.0,
            },
            rotation_limits: None,
            zoom_limits: None,
            conditions: Vec::new(),
        }
    }

    fn catalog() -> LoadScreenCatalog {
        LoadScreenCatalog {
            screens: vec![
                screen(Some("LSDeathclaw"), "0001:DC"),
                screen(Some("LSDog"), "0002:Dog"),
                screen(None, "0003:Anon"),
            ],
            ..LoadScreenCatalog::default()
        }
    }

    fn named(name: &str) -> ScreenRequest {
        ScreenRequest::Named(name.to_string())
    }

    #[test]
    fn a_named_request_finds_the_screen_by_editor_id_or_form_key_whatever_the_case() {
        let catalog = catalog();
        let mut rng = SplitMix64::new(1);
        for (name, expected) in [
            ("lsdog", "0002:Dog"),
            ("0001:dc", "0001:DC"),
            ("0003:ANON", "0003:Anon"),
        ] {
            let found = requested_screen(&catalog, &named(name), &mut rng).unwrap();
            assert_eq!(found.form_key, expected, "{name}");
        }
    }

    #[test]
    fn an_unknown_name_is_an_error_naming_it() {
        let mut rng = SplitMix64::new(1);
        let error = requested_screen(&catalog(), &named("Nope"), &mut rng).unwrap_err();
        assert!(matches!(error, PreviewError::UnknownScreen(name) if name == "Nope"));
    }

    #[test]
    fn a_random_request_draws_from_the_catalog_and_varies() {
        let catalog = catalog();
        let mut rng = SplitMix64::new(3);
        let drawn: Vec<_> = (0..40)
            .map(|_| {
                requested_screen(&catalog, &ScreenRequest::Random, &mut rng)
                    .unwrap()
                    .form_key
                    .clone()
            })
            .collect();
        for screen in &catalog.screens {
            assert!(drawn.contains(&screen.form_key), "{drawn:?}");
        }
    }

    #[test]
    fn a_random_request_on_an_empty_catalog_is_an_error() {
        let mut rng = SplitMix64::new(1);
        let empty = LoadScreenCatalog::default();
        assert!(matches!(
            requested_screen(&empty, &ScreenRequest::Random, &mut rng),
            Err(PreviewError::EmptyCatalog)
        ));
    }

    #[test]
    fn an_entry_shows_the_editor_id_with_the_form_key_or_the_form_key_alone() {
        let catalog = catalog();
        assert_eq!(ScreenEntry::of(&catalog.screens[0]).text, "LSDeathclaw (0001:DC)");
        assert_eq!(ScreenEntry::of(&catalog.screens[2]).text, "0003:Anon");
    }

    #[test]
    fn the_filter_matches_any_part_of_an_entry_whatever_the_case() {
        let entries: Vec<_> = catalog().screens.iter().map(ScreenEntry::of).collect();
        let keys = |filter| {
            matching_entries(&entries, filter)
                .iter()
                .map(|entry| entry.form_key.as_str())
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(""), ["0001:DC", "0002:Dog", "0003:Anon"]);
        assert_eq!(keys("  "), ["0001:DC", "0002:Dog", "0003:Anon"]);
        assert_eq!(keys("DOG"), ["0002:Dog"]);
        assert_eq!(keys("0003"), ["0003:Anon"]);
        assert_eq!(keys("zzz"), Vec::<&str>::new());
    }

    #[test]
    fn a_preview_opens_in_play_with_none_open_only() {
        assert!(check_can_open(Phase::InGame, false).is_ok());
        assert!(matches!(
            check_can_open(Phase::InGame, true),
            Err(PreviewError::AlreadyOpen)
        ));
        for phase in [
            Phase::MainMenu,
            Phase::SpawnSelect,
            Phase::Loading,
            Phase::Revealing,
        ] {
            assert!(
                matches!(
                    check_can_open(phase, false),
                    Err(PreviewError::WorldNotShown)
                ),
                "{phase:?}"
            );
        }
    }

    #[test]
    fn escape_closes_on_its_first_press_only() {
        assert_eq!(preview_key(KeyCode::Escape, true, false), PreviewKey::Close);
        assert_eq!(preview_key(KeyCode::Escape, true, true), PreviewKey::Ignore);
        assert_eq!(preview_key(KeyCode::Escape, false, false), PreviewKey::Ignore);
    }

    #[test]
    fn the_pan_keys_move_the_model_pressed_or_released_or_repeated() {
        for (is_pressed, is_repeat) in [(true, false), (false, false), (true, true)] {
            assert_eq!(
                preview_key(KeyCode::KeyA, is_pressed, is_repeat),
                PreviewKey::Pan(PanKey::Left)
            );
        }
    }

    #[test]
    fn a_gameplay_key_does_nothing_in_the_preview() {
        for code in [KeyCode::KeyE, KeyCode::Space, KeyCode::KeyF, KeyCode::Tab] {
            assert_eq!(preview_key(code, true, false), PreviewKey::Ignore, "{code:?}");
        }
    }

    #[test]
    fn a_preview_remembers_the_cursor_it_found_and_gives_it_back_once() {
        for locked in [false, true] {
            let mut preview = LoadScreenPreview::default();
            preview.begin(false, locked);
            assert!(preview.is_open());
            assert_eq!(preview.end(), Some(locked));
            assert!(!preview.is_open());
            assert_eq!(preview.end(), None, "a second close restores nothing");
        }
    }

    #[test]
    fn a_closed_preview_has_no_clock_and_a_pin_does_nothing() {
        let mut preview = LoadScreenPreview::default();
        assert!(!preview.is_open());
        assert_eq!(preview.loading_seconds(), None);
        assert!(!preview.take_pin(true));
        assert_eq!(preview.end(), None);
    }

    #[test]
    fn an_unpinned_preview_runs_its_clock_and_never_pins() {
        let mut preview = LoadScreenPreview::default();
        preview.begin(false, false);
        preview.advance(0.5);
        preview.advance(0.25);
        assert_eq!(preview.loading_seconds(), Some(0.75));
        assert!(!preview.take_pin(true));
    }

    #[test]
    fn a_step_that_is_not_a_positive_number_does_not_move_the_clock() {
        let mut preview = LoadScreenPreview::default();
        preview.begin(false, false);
        for step in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            preview.advance(step);
        }
        assert_eq!(preview.loading_seconds(), Some(0.0));
    }

    #[test]
    fn a_pinned_preview_holds_its_clock_still_and_pins_once_the_model_is_shown() {
        let mut preview = LoadScreenPreview::default();
        preview.begin(true, false);
        preview.advance(3.0);
        assert_eq!(preview.loading_seconds(), Some(PINNED_LOGO_SECONDS));
        assert!(!preview.take_pin(false), "the model is not shown yet");
        assert!(preview.take_pin(true));
        assert!(!preview.take_pin(true), "pinned once only");
    }

    #[test]
    fn opening_a_preview_forgets_the_error_of_the_last() {
        let mut preview = LoadScreenPreview {
            error: Some("old".to_string()),
            ..LoadScreenPreview::default()
        };
        preview.begin(false, false);
        assert_eq!(preview.error, None);
    }
}
