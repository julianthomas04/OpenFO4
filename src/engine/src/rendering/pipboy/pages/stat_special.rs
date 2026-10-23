//! STAT / SPECIAL: the seven attributes with the Vault Boy and description of the selected one.

use egui::Pos2;

use super::vault_boy;
use crate::assets::pipboy_art::FixedArt;
use crate::rendering::pipboy::scaleform_text::{FieldSpec, TextAutoSize};
use crate::rendering::pipboy::scrolling_list::{
    ListEvent, ListKeys, ListLayout, ListState, ListView, list_keys,
};
use crate::rendering::pipboy::shapes::{Shade, code_colour};
use crate::rendering::pipboy::tab::{BottomBar, InputResult, NavInput, Tab, TabContext};
use crate::sim::pipboy::Special;
use crate::sim::pipboy::character::SpecialValue;
use crate::ui_layout::{
    LiteralKey, ModuleKeys, PlacementKey, TextKey, UiLayout, UiLayoutError, key_check_fields,
};

const MOVIE: &str = "Pipboy_StatsPage";

pub(crate) static LIST: ListKeys = list_keys!(
    movie: "Pipboy_StatsPage",
    list: "SPECIALTab_mc/List_mc",
    set_prop: "SPECIALTab.__setProp_List_mc_SPECIALTab_List_0",
    entry: "SPECIALListEntry",
    border: "@2",
    arrows: none,
);

struct SpecialKeys {
    value: TextKey,
    marks: LiteralKey,
    value_colours: LiteralKey,
    description: TextKey,
    art: PlacementKey,
}

static KEYS: SpecialKeys = SpecialKeys {
    value: TextKey::new(MOVIE, "[SPECIALListEntry]/Value_tf"),
    marks: LiteralKey::texts(MOVIE, "SPECIALListEntry.SetEntryText#$3+", 2),
    value_colours: LiteralKey::new(
        MOVIE,
        "SPECIALListEntry.SetEntryText#Value_tf.textColor=",
        2,
    ),
    description: TextKey::new(MOVIE, "SPECIALTab_mc/Description_tf"),
    art: PlacementKey::new(MOVIE, "SPECIALTab_mc/VBHolder_mc"),
};

key_check_fields!(SpecialKeys {
    value,
    marks,
    value_colours,
    description,
    art
});

fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    SpecialLayout::resolve(layout).map(drop)
}

pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[&LIST, &KEYS],
    resolve: validate,
};

/// The value cell's markers for a lowered and a raised attribute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValueMarks {
    pub lowered: String,
    pub raised: String,
}

impl ValueMarks {
    /// The value cell text: the marker of a modified attribute, then its value.
    pub fn text(&self, value: SpecialValue) -> String {
        let total = (i16::from(value.base) + i16::from(value.bonus)).max(0);
        let mark = match value.bonus.signum() {
            -1 => self.lowered.as_str(),
            1 => self.raised.as_str(),
            _ => "",
        };
        format!("{mark}{total}")
    }
}

pub struct SpecialLayout {
    list: ListLayout,
    value: FieldSpec,
    marks: ValueMarks,
    value_shades: [Shade; 2],
    description: FieldSpec,
    art: Pos2,
}

impl SpecialLayout {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let art = layout.get(&KEYS.art)?.world.matrix;
        Ok(Self {
            list: ListLayout::resolve(layout, &LIST)?,
            value: FieldSpec::resolve(layout, &KEYS.value, TextAutoSize::None)?,
            marks: ValueMarks {
                lowered: layout.lit_string(&KEYS.marks, 0)?,
                raised: layout.lit_string(&KEYS.marks, 1)?,
            },
            value_shades: [
                code_colour(layout, &KEYS.value_colours, 0)?,
                code_colour(layout, &KEYS.value_colours, 1)?,
            ],
            description: FieldSpec::resolve(layout, &KEYS.description, TextAutoSize::Shrink)?,
            art: Pos2::new(art.tx, art.ty),
        })
    }
}

pub struct SpecialPage {
    layout: SpecialLayout,
    list: ListState,
    art_started: f64,
}

impl SpecialPage {
    pub fn new(layout: SpecialLayout) -> Self {
        Self {
            layout,
            list: ListState::default(),
            art_started: 0.0,
        }
    }

    fn view(&self) -> ListView<'_> {
        ListView::new(&self.layout.list, Special::ALL.len())
    }
}

impl Tab for SpecialPage {
    fn handle_input(&mut self, input: NavInput, cx: &mut TabContext<'_>) -> InputResult {
        match input {
            NavInput::Up
            | NavInput::Down
            | NavInput::Scrolled(_)
            | NavInput::PointerMoved(_)
            | NavInput::Clicked(_) => {
                let view = ListView::new(&self.layout.list, Special::ALL.len());
                if self.list.handle(input, &view) == ListEvent::Moved {
                    self.art_started = cx.seconds_open;
                }
            }
            _ => return InputResult::Ignored,
        }
        InputResult::Consumed
    }

    fn draw(&mut self, cx: &mut TabContext<'_>) {
        let model = cx.snapshot.model;
        let view = ListView::new(&self.layout.list, Special::ALL.len());
        if self.list.selected().is_none() {
            self.list.select(&view, self.list.scroll());
        }
        let layout = &self.layout;
        let [selected_shade, unselected_shade] = layout.value_shades;
        let texts = &model.catalog.special_texts;
        let tint = cx.tint;
        let ui_layout = cx.layout;
        self.list
            .paint(cx.painter, tint, &self.view(), |painter, row| {
                let special = Special::ALL[row.index];
                let name = layout.list.entry_text(ui_layout, &texts.get(special).name);
                layout.list.paint_text(painter, row, &name);
                let value_shade = if row.selected {
                    selected_shade
                } else {
                    unselected_shade
                };
                layout.value.translated(row.origin).paint_coloured(
                    painter,
                    &layout.marks.text(model.character.special.get(special)),
                    value_shade.colour(tint),
                );
            });
        let Some(selected) = self.list.selected().map(|index| Special::ALL[index]) else {
            return;
        };
        let library = cx.art_library;
        vault_boy::draw(
            cx,
            library.fixed(FixedArt::SpecialVaultBoy(selected)),
            layout.art,
            self.art_started,
        );
        let description =
            layout
                .list
                .rules
                .apply(cx.layout, &texts.get(selected).description, false, false);
        layout.description.paint(cx.painter, &description, cx.tint);
    }

    fn bottom_bar(&self) -> BottomBar {
        BottomBar::Stat
    }

    fn select_row(&mut self, row: usize) {
        let view = ListView::new(&self.layout.list, Special::ALL.len());
        self.list.select(&view, row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks() -> ValueMarks {
        ValueMarks {
            lowered: "v ".to_owned(),
            raised: "^ ".to_owned(),
        }
    }

    #[test]
    fn a_plain_rank_shows_its_number_and_a_bonus_marks_the_total() {
        let marks = marks();
        assert_eq!(marks.text(SpecialValue { base: 6, bonus: 0 }), "6");
        assert_eq!(marks.text(SpecialValue { base: 6, bonus: 2 }), "^ 8");
        assert_eq!(marks.text(SpecialValue { base: 3, bonus: -1 }), "v 2");
        assert_eq!(marks.text(SpecialValue { base: 1, bonus: -5 }), "v 0");
    }
}
