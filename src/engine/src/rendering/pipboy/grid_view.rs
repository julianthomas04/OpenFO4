//! The shell's own hint, added after every page's hints: it opens the perk chart and reads
//! "LEVEL UP (n)", flashing, while perk points are unspent.

use super::tab::{ButtonHint, HintKey};
use crate::ui_layout::{LiteralKey, ModuleKeys, UiLayout, UiLayoutError, key_check_fields};

const MOVIE: &str = "PipboyMenu";

struct GridViewKeys {
    label: LiteralKey,
    level_up_open: LiteralKey,
    level_up_close: LiteralKey,
    points_above: LiteralKey,
}

static KEYS: GridViewKeys = GridViewKeys {
    label: LiteralKey::texts(MOVIE, "PipboyMenu.<init>#new BSButtonHintData(0)", 1),
    level_up_open: LiteralKey::texts(
        MOVIE,
        "PipboyMenu.InvalidatePartialData#+DataObj.PerkPoints",
        1,
    ),
    level_up_close: LiteralKey::texts(
        MOVIE,
        "PipboyMenu.InvalidatePartialData#(\"$$LEVELUP (\"+DataObj.PerkPoints)+",
        1,
    ),
    points_above: LiteralKey::new(
        MOVIE,
        "PipboyMenu.InvalidatePartialData#DataObj.PerkPoints>",
        2,
    ),
};

key_check_fields!(GridViewKeys {
    label,
    level_up_open,
    level_up_close,
    points_above,
});

fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    GridViewHint::resolve(layout).map(drop)
}

pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[&KEYS],
    resolve: validate,
};

/// The perk-chart hint's texts as the shell builds them.
#[derive(Debug, Clone, PartialEq)]
pub struct GridViewHint {
    label: String,
    level_up_open: String,
    level_up_close: String,
    points_above: u32,
}

impl GridViewHint {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let points_above = layout.lit_number(&KEYS.points_above, 0)?;
        if points_above < 0.0 || points_above.fract() != 0.0 {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                KEYS.points_above.path,
                "not a whole number",
            ));
        }
        Ok(Self {
            label: layout.lit_string(&KEYS.label, 0)?,
            level_up_open: layout.lit_string(&KEYS.level_up_open, 0)?,
            level_up_close: layout.lit_string(&KEYS.level_up_close, 0)?,
            points_above: points_above as u32,
        })
    }

    /// The hint with `perk_points` unspent; its label is a translate key the hint bar resolves.
    pub fn hint(&self, perk_points: u32) -> ButtonHint {
        if perk_points > self.points_above {
            let label = format!("{}{perk_points}{}", self.level_up_open, self.level_up_close);
            ButtonHint::new(HintKey::T, label).flashing(true)
        } else {
            ButtonHint::new(HintKey::T, self.label.clone())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_view() -> GridViewHint {
        GridViewHint {
            label: "$FIXTURE_CHART".to_owned(),
            level_up_open: "$$FIXTURE_RISE <".to_owned(),
            level_up_close: ">".to_owned(),
            points_above: 0,
        }
    }

    #[test]
    fn unspent_points_turn_the_hint_into_a_flashing_level_up() {
        let plain = grid_view().hint(0);
        assert_eq!(
            (plain.label.as_str(), plain.flashing),
            ("$FIXTURE_CHART", false)
        );
        let level_up = grid_view().hint(2);
        assert_eq!(
            (level_up.label.as_str(), level_up.flashing),
            ("$$FIXTURE_RISE <2>", true)
        );
    }
}
