//! Painter helpers for the Pip-Boy look: tinted greys, filled boxes, sprites and one-line text
//! fields shrunk to fit. Everything is in stage pixels.

use egui::epaint::{Mesh, Vertex};
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, Galley, Painter, Pos2, Rect, Shape, Vec2};
use std::sync::Arc;

use super::art::ArtSprite;
use super::fonts::{Weight, font_id};
use crate::rendering::ui_colour::multiply_encoded;
use crate::ui_layout::scaleform_text::fit_size;
use crate::ui_layout::values::Matrix;

/// The Pip-Boy tint scaled to a greyscale fraction of full white. The only place the grey model
/// lives: a grey `g` of the original art is `tint * g / 255`.
pub fn shade(tint: Color32, fraction: f32) -> Color32 {
    let scale = |channel: u8| (f32::from(channel) * fraction.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgb(scale(tint.r()), scale(tint.g()), scale(tint.b()))
}

/// `colour` multiplied channel by channel with `tint`, keeping its alpha: authored white and grey
/// art and text take the tint, black stays black.
pub fn tinted(colour: Color32, tint: Color32) -> Color32 {
    Color32::from_rgba_premultiplied(
        multiply_encoded(colour.r(), tint.r()),
        multiply_encoded(colour.g(), tint.g()),
        multiply_encoded(colour.b(), tint.b()),
        colour.a(),
    )
}

/// `colour` at `alpha` of its opacity.
pub fn faded(colour: Color32, alpha: f32) -> Color32 {
    colour.gamma_multiply(alpha.clamp(0.0, 1.0))
}

pub const BLACK: Color32 = Color32::BLACK;

pub fn fill(painter: &Painter, rect: Rect, colour: Color32) {
    painter.rect_filled(rect, 0.0, colour);
}

/// A `thickness` outline drawn inside `rect`.
pub fn outline(painter: &Painter, rect: Rect, thickness: f32, colour: Color32) {
    let (w, h) = (rect.width(), rect.height());
    let bars = [
        Rect::from_min_size(rect.min, Vec2::new(w, thickness)),
        Rect::from_min_size(
            Pos2::new(rect.min.x, rect.max.y - thickness),
            Vec2::new(w, thickness),
        ),
        Rect::from_min_size(rect.min, Vec2::new(thickness, h)),
        Rect::from_min_size(
            Pos2::new(rect.max.x - thickness, rect.min.y),
            Vec2::new(thickness, h),
        ),
    ];
    for bar in bars {
        fill(painter, bar, colour);
    }
}

pub fn draw_sprite(painter: &Painter, sprite: ArtSprite, at: Pos2, colour: Color32) {
    const FULL_UV: Rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
    painter.image(
        sprite.texture,
        sprite.rect.translate(at.to_vec2()),
        FULL_UV,
        colour,
    );
}

/// Draws `sprite`, rasterised at `raster_scale`, through `matrix` from its registration point:
/// rotated, skewed or mirrored art such as a clip nested in an animated parent.
pub fn draw_sprite_affine(
    painter: &Painter,
    sprite: ArtSprite,
    raster_scale: f32,
    matrix: &Matrix,
    colour: Color32,
) {
    let rect = sprite.rect;
    let corner = |at: Pos2| {
        let (x, y) = matrix.apply(at.x / raster_scale, at.y / raster_scale);
        Pos2::new(x, y)
    };
    let mut mesh = Mesh::with_texture(sprite.texture);
    for (at, uv) in [
        (rect.left_top(), Pos2::new(0.0, 0.0)),
        (rect.right_top(), Pos2::new(1.0, 0.0)),
        (rect.left_bottom(), Pos2::new(0.0, 1.0)),
        (rect.right_bottom(), Pos2::new(1.0, 1.0)),
    ] {
        mesh.vertices.push(Vertex {
            pos: corner(at),
            uv,
            color: colour,
        });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(2, 1, 3);
    painter.add(Shape::mesh(mesh));
}

/// Where a text field sits and how its text aligns inside the content width. `origin` is the
/// top-left of the first glyph line; `width` is the content width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextField {
    pub origin: Pos2,
    pub width: f32,
    pub align: Align,
}

impl TextField {
    pub fn left(origin: Pos2, width: f32) -> Self {
        Self {
            origin,
            width,
            align: Align::Min,
        }
    }

    pub fn centre(origin: Pos2, width: f32) -> Self {
        Self {
            origin,
            width,
            align: Align::Center,
        }
    }

    pub fn right(origin: Pos2, width: f32) -> Self {
        Self {
            origin,
            width,
            align: Align::Max,
        }
    }

    /// The x a galley laid out with this field's alignment is positioned at.
    fn anchor_x(&self) -> f32 {
        self.origin.x + self.width * self.align.to_factor()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub weight: Weight,
    pub size: f32,
    pub colour: Color32,
}

impl TextStyle {
    pub fn regular(size: f32, colour: Color32) -> Self {
        Self {
            weight: Weight::Regular,
            size,
            colour,
        }
    }

    pub fn bold(size: f32, colour: Color32) -> Self {
        Self {
            weight: Weight::Bold,
            size,
            colour,
        }
    }
}

fn job(text: &str, style: TextStyle, size: f32, align: Align, wrap_width: f32) -> LayoutJob {
    let mut job = LayoutJob::single_section(
        text.to_owned(),
        TextFormat::simple(font_id(style.weight, size), style.colour),
    );
    job.halign = align;
    job.wrap.max_width = wrap_width;
    job
}

fn layout(painter: &Painter, job: LayoutJob) -> Arc<Galley> {
    painter.layout_job(job)
}

/// The width of `text` on one line.
pub fn text_width(painter: &Painter, text: &str, weight: Weight, size: f32) -> f32 {
    let style = TextStyle {
        weight,
        size,
        colour: Color32::WHITE,
    };
    layout(painter, job(text, style, size, Align::Min, f32::INFINITY))
        .rect
        .width()
}

/// One line of text in a field, shrunk in the game's half-pixel steps until it fits the field
/// width. Returns the drawn rect.
pub fn text_field(painter: &Painter, field: TextField, text: &str, style: TextStyle) -> Rect {
    let size = fit_size(style.size, |size| {
        text_width(painter, text, style.weight, size) <= field.width
    });
    let galley = layout(painter, job(text, style, size, field.align, f32::INFINITY));
    let pos = Pos2::new(field.anchor_x(), field.origin.y);
    let rect = galley.rect.translate(pos.to_vec2());
    painter.galley(pos, galley, style.colour);
    rect
}

#[cfg(test)]
mod tests {
    use super::*;

    const TINT: Color32 = Color32::from_rgb(100, 200, 40);

    #[test]
    fn shade_scales_every_channel_and_clamps_the_fraction() {
        assert_eq!(shade(TINT, 0.5), Color32::from_rgb(50, 100, 20));
        assert_eq!(shade(TINT, 0.0), Color32::BLACK);
        assert_eq!(shade(TINT, 2.0), TINT);
        assert_eq!(shade(TINT, -1.0), Color32::BLACK);
    }

    #[test]
    fn tinting_multiplies_the_channels_and_keeps_the_alpha() {
        let colour = Color32::from_rgba_premultiplied(255, 128, 0, 90);
        assert_eq!(
            tinted(colour, TINT),
            Color32::from_rgba_premultiplied(100, 100, 0, 90)
        );
        assert_eq!(tinted(Color32::WHITE, TINT), TINT);
        assert_eq!(tinted(Color32::BLACK, TINT), Color32::BLACK);
    }

    #[test]
    fn fields_anchor_their_galley_by_alignment() {
        let origin = Pos2::new(10.0, 5.0);
        assert!((TextField::left(origin, 100.0).anchor_x() - 10.0).abs() < 1e-6);
        assert!((TextField::centre(origin, 100.0).anchor_x() - 60.0).abs() < 1e-6);
        assert!((TextField::right(origin, 100.0).anchor_x() - 110.0).abs() < 1e-6);
    }

    #[test]
    fn fading_scales_alpha_and_colour_together() {
        let faded = faded(Color32::from_rgb(200, 100, 0), 0.5);
        assert_eq!(faded, Color32::from_rgb(200, 100, 0).gamma_multiply(0.5));
        assert!(faded.a() < 200);
    }
}
