//! The item card's layout from the movie: each entry type's bounds, background, text fields and
//! glyphs, the stacking constants and the comparison glyphs.

use egui::{Pos2, Rect, Vec2};

use super::entry_text::{CountSuffix, CountSuffixKeys, IconKeys, icon_keys};
use crate::assets::pipboy_art::Icon;
use crate::rendering::pipboy::scaleform_text::{FieldSpec, TextAutoSize};
use crate::rendering::pipboy::shapes::{OutlinedShape, Shade};
use crate::ui_layout::{
    BoundsKey, LiteralKey, NumberKey, PlacementKey, ShapeKey, StringKey, TextKey, UiLayout,
    UiLayoutError, key_check_fields,
};

pub const MOVIE: &str = "Pipboy_InvPage";

/// How far two glyph offsets may differ and still count as the same offset.
const GLYPH_TOLERANCE: f32 = 1e-3;

pub struct RowKeys {
    pub bounds: BoundsKey,
    pub background: ShapeKey,
    pub label: TextKey,
    pub value: TextKey,
}

macro_rules! row_keys {
    ($class:literal) => {
        RowKeys {
            bounds: BoundsKey::new(MOVIE, concat!("[", $class, "]")),
            background: ShapeKey::new(MOVIE, concat!("[", $class, "]/@1/@1")),
            label: TextKey::new(MOVIE, concat!("[", $class, "]/Label_tf")),
            value: TextKey::new(MOVIE, concat!("[", $class, "]/Value_tf")),
        }
    };
}

key_check_fields!(RowKeys {
    bounds,
    background,
    label,
    value
});

/// A fixed-height entry: its bounds, background and text fields.
#[derive(Debug, Clone)]
pub struct Row {
    bounds: [f32; 4],
    pub background: OutlinedShape,
    pub label: FieldSpec,
    pub value: FieldSpec,
    /// The value field's `x` as the entry's code reads it.
    pub value_x: f32,
}

impl Row {
    fn resolve(layout: &UiLayout, keys: &RowKeys) -> Result<Self, UiLayoutError> {
        Ok(Self {
            bounds: layout.get(&keys.bounds)?,
            background: OutlinedShape::resolve(layout, &keys.background)?,
            label: FieldSpec::resolve(layout, &keys.label, TextAutoSize::Shrink)?,
            value: FieldSpec::resolve(layout, &keys.value, TextAutoSize::Shrink)?,
            value_x: layout.get(&keys.value)?.placement.local.matrix.tx,
        })
    }

    pub fn height(&self) -> f32 {
        self.bounds[3] - self.bounds[1]
    }
}

/// A background whose height the entry's code sets: its edges follow the new height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stretch {
    x: (f32, f32),
    top: f32,
    edge_y: (f32, f32),
    bounds_y: (f32, f32),
    pub shade: Shade,
}

impl Stretch {
    fn resolve(layout: &UiLayout, key: &ShapeKey) -> Result<Self, UiLayoutError> {
        let shape = layout.get(key)?;
        let matrix = shape.placement.world.matrix;
        let [ex0, ey0, ex1, ey1] = shape.edge_bounds;
        let colour = shape
            .fills
            .iter()
            .find_map(|fill| fill.color)
            .ok_or_else(|| {
                UiLayoutError::unresolved(key.movie, key.path, "the background has no solid fill")
            })?;
        Ok(Self {
            x: (matrix.tx + matrix.a * ex0, matrix.tx + matrix.a * ex1),
            top: matrix.ty,
            edge_y: (ey0, ey1),
            bounds_y: (shape.bounds[1], shape.bounds[3]),
            shade: Shade::of(colour, &shape.placement.world.color),
        })
    }

    fn scale(&self, height: f32) -> f32 {
        height / (self.bounds_y.1 - self.bounds_y.0)
    }

    /// The filled rectangle once the clip's `height` is `height`, in entry space.
    pub fn rect(&self, height: f32) -> Rect {
        let scale = self.scale(height);
        Rect::from_min_max(
            Pos2::new(self.x.0, self.top + scale * self.edge_y.0),
            Pos2::new(self.x.1, self.top + scale * self.edge_y.1),
        )
    }

    /// Its top and bottom as the entry's bounds see them.
    pub fn bounds(&self, height: f32) -> (f32, f32) {
        let scale = self.scale(height);
        (
            self.top + scale * self.bounds_y.0,
            self.top + scale * self.bounds_y.1,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimerIcon {
    original_x: f32,
    y: f32,
    half_width: f32,
}

impl TimerIcon {
    /// The icon's `x` once the value text starts at `value_left` (entry space).
    pub fn x(&self, value_left: f32, gap: f32) -> f32 {
        let moved = value_left - self.half_width - gap;
        if moved < self.original_x {
            moved
        } else {
            self.original_x
        }
    }

    pub fn y(&self) -> f32 {
        self.y
    }
}

pub struct TimedKeys {
    pub row: RowKeys,
    pub comparison: PlacementKey,
    pub icon: IconKeys,
    pub gap: LiteralKey,
}

key_check_fields!(TimedKeys {
    row,
    comparison,
    icon,
    gap
});

#[derive(Debug, Clone)]
pub struct Timed {
    pub row: Row,
    pub comparison: Vec2,
    pub icon: TimerIcon,
    pub gap: f32,
}

pub struct AmmoKeys {
    pub row: RowKeys,
    pub glyph: PlacementKey,
}

key_check_fields!(AmmoKeys { row, glyph });

#[derive(Debug, Clone)]
pub struct Ammo {
    pub row: Row,
    pub glyph: Vec2,
}

pub struct StandardKeys {
    pub row: RowKeys,
    pub comparison: PlacementKey,
}

key_check_fields!(StandardKeys { row, comparison });

#[derive(Debug, Clone)]
pub struct Standard {
    pub row: Row,
    pub comparison: Vec2,
}

pub struct DescriptionKeys {
    pub background: ShapeKey,
    pub label: TextKey,
    pub padding: LiteralKey,
}

key_check_fields!(DescriptionKeys {
    background,
    label,
    padding
});

#[derive(Debug, Clone, Copy)]
pub struct Description {
    pub background: Stretch,
    pub label: FieldSpec,
    pub padding: f32,
}

pub struct ChipKeys {
    pub bounds: BoundsKey,
    pub background: ShapeKey,
    pub value: TextKey,
    pub comparison: PlacementKey,
    pub icon: PlacementKey,
}

key_check_fields!(ChipKeys {
    bounds,
    background,
    value,
    comparison,
    icon
});

pub struct MultiKeys {
    pub label: TextKey,
    pub background: ShapeKey,
    pub holder: PlacementKey,
    pub chip: ChipKeys,
}

key_check_fields!(MultiKeys {
    label,
    background,
    holder,
    chip
});

#[derive(Debug, Clone)]
pub struct Chip {
    pub bounds: [f32; 4],
    pub background: OutlinedShape,
    pub value: FieldSpec,
    pub comparison: Vec2,
    pub icon: Vec2,
}

#[derive(Debug, Clone)]
pub struct Multi {
    pub label: FieldSpec,
    pub background: Stretch,
    pub holder: Vec2,
    pub chip: Chip,
}

impl Multi {
    /// The vertical extent of `count` stacked chips in the holder's space, and the pitch
    /// between chips.
    fn chip_pitch(&self, spacing: f32) -> f32 {
        self.chip.bounds[3] - self.chip.bounds[1] + spacing
    }

    pub fn chip_y(&self, index: usize, spacing: f32) -> f32 {
        index as f32 * self.chip_pitch(spacing)
    }

    /// The entry's bounds `(top, bottom)` with `count` chips and the height of its background.
    pub fn bounds(&self, count: usize, spacing: f32) -> ((f32, f32), f32) {
        let pitch = self.chip_pitch(spacing);
        let holder_top = self.holder.y + self.chip.bounds[1];
        let holder_bottom = self.holder.y + (count as f32 - 1.0) * pitch + self.chip.bounds[3];
        let background_height = (holder_bottom - holder_top) + spacing;
        let (bg_top, bg_bottom) = self.background.bounds(background_height);
        let label_top = self.label.geometry.top;
        let label_bottom = self.label.geometry.bottom;
        (
            (
                holder_top.min(bg_top).min(label_top),
                holder_bottom.max(bg_bottom).max(label_bottom),
            ),
            background_height,
        )
    }
}

pub struct ComponentsKeys {
    pub holder: PlacementKey,
    pub spacing: NumberKey,
    pub text: TextKey,
    pub bounds: BoundsKey,
    pub icon: IconKeys,
    pub suffix: CountSuffixKeys,
    pub lead: LiteralKey,
}

key_check_fields!(ComponentsKeys {
    holder,
    spacing,
    text,
    bounds,
    icon,
    suffix,
    lead
});

#[derive(Debug, Clone)]
pub struct Components {
    pub holder: Vec2,
    pub spacing: f32,
    pub text: FieldSpec,
    pub text_x: f32,
    pub text_width: f32,
    pub pitch: f32,
    pub icon_y: f32,
    pub lead: f32,
    pub suffix: CountSuffix,
}

/// The glyph clips of a comparison frame: where the second and third glyph of the positive and
/// of the negative frames sit.
pub struct ComparisonKeys {
    pub positive: [PlacementKey; 3],
    pub negative: [PlacementKey; 3],
}

macro_rules! glyph_keys {
    ($better:literal, $best:literal) => {
        [
            PlacementKey::new(
                MOVIE,
                concat!("[ItemCard_StandardEntry]/Comparison_mc:", $better, "/@1"),
            ),
            PlacementKey::new(
                MOVIE,
                concat!("[ItemCard_StandardEntry]/Comparison_mc:", $better, "/@3"),
            ),
            PlacementKey::new(
                MOVIE,
                concat!("[ItemCard_StandardEntry]/Comparison_mc:", $best, "/@5"),
            ),
        ]
    };
}

key_check_fields!(ComparisonKeys { positive, negative });

/// The comparison glyphs of a rating: one to three, each at its own offset from the clip.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    offsets: [f32; 3],
}

impl Comparison {
    fn resolve(layout: &UiLayout, keys: &ComparisonKeys) -> Result<Self, UiLayoutError> {
        let offsets = |glyphs: &[PlacementKey; 3]| -> Result<[f32; 3], UiLayoutError> {
            Ok([
                layout.get(&glyphs[0])?.local.matrix.tx,
                layout.get(&glyphs[1])?.local.matrix.tx,
                layout.get(&glyphs[2])?.local.matrix.tx,
            ])
        };
        let positive = offsets(&keys.positive)?;
        let negative = offsets(&keys.negative)?;
        if positive
            .iter()
            .zip(&negative)
            .any(|(up, down)| (up - down).abs() > GLYPH_TOLERANCE)
        {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                keys.negative[1].path,
                "the glyphs of a negative rating sit elsewhere than a positive one's",
            ));
        }
        Ok(Self { offsets: positive })
    }

    /// The offsets of the glyphs a rating shows; none for a rating of zero.
    pub fn glyph_offsets(&self, rating: i8) -> &[f32] {
        let count = usize::from(rating.unsigned_abs()).min(self.offsets.len());
        &self.offsets[..count]
    }

    pub fn icon(rating: i8) -> Icon {
        if rating < 0 {
            Icon::ComparisonWorse
        } else {
            Icon::ComparisonBetter
        }
    }
}

pub struct CardKeys {
    pub anchor: PlacementKey,
    pub spacing: NumberKey,
    pub multi_spacing: NumberKey,
    pub damage_label: StringKey,
    pub resist_label: StringKey,
    pub standard: StandardKeys,
    pub ammo: AmmoKeys,
    pub timed: TimedKeys,
    pub description: DescriptionKeys,
    pub multi: MultiKeys,
    pub components: ComponentsKeys,
    pub comparison: ComparisonKeys,
}

macro_rules! standard_keys {
    ($class:literal) => {
        StandardKeys {
            row: row_keys!($class),
            comparison: PlacementKey::new(MOVIE, concat!("[", $class, "]/Comparison_mc")),
        }
    };
}

pub static CARD_KEYS: CardKeys = CardKeys {
    anchor: PlacementKey::new(MOVIE, "ItemCard_mc"),
    spacing: NumberKey::new(MOVIE, "Components.ItemCard.ENTRY_SPACING"),
    multi_spacing: NumberKey::new(MOVIE, "Components.ItemCard_MultiEntry.ENTRY_SPACING"),
    damage_label: StringKey::new(MOVIE, "Components.ItemCard_MultiEntry.DMG_WEAP_ID"),
    resist_label: StringKey::new(MOVIE, "Components.ItemCard_MultiEntry.DMG_ARMO_ID"),
    standard: standard_keys!("ItemCard_StandardEntry"),
    ammo: AmmoKeys {
        row: row_keys!("ItemCard_AmmoEntry"),
        glyph: PlacementKey::new(MOVIE, "[ItemCard_AmmoEntry]/@5"),
    },
    timed: TimedKeys {
        row: row_keys!("Components.ItemCard_TimedEntry"),
        comparison: PlacementKey::new(MOVIE, "[Components.ItemCard_TimedEntry]/Comparison_mc"),
        icon: icon_keys!(MOVIE, "[Components.ItemCard_TimedEntry]/TimerIcon_mc"),
        gap: LiteralKey::new(
            MOVIE,
            "Components.ItemCard_TimedEntry.PopulateEntry#((Value_tf.x+Value_tf.getLineMetrics().x)-(TimerIcon_mc.width/2))-",
            1,
        ),
    },
    description: DescriptionKeys {
        background: ShapeKey::new(
            MOVIE,
            "[Components.ItemCard_DescriptionEntry]/Background_mc/@1",
        ),
        label: TextKey::new(MOVIE, "[Components.ItemCard_DescriptionEntry]/Label_tf"),
        padding: LiteralKey::new(
            MOVIE,
            "Components.ItemCard_DescriptionEntry.PopulateEntry#Label_tf.textHeight+",
            1,
        ),
    },
    multi: MultiKeys {
        label: TextKey::new(MOVIE, "[Components.ItemCard_MultiEntry]/Label_tf"),
        background: ShapeKey::new(MOVIE, "[Components.ItemCard_MultiEntry]/Background_mc/@1"),
        holder: PlacementKey::new(MOVIE, "[Components.ItemCard_MultiEntry]/EntryHolder_mc"),
        chip: ChipKeys {
            bounds: BoundsKey::new(MOVIE, "[ItemCard_MultiEntry_Value]"),
            background: ShapeKey::new(MOVIE, "[ItemCard_MultiEntry_Value]/@1/@1"),
            value: TextKey::new(MOVIE, "[ItemCard_MultiEntry_Value]/Value_tf"),
            comparison: PlacementKey::new(MOVIE, "[ItemCard_MultiEntry_Value]/Comparison_mc"),
            icon: PlacementKey::new(MOVIE, "[ItemCard_MultiEntry_Value]/Icon_mc"),
        },
    },
    components: ComponentsKeys {
        holder: PlacementKey::new(
            MOVIE,
            "[Components.ItemCard_ComponentsEntry]/EntryHolder_mc",
        ),
        spacing: NumberKey::new(MOVIE, "Components.ItemCard_ComponentsEntry.ENTRY_SPACING"),
        text: TextKey::new(
            MOVIE,
            "[Components.ItemCard_ComponentEntry_Entry]/textField",
        ),
        bounds: BoundsKey::new(MOVIE, "[Components.ItemCard_ComponentEntry_Entry]"),
        icon: icon_keys!(
            MOVIE,
            "[Components.ItemCard_ComponentEntry_Entry]/FavIcon_mc",
        ),
        suffix: CountSuffixKeys::new(
            LiteralKey::texts(
                MOVIE,
                "Components.ItemCard_ComponentEntry_Entry.SetEntryText#+$1.count",
                1,
            ),
            LiteralKey::texts(
                MOVIE,
                "Components.ItemCard_ComponentEntry_Entry.SetEntryText#(\" (\"+$1.count)+",
                1,
            ),
        ),
        lead: LiteralKey::new(
            MOVIE,
            "Components.ItemCard_ComponentEntry_Entry.SetEntryText#((textField.x+(textField.width/2))+(textField.textWidth/2))+",
            1,
        ),
    },
    comparison: ComparisonKeys {
        positive: glyph_keys!("Better", "Best"),
        negative: glyph_keys!("Worse", "Worst"),
    },
};

key_check_fields!(CardKeys {
    anchor,
    spacing,
    multi_spacing,
    damage_label,
    resist_label,
    standard,
    ammo,
    timed,
    description,
    multi,
    components,
    comparison
});

/// The item card resolved from the layout.
#[derive(Debug, Clone)]
pub struct CardLayout {
    pub anchor: Vec2,
    pub spacing: f32,
    pub multi_spacing: f32,
    pub damage_label: String,
    pub resist_label: String,
    pub standard: Standard,
    pub ammo: Ammo,
    pub timed: Timed,
    pub description: Description,
    pub multi: Multi,
    pub components: Components,
    pub comparison: Comparison,
}

fn origin(layout: &UiLayout, key: &PlacementKey) -> Result<Vec2, UiLayoutError> {
    let matrix = layout.get(key)?.world.matrix;
    Ok(Vec2::new(matrix.tx, matrix.ty))
}

fn local_origin(layout: &UiLayout, key: &PlacementKey) -> Result<Vec2, UiLayoutError> {
    let matrix = layout.get(key)?.local.matrix;
    Ok(Vec2::new(matrix.tx, matrix.ty))
}

impl CardLayout {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let keys = &CARD_KEYS;
        let timer_matrix = layout.get(&keys.timed.icon.clip)?.local.matrix;
        let [tx0, _, tx1, _] = layout.get(&keys.timed.icon.bounds)?;
        let [_, entry_top, _, entry_bottom] = layout.get(&keys.components.bounds)?;
        let chip = &keys.multi.chip;
        let components = &keys.components;
        Ok(Self {
            anchor: origin(layout, &keys.anchor)?,
            spacing: layout.get(&keys.spacing)? as f32,
            multi_spacing: layout.get(&keys.multi_spacing)? as f32,
            damage_label: layout.get(&keys.damage_label)?,
            resist_label: layout.get(&keys.resist_label)?,
            standard: Standard {
                row: Row::resolve(layout, &keys.standard.row)?,
                comparison: origin(layout, &keys.standard.comparison)?,
            },
            ammo: Ammo {
                row: Row::resolve(layout, &keys.ammo.row)?,
                glyph: origin(layout, &keys.ammo.glyph)?,
            },
            timed: Timed {
                row: Row::resolve(layout, &keys.timed.row)?,
                comparison: origin(layout, &keys.timed.comparison)?,
                icon: TimerIcon {
                    original_x: timer_matrix.tx,
                    y: timer_matrix.ty,
                    half_width: (tx1 - tx0) / 2.0,
                },
                gap: layout.lit_number(&keys.timed.gap, 0)? as f32,
            },
            description: Description {
                background: Stretch::resolve(layout, &keys.description.background)?,
                label: FieldSpec::resolve(layout, &keys.description.label, TextAutoSize::None)?,
                padding: layout.lit_number(&keys.description.padding, 0)? as f32,
            },
            multi: Multi {
                label: FieldSpec::resolve(layout, &keys.multi.label, TextAutoSize::Shrink)?,
                background: Stretch::resolve(layout, &keys.multi.background)?,
                holder: local_origin(layout, &keys.multi.holder)?,
                chip: Chip {
                    bounds: layout.get(&chip.bounds)?,
                    background: OutlinedShape::resolve(layout, &chip.background)?,
                    value: FieldSpec::resolve(layout, &chip.value, TextAutoSize::Shrink)?,
                    comparison: origin(layout, &chip.comparison)?,
                    icon: origin(layout, &chip.icon)?,
                },
            },
            components: {
                let text = layout.get(&components.text)?;
                Components {
                    holder: local_origin(layout, &components.holder)?,
                    spacing: layout.get(&components.spacing)? as f32,
                    text: FieldSpec::resolve(layout, &components.text, TextAutoSize::Shrink)?,
                    text_x: text.placement.local.matrix.tx,
                    text_width: text.bounds[2] - text.bounds[0],
                    pitch: entry_bottom - entry_top,
                    icon_y: layout.get(&components.icon.clip)?.local.matrix.ty,
                    lead: layout.lit_number(&components.lead, 0)? as f32,
                    suffix: components.suffix.resolve(layout)?,
                }
            },
            comparison: Comparison::resolve(layout, &keys.comparison)?,
        })
    }
}
