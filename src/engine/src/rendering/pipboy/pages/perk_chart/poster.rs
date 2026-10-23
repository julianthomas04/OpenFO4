//! The Vault-Tec poster behind the chart and the grime overlay multiplied over it: two stacked
//! images each, scaled with the grid and cut to the part that shows on the stage.

use egui::{Color32, Painter, Pos2, Rect, Shape, Vec2};

use crate::rendering::pipboy::pages::perk_chart_layout::{GridConstants, GridView};
use crate::rendering::pipboy::perk_grid::{OverlayLayer, PerkGridTextures, multiply_overlay};

/// A poster image's place in grid units and the part of the image it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layer {
    pub grid: Rect,
    pub uv: Rect,
}

/// The two layers of one poster: the top image, then the bottom image below it. Each image is
/// scaled to the grid's texture width and shows only its non-empty rows.
pub fn layers(constants: &GridConstants, top_size: Vec2, bottom_size: Vec2) -> [Layer; 2] {
    let layer = |size: Vec2, y: f32, height: f32| {
        let scale = constants.texture_width / size.x;
        Layer {
            grid: Rect::from_min_size(
                Pos2::new(0.0, y),
                Vec2::new(constants.texture_width, height),
            ),
            uv: Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, height / (size.y * scale))),
        }
    };
    [
        layer(top_size, 0.0, constants.top_height),
        layer(
            bottom_size,
            constants.top_height,
            constants.content_height - constants.top_height,
        ),
    ]
}

/// The part of `layer` that lies on `stage` for `view`: the stage rectangle and its image part.
pub fn visible(layer: &Layer, view: &GridView, stage: Rect) -> Option<(Rect, Rect)> {
    let rect = Rect::from_min_max(view.to_stage(layer.grid.min), view.to_stage(layer.grid.max));
    let shown = rect.intersect(stage);
    if shown.width() <= 0.0 || shown.height() <= 0.0 {
        return None;
    }
    let share = |value: f32, from: f32, size: f32| (value - from) / size;
    let uv_at = |point: Pos2| {
        Pos2::new(
            layer.uv.min.x + share(point.x, rect.min.x, rect.width()) * layer.uv.width(),
            layer.uv.min.y + share(point.y, rect.min.y, rect.height()) * layer.uv.height(),
        )
    };
    Some((
        shown,
        Rect::from_min_max(uv_at(shown.min), uv_at(shown.max)),
    ))
}

pub fn draw_poster(
    painter: &Painter,
    textures: &PerkGridTextures,
    constants: &GridConstants,
    view: &GridView,
    stage: Rect,
) {
    let layers = layers(constants, textures.top.size, textures.bottom.size);
    for (layer, image) in layers.iter().zip([textures.top, textures.bottom]) {
        if let Some((rect, uv)) = visible(layer, view, stage) {
            painter.image(image.texture, rect, uv, Color32::WHITE);
        }
    }
}

pub fn draw_overlay(
    painter: &Painter,
    textures: &PerkGridTextures,
    constants: &GridConstants,
    view: &GridView,
    stage: Rect,
    alpha: f32,
) {
    let layers = layers(
        constants,
        textures.overlay_top_size,
        textures.overlay_bottom_size,
    );
    for (layer, which) in layers.iter().zip([OverlayLayer::Top, OverlayLayer::Bottom]) {
        if let Some((rect, uv)) = visible(layer, view, stage) {
            painter.add(Shape::Callback(multiply_overlay(rect, which, uv, alpha)));
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact literals")]
mod tests {
    use super::*;
    use crate::rendering::pipboy::pages::perk_chart_layout::HoverScroll;

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

    const STAGE: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1280.0, 720.0));

    fn poster() -> [Layer; 2] {
        layers(&constants(), Vec2::splat(2560.0), Vec2::splat(2560.0))
    }

    #[test]
    fn the_images_stack_and_cover_the_poster_height_without_a_gap() {
        let [top, bottom] = poster();
        assert_eq!(top.grid.max.y, bottom.grid.min.y);
        assert_eq!(bottom.grid.max.y, 3000.0);
        assert!((top.uv.max.y - 1500.0 / 2560.0).abs() < 1e-6);
        assert!((bottom.uv.max.y - 1500.0 / 2560.0).abs() < 1e-6);
    }

    #[test]
    fn a_smaller_image_is_scaled_to_the_grid_width() {
        let layers = layers(&constants(), Vec2::new(1024.0, 1024.0), Vec2::splat(2560.0));
        assert!((layers[0].uv.max.y - 1500.0 / 2560.0).abs() < 1e-6);
    }

    #[test]
    fn the_opening_view_shows_the_top_image_across_the_stage_and_not_the_bottom_one() {
        let view = GridView::initial(&constants());
        let [top, bottom] = poster();
        let (rect, uv) = visible(&top, &view, STAGE).unwrap();
        assert_eq!(rect, STAGE);
        assert!((uv.max.x - 1.0).abs() < 1e-6);
        assert!((uv.max.y - 720.0 / 0.5 / 2560.0).abs() < 1e-5);
        assert!(visible(&bottom, &view, STAGE).is_none());
    }

    #[test]
    fn a_scrolled_view_shows_both_images_where_they_meet() {
        let mut view = GridView::initial(&constants());
        view.move_by(&constants(), Vec2::new(0.0, -400.0));
        let [top, bottom] = poster();
        let (top_rect, _) = visible(&top, &view, STAGE).unwrap();
        let (bottom_rect, bottom_uv) = visible(&bottom, &view, STAGE).unwrap();
        assert!((top_rect.max.y - bottom_rect.min.y).abs() < 1e-3);
        assert_eq!(bottom_uv.min.y, 0.0);
    }
}
