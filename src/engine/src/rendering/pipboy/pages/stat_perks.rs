//! STAT / PERKS: the owned perks with their rank stars, Vault Boy and rank description.

use egui::Pos2;

use super::vault_boy;
use crate::assets::pipboy_art::Icon;
use crate::rendering::pipboy::scaleform_text::{FieldSpec, TextAutoSize};
use crate::rendering::pipboy::scrolling_list::{
    ListEvent, ListKeys, ListLayout, ListState, ListView, list_keys,
};
use crate::rendering::pipboy::tab::{
    BottomBar, ButtonHint, HintKey, InputResult, NavInput, Tab, TabContext,
};
use crate::sim::pipboy::{PerkDefinition, PipBoyModel, PipBoySnapshot};
use crate::ui_layout::{
    BoundsKey, LiteralKey, ModuleKeys, PlacementKey, TextKey, UiLayout, UiLayoutError,
    key_check_fields,
};

const MOVIE: &str = "Pipboy_StatsPage";

pub(crate) static LIST: ListKeys = list_keys!(
    movie: "Pipboy_StatsPage",
    list: "PerksTab_mc/List_mc",
    set_prop: "PerksTab.__setProp_List_mc_PerksTab_List_0",
    entry: "PerksListEntry",
    border: "@2",
    arrows: "@1",
);

struct PerksKeys {
    rank_above: LiteralKey,
    rank_open: LiteralKey,
    rank_close: LiteralKey,
    description: TextKey,
    art: PlacementKey,
    stars: PlacementKey,
    star: BoundsKey,
    star_pitch: LiteralKey,
    star_divisor: LiteralKey,
    stars_above: LiteralKey,
    previous_above: LiteralKey,
    hint_labels: LiteralKey,
}

static KEYS: PerksKeys = PerksKeys {
    rank_above: LiteralKey::new(MOVIE, "PerksListEntry.SetEntryText#$1.rank>", 1),
    rank_open: LiteralKey::texts(MOVIE, "PerksListEntry.SetEntryText#+$1.rank", 1),
    rank_close: LiteralKey::texts(MOVIE, "PerksListEntry.SetEntryText#(\" (\"+$1.rank)+", 1),
    description: TextKey::new(MOVIE, "PerksTab_mc/Description_tf"),
    art: PlacementKey::new(MOVIE, "PerksTab_mc/VBHolder_mc"),
    stars: PlacementKey::new(MOVIE, "PerksTab_mc/StarHolder_mc"),
    star: BoundsKey::new(MOVIE, "[PerkRankStar]"),
    star_pitch: LiteralKey::new(MOVIE, "PerksTab.redrawUIComponent#*$3", 1),
    star_divisor: LiteralKey::new(MOVIE, "PerksTab.redrawUIComponent#StarHolder_mc.width/", 1),
    stars_above: LiteralKey::new(MOVIE, "PerksTab.redrawUIComponent#_MaxSelectedRank>", 1),
    previous_above: LiteralKey::new(MOVIE, "PerksTab.redrawUIComponent#_CurrSelectedRank>", 1),
    hint_labels: LiteralKey::texts(MOVIE, "PerksTab.<init>#new BSButtonHintData(0)", 2),
};

key_check_fields!(PerksKeys {
    rank_above,
    rank_open,
    rank_close,
    description,
    art,
    stars,
    star,
    star_pitch,
    star_divisor,
    stars_above,
    previous_above,
    hint_labels,
});

fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    PerksLayout::resolve(layout).map(drop)
}

pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[&LIST, &KEYS],
    resolve: validate,
};

/// The row text pieces: the rank follows the name only past `above`.
#[derive(Debug, Clone, PartialEq)]
pub struct RankSuffix {
    pub above: u8,
    pub open: String,
    pub close: String,
}

impl RankSuffix {
    /// `shown` (the entry's name as its field shows it) with the rank appended when it is past
    /// one.
    pub fn label(&self, shown: &str, rank: u8) -> String {
        if rank > self.above {
            format!("{shown}{}{rank}{}", self.open, self.close)
        } else {
            shown.to_owned()
        }
    }
}

/// The rank stars: a row of `max_rank` stars centred on the holder's authored x, the first
/// `rank` full, shown only for perks with more than `above` ranks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StarRow {
    pub anchor: Pos2,
    pub pitch: f32,
    pub divisor: f32,
    pub star_width: f32,
    pub above: u8,
}

impl StarRow {
    /// Where each star's registration point goes.
    pub fn positions(&self, max_rank: u8) -> Vec<Pos2> {
        if max_rank <= self.above {
            return Vec::new();
        }
        let count = f32::from(max_rank);
        let width = self.pitch * (count - 1.0) + self.star_width;
        let holder_x = self.anchor.x - width / self.divisor;
        (0..max_rank)
            .map(|i| Pos2::new(holder_x + self.pitch * f32::from(i), self.anchor.y))
            .collect()
    }
}

pub struct PerksLayout {
    list: ListLayout,
    suffix: RankSuffix,
    description: FieldSpec,
    art: Pos2,
    stars: StarRow,
    previous_above: u8,
    previous_label: String,
    next_label: String,
}

fn small_whole(layout: &UiLayout, key: &LiteralKey) -> Result<u8, UiLayoutError> {
    u8::try_from(layout.lit_whole(key, 0)?)
        .map_err(|_| UiLayoutError::unresolved(key.movie, key.path, "not a small whole number"))
}

impl PerksLayout {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let art = layout.get(&KEYS.art)?.world.matrix;
        let stars = layout.get(&KEYS.stars)?.world.matrix;
        let [star_left, _, star_right, _] = layout.get(&KEYS.star)?;
        Ok(Self {
            list: ListLayout::resolve(layout, &LIST)?,
            suffix: RankSuffix {
                above: small_whole(layout, &KEYS.rank_above)?,
                open: layout.lit_string(&KEYS.rank_open, 0)?,
                close: layout.lit_string(&KEYS.rank_close, 0)?,
            },
            description: FieldSpec::resolve(layout, &KEYS.description, TextAutoSize::Shrink)?,
            art: Pos2::new(art.tx, art.ty),
            stars: StarRow {
                anchor: Pos2::new(stars.tx, stars.ty),
                pitch: layout.lit_number(&KEYS.star_pitch, 0)? as f32,
                divisor: layout.lit_number(&KEYS.star_divisor, 0)? as f32,
                star_width: star_right - star_left,
                above: small_whole(layout, &KEYS.stars_above)?,
            },
            previous_above: small_whole(layout, &KEYS.previous_above)?,
            previous_label: layout.lit_string(&KEYS.hint_labels, 0)?,
            next_label: layout.lit_string(&KEYS.hint_labels, 1)?,
        })
    }
}

/// Owned perks in the game's `sortOn("text")` order: by UTF-16 code units, case-sensitive.
pub fn owned_perks(model: &PipBoyModel) -> Vec<&PerkDefinition> {
    let mut owned: Vec<&PerkDefinition> = model
        .catalog
        .perks
        .iter()
        .filter(|perk| model.perks.rank(&perk.id) > 0 && perk.is_available_to(model.character.sex))
        .collect();
    owned.sort_by(|a, b| a.name.encode_utf16().cmp(b.name.encode_utf16()));
    owned
}

/// The highest rank number a perk has; a perk without rank data counts as one rank.
fn highest_rank(perk: &PerkDefinition) -> u8 {
    u8::try_from(perk.ranks.len()).unwrap_or(u8::MAX).max(1)
}

pub struct PerksPage {
    layout: PerksLayout,
    list: ListState,
    /// Rank whose description and stars are shown, `1`-based; reset to the owned rank when the
    /// selection changes.
    viewed_rank: Option<u8>,
    art_started: f64,
}

impl PerksPage {
    pub fn new(layout: PerksLayout) -> Self {
        Self {
            layout,
            list: ListState::default(),
            viewed_rank: None,
            art_started: 0.0,
        }
    }

    fn selected<'a>(&self, model: &'a PipBoyModel) -> Option<&'a PerkDefinition> {
        let owned = owned_perks(model);
        owned.get(self.list.selected()?).copied()
    }

    fn rank_shown(&self, model: &PipBoyModel, perk: &PerkDefinition) -> u8 {
        self.viewed_rank
            .unwrap_or_else(|| model.perks.rank(&perk.id))
            .clamp(1, highest_rank(perk))
    }
}

impl Tab for PerksPage {
    fn handle_input(&mut self, input: NavInput, cx: &mut TabContext<'_>) -> InputResult {
        let model = cx.snapshot.model;
        match input {
            NavInput::Hint(key @ (HintKey::Ctrl | HintKey::Alt)) => {
                if let Some(perk) = self.selected(model) {
                    let shown = self.rank_shown(model, perk);
                    let target = if key == HintKey::Ctrl {
                        shown.saturating_sub(1)
                    } else {
                        shown + 1
                    };
                    self.viewed_rank = Some(target.clamp(1, highest_rank(perk)));
                }
            }
            NavInput::Up
            | NavInput::Down
            | NavInput::Scrolled(_)
            | NavInput::PointerMoved(_)
            | NavInput::Clicked(_) => {
                let view = ListView::new(&self.layout.list, owned_perks(model).len());
                if self.list.handle(input, &view) == ListEvent::Moved {
                    self.viewed_rank = None;
                    self.art_started = cx.seconds_open;
                }
            }
            _ => return InputResult::Ignored,
        }
        InputResult::Consumed
    }

    fn draw(&mut self, cx: &mut TabContext<'_>) {
        let snapshot = *cx.snapshot;
        let model = snapshot.model;
        let owned = owned_perks(model);
        let view = ListView::new(&self.layout.list, owned.len());
        self.list.invalidate(&view);
        if self.list.selected().is_none() && !owned.is_empty() {
            self.list.select(&view, self.list.scroll());
        }
        let layout = &self.layout;
        let ui_layout = cx.layout;
        self.list.paint(cx.painter, cx.tint, &view, |painter, row| {
            let perk = owned[row.index];
            let shown = layout.list.entry_text(ui_layout, &perk.name);
            let label = layout.suffix.label(&shown, model.perks.rank(&perk.id));
            layout.list.paint_text(painter, row, &label);
        });
        self.list.paint_arrows(cx, &view);
        let Some(perk) = self.selected(model) else {
            return;
        };
        let library = cx.art_library;
        if let Some(clip) = perk
            .vault_boy_swf
            .as_deref()
            .and_then(|swf| library.clip_for_swf(swf))
        {
            vault_boy::draw(cx, clip, self.layout.art, self.art_started);
        }
        let shown = self.rank_shown(model, perk);
        let white = cx.shade(1.0);
        let stars = self.layout.stars.positions(highest_rank(perk));
        for (i, at) in stars.into_iter().enumerate() {
            let icon = if i < usize::from(shown) {
                Icon::StarFilled
            } else {
                Icon::StarEmpty
            };
            cx.draw_icon(icon, at, 1.0, white);
        }
        if let Some(rank) = perk.ranks.get(usize::from(shown) - 1) {
            let text = self
                .layout
                .list
                .rules
                .apply(cx.layout, &rank.description, false, false);
            self.layout.description.paint(cx.painter, &text, cx.tint);
        }
    }

    fn hints(&self, snapshot: &PipBoySnapshot<'_>) -> Vec<ButtonHint> {
        let model = snapshot.model;
        let (previous, next) = self.selected(model).map_or((false, false), |perk| {
            let shown = self.rank_shown(model, perk);
            (
                shown > self.layout.previous_above,
                shown < highest_rank(perk),
            )
        });
        vec![
            ButtonHint::new(HintKey::Ctrl, self.layout.previous_label.clone()).enabled(previous),
            ButtonHint::new(HintKey::Alt, self.layout.next_label.clone()).enabled(next),
        ]
    }

    fn bottom_bar(&self) -> BottomBar {
        BottomBar::Stat
    }

    fn select_row(&mut self, row: usize) {
        let view = ListView::new(&self.layout.list, usize::MAX);
        self.list.select(&view, row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::pipboy::perks::PerkRank;
    use crate::sim::pipboy::{PerkId, PipBoyCatalog};
    use std::sync::Arc;

    fn perk(id: &str, name: &str, ranks: usize) -> PerkDefinition {
        PerkDefinition {
            id: PerkId(id.to_owned()),
            name: name.to_owned(),
            ranks: (0..ranks)
                .map(|n| PerkRank {
                    form_key: format!("{id}{n}"),
                    description: String::new(),
                    required_level: 1,
                })
                .collect(),
            vault_boy_swf: None,
            chart: None,
            sex: None,
        }
    }

    fn model() -> PipBoyModel {
        let catalog = PipBoyCatalog {
            perks: vec![
                perk("a", "Zealot", 3),
                perk("b", "armorer", 1),
                perk("c", "Unowned", 2),
            ],
            ..PipBoyCatalog::default()
        };
        let mut model = PipBoyModel::new(Arc::new(catalog));
        model.perks.0.insert(PerkId("a".into()), 2);
        model.perks.0.insert(PerkId("b".into()), 1);
        model
    }

    fn stars() -> StarRow {
        StarRow {
            anchor: Pos2::new(621.0, 418.85),
            pitch: 37.5,
            divisor: 2.0,
            star_width: 35.0,
            above: 1,
        }
    }

    #[test]
    fn owned_perks_sort_by_code_units_with_capitals_first() {
        let model = model();
        let names: Vec<_> = owned_perks(&model)
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, ["Zealot", "armorer"]);
    }

    #[test]
    fn a_perk_of_the_other_sex_is_not_listed_even_when_owned() {
        use crate::sim::pipboy::Sex;
        let mut model = model();
        let mut female_only = perk("f", "Lady Killer", 1);
        female_only.sex = Some(Sex::Female);
        let mut catalog = (*model.catalog).clone();
        catalog.perks.push(female_only);
        model.catalog = Arc::new(catalog);
        model.perks.0.insert(PerkId("f".into()), 1);

        model.character.sex = Sex::Male;
        assert!(owned_perks(&model).iter().all(|p| p.name != "Lady Killer"));
        model.character.sex = Sex::Female;
        assert!(owned_perks(&model).iter().any(|p| p.name == "Lady Killer"));
    }

    #[test]
    fn the_rank_follows_the_name_only_past_the_first_rank() {
        let suffix = RankSuffix {
            above: 1,
            open: " (".to_owned(),
            close: ")".to_owned(),
        };
        assert_eq!(suffix.label("Zealot", 1), "Zealot");
        assert_eq!(suffix.label("Zealot", 2), "Zealot (2)");
    }

    #[test]
    fn stars_are_centred_on_the_holder_by_their_whole_width() {
        let xs: Vec<_> = stars().positions(3).iter().map(|p| p.x).collect();
        let width = 2.0 * 37.5 + 35.0;
        assert!((xs[0] - (621.0 - width / 2.0)).abs() < 1e-4);
        assert!((xs[2] - xs[0] - 75.0).abs() < 1e-4);
    }

    #[test]
    fn a_single_rank_perk_has_no_stars() {
        assert_eq!(stars().positions(1), []);
    }

    #[test]
    fn a_perk_without_rank_data_counts_as_one_rank() {
        assert_eq!(highest_rank(&perk("x", "Rankless", 0)), 1);
    }
}
