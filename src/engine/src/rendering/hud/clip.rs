//! Flattens a sprite's timeline from the layout into one list of solid rectangles per frame.
//!
//! The clip's direct children are placed per frame from their instance life and track. Their own
//! children are shown as the sprite starts: nested timelines play from their first frame, which is
//! the only frame these clips use. Coordinates are relative to the clip's origin.

use super::shape::solid_fill;
use crate::ui_layout::values::{LocalPlacement, Matrix, Placement};
use crate::ui_layout::{Movie, UiLayoutError};

const RIGHT_ANGLE_TOLERANCE: f32 = 1e-3;

/// A solid rectangle of one frame: its colour is the shape's fill times the colour transforms of
/// the chain, its alpha likewise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub colour: [f32; 3],
    pub alpha: f32,
}

#[derive(Debug, thiserror::Error)]
pub enum ClipError {
    #[error(transparent)]
    Layout(#[from] UiLayoutError),
    #[error("the clip '{0}' has labelled frames, which the flattening does not handle")]
    Labelled(String),
    #[error("'{0}' is rotated by something other than a right angle, so it is no rectangle")]
    NotARectangle(String),
}

impl ClipError {
    /// The error as a layout failure of the clip at `key` in `movie`.
    pub fn into_layout(self, movie: &str, key: &str) -> UiLayoutError {
        match self {
            Self::Layout(error) => error,
            other => UiLayoutError::unresolved(movie, key, other.to_string()),
        }
    }
}

#[derive(Clone, Copy)]
struct Chain {
    matrix: Matrix,
    colour: [f32; 3],
    alpha: f32,
}

impl Chain {
    const START: Self = Self {
        matrix: Matrix {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: 0.0,
            ty: 0.0,
        },
        colour: [1.0; 3],
        alpha: 1.0,
    };

    fn then(self, local: &LocalPlacement) -> Self {
        let [mr, mg, mb, ma] = local.color.mult;
        Self {
            matrix: compose(self.matrix, local.matrix),
            colour: [
                self.colour[0] * mr,
                self.colour[1] * mg,
                self.colour[2] * mb,
            ],
            alpha: self.alpha * ma,
        }
    }
}

/// `parent` applied after `child`.
fn compose(parent: Matrix, child: Matrix) -> Matrix {
    Matrix {
        a: parent.a * child.a + parent.c * child.b,
        b: parent.b * child.a + parent.d * child.b,
        c: parent.a * child.c + parent.c * child.d,
        d: parent.b * child.c + parent.d * child.d,
        tx: parent.a * child.tx + parent.c * child.ty + parent.tx,
        ty: parent.b * child.tx + parent.d * child.ty + parent.ty,
    }
}

/// The placement of `placement` at `frame` of its parent's timeline, or `None` while it is not on
/// the display list.
fn local_at(placement: &Placement, frame: u32) -> Option<&LocalPlacement> {
    let [first, last] = placement.frames;
    if frame < first || frame > last {
        return None;
    }
    let keyed = placement
        .track()
        .iter()
        .rev()
        .find(|key| key.frame <= frame)
        .map(|key| &key.placement);
    Some(keyed.unwrap_or(&placement.local))
}

/// A path of the clip's subtree split into its view (`""` or `":3"`) and the child segments.
struct ClipPath {
    view: String,
    segments: Vec<String>,
}

impl ClipPath {
    fn parse(root: &str, path: &str) -> Option<Self> {
        let rest = path.strip_prefix(root)?;
        let (view, below) = match rest.find('/') {
            Some(slash) => rest.split_at(slash),
            None => (rest, ""),
        };
        let segments: Vec<String> = below
            .split('/')
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect();
        let nested_view = segments.iter().skip(1).any(|segment| segment.contains(':'));
        (!segments.is_empty() && !nested_view).then(|| Self {
            view: view.to_owned(),
            segments,
        })
    }

    fn ancestor(&self, root: &str, depth: usize) -> String {
        format!("{root}{}/{}", self.view, self.segments[..depth].join("/"))
    }
}

fn is_not_a_shape(error: &UiLayoutError) -> bool {
    matches!(error, UiLayoutError::WrongKind { .. })
}

/// The rectangles of every frame `1..=frame_count` of the sprite at `root`, as frame lists indexed
/// from 0.
pub fn flatten_clip(
    movie: &Movie,
    movie_name: &str,
    root: &str,
    frame_count: u32,
) -> Result<Vec<Vec<ClipRect>>, ClipError> {
    if !movie.instance_timeline(root)?.labels.is_empty() {
        return Err(ClipError::Labelled(root.to_owned()));
    }
    let mut shapes = Vec::new();
    for path in movie.paths_under(root) {
        let Some(parsed) = ClipPath::parse(root, &path) else {
            continue;
        };
        match movie.shape(&path) {
            Ok(shape) => shapes.push((path, parsed, shape)),
            Err(error) if is_not_a_shape(&error) => {}
            Err(error) => return Err(error.into()),
        }
    }

    let mut frames = vec![Vec::new(); frame_count as usize];
    for (path, parsed, shape) in &shapes {
        let fill = solid_fill(movie_name, path, shape)?;
        for (index, rects) in frames.iter_mut().enumerate() {
            let frame = index as u32 + 1;
            let Some(chain) = chain_at(movie, root, parsed, frame)? else {
                continue;
            };
            let alpha = chain.alpha * fill[3];
            if alpha <= 0.0 {
                continue;
            }
            let colour = [
                fill[0] * chain.colour[0],
                fill[1] * chain.colour[1],
                fill[2] * chain.colour[2],
            ];
            if let Some(rect) = rectangle(path, shape.bounds, chain.matrix, colour, alpha)? {
                rects.push(rect);
            }
        }
    }
    Ok(frames)
}

/// The composed transform of the shape's ancestors at `frame`, or `None` when one is not shown.
fn chain_at(
    movie: &Movie,
    root: &str,
    path: &ClipPath,
    frame: u32,
) -> Result<Option<Chain>, ClipError> {
    let mut chain = Chain::START;
    for depth in 1..=path.segments.len() {
        let key = path.ancestor(root, depth);
        let placement = movie.placement(&key)?;
        let local = if depth == 1 {
            local_at(&placement, frame)
        } else {
            Some(&placement.local)
        };
        let Some(local) = local.filter(|local| local.visible) else {
            return Ok(None);
        };
        chain = chain.then(local);
    }
    Ok(Some(chain))
}

fn rectangle(
    path: &str,
    bounds: [f32; 4],
    matrix: Matrix,
    colour: [f32; 3],
    alpha: f32,
) -> Result<Option<ClipRect>, ClipError> {
    let scale = [matrix.a, matrix.b, matrix.c, matrix.d]
        .iter()
        .fold(0.0_f32, |largest, value| largest.max(value.abs()))
        .max(f32::MIN_POSITIVE);
    let tolerance = RIGHT_ANGLE_TOLERANCE * scale;
    let upright = matrix.b.abs() <= tolerance && matrix.c.abs() <= tolerance;
    let quarter_turn = matrix.a.abs() <= tolerance && matrix.d.abs() <= tolerance;
    if !upright && !quarter_turn {
        return Err(ClipError::NotARectangle(path.to_owned()));
    }
    let [xmin, ymin, xmax, ymax] = bounds;
    let corners = [
        matrix.apply(xmin, ymin),
        matrix.apply(xmax, ymax),
        matrix.apply(xmin, ymax),
        matrix.apply(xmax, ymin),
    ];
    let (mut left, mut top) = (f32::MAX, f32::MAX);
    let (mut right, mut bottom) = (f32::MIN, f32::MIN);
    for (x, y) in corners {
        left = left.min(x);
        right = right.max(x);
        top = top.min(y);
        bottom = bottom.max(y);
    }
    let (w, h) = (right - left, bottom - top);
    Ok((w > 0.0 && h > 0.0).then_some(ClipRect {
        x: left,
        y: top,
        w,
        h,
        colour,
        alpha,
    }))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::ui_layout::test_support::{
        assert_close, identity_color, instance, layout_with_movie, movie_json,
    };

    fn shape_character(bounds: [f64; 4]) -> Value {
        json!({"kind": "shape", "bounds": bounds, "fills": [{"type": "solid", "color": [255, 255, 255, 255]}], "lines": [], "paths": []})
    }

    fn sprite_character(frame_count: u32) -> Value {
        json!({"kind": "sprite", "frameCount": frame_count, "labels": {}})
    }

    fn at(mut value: Value, frames: [u32; 2], matrix: [f64; 6]) -> Value {
        value["frames"] = json!(frames);
        value["local"]["matrix"] = json!(matrix);
        value
    }

    fn fixture() -> crate::ui_layout::UiLayout {
        let mut grow = at(
            instance(2, "sprite", 0.0, 0.0),
            [2, 3],
            [1.0, 0.0, 0.0, 1.0, 10.0, 0.0],
        );
        grow["track"] = json!([
            {"frame": 2, "matrix": [1, 0, 0, 1, 10, 0], "color": identity_color(), "visible": true},
            {"frame": 3, "matrix": [2, 0, 0, 1, 20, 0], "color": identity_color(), "visible": true},
        ]);
        let mut turned = at(
            instance(2, "sprite", 0.0, 0.0),
            [3, 3],
            [0.0, 1.0, -1.0, 0.0, 0.0, 0.0],
        );
        turned["name"] = json!("turned");
        let parts = json!({
            "characters": {
                "1": sprite_character(3),
                "2": sprite_character(1),
                "3": shape_character([0.0, 0.0, 4.0, 2.0]),
            },
            "instances": {
                "Clip": instance(1, "sprite", 640.0, 360.0),
                "Clip:2/@1": grow,
                "Clip:2/@1/@1": instance(3, "shape", 0.0, 0.0),
                "Clip:3/@3": turned,
                "Clip:3/@3/@1": instance(3, "shape", 0.0, 0.0),
            },
        });
        layout_with_movie("Fixture", &movie_json("Fixture", &parts))
    }

    #[test]
    fn each_frame_lists_the_children_alive_then_placed_by_their_track() {
        let layout = fixture();
        let frames = flatten_clip(&layout.movie("Fixture").unwrap(), "Fixture", "Clip", 3).unwrap();
        assert_eq!(frames[0], []);
        let second = frames[1][0];
        assert_close(second.x, 10.0);
        assert_close(second.w, 4.0);
        let third = &frames[2];
        assert_close(third[0].x, 20.0);
        assert_close(third[0].w, 8.0);
        assert_close(third[0].h, 2.0);
    }

    #[test]
    fn a_quarter_turn_swaps_width_and_height() {
        let layout = fixture();
        let frames = flatten_clip(&layout.movie("Fixture").unwrap(), "Fixture", "Clip", 3).unwrap();
        let turned = frames[2][1];
        assert_close(turned.w, 2.0);
        assert_close(turned.h, 4.0);
    }

    #[test]
    fn a_piece_with_zero_alpha_is_dropped() {
        let mut layout_parts = json!({
            "characters": {
                "1": sprite_character(1),
                "3": shape_character([0.0, 0.0, 4.0, 2.0]),
            },
            "instances": {
                "Clip": instance(1, "sprite", 0.0, 0.0),
                "Clip/@1": instance(3, "shape", 0.0, 0.0),
            },
        });
        layout_parts["instances"]["Clip/@1"]["local"]["color"]["mult"] = json!([1, 1, 1, 0]);
        let layout = layout_with_movie("Fixture", &movie_json("Fixture", &layout_parts));
        let frames = flatten_clip(&layout.movie("Fixture").unwrap(), "Fixture", "Clip", 1).unwrap();
        assert_eq!(frames[0], []);
    }

    #[test]
    fn a_skewed_piece_is_an_error_not_a_guess() {
        let parts = json!({
            "characters": {
                "1": sprite_character(1),
                "3": shape_character([0.0, 0.0, 4.0, 2.0]),
            },
            "instances": {
                "Clip": instance(1, "sprite", 0.0, 0.0),
                "Clip/@1": at(instance(3, "shape", 0.0, 0.0), [1, 1], [1.0, 0.5, 0.5, 1.0, 0.0, 0.0]),
            },
        });
        let layout = layout_with_movie("Fixture", &movie_json("Fixture", &parts));
        let error =
            flatten_clip(&layout.movie("Fixture").unwrap(), "Fixture", "Clip", 1).unwrap_err();
        assert!(matches!(error, ClipError::NotARectangle(_)));
    }
}
