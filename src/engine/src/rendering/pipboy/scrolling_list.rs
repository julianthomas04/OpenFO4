//! The game's scrolling list (`BSScrollingList` showing `BSScrollingListEntry` rows), laid out
//! from the layout: the list's border bounds its height, rows stack by the height of their entry
//! clip plus the list's vertical spacing, the selected row shows its border as the selection bar,
//! and the scroll arrows show while there is more to scroll to.
//!
//! A page declares its list with [`list_keys!`], resolves the [`ListKeys`] once into a
//! [`ListLayout`] and keeps a [`ListState`] for selection and scroll position.

use egui::{Color32, Painter, Pos2, Rect, Vec2};

use super::paint::{faded, fill};
use super::scaleform_text::{FieldSpec, PaintedField, TextAutoSize};
use super::shapes::{Shade, code_colour, rect_of};
use super::tab::{NavInput, TabContext};
use crate::assets::pipboy_art::FixedArt;
use crate::ui_layout::scaleform_text::AutoSize;
use crate::ui_layout::set_text::{SetTextKeys, SetTextRules};
use crate::ui_layout::values::{ColorTransform, Matrix, Placement};
use crate::ui_layout::{
    BoundsKey, Declaration, KeyCheck, LiteralKey, NumberKey, PlacementKey, ShapeKey, StringKey,
    TextKey, UiLayout, UiLayoutError, key_check_fields,
};

/// The keys of one list. Build them with [`list_keys!`].
pub struct ListKeys {
    pub movie: &'static str,
    /// The list clip.
    pub list: PlacementKey,
    /// Its `border` clip, which sets the list's extent.
    pub border: PlacementKey,
    pub border_bounds: BoundsKey,
    /// The list's `numListItems`, `verticalSpacing`, `textOption` and `listEntryClass`, as its
    /// component parameters set them.
    pub num_items: LiteralKey,
    pub spacing: LiteralKey,
    pub text_option: LiteralKey,
    pub entry_class: LiteralKey,
    /// The entry symbol's class, which `listEntryClass` must name.
    pub entry_symbol: &'static str,
    /// The entry symbol: all of it, its border shape and its text field.
    pub entry: BoundsKey,
    pub entry_border: ShapeKey,
    pub entry_border_clip: PlacementKey,
    pub entry_text: TextKey,
    pub arrows: Option<ArrowKeys>,
    pub shared: SharedListKeys,
}

/// The scroll arrows of a list.
pub struct ArrowKeys {
    pub up: ShapeKey,
    pub down: ShapeKey,
    pub up_clip: PlacementKey,
    pub down_clip: PlacementKey,
}

/// The keys every list of a movie shares: the list and entry classes' own constants and
/// literals.
pub struct SharedListKeys {
    pub set_text: SetTextKeys,
    pub selected_alpha: NumberKey,
    pub shrink_option: StringKey,
    pub multiline_option: StringKey,
    pub none_option: StringKey,
    pub text_colours: LiteralKey,
    pub border_alpha: LiteralKey,
    pub inset_factor: LiteralKey,
    pub multiline_pad: LiteralKey,
    pub multiline_above: LiteralKey,
}

impl SharedListKeys {
    pub const fn new(movie: &'static str) -> Self {
        Self {
            set_text: SetTextKeys::new(movie),
            selected_alpha: NumberKey::new(movie, "Shared.GlobalFunc.SELECTED_RECT_ALPHA"),
            shrink_option: StringKey::new(
                movie,
                "Shared.AS3.BSScrollingList.TEXT_OPTION_SHRINK_TO_FIT",
            ),
            multiline_option: StringKey::new(
                movie,
                "Shared.AS3.BSScrollingList.TEXT_OPTION_MULTILINE",
            ),
            none_option: StringKey::new(movie, "Shared.AS3.BSScrollingList.TEXT_OPTION_NONE"),
            text_colours: LiteralKey::new(
                movie,
                "Shared.AS3.BSScrollingListEntry.SetEntryText#textField.textColor=",
                2,
            ),
            border_alpha: LiteralKey::new(
                movie,
                "Shared.AS3.BSScrollingListEntry.SetEntryText#border.alpha=",
                1,
            ),
            inset_factor: LiteralKey::new(
                movie,
                "Shared.AS3.BSScrollingListEntry.SetEntryText#$3*",
                1,
            ),
            multiline_pad: LiteralKey::new(
                movie,
                "Shared.AS3.BSScrollingListEntry.SetEntryText#(textField.textHeight+($3*2))+",
                1,
            ),
            multiline_above: LiteralKey::new(
                movie,
                "Shared.AS3.BSScrollingListEntry.SetEntryText#textField.numLines>",
                1,
            ),
        }
    }
}

/// Declares the [`ListKeys`] of a list.
///
/// `list` is the list clip's path, `set_prop` the class method that sets its component
/// parameters, `entry` the entry symbol's class and `border` the path of the border shape inside
/// the entry's `border` clip. `arrows` names the shape inside the list's `ScrollUp` and
/// `ScrollDown` clips, for lists that have them. `member` is the list's member name in the
/// class that sets its parameters, `List_mc` when left out.
///
/// ```ignore
/// static LIST: ListKeys = list_keys!(
///     movie: "Pipboy_StatsPage",
///     list: "SPECIALTab_mc/List_mc",
///     set_prop: "SPECIALTab.__setProp_List_mc_SPECIALTab_List_0",
///     entry: "SPECIALListEntry",
///     border: "@2",
///     arrows: none,
/// );
/// ```
macro_rules! list_keys {
    (
        movie: $movie:literal,
        list: $list:literal,
        set_prop: $prop:literal,
        $(member: $member:literal,)?
        entry: $entry:literal,
        border: $border:literal,
        arrows: none $(,)?
    ) => {
        $crate::rendering::pipboy::scrolling_list::list_keys!(
            @build $movie, $list, $prop, [$($member)?], $entry, $border, None
        )
    };
    (
        movie: $movie:literal,
        list: $list:literal,
        set_prop: $prop:literal,
        $(member: $member:literal,)?
        entry: $entry:literal,
        border: $border:literal,
        arrows: $arrow:literal $(,)?
    ) => {
        $crate::rendering::pipboy::scrolling_list::list_keys!(
            @build $movie, $list, $prop, [$($member)?], $entry, $border,
            Some($crate::rendering::pipboy::scrolling_list::ArrowKeys {
                up: $crate::ui_layout::ShapeKey::new($movie, concat!($list, "/ScrollUp/", $arrow)),
                down: $crate::ui_layout::ShapeKey::new($movie, concat!($list, "/ScrollDown/", $arrow)),
                up_clip: $crate::ui_layout::PlacementKey::new($movie, concat!($list, "/ScrollUp")),
                down_clip: $crate::ui_layout::PlacementKey::new($movie, concat!($list, "/ScrollDown")),
            })
        )
    };
    (@member []) => { "List_mc" };
    (@member [$member:literal]) => { $member };
    (@build $movie:literal, $list:literal, $prop:literal, [$($member:literal)?], $entry:literal, $border:literal, $arrows:expr) => {
        $crate::rendering::pipboy::scrolling_list::ListKeys {
            movie: $movie,
            list: $crate::ui_layout::PlacementKey::new($movie, $list),
            border: $crate::ui_layout::PlacementKey::new($movie, concat!($list, "/border")),
            border_bounds: $crate::ui_layout::BoundsKey::new($movie, concat!($list, "/border")),
            num_items: $crate::ui_layout::LiteralKey::new($movie, concat!($prop, "#", $crate::rendering::pipboy::scrolling_list::list_keys!(@member [$($member)?]), ".numListItems="), 1),
            spacing: $crate::ui_layout::LiteralKey::new($movie, concat!($prop, "#", $crate::rendering::pipboy::scrolling_list::list_keys!(@member [$($member)?]), ".verticalSpacing="), 1),
            text_option: $crate::ui_layout::LiteralKey::texts($movie, concat!($prop, "#", $crate::rendering::pipboy::scrolling_list::list_keys!(@member [$($member)?]), ".textOption="), 1),
            entry_class: $crate::ui_layout::LiteralKey::texts($movie, concat!($prop, "#", $crate::rendering::pipboy::scrolling_list::list_keys!(@member [$($member)?]), ".listEntryClass="), 1),
            entry_symbol: $entry,
            entry: $crate::ui_layout::BoundsKey::new($movie, concat!("[", $entry, "]")),
            entry_border: $crate::ui_layout::ShapeKey::new($movie, concat!("[", $entry, "]/border/", $border)),
            entry_border_clip: $crate::ui_layout::PlacementKey::new($movie, concat!("[", $entry, "]/border")),
            entry_text: $crate::ui_layout::TextKey::new($movie, concat!("[", $entry, "]/textField")),
            arrows: $arrows,
            shared: $crate::rendering::pipboy::scrolling_list::SharedListKeys::new($movie),
        }
    };
}
pub(crate) use list_keys;

key_check_fields!(ArrowKeys {
    up,
    down,
    up_clip,
    down_clip
});
key_check_fields!(SharedListKeys {
    set_text,
    selected_alpha,
    shrink_option,
    multiline_option,
    none_option,
    text_colours,
    border_alpha,
    inset_factor,
    multiline_pad,
    multiline_above,
});

impl ListKeys {
    fn parts(&self) -> [&dyn KeyCheck; 13] {
        [
            &self.list,
            &self.border,
            &self.border_bounds,
            &self.num_items,
            &self.spacing,
            &self.text_option,
            &self.entry_class,
            &self.entry,
            &self.entry_border,
            &self.entry_border_clip,
            &self.entry_text,
            &self.shared,
            &self.arrows,
        ]
    }
}

impl KeyCheck for ListKeys {
    fn failures(&self, layout: &UiLayout) -> Vec<UiLayoutError> {
        let failures: Vec<_> = self
            .parts()
            .iter()
            .flat_map(|part| part.failures(layout))
            .collect();
        if failures.is_empty() {
            ListLayout::resolve(layout, self)
                .err()
                .into_iter()
                .collect()
        } else {
            failures
        }
    }

    fn declare(&self, out: &mut Vec<Declaration>) {
        for part in self.parts() {
            part.declare(out);
        }
    }
}

/// How the list sets its entries' text (`textOption`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOption {
    None,
    ShrinkToFit,
    /// Rows grow with their wrapped text.
    MultiLine,
}

/// A scroll arrow: its clip's placement (the down arrow is the up arrow turned round), where it
/// takes clicks and its glyph's colour.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Arrow {
    clip: Matrix,
    hit: Rect,
    shade: Shade,
}

impl Arrow {
    fn resolve(
        layout: &UiLayout,
        key: &ShapeKey,
        clip: &PlacementKey,
    ) -> Result<Self, UiLayoutError> {
        let clip = layout.get(clip)?.world.matrix;
        let shape = layout.get(key)?;
        let matrix = shape.placement.world.matrix;
        let line = shape.lines.first().ok_or_else(|| {
            UiLayoutError::unresolved(key.movie, key.path, "the arrow has no line style")
        })?;
        let colour = line.color.ok_or_else(|| {
            UiLayoutError::unresolved(key.movie, key.path, "the arrow's line has no colour")
        })?;
        Ok(Self {
            clip,
            hit: rect_of(matrix.bounds(shape.bounds)),
            shade: Shade::of(colour, &shape.placement.world.color),
        })
    }

    fn paint(&self, cx: &mut TabContext<'_>, offset: Vec2) {
        let clip = Matrix {
            tx: self.clip.tx + offset.x,
            ty: self.clip.ty + offset.y,
            ..self.clip
        };
        let colour = self.shade.colour(cx.tint);
        cx.draw_fixed_affine(FixedArt::ListScrollArrow, 0, &clip, colour);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Arrows {
    up: Arrow,
    down: Arrow,
}

/// One list resolved from the layout, in stage pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct ListLayout {
    /// Where the list places an entry's registration point at scroll offset `0`.
    origin: Pos2,
    list_height: f32,
    max_items: usize,
    spacing: f32,
    /// The entry clip's height and its bounds' horizontal extent, in entry space.
    entry_height: f32,
    entry_bounds: [f32; 4],
    border_offset: f32,
    bar: Rect,
    bar_shade: Shade,
    selected_alpha: f32,
    unselected_alpha: f32,
    /// The entry's text field, in entry space.
    pub text: FieldSpec,
    text_shades: [Shade; 2],
    pub option: TextOption,
    multiline_inset_factor: f32,
    multiline_pad: f32,
    multiline_above: usize,
    arrows: Option<Arrows>,
    pub rules: SetTextRules,
}

/// The selection bar's colour: the border shape's fill under the colour transform it inherits,
/// with the alpha the list's code sets on the `border` clip replacing that clip's authored one.
/// Only the shape's own transform and fill alpha remain in the alpha.
fn selection_bar_shade(fill: [u8; 4], placement: &Placement) -> Shade {
    let own = &placement.local.color;
    let own_alpha = Shade::of(
        fill,
        &ColorTransform {
            mult: [1.0, 1.0, 1.0, own.mult[3]],
            add: [0.0, 0.0, 0.0, own.add[3]],
        },
    )
    .alpha;
    Shade {
        alpha: own_alpha,
        ..Shade::of(fill, &placement.world.color)
    }
}

fn translation(movie: &str, key: &str, matrix: &Matrix) -> Result<Vec2, UiLayoutError> {
    let translated = Matrix {
        tx: matrix.tx,
        ty: matrix.ty,
        ..Matrix::IDENTITY
    };
    if *matrix == translated {
        Ok(Vec2::new(matrix.tx, matrix.ty))
    } else {
        Err(UiLayoutError::unresolved(
            movie,
            key,
            "the list clip is scaled or rotated",
        ))
    }
}

impl ListLayout {
    pub fn resolve(layout: &UiLayout, keys: &ListKeys) -> Result<Self, UiLayoutError> {
        let shared = &keys.shared;
        let list = layout.get(&keys.list)?;
        let origin = translation(keys.movie, keys.list.path, &list.world.matrix)?;
        let border = layout.get(&keys.border)?;
        let [_, border_top, _, border_bottom] = layout.get(&keys.border_bounds)?;
        let class = layout.lit_string(&keys.entry_class, 0)?;
        if class != keys.entry_symbol {
            return Err(UiLayoutError::unresolved(
                keys.movie,
                keys.entry_class.path,
                format!(
                    "the list makes '{class}' entries, not '{}'",
                    keys.entry_symbol
                ),
            ));
        }
        let option_text = layout.lit_string(&keys.text_option, 0)?;
        let option = if option_text == layout.get(&shared.shrink_option)? {
            TextOption::ShrinkToFit
        } else if option_text == layout.get(&shared.multiline_option)? {
            TextOption::MultiLine
        } else if option_text == layout.get(&shared.none_option)? {
            TextOption::None
        } else {
            return Err(UiLayoutError::unresolved(
                keys.movie,
                keys.text_option.path,
                format!("'{option_text}' is no text option"),
            ));
        };
        let auto_size = match option {
            TextOption::ShrinkToFit => TextAutoSize::Shrink,
            TextOption::None | TextOption::MultiLine => TextAutoSize::None,
        };
        let entry_border = layout.get(&keys.entry_border)?;
        let entry_border_clip = layout.get(&keys.entry_border_clip)?;
        let bar_colour = entry_border
            .fills
            .iter()
            .find_map(|fill| fill.color)
            .ok_or_else(|| {
                UiLayoutError::unresolved(keys.movie, keys.entry_border.path, "no solid fill")
            })?;
        let arrows = match &keys.arrows {
            Some(arrows) => Some(Arrows {
                up: Arrow::resolve(layout, &arrows.up, &arrows.up_clip)?,
                down: Arrow::resolve(layout, &arrows.down, &arrows.down_clip)?,
            }),
            None => None,
        };
        let entry_bounds = layout.get(&keys.entry)?;
        Ok(Self {
            origin: (origin + Vec2::new(0.0, border.local.matrix.ty)).to_pos2(),
            list_height: border_bottom - border_top,
            max_items: layout.lit_whole(&keys.num_items, 0)? as usize,
            spacing: layout.lit_number(&keys.spacing, 0)? as f32,
            entry_height: entry_bounds[3] - entry_bounds[1],
            entry_bounds,
            border_offset: entry_border_clip.local.matrix.ty,
            bar: rect_of(
                entry_border
                    .placement
                    .world
                    .matrix
                    .bounds(entry_border.edge_bounds),
            ),
            bar_shade: selection_bar_shade(bar_colour, &entry_border.placement),
            selected_alpha: layout.get(&shared.selected_alpha)? as f32,
            unselected_alpha: layout.lit_number(&shared.border_alpha, 0)? as f32,
            text: FieldSpec::resolve(layout, &keys.entry_text, auto_size)?,
            text_shades: [
                code_colour(layout, &shared.text_colours, 0)?,
                code_colour(layout, &shared.text_colours, 1)?,
            ],
            option,
            multiline_inset_factor: layout.lit_number(&shared.inset_factor, 0)? as f32,
            multiline_pad: layout.lit_number(&shared.multiline_pad, 0)? as f32,
            multiline_above: layout.lit_whole(&shared.multiline_above, 0)? as usize,
            arrows,
            rules: shared.set_text.resolve(layout)?,
        })
    }

    /// The height of every entry while its text fits one line (the entry clip's height).
    pub fn entry_height(&self) -> f32 {
        self.entry_height
    }

    pub fn max_items(&self) -> usize {
        self.max_items
    }

    /// The text the entry's text field shows for `text`: the game's text-setting rule applied.
    pub fn entry_text(&self, layout: &UiLayout, text: &str) -> String {
        self.rules.apply(layout, text, false, false)
    }

    /// The height of a multi-line entry showing `shown`: its text field grows with the wrapped
    /// text and, past one line, its border to the text height plus twice the text's inset from
    /// the border plus the pad.
    pub fn multiline_height(&self, painter: &Painter, shown: &str) -> f32 {
        let field = self.text.growing(AutoSize::Left);
        let style = field.style(Color32::WHITE);
        let lines = super::scaleform_text::line_count(painter, shown, &field.geometry, style);
        if lines <= self.multiline_above {
            return self.entry_height;
        }
        let text_height =
            super::scaleform_text::text_height(painter, shown, &field.geometry, style);
        let inset = field.geometry.top - self.border_offset;
        let border_height = text_height + inset * self.multiline_inset_factor + self.multiline_pad;
        let border_bottom = self.border_offset + border_height;
        let text_bottom = field.geometry.top + text_height + 2.0 * field.geometry.vertical_inset;
        border_bottom.max(text_bottom).max(self.entry_bounds[3]) - self.entry_bounds[1]
    }

    /// Draws `shown` (see [`Self::entry_text`]) in the entry's text field on `row`.
    pub fn paint_text(&self, painter: &Painter, row: &Row, shown: &str) -> PaintedField {
        let mut field = self.text.translated(row.origin);
        if self.option == TextOption::MultiLine {
            field = field.growing(AutoSize::Left);
        }
        field.paint_coloured(painter, shown, row.text_colour)
    }
}

/// How tall each entry is.
#[derive(Clone, Copy)]
pub enum RowHeights<'a> {
    /// Every entry is the entry clip's height.
    Fixed,
    /// Entry `i` is `height(i)` tall (multi-line lists).
    Each(&'a dyn Fn(usize) -> f32),
}

/// A list's content as the state machine sees it.
#[derive(Clone, Copy)]
pub struct ListView<'a> {
    pub layout: &'a ListLayout,
    pub len: usize,
    pub heights: RowHeights<'a>,
    offset: Vec2,
    dimmed: Option<(&'a dyn Fn(usize) -> bool, f32)>,
    /// Whether entries can be selected. A display-only list scrolls instead: up and down move
    /// the scroll position, the wheel never selects and clicks only reach the arrows.
    pub selectable: bool,
}

impl<'a> ListView<'a> {
    pub fn new(layout: &'a ListLayout, len: usize) -> Self {
        Self {
            layout,
            len,
            heights: RowHeights::Fixed,
            offset: Vec2::ZERO,
            dimmed: None,
            selectable: true,
        }
    }

    /// The list moved by `offset` from its authored place, its arrows with it (a list the menu
    /// repositions).
    #[must_use]
    pub fn offset(self, offset: Vec2) -> Self {
        Self { offset, ..self }
    }

    /// Entries for which `dimmed` holds are drawn at `alpha`, their selection bar included.
    #[must_use]
    pub fn dimmed(self, dimmed: &'a dyn Fn(usize) -> bool, alpha: f32) -> Self {
        Self {
            dimmed: Some((dimmed, alpha)),
            ..self
        }
    }

    fn origin(&self) -> Vec2 {
        self.layout.origin.to_vec2() + self.offset
    }

    /// The list as a display-only one (see [`Self::selectable`]).
    #[must_use]
    pub fn display_only(self) -> Self {
        Self {
            selectable: false,
            ..self
        }
    }

    fn alpha(&self, index: usize) -> f32 {
        match self.dimmed {
            Some((dimmed, alpha)) if dimmed(index) => alpha,
            _ => 1.0,
        }
    }

    #[must_use]
    pub fn with_heights(self, heights: &'a dyn Fn(usize) -> f32) -> Self {
        Self {
            heights: RowHeights::Each(heights),
            ..self
        }
    }

    fn height(&self, index: usize) -> f32 {
        match self.heights {
            RowHeights::Fixed => self.layout.entry_height,
            RowHeights::Each(height) => height(index),
        }
    }

    /// The highest scroll position: the last entries that fit the list end at its bottom. As in
    /// the game's own calculation an entry fits only while the stacked height stays strictly below
    /// the list height, whereas [`Self::rows`] shows entries up to and including it: a list that
    /// its entries fill exactly keeps one scroll step more than its rows need.
    pub fn max_scroll(&self) -> usize {
        let Some(last) = self.len.checked_sub(1) else {
            return 0;
        };
        let layout = self.layout;
        let mut height = self.height(last);
        let mut first = Some(last);
        let mut shown = 1;
        while let Some(current) = first
            && height < layout.list_height
            && shown < layout.max_items
        {
            first = current.checked_sub(1).map(|previous| {
                height += self.height(previous) + layout.spacing;
                if height < layout.list_height {
                    shown += 1;
                    previous
                } else {
                    current
                }
            });
        }
        first.unwrap_or(0)
    }

    /// The rows shown from `scroll`: each entry's index and its top offset from the list origin.
    pub fn rows(&self, scroll: usize) -> Vec<(usize, f32)> {
        let layout = self.layout;
        let mut filled = 0.0;
        let mut top = 0.0;
        let mut rows = Vec::new();
        for index in scroll..self.len {
            if rows.len() >= layout.max_items || filled > layout.list_height {
                break;
            }
            let height = self.height(index);
            filled += height;
            let fits = filled <= layout.list_height && rows.len() < layout.max_items;
            if fits || layout.option == TextOption::MultiLine {
                rows.push((index, top));
                top += height + layout.spacing;
            }
            if fits {
                filled += layout.spacing;
            }
        }
        rows
    }
}

/// What a list input did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListEvent {
    None,
    /// The selection moved to a new entry.
    Moved,
    /// Only the scroll position changed.
    Scrolled,
    /// An entry was clicked: it is selected and its action runs.
    Activated(usize),
}

/// One shown row as a page paints it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Row {
    pub index: usize,
    pub selected: bool,
    /// Where the entry clip's registration point is.
    pub origin: Vec2,
    pub height: f32,
    /// The colour the entry's code gives its text: black on the selection bar, white otherwise,
    /// under the tint, at the row's opacity.
    pub text_colour: Color32,
    /// The entry's opacity: below 1 when the page dims it.
    pub alpha: f32,
}

/// Selection and scroll position of a list, changed the way the game's list changes them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ListState {
    selected: Option<usize>,
    scroll: usize,
    pointer: Option<Pos2>,
}

impl ListState {
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// Re-establishes the list after its entries changed: the selection stays inside the list
    /// and the scroll position at most the highest one.
    pub fn invalidate(&mut self, view: &ListView<'_>) {
        if let Some(selected) = self.selected
            && selected >= view.len
        {
            self.selected = view.len.checked_sub(1);
        }
        self.scroll = self.scroll.min(view.max_scroll());
    }

    fn set_scroll(&mut self, view: &ListView<'_>, scroll: isize) -> bool {
        let max = view.max_scroll();
        match usize::try_from(scroll) {
            Ok(scroll) if scroll != self.scroll && scroll <= max => {
                self.scroll = scroll;
                true
            }
            _ => false,
        }
    }

    /// Selects `index` and scrolls just enough to show it.
    pub fn select(&mut self, view: &ListView<'_>, index: usize) -> bool {
        let Some(last) = view.len.checked_sub(1) else {
            self.selected = None;
            return false;
        };
        let index = index.min(last);
        if self.selected == Some(index) {
            return false;
        }
        self.selected = Some(index);
        let rows = view.rows(self.scroll);
        let shown = rows.iter().any(|(row, _)| *row == index);
        let overflows = view.layout.option == TextOption::MultiLine
            && rows.last().is_some_and(|(row, top)| {
                *row == index && top + view.height(*row) > view.layout.list_height
            });
        if !shown || overflows {
            let first = self.scroll;
            let last_shown = rows.last().map_or(first, |(row, _)| *row);
            let shift = if index < first {
                index as isize - first as isize
            } else if index > last_shown {
                index as isize - last_shown as isize
            } else {
                1
            };
            self.set_scroll(view, self.scroll as isize + shift);
        }
        true
    }

    fn row_at(&self, view: &ListView<'_>, pos: Pos2) -> Option<usize> {
        let layout = view.layout;
        let [x0, y0, x1, _] = layout.entry_bounds;
        view.rows(self.scroll).into_iter().find_map(|(index, top)| {
            let origin = view.origin().to_pos2() + Vec2::new(0.0, top);
            let rect = Rect::from_min_max(
                origin + Vec2::new(x0, y0),
                origin + Vec2::new(x1, y0 + view.height(index)),
            );
            rect.contains(pos).then_some(index)
        })
    }

    fn arrow_at(&self, view: &ListView<'_>, pos: Pos2) -> Option<isize> {
        let arrows = view.layout.arrows?;
        let up_shown = self.scroll > 0;
        let down_shown = self.scroll < view.max_scroll();
        let pos = pos - view.offset;
        if up_shown && arrows.up.hit.contains(pos) {
            Some(-1)
        } else if down_shown && arrows.down.hit.contains(pos) {
            Some(1)
        } else {
            None
        }
    }

    /// Applies one input: up and down move the selection, the wheel scrolls one entry and
    /// selects the entry under the pointer, hovering selects, a click activates, and the arrows
    /// scroll.
    pub fn handle(&mut self, input: NavInput, view: &ListView<'_>) -> ListEvent {
        let moved = |changed: bool| {
            if changed {
                ListEvent::Moved
            } else {
                ListEvent::None
            }
        };
        if !view.selectable {
            return self.handle_display_only(input, view);
        }
        match input {
            NavInput::Up => match self.selected {
                Some(selected) if selected > 0 => moved(self.select(view, selected - 1)),
                _ => ListEvent::None,
            },
            NavInput::Down => {
                let next = self.selected.map_or(0, |selected| selected + 1);
                if next < view.len {
                    moved(self.select(view, next))
                } else {
                    ListEvent::None
                }
            }
            NavInput::Scrolled(delta) => self.wheel(view, delta),
            NavInput::PointerMoved(pos) => {
                self.pointer = Some(pos);
                match self.row_at(view, pos) {
                    Some(index) => moved(self.select(view, index)),
                    None => ListEvent::None,
                }
            }
            NavInput::Clicked(pos) => {
                self.pointer = Some(pos);
                if let Some(step) = self.arrow_at(view, pos) {
                    let scrolled = self.set_scroll(view, self.scroll as isize + step);
                    return if scrolled {
                        ListEvent::Scrolled
                    } else {
                        ListEvent::None
                    };
                }
                match self.row_at(view, pos) {
                    Some(index) => {
                        self.select(view, index);
                        ListEvent::Activated(index)
                    }
                    None => ListEvent::None,
                }
            }
            _ => ListEvent::None,
        }
    }

    fn handle_display_only(&mut self, input: NavInput, view: &ListView<'_>) -> ListEvent {
        let scrolled = |changed: bool| {
            if changed {
                ListEvent::Scrolled
            } else {
                ListEvent::None
            }
        };
        let step = |state: &mut Self, step: isize| {
            scrolled(state.set_scroll(view, state.scroll as isize + step))
        };
        match input {
            NavInput::Up => step(self, -1),
            NavInput::Down => step(self, 1),
            NavInput::Scrolled(delta) if delta > 0.0 => step(self, -1),
            NavInput::Scrolled(delta) if delta < 0.0 => step(self, 1),
            NavInput::PointerMoved(pos) => {
                self.pointer = Some(pos);
                ListEvent::None
            }
            NavInput::Clicked(pos) => {
                self.pointer = Some(pos);
                match self.arrow_at(view, pos) {
                    Some(arrow) => step(self, arrow),
                    None => ListEvent::None,
                }
            }
            _ => ListEvent::None,
        }
    }

    fn wheel(&mut self, view: &ListView<'_>, delta: f32) -> ListEvent {
        if view.max_scroll() == 0 {
            return ListEvent::None;
        }
        let step = if delta < 0.0 {
            1
        } else if delta > 0.0 {
            -1
        } else {
            0
        };
        let scrolled = step != 0 && self.set_scroll(view, self.scroll as isize + step);
        let selected = self
            .pointer
            .and_then(|pos| self.row_at(view, pos))
            .is_some_and(|index| self.select(view, index));
        if selected {
            ListEvent::Moved
        } else if scrolled {
            ListEvent::Scrolled
        } else {
            ListEvent::None
        }
    }

    /// Paints the shown rows: the selection bar behind the selected one, then `row` for each;
    /// then the arrows through [`Self::paint_arrows`].
    pub fn paint(
        &self,
        painter: &Painter,
        tint: Color32,
        view: &ListView<'_>,
        mut row: impl FnMut(&Painter, &Row),
    ) {
        let layout = view.layout;
        let [selected_shade, unselected_shade] = layout.text_shades;
        for (index, top) in view.rows(self.scroll) {
            let origin = view.origin() + Vec2::new(0.0, top);
            let selected = self.selected == Some(index);
            let alpha = view.alpha(index);
            let bar_alpha = if selected {
                layout.selected_alpha
            } else {
                layout.unselected_alpha
            };
            if bar_alpha > 0.0 {
                let shade = Shade {
                    alpha: layout.bar_shade.alpha * bar_alpha * alpha,
                    ..layout.bar_shade
                };
                let mut bar = layout.bar.translate(origin);
                if let RowHeights::Each(_) = view.heights {
                    let above_bar = layout.bar.min.y - layout.entry_bounds[1];
                    bar.max.y = bar.min.y + view.height(index) - above_bar;
                }
                fill(painter, bar, shade.colour(tint));
            }
            let text_shade = if selected {
                selected_shade
            } else {
                unselected_shade
            };
            row(
                painter,
                &Row {
                    index,
                    selected,
                    origin,
                    height: view.height(index),
                    text_colour: faded(text_shade.colour(tint), alpha),
                    alpha,
                },
            );
        }
    }

    /// Paints the scroll arrows that apply: up while scrolled down, down while more entries
    /// follow.
    pub fn paint_arrows(&self, cx: &mut TabContext<'_>, view: &ListView<'_>) {
        let Some(arrows) = view.layout.arrows else {
            return;
        };
        if self.scroll > 0 {
            arrows.up.paint(cx, view.offset);
        }
        if self.scroll < view.max_scroll() {
            arrows.down.paint(cx, view.offset);
        }
    }
}

/// A colour after Flash's brightness adjustment: a negative `amount` scales the colour channels
/// toward black, keeping the alpha.
pub fn brightness(colour: Color32, amount: f32) -> Color32 {
    let scale = (1.0 + amount.min(0.0)).clamp(0.0, 1.0);
    let channel = |value: u8| (f32::from(value) * scale).round() as u8;
    Color32::from_rgba_premultiplied(
        channel(colour.r()),
        channel(colour.g()),
        channel(colour.b()),
        colour.a(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::pipboy::fonts::Weight;
    use crate::ui_layout::scaleform_text::FieldGeometry;
    use crate::ui_layout::test_support::{
        FIXTURE_METRICS, assert_close, layout_with_movie, placement, set_text_movie, text_field,
    };

    const ORIGIN: Pos2 = Pos2::new(40.0, 100.0);

    fn rules() -> SetTextRules {
        static KEYS: SetTextKeys = SetTextKeys::new("Fixture");
        KEYS.resolve(&layout_with_movie("Fixture", &set_text_movie()))
            .unwrap()
    }

    /// A list 100 px tall of 30 px entries, at most `max_items` shown, with arrows.
    fn layout(max_items: usize) -> ListLayout {
        let arrow = |y: f32, down: bool| Arrow {
            clip: Matrix {
                d: if down { -1.0 } else { 1.0 },
                tx: 60.0,
                ty: y,
                ..Matrix::IDENTITY
            },
            hit: Rect::from_min_size(Pos2::new(60.0, y), Vec2::splat(13.0)),
            shade: Shade::WHITE,
        };
        ListLayout {
            origin: ORIGIN,
            list_height: 100.0,
            max_items,
            spacing: 0.0,
            entry_height: 30.0,
            entry_bounds: [0.0, 0.0, 300.0, 30.0],
            border_offset: 0.0,
            bar: Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 30.0)),
            bar_shade: Shade::WHITE,
            selected_alpha: 1.0,
            unselected_alpha: 0.0,
            text: FieldSpec {
                geometry: FieldGeometry::of(
                    &text_field([-2.0, -2.0, 252.0, 30.0], 20.0),
                    FIXTURE_METRICS,
                ),
                weight: Weight::Regular,
                shade: Shade::WHITE,
                auto_size: TextAutoSize::Shrink,
                field_auto_size: None,
            },
            text_shades: [
                Shade {
                    grey: 0.0,
                    alpha: 1.0,
                },
                Shade::WHITE,
            ],
            option: TextOption::ShrinkToFit,
            multiline_inset_factor: 2.0,
            multiline_pad: 5.0,
            multiline_above: 1,
            arrows: Some(Arrows {
                up: arrow(80.0, false),
                down: arrow(190.0, true),
            }),
            rules: rules(),
        }
    }

    fn row_centre(visible_row: usize) -> Pos2 {
        ORIGIN + Vec2::new(150.0, 30.0 * visible_row as f32 + 15.0)
    }

    #[test]
    fn rows_are_shown_while_their_heights_fit_the_border() {
        let layout = layout(10);
        let view = ListView::new(&layout, 8);
        assert_eq!(view.rows(0), [(0, 0.0), (1, 30.0), (2, 60.0)]);
        assert_eq!(ListView::new(&layout, 0).rows(0), []);
    }

    #[test]
    fn no_more_rows_than_the_list_has_entry_clips_are_shown() {
        let layout = layout(2);
        assert_eq!(ListView::new(&layout, 8).rows(0).len(), 2);
    }

    #[test]
    fn the_highest_scroll_position_shows_the_last_entries_that_fit() {
        let layout = layout(10);
        assert_eq!(ListView::new(&layout, 5).max_scroll(), 2);
        assert_eq!(ListView::new(&layout, 3).max_scroll(), 0);
        assert_eq!(ListView::new(&layout, 0).max_scroll(), 0);
    }

    #[test]
    fn entries_that_fill_the_list_exactly_all_show_but_max_scroll_counts_them_as_not_fitting() {
        let layout = ListLayout {
            list_height: 90.0,
            ..layout(10)
        };
        let view = ListView::new(&layout, 8);
        assert_eq!(view.rows(5), [(5, 0.0), (6, 30.0), (7, 60.0)]);
        assert_eq!(view.max_scroll(), 6);
        assert_eq!(view.rows(6), [(6, 0.0), (7, 30.0)]);
    }

    #[test]
    fn selecting_past_the_window_scrolls_just_enough_to_show_it() {
        let layout = layout(10);
        let view = ListView::new(&layout, 8);
        let mut state = ListState::default();
        assert!(state.select(&view, 4));
        assert_eq!(state.scroll(), 2);
        state.select(&view, 0);
        assert_eq!(state.scroll(), 0);
    }

    #[test]
    fn down_from_no_selection_selects_the_first_entry_and_up_stops_at_it() {
        let layout = layout(10);
        let view = ListView::new(&layout, 3);
        let mut state = ListState::default();
        assert_eq!(state.handle(NavInput::Down, &view), ListEvent::Moved);
        assert_eq!(state.selected(), Some(0));
        assert_eq!(state.handle(NavInput::Up, &view), ListEvent::None);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn the_wheel_scrolls_one_entry_and_selects_the_entry_under_the_pointer() {
        let layout = layout(10);
        let view = ListView::new(&layout, 8);
        let mut state = ListState::default();
        state.handle(NavInput::PointerMoved(row_centre(0)), &view);
        assert_eq!(state.selected(), Some(0));
        let event = state.handle(NavInput::Scrolled(-1.0), &view);
        assert_eq!(event, ListEvent::Moved);
        assert_eq!(state.scroll(), 1);
        assert_eq!(state.selected(), Some(1));
        state.handle(NavInput::Scrolled(1.0), &view);
        assert_eq!(state.scroll(), 0);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn the_wheel_does_nothing_on_a_list_that_cannot_scroll() {
        let layout = layout(10);
        let view = ListView::new(&layout, 3);
        let mut state = ListState::default();
        state.handle(NavInput::PointerMoved(row_centre(2)), &view);
        assert_eq!(
            state.handle(NavInput::Scrolled(-1.0), &view),
            ListEvent::None
        );
        assert_eq!(state.scroll(), 0);
    }

    #[test]
    fn the_wheel_stops_at_the_highest_scroll_position() {
        let layout = layout(10);
        let view = ListView::new(&layout, 5);
        let mut state = ListState::default();
        for _ in 0..5 {
            state.handle(NavInput::Scrolled(-1.0), &view);
        }
        assert_eq!(state.scroll(), 2);
    }

    #[test]
    fn a_click_activates_the_entry_and_a_click_on_an_arrow_scrolls() {
        let layout = layout(10);
        let view = ListView::new(&layout, 8);
        let mut state = ListState::default();
        let event = state.handle(NavInput::Clicked(row_centre(1)), &view);
        assert_eq!(event, ListEvent::Activated(1));
        let down = Pos2::new(66.0, 196.0);
        assert_eq!(
            state.handle(NavInput::Clicked(down), &view),
            ListEvent::Scrolled
        );
        assert_eq!(state.scroll(), 1);
        let up = Pos2::new(66.0, 86.0);
        assert_eq!(
            state.handle(NavInput::Clicked(up), &view),
            ListEvent::Scrolled
        );
        assert_eq!(state.handle(NavInput::Clicked(up), &view), ListEvent::None);
    }

    #[test]
    fn a_moved_list_takes_its_rows_and_arrows_along() {
        let layout = layout(10);
        let shift = Vec2::new(0.0, 40.0);
        let view = ListView::new(&layout, 8).offset(shift);
        let mut state = ListState::default();
        assert_eq!(
            state.handle(NavInput::Clicked(row_centre(1) + shift), &view),
            ListEvent::Activated(1)
        );
        let down = Pos2::new(66.0, 196.0) + shift;
        assert_eq!(
            state.handle(NavInput::Clicked(down), &view),
            ListEvent::Scrolled
        );
    }

    #[test]
    fn dimmed_entries_carry_their_opacity_to_the_row() {
        let layout = layout(10);
        let odd = |index: usize| index % 2 == 1;
        let view = ListView::new(&layout, 4).dimmed(&odd, 0.25);
        let ctx = egui::Context::default();
        let painter = Painter::new(ctx, egui::LayerId::background(), Rect::EVERYTHING);
        let mut alphas = Vec::new();
        ListState::default().paint(&painter, Color32::WHITE, &view, |_, row| {
            alphas.push(row.alpha);
        });
        assert_eq!(alphas, [1.0, 0.25, 1.0]);
    }

    #[test]
    fn the_selection_bar_ignores_the_authored_alpha_of_the_border_clip() {
        let mut border = placement(0.0, 0.0);
        border.world.color.mult[3] = 0.0;
        let shade = selection_bar_shade([200, 200, 200, 255], &border);
        assert_close(shade.alpha, 1.0);
        let mut layout = layout(10);
        layout.bar_shade = shade;
        let drawn = layout.bar_shade.alpha * layout.selected_alpha * 0.25;
        assert_close(drawn, 0.25);
    }

    #[test]
    fn the_selection_bar_keeps_the_alpha_of_the_shape_itself() {
        let mut border = placement(0.0, 0.0);
        border.local.color.mult[3] = 0.5;
        border.world.color.mult[3] = 0.0;
        assert_close(
            selection_bar_shade([200, 200, 200, 255], &border).alpha,
            0.5,
        );
    }

    #[test]
    fn a_list_names_its_member_in_the_parameter_keys() {
        static KEYS: ListKeys = list_keys!(
            movie: "Fixture",
            list: "Holder_mc/Other_mc",
            set_prop: "Holder.__setProp_Other",
            member: "Other_mc",
            entry: "Entry",
            border: "@1",
            arrows: none,
        );
        assert_eq!(
            KEYS.num_items.path,
            "Holder.__setProp_Other#Other_mc.numListItems="
        );
    }

    #[test]
    fn invalidating_keeps_the_selection_and_scroll_inside_a_shorter_list() {
        let layout = layout(10);
        let mut state = ListState::default();
        state.select(&ListView::new(&layout, 8), 7);
        state.invalidate(&ListView::new(&layout, 4));
        assert_eq!(state.selected(), Some(3));
        assert_eq!(state.scroll(), 1);
        state.invalidate(&ListView::new(&layout, 0));
        assert_eq!(state.selected(), None);
        assert_eq!(state.scroll(), 0);
    }

    #[test]
    fn taller_entries_take_their_own_heights() {
        let layout = layout(10);
        let heights = |index: usize| if index == 0 { 70.0 } else { 30.0 };
        let view = ListView::new(&layout, 4).with_heights(&heights);
        assert_eq!(view.rows(0), [(0, 0.0), (1, 70.0)]);
        assert_eq!(view.max_scroll(), 1);
    }

    #[test]
    fn brightness_scales_the_colour_toward_black_and_keeps_alpha() {
        let dimmed = brightness(Color32::from_rgb(200, 100, 50), -0.4);
        assert_eq!(dimmed, Color32::from_rgb(120, 60, 30));
        assert_eq!(brightness(Color32::WHITE, 0.3), Color32::WHITE);
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR"]
    fn the_installed_stat_lists_stack_their_entries_by_the_entry_height() {
        use crate::rendering::pipboy::pages::{stat_perks, stat_special};
        let installed = crate::ui_layout::test_support::installed_layout();
        let special = ListLayout::resolve(&installed, &stat_special::LIST).unwrap();
        let perks = ListLayout::resolve(&installed, &stat_perks::LIST).unwrap();
        assert!((special.entry_height() - 42.15).abs() < 0.01);
        assert_eq!(ListView::new(&special, 7).rows(0).len(), 7);
        assert_eq!(ListView::new(&perks, 30).rows(0).len(), 10);
        assert_eq!(special.option, TextOption::ShrinkToFit);
    }
}
