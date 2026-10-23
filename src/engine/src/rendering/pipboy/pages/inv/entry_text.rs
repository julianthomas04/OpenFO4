//! The text a list or card entry shows for a name and a count, and the clip decorations that
//! follow it.

use egui::{Color32, Painter, Pos2, Vec2};

use crate::assets::pipboy_art::Icon;
use crate::rendering::pipboy::scaleform_text::PaintedField;
use crate::rendering::pipboy::scrolling_list::{ListLayout, Row};
use crate::rendering::pipboy::shapes::OutlinedShape;
use crate::ui_layout::set_text::SetTextRules;
use crate::ui_layout::{
    BoundsKey, LiteralKey, PlacementKey, ShapeKey, TextKey, UiLayout, UiLayoutError,
    key_check_fields,
};

/// The two strings around a count appended to a name.
pub struct CountSuffixKeys {
    open: LiteralKey,
    close: LiteralKey,
}

impl CountSuffixKeys {
    pub const fn new(open: LiteralKey, close: LiteralKey) -> Self {
        Self { open, close }
    }

    pub fn resolve(&self, layout: &UiLayout) -> Result<CountSuffix, UiLayoutError> {
        Ok(CountSuffix {
            open: layout.lit_string(&self.open, 0)?,
            close: layout.lit_string(&self.close, 0)?,
        })
    }
}

key_check_fields!(CountSuffixKeys { open, close });

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountSuffix {
    open: String,
    close: String,
}

impl CountSuffix {
    /// `name` followed by the count unless it is exactly one.
    pub fn apply(&self, name: &str, count: u32) -> String {
        if count == 1 {
            name.to_owned()
        } else {
            format!("{name}{}{count}{}", self.open, self.close)
        }
    }
}

/// A list entry's name: cut to the length limit, given its count, then shown through the text
/// rule again.
pub fn list_entry_text(
    rules: &SetTextRules,
    layout: &UiLayout,
    suffix: &CountSuffix,
    name: &str,
    count: u32,
) -> String {
    let cut = rules.apply(layout, name, false, true);
    rules.apply(layout, &suffix.apply(&cut, count), false, false)
}

pub struct IconKeys {
    pub bounds: BoundsKey,
    pub clip: PlacementKey,
}

/// The keys of a one-shape icon clip at `$path`: the clip's placement and the bounds of its
/// shape, which is the clip's own width.
macro_rules! icon_keys {
    ($movie:expr, $path:literal $(,)?) => {
        $crate::rendering::pipboy::pages::inv::entry_text::IconKeys {
            bounds: $crate::ui_layout::BoundsKey::new($movie, concat!($path, "/@1")),
            clip: $crate::ui_layout::PlacementKey::new($movie, $path),
        }
    };
}
pub(crate) use icon_keys;

key_check_fields!(IconKeys { bounds, clip });

/// One decoration clip of an entry: its width, which spaces the icons after it, and its height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconSlot {
    pub icon: Icon,
    width: f32,
    y: f32,
}

impl IconSlot {
    pub fn resolve(layout: &UiLayout, keys: &IconKeys, icon: Icon) -> Result<Self, UiLayoutError> {
        let [x0, _, x1, _] = layout.get(&keys.bounds)?;
        Ok(Self {
            icon,
            width: x1 - x0,
            y: layout.get(&keys.clip)?.local.matrix.ty,
        })
    }

    fn advance(&self, gap: f32) -> f32 {
        self.width / 2.0 + gap
    }
}

/// The keys of an entry's decorations, in the order they follow the text.
pub struct EntryIconKeys {
    pub text: TextKey,
    pub legendary: Option<IconKeys>,
    pub favourite: Option<IconKeys>,
    pub search: IconKeys,
    pub equip: ShapeKey,
    /// The space the icons keep from each other's centre and the first from the text.
    pub gap: LiteralKey,
    pub lead: LiteralKey,
}

key_check_fields!(EntryIconKeys {
    text,
    legendary,
    favourite,
    search,
    equip,
    gap,
    lead
});

/// Which decorations one entry shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EntryFlags {
    pub legendary: bool,
    pub favourite: bool,
    pub search: bool,
    pub equipped: bool,
}

/// The decorations of an inventory entry: the text field narrows by the space the icons after the
/// first take, and the icons follow the drawn text.
#[derive(Debug, Clone)]
pub struct EntryIcons {
    text_x: f32,
    reduction: f32,
    gap: f32,
    lead: f32,
    slots: Vec<(Slot, IconSlot)>,
    equip: OutlinedShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Legendary,
    Favourite,
    Search,
}

impl EntryIcons {
    pub fn resolve(layout: &UiLayout, keys: &EntryIconKeys) -> Result<Self, UiLayoutError> {
        let mut slots = Vec::new();
        if let Some(legendary) = &keys.legendary {
            slots.push((
                Slot::Legendary,
                IconSlot::resolve(layout, legendary, Icon::Legendary)?,
            ));
        }
        if let Some(favourite) = &keys.favourite {
            slots.push((
                Slot::Favourite,
                IconSlot::resolve(layout, favourite, Icon::Favourite)?,
            ));
        }
        slots.push((
            Slot::Search,
            IconSlot::resolve(layout, &keys.search, Icon::SearchTag)?,
        ));
        let gap = layout.lit_number(&keys.gap, 0)? as f32;
        let reduction = slots
            .iter()
            .skip(1)
            .map(|(_, slot)| slot.advance(gap))
            .sum();
        Ok(Self {
            text_x: layout.get(&keys.text)?.placement.local.matrix.tx,
            reduction,
            gap,
            lead: layout.lit_number(&keys.lead, 0)? as f32,
            slots,
            equip: OutlinedShape::resolve(layout, &keys.equip)?,
        })
    }

    fn shows(flags: EntryFlags, slot: Slot) -> bool {
        match slot {
            Slot::Legendary => flags.legendary,
            Slot::Favourite => flags.favourite,
            Slot::Search => flags.search,
        }
    }

    /// Paints `shown` and the equip square on `row`; the icons come back for the caller to draw
    /// with its art context.
    pub fn paint_row(
        &self,
        painter: &Painter,
        list: &ListLayout,
        row: &Row,
        shown: &str,
        flags: EntryFlags,
    ) -> Vec<(Icon, Pos2, Color32)> {
        let mut text = list.text;
        text.geometry.right -= self.reduction;
        let painted: PaintedField =
            text.translated(row.origin)
                .paint_coloured(painter, shown, row.text_colour);
        if flags.equipped {
            let square = self.equip.translated(row.origin).bounds();
            crate::rendering::pipboy::paint::fill(painter, square, row.text_colour);
        }
        let mut x = self.text_x + (painted.run.1 - painted.run.0) + self.lead;
        let mut icons = Vec::new();
        for (kind, slot) in &self.slots {
            if Self::shows(flags, *kind) {
                icons.push((
                    slot.icon,
                    row.origin.to_pos2() + Vec2::new(x, slot.y),
                    row.text_colour,
                ));
                x += slot.advance(self.gap);
            }
        }
        icons
    }

    pub fn reduction(&self) -> f32 {
        self.reduction
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suffix() -> CountSuffix {
        CountSuffix {
            open: " <".to_owned(),
            close: ">".to_owned(),
        }
    }

    #[test]
    fn a_count_follows_the_name_unless_it_is_one() {
        assert_eq!(suffix().apply("thing", 1), "thing");
        assert_eq!(suffix().apply("thing", 3), "thing <3>");
        assert_eq!(suffix().apply("thing", 0), "thing <0>");
    }
}
