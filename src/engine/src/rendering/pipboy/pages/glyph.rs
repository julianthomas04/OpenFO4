//! Glyph placement helpers shared by the DATA, MAP and RADIO pages: clips at their registration
//! point, rotated sprites.

use egui::epaint::{Mesh, Vertex};
use egui::{Color32, Pos2, Shape, Vec2};

use crate::assets::pipboy_art::ArtClip;
use crate::rendering::pipboy::art::ArtSprite;
use crate::rendering::pipboy::paint::draw_sprite;
use crate::rendering::pipboy::tab::TabContext;

/// Frame 0 of `clip` with its registration point at `at`, drawn through `painter` (which may
/// clip it) at `symbol_scale` times its authored size.
pub fn draw_clip_registered(
    cx: &mut TabContext<'_>,
    painter: &egui::Painter,
    clip: &ArtClip,
    at: Pos2,
    symbol_scale: f32,
    colour: Color32,
) {
    let sprite = cx
        .art
        .clip_scaled(cx.painter.ctx(), cx.art_library, clip, 0, symbol_scale);
    if let Some(sprite) = sprite {
        draw_sprite(painter, sprite, at, colour);
    }
}

/// `sprite` turned clockwise by `angle` radians about its own centre, which sits on `centre`.
pub fn draw_sprite_rotated(
    painter: &egui::Painter,
    sprite: ArtSprite,
    centre: Pos2,
    angle: f32,
    colour: Color32,
) {
    let (sin, cos) = angle.sin_cos();
    let half = sprite.rect.size() / 2.0;
    let corner = |x: f32, y: f32| {
        let local = Vec2::new(x * half.x, y * half.y);
        centre + Vec2::new(local.x * cos - local.y * sin, local.x * sin + local.y * cos)
    };
    let corners = [
        (corner(-1.0, -1.0), Pos2::new(0.0, 0.0)),
        (corner(1.0, -1.0), Pos2::new(1.0, 0.0)),
        (corner(1.0, 1.0), Pos2::new(1.0, 1.0)),
        (corner(-1.0, 1.0), Pos2::new(0.0, 1.0)),
    ];
    let mut mesh = Mesh::with_texture(sprite.texture);
    mesh.vertices = corners
        .iter()
        .map(|&(pos, uv)| Vertex {
            pos,
            uv,
            color: colour,
        })
        .collect();
    mesh.indices = vec![0, 1, 2, 0, 2, 3];
    painter.add(Shape::mesh(mesh));
}
