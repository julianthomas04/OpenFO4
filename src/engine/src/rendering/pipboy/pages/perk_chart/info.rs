//! The info panel under or over the chart: the menu's layout rule for where the panel sits, how
//! wide and tall its background is and where the rank stars and the lines under the description
//! go, and the painting of it.

use egui::{Pos2, Rect, Vec2};

use super::frame::outline_rect;
use super::layout::{ChartLayout, InfoLayout, InfoRules, Text, rgba};
use super::stars::draw_rank_star;
use crate::rendering::pipboy::pages::perk_chart_layout::{Cell, RankStar, info_star};
use crate::rendering::pipboy::paint;
use crate::rendering::pipboy::scaleform_text::{lay_out, paint_field};
use crate::rendering::pipboy::tab::TabContext;
use crate::ui_layout::scaleform_text::FieldGeometry;

/// The panel background's x and width as of the previous selection; the star holder centres on
/// them before the background takes its new geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelMemory {
    pub background_x: f32,
    pub background_width: f32,
}

impl PanelMemory {
    pub fn authored(rules: &InfoRules) -> Self {
        let background = &rules.background;
        Self {
            background_x: background.origin.x,
            background_width: background.bounds[2] - background.bounds[0],
        }
    }
}

/// The panel background's geometry per selection or rank change: the star holder centres on the
/// geometry the background had before the latest change, however often the panel is redrawn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PanelHistory {
    shown: Option<(Cell, u8)>,
    before: Option<PanelMemory>,
    latest: Option<PanelMemory>,
}

impl PanelHistory {
    /// The memory the star holder centres on while `shown` (the selected cell and viewed rank)
    /// is on screen.
    pub fn before(&mut self, shown: (Cell, u8), rules: &InfoRules) -> PanelMemory {
        if self.shown != Some(shown) {
            self.before = self.latest;
            self.shown = Some(shown);
        }
        self.before.unwrap_or_else(|| PanelMemory::authored(rules))
    }

    pub fn record(&mut self, memory: PanelMemory) {
        self.latest = Some(memory);
    }
}

/// What the description text measures: its height and where its first line starts and how wide it
/// is, in stage pixels at the panel's authored place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextMeasure {
    pub height: f32,
    pub first_line_left: f32,
    pub first_line_width: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlanInput {
    pub measure: TextMeasure,
    /// The rank stars the panel shows; none for a perk with a single rank.
    pub star_count: usize,
    pub star_width: f32,
    /// The selected card's y on the stage.
    pub selected_stage_y: f32,
}

/// Where everything of the panel goes on the stage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelPlan {
    /// How far the panel moved down from its authored place.
    pub shift_y: f32,
    pub height: f32,
    /// The background's outline rectangle and the thickness of its vertical and horizontal edges.
    pub outline: Rect,
    pub edge_thickness: Vec2,
    pub star_holder: Pos2,
    pub requirements_shift_y: f32,
    pub confirm_shift_y: f32,
    pub memory: PanelMemory,
}

pub fn plan(rules: &InfoRules, input: &PlanInput, previous: PanelMemory) -> PanelPlan {
    let star_y = rules.description_y + input.measure.height + rules.star_gap;
    let requirements_y = star_y + rules.requirements_gap;
    let confirm_y = star_y + rules.confirm_gap;
    let height = requirements_y - rules.name_y + rules.height_padding;

    let first_line_left = input.measure.first_line_left - rules.panel.x;
    let background_x = first_line_left - rules.width_padding / rules.width_halves;
    let background_width = input.measure.first_line_width + rules.width_padding;

    let holder_width =
        rules.star_pitch * input.star_count.saturating_sub(1) as f32 + input.star_width;
    let holder_x = previous.background_x
        + (previous.background_width - holder_width) / rules.star_centre_halves;

    let panel_y = if input.selected_stage_y > rules.y_bound {
        rules.upper_y
    } else {
        rules.lower_y - height
    };
    let origin = Pos2::new(rules.panel.x, panel_y);

    let background = &rules.background;
    let [left, top, right, bottom] = background.bounds;
    let scale = Vec2::new(background_width / (right - left), height / (bottom - top));
    let [path_left, path_top, path_right, path_bottom] = background.edge_bounds;
    let outline = Rect::from_min_max(
        origin
            + Vec2::new(
                background_x + scale.x * path_left,
                background.origin.y + scale.y * path_top,
            ),
        origin
            + Vec2::new(
                background_x + scale.x * path_right,
                background.origin.y + scale.y * path_bottom,
            ),
    );

    PanelPlan {
        shift_y: panel_y - rules.panel.y,
        height,
        outline,
        edge_thickness: scale * background.line_width,
        star_holder: Pos2::new(rules.panel.x + holder_x, panel_y + star_y),
        requirements_shift_y: panel_y - rules.panel.y + requirements_y - rules.requirements_y,
        confirm_shift_y: panel_y - rules.panel.y + confirm_y - rules.confirm_y,
        memory: PanelMemory {
            background_x,
            background_width,
        },
    }
}

fn shifted(geometry: FieldGeometry, dy: f32) -> FieldGeometry {
    FieldGeometry {
        top: geometry.top + dy,
        bottom: geometry.bottom + dy,
        ..geometry
    }
}

/// Measures the description the way the menu does before it places the rest of the panel.
pub fn measure(cx: &TabContext<'_>, info: &InfoLayout, text: &str) -> TextMeasure {
    let geometry = info.description.geometry();
    let galley = lay_out(cx.painter, text, &geometry, info.description.style(cx.tint));
    let first_line = galley.rows.first().map(egui::epaint::text::PlacedRow::rect);
    TextMeasure {
        height: galley.rect.height(),
        first_line_left: first_line
            .map_or(geometry.anchor_x(), |rect| geometry.anchor_x() + rect.min.x),
        first_line_width: first_line.map_or(0.0, |rect| rect.width()),
    }
}

/// What the panel shows for the selected perk.
pub struct PanelContent<'a> {
    pub name: &'a str,
    pub description: &'a str,
    /// The requirement line under the stars; empty when the rank needs none shown.
    pub requirements: &'a str,
    /// The line that replaces the requirements while confirming or after a refusal.
    pub notice: Option<&'a str>,
    pub stars: &'a [RankStar],
}

fn paint_text_in(cx: &TabContext<'_>, field: &Text, shift_y: f32, text: &str, shrink: bool) {
    let style = if shrink {
        field.style(cx.tint).shrinking()
    } else {
        field.style(cx.tint)
    };
    paint_field(cx.painter, &shifted(field.geometry(), shift_y), text, style);
}

fn paint_background(cx: &TabContext<'_>, layout: &ChartLayout, plan: &PanelPlan) {
    paint::fill(
        cx.painter,
        plan.outline,
        paint::tinted(rgba(layout.info.rules.background.fill), cx.tint),
    );
    outline_rect(cx.painter, plan.outline, plan.edge_thickness, cx.shade(1.0));
}

pub fn draw(
    cx: &mut TabContext<'_>,
    layout: &ChartLayout,
    plan: &PanelPlan,
    content: &PanelContent<'_>,
) {
    paint_background(cx, layout, plan);
    let info = &layout.info;
    paint_text_in(cx, &info.name, plan.shift_y, content.name, false);
    paint_text_in(
        cx,
        &info.description,
        plan.shift_y,
        content.description,
        false,
    );
    if !content.requirements.is_empty() {
        paint_text_in(
            cx,
            &info.requirements,
            plan.requirements_shift_y,
            content.requirements,
            false,
        );
    }
    if let Some(notice) = content.notice {
        paint_text_in(cx, &info.confirm, plan.confirm_shift_y, notice, false);
    }
    let size = layout.rank_star_size * info.rules.star_scale;
    for (index, star) in content.stars.iter().enumerate() {
        let at = plan.star_holder + Vec2::new(info.rules.star_pitch * index as f32, 0.0);
        draw_rank_star(cx, layout, at, size, *star);
    }
}

/// The star states of the panel for a perk with `max_rank` ranks.
pub fn panel_stars(max_rank: u8, held: u8, available_slot: bool) -> Vec<RankStar> {
    if max_rank <= 1 {
        return Vec::new();
    }
    (0..max_rank)
        .map(|index| info_star(index, held, available_slot))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::pipboy::pages::perk_chart::layout::info::BackgroundLayout;

    fn rules() -> InfoRules {
        InfoRules {
            panel: Vec2::new(-12.95, 511.1),
            background: BackgroundLayout {
                origin: Vec2::new(115.0, 10.0),
                bounds: [-1.0, -1.0, 1072.0, 131.75],
                edge_bounds: [0.0, 0.0, 1071.0, 130.75],
                fill: [51, 51, 51, 178],
                line_width: 2.0,
            },
            name_y: 14.25,
            description_y: 46.0,
            requirements_y: 117.0,
            confirm_y: 112.0,
            star_gap: 4.0,
            requirements_gap: 43.0,
            confirm_gap: 35.0,
            width_padding: 100.0,
            width_halves: 2.0,
            height_padding: 30.0,
            star_pitch: 22.5,
            star_scale: 0.5,
            star_centre_halves: 2.0,
            lower_y: 617.75,
            upper_y: 35.0,
            y_bound: 460.0,
            entry_y_offset: 46.75,
        }
    }

    fn input(selected_stage_y: f32) -> PlanInput {
        PlanInput {
            measure: TextMeasure {
                height: 48.0,
                first_line_left: 400.0,
                first_line_width: 300.0,
            },
            star_count: 3,
            star_width: 20.0,
            selected_stage_y,
        }
    }

    fn previous() -> PanelMemory {
        PanelMemory {
            background_x: 100.0,
            background_width: 400.0,
        }
    }

    #[test]
    fn the_panel_is_as_tall_as_its_text_needs() {
        let plan = plan(&rules(), &input(100.0), previous());
        assert!(
            (plan.height - (48.0 + 108.75)).abs() < 1e-3,
            "{}",
            plan.height
        );
    }

    #[test]
    fn a_selection_in_the_lower_part_of_the_stage_puts_the_panel_on_top() {
        let top = plan(&rules(), &input(461.0), previous());
        assert!((top.shift_y - (35.0 - 511.1)).abs() < 1e-3);
        let bottom = plan(&rules(), &input(460.0), previous());
        assert!((bottom.shift_y - (617.75 - bottom.height - 511.1)).abs() < 1e-3);
    }

    #[test]
    fn the_background_hugs_the_first_text_line() {
        let plan = plan(&rules(), &input(100.0), previous());
        let remembered = plan.memory;
        assert!((remembered.background_width - 400.0).abs() < 1e-3);
        assert!((remembered.background_x - (400.0 + 12.95 - 50.0)).abs() < 1e-3);
        let expected_left = -12.95 + remembered.background_x;
        assert!((plan.outline.min.x - expected_left).abs() < 1e-3);
    }

    #[test]
    fn the_star_holder_centres_on_the_previous_background() {
        let plan = plan(&rules(), &input(100.0), previous());
        let holder_width = 22.5 * 2.0 + 20.0;
        let expected = -12.95 + 100.0 + (400.0 - holder_width) / 2.0;
        assert!((plan.star_holder.x - expected).abs() < 1e-3);
    }

    #[test]
    fn the_lines_below_the_stars_follow_the_text_height() {
        let plan = plan(&rules(), &input(100.0), previous());
        let star_y = 46.0 + 48.0 + 4.0;
        assert!((plan.requirements_shift_y - (plan.shift_y + star_y + 43.0 - 117.0)).abs() < 1e-3);
        assert!((plan.confirm_shift_y - (plan.shift_y + star_y + 35.0 - 112.0)).abs() < 1e-3);
    }

    #[test]
    fn the_outline_scales_with_the_background() {
        let plan = plan(&rules(), &input(100.0), previous());
        let sy = plan.height / (131.75 + 1.0);
        assert!((plan.edge_thickness.y - 2.0 * sy).abs() < 1e-3);
        assert!((plan.edge_thickness.x - 2.0 * 400.0 / 1073.0).abs() < 1e-3);
    }

    #[test]
    fn a_single_rank_perk_shows_no_stars_in_the_panel() {
        assert_eq!(panel_stars(1, 1, false), []);
        assert_eq!(
            panel_stars(3, 1, true),
            [RankStar::Full, RankStar::Available, RankStar::Empty]
        );
    }
}
