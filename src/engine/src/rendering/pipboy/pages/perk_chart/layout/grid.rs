//! The grid: its scroll constants, where each card sits and the rect the mouse hits.

use egui::{Pos2, Rect, Vec2};

use super::{MOVIE, STAGE, number, shape_rect};
use crate::rendering::pipboy::pages::perk_chart_layout::{
    COLUMNS, GridConstants, HoverScroll, ROWS,
};
use crate::ui_layout::{KeyGroup, PlacementKey, ShapeKey, UiLayout, UiLayoutError};

pub(crate) static CELL_PLACEMENTS: [[PlacementKey; COLUMNS]; ROWS] = [
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/PerkAnim_", "", 0),
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/PerkAnim_", "", 1),
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/PerkAnim_", "", 2),
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/PerkAnim_", "", 3),
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/PerkAnim_", "", 4),
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/PerkAnim_", "", 5),
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/PerkAnim_", "", 6),
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/PerkAnim_", "", 7),
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/PerkAnim_", "", 8),
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/PerkAnim_", "", 9),
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/PerkAnim_", "", 10),
];

pub(crate) static CELL_BORDERS: [[ShapeKey; COLUMNS]; ROWS] = [
    grid_row!(
        ShapeKey,
        "[PerkGrid]/PerksHolder_mc/PerkAnim_",
        "/border/@1",
        0
    ),
    grid_row!(
        ShapeKey,
        "[PerkGrid]/PerksHolder_mc/PerkAnim_",
        "/border/@1",
        1
    ),
    grid_row!(
        ShapeKey,
        "[PerkGrid]/PerksHolder_mc/PerkAnim_",
        "/border/@1",
        2
    ),
    grid_row!(
        ShapeKey,
        "[PerkGrid]/PerksHolder_mc/PerkAnim_",
        "/border/@1",
        3
    ),
    grid_row!(
        ShapeKey,
        "[PerkGrid]/PerksHolder_mc/PerkAnim_",
        "/border/@1",
        4
    ),
    grid_row!(
        ShapeKey,
        "[PerkGrid]/PerksHolder_mc/PerkAnim_",
        "/border/@1",
        5
    ),
    grid_row!(
        ShapeKey,
        "[PerkGrid]/PerksHolder_mc/PerkAnim_",
        "/border/@1",
        6
    ),
    grid_row!(
        ShapeKey,
        "[PerkGrid]/PerksHolder_mc/PerkAnim_",
        "/border/@1",
        7
    ),
    grid_row!(
        ShapeKey,
        "[PerkGrid]/PerksHolder_mc/PerkAnim_",
        "/border/@1",
        8
    ),
    grid_row!(
        ShapeKey,
        "[PerkGrid]/PerksHolder_mc/PerkAnim_",
        "/border/@1",
        9
    ),
    grid_row!(
        ShapeKey,
        "[PerkGrid]/PerksHolder_mc/PerkAnim_",
        "/border/@1",
        10
    ),
];

/// Star holders exist for the perk rows only.
pub(crate) static STAR_HOLDERS: [[PlacementKey; COLUMNS]; ROWS - 1] = [
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/StarHolder_", "", 1),
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/StarHolder_", "", 2),
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/StarHolder_", "", 3),
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/StarHolder_", "", 4),
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/StarHolder_", "", 5),
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/StarHolder_", "", 6),
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/StarHolder_", "", 7),
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/StarHolder_", "", 8),
    grid_row!(PlacementKey, "[PerkGrid]/PerksHolder_mc/StarHolder_", "", 9),
    grid_row!(
        PlacementKey,
        "[PerkGrid]/PerksHolder_mc/StarHolder_",
        "",
        10
    ),
];

number!(MIN_SCALE, "PerkGrid.MIN_SCALE");
number!(MAX_SCALE, "PerkGrid.MAX_SCALE");
number!(CLOSE_ENOUGH, "Shared.GlobalFunc.CLOSE_ENOUGH_EPSILON");
number!(HOVER_X_MIN, "PerkGrid.MOUSE_HOVER_SCROLL_X_MIN");
number!(HOVER_X_MAX, "PerkGrid.MOUSE_HOVER_SCROLL_X_MAX");
number!(HOVER_Y_MIN, "PerkGrid.MOUSE_HOVER_SCROLL_Y_MIN");
number!(HOVER_Y_MAX, "PerkGrid.MOUSE_HOVER_SCROLL_Y_MAX");
number!(HOVER_X_SPEED, "PerkGrid.MOUSE_HOVER_SCROLL_X_SPEED");
number!(HOVER_Y_SPEED, "PerkGrid.MOUSE_HOVER_SCROLL_Y_SPEED");
number!(MAX_X, "PerkGrid.MAX_X_VAL");
number!(MAX_Y, "PerkGrid.MAX_Y_VAL");
number!(TEXTURE_WIDTH, "PerkGrid.TEX_WIDTH");
number!(TOP_HEIGHT, "PerkGrid.TOP_HEIGHT");
number!(BOTTOM_HEIGHT, "PerkGrid.BOTTOM_HEIGHT");
number!(BORDER_WIDTH, "PerkGrid.BORDER_WIDTH");
number!(BORDER_HEIGHT, "PerkGrid.BORDER_HEIGHT");
number!(MOUSE_DRAG_MOVE_SPEED, "PerkGrid.MOUSE_DRAG_MOVE_SPEED");

pub(crate) static KEYS: KeyGroup = KeyGroup(&[
    &CELL_PLACEMENTS,
    &CELL_BORDERS,
    &STAR_HOLDERS,
    &MIN_SCALE,
    &MAX_SCALE,
    &CLOSE_ENOUGH,
    &HOVER_X_MIN,
    &HOVER_X_MAX,
    &HOVER_Y_MIN,
    &HOVER_Y_MAX,
    &HOVER_X_SPEED,
    &HOVER_Y_SPEED,
    &MAX_X,
    &MAX_Y,
    &TEXTURE_WIDTH,
    &TOP_HEIGHT,
    &BOTTOM_HEIGHT,
    &BORDER_WIDTH,
    &BORDER_HEIGHT,
    &MOUSE_DRAG_MOVE_SPEED,
]);

/// Where a card sits in grid units: the clip's registration point and its scale, and the rect
/// the mouse hits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellLayout {
    pub origin: Pos2,
    pub scale: Vec2,
    pub border: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StarHolderLayout {
    pub origin: Pos2,
    pub scale: Vec2,
}

pub struct GridLayout {
    pub constants: GridConstants,
    pub cells: Vec<CellLayout>,
    pub star_holders: Vec<Option<StarHolderLayout>>,
}

pub fn load(layout: &UiLayout) -> Result<GridLayout, UiLayoutError> {
    let frame_rate = layout.get(&STAGE)?.fps;
    if !(frame_rate.is_finite() && frame_rate > 0.0) {
        return Err(UiLayoutError::unresolved(
            MOVIE,
            "stage",
            "the movie has no frame rate",
        ));
    }
    let constants = GridConstants {
        frame_rate,
        min_scale: number(layout, &MIN_SCALE)?,
        max_scale: number(layout, &MAX_SCALE)?,
        close_enough: number(layout, &CLOSE_ENOUGH)?,
        hover: HoverScroll {
            x_min: number(layout, &HOVER_X_MIN)?,
            x_max: number(layout, &HOVER_X_MAX)?,
            y_min: number(layout, &HOVER_Y_MIN)?,
            y_max: number(layout, &HOVER_Y_MAX)?,
            x_speed: number(layout, &HOVER_X_SPEED)?,
            y_speed: number(layout, &HOVER_Y_SPEED)?,
        },
        max_x: number(layout, &MAX_X)?,
        max_y: number(layout, &MAX_Y)?,
        texture_width: number(layout, &TEXTURE_WIDTH)?,
        top_height: number(layout, &TOP_HEIGHT)?,
        content_height: number(layout, &TOP_HEIGHT)? + number(layout, &BOTTOM_HEIGHT)?,
        border_width: number(layout, &BORDER_WIDTH)?,
        border_height: number(layout, &BORDER_HEIGHT)?,
        drag_speed: number(layout, &MOUSE_DRAG_MOVE_SPEED)?,
    };
    let mut cells = Vec::with_capacity(COLUMNS * ROWS);
    for (placements, borders) in CELL_PLACEMENTS.iter().zip(&CELL_BORDERS) {
        for (placement, border) in placements.iter().zip(borders) {
            let matrix = layout.get(placement)?.local.matrix;
            cells.push(CellLayout {
                origin: Pos2::new(matrix.tx, matrix.ty),
                scale: Vec2::new(matrix.a, matrix.d),
                border: shape_rect(&layout.get(border)?),
            });
        }
    }
    let mut star_holders = vec![None; COLUMNS];
    for row in &STAR_HOLDERS {
        for key in row {
            let matrix = layout.get(key)?.local.matrix;
            star_holders.push(Some(StarHolderLayout {
                origin: Pos2::new(matrix.tx, matrix.ty),
                scale: Vec2::new(matrix.a, matrix.d),
            }));
        }
    }
    Ok(GridLayout {
        constants,
        cells,
        star_holders,
    })
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact literals")]
mod tests {
    use super::*;
    use crate::rendering::pipboy::pages::perk_chart_layout::Cell;
    use crate::rendering::pipboy::test_layout::pipboy_layout;

    fn grid() -> GridLayout {
        load(&pipboy_layout()).unwrap()
    }

    #[test]
    fn every_cell_and_every_perk_row_star_holder_is_read() {
        let grid = grid();
        assert_eq!(grid.cells.len(), COLUMNS * ROWS);
        assert_eq!(grid.star_holders.len(), COLUMNS * ROWS);
        assert!(grid.star_holders[..COLUMNS].iter().all(Option::is_none));
        assert!(grid.star_holders[COLUMNS..].iter().all(Option::is_some));
    }

    #[test]
    fn the_hit_boxes_are_in_grid_units_and_the_cards_have_their_own_scale() {
        let grid = grid();
        let first = grid.cells[Cell::ALL[0].index()];
        assert!(first.border.width() > 0.0 && first.border.height() > 0.0);
        assert!(first.scale.x > 0.0 && first.scale.y > 0.0);
    }

    #[test]
    fn the_constants_come_from_the_movie_and_the_content_is_top_plus_bottom() {
        let layout = pipboy_layout();
        let constants = grid().constants;
        assert_eq!(constants.min_scale, number(&layout, &MIN_SCALE).unwrap());
        assert_eq!(
            constants.content_height,
            number(&layout, &TOP_HEIGHT).unwrap() + number(&layout, &BOTTOM_HEIGHT).unwrap()
        );
        assert_eq!(
            constants.drag_speed,
            number(&layout, &MOUSE_DRAG_MOVE_SPEED).unwrap()
        );
    }

    #[test]
    fn the_zoom_range_the_hover_zones_and_the_frame_rate_come_from_the_movie() {
        let layout = pipboy_layout();
        let constants = grid().constants;
        assert_eq!(constants.max_scale, number(&layout, &MAX_SCALE).unwrap());
        assert_eq!(
            constants.close_enough,
            number(&layout, &CLOSE_ENOUGH).unwrap()
        );
        let hover = constants.hover;
        assert_eq!(hover.x_min, number(&layout, &HOVER_X_MIN).unwrap());
        assert_eq!(hover.x_max, number(&layout, &HOVER_X_MAX).unwrap());
        assert_eq!(hover.y_min, number(&layout, &HOVER_Y_MIN).unwrap());
        assert_eq!(hover.y_max, number(&layout, &HOVER_Y_MAX).unwrap());
        assert_eq!(hover.x_speed, number(&layout, &HOVER_X_SPEED).unwrap());
        assert_eq!(hover.y_speed, number(&layout, &HOVER_Y_SPEED).unwrap());
        assert_eq!(constants.frame_rate, layout.get(&STAGE).unwrap().fps);
    }
}
