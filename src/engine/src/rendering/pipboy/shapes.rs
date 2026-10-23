//! Authored shapes of the layout drawn in the Pip-Boy tint: filled rectangles (panels, bars,
//! caps) and the game's meter (an outline over a fill and a ghost fill).

use egui::{Color32, Painter, Pos2, Rect, Vec2};

use super::paint::{faded, fill, shade};
use crate::ui_layout::outline::{self, PartKind};
use crate::ui_layout::values::{ColorTransform, Matrix, Shape, TrackKey};
use crate::ui_layout::{
    BoundsKey, Declaration, Key, KeyCheck, LiteralKey, PlacementKey, ShapeKey, UiLayout,
    UiLayoutError,
};

pub fn rect_of([x0, y0, x1, y1]: [f32; 4]) -> Rect {
    Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1))
}

/// A colour of the original art as the tinted screen shows it: its grey level after the colour
/// transform, as a fraction of white, and its opacity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shade {
    pub grey: f32,
    pub alpha: f32,
}

impl Shade {
    pub const WHITE: Self = Self {
        grey: 1.0,
        alpha: 1.0,
    };

    /// `rgba` (0..255) after `transform`; the grey is the mean of the three channels, since the
    /// interface art is monochrome.
    pub fn of(rgba: [u8; 4], transform: &ColorTransform) -> Self {
        let channel = |index: usize| {
            (f32::from(rgba[index]) * transform.mult[index] + transform.add[index])
                .clamp(0.0, 255.0)
                / 255.0
        };
        Self {
            grey: (channel(0) + channel(1) + channel(2)) / 3.0,
            alpha: channel(3),
        }
    }

    pub fn colour(&self, tint: Color32) -> Color32 {
        faded(shade(tint, self.grey), self.alpha)
    }
}

/// The opacity of an animated clip on `frame`: that of its last key at or before the frame,
/// `base` before its first.
pub fn alpha_at_frame(track: &[TrackKey], base: &ColorTransform, frame: u32) -> f32 {
    let color = track
        .iter()
        .take_while(|key| key.frame <= frame)
        .last()
        .map_or(base, |key| &key.placement.color);
    Shade::of([0, 0, 0, 255], color).alpha
}

/// A colour the menu's code sets as a 24-bit number (`textColor = 0xRRGGBB`), as a shade.
pub fn code_colour(
    layout: &UiLayout,
    key: &LiteralKey,
    index: usize,
) -> Result<Shade, UiLayoutError> {
    let value = layout.lit_number(key, index)?;
    if !(0.0..=f64::from(0x00FF_FFFF)).contains(&value) || value.fract() != 0.0 {
        return Err(UiLayoutError::unresolved(
            key.movie,
            key.path,
            format!("{value} is not a 24-bit colour"),
        ));
    }
    let [_, r, g, b] = (value as u32).to_be_bytes();
    Ok(Shade::of([r, g, b, 255], &ColorTransform::IDENTITY))
}

fn axis_aligned<K>(key: &Key<K>, matrix: &Matrix) -> Result<(), UiLayoutError> {
    if matrix.is_axis_aligned() {
        Ok(())
    } else {
        Err(UiLayoutError::unresolved(
            key.movie,
            key.path,
            "the shape is rotated or skewed",
        ))
    }
}

fn fill_shade(key: &ShapeKey, shape: &Shape) -> Result<Shade, UiLayoutError> {
    let colour = shape
        .fills
        .iter()
        .find_map(|fill| fill.color)
        .ok_or_else(|| {
            UiLayoutError::unresolved(key.movie, key.path, "the shape has no solid fill")
        })?;
    Ok(Shade::of(colour, &shape.placement.world.color))
}

/// The box of a shape's edges at its world placement, for shapes the code only measures (a hit
/// area) and never paints.
pub fn edge_rect(layout: &UiLayout, key: &ShapeKey) -> Result<Rect, UiLayoutError> {
    let shape = layout.get(key)?;
    let matrix = shape.placement.world.matrix;
    axis_aligned(key, &matrix)?;
    Ok(rect_of(matrix.bounds(shape.edge_bounds)))
}

/// One painted box of a shape's outline, after the placement it was resolved under.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapePart {
    pub rect: Rect,
    pub shade: Shade,
    pub kind: PartKind,
}

impl ShapePart {
    fn paint(&self, painter: &Painter, tint: Color32, offset: Vec2) {
        fill(
            painter,
            self.rect.translate(offset),
            self.shade.colour(tint),
        );
    }

    fn paint_mapped(&self, painter: &Painter, tint: Color32, map: impl Fn(Color32) -> Color32) {
        fill(painter, self.rect, map(self.shade.colour(tint)));
    }
}

/// The union of the boxes of `parts`.
fn union_rect<'a>(parts: impl Iterator<Item = &'a ShapePart>) -> Rect {
    parts.fold(Rect::NOTHING, |bounds, part| bounds.union(part.rect))
}

/// The size of one stage pixel in the shape's own pixels, which a hairline stroke measures.
fn stage_pixel(matrix: &Matrix) -> [f32; 2] {
    let size = |scale: f32| if scale == 0.0 { 1.0 } else { 1.0 / scale.abs() };
    [size(matrix.a), size(matrix.d)]
}

/// The boxes `shape` paints, from its recorded outline: its fills and strokes, placed by
/// `matrix` and coloured under the shape's world colour transform.
fn outline_parts(
    key: &ShapeKey,
    shape: &Shape,
    matrix: &Matrix,
) -> Result<Vec<ShapePart>, UiLayoutError> {
    axis_aligned(key, matrix)?;
    let parts = outline::parts(shape, stage_pixel(matrix))
        .map_err(|error| UiLayoutError::unresolved(key.movie, key.path, error.to_string()))?;
    Ok(parts
        .iter()
        .map(|part| ShapePart {
            rect: rect_of(matrix.bounds(part.rect)),
            shade: Shade::of(part.colour, &shape.placement.world.color),
            kind: part.kind,
        })
        .collect())
}

/// A shape drawn from its outline at its world placement: its fills and its strokes (a stroked
/// rectangle is as thick as its edges plus its line width).
#[derive(Debug, Clone, PartialEq)]
pub struct OutlinedShape {
    parts: Vec<ShapePart>,
}

impl OutlinedShape {
    pub fn resolve(layout: &UiLayout, key: &ShapeKey) -> Result<Self, UiLayoutError> {
        let shape = layout.get(key)?;
        let parts = outline_parts(key, &shape, &shape.placement.world.matrix)?;
        Ok(Self { parts })
    }

    pub fn paint(&self, painter: &Painter, tint: Color32) {
        for part in &self.parts {
            part.paint(painter, tint, Vec2::ZERO);
        }
    }

    /// Paints the shape with every colour passed through `map` after the tint.
    pub fn paint_mapped(&self, painter: &Painter, tint: Color32, map: impl Fn(Color32) -> Color32) {
        for part in &self.parts {
            part.paint_mapped(painter, tint, &map);
        }
    }

    pub fn parts(&self) -> &[ShapePart] {
        &self.parts
    }

    /// The box around everything the shape paints, strokes included: what Flash reports as the
    /// shape's bounds.
    pub fn bounds(&self) -> Rect {
        union_rect(self.parts.iter())
    }

    /// The box around the shape's fills alone: what a mask clips to and what a fill hit-tests.
    /// `None` for a shape that only strokes.
    pub fn fill_bounds(&self) -> Option<Rect> {
        let fills = union_rect(self.parts.iter().filter(|part| part.kind == PartKind::Fill));
        (fills != Rect::NOTHING).then_some(fills)
    }

    /// The fill area of the shape `key` at its world placement.
    pub fn fill_area(layout: &UiLayout, key: &ShapeKey) -> Result<Rect, UiLayoutError> {
        Self::resolve(layout, key)?.fill_bounds().ok_or_else(|| {
            UiLayoutError::unresolved(key.movie, key.path, "the shape has no fill, only strokes")
        })
    }

    #[must_use]
    pub fn translated(&self, offset: Vec2) -> Self {
        Self {
            parts: self
                .parts
                .iter()
                .map(|part| ShapePart {
                    rect: part.rect.translate(offset),
                    ..*part
                })
                .collect(),
        }
    }

    /// The shape after its clip is scaled by `factor` about the top left of its bounds.
    #[must_use]
    pub fn scaled(&self, factor: Vec2) -> Self {
        let anchor = self.bounds().min;
        let map = |point: Pos2| anchor + (point - anchor) * factor;
        Self {
            parts: self
                .parts
                .iter()
                .map(|part| ShapePart {
                    rect: Rect::from_two_pos(map(part.rect.min), map(part.rect.max)),
                    ..*part
                })
                .collect(),
        }
    }

    /// The shape stretched and moved so its bounds are `target`.
    #[must_use]
    pub fn fitted_to(&self, target: Rect) -> Self {
        let bounds = self.bounds();
        let ratio = |new: f32, old: f32| if old > 0.0 { new / old } else { 1.0 };
        let scaled = self.scaled(Vec2::new(
            ratio(target.width(), bounds.width()),
            ratio(target.height(), bounds.height()),
        ));
        scaled.translated(target.min - scaled.bounds().min)
    }

    /// The shape as wide as `width`, scaled about the left of its bounds.
    #[must_use]
    pub fn with_width(&self, width: f32) -> Self {
        let current = self.bounds().width();
        if current > 0.0 {
            self.scaled(Vec2::new(width / current, 1.0))
        } else {
            self.clone()
        }
    }

    /// A shape of one box, for tests that need a shape without a layout.
    #[cfg(test)]
    pub fn from_rect(rect: Rect, shade: Shade) -> Self {
        Self {
            parts: vec![ShapePart {
                rect,
                shade,
                kind: PartKind::Fill,
            }],
        }
    }
}

/// A bracket shape in its clip's space (a bar along one edge, hooks across its width), drawn from
/// the shape's outline.
#[derive(Debug, Clone, PartialEq)]
pub struct Bracket {
    parts: Vec<ShapePart>,
}

impl Bracket {
    /// The shape `key` in the space of its clip, mirrored horizontally when the clip is.
    pub fn resolve(
        layout: &UiLayout,
        key: &ShapeKey,
        mirrored: bool,
    ) -> Result<Self, UiLayoutError> {
        let shape = layout.get(key)?;
        let mut parts = outline_parts(key, &shape, &shape.placement.local.matrix)?;
        if mirrored {
            for part in &mut parts {
                part.rect = Rect::from_min_max(
                    Pos2::new(-part.rect.max.x, part.rect.min.y),
                    Pos2::new(-part.rect.min.x, part.rect.max.y),
                );
            }
        }
        Ok(Self { parts })
    }

    /// Paints the bracket with its clip's registration point at `origin`.
    pub fn paint(&self, painter: &Painter, tint: Color32, origin: Pos2) {
        for part in &self.parts {
            part.paint(painter, tint, origin.to_vec2());
        }
    }

    /// The box around everything the bracket paints, in its clip's space.
    pub fn bounds(&self) -> Rect {
        union_rect(self.parts.iter())
    }
}

/// The parts of one meter clip (`Pipboy_Meter`): the clip, its outline shape, and its fill and
/// ghost-fill clips with their shapes.
pub struct MeterKeys {
    pub clip: PlacementKey,
    pub bounds: BoundsKey,
    pub outline: ShapeKey,
    pub fill: PlacementKey,
    pub fill_shape: ShapeKey,
    pub ghost: PlacementKey,
    pub ghost_shape: ShapeKey,
}

impl KeyCheck for MeterKeys {
    fn failures(&self, layout: &UiLayout) -> Vec<UiLayoutError> {
        let failures = MeterParts(self).failures(layout);
        if failures.is_empty() {
            Meter::resolve(layout, self).err().into_iter().collect()
        } else {
            failures
        }
    }

    fn declare(&self, out: &mut Vec<Declaration>) {
        MeterParts(self).declare(out);
    }
}

struct MeterParts<'a>(&'a MeterKeys);

impl KeyCheck for MeterParts<'_> {
    fn failures(&self, layout: &UiLayout) -> Vec<UiLayoutError> {
        let keys = self.0;
        let parts: [&dyn KeyCheck; 7] = [
            &keys.clip,
            &keys.bounds,
            &keys.outline,
            &keys.fill,
            &keys.fill_shape,
            &keys.ghost,
            &keys.ghost_shape,
        ];
        parts
            .iter()
            .flat_map(|part| part.failures(layout))
            .collect()
    }

    fn declare(&self, out: &mut Vec<Declaration>) {
        let keys = self.0;
        keys.clip.declare(out);
        keys.bounds.declare(out);
        keys.outline.declare(out);
        keys.fill.declare(out);
        keys.fill_shape.declare(out);
        keys.ghost.declare(out);
        keys.ghost_shape.declare(out);
    }
}

/// One fill clip of a meter, in the meter's own space.
#[derive(Debug, Clone, Copy, PartialEq)]
struct MeterFill {
    local: Matrix,
    edges: [f32; 4],
    bounds_width: f32,
    shade: Shade,
}

impl MeterFill {
    fn resolve(
        layout: &UiLayout,
        clip: &PlacementKey,
        shape_key: &ShapeKey,
    ) -> Result<Self, UiLayoutError> {
        let placement = layout.get(clip)?;
        let shape = layout.get(shape_key)?;
        let local = placement.local.matrix.then(&shape.placement.local.matrix);
        axis_aligned(shape_key, &local)?;
        let bounds_width = shape.bounds[2] - shape.bounds[0];
        if bounds_width <= 0.0 {
            return Err(UiLayoutError::unresolved(
                shape_key.movie,
                shape_key.path,
                "the fill has no width",
            ));
        }
        Ok(Self {
            local,
            edges: shape.edge_bounds,
            bounds_width,
            shade: fill_shade(shape_key, &shape)?,
        })
    }

    /// The fill's edge rectangle in meter space after `fill.width = width` (meter units).
    fn rect(&self, width: f32) -> [f32; 4] {
        let scale_x = width / self.bounds_width;
        let [x0, y0, x1, y1] = self.edges;
        let x = |v: f32| self.local.tx + scale_x * v;
        let y = |v: f32| self.local.ty + self.local.d * v;
        [x(x0), y(y0), x(x1), y(y1)]
    }
}

/// The game's meter: a scaling-stroke outline drawn over a ghost fill and a fill, each fill as
/// wide as its value's share of the clip's initial width.
#[derive(Debug, Clone, PartialEq)]
pub struct Meter {
    world: Matrix,
    outline: Vec<ShapePart>,
    initial_width: f32,
    fill: MeterFill,
    ghost: MeterFill,
}

impl Meter {
    pub fn resolve(layout: &UiLayout, keys: &MeterKeys) -> Result<Self, UiLayoutError> {
        let clip = layout.get(&keys.clip)?;
        let world = clip.world.matrix;
        axis_aligned(&keys.clip, &world)?;
        if world.a <= 0.0 {
            return Err(UiLayoutError::unresolved(
                keys.clip.movie,
                keys.clip.path,
                "the meter is not scaled horizontally to a positive width",
            ));
        }
        let outline = layout.get(&keys.outline)?;
        let [bx0, _, bx1, _] = layout.get(&keys.bounds)?;
        Ok(Self {
            world,
            outline: outline_parts(&keys.outline, &outline, &outline.placement.world.matrix)?,
            initial_width: (bx1 - bx0) / world.a,
            fill: MeterFill::resolve(layout, &keys.fill, &keys.fill_shape)?,
            ghost: MeterFill::resolve(layout, &keys.ghost, &keys.ghost_shape)?,
        })
    }

    fn stage(&self, local: [f32; 4]) -> Rect {
        rect_of(self.world.bounds(local))
    }

    /// `SetMeter(value, ghost, max)`: the value is capped at the maximum, the ghost is not, and
    /// each fill shows only for a positive amount and maximum.
    pub fn paint(&self, painter: &Painter, tint: Color32, value: f32, ghost: f32, max: f32) {
        let value = value.min(max);
        for (part, amount) in [(&self.ghost, ghost), (&self.fill, value)] {
            if amount > 0.0 && max > 0.0 {
                let width = amount / max * self.initial_width;
                fill(
                    painter,
                    self.stage(part.rect(width)),
                    part.shade.colour(tint),
                );
            }
        }
        for part in &self.outline {
            part.paint(painter, tint, Vec2::ZERO);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_layout::test_support::{
        assert_close, identity_color, layout_with_movie, movie_json,
    };
    use serde_json::json;

    fn box_shape() -> OutlinedShape {
        OutlinedShape::from_rect(
            Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(30.0, 40.0)),
            Shade::WHITE,
        )
    }

    #[test]
    fn scaling_a_shape_keeps_the_top_left_of_its_bounds() {
        let scaled = box_shape().scaled(Vec2::new(2.0, 0.5));
        assert_eq!(scaled.bounds().min, Pos2::new(10.0, 20.0));
        assert_eq!(scaled.bounds().size(), Vec2::new(40.0, 10.0));
    }

    #[test]
    fn a_shape_is_fitted_to_a_target_rectangle() {
        let target = Rect::from_min_max(Pos2::new(0.0, 5.0), Pos2::new(8.0, 9.0));
        assert_eq!(box_shape().fitted_to(target).bounds(), target);
    }

    #[test]
    fn a_shape_takes_a_width_and_an_empty_shape_stays_empty() {
        assert_close(box_shape().with_width(5.0).bounds().width(), 5.0);
        let empty = box_shape().with_width(0.0);
        assert_close(empty.bounds().width(), 0.0);
        assert_close(empty.with_width(9.0).bounds().width(), 0.0);
    }

    const MOVIE: &str = "Fixture";
    const KEYS: MeterKeys = MeterKeys {
        clip: PlacementKey::new(MOVIE, "M"),
        bounds: BoundsKey::new(MOVIE, "M"),
        outline: ShapeKey::new(MOVIE, "M/@1"),
        fill: PlacementKey::new(MOVIE, "M/Fill_mc"),
        fill_shape: ShapeKey::new(MOVIE, "M/Fill_mc/@2"),
        ghost: PlacementKey::new(MOVIE, "M/GhostFill_mc"),
        ghost_shape: ShapeKey::new(MOVIE, "M/GhostFill_mc/@1"),
    };

    fn placed(char: u32, kind: &str, local: [f64; 6], world: [f64; 6]) -> serde_json::Value {
        json!({
            "char": char, "kind": kind, "depth": 1, "frames": [1, 1],
            "local": {"matrix": local, "color": identity_color(), "visible": true},
            "world": {"matrix": world, "color": identity_color()},
        })
    }

    /// A meter shaped like the game's: a 200 x 25 outline with a 2 px stroke, fills 100 px wide
    /// at scale (2, 0.25), the clip scaled by (0.5, 0.4) at (10, 20).
    fn meter() -> Meter {
        let movie = movie_json(
            MOVIE,
            &json!({
                "characters": {
                    "1": {"kind": "sprite", "frameCount": 1, "labels": {}},
                    "2": {"kind": "shape", "bounds": [-1, -1, 201, 26], "edgeBounds": [0, 0, 200, 25],
                          "fills": [],
                          "lines": [{"width": 2, "color": [255, 255, 255, 255], "join": "miter",
                                     "miterLimit": 3, "startCap": "none", "endCap": "none"}],
                          "paths": [{"fillLeft": 0, "fillRight": 0, "line": 1, "start": [200, 25],
                                     "edges": [[0, 25], [0, 0], [200, 0], [200, 25]]}]},
                    "3": {"kind": "shape", "bounds": [-0.5, -0.5, 100.5, 104], "edgeBounds": [0, 0, 100, 104],
                          "fills": [{"type": "solid", "color": [255, 255, 255, 255]}], "lines": [], "paths": []},
                    "4": {"kind": "shape", "bounds": [-0.5, -0.5, 100.5, 100.5], "edgeBounds": [0, 0, 100, 100],
                          "fills": [{"type": "solid", "color": [64, 64, 64, 255]}], "lines": [], "paths": []},
                },
                "instances": {
                    "M": placed(1, "sprite", [0.5, 0.0, 0.0, 0.4, 10.0, 20.0], [0.5, 0.0, 0.0, 0.4, 10.0, 20.0]),
                    "M/@1": placed(2, "shape", [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], [0.5, 0.0, 0.0, 0.4, 10.0, 20.0]),
                    "M/Fill_mc": placed(1, "sprite", [2.0, 0.0, 0.0, 0.25, 0.0, 0.0], [1.0, 0.0, 0.0, 0.1, 10.0, 20.0]),
                    "M/Fill_mc/@2": placed(3, "shape", [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.1, 10.0, 20.0]),
                    "M/GhostFill_mc": placed(1, "sprite", [2.0, 0.0, 0.0, 0.25, 0.0, 0.0], [1.0, 0.0, 0.0, 0.1, 10.0, 20.0]),
                    "M/GhostFill_mc/@1": placed(4, "shape", [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.1, 10.0, 20.0]),
                },
            }),
        );
        Meter::resolve(&layout_with_movie(MOVIE, &movie), &KEYS).unwrap()
    }

    #[test]
    fn the_outline_is_the_strokes_boxes_in_stage_space_without_overlap() {
        let outline = meter().outline;
        assert!(outline.iter().all(|part| part.kind == PartKind::Stroke));
        let bounds = union_rect(outline.iter());
        assert_close(bounds.min.x, 10.0 - 0.5);
        assert_close(bounds.max.x, 10.0 + 0.5 * 201.0);
        assert_close(bounds.min.y, 20.0 - 0.4);
        assert_close(bounds.max.y, 20.0 + 0.4 * 26.0);
        for (index, a) in outline.iter().enumerate() {
            for b in &outline[index + 1..] {
                assert!(!a.rect.intersects(b.rect) || a.rect.intersect(b.rect).area() < 1e-6);
            }
        }
    }

    #[test]
    fn a_meter_fill_with_no_width_is_refused() {
        let movie = movie_json(
            MOVIE,
            &json!({
                "characters": {
                    "1": {"kind": "sprite", "frameCount": 1, "labels": {}},
                    "2": {"kind": "shape", "bounds": [0, 0, 0, 5], "edgeBounds": [0, 0, 0, 5],
                          "fills": [{"type": "solid", "color": [255, 255, 255, 255]}], "lines": [], "paths": []},
                },
                "instances": {
                    "M/Fill_mc": placed(1, "sprite", [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
                    "M/Fill_mc/@2": placed(2, "shape", [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
                },
            }),
        );
        let layout = layout_with_movie(MOVIE, &movie);
        let error = MeterFill::resolve(&layout, &KEYS.fill, &KEYS.fill_shape).unwrap_err();
        assert!(error.to_string().contains("no width"));
    }

    #[test]
    fn a_shapes_fill_bounds_leave_out_its_strokes() {
        let stroke = ShapePart {
            rect: Rect::from_min_max(Pos2::new(-1.0, -1.0), Pos2::new(11.0, 11.0)),
            shade: Shade::WHITE,
            kind: PartKind::Stroke,
        };
        let fill = ShapePart {
            rect: Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(10.0, 10.0)),
            shade: Shade::WHITE,
            kind: PartKind::Fill,
        };
        let framed = OutlinedShape {
            parts: vec![stroke, fill],
        };
        assert_eq!(framed.bounds(), stroke.rect);
        assert_eq!(framed.fill_bounds(), Some(fill.rect));
        let only_stroked = OutlinedShape {
            parts: vec![stroke],
        };
        assert_eq!(only_stroked.fill_bounds(), None);
    }

    #[test]
    fn the_initial_width_is_the_clips_bounds_in_its_own_space() {
        assert_close(meter().initial_width, 202.0);
    }

    #[test]
    fn a_full_fill_spans_the_fill_edges_scaled_to_the_initial_width() {
        let meter = meter();
        let rect = meter.stage(meter.fill.rect(meter.initial_width));
        assert_close(rect.min.x, 10.0);
        assert_close(rect.width(), 0.5 * 100.0 * 202.0 / 101.0);
        assert_close(rect.height(), 0.4 * 0.25 * 104.0);
    }

    #[test]
    fn the_ghost_takes_its_shade_from_its_fill_colour() {
        assert_close(meter().ghost.shade.grey, 64.0 / 255.0);
    }

    #[test]
    fn a_shade_applies_the_colour_transform() {
        let transform = ColorTransform {
            mult: [0.5, 0.5, 0.5, 1.0],
            add: [0.0; 4],
        };
        let shade = Shade::of([255, 255, 255, 255], &transform);
        assert_close(shade.grey, 0.5);
        assert_close(shade.alpha, 1.0);
    }
}
