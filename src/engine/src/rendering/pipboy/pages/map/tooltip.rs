//! The info tooltip of the selected map marker: its name over a background that grows to fit,
//! and for a workshop settlement a row with its population and happiness inside the same
//! background.

use egui::{Pos2, Rect, Vec2};

use super::layout::MOVIE;
use crate::assets::pipboy_art::{FixedArt, Icon};
use crate::rendering::pipboy::scaleform_text::{
    FieldSpec, TextAutoSize, natural_width, text_height,
};
use crate::rendering::pipboy::shapes::OutlinedShape;
use crate::rendering::pipboy::tab::TabContext;
use crate::sim::pipboy::{MapMarker, Settlement};
use crate::ui_layout::scaleform_text::AutoSize;
use crate::ui_layout::{
    LiteralKey, PlacementKey, ShapeKey, TextKey, UiLayout, UiLayoutError, key_check_fields,
};

pub struct TooltipKeys {
    background: ShapeKey,
    location: TextKey,
    population_icon: PlacementKey,
    happy_icon: PlacementKey,
    population: TextKey,
    happy: TextKey,
    padding: LiteralKey,
    extra_height: LiteralKey,
    row_gap: LiteralKey,
    halve: LiteralKey,
    row_x: LiteralKey,
    text_gap: LiteralKey,
    icon_gap: LiteralKey,
    percent: LiteralKey,
}

pub static KEYS: TooltipKeys = TooltipKeys {
    background: ShapeKey::new(MOVIE, "[SelectionInfo]/Background_mc/@1"),
    location: TextKey::new(MOVIE, "[SelectionInfo]/Location_tf"),
    population_icon: PlacementKey::new(MOVIE, "[Selection_WorkshopInfo]/PopulationIcon_mc"),
    happy_icon: PlacementKey::new(MOVIE, "[Selection_WorkshopInfo]/HappyIcon_mc"),
    population: TextKey::new(MOVIE, "[Selection_WorkshopInfo]/Population_tf"),
    happy: TextKey::new(MOVIE, "[Selection_WorkshopInfo]/Happy_tf"),
    padding: LiteralKey::new(MOVIE, "SelectionInfo.redrawUIComponent#$1+", 1),
    extra_height: LiteralKey::new(
        MOVIE,
        "SelectionInfo.redrawUIComponent#(Location_tf.height+(WorkshopInfo_mc.visible?WorkshopInfo_mc.height:0))+",
        1,
    ),
    row_gap: LiteralKey::new(
        MOVIE,
        "SelectionInfo.redrawUIComponent#(Location_tf.y+Location_tf.height)+",
        1,
    ),
    halve: LiteralKey::new(
        MOVIE,
        "SelectionInfo.redrawUIComponent#-(Background_mc.width)/",
        1,
    ),
    row_x: LiteralKey::new(
        MOVIE,
        "SelectionInfo.redrawUIComponent#WorkshopInfo_mc.x=",
        1,
    ),
    text_gap: LiteralKey::new(
        MOVIE,
        "Selection_WorkshopInfo.redrawUIComponent#(Population_tf.x+$1.width)+",
        1,
    ),
    icon_gap: LiteralKey::new(
        MOVIE,
        "Selection_WorkshopInfo.redrawUIComponent#(HappyIcon_mc.x+HappyIcon_mc.width)+",
        1,
    ),
    percent: LiteralKey::texts(
        MOVIE,
        "Selection_WorkshopInfo.redrawUIComponent#_HappyPct.toString()+",
        1,
    ),
};

key_check_fields!(TooltipKeys {
    background,
    location,
    population_icon,
    happy_icon,
    population,
    happy,
    padding,
    extra_height,
    row_gap,
    halve,
    row_x,
    text_gap,
    icon_gap,
    percent
});

/// The icon placement in the workshop row: where its registration point sits and its symbol scale.
#[derive(Clone, Copy)]
struct IconPlacement {
    at: Vec2,
    scale: f32,
}

pub struct TooltipLayout {
    background: OutlinedShape,
    location: FieldSpec,
    location_y: f32,
    population_icon: IconPlacement,
    happy_icon: IconPlacement,
    population: FieldSpec,
    population_x: f32,
    happy: FieldSpec,
    happy_x: f32,
    padding: f32,
    extra_height: f32,
    row_gap: f32,
    halve: f32,
    row_x: f32,
    text_gap: f32,
    icon_gap: f32,
    percent: String,
}

fn icon(layout: &UiLayout, key: &PlacementKey) -> Result<IconPlacement, UiLayoutError> {
    let matrix = layout.get(key)?.world.matrix;
    Ok(IconPlacement {
        at: Vec2::new(matrix.tx, matrix.ty),
        scale: matrix.a,
    })
}

fn placement_of(layout: &UiLayout, key: &TextKey) -> Result<Vec2, UiLayoutError> {
    let matrix = layout.get(key)?.placement.world.matrix;
    Ok(Vec2::new(matrix.tx, matrix.ty))
}

fn literal(layout: &UiLayout, key: &LiteralKey) -> Result<f32, UiLayoutError> {
    Ok(layout.lit_number(key, 0)? as f32)
}

impl TooltipLayout {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        Ok(Self {
            background: OutlinedShape::resolve(layout, &KEYS.background)?,
            location: FieldSpec::resolve(layout, &KEYS.location, TextAutoSize::None)?
                .growing(AutoSize::Center),
            location_y: placement_of(layout, &KEYS.location)?.y,
            population_icon: icon(layout, &KEYS.population_icon)?,
            happy_icon: icon(layout, &KEYS.happy_icon)?,
            population: FieldSpec::resolve(layout, &KEYS.population, TextAutoSize::None)?,
            population_x: placement_of(layout, &KEYS.population)?.x,
            happy: FieldSpec::resolve(layout, &KEYS.happy, TextAutoSize::None)?,
            happy_x: placement_of(layout, &KEYS.happy)?.x,
            padding: literal(layout, &KEYS.padding)?,
            extra_height: literal(layout, &KEYS.extra_height)?,
            row_gap: literal(layout, &KEYS.row_gap)?,
            halve: literal(layout, &KEYS.halve)?,
            row_x: literal(layout, &KEYS.row_x)?,
            text_gap: literal(layout, &KEYS.text_gap)?,
            icon_gap: literal(layout, &KEYS.icon_gap)?,
            percent: layout.lit_string(&KEYS.percent, 0)?,
        })
    }

    /// The background around a name of the widest line `widest` and field height
    /// `text_height`, with the workshop row's size when it is shown, hung from `anchor`.
    fn frame(&self, anchor: Pos2, widest: f32, text_height: f32, row: Option<Vec2>) -> Rect {
        let width = widest.max(row.map_or(0.0, |size| size.x)) + self.padding;
        let height = text_height + row.map_or(0.0, |size| size.y) + self.extra_height;
        Rect::from_min_size(
            Pos2::new(anchor.x - width / self.halve, anchor.y),
            Vec2::new(width, height),
        )
    }
}

/// The settlement a marker stands for: the one whose location is the marker's position.
fn settlement_at<'a>(settlements: &'a [Settlement], marker: &MapMarker) -> Option<&'a Settlement> {
    settlements
        .iter()
        .find(|settlement| settlement.location == Some(marker.position))
}

/// The art's bounds as a rectangle relative to its registration point, scaled.
fn icon_box(cx: &TabContext<'_>, art: Icon, placement: IconPlacement, at: Vec2) -> Rect {
    let bounds = cx.art_library.fixed(FixedArt::Icon(art)).bounds;
    let scale = placement.scale;
    Rect::from_min_max(
        Pos2::new(at.x + bounds[0] * scale, at.y + bounds[1] * scale),
        Pos2::new(at.x + bounds[2] * scale, at.y + bounds[3] * scale),
    )
}

/// The workshop row's resolved parts, in the row's own space.
struct Row {
    population_text: String,
    happy_text: String,
    happy_icon_at: Vec2,
    happy_x: f32,
    bounds: Rect,
}

fn measure_row(cx: &TabContext<'_>, layout: &TooltipLayout, settlement: &Settlement) -> Row {
    let painter = cx.painter;
    let population_text = settlement.stats.population.to_string();
    let happy_text = format!("{}{}", settlement.stats.happiness, layout.percent);
    let style = layout.population.style(egui::Color32::WHITE);
    let population_width = natural_width(
        painter,
        &population_text,
        &layout.population.geometry,
        style,
    );
    let happy_icon_x = layout.population_x + population_width + layout.text_gap;
    let happy_icon_at = Vec2::new(happy_icon_x, layout.happy_icon.at.y);
    let happy_icon_box = icon_box(cx, Icon::Happiness, layout.happy_icon, happy_icon_at);
    let happy_x = happy_icon_x + happy_icon_box.width() + layout.icon_gap;
    let population_icon_box = icon_box(
        cx,
        Icon::Population,
        layout.population_icon,
        layout.population_icon.at,
    );
    let text_box = |field: &FieldSpec, shift: f32| {
        Rect::from_min_max(
            Pos2::new(field.geometry.left + shift, field.geometry.top),
            Pos2::new(field.geometry.right + shift, field.geometry.bottom),
        )
    };
    let bounds = population_icon_box
        .union(happy_icon_box)
        .union(text_box(&layout.population, 0.0))
        .union(text_box(&layout.happy, happy_x - layout.happy_x));
    Row {
        population_text,
        happy_text,
        happy_icon_at,
        happy_x,
        bounds,
    }
}

/// Draws the tooltip of `marker` hung from its on-stage position `anchor` (the marker's bottom).
pub fn draw(cx: &mut TabContext<'_>, layout: &TooltipLayout, marker: &MapMarker, anchor: Pos2) {
    if marker.name.is_empty() {
        return;
    }
    let snapshot = *cx.snapshot;
    let painter = cx.painter.clone();
    let tint = cx.tint;
    let location = layout.location.translated(anchor.to_vec2());
    let style = location.style(egui::Color32::WHITE);
    let field_height = text_height(&painter, &marker.name, &location.geometry, style)
        + 2.0 * location.geometry.vertical_inset;
    let widest = marker
        .name
        .lines()
        .map(|line| natural_width(&painter, line, &location.geometry, style))
        .fold(0.0, f32::max)
        .min(location.geometry.content_width());
    let row = settlement_at(&snapshot.model.settlements, marker)
        .map(|settlement| measure_row(cx, layout, settlement));
    let frame = layout.frame(
        anchor,
        widest,
        field_height,
        row.as_ref().map(|row| row.bounds.size()),
    );
    layout.background.fitted_to(frame).paint(&painter, tint);
    location.paint(&painter, &marker.name, tint);
    let Some(row) = row else {
        return;
    };
    let origin = Vec2::new(
        anchor.x + layout.row_x,
        anchor.y + layout.location_y + field_height + layout.row_gap,
    );
    let white = cx.shade(1.0);
    let population = layout.population.translated(origin);
    population.paint(&painter, &row.population_text, tint);
    let happy = layout
        .happy
        .translated(origin + Vec2::new(row.happy_x - layout.happy_x, 0.0));
    happy.paint(&painter, &row.happy_text, tint);
    let population_icon = layout.population_icon;
    cx.draw_icon(
        Icon::Population,
        (origin + population_icon.at).to_pos2(),
        population_icon.scale,
        white,
    );
    cx.draw_icon(
        Icon::Happiness,
        (origin + row.happy_icon_at).to_pos2(),
        layout.happy_icon.scale,
        white,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::pipboy::test_layout::pipboy_layout;
    use crate::sim::pipboy::MapMarkerId;
    use crate::sim::pipboy::workshops::SettlementStats;
    use glam::Vec3;

    fn marker(position: Vec3) -> MapMarker {
        MapMarker {
            id: MapMarkerId("m".to_owned()),
            name: "Alpha".to_owned(),
            marker_type: 1,
            flags: 0,
            position,
            discovered: true,
            can_fast_travel: true,
        }
    }

    fn settlement(name: &str, location: Option<Vec3>) -> Settlement {
        Settlement {
            name: name.to_owned(),
            needs_attention: false,
            raider_owned: false,
            stats: SettlementStats::default(),
            location,
        }
    }

    #[test]
    fn a_marker_matches_the_settlement_at_its_position_not_one_that_shares_its_name() {
        let here = Vec3::new(1.0, 2.0, 3.0);
        let settlements = [
            settlement("Alpha", Some(Vec3::new(9.0, 9.0, 9.0))),
            settlement("Alpha Two", Some(here)),
            settlement("Nowhere", None),
        ];
        let found = settlement_at(&settlements, &marker(here)).map(|s| s.name.as_str());
        assert_eq!(found, Some("Alpha Two"));
        assert!(settlement_at(&settlements, &marker(Vec3::ZERO)).is_none());
    }

    #[test]
    fn the_background_is_centred_on_the_anchor_and_grows_with_the_widest_content() {
        let layout = TooltipLayout::resolve(&pipboy_layout()).unwrap();
        let anchor = Pos2::new(100.0, 40.0);
        let name_only = layout.frame(anchor, 30.0, 10.0, None);
        let wider = layout.frame(anchor, 30.0, 10.0, Some(Vec2::new(80.0, 25.0)));
        assert!((name_only.min.x - (anchor.x - name_only.width() / layout.halve)).abs() < 1e-4);
        assert!((name_only.width() - (30.0 + layout.padding)).abs() < 1e-4);
        assert!((wider.width() - (80.0 + layout.padding)).abs() < 1e-4);
        assert!(
            (wider.height() - name_only.height() - 25.0).abs() < 1e-4,
            "the row adds its height"
        );
        assert!((name_only.min.y - anchor.y).abs() < 1e-4);
    }
}
