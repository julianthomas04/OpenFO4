//! What the quantity and favourites menus share: a black panel with a three-sided outline whose
//! open top edge is drawn around the header text, and the header itself.

use egui::{Color32, Painter, Pos2, Rect, Vec2};

use crate::assets::pipboy_art::FixedArt;
use crate::rendering::pipboy::paint::fill;
use crate::rendering::pipboy::scaleform_text::{FieldSpec, PaintedField, TextAutoSize};
use crate::rendering::pipboy::shapes::{OutlinedShape, Shade, rect_of};
use crate::rendering::pipboy::tab::TabContext;
use crate::ui_layout::{
    LiteralKey, PlacementKey, ShapeKey, TextKey, UiLayout, UiLayoutError, key_check_fields,
};

/// The plain text of an authored field: its markup removed.
pub fn authored_text(
    layout: &UiLayout,
    key: &TextKey,
    rules: &crate::ui_layout::set_text::SetTextRules,
) -> Result<String, UiLayoutError> {
    let html = layout.get(key)?.initial_text.ok_or_else(|| {
        UiLayoutError::unresolved(key.movie, key.path, "the field has no authored text")
    })?;
    let mut text = String::new();
    let mut in_tag = false;
    for character in html.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => text.push(character),
            _ => {}
        }
    }
    Ok(rules.apply(layout, text.trim_end_matches('\r'), false, false))
}

/// Where an art clip sits: its registration point and whether the layout turns it half round.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArtPlacement {
    pub at: Pos2,
    half_turn: bool,
}

impl ArtPlacement {
    pub fn resolve(layout: &UiLayout, key: &PlacementKey) -> Result<Self, UiLayoutError> {
        let matrix = layout.get(key)?.world.matrix;
        let turned = matrix.a < 0.0 && matrix.d < 0.0;
        let upright = matrix.a > 0.0 && matrix.d > 0.0;
        let unit = (matrix.a.abs() - 1.0).abs() < 1e-3 && (matrix.d.abs() - 1.0).abs() < 1e-3;
        if matrix.b != 0.0 || matrix.c != 0.0 || !unit || !(turned || upright) {
            return Err(UiLayoutError::unresolved(
                key.movie,
                key.path,
                "the clip is scaled, skewed or flipped rather than placed or turned half round",
            ));
        }
        Ok(Self {
            at: Pos2::new(matrix.tx, matrix.ty),
            half_turn: turned,
        })
    }

    /// Draws `art` at its placement moved by `offset`; without the art the frame stays empty and
    /// the cache logs it.
    pub fn draw(&self, cx: &mut TabContext<'_>, art: FixedArt, offset: Vec2, colour: Color32) {
        let at = self.at + offset;
        if !self.half_turn {
            cx.draw_fixed(art, 0, at, 1.0, colour);
            return;
        }
        let sprite = cx
            .art
            .fixed_scaled(cx.painter.ctx(), cx.art_library, art, 0, 1.0);
        if let Some(sprite) = sprite {
            let turned = Rect::from_min_max(
                Pos2::new(-sprite.rect.max.x, -sprite.rect.max.y),
                Pos2::new(-sprite.rect.min.x, -sprite.rect.min.y),
            );
            cx.painter.image(
                sprite.texture,
                turned.translate(at.to_vec2()),
                Rect::from_min_max(Pos2::new(1.0, 1.0), Pos2::new(0.0, 0.0)),
                colour,
            );
        }
    }
}

/// A shape drawn only as a line: the rectangle its centreline encloses and its width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokedShape {
    pub edges: Rect,
    pub width: f32,
    pub shade: Shade,
}

impl StrokedShape {
    pub fn resolve(layout: &UiLayout, key: &ShapeKey) -> Result<Self, UiLayoutError> {
        let shape = layout.get(key)?;
        let matrix = shape.placement.world.matrix;
        let scaled = (matrix.a - 1.0).abs() > f32::EPSILON || (matrix.d - 1.0).abs() > f32::EPSILON;
        if matrix.b != 0.0 || matrix.c != 0.0 || scaled {
            return Err(UiLayoutError::unresolved(
                key.movie,
                key.path,
                "the outline is scaled or rotated",
            ));
        }
        let line = shape.lines.first().ok_or_else(|| {
            UiLayoutError::unresolved(key.movie, key.path, "the shape has no line style")
        })?;
        let colour = line.color.ok_or_else(|| {
            UiLayoutError::unresolved(key.movie, key.path, "the line has no colour")
        })?;
        Ok(Self {
            edges: rect_of(matrix.bounds(shape.edge_bounds)),
            width: line.width,
            shade: Shade::of(colour, &shape.placement.world.color),
        })
    }

    fn half(&self) -> f32 {
        self.width / 2.0
    }

    /// The rectangle the line and its stroke cover.
    pub fn outer(&self) -> Rect {
        self.edges.expand(self.half())
    }

    pub fn translated(&self, offset: Vec2) -> Self {
        Self {
            edges: self.edges.translate(offset),
            ..*self
        }
    }

    pub fn paint_closed(&self, painter: &Painter, tint: Color32) {
        let colour = self.shade.colour(tint);
        let outer = self.outer();
        for bar in [
            Rect::from_min_max(outer.min, Pos2::new(outer.max.x, outer.min.y + self.width)),
            Rect::from_min_max(Pos2::new(outer.min.x, outer.max.y - self.width), outer.max),
            Rect::from_min_max(outer.min, Pos2::new(outer.min.x + self.width, outer.max.y)),
            Rect::from_min_max(Pos2::new(outer.max.x - self.width, outer.min.y), outer.max),
        ] {
            fill(painter, bar, colour);
        }
    }

    /// The left, bottom and right sides; the top is left open.
    pub fn paint_open_top(&self, painter: &Painter, tint: Color32) {
        let colour = self.shade.colour(tint);
        let (edges, half) = (self.edges, self.half());
        for bar in [
            Rect::from_min_max(
                Pos2::new(edges.min.x - half, edges.min.y),
                Pos2::new(edges.min.x + half, edges.max.y + half),
            ),
            Rect::from_min_max(
                Pos2::new(edges.max.x - half, edges.min.y),
                Pos2::new(edges.max.x + half, edges.max.y + half),
            ),
            Rect::from_min_max(
                Pos2::new(edges.min.x - half, edges.max.y - half),
                Pos2::new(edges.max.x + half, edges.max.y + half),
            ),
        ] {
            fill(painter, bar, colour);
        }
    }

    /// The area the shape covers as a solid: the line's colour fills everything it encloses.
    pub fn paint_solid(&self, painter: &Painter, tint: Color32) {
        fill(painter, self.outer(), self.shade.colour(tint));
    }
}

pub struct FrameKeys {
    pub background: ShapeKey,
    pub outline: ShapeKey,
    pub holder: PlacementKey,
    pub header: TextKey,
    pub gap_before: LiteralKey,
    pub gap_after: LiteralKey,
    pub right_inset: LiteralKey,
    pub line_width: LiteralKey,
}

key_check_fields!(FrameKeys {
    background,
    outline,
    holder,
    header,
    gap_before,
    gap_after,
    right_inset,
    line_width,
});

/// The keys of a menu symbol `$symbol`; `$code` is the method of its class that draws the top
/// edge.
macro_rules! frame_keys {
    ($movie:literal, $symbol:literal, $code:literal) => {
        $crate::rendering::pipboy::pages::inv::modal::FrameKeys {
            background: $crate::ui_layout::ShapeKey::new(
                $movie,
                concat!("[", $symbol, "]/Background_mc/@1"),
            ),
            outline: $crate::ui_layout::ShapeKey::new($movie, concat!("[", $symbol, "]/@3")),
            holder: $crate::ui_layout::PlacementKey::new(
                $movie,
                concat!("[", $symbol, "]/TopBracketHolder_mc"),
            ),
            header: $crate::ui_layout::TextKey::new($movie, concat!("[", $symbol, "]/Header_tf")),
            gap_before: $crate::ui_layout::LiteralKey::new(
                $movie,
                concat!($code, "#($1.x-TopBracketHolder_mc.x)-"),
                1,
            ),
            gap_after: $crate::ui_layout::LiteralKey::new(
                $movie,
                concat!($code, "#($1.y-TopBracketHolder_mc.x)+"),
                1,
            ),
            right_inset: $crate::ui_layout::LiteralKey::new(
                $movie,
                concat!(
                    $code,
                    "#((Background_mc.x+Background_mc.width)-TopBracketHolder_mc.x)-"
                ),
                1,
            ),
            line_width: $crate::ui_layout::LiteralKey::new(
                $movie,
                concat!($code, "#new Shape().graphics.lineStyle(0)"),
                1,
            ),
        }
    };
}
pub(crate) use frame_keys;

#[derive(Debug, Clone)]
pub struct ModalFrame {
    background: OutlinedShape,
    outline: StrokedShape,
    holder: Pos2,
    header: FieldSpec,
    pub header_text: String,
    gap_before: f32,
    gap_after: f32,
    right_inset: f32,
    line_width: f32,
}

/// How the top edge runs: from the holder, around the header's glyphs, to the panel's right.
#[derive(Debug, Clone, Copy, PartialEq)]
struct EdgeSpec {
    holder: Pos2,
    line_width: f32,
    gap_before: f32,
    gap_after: f32,
    right_end: f32,
}

/// The two pieces of the top edge around the header's glyph run; a piece the header leaves no
/// room for is absent.
fn top_edges(spec: &EdgeSpec, run: (f32, f32)) -> [Option<Rect>; 2] {
    let half = spec.line_width / 2.0;
    let piece = |from: f32, to: f32| {
        (to > from).then(|| {
            Rect::from_min_max(
                Pos2::new(from, spec.holder.y - half),
                Pos2::new(to, spec.holder.y + half),
            )
        })
    };
    [
        piece(spec.holder.x, run.0 - spec.gap_before),
        piece(run.1 + spec.gap_after, spec.right_end),
    ]
}

impl ModalFrame {
    fn edge_spec(&self) -> EdgeSpec {
        EdgeSpec {
            holder: self.holder,
            line_width: self.line_width,
            gap_before: self.gap_before,
            gap_after: self.gap_after,
            right_end: self.background.bounds().max.x - self.right_inset,
        }
    }

    pub fn resolve(
        layout: &UiLayout,
        keys: &FrameKeys,
        rules: &crate::ui_layout::set_text::SetTextRules,
    ) -> Result<Self, UiLayoutError> {
        let holder = layout.get(&keys.holder)?.world.matrix;
        Ok(Self {
            background: OutlinedShape::resolve(layout, &keys.background)?,
            outline: StrokedShape::resolve(layout, &keys.outline)?,
            holder: Pos2::new(holder.tx, holder.ty),
            header: FieldSpec::resolve(layout, &keys.header, TextAutoSize::Shrink)?,
            header_text: authored_text(layout, &keys.header, rules)?,
            gap_before: layout.lit_number(&keys.gap_before, 0)? as f32,
            gap_after: layout.lit_number(&keys.gap_after, 0)? as f32,
            right_inset: layout.lit_number(&keys.right_inset, 0)? as f32,
            line_width: layout.lit_number(&keys.line_width, 0)? as f32,
        })
    }

    pub fn draw(&self, painter: &Painter, tint: Color32) -> PaintedField {
        self.background.paint(painter, tint);
        self.outline.paint_open_top(painter, tint);
        let painted = self.header.paint(painter, &self.header_text, tint);
        let colour = self.outline.shade.colour(tint);
        for edge in top_edges(&self.edge_spec(), painted.run)
            .into_iter()
            .flatten()
        {
            fill(painter, edge, colour);
        }
        painted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> EdgeSpec {
        EdgeSpec {
            holder: Pos2::new(2.0, 3.0),
            line_width: 2.0,
            gap_before: 5.0,
            gap_after: 5.0,
            right_end: 96.0,
        }
    }

    #[test]
    fn the_top_edge_stops_short_of_the_header_and_of_the_panel_side() {
        let [left, right] = top_edges(&spec(), (40.0, 60.0));
        let (left, right) = (left.unwrap(), right.unwrap());
        assert_eq!((left.min.x, left.max.x), (2.0, 35.0));
        assert_eq!((right.min.x, right.max.x), (65.0, 96.0));
        assert_eq!((left.min.y, left.max.y), (2.0, 4.0));
    }

    #[test]
    fn a_header_wider_than_the_panel_leaves_no_edge_on_that_side() {
        let [left, right] = top_edges(&spec(), (3.0, 99.0));
        assert!(left.is_none());
        assert!(right.is_none());
    }

    #[test]
    fn the_stroked_outline_covers_half_its_width_outside_the_centreline() {
        let shape = StrokedShape {
            edges: Rect::from_min_max(Pos2::new(1.0, 1.0), Pos2::new(99.0, 49.0)),
            width: 2.0,
            shade: Shade::WHITE,
        };
        assert_eq!(shape.outer().min, Pos2::new(0.0, 0.0));
        assert_eq!(shape.outer().max, Pos2::new(100.0, 50.0));
    }
}
