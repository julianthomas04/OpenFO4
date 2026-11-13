//! The texts shown when the Pip-Boy refuses an action (a drop), from game settings, and the
//! route of a refusal into the HUD message list.

use std::collections::HashMap;
use std::sync::Arc;

use super::game_text::required_nonblank_text;
use crate::assets::db::DbClient;
use crate::sim::SimWorld;
use crate::sim::pipboy::PipBoyActionError;
use crate::startup::StartupError;

/// The text of every setting a refusal names. A missing or blank setting stops startup.
pub(super) fn load(db: &DbClient) -> Result<HashMap<&'static str, Arc<str>>, StartupError> {
    read(db).map_err(StartupError::RefusalTextSettings)
}

fn read(db: &DbClient) -> anyhow::Result<HashMap<&'static str, Arc<str>>> {
    PipBoyActionError::REFUSAL_SETTINGS
        .into_iter()
        .map(|setting| Ok((setting, required_nonblank_text(db, setting)?)))
        .collect()
}

/// Turns the Pip-Boy's rejections into the HUD message lines the game shows for them.
pub(super) struct RefusalMessages {
    texts: HashMap<&'static str, Arc<str>>,
    shown_count: u64,
}

impl RefusalMessages {
    pub(super) fn new(texts: HashMap<&'static str, Arc<str>>) -> Self {
        Self {
            texts,
            shown_count: 0,
        }
    }

    /// [`next`](Self::next) for the rejection the sim's Pip-Boy model holds now.
    pub(super) fn next_of(&mut self, sim: &SimWorld) -> Option<Arc<str>> {
        let snapshot = sim.pipboy_snapshot();
        self.next(
            snapshot.model.rejection_count,
            snapshot.model.last_rejection.as_ref(),
        )
    }

    /// The line for the newest rejection, once per rejection (an identical one counted again shows
    /// again). `None` for a rejection already shown, and for one whose text is not a game setting
    /// (the Pip-Boy shows or logs it itself). A setting whose text was never loaded is reported.
    pub(super) fn next(
        &mut self,
        rejection_count: u64,
        last: Option<&PipBoyActionError>,
    ) -> Option<Arc<str>> {
        if rejection_count == self.shown_count {
            return None;
        }
        self.shown_count = rejection_count;
        let setting = last?.refusal_setting()?;
        let text = self.texts.get(setting).cloned();
        if text.is_none() {
            tracing::error!("refusal text '{setting}' was never loaded");
        }
        text
    }
}

/// Applies the actions one by one and lets `after` see the state after each, so a rejection is
/// read before the next action can replace it: every rejection of a flush reaches `after`, in
/// order.
pub(super) fn apply_each<S, A>(
    state: &mut S,
    actions: impl IntoIterator<Item = A>,
    mut apply: impl FnMut(&mut S, A),
    mut after: impl FnMut(&S),
) {
    for action in actions {
        apply(state, action);
        after(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::db::test_support::database_with;
    use crate::sim::pipboy::{ItemId, PipBoyAction, PipBoyCatalog, PipBoyModel};

    const KEY_TEXT: &str = "Cannot do that with a key";
    const JUMP_TEXT: &str = "Cannot do that in mid-air";

    fn settings(key: &str, jump: &str) -> String {
        format!(
            "INSERT INTO game_settings (form_key, editor_id, value_type, value_string)
             VALUES ('1:T.esm', 'sNoKeyDropWarning', 'string', '{key}'),
                    ('2:T.esm', 'sNoJumpWarning', 'string', '{jump}');"
        )
    }

    #[test]
    fn every_setting_a_refusal_names_is_loaded_with_its_text() {
        let texts = load(&database_with(&settings(KEY_TEXT, JUMP_TEXT))).unwrap();
        assert_eq!(texts.len(), PipBoyActionError::REFUSAL_SETTINGS.len());
        assert_eq!(texts["sNoKeyDropWarning"].as_ref(), KEY_TEXT);
        assert_eq!(texts["sNoJumpWarning"].as_ref(), JUMP_TEXT);
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the converted database"]
    fn the_installed_database_has_every_refusal_text() {
        let db = crate::assets::db::test_support::installed_database();
        let texts = load(&db).unwrap();
        assert_eq!(texts.len(), PipBoyActionError::REFUSAL_SETTINGS.len());
    }

    fn messages() -> RefusalMessages {
        load(&database_with(&settings(KEY_TEXT, JUMP_TEXT)))
            .map(RefusalMessages::new)
            .unwrap()
    }

    #[test]
    fn a_new_rejection_with_a_game_setting_gives_its_text_once() {
        let mut messages = messages();
        let error = PipBoyActionError::KeyNotDroppable(ItemId(4));
        assert_eq!(messages.next(1, Some(&error)).as_deref(), Some(KEY_TEXT));
        assert_eq!(messages.next(1, Some(&error)), None);
    }

    #[test]
    fn an_identical_rejection_counted_again_shows_its_text_again() {
        let mut messages = messages();
        let error = PipBoyActionError::Airborne;
        assert_eq!(messages.next(1, Some(&error)).as_deref(), Some(JUMP_TEXT));
        assert_eq!(messages.next(2, Some(&error)).as_deref(), Some(JUMP_TEXT));
    }

    #[test]
    fn a_rejection_without_a_game_setting_or_without_a_rejection_gives_no_line() {
        let mut messages = messages();
        let unknown = PipBoyActionError::UnknownItem(ItemId(9));
        assert_eq!(messages.next(1, Some(&unknown)), None);
        assert_eq!(messages.next(2, None), None);
    }

    #[test]
    fn a_setting_whose_text_was_never_loaded_gives_no_line() {
        let mut messages = RefusalMessages::new(HashMap::new());
        assert_eq!(messages.next(1, Some(&PipBoyActionError::Airborne)), None);
    }

    enum Step {
        Reject(PipBoyActionError),
        Accept,
    }

    fn lines_of_one_flush(steps: Vec<Step>) -> Vec<String> {
        let mut messages = messages();
        let mut model = PipBoyModel::new(Arc::new(PipBoyCatalog::default()));
        let mut lines = Vec::new();
        apply_each(
            &mut model,
            steps,
            |model, step| match step {
                Step::Reject(error) => model.reject(error),
                Step::Accept => {
                    model.submit(PipBoyAction::TurnOffRadio);
                }
            },
            |model| {
                let line = messages.next(model.rejection_count, model.last_rejection.as_ref());
                lines.extend(line.map(|line| line.to_string()));
            },
        );
        lines
    }

    #[test]
    fn two_refusals_of_one_flush_both_reach_the_list_in_order() {
        let lines = lines_of_one_flush(vec![
            Step::Reject(PipBoyActionError::KeyNotDroppable(ItemId(4))),
            Step::Reject(PipBoyActionError::Airborne),
        ]);
        assert_eq!(lines, [KEY_TEXT, JUMP_TEXT]);
    }

    #[test]
    fn a_refusal_followed_by_an_accepted_action_in_one_flush_is_not_lost() {
        let lines = lines_of_one_flush(vec![
            Step::Reject(PipBoyActionError::Airborne),
            Step::Accept,
        ]);
        assert_eq!(lines, [JUMP_TEXT]);
    }

    #[test]
    fn an_accepted_action_alone_gives_no_line() {
        assert_eq!(lines_of_one_flush(vec![Step::Accept]), Vec::<String>::new());
    }

    #[test]
    fn a_missing_setting_stops_startup_naming_it() {
        let only_key = "INSERT INTO game_settings (form_key, editor_id, value_type, value_string)
             VALUES ('1:T.esm', 'sNoKeyDropWarning', 'string', 'text');";
        let error = load(&database_with(only_key)).unwrap_err();
        assert!(matches!(error, StartupError::RefusalTextSettings(_)));
        assert!(error.to_string().contains("sNoJumpWarning"), "{error}");
    }

    #[test]
    fn a_blank_setting_stops_startup_naming_it() {
        let error = load(&database_with(&settings(KEY_TEXT, "  "))).unwrap_err();
        assert!(matches!(error, StartupError::RefusalTextSettings(_)));
        assert!(error.to_string().contains("sNoJumpWarning"), "{error}");
    }
}
