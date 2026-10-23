//! The favourites menu: twelve tiles in a cross, one per quick-key, to assign the selected item
//! to.

use egui::{Pos2, Vec2};

use super::card_model::carried;
use super::modal::{ArtPlacement, FrameKeys, ModalFrame, StrokedShape, frame_keys};
use super::native::{EMPTY_FAVOURITE_ICON_TYPE, FAVOURITE_ICON_TYPE};
use crate::assets::pipboy_art::FixedArt;
use crate::rendering::pipboy::scaleform_text::{FieldSpec, TextAutoSize};
use crate::rendering::pipboy::shapes::OutlinedShape;
use crate::rendering::pipboy::tab::{HintKey, NavInput, TabContext};
use crate::sim::pipboy::inventory::{
    FAVOURITE_SLOT_COUNT, FavouriteSlot, InventoryItem, ItemDetails,
};
use crate::sim::pipboy::{Inventory, ItemId};
use crate::ui_layout::set_text::SetTextRules;
use crate::ui_layout::{
    LiteralKey, PlacementKey, ShapeKey, TextKey, UiLayout, UiLayoutError, key_check_fields,
};

const MOVIE: &str = "Pipboy_InvPage";
const TILES: usize = FAVOURITE_SLOT_COUNT as usize;
/// The selection while no tile is selected.
const NONE: usize = TILES;

pub struct TileKeys {
    shape: ShapeKey,
    origin: PlacementKey,
}

key_check_fields!(TileKeys { shape, origin });

macro_rules! tile {
    ($index:literal) => {
        TileKeys {
            shape: ShapeKey::new(
                MOVIE,
                concat!("[FavoritesCross]/EntryHolder_mc/Entry_", $index, "/@1/@1"),
            ),
            origin: PlacementKey::new(
                MOVIE,
                concat!("[FavoritesCross]/EntryHolder_mc/Entry_", $index),
            ),
        }
    };
}

pub struct FavouritesKeys {
    frame: FrameKeys,
    cross: PlacementKey,
    centre: PlacementKey,
    tiles: [TileKeys; TILES],
    selection: ShapeKey,
    selection_clip: PlacementKey,
    icon: PlacementKey,
    quickkey: TextKey,
    quickkey_texts: LiteralKey,
    quickkey_indices: LiteralKey,
    quickkey_offset: LiteralKey,
    name: TextKey,
    ammo: TextKey,
    count_open: LiteralKey,
    count_close: LiteralKey,
    count_hidden: LiteralKey,
    ammo_open: LiteralKey,
    ammo_close: LiteralKey,
    empty_name: LiteralKey,
    empty_ammo: LiteralKey,
    close_delay: LiteralKey,
}

key_check_fields!(FavouritesKeys {
    frame,
    cross,
    centre,
    tiles,
    selection,
    selection_clip,
    icon,
    quickkey,
    quickkey_texts,
    quickkey_indices,
    quickkey_offset,
    name,
    ammo,
    count_open,
    count_close,
    count_hidden,
    ammo_open,
    ammo_close,
    empty_name,
    empty_ammo,
    close_delay,
});

pub static FAVOURITES_KEYS: FavouritesKeys = FavouritesKeys {
    frame: frame_keys!(
        "Pipboy_InvPage",
        "PipboyFavoritesMenu",
        "PipboyFavoritesMenu.<init>"
    ),
    cross: PlacementKey::new(MOVIE, "[PipboyFavoritesMenu]/Cross_mc"),
    centre: PlacementKey::new(MOVIE, "[FavoritesCross]/@1"),
    tiles: [
        tile!(0),
        tile!(1),
        tile!(2),
        tile!(3),
        tile!(4),
        tile!(5),
        tile!(6),
        tile!(7),
        tile!(8),
        tile!(9),
        tile!(10),
        tile!(11),
    ],
    selection: ShapeKey::new(MOVIE, "[FavoritesCross]/Selection_mc/@1"),
    selection_clip: PlacementKey::new(MOVIE, "[FavoritesCross]/Selection_mc"),
    icon: PlacementKey::new(MOVIE, "[FavoritesEntry]/Icon_mc"),
    quickkey: TextKey::new(MOVIE, "[FavoritesEntry]/Quickkey_tf"),
    quickkey_texts: LiteralKey::texts(
        MOVIE,
        "FavoritesEntry.redrawUIComponent#GlobalFunc.SetText(1)",
        3,
    ),
    quickkey_indices: LiteralKey::new(MOVIE, "FavoritesEntry.redrawUIComponent#_EntryIndex==", 3),
    quickkey_offset: LiteralKey::new(MOVIE, "FavoritesEntry.redrawUIComponent#_EntryIndex+", 1),
    name: TextKey::new(MOVIE, "[PipboyFavoritesMenu]/SelectionName_tf"),
    ammo: TextKey::new(MOVIE, "[PipboyFavoritesMenu]/SelectionAmmo_tf"),
    count_open: LiteralKey::texts(
        MOVIE,
        "PipboyFavoritesMenu.onFavSelection#+Cross_mc.selectedEntry.count",
        1,
    ),
    count_close: LiteralKey::texts(
        MOVIE,
        "PipboyFavoritesMenu.onFavSelection#(\" (\"+Cross_mc.selectedEntry.count)+",
        1,
    ),
    count_hidden: LiteralKey::new(
        MOVIE,
        "PipboyFavoritesMenu.onFavSelection#Cross_mc.selectedEntry.count==",
        1,
    ),
    ammo_open: LiteralKey::texts(
        MOVIE,
        "PipboyFavoritesMenu.onFavSelection#Cross_mc.selectedEntry.ammoText+",
        1,
    ),
    ammo_close: LiteralKey::texts(
        MOVIE,
        "PipboyFavoritesMenu.onFavSelection#((Cross_mc.selectedEntry.ammoText+\" (\")+Cross_mc.selectedEntry.ammoCount)+",
        1,
    ),
    empty_name: LiteralKey::texts(
        MOVIE,
        "PipboyFavoritesMenu.onFavSelection#_SelectedItemName=",
        1,
    ),
    empty_ammo: LiteralKey::texts(
        MOVIE,
        "PipboyFavoritesMenu.onFavSelection#_SelectedItemAmmo=",
        2,
    ),
    close_delay: LiteralKey::new(MOVIE, "Pipboy_InvPage.onFavEntryClick#new Timer(0)", 1),
};

/// The tile a direction key moves to from each position (the tiles, then none), as the cross's
/// code tables them.
mod moves {
    pub const UP: [usize; 13] = [8, 8, 8, 8, 8, 8, 6, 6, 7, 8, 9, 10, 8];
    pub const DOWN: [usize; 13] = [9, 9, 9, 9, 9, 9, 7, 8, 9, 10, 11, 11, 9];
    pub const LEFT: [usize; 13] = [0, 0, 1, 2, 3, 4, 2, 2, 2, 2, 2, 2, 2];
    pub const RIGHT: [usize; 13] = [1, 2, 3, 4, 5, 5, 3, 3, 3, 3, 3, 3, 3];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    fn from(self, selected: usize) -> usize {
        let table = match self {
            Direction::Up => &moves::UP,
            Direction::Down => &moves::DOWN,
            Direction::Left => &moves::LEFT,
            Direction::Right => &moves::RIGHT,
        };
        table[selected]
    }

    fn of(input: NavInput) -> Option<Self> {
        match input {
            NavInput::Up => Some(Self::Up),
            NavInput::Down => Some(Self::Down),
            NavInput::PrevSubTab => Some(Self::Left),
            NavInput::NextSubTab => Some(Self::Right),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
struct Tile {
    shape: OutlinedShape,
    origin: Vec2,
}

#[derive(Debug, Clone)]
pub struct FavouritesLayout {
    frame: ModalFrame,
    cross: Vec2,
    centre: ArtPlacement,
    tiles: Vec<Tile>,
    selection: StrokedShape,
    selection_origin: Vec2,
    icon: Vec2,
    quickkey: FieldSpec,
    quickkey_texts: Vec<String>,
    quickkey_indices: Vec<usize>,
    quickkey_offset: usize,
    name: FieldSpec,
    ammo: FieldSpec,
    count_open: String,
    count_close: String,
    count_hidden: u32,
    ammo_open: String,
    ammo_close: String,
    empty_name: String,
    empty_ammo: String,
    pub close_delay_seconds: f64,
}

fn vector(layout: &UiLayout, key: &PlacementKey) -> Result<Vec2, UiLayoutError> {
    let matrix = layout.get(key)?.world.matrix;
    Ok(Vec2::new(matrix.tx, matrix.ty))
}

impl FavouritesLayout {
    pub fn resolve(layout: &UiLayout, rules: &SetTextRules) -> Result<Self, UiLayoutError> {
        let keys = &FAVOURITES_KEYS;
        let tiles = keys
            .tiles
            .iter()
            .map(|tile| {
                Ok(Tile {
                    shape: OutlinedShape::resolve(layout, &tile.shape)?,
                    origin: vector(layout, &tile.origin)?,
                })
            })
            .collect::<Result<Vec<_>, UiLayoutError>>()?;
        let local = layout.get(&keys.icon)?.local.matrix;
        let texts = (0..3)
            .map(|index| layout.lit_string(&keys.quickkey_texts, index))
            .collect::<Result<Vec<_>, _>>()?;
        let numbered = (0..3)
            .map(|index| {
                layout
                    .lit_number(&keys.quickkey_indices, index)
                    .map(|n| n as usize)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            frame: ModalFrame::resolve(layout, &keys.frame, rules)?,
            cross: vector(layout, &keys.cross)?,
            centre: ArtPlacement::resolve(layout, &keys.centre)?,
            tiles,
            selection: StrokedShape::resolve(layout, &keys.selection)?,
            selection_origin: vector(layout, &keys.selection_clip)?,
            icon: Vec2::new(local.tx, local.ty),
            quickkey: FieldSpec::resolve(layout, &keys.quickkey, TextAutoSize::None)?,
            quickkey_texts: texts,
            quickkey_indices: numbered,
            quickkey_offset: layout.lit_number(&keys.quickkey_offset, 0)? as usize,
            name: FieldSpec::resolve(layout, &keys.name, TextAutoSize::Shrink)?,
            ammo: FieldSpec::resolve(layout, &keys.ammo, TextAutoSize::Shrink)?,
            count_open: layout.lit_string(&keys.count_open, 0)?,
            count_close: layout.lit_string(&keys.count_close, 0)?,
            count_hidden: layout.lit_number(&keys.count_hidden, 0)? as u32,
            ammo_open: layout.lit_string(&keys.ammo_open, 0)?,
            ammo_close: layout.lit_string(&keys.ammo_close, 0)?,
            empty_name: layout.lit_string(&keys.empty_name, 0)?,
            empty_ammo: layout.lit_string(&keys.empty_ammo, 0)?,
            close_delay_seconds: layout.lit_number(&keys.close_delay, 0)? / 1000.0,
        })
    }

    fn tile_at(&self, at: Pos2) -> Option<usize> {
        self.tiles
            .iter()
            .position(|tile| tile.shape.translated(self.cross).bounds().contains(at))
    }

    /// The key printed on tile `index`.
    fn quickkey_label(&self, index: usize) -> String {
        self.quickkey_indices
            .iter()
            .position(|special| *special == index)
            .map_or_else(
                || (index + self.quickkey_offset).to_string(),
                |position| self.quickkey_texts[position].clone(),
            )
    }

    /// What the input does to `state`; `favourite_key` is the key that opened the menu, which
    /// closes it again.
    pub fn handle(
        &self,
        state: &mut Favourites,
        input: NavInput,
        favourite_key: Option<HintKey>,
    ) -> FavouritesOutcome {
        if let Some(direction) = Direction::of(input) {
            state.selected = direction.from(state.selected);
            return FavouritesOutcome::Open;
        }
        match input {
            NavInput::PointerMoved(at) => {
                if let Some(tile) = self.tile_at(at) {
                    state.selected = tile;
                }
            }
            NavInput::Clicked(at) => {
                let over = self.tile_at(at);
                if let Some(tile) = over {
                    state.selected = tile;
                }
                if over.is_some() || state.selected != NONE {
                    return state.assign();
                }
            }
            NavInput::Slot(slot) if usize::from(slot) < TILES => {
                state.selected = usize::from(slot);
                return state.assign();
            }
            NavInput::Hint(HintKey::Enter | HintKey::E) if state.selected != NONE => {
                return state.assign();
            }
            NavInput::Hint(HintKey::Tab) => return FavouritesOutcome::Close,
            NavInput::Hint(key) if Some(key) == favourite_key => {
                return FavouritesOutcome::Close;
            }
            _ => {}
        }
        FavouritesOutcome::Open
    }

    /// The text under the cross for the selected tile: the item with its count when it is not
    /// one, and the ammunition it uses.
    fn selection_texts(
        &self,
        entry: Option<&InventoryItem>,
        inventory: &Inventory,
    ) -> (String, String) {
        let Some(item) = entry else {
            return (self.empty_name.clone(), self.empty_ammo.clone());
        };
        let name = if item.count == self.count_hidden {
            item.name.clone()
        } else {
            format!(
                "{}{}{}{}",
                item.name, self.count_open, item.count, self.count_close
            )
        };
        let ammo = match &item.details {
            ItemDetails::Weapon(weapon) => weapon.ammo_name.as_ref().map(|ammo| {
                format!(
                    "{ammo}{}{}{}",
                    self.ammo_open,
                    carried(inventory, ammo),
                    self.ammo_close
                )
            }),
            _ => None,
        };
        (name, ammo.unwrap_or_else(|| self.empty_ammo.clone()))
    }

    pub fn draw(
        &self,
        cx: &mut TabContext<'_>,
        state: &Favourites,
        inventory: &Inventory,
        rules: &SetTextRules,
    ) {
        self.frame.draw(cx.painter, cx.tint);
        let entries = Favourites::entries(inventory);
        let white = cx.shade(1.0);
        self.centre
            .draw(cx, FixedArt::FavouritesCentre, self.cross, white);
        for (index, tile) in self.tiles.iter().enumerate() {
            tile.shape.translated(self.cross).paint(cx.painter, cx.tint);
            let icon_type = if entries[index].is_some() {
                FAVOURITE_ICON_TYPE
            } else {
                EMPTY_FAVOURITE_ICON_TYPE
            };
            let at = (self.cross + tile.origin + self.icon).to_pos2();
            cx.draw_fixed(FixedArt::Hotkey, (icon_type - 1) as usize, at, 1.0, white);
            self.quickkey.translated(self.cross + tile.origin).paint(
                cx.painter,
                &self.quickkey_label(index),
                cx.tint,
            );
        }
        if let Some(tile) = self.tiles.get(state.selected) {
            self.selection
                .translated(self.cross + tile.origin - self.selection_origin)
                .paint_closed(cx.painter, cx.tint);
        }
        let selected = state.selected.min(NONE);
        let entry = entries.get(selected).copied().flatten();
        let (name, ammo) = self.selection_texts(entry, inventory);
        let name = rules.apply(cx.layout, &name, false, false);
        let ammo = rules.apply(cx.layout, &ammo, false, false);
        self.name.paint(cx.painter, &name, cx.tint);
        self.ammo.paint(cx.painter, &ammo, cx.tint);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FavouritesOutcome {
    Open,
    /// The item goes into the quick-key; the menu closes after a moment.
    Assign {
        item: ItemId,
        slot: FavouriteSlot,
    },
    Close,
}

/// Which tile of the cross is selected, and when the menu closes after an assignment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Favourites {
    pub item: ItemId,
    selected: usize,
    pub closing_at: Option<f64>,
}

impl Favourites {
    pub fn new(item: ItemId) -> Self {
        Self {
            item,
            selected: NONE,
            closing_at: None,
        }
    }

    pub fn selected(&self) -> Option<usize> {
        (self.selected != NONE).then_some(self.selected)
    }

    fn assign(&self) -> FavouritesOutcome {
        match u8::try_from(self.selected) {
            Ok(slot) if self.selected != NONE => FavouritesOutcome::Assign {
                item: self.item,
                slot: FavouriteSlot(slot),
            },
            _ => FavouritesOutcome::Open,
        }
    }

    /// The item on each quick-key.
    pub fn entries(inventory: &Inventory) -> [Option<&InventoryItem>; TILES] {
        std::array::from_fn(|slot| {
            inventory
                .items
                .iter()
                .find(|item| item.favourite == u8::try_from(slot).ok().map(FavouriteSlot))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_direction_leads_from_every_position_to_a_position_of_the_cross() {
        for direction in [
            Direction::Up,
            Direction::Down,
            Direction::Left,
            Direction::Right,
        ] {
            for position in 0..=NONE {
                assert!(direction.from(position) < TILES);
            }
        }
    }

    #[test]
    fn a_direction_key_from_no_selection_selects_a_tile_next_to_the_centre() {
        for direction in [
            Direction::Up,
            Direction::Down,
            Direction::Left,
            Direction::Right,
        ] {
            let tile = direction.from(NONE);
            assert!(tile < TILES);
        }
        assert_ne!(Direction::Up.from(NONE), Direction::Down.from(NONE));
        assert_ne!(Direction::Left.from(NONE), Direction::Right.from(NONE));
    }

    #[test]
    fn nothing_is_assigned_while_no_tile_is_selected() {
        let state = Favourites::new(ItemId(3));
        assert_eq!(state.assign(), FavouritesOutcome::Open);
        assert_eq!(state.selected(), None);
    }

    #[test]
    fn a_selected_tile_is_the_slot_the_item_goes_to() {
        let mut state = Favourites::new(ItemId(3));
        state.selected = 4;
        assert_eq!(
            state.assign(),
            FavouritesOutcome::Assign {
                item: ItemId(3),
                slot: FavouriteSlot(4)
            }
        );
    }
}
