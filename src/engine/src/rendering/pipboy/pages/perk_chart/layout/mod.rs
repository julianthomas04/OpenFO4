//! The perk chart's values from the `LevelUpMenu` layout document, read once and grouped by
//! area. Every key is declared in its area's module and resolved at boot through
//! [`LAYOUT_KEYS`]; the whole load runs at boot too, so what single keys cannot say alone fails
//! there.

macro_rules! number {
    ($name:ident, $key:literal) => {
        pub(crate) static $name: crate::ui_layout::NumberKey =
            crate::ui_layout::NumberKey::new(super::MOVIE, $key);
    };
}

macro_rules! literal {
    ($name:ident, $key:literal, $count:literal) => {
        pub(crate) static $name: crate::ui_layout::LiteralKey =
            crate::ui_layout::LiteralKey::new(super::MOVIE, $key, $count);
    };
}

macro_rules! text_literal {
    ($name:ident, $key:literal, $count:literal) => {
        pub(crate) static $name: crate::ui_layout::LiteralKey =
            crate::ui_layout::LiteralKey::texts(super::MOVIE, $key, $count);
    };
}

macro_rules! grid_row {
    ($key:ty, $prefix:literal, $suffix:literal, $row:literal) => {
        [
            <$key>::new(super::MOVIE, concat!($prefix, $row, "_0", $suffix)),
            <$key>::new(super::MOVIE, concat!($prefix, $row, "_1", $suffix)),
            <$key>::new(super::MOVIE, concat!($prefix, $row, "_2", $suffix)),
            <$key>::new(super::MOVIE, concat!($prefix, $row, "_3", $suffix)),
            <$key>::new(super::MOVIE, concat!($prefix, $row, "_4", $suffix)),
            <$key>::new(super::MOVIE, concat!($prefix, $row, "_5", $suffix)),
            <$key>::new(super::MOVIE, concat!($prefix, $row, "_6", $suffix)),
        ]
    };
}

pub mod cards;
pub mod grid;
pub mod headers;
pub mod hints;
pub mod info;
pub mod stars;
pub mod text;
pub mod xp;

use egui::{Color32, Pos2, Rect, Vec2};

pub use self::cards::CardLayout;
pub use self::grid::{CellLayout, StarHolderLayout};
pub use self::headers::HeaderLayout;
pub use self::hints::HintLabels;
pub use self::info::{InfoLayout, InfoRules};
pub use self::stars::{GridStars, OverlayLayout, SpecialStars, StarBlink, StarFrames};
pub use self::text::Text;
pub use self::xp::XpLayout;
use super::filter;
use crate::rendering::pipboy::art::ColourMatrix;
use crate::rendering::pipboy::hint_bar::{HintBarLayout, MoviePlacement};
use crate::rendering::pipboy::pages::perk_chart_layout::{Cell, GridConstants};
use crate::ui_layout::set_text::{SetTextKeys, SetTextRules};
use crate::ui_layout::values::Shape;
use crate::ui_layout::{
    LiteralKey, ModuleKeys, NumberKey, PlacementKey, StageKey, UiLayout, UiLayoutError,
};

pub const MOVIE: &str = "LevelUpMenu";

pub(crate) static SET_TEXT: SetTextKeys = SetTextKeys::new(MOVIE);

pub(super) static STAGE: StageKey = StageKey::of(MOVIE);

fn load_check(layout: &UiLayout) -> Result<(), UiLayoutError> {
    ChartLayout::load(layout).map(drop)
}

/// Every layout key the perk chart reads; resolved at boot. The whole load runs too, so what the
/// single keys cannot say alone (a blend mode, a bracket style, a hint key name, a font) fails
/// there instead of while drawing.
pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[
        &cards::KEYS,
        &grid::KEYS,
        &headers::KEYS,
        &stars::KEYS,
        &info::KEYS,
        &xp::KEYS,
        &hints::KEYS,
        &SET_TEXT,
        &STAGE,
        &filter::LAYOUT_KEYS,
    ],
    resolve: load_check,
};

pub(crate) fn number(layout: &UiLayout, key: &NumberKey) -> Result<f32, UiLayoutError> {
    Ok(layout.get(key)? as f32)
}

/// The first value of a literal list as a number.
pub(crate) fn single(layout: &UiLayout, key: &LiteralKey) -> Result<f32, UiLayoutError> {
    Ok(layout.lit_number(key, 0)? as f32)
}

pub(crate) fn unresolved(path: &str, reason: impl Into<String>) -> UiLayoutError {
    UiLayoutError::unresolved(MOVIE, path, reason.into())
}

pub(crate) fn rgba([r, g, b, a]: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

/// Where the sprite at `key` sits in its parent.
pub(crate) fn local_origin(layout: &UiLayout, key: &PlacementKey) -> Result<Vec2, UiLayoutError> {
    let matrix = layout.get(key)?.local.matrix;
    Ok(Vec2::new(matrix.tx, matrix.ty))
}

/// A string literal of the menu with its leading `$$` reduced to the single `$` of a translate
/// key.
pub(crate) fn translate_key_of(literal: &str) -> &str {
    literal.strip_prefix('$').unwrap_or(literal)
}

/// What a text field shows after the menu's `SetText` of the string `key`.
pub(crate) struct Words<'a> {
    layout: &'a UiLayout,
    rules: SetTextRules,
}

impl<'a> Words<'a> {
    pub(crate) fn new(layout: &'a UiLayout) -> Result<Self, UiLayoutError> {
        Ok(Self {
            layout,
            rules: SET_TEXT.resolve(layout)?,
        })
    }

    pub(crate) fn text(&self, key: &str) -> String {
        self.rules.apply(self.layout, key, false, false)
    }
}

pub struct ChartLayout {
    /// The chart movie's stage, which the chart is drawn on.
    pub stage: Rect,
    pub unavailable_filter: ColourMatrix,
    pub hint_bar: HintBarLayout,
    pub hint_placement: MoviePlacement,
    pub grid: GridConstants,
    pub cells: Vec<CellLayout>,
    pub star_holders: Vec<Option<StarHolderLayout>>,
    pub headers: Vec<HeaderLayout>,
    pub overlay: OverlayLayout,
    pub grid_stars: GridStars,
    pub special_stars: SpecialStars,
    pub rank_star_size: Vec2,
    pub star_frames: StarFrames,
    pub cards: CardLayout,
    pub star_blink: StarBlink,
    pub info: InfoLayout,
    pub xp: XpLayout,
    pub hints: HintLabels,
}

impl ChartLayout {
    pub fn load(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let words = Words::new(layout)?;
        let grid = grid::load(layout)?;
        let stars = stars::load(layout)?;
        let hints = hints::load(layout, &words)?;
        let stage = Rect::from_min_size(
            Pos2::ZERO,
            crate::rendering::pipboy::stage::perk_chart_size(layout)?,
        );
        Ok(Self {
            stage,
            unavailable_filter: filter::unavailable_filter(layout)?,
            hint_bar: hints.bar,
            hint_placement: hints.placement,
            grid: grid.constants,
            cells: grid.cells,
            star_holders: grid.star_holders,
            headers: headers::load(layout)?,
            overlay: stars.overlay,
            grid_stars: stars.grid,
            special_stars: stars.special,
            rank_star_size: stars.rank_star_size,
            star_frames: stars.frames,
            cards: cards::load(layout)?,
            star_blink: stars.blink,
            info: info::load(layout, &words)?,
            xp: xp::load(layout, &words)?,
            hints: hints.labels,
        })
    }

    pub fn cell(&self, cell: Cell) -> &CellLayout {
        &self.cells[cell.index()]
    }

    pub fn star_holder(&self, cell: Cell) -> Option<&StarHolderLayout> {
        self.star_holders[cell.index()].as_ref()
    }
}

pub(crate) fn shape_rect(shape: &Shape) -> Rect {
    let [x_min, y_min, x_max, y_max] = shape.bounds;
    let matrix = shape.placement.world.matrix;
    let (left, top) = matrix.apply(x_min, y_min);
    let (right, bottom) = matrix.apply(x_max, y_max);
    Rect::from_min_max(
        Pos2::new(left.min(right), top.min(bottom)),
        Pos2::new(left.max(right), top.max(bottom)),
    )
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact literals")]
mod tests {
    use super::*;
    use crate::rendering::pipboy::test_layout::pipboy_layout;

    #[test]
    fn the_fixture_satisfies_every_declared_key() {
        pipboy_layout().validate(&[&LAYOUT_KEYS]).unwrap();
    }

    #[test]
    fn a_leading_dollar_pair_reduces_to_one_translate_dollar() {
        assert_eq!(translate_key_of("$$word"), "$word");
        assert_eq!(translate_key_of("word"), "word");
    }

    #[test]
    fn the_whole_layout_loads_from_the_fixture() {
        ChartLayout::load(&pipboy_layout()).unwrap();
    }

    /// Developer check against a real install: every key of the chart resolves in the installed
    /// movie and the chart's layout loads from it.
    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the installer's layout documents"]
    fn the_installed_movie_satisfies_the_charts_keys_and_loads() {
        let install = std::env::var(crate::assets::INSTALL_DIR_VAR).expect("OPENFO4_INSTALL_DIR");
        let dir = crate::assets::layout::ui_layout_dir(&format!(
            "{install}/{}",
            crate::assets::layout::ASSETS_DIR
        ));
        let document = UiLayout::open(std::path::Path::new(&dir)).unwrap();
        document.validate(&[&LAYOUT_KEYS]).unwrap();

        let layout = ChartLayout::load(&document).unwrap();

        assert_eq!(layout.grid.min_scale, 0.625);
        assert_eq!(layout.grid.content_height, 3047.5);
        assert_eq!(layout.hints.choose_template, "CHOOSE (#)");
        assert_eq!(layout.info.error_seconds, 2.0);
        assert_eq!(layout.xp.level_word, "LEVEL");
        assert_eq!(layout.star_blink.fade.len(), 40);
        let library = crate::assets::pipboy_art::ArtLibrary::load(std::path::Path::new(&format!(
            "{install}/{}",
            crate::assets::layout::ASSETS_DIR
        )))
        .unwrap();
        for class in &layout.cards.special_classes {
            assert!(
                library.perk_card_for_class(class).is_some(),
                "no card clip for {class}"
            );
        }
        for id in [
            super::super::stars::RANK_STAR_CLIP,
            super::super::stars::SPECIAL_STAR_CLIP,
            super::super::grid::STRAY_CLIP,
        ] {
            assert!(library.clip(id).is_some(), "no clip {id}");
        }
        let iron_fist = layout.cell(Cell { column: 0, row: 1 });
        assert!(
            (iron_fist.origin.x - 355.4).abs() < 0.1 && (iron_fist.origin.y - 555.4).abs() < 0.1
        );
    }
}
