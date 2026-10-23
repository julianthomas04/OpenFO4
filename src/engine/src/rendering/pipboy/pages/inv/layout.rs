//! Everything INV reads from the layout documents, resolved once.

use super::card_layout::{CARD_KEYS, CardLayout, MOVIE};
use super::doll::{DOLL_KEYS, DollLayout};
use super::entry_text::{CountSuffix, CountSuffixKeys, EntryIconKeys, EntryIcons, icon_keys};
use super::favourites::{FAVOURITES_KEYS, FavouritesLayout};
use super::native;
use super::quantity::{QUANTITY_KEYS, QuantityLayout};
use crate::rendering::pipboy::scrolling_list::{ListKeys, ListLayout, list_keys};
use crate::rendering::pipboy::shapes::OutlinedShape;
use crate::sim::pipboy::inventory::SortMode;
use crate::ui_layout::{
    LiteralKey, ModuleKeys, NumberKey, ShapeKey, TextKey, UiLayout, UiLayoutError, key_check_fields,
};
use egui::{Pos2, Rect, Vec2};

pub(crate) static LIST: ListKeys = list_keys!(
    movie: "Pipboy_InvPage",
    list: "List_mc",
    set_prop: "Pipboy_InvPage.__setProp_List_mc_Scene1_List_0",
    entry: "InvListEntry",
    border: "@1",
    arrows: "@1",
);

pub(crate) static COMPONENT_LIST: ListKeys = list_keys!(
    movie: "Pipboy_InvPage",
    list: "ComponentList_mc",
    set_prop: "Pipboy_InvPage.__setProp_ComponentList_mc_Scene1_ComponentView_0",
    member: "ComponentList_mc",
    entry: "ComponentListEntry",
    border: "@1",
    arrows: "@1",
);

pub(crate) static OWNER_LIST: ListKeys = list_keys!(
    movie: "Pipboy_InvPage",
    list: "ComponentOwnersList_mc",
    set_prop: "Pipboy_InvPage.__setProp_ComponentOwnersList_mc_Scene1_ComponentView_0",
    member: "ComponentOwnersList_mc",
    entry: "ComponentOwnersListEntry",
    border: "@1",
    arrows: "@1",
);

const ICON_GAP: LiteralKey =
    LiteralKey::new(MOVIE, "InvListEntry.SetEntryText#(FavIcon_mc.width/2)+", 2);
const ICON_LEAD: LiteralKey = LiteralKey::new(
    MOVIE,
    "InvListEntry.SetEntryText#(textField.getLineMetrics().width+textField.x)+",
    1,
);

static ITEM_ICONS: EntryIconKeys = EntryIconKeys {
    text: TextKey::new(MOVIE, "[InvListEntry]/textField"),
    legendary: Some(icon_keys!(MOVIE, "[InvListEntry]/LegendaryIcon_mc")),
    favourite: Some(icon_keys!(MOVIE, "[InvListEntry]/FavIcon_mc")),
    search: icon_keys!(MOVIE, "[InvListEntry]/SearchIcon_mc"),
    equip: ShapeKey::new(MOVIE, "[InvListEntry]/EquipIcon_mc/@1"),
    gap: ICON_GAP,
    lead: ICON_LEAD,
};

static COMPONENT_ICONS: EntryIconKeys = EntryIconKeys {
    text: TextKey::new(MOVIE, "[ComponentListEntry]/textField"),
    legendary: None,
    favourite: None,
    search: icon_keys!(MOVIE, "[ComponentListEntry]/SearchIcon_mc"),
    equip: ShapeKey::new(MOVIE, "[ComponentListEntry]/EquipIcon_mc/@1"),
    gap: ICON_GAP,
    lead: ICON_LEAD,
};

static COUNT_SUFFIX: CountSuffixKeys = CountSuffixKeys::new(
    LiteralKey::texts(MOVIE, "InvListEntry.SetEntryText#+$1.count", 1),
    LiteralKey::texts(MOVIE, "InvListEntry.SetEntryText#(\" (\"+$1.count)+", 1),
);

macro_rules! sort_text {
    ($index:literal) => {
        LiteralKey::texts(
            MOVIE,
            concat!("Pipboy_InvPage.<init>#SortText=[", $index, "]"),
            1,
        )
    };
}

struct HintKeys {
    labels: LiteralKey,
    sort: [LiteralKey; 7],
    component_toggle: LiteralKey,
    favourite: LiteralKey,
}

key_check_fields!(HintKeys {
    labels,
    sort,
    component_toggle,
    favourite,
});

static HINT_KEYS: HintKeys = HintKeys {
    labels: LiteralKey::texts(MOVIE, "Pipboy_InvPage.<init>#new BSButtonHintData(0)", 10),
    sort: [
        sort_text!(0),
        sort_text!(1),
        sort_text!(2),
        sort_text!(3),
        sort_text!(4),
        sort_text!(5),
        sort_text!(6),
    ],
    component_toggle: LiteralKey::texts(
        MOVIE,
        "Pipboy_InvPage.SetButtons#ComponentToggleButton.ButtonText=",
        2,
    ),
    favourite: LiteralKey::texts(MOVIE, "Pipboy_InvPage.SetButtons#FavButton.ButtonText=", 2),
};

static DIM: ShapeKey = ShapeKey::new(MOVIE, "ModalFadeRect_mc/ModalFadeInputCatcher/@1");

static DROP_THRESHOLD: NumberKey =
    NumberKey::new(MOVIE, "Pipboy_InvPage.DROP_ITEM_COUNT_THRESHOLD");

fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    InventoryLayout::resolve(layout).map(drop)
}

pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[
        &LIST,
        &COMPONENT_LIST,
        &OWNER_LIST,
        &ITEM_ICONS,
        &COMPONENT_ICONS,
        &COUNT_SUFFIX,
        &CARD_KEYS,
        &DOLL_KEYS,
        &QUANTITY_KEYS,
        &FAVOURITES_KEYS,
        &HINT_KEYS,
        &DIM,
        &DROP_THRESHOLD,
        &native::LABEL_KEYS,
    ],
    resolve: validate,
};

/// The hint labels, translated.
#[derive(Debug, Clone)]
pub struct HintLabels {
    pub inspect: String,
    pub drop: String,
    pub favourite: String,
    pub cycle_damage: String,
    pub accept: String,
    pub cancel: String,
    pub component_view: String,
    pub item_view: String,
    pub tag_for_search: String,
    sort: [String; 7],
}

impl HintLabels {
    pub fn sort(&self, mode: SortMode) -> &str {
        &self.sort[match mode {
            SortMode::Alphabetical => 0,
            SortMode::Damage => 1,
            SortMode::RateOfFire => 2,
            SortMode::Range => 3,
            SortMode::Accuracy => 4,
            SortMode::Value => 5,
            SortMode::Weight => 6,
        }]
    }

    fn resolve(
        layout: &UiLayout,
        rules: &crate::ui_layout::set_text::SetTextRules,
    ) -> Result<Self, UiLayoutError> {
        let keys = &HINT_KEYS;
        let shown = |key: &LiteralKey, index: usize| -> Result<String, UiLayoutError> {
            Ok(rules.apply(layout, &layout.lit_string(key, index)?, false, false))
        };
        let label = |index| shown(&keys.labels, index);
        let sort = |index: usize| shown(&keys.sort[index], 0);
        Ok(Self {
            inspect: label(2)?,
            drop: label(3)?,
            favourite: label(4)?,
            cycle_damage: label(5)?,
            accept: label(6)?,
            cancel: label(7)?,
            component_view: label(8)?,
            item_view: shown(&keys.component_toggle, 0)?,
            tag_for_search: shown(&keys.favourite, 0)?,
            sort: [
                sort(0)?,
                sort(1)?,
                sort(2)?,
                sort(3)?,
                sort(4)?,
                sort(5)?,
                sort(6)?,
            ],
        })
    }
}

/// INV's layout: its three lists, entry decorations, item card, paper doll, prompts and hints.
#[derive(Debug, Clone)]
pub struct InventoryLayout {
    pub items: ListLayout,
    pub components: ListLayout,
    pub owners: ListLayout,
    pub item_icons: EntryIcons,
    pub component_icons: EntryIcons,
    pub suffix: CountSuffix,
    pub card: CardLayout,
    pub doll: DollLayout,
    pub quantity: QuantityLayout,
    pub favourites: FavouritesLayout,
    pub hints: HintLabels,
    /// The layer over the page while a prompt is open.
    pub dim: OutlinedShape,
    /// Where the pointer scrolls the owners list.
    pub owners_area: Rect,
    pub drop_threshold: u32,
}

impl InventoryLayout {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let items = ListLayout::resolve(layout, &LIST)?;
        let rules = items.rules.clone();
        let threshold = layout.get(&DROP_THRESHOLD)?;
        let owners = ListLayout::resolve(layout, &OWNER_LIST)?;
        let owners_origin = layout.get(&OWNER_LIST.list)?.world.matrix;
        let [left, _, right, _] = layout.get(&OWNER_LIST.border_bounds)?;
        let owners_area = Rect::from_min_size(
            Pos2::new(owners_origin.tx, owners_origin.ty),
            Vec2::new(
                right - left,
                owners.entry_height() * owners.max_items() as f32,
            ),
        );
        Ok(Self {
            components: ListLayout::resolve(layout, &COMPONENT_LIST)?,
            owners,
            owners_area,
            item_icons: EntryIcons::resolve(layout, &ITEM_ICONS)?,
            component_icons: EntryIcons::resolve(layout, &COMPONENT_ICONS)?,
            suffix: COUNT_SUFFIX.resolve(layout)?,
            card: CardLayout::resolve(layout)?,
            doll: DollLayout::resolve(layout)?,
            quantity: QuantityLayout::resolve(layout, &rules)?,
            favourites: FavouritesLayout::resolve(layout, &rules)?,
            hints: HintLabels::resolve(layout, &rules)?,
            dim: OutlinedShape::resolve(layout, &DIM)?,
            drop_threshold: threshold as u32,
            items,
        })
    }
}
