//! The typed values the layout accessors return.

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

/// A 2D affine matrix in stage pixels: `x' = a*x + c*y + tx`, `y' = b*x + d*y + ty`.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(from = "[f64; 6]")]
pub struct Matrix {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl From<[f64; 6]> for Matrix {
    fn from([a, b, c, d, tx, ty]: [f64; 6]) -> Self {
        Self {
            a: a as f32,
            b: b as f32,
            c: c as f32,
            d: d as f32,
            tx: tx as f32,
            ty: ty as f32,
        }
    }
}

impl Matrix {
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    /// `self` applied after `inner`: a point of `inner`'s space mapped through both.
    #[must_use]
    pub fn then(&self, inner: &Matrix) -> Matrix {
        Matrix {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            tx: self.a * inner.tx + self.c * inner.ty + self.tx,
            ty: self.b * inner.tx + self.d * inner.ty + self.ty,
        }
    }

    /// The axis-aligned box around `bounds` `[xmin, ymin, xmax, ymax]` after this matrix.
    pub fn bounds(&self, [xmin, ymin, xmax, ymax]: [f32; 4]) -> [f32; 4] {
        let corners = [
            self.apply(xmin, ymin),
            self.apply(xmax, ymin),
            self.apply(xmin, ymax),
            self.apply(xmax, ymax),
        ];
        corners.iter().fold(
            [
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ],
            |[x0, y0, x1, y1], (x, y)| [x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y)],
        )
    }

    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.tx,
            self.b * x + self.d * y + self.ty,
        )
    }

    pub fn is_axis_aligned(&self) -> bool {
        self.b == 0.0 && self.c == 0.0
    }
}

/// Per channel `out = in * mult + add`, channels r, g, b, a; `add` in 0..255 units.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct ColorTransform {
    pub mult: [f32; 4],
    pub add: [f32; 4],
}

impl ColorTransform {
    pub const IDENTITY: Self = Self {
        mult: [1.0; 4],
        add: [0.0; 4],
    };
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Filter {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub params: HashMap<String, serde_json::Value>,
}

/// The state of one display object at one frame.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalPlacement {
    pub matrix: Matrix,
    pub color: ColorTransform,
    pub visible: bool,
    pub blend: Option<String>,
    pub clip_depth: Option<u32>,
    pub ratio: Option<u32>,
    pub cache_as_bitmap: Option<bool>,
    pub class_name: Option<String>,
    pub filters: Option<Vec<Filter>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TrackKey {
    pub frame: u32,
    #[serde(flatten)]
    pub placement: LocalPlacement,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct World {
    pub matrix: Matrix,
    pub color: ColorTransform,
}

/// A display object: where it sits locally and composed up to its path's root.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Placement {
    pub depth: i32,
    pub name: Option<String>,
    /// First and last frame of its life on the parent timeline.
    pub frames: [u32; 2],
    pub local: LocalPlacement,
    pub world: World,
    pub track: Option<Vec<TrackKey>>,
}

impl Placement {
    pub fn track(&self) -> &[TrackKey] {
        self.track.as_deref().unwrap_or(&[])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    Left,
    Right,
    Center,
    Justify,
}

/// A text field as authored: character data plus placement.
#[derive(Debug, Clone, PartialEq)]
pub struct TextField {
    /// Authored bounds `[xmin, ymin, xmax, ymax]` in the field's own space.
    pub bounds: [f32; 4],
    /// Font class name, resolved to a family through the font configuration.
    pub font: String,
    pub size: f32,
    pub color: Option<[u8; 4]>,
    pub align: Align,
    pub left_margin: f32,
    pub right_margin: f32,
    pub indent: f32,
    pub leading: f32,
    pub letter_spacing: f32,
    pub multiline: bool,
    pub word_wrap: bool,
    pub auto_size: bool,
    pub html: bool,
    pub initial_text: Option<String>,
    pub placement: Placement,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Fill {
    #[serde(rename = "type")]
    pub kind: String,
    pub color: Option<[u8; 4]>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Line {
    pub width: f32,
    pub color: Option<[u8; 4]>,
    #[serde(default)]
    pub start_cap: Option<String>,
    #[serde(default)]
    pub end_cap: Option<String>,
    #[serde(default)]
    pub join: Option<String>,
    #[serde(default)]
    pub miter_limit: Option<f32>,
}

/// One edge of a shape's outline in the shape's pixels: a straight line or a quadratic curve to
/// its end point.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Edge {
    Straight([f32; 2]),
    Curve([f32; 4]),
}

impl Edge {
    pub fn end(self) -> [f32; 2] {
        match self {
            Self::Straight(end) => end,
            Self::Curve([_, _, x, y]) => [x, y],
        }
    }
}

/// A run of edges drawn with one set of styles. Style numbers index the shape's fills and lines
/// from 1; 0 is none.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapePath {
    pub fill_left: usize,
    pub fill_right: usize,
    pub line: usize,
    pub start: [f32; 2],
    pub edges: Vec<Edge>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    /// Bounds including half the stroke widths, as Flash measures a display object.
    pub bounds: [f32; 4],
    /// Bounds of the edges alone (the filled area); the stroke-inclusive bounds when the file
    /// records none.
    pub edge_bounds: [f32; 4],
    pub fills: Vec<Fill>,
    pub lines: Vec<Line>,
    /// The outline every document records; empty for a shape without edges.
    pub paths: Vec<ShapePath>,
    pub placement: Placement,
}

/// A movie's stage rectangle in its own pixels and its frame rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovieStage {
    pub rect: [f32; 4],
    pub fps: f32,
}

/// Frame count and labels of a sprite or the main timeline (frames are 1-based).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timeline {
    pub frame_count: u32,
    pub labels: BTreeMap<String, u32>,
}

/// One value of an `ActionScript` literal list.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    Flag(bool),
    Number(f64),
    Text(String),
}

impl Scalar {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Flag(_) => "a boolean",
            Self::Number(_) => "a number",
            Self::Text(_) => "a string",
        }
    }
}
