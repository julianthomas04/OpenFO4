//! The level box in the lower left: the level, the experience meter and the brackets around them.

use egui::{Pos2, Rect, Vec2};

use super::frame::{brackets, outline_rect};
use super::layout::{ChartLayout, XpLayout, rgba};
use crate::rendering::pipboy::bottom_bar::xp_fraction;
use crate::rendering::pipboy::paint::{fill, tinted};
use crate::rendering::pipboy::scaleform_text::paint_field;
use crate::rendering::pipboy::tab::TabContext;
use crate::sim::pipboy::PipBoyModel;

fn holder_rect(xp: &XpLayout) -> Rect {
    let [left, top, right, bottom] = xp.bounds;
    Rect::from_min_max(
        Pos2::new(xp.holder.x + left, xp.holder.y + top),
        Pos2::new(xp.holder.x + right, xp.holder.y + bottom),
    )
}

/// A shape's rectangle inside the meter, scaled by the meter's own scale.
fn in_meter(xp: &XpLayout, origin: Vec2, scale: Vec2, bounds: [f32; 4]) -> Rect {
    let place = |x: f32, y: f32| {
        Pos2::new(
            xp.holder.x + xp.meter_origin.x + xp.meter_scale.x * (origin.x + scale.x * x),
            xp.holder.y + xp.meter_origin.y + xp.meter_scale.y * (origin.y + scale.y * y),
        )
    };
    Rect::from_min_max(place(bounds[0], bounds[1]), place(bounds[2], bounds[3]))
}

/// The meter's outline rectangle and its edge thicknesses.
fn meter_outline(xp: &XpLayout) -> (Rect, Vec2) {
    let path = in_meter(xp, Vec2::ZERO, Vec2::splat(1.0), xp.outline_path);
    (path, xp.meter_scale * xp.outline_width)
}

/// The meter's fill for the experience `fraction` of the level.
fn meter_fill(xp: &XpLayout, fraction: f32) -> Rect {
    let full = in_meter(
        xp,
        xp.meter_fill_origin,
        xp.meter_fill_scale,
        xp.meter_fill_bounds,
    );
    Rect::from_min_size(full.min, Vec2::new(full.width() * fraction, full.height()))
}

pub fn draw(cx: &TabContext<'_>, layout: &ChartLayout, model: &PipBoyModel) {
    let xp = &layout.xp;
    let painter = cx.painter;
    let progression = model.character.progression;
    let holder = holder_rect(xp);
    fill(painter, holder, tinted(rgba(xp.fill), cx.tint));
    brackets(
        painter,
        holder.expand2(xp.brackets.padding),
        xp.brackets.line_width,
        xp.brackets.corner,
        cx.shade(1.0),
    );

    paint_field(
        painter,
        &xp.text.geometry(),
        &format!("{} {}", xp.level_word, progression.level),
        xp.text.style(cx.tint).shrinking(),
    );

    let (outline, thickness) = meter_outline(xp);
    outline_rect(painter, outline, thickness, cx.shade(1.0));
    let fraction = xp_fraction(progression.xp_into_level, progression.xp_for_next_level);
    fill(painter, meter_fill(xp, fraction), cx.shade(1.0));
}

#[cfg(test)]
mod tests {
    use egui::Color32;

    use super::*;
    use crate::rendering::pipboy::fonts::Weight;
    use crate::rendering::pipboy::pages::perk_chart::layout::Text;
    use crate::rendering::pipboy::pages::perk_chart::layout::xp::BracketLayout;
    use crate::ui_layout::test_support::text_field;

    fn layout() -> XpLayout {
        XpLayout {
            holder: Vec2::new(50.0, 645.0),
            bounds: [0.0, 0.0, 240.0, 46.0],
            fill: [51, 51, 51, 178],
            text: Text {
                field: text_field([-2.0, -2.0, 102.0, 24.8], 19.0),
                weight: Weight::Regular,
                colour: Color32::WHITE,
                metrics: crate::ui_layout::test_support::FIXTURE_METRICS,
            },
            level_word: "LEVEL".to_owned(),
            brackets: BracketLayout {
                line_width: 1.5,
                corner: 6.0,
                padding: Vec2::ZERO,
            },
            meter_origin: Vec2::new(8.55, 26.4),
            meter_scale: Vec2::new(1.0949, 0.44),
            outline_path: [0.0, 0.0, 200.0, 25.0],
            outline_width: 2.0,
            meter_fill_origin: Vec2::ZERO,
            meter_fill_scale: Vec2::new(2.0768, 0.2604),
            meter_fill_bounds: [0.0, 0.0, 96.3, 95.95],
        }
    }

    #[test]
    fn the_holder_covers_its_authored_box() {
        let rect = holder_rect(&layout());
        assert_eq!(
            (rect.min, rect.max),
            (Pos2::new(50.0, 645.0), Pos2::new(290.0, 691.0))
        );
    }

    #[test]
    fn the_meter_outline_sits_where_the_movie_puts_it() {
        let (outline, thickness) = meter_outline(&layout());
        assert!((outline.min.x - 58.55).abs() < 0.01);
        assert!((outline.width() - 200.0 * 1.0949).abs() < 0.01);
        assert!((thickness.x - 2.19).abs() < 0.01 && (thickness.y - 0.88).abs() < 0.01);
    }

    #[test]
    fn the_fill_grows_from_the_left_with_the_experience_share() {
        let full = meter_fill(&layout(), 1.0);
        let half = meter_fill(&layout(), 0.5);
        assert_eq!(full.min, half.min);
        assert!((half.width() - full.width() / 2.0).abs() < 1e-3);
        assert!((full.width() - 200.0 * 1.0949).abs() < 0.05);
    }
}
