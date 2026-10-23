//! A shape's outline as the boxes it paints: its fills as the union of the boxes inside their
//! edges (even-odd), and each line style's strokes as the union of one box per segment, every
//! segment reaching half a line width past its right-angled joins (a round join is drawn as the
//! mitre it differs from by less than a tenth of the line width squared). Boxes never overlap, so
//! a translucent fill or stroke has one opacity everywhere.
//! Only axis-aligned outlines can be reduced to boxes; anything else is reported as such.

#![allow(
    clippy::float_cmp,
    reason = "outline coordinates are twips divided by 20, equal exactly when they coincide"
)]

use super::values::{Edge, Line, Shape, ShapePath};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartKind {
    Fill,
    Stroke,
}

/// A box of the shape's outline, in the shape's pixels, and the colour it is painted in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutlinePart {
    /// `[xmin, ymin, xmax, ymax]`.
    pub rect: [f32; 4],
    pub colour: [u8; 4],
    pub kind: PartKind,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutlineError {
    #[error("the shape records no outline")]
    NoOutline,
    #[error("an edge is a curve or is neither horizontal nor vertical")]
    NotAxisAligned,
    #[error("fill {0} is not a solid colour")]
    NotSolid(usize),
    #[error("line style {0} is not in the shape")]
    UnknownStyle(usize),
    #[error("the stroke has {0}, which is not drawn")]
    UnsupportedStroke(&'static str),
}

/// The widest mitre a right-angled corner needs, as a ratio of the stroke width.
const RIGHT_ANGLE_MITRE: f32 = std::f32::consts::SQRT_2;

/// The boxes of every fill and stroke of `shape`: fills in style order, then strokes in the
/// order their line styles first appear. `pixel` is the size of one stage pixel in the shape's
/// pixels along each axis: a hairline (line width 0) is one stage pixel thick, as in Flash.
pub fn parts(shape: &Shape, pixel: [f32; 2]) -> Result<Vec<OutlinePart>, OutlineError> {
    if shape.paths.is_empty() {
        return Err(OutlineError::NoOutline);
    }
    let mut parts = Vec::new();
    for (index, fill) in shape.fills.iter().enumerate() {
        let edges = fill_edges(&shape.paths, index + 1)?;
        if edges.is_empty() {
            continue;
        }
        let colour = fill.color.ok_or(OutlineError::NotSolid(index + 1))?;
        parts.extend(inside_boxes(&edges).into_iter().map(|rect| OutlinePart {
            rect,
            colour,
            kind: PartKind::Fill,
        }));
    }
    let mut styles: Vec<usize> = Vec::new();
    for path in shape.paths.iter().filter(|path| path.line > 0) {
        if !styles.contains(&path.line) {
            styles.push(path.line);
        }
    }
    for style in styles {
        let line = shape
            .lines
            .get(style - 1)
            .ok_or(OutlineError::UnknownStyle(style))?;
        let colour = line
            .color
            .ok_or(OutlineError::UnsupportedStroke("a gradient"))?;
        require_right_angle_joins(line)?;
        let mut boxes = Vec::new();
        for path in shape.paths.iter().filter(|path| path.line == style) {
            boxes.extend(stroke_boxes(path, line, pixel)?);
        }
        parts.extend(union_boxes(&boxes).into_iter().map(|rect| OutlinePart {
            rect,
            colour,
            kind: PartKind::Stroke,
        }));
    }
    Ok(parts)
}

struct Segment {
    from: [f32; 2],
    to: [f32; 2],
}

impl Segment {
    fn is_vertical(&self) -> bool {
        self.from[0] == self.to[0]
    }

    fn is_horizontal(&self) -> bool {
        self.from[1] == self.to[1]
    }

    fn is_empty(&self) -> bool {
        self.is_vertical() && self.is_horizontal()
    }
}

fn segments(path: &ShapePath) -> Result<Vec<Segment>, OutlineError> {
    let mut from = path.start;
    path.edges
        .iter()
        .map(|edge| {
            let Edge::Straight(to) = *edge else {
                return Err(OutlineError::NotAxisAligned);
            };
            let segment = Segment { from, to };
            if !segment.is_vertical() && !segment.is_horizontal() {
                return Err(OutlineError::NotAxisAligned);
            }
            from = to;
            Ok(segment)
        })
        .collect()
}

fn fill_edges(paths: &[ShapePath], style: usize) -> Result<Vec<Segment>, OutlineError> {
    let mut edges = Vec::new();
    for path in paths {
        for side in [path.fill_left, path.fill_right] {
            if side == style {
                edges.extend(segments(path)?);
            }
        }
    }
    Ok(edges)
}

fn sorted_unique(mut values: Vec<f32>) -> Vec<f32> {
    values.sort_by(f32::total_cmp);
    values.dedup();
    values
}

/// The boxes inside `edges` by the even-odd rule: the grid their coordinates span, each cell kept
/// when an odd number of vertical edges lies to its left across its height, merged into runs
/// along x and then into boxes along y.
fn inside_boxes(edges: &[Segment]) -> Vec<[f32; 4]> {
    let verticals: Vec<&Segment> = edges
        .iter()
        .filter(|edge| edge.is_vertical() && !edge.is_horizontal())
        .collect();
    let xs = sorted_unique(verticals.iter().map(|edge| edge.from[0]).collect());
    let ys = sorted_unique(
        edges
            .iter()
            .filter(|edge| edge.is_horizontal())
            .map(|edge| edge.from[1])
            .collect(),
    );
    merge_cells(&xs, &ys, |x, y| {
        let crossings = verticals
            .iter()
            .filter(|edge| {
                let top = edge.from[1].min(edge.to[1]);
                let bottom = edge.from[1].max(edge.to[1]);
                edge.from[0] < x && top <= y && y < bottom
            })
            .count();
        crossings % 2 == 1
    })
}

/// The union of `rects` as boxes that do not overlap.
fn union_boxes(rects: &[[f32; 4]]) -> Vec<[f32; 4]> {
    let rects: Vec<&[f32; 4]> = rects
        .iter()
        .filter(|[x0, y0, x1, y1]| x0 < x1 && y0 < y1)
        .collect();
    let xs = sorted_unique(rects.iter().flat_map(|r| [r[0], r[2]]).collect());
    let ys = sorted_unique(rects.iter().flat_map(|r| [r[1], r[3]]).collect());
    merge_cells(&xs, &ys, |x, y| {
        rects
            .iter()
            .any(|r| r[0] < x && x < r[2] && r[1] < y && y < r[3])
    })
}

/// The cells of the grid `xs` x `ys` whose middle satisfies `covered`, merged into runs along x
/// and then into boxes along y.
fn merge_cells(xs: &[f32], ys: &[f32], covered: impl Fn(f32, f32) -> bool) -> Vec<[f32; 4]> {
    let mut boxes: Vec<[f32; 4]> = Vec::new();
    let mut previous_row: Vec<usize> = Vec::new();
    for row in ys.windows(2) {
        let middle = f32::midpoint(row[0], row[1]);
        let mut runs: Vec<[f32; 2]> = Vec::new();
        for cell in xs.windows(2) {
            if !covered(f32::midpoint(cell[0], cell[1]), middle) {
                continue;
            }
            match runs.last_mut() {
                Some(run) if run[1] == cell[0] => run[1] = cell[1],
                _ => runs.push([cell[0], cell[1]]),
            }
        }
        let mut this_row = Vec::new();
        for run in runs {
            let above = previous_row.iter().copied().find(|&index| {
                let above = boxes[index];
                above[0] == run[0] && above[2] == run[1] && above[3] == row[0]
            });
            if let Some(index) = above {
                boxes[index][3] = row[1];
                this_row.push(index);
            } else {
                boxes.push([run[0], row[0], run[1], row[1]]);
                this_row.push(boxes.len() - 1);
            }
        }
        previous_row = this_row;
    }
    boxes
}

fn require_right_angle_joins(line: &Line) -> Result<(), OutlineError> {
    let right_angle_join = match line.join.as_deref() {
        Some("miter") => line
            .miter_limit
            .is_some_and(|limit| limit >= RIGHT_ANGLE_MITRE),
        Some("round") => true,
        _ => false,
    };
    if right_angle_join {
        Ok(())
    } else {
        Err(OutlineError::UnsupportedStroke(
            "a join that is neither a right-angle mitre nor round",
        ))
    }
}

/// The overlapping boxes of one path's stroke: each segment as thick as the line and reaching
/// half a line width past its ends where it joins another segment.
fn stroke_boxes(
    path: &ShapePath,
    line: &Line,
    pixel: [f32; 2],
) -> Result<Vec<[f32; 4]>, OutlineError> {
    let thickness = if line.width > 0.0 {
        [line.width, line.width]
    } else {
        pixel
    };
    let half = thickness.map(|width| width / 2.0);
    let mut segments = segments(path)?;
    let closed = segments.last().is_some_and(|last| last.to == path.start);
    if !closed {
        extend_open_ends(&mut segments, line, half)?;
    }
    let last = segments.len().saturating_sub(1);
    Ok(segments
        .iter()
        .enumerate()
        .filter(|(_, segment)| !segment.is_empty())
        .map(|(index, segment)| {
            let (joined_start, joined_end) = (closed || index > 0, closed || index < last);
            let [x0, x1] = ordered(segment.from[0], segment.to[0]);
            let [y0, y1] = ordered(segment.from[1], segment.to[1]);
            let reach = |joined: bool, half: f32| if joined { half } else { 0.0 };
            if segment.is_vertical() {
                let (before, after) = if segment.from[1] <= segment.to[1] {
                    (joined_start, joined_end)
                } else {
                    (joined_end, joined_start)
                };
                [
                    x0 - half[0],
                    y0 - reach(before, half[1]),
                    x1 + half[0],
                    y1 + reach(after, half[1]),
                ]
            } else {
                let (before, after) = if segment.from[0] <= segment.to[0] {
                    (joined_start, joined_end)
                } else {
                    (joined_end, joined_start)
                };
                [
                    x0 - reach(before, half[0]),
                    y0 - half[1],
                    x1 + reach(after, half[0]),
                    y1 + half[1],
                ]
            }
        })
        .collect())
}

/// Moves the free ends of an open stroke outwards by half the line width when its caps stick
/// out (square, or round which is drawn as square), as the caps of a closed path never show.
/// `half` is half the line's thickness across a vertical and across a horizontal segment.
fn extend_open_ends(
    segments: &mut [Segment],
    line: &Line,
    half: [f32; 2],
) -> Result<(), OutlineError> {
    let sticks_out = |cap: &Option<String>| match cap.as_deref() {
        Some("none") => Ok(false),
        Some("square" | "round") => Ok(true),
        _ => Err(OutlineError::UnsupportedStroke("an unknown cap")),
    };
    let push = |from: [f32; 2], to: [f32; 2]| {
        let direction = [(to[0] - from[0]).signum(), (to[1] - from[1]).signum()];
        let step = |axis: usize| {
            if from[axis] == to[axis] {
                0.0
            } else {
                direction[axis] * half[axis]
            }
        };
        [to[0] + step(0), to[1] + step(1)]
    };
    let (extend_end, extend_start) = (sticks_out(&line.end_cap)?, sticks_out(&line.start_cap)?);
    if let Some(last) = segments.last_mut().filter(|_| extend_end) {
        last.to = push(last.from, last.to);
    }
    if let Some(first) = segments.first_mut().filter(|_| extend_start) {
        first.from = push(first.to, first.from);
    }
    Ok(())
}

fn ordered(a: f32, b: f32) -> [f32; 2] {
    [a.min(b), a.max(b)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(left: f32, top: f32, right: f32, bottom: f32) -> Vec<Edge> {
        vec![
            Edge::Straight([right, top]),
            Edge::Straight([right, bottom]),
            Edge::Straight([left, bottom]),
            Edge::Straight([left, top]),
        ]
    }

    fn path(
        fill_left: usize,
        fill_right: usize,
        line: usize,
        start: [f32; 2],
        edges: Vec<Edge>,
    ) -> ShapePath {
        ShapePath {
            fill_left,
            fill_right,
            line,
            start,
            edges,
        }
    }

    fn shape(fills: usize, lines: Vec<Line>, paths: Vec<ShapePath>) -> Shape {
        Shape {
            bounds: [0.0; 4],
            edge_bounds: [0.0; 4],
            fills: (0..fills)
                .map(|index| crate::ui_layout::values::Fill {
                    kind: "solid".to_owned(),
                    color: Some([index as u8, 0, 0, 255]),
                })
                .collect(),
            lines,
            paths,
            placement: crate::ui_layout::test_support::placement(0.0, 0.0),
        }
    }

    fn mitred_line(width: f32) -> Line {
        Line {
            width,
            color: Some([9, 9, 9, 255]),
            start_cap: Some("none".to_owned()),
            end_cap: Some("none".to_owned()),
            join: Some("miter".to_owned()),
            miter_limit: Some(3.0),
        }
    }

    const STAGE_PIXEL: [f32; 2] = [1.0, 1.0];

    fn outlined(shape: &Shape) -> Result<Vec<OutlinePart>, OutlineError> {
        parts(shape, STAGE_PIXEL)
    }

    fn area(boxes: &[[f32; 4]]) -> f32 {
        boxes
            .iter()
            .map(|[x0, y0, x1, y1]| (x1 - x0) * (y1 - y0))
            .sum()
    }

    fn assert_disjoint(boxes: &[[f32; 4]]) {
        for (index, a) in boxes.iter().enumerate() {
            for b in &boxes[index + 1..] {
                let overlaps = a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
                assert!(!overlaps, "{a:?} overlaps {b:?}");
            }
        }
    }

    fn rects(parts: &[OutlinePart], kind: PartKind) -> Vec<[f32; 4]> {
        parts
            .iter()
            .filter(|part| part.kind == kind)
            .map(|part| part.rect)
            .collect()
    }

    #[test]
    fn a_rectangle_fills_its_own_box() {
        let shape = shape(
            1,
            vec![],
            vec![path(0, 1, 0, [10.0, 4.0], rectangle(10.0, 4.0, 30.0, 9.0))],
        );
        let parts = outlined(&shape).unwrap();
        assert_eq!(rects(&parts, PartKind::Fill), [[10.0, 4.0, 30.0, 9.0]]);
        assert_eq!(parts[0].colour, [0, 0, 0, 255]);
    }

    #[test]
    fn the_fill_side_does_not_change_the_area() {
        let left = shape(
            1,
            vec![],
            vec![path(1, 0, 0, [0.0, 0.0], rectangle(0.0, 0.0, 5.0, 5.0))],
        );
        assert_eq!(
            rects(&outlined(&left).unwrap(), PartKind::Fill),
            [[0.0, 0.0, 5.0, 5.0]]
        );
    }

    #[test]
    fn an_l_shaped_outline_is_covered_by_boxes_without_overlap() {
        let outline = vec![
            Edge::Straight([2.0, 0.0]),
            Edge::Straight([2.0, 9.0]),
            Edge::Straight([9.0, 9.0]),
            Edge::Straight([9.0, 11.0]),
            Edge::Straight([0.0, 11.0]),
            Edge::Straight([0.0, 0.0]),
        ];
        let shape = shape(1, vec![], vec![path(1, 0, 0, [0.0, 0.0], outline)]);
        let boxes = rects(&outlined(&shape).unwrap(), PartKind::Fill);
        assert_eq!(boxes, [[0.0, 0.0, 2.0, 9.0], [0.0, 9.0, 9.0, 11.0]]);
    }

    #[test]
    fn an_inner_loop_is_a_hole() {
        let outer = rectangle(0.0, 0.0, 30.0, 30.0);
        let mut hole = rectangle(10.0, 10.0, 20.0, 20.0);
        hole.reverse();
        let shape = shape(
            1,
            vec![],
            vec![
                path(1, 0, 0, [0.0, 0.0], outer),
                path(1, 0, 0, [10.0, 10.0], hole),
            ],
        );
        assert_eq!(
            area(&rects(&outlined(&shape).unwrap(), PartKind::Fill)),
            800.0
        );
    }

    #[test]
    fn a_bracket_of_three_runs_covers_its_hook_and_its_bar_with_the_gaps_between_them() {
        let hook = vec![
            Edge::Straight([0.0, -28.0]),
            Edge::Straight([0.0, -24.0]),
            Edge::Straight([2.0, -24.0]),
            Edge::Straight([2.0, -26.0]),
            Edge::Straight([9.0, -26.0]),
            Edge::Straight([9.0, -28.0]),
        ];
        let upper_bar = vec![
            Edge::Straight([2.0, -14.5]),
            Edge::Straight([0.0, -14.5]),
            Edge::Straight([0.0, -23.5]),
            Edge::Straight([2.0, -23.5]),
        ];
        let lower_bar = vec![
            Edge::Straight([2.0, 2.0]),
            Edge::Straight([0.0, 2.0]),
            Edge::Straight([0.0, -14.0]),
            Edge::Straight([2.0, -14.0]),
        ];
        let shape = shape(
            1,
            vec![],
            vec![
                path(1, 0, 0, [9.0, -28.0], hook),
                path(0, 1, 0, [2.0, -23.5], upper_bar),
                path(0, 1, 0, [2.0, -14.0], lower_bar),
            ],
        );
        assert_eq!(
            rects(&outlined(&shape).unwrap(), PartKind::Fill),
            [
                [0.0, -28.0, 9.0, -26.0],
                [0.0, -26.0, 2.0, -24.0],
                [0.0, -23.5, 2.0, -14.5],
                [0.0, -14.0, 2.0, 2.0],
            ]
        );
    }

    #[test]
    fn the_edge_between_two_runs_of_one_fill_disappears() {
        let shape = shape(
            1,
            vec![],
            vec![
                path(1, 0, 0, [0.0, 0.0], rectangle(0.0, 0.0, 4.0, 4.0)),
                path(1, 0, 0, [4.0, 0.0], rectangle(4.0, 0.0, 8.0, 4.0)),
            ],
        );
        assert_eq!(
            rects(&outlined(&shape).unwrap(), PartKind::Fill),
            [[0.0, 0.0, 8.0, 4.0]]
        );
    }

    #[test]
    fn each_fill_keeps_its_own_colour() {
        let shape = shape(
            2,
            vec![],
            vec![
                path(1, 0, 0, [0.0, 0.0], rectangle(0.0, 0.0, 4.0, 4.0)),
                path(0, 2, 0, [10.0, 0.0], rectangle(10.0, 0.0, 14.0, 4.0)),
            ],
        );
        let colours: Vec<u8> = outlined(&shape)
            .unwrap()
            .iter()
            .map(|part| part.colour[0])
            .collect();
        assert_eq!(colours, [0, 1]);
    }

    #[test]
    fn a_stroked_rectangle_is_a_frame_around_its_edges() {
        let shape = shape(
            1,
            vec![mitred_line(1.0)],
            vec![path(
                0,
                1,
                1,
                [1.0, 25.75],
                rectangle(1.0, 25.75, 391.0, 26.75),
            )],
        );
        let parts = outlined(&shape).unwrap();
        let union_x = rects(&parts, PartKind::Stroke)
            .iter()
            .fold([f32::MAX, f32::MIN], |[lo, hi], b| {
                [lo.min(b[0]), hi.max(b[2])]
            });
        let union_y = rects(&parts, PartKind::Stroke)
            .iter()
            .fold([f32::MAX, f32::MIN], |[lo, hi], b| {
                [lo.min(b[1]), hi.max(b[3])]
            });
        assert_eq!(union_x, [0.5, 391.5]);
        assert_eq!(union_y, [25.25, 27.25]);
        assert_eq!(rects(&parts, PartKind::Fill), [[1.0, 25.75, 391.0, 26.75]]);
    }

    #[test]
    fn a_closed_stroke_is_one_frame_whose_boxes_do_not_overlap() {
        let shape = shape(
            0,
            vec![mitred_line(2.0)],
            vec![path(0, 0, 1, [0.0, 0.0], rectangle(0.0, 0.0, 10.0, 10.0))],
        );
        let boxes = rects(&outlined(&shape).unwrap(), PartKind::Stroke);
        assert_disjoint(&boxes);
        assert_eq!(area(&boxes), 12.0 * 12.0 - 8.0 * 8.0);
        assert_eq!(boxes.iter().map(|b| b[0]).fold(f32::MAX, f32::min), -1.0);
        assert_eq!(boxes.iter().map(|b| b[2]).fold(f32::MIN, f32::max), 11.0);
    }

    #[test]
    fn an_open_stroke_has_no_corner_at_its_ends() {
        let shape = shape(
            0,
            vec![mitred_line(2.0)],
            vec![path(
                0,
                0,
                1,
                [0.0, 0.0],
                vec![Edge::Straight([10.0, 0.0]), Edge::Straight([10.0, 10.0])],
            )],
        );
        let boxes = rects(&outlined(&shape).unwrap(), PartKind::Stroke);
        assert_disjoint(&boxes);
        assert_eq!(area(&boxes), 22.0 + 22.0 - 4.0);
        assert_eq!(boxes.iter().map(|b| b[0]).fold(f32::MAX, f32::min), 0.0);
        assert_eq!(boxes.iter().map(|b| b[1]).fold(f32::MAX, f32::min), -1.0);
    }

    #[test]
    fn a_vertex_between_collinear_segments_adds_nothing() {
        let line = |edges: Vec<Edge>| {
            let shape = shape(
                0,
                vec![mitred_line(2.0)],
                vec![path(0, 0, 1, [0.0, 0.0], edges)],
            );
            rects(&outlined(&shape).unwrap(), PartKind::Stroke)
        };
        let split = line(vec![
            Edge::Straight([5.0, 0.0]),
            Edge::Straight([10.0, 0.0]),
        ]);
        assert_eq!(split, line(vec![Edge::Straight([10.0, 0.0])]));
    }

    #[test]
    fn the_strokes_of_several_paths_of_one_style_are_composited_once() {
        let shape = shape(
            0,
            vec![mitred_line(2.0)],
            vec![
                path(0, 0, 1, [0.0, 0.0], vec![Edge::Straight([10.0, 0.0])]),
                path(0, 0, 1, [5.0, -5.0], vec![Edge::Straight([5.0, 5.0])]),
            ],
        );
        let boxes = rects(&outlined(&shape).unwrap(), PartKind::Stroke);
        assert_disjoint(&boxes);
        assert_eq!(area(&boxes), 10.0 * 2.0 + 10.0 * 2.0 - 2.0 * 2.0);
    }

    #[test]
    fn a_hairline_is_one_stage_pixel_thick() {
        let shape = shape(
            0,
            vec![mitred_line(0.0)],
            vec![path(0, 0, 1, [0.0, 0.0], vec![Edge::Straight([10.0, 0.0])])],
        );
        let at = |pixel: [f32; 2]| rects(&parts(&shape, pixel).unwrap(), PartKind::Stroke);
        assert_eq!(at([1.0, 1.0]), [[0.0, -0.5, 10.0, 0.5]]);
        assert_eq!(at([0.5, 0.25]), [[0.0, -0.125, 10.0, 0.125]]);
    }

    #[test]
    fn curves_and_diagonals_are_not_boxes() {
        let curve = shape(
            1,
            vec![],
            vec![path(
                1,
                0,
                0,
                [0.0, 0.0],
                vec![Edge::Curve([1.0, 1.0, 2.0, 0.0])],
            )],
        );
        assert_eq!(outlined(&curve), Err(OutlineError::NotAxisAligned));
        let diagonal = shape(
            1,
            vec![],
            vec![path(1, 0, 0, [0.0, 0.0], vec![Edge::Straight([2.0, 3.0])])],
        );
        assert_eq!(outlined(&diagonal), Err(OutlineError::NotAxisAligned));
    }

    #[test]
    fn a_shape_without_paths_has_no_outline() {
        assert_eq!(
            outlined(&shape(1, vec![], vec![])),
            Err(OutlineError::NoOutline)
        );
    }

    #[test]
    fn bevelled_strokes_are_refused() {
        let mut bevel = mitred_line(1.0);
        bevel.join = Some("bevel".to_owned());
        let shape = shape(
            0,
            vec![bevel],
            vec![path(0, 0, 1, [0.0, 0.0], rectangle(0.0, 0.0, 4.0, 4.0))],
        );
        assert_eq!(
            outlined(&shape),
            Err(OutlineError::UnsupportedStroke(
                "a join that is neither a right-angle mitre nor round"
            ))
        );
    }

    #[test]
    fn a_round_join_is_drawn_like_the_mitre() {
        let mut round = mitred_line(2.0);
        let mitre_shape = shape(
            0,
            vec![round.clone()],
            vec![path(0, 0, 1, [0.0, 0.0], rectangle(0.0, 0.0, 4.0, 4.0))],
        );
        round.join = Some("round".to_owned());
        let round_shape = shape(
            0,
            vec![round],
            vec![path(0, 0, 1, [0.0, 0.0], rectangle(0.0, 0.0, 4.0, 4.0))],
        );
        assert_eq!(outlined(&round_shape), outlined(&mitre_shape));
    }

    #[test]
    fn the_caps_of_an_open_stroke_stick_out_and_those_of_a_closed_one_do_not() {
        let mut capped = mitred_line(2.0);
        capped.start_cap = Some("square".to_owned());
        capped.end_cap = Some("square".to_owned());
        let open = shape(
            0,
            vec![capped.clone()],
            vec![path(0, 0, 1, [0.0, 0.0], vec![Edge::Straight([4.0, 0.0])])],
        );
        let boxes = outlined(&open).unwrap();
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].rect, [-1.0, -1.0, 5.0, 1.0]);
        let closed = shape(
            0,
            vec![capped],
            vec![path(0, 0, 1, [0.0, 0.0], rectangle(0.0, 0.0, 4.0, 4.0))],
        );
        let frame = rects(&outlined(&closed).unwrap(), PartKind::Stroke);
        assert_eq!(area(&frame), 6.0 * 6.0 - 2.0 * 2.0);
    }
}
