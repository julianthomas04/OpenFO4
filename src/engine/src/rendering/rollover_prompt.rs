//! What the HUD's rollover widget shows for the aimed object: its name and, beside the key hint,
//! a verb.

use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RolloverPrompt {
    pub name: Arc<str>,
    /// `None` for a prompt that is only a name, with no key hint.
    pub verb: Option<Verb>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Verb {
    pub text: Arc<str>,
    pub tone: VerbTone,
}

impl Verb {
    pub(super) fn normal(text: Arc<str>) -> Self {
        Self {
            text,
            tone: VerbTone::Normal,
        }
    }

    pub(super) fn warning(text: Arc<str>) -> Self {
        Self {
            text,
            tone: VerbTone::Warning,
        }
    }
}

/// How the whole prompt is coloured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VerbTone {
    /// The HUD colour.
    Normal,
    /// The colour of an action the game flags as a theft.
    Warning,
}
