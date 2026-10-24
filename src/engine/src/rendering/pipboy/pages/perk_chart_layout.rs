//! Pure rules of the perk chart: which perk sits in which cell, how the grid scrolls,
//! which rank is shown and which stars are drawn. Every number comes from the caller (the layout
//! keys); nothing here knows the stage.

use egui::{Pos2, Vec2};

use crate::sim::pipboy::perks::PerkDefinition;
use crate::sim::pipboy::{Sex, Special};

pub const COLUMNS: usize = Special::ALL.len();
/// Rows of the chart: the SPECIAL row and one row per SPECIAL rank that unlocks perks.
pub const ROWS: usize = 11;
/// The row that shows the seven SPECIAL attributes instead of perks.
pub const SPECIAL_ROW: usize = 0;

/// A cell of the chart: SPECIAL column and row (`0` is the SPECIAL row, `n` the perks that need
/// SPECIAL rank `n`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Cell {
    pub column: usize,
    pub row: usize,
}

impl Cell {
    pub const ALL: [Cell; COLUMNS * ROWS] = {
        let mut cells = [Cell { column: 0, row: 0 }; COLUMNS * ROWS];
        let mut index = 0;
        while index < cells.len() {
            cells[index] = Cell {
                column: index % COLUMNS,
                row: index / COLUMNS,
            };
            index += 1;
        }
        cells
    };

    pub fn index(self) -> usize {
        self.row * COLUMNS + self.column
    }

    pub fn special(self) -> Special {
        Special::ALL[self.column]
    }

    pub fn is_special_row(self) -> bool {
        self.row == SPECIAL_ROW
    }
}

/// The perk a character of `sex` sees in `cell`: sex-specific perks that share a cell with a
/// counterpart show only the one for that sex. The SPECIAL row holds no perk.
pub fn perk_in_cell(perks: &[PerkDefinition], sex: Sex, cell: Cell) -> Option<&PerkDefinition> {
    if cell.is_special_row() {
        return None;
    }
    perks.iter().find(|perk| {
        perk.is_available_to(sex)
            && perk.chart.is_some_and(|slot| {
                slot.special == cell.special() && usize::from(slot.required_special) == cell.row
            })
    })
}

/// The wheel notches that zoom the chart from its smallest scale to its largest at a constant step.
/// Stand-in: measured in the game, the step itself is not in the menu's data.
pub const WHEEL_NOTCHES_FULL_ZOOM: f32 = 6.0;

/// The pointer zones at the stage edges that scroll the grid, and the grid's move per frame in
/// each.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoverScroll {
    pub x_min: f32,
    pub x_max: f32,
    pub y_min: f32,
    pub y_max: f32,
    pub x_speed: f32,
    pub y_speed: f32,
}

impl HoverScroll {
    /// The grid's move in one frame with the pointer at `pointer` in stage pixels: toward the
    /// centre from whichever edge zone it is in.
    pub fn step(&self, pointer: Pos2) -> Vec2 {
        let mut step = Vec2::ZERO;
        if pointer.x < self.x_min {
            step.x = self.x_speed;
        }
        if pointer.x > self.x_max {
            step.x = -self.x_speed;
        }
        if pointer.y < self.y_min {
            step.y = self.y_speed;
        }
        if pointer.y > self.y_max {
            step.y = -self.y_speed;
        }
        step
    }
}

/// The constants the grid scrolls by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridConstants {
    pub min_scale: f32,
    pub max_scale: f32,
    /// Two scales closer than this are the same scale.
    pub close_enough: f32,
    pub hover: HoverScroll,
    /// The movie's frame rate, which the per-frame hover scroll runs at.
    pub frame_rate: f32,
    pub max_x: f32,
    pub max_y: f32,
    pub texture_width: f32,
    /// Where the bottom poster image starts.
    pub top_height: f32,
    /// Height of the poster's non-empty part (top plus bottom image).
    pub content_height: f32,
    pub border_width: f32,
    pub border_height: f32,
    /// How far the grid moves per pixel the pointer drags.
    pub drag_speed: f32,
}

impl GridConstants {
    /// The scale one wheel notch adds.
    pub fn zoom_per_notch(&self) -> f32 {
        (self.max_scale - self.min_scale) / WHEEL_NOTCHES_FULL_ZOOM
    }
}

/// Where the grid sits on the stage: its scale and the stage position of its origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridView {
    pub scale: f32,
    pub x: f32,
    pub y: f32,
}

impl GridView {
    /// The view the chart opens in: smallest scale, moved by the maximum offsets.
    pub fn initial(constants: &GridConstants) -> Self {
        let mut view = Self {
            scale: constants.min_scale,
            x: 0.0,
            y: 0.0,
        };
        view.move_by(constants, Vec2::new(constants.max_x, constants.max_y));
        view
    }

    fn min_x(&self, constants: &GridConstants) -> f32 {
        -(constants.texture_width * self.scale) + constants.border_width
    }

    fn min_y(&self, constants: &GridConstants) -> f32 {
        -(constants.content_height * self.scale) + constants.border_height
    }

    /// Moves the grid by `delta` stage pixels, keeping the poster over the whole stage.
    pub fn move_by(&mut self, constants: &GridConstants, delta: Vec2) {
        self.x = (self.x + delta.x)
            .max(self.min_x(constants))
            .min(constants.max_x);
        self.y = (self.y + delta.y)
            .max(self.min_y(constants))
            .min(constants.max_y);
    }

    /// Zooms by `delta` about the grid point under `pointer` (stage pixels), which stays under it
    /// as far as the scroll limits allow. Returns whether the scale changed.
    pub fn zoom(&mut self, constants: &GridConstants, delta: f32, pointer: Pos2) -> bool {
        let close = |a: f32, b: f32| (a - b).abs() < constants.close_enough;
        let old = self.scale;
        let blocked = if delta > 0.0 {
            close(old, constants.max_scale) || old >= constants.max_scale
        } else if delta < 0.0 {
            close(old, constants.min_scale) || old <= constants.min_scale
        } else {
            true
        };
        if blocked {
            return false;
        }
        let anchor = Vec2::new((pointer.x - self.x) / old, (pointer.y - self.y) / old);
        let new = (old + delta).clamp(constants.min_scale, constants.max_scale);
        if close(new, old) {
            return false;
        }
        self.scale = new;
        let target = Vec2::new(pointer.x - anchor.x * new, pointer.y - anchor.y * new);
        self.move_by(constants, target - Vec2::new(self.x, self.y));
        true
    }

    /// A position in grid units as a stage position.
    pub fn to_stage(self, grid: Pos2) -> Pos2 {
        Pos2::new(self.x + grid.x * self.scale, self.y + grid.y * self.scale)
    }
}

/// The rank whose text the info panel shows: the owned rank plus the viewing offset.
pub fn shown_rank(owned: u8, offset: i8, max_rank: u8) -> u8 {
    let rank = i16::from(owned) + i16::from(offset);
    u8::try_from(rank.clamp(1, i16::from(max_rank.max(1)))).unwrap_or(1)
}

/// The rank-viewing offset a fresh selection starts with: the owned rank when everything is
/// owned, the next rank when nothing is owned or points are spent.
pub fn initial_offset(owned: u8, max_rank: u8, unspent_points: u32) -> i8 {
    if owned == max_rank {
        0
    } else {
        i8::from(owned == 0 || unspent_points > 0)
    }
}

/// Whether the previous-rank step is possible.
pub fn can_view_previous(owned: u8, offset: i8, max_rank: u8) -> bool {
    max_rank > 1 && i16::from(owned) + i16::from(offset) - 1 != 0
}

/// Whether the next-rank step is possible.
pub fn can_view_next(owned: u8, offset: i8, max_rank: u8) -> bool {
    max_rank > 1 && i16::from(owned) + i16::from(offset) < i16::from(max_rank)
}

/// A rank star's look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RankStar {
    Empty,
    Full,
    Available,
}

/// The star of rank index `index` in the info panel when `viewed_rank` ranks count as held.
/// `available_slot` marks the star the next point would fill, if any.
pub fn info_star(index: u8, held: u8, available_slot: bool) -> RankStar {
    if available_slot && index == held {
        RankStar::Available
    } else if index < held {
        RankStar::Full
    } else {
        RankStar::Empty
    }
}

/// The stars a card shows in the grid: only held ranks and the next one when it can be taken.
/// Perks with one rank show their star only once it is held.
pub fn grid_stars(max_rank: u8, held: u8, available_slot: bool) -> Vec<RankStar> {
    if max_rank <= 1 && held != 1 {
        return Vec::new();
    }
    (0..max_rank)
        .filter_map(|index| {
            let available = available_slot && held == index && held != 0;
            let full = index < held;
            if available {
                Some(RankStar::Available)
            } else if full {
                Some(RankStar::Full)
            } else {
                None
            }
        })
        .collect()
}

/// How many stars a header counter draws for a SPECIAL value.
pub fn special_star_count(value: u8, cap: u8) -> u8 {
    value.min(cap)
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact literals")]
mod tests {
    use super::*;
    use crate::sim::pipboy::perks::{PerkChartSlot, PerkId, PerkRank};

    fn constants() -> GridConstants {
        GridConstants {
            min_scale: 0.5,
            max_scale: 1.0,
            close_enough: 0.001,
            frame_rate: 30.0,
            hover: HoverScroll {
                x_min: 30.0,
                x_max: 1200.0,
                y_min: 12.0,
                y_max: 700.0,
                x_speed: 20.0,
                y_speed: 10.0,
            },
            max_x: 0.0,
            max_y: 0.0,
            texture_width: 2560.0,
            top_height: 1500.0,
            content_height: 3000.0,
            border_width: 1280.0,
            border_height: 720.0,
            drag_speed: 1.0,
        }
    }

    fn perk(name: &str, special: Special, row: u8, sex: Option<Sex>) -> PerkDefinition {
        PerkDefinition {
            id: PerkId(name.to_owned()),
            name: name.to_owned(),
            ranks: vec![PerkRank {
                form_key: name.to_owned(),
                description: String::new(),
                required_level: 1,
            }],
            vault_boy_swf: None,
            chart: Some(PerkChartSlot {
                special,
                required_special: row,
            }),
            sex,
        }
    }

    #[test]
    fn a_cell_holds_the_perk_of_its_special_and_rank_row() {
        let perks = [
            perk("Iron Fist", Special::Strength, 1, None),
            perk("Big Leagues", Special::Strength, 2, None),
        ];
        let found = perk_in_cell(&perks, Sex::Male, Cell { column: 0, row: 2 });
        assert_eq!(found.map(|p| p.name.as_str()), Some("Big Leagues"));
        assert!(perk_in_cell(&perks, Sex::Male, Cell { column: 1, row: 2 }).is_none());
    }

    #[test]
    fn the_special_row_holds_no_perk() {
        let perks = [perk("Iron Fist", Special::Strength, 1, None)];
        assert!(perk_in_cell(&perks, Sex::Male, Cell { column: 0, row: 0 }).is_none());
    }

    #[test]
    fn a_shared_cell_shows_the_perk_of_the_characters_sex() {
        let perks = [
            perk("Black Widow", Special::Charisma, 2, Some(Sex::Female)),
            perk("Lady Killer", Special::Charisma, 2, Some(Sex::Male)),
        ];
        let cell = Cell { column: 3, row: 2 };
        let name = |sex| perk_in_cell(&perks, sex, cell).map(|p| p.name.clone());
        assert_eq!(name(Sex::Female), Some("Black Widow".to_owned()));
        assert_eq!(name(Sex::Male), Some("Lady Killer".to_owned()));
    }

    #[test]
    fn a_perk_without_a_chart_slot_never_appears() {
        let mut magazine = perk("Magazine", Special::Luck, 1, None);
        magazine.chart = None;
        assert!(perk_in_cell(&[magazine], Sex::Male, Cell { column: 6, row: 1 }).is_none());
    }

    #[test]
    fn every_cell_has_its_own_index_in_row_major_order() {
        assert_eq!(Cell::ALL.len(), 77);
        assert!(
            Cell::ALL
                .iter()
                .enumerate()
                .all(|(i, cell)| cell.index() == i)
        );
        assert_eq!(Cell::ALL[COLUMNS], Cell { column: 0, row: 1 });
    }

    #[test]
    fn the_chart_opens_at_the_smallest_scale_in_the_top_left() {
        let view = GridView::initial(&constants());
        assert_eq!((view.scale, view.x, view.y), (0.5, 0.0, 0.0));
    }

    #[test]
    fn scrolling_stays_inside_the_poster_and_does_not_move_sideways_at_the_smallest_scale() {
        let constants = constants();
        let mut view = GridView::initial(&constants);
        view.move_by(&constants, Vec2::new(-500.0, -5000.0));
        assert_eq!(view.x, 0.0, "the poster is exactly as wide as the stage");
        assert!((view.y - (-(3000.0 * 0.5) + 720.0)).abs() < 1e-3);
        view.move_by(&constants, Vec2::new(300.0, 9000.0));
        assert_eq!((view.x, view.y), (0.0, 0.0));
    }

    #[test]
    fn a_zoom_keeps_the_grid_point_under_the_pointer_where_the_limits_allow() {
        let constants = constants();
        let mut view = GridView {
            scale: 0.6,
            x: -300.0,
            y: -400.0,
        };
        let pointer = Pos2::new(500.0, 300.0);
        let anchor = Pos2::new(
            (pointer.x - view.x) / view.scale,
            (pointer.y - view.y) / view.scale,
        );

        assert!(view.zoom(&constants, 0.1, pointer));

        assert!((view.scale - 0.7).abs() < 1e-6);
        assert!((view.to_stage(anchor) - pointer).length() < 1e-3);
    }

    #[test]
    fn a_zoom_is_clamped_to_the_scale_range_and_then_to_the_scroll_limits() {
        let constants = constants();
        let mut view = GridView {
            scale: 0.9,
            x: -100.0,
            y: -100.0,
        };

        assert!(view.zoom(&constants, 5.0, Pos2::new(640.0, 360.0)));
        assert_eq!(view.scale, constants.max_scale);
        assert!(!view.zoom(&constants, 0.1, Pos2::new(640.0, 360.0)));

        assert!(view.zoom(&constants, -5.0, Pos2::new(1280.0, 720.0)));
        assert_eq!(view.scale, constants.min_scale);
        assert!(!view.zoom(&constants, -0.1, Pos2::new(640.0, 360.0)));
        assert!(view.x <= constants.max_x && view.y <= constants.max_y);
        assert!(view.x >= -(constants.texture_width * view.scale) + constants.border_width);
        assert!(view.y >= -(constants.content_height * view.scale) + constants.border_height);
    }

    #[test]
    fn a_zero_zoom_changes_nothing() {
        let constants = constants();
        let mut view = GridView::initial(&constants);
        assert!(!view.zoom(&constants, 0.0, Pos2::new(10.0, 10.0)));
        assert_eq!(view, GridView::initial(&constants));
    }

    #[test]
    fn a_notch_is_a_sixth_of_the_scale_range() {
        let constants = constants();
        assert!((constants.zoom_per_notch() - (1.0 - 0.5) / 6.0).abs() < 1e-6);
    }

    #[test]
    fn the_edge_zones_push_the_grid_toward_the_centre() {
        let hover = constants().hover;
        let centre = Pos2::new(640.0, 360.0);
        assert_eq!(hover.step(centre), Vec2::ZERO);
        assert_eq!(
            hover.step(Pos2::new(hover.x_min - 1.0, centre.y)),
            Vec2::new(hover.x_speed, 0.0)
        );
        assert_eq!(
            hover.step(Pos2::new(hover.x_max + 1.0, centre.y)),
            Vec2::new(-hover.x_speed, 0.0)
        );
        assert_eq!(
            hover.step(Pos2::new(centre.x, hover.y_min - 1.0)),
            Vec2::new(0.0, hover.y_speed)
        );
        assert_eq!(
            hover.step(Pos2::new(centre.x, hover.y_max + 1.0)),
            Vec2::new(0.0, -hover.y_speed)
        );
        assert_eq!(
            hover.step(Pos2::new(hover.x_min - 1.0, hover.y_max + 1.0)),
            Vec2::new(hover.x_speed, -hover.y_speed)
        );
        assert_eq!(hover.step(Pos2::new(hover.x_min, hover.y_min)), Vec2::ZERO);
    }

    #[test]
    fn grid_positions_map_to_the_stage_by_scale_and_offset() {
        let view = GridView {
            scale: 0.8,
            x: -100.0,
            y: -40.0,
        };
        let stage = view.to_stage(Pos2::new(500.0, 300.0));
        assert!((stage - Pos2::new(300.0, 200.0)).length() < 1e-3);
    }

    #[test]
    fn a_fresh_selection_views_the_next_rank_unless_everything_is_owned_or_nothing_can_be_taken() {
        assert_eq!(initial_offset(3, 3, 5), 0);
        assert_eq!(initial_offset(0, 3, 0), 1);
        assert_eq!(initial_offset(1, 3, 2), 1);
        assert_eq!(initial_offset(1, 3, 0), 0);
    }

    #[test]
    fn the_shown_rank_follows_the_offset_inside_the_perks_ranks() {
        assert_eq!(shown_rank(0, 1, 3), 1);
        assert_eq!(shown_rank(2, 1, 3), 3);
        assert_eq!(shown_rank(3, 0, 3), 3);
        assert_eq!(shown_rank(3, 1, 3), 3, "never past the last rank");
        assert_eq!(shown_rank(0, 0, 3), 1, "never below the first rank");
    }

    #[test]
    fn rank_viewing_stops_at_the_first_and_last_rank_and_needs_several_ranks() {
        assert!(!can_view_previous(0, 1, 3));
        assert!(can_view_previous(1, 1, 3));
        assert!(can_view_next(0, 1, 3));
        assert!(!can_view_next(2, 1, 3));
        assert!(!can_view_previous(1, 1, 1) && !can_view_next(0, 0, 1));
    }

    #[test]
    fn info_stars_are_full_below_the_held_rank_and_the_next_one_is_available_when_it_can_be_taken()
    {
        let stars: Vec<_> = (0..3).map(|i| info_star(i, 1, true)).collect();
        assert_eq!(
            stars,
            [RankStar::Full, RankStar::Available, RankStar::Empty]
        );
        let blocked: Vec<_> = (0..3).map(|i| info_star(i, 1, false)).collect();
        assert_eq!(blocked, [RankStar::Full, RankStar::Empty, RankStar::Empty]);
    }

    #[test]
    fn grid_stars_show_only_held_ranks_and_the_next_available_one() {
        assert_eq!(
            grid_stars(3, 1, true),
            [RankStar::Full, RankStar::Available]
        );
        assert_eq!(grid_stars(3, 2, false), [RankStar::Full, RankStar::Full]);
        assert!(
            grid_stars(3, 0, true).is_empty(),
            "nothing held, nothing shown"
        );
    }

    #[test]
    fn a_single_rank_perk_shows_its_star_only_when_held() {
        assert_eq!(grid_stars(1, 1, false), [RankStar::Full]);
        assert_eq!(grid_stars(1, 0, true), []);
    }

    #[test]
    fn header_counters_never_draw_more_than_ten_stars() {
        assert_eq!(special_star_count(7, 10), 7);
        assert_eq!(special_star_count(10, 10), 10);
        assert_eq!(special_star_count(15, 10), 10);
    }
}
