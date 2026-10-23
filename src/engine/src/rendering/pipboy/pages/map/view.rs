//! The map holders' scale, position, zoom, pan and marker rules: one holder per mode, each
//! showing the texture at its native size times the holder scale, as the menu's code does.

use egui::{Pos2, Rect, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapMode {
    World,
    Local,
}

impl MapMode {
    pub fn toggled(self) -> Self {
        match self {
            MapMode::World => MapMode::Local,
            MapMode::Local => MapMode::World,
        }
    }

    const ALL: [MapMode; 2] = [MapMode::World, MapMode::Local];

    fn index(self) -> usize {
        match self {
            MapMode::World => 0,
            MapMode::Local => 1,
        }
    }
}

/// The clip scale of a marker at map scale 0 and 1, unselected and selected.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkerScales {
    pub min: f32,
    pub max: f32,
    pub selected_min: f32,
    pub selected_max: f32,
}

/// The fractions of the border the holder may leave uncovered or must overshoot at each edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanGutters {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

/// Everything one mode's holder is authored with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModeParams {
    pub min_scale: f32,
    pub max_scale: f32,
    pub start_zoom: f32,
    pub wheel_speed: f32,
    pub drag_speed: f32,
    pub marker_scales: MarkerScales,
    pub pan_gutters: PanGutters,
    /// The mask clip's origin and size: the border the pan limits and the zoom centre use.
    pub border: Rect,
    /// The area the mask reveals.
    pub window: Rect,
    /// Where the texture and marker holders sit inside the map holder.
    pub texture_offset: Vec2,
    pub marker_offset: Vec2,
}

/// How far in from each side of the border a clamped marker is held.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClampInset {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rules {
    pub select_radius: f32,
    pub close_enough: f32,
    /// The fraction of pi on either side of the horizontal that counts as the left or right edge.
    pub border_sector: f32,
    pub clamp_inset: ClampInset,
}

/// A holder's scale parameter and its position in stage pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Holder {
    pub map_scale: f32,
    pub origin: Vec2,
}

/// Both holders; the current mode's is the one shown and driven.
#[derive(Clone, Debug)]
pub struct MapView {
    mode: MapMode,
    params: [ModeParams; 2],
    holders: [Holder; 2],
    rules: Rules,
    texture: Vec2,
}

impl MapView {
    pub fn new(world: ModeParams, local: ModeParams, rules: Rules) -> Self {
        let holder = |params: &ModeParams| Holder {
            map_scale: params.start_zoom,
            origin: Vec2::ZERO,
        };
        Self {
            mode: MapMode::World,
            holders: [holder(&world), holder(&local)],
            params: [world, local],
            rules,
            texture: Vec2::ZERO,
        }
    }

    pub fn mode(&self) -> MapMode {
        self.mode
    }

    pub fn params(&self) -> &ModeParams {
        &self.params[self.mode.index()]
    }

    fn holder(&self) -> &Holder {
        &self.holders[self.mode.index()]
    }

    fn holder_mut(&mut self) -> &mut Holder {
        &mut self.holders[self.mode.index()]
    }

    pub fn texture(&self) -> Vec2 {
        self.texture
    }

    pub fn window(&self) -> Rect {
        self.params().window
    }

    /// Learns the texture's native size; the first time, both holders start centred on its middle.
    pub fn set_texture(&mut self, size: Vec2) {
        let first = self.texture == Vec2::ZERO;
        self.texture = size;
        if first {
            for mode in MapMode::ALL {
                self.mode = mode;
                self.centre_on(size / 2.0);
            }
            self.mode = MapMode::World;
        }
    }

    fn scale_at(params: &ModeParams, map_scale: f32) -> f32 {
        params.min_scale + map_scale * (params.max_scale - params.min_scale)
    }

    /// The holder's scale: the stage pixels one texel spans.
    pub fn scale(&self) -> f32 {
        Self::scale_at(self.params(), self.holder().map_scale)
    }

    /// A marker clip's scale inside the holder.
    pub fn clip_scale(&self, selected: bool) -> f32 {
        let scales = self.params().marker_scales;
        let (low, high) = if selected {
            (scales.selected_min, scales.selected_max)
        } else {
            (scales.min, scales.max)
        };
        low + self.holder().map_scale * (high - low)
    }

    /// The scale a marker is drawn at on the stage: the holder's times the clip's.
    pub fn marker_scale(&self, selected: bool) -> f32 {
        self.scale() * self.clip_scale(selected)
    }

    /// Where the texture lands on the stage.
    pub fn texture_rect(&self) -> Rect {
        let scale = self.scale();
        let min = self.holder().origin + self.params().texture_offset * scale;
        Rect::from_min_size(min.to_pos2(), self.texture * scale)
    }

    /// The stage point of a marker clip position (in texels, inside the marker holder).
    pub fn marker_to_stage(&self, clip: Vec2) -> Pos2 {
        let offset = self.params().marker_offset;
        (self.holder().origin + (offset + clip) * self.scale()).to_pos2()
    }

    /// The marker clip position under a stage point.
    pub fn stage_to_marker(&self, stage: Pos2) -> Vec2 {
        (stage.to_vec2() - self.holder().origin) / self.scale() - self.params().marker_offset
    }

    fn close(&self, a: f32, b: f32) -> bool {
        (a - b).abs() < self.rules.close_enough
    }

    fn limits(&self) -> (Vec2, Vec2) {
        let params = self.params();
        let gutters = params.pan_gutters;
        let size = self.texture * self.scale();
        let border = params.border.size();
        let high = Vec2::new(border.x * gutters.left, border.y * gutters.top);
        let low = Vec2::new(
            -size.x + border.x * (1.0 - gutters.right),
            -size.y + border.y * (1.0 - gutters.bottom),
        );
        (low, high)
    }

    /// Moves the holder by `delta` stage pixels within its pan limits.
    pub fn move_map(&mut self, delta: Vec2) {
        let (low, high) = self.limits();
        let origin = self.holder().origin + delta;
        self.holder_mut().origin = Vec2::new(
            origin.x.max(low.x).min(high.x),
            origin.y.max(low.y).min(high.y),
        );
    }

    /// A pointer drag of `delta` stage pixels.
    pub fn drag(&mut self, delta: Vec2) {
        let speed = self.params().drag_speed;
        self.move_map(delta * speed);
    }

    /// A wheel delta; ignored while a drag is moving the map. Returns whether the scale changed.
    pub fn wheel(&mut self, delta: f32, moving: bool) -> bool {
        let speed = self.params().wheel_speed;
        self.zoom(delta * speed, moving)
    }

    /// Changes the map scale parameter by `step`, keeping the texel under the border's centre
    /// fixed. Returns whether it changed.
    pub fn zoom(&mut self, step: f32, moving: bool) -> bool {
        let before = self.holder().map_scale;
        let can_step = (step > 0.0 && before < 1.0 && !self.close(before, 1.0))
            || (step < 0.0 && before > 0.0 && !self.close(before, 0.0));
        if moving || !can_step || self.texture == Vec2::ZERO {
            return false;
        }
        let border = self.params().border.size();
        let scale = self.scale();
        let origin = self.holder().origin;
        let centre = (border / 2.0 - origin) / scale;
        self.holder_mut().map_scale = (before + step).clamp(0.0, 1.0);
        let target = -centre * self.scale() + border / 2.0;
        self.move_map(target - origin);
        !self.close(before, self.holder().map_scale)
    }

    /// Moves the holder so the texel at `map_position` sits at the middle of the border, the
    /// menu's own centring on a marker.
    pub fn centre_on(&mut self, map_position: Vec2) {
        let border = self.params().border;
        let target = -map_position * self.scale() + border.min.to_vec2() + border.size() / 2.0;
        let origin = self.holder().origin;
        self.move_map(target - origin);
    }

    /// Switches mode; the new holder restarts at its start zoom as it does when it is shown.
    pub fn show(&mut self, mode: MapMode) {
        self.mode = mode;
        self.restart_zoom();
    }

    pub fn restart_zoom(&mut self) {
        let start = self.params().start_zoom;
        self.holder_mut().map_scale = start;
    }

    /// The nearest candidate whose clip lies inside the square around the pointer.
    pub fn nearest(
        &self,
        pointer: Pos2,
        candidates: impl IntoIterator<Item = (usize, Vec2)>,
    ) -> Option<usize> {
        let radius = self.rules.select_radius;
        let mouse = (pointer.to_vec2() - self.holder().origin) / self.scale();
        let offset = self.params().marker_offset;
        let mut best: Option<(usize, f32)> = None;
        for (index, clip) in candidates {
            let at = clip + offset;
            let inside = at.x >= mouse.x - radius
                && at.x < mouse.x + radius
                && at.y >= mouse.y - radius
                && at.y < mouse.y + radius;
            let distance = (at - mouse).length();
            if inside && best.is_none_or(|(_, nearest)| distance < nearest) {
                best = Some((index, distance));
            }
        }
        best.map(|(index, _)| index)
    }

    /// A clamped marker's clip position: the border's inset rectangle's nearest point on the line
    /// to the border's centre when the marker lies outside it.
    pub fn clamped(&self, clip: Vec2) -> Vec2 {
        let params = self.params();
        let (scale, origin) = (self.scale(), self.holder().origin);
        let border = params.border;
        let inset = self.rules.clamp_inset;
        let top_left = (border.min.to_vec2() - origin + Vec2::new(inset.left, inset.top)) / scale;
        let size = Vec2::new(
            border.width() - (inset.left + inset.right),
            border.height() - (inset.top + inset.bottom),
        ) / scale;
        let rect = Rect::from_min_size(top_left.to_pos2(), size);
        let centre = (border.min.to_vec2() - origin + border.size() / 2.0) / scale;
        let at = clip + params.marker_offset;
        let inside =
            at.x >= rect.min.x && at.x < rect.max.x && at.y >= rect.min.y && at.y < rect.max.y;
        if inside {
            return clip;
        }
        border_intersection(rect, at, centre, self.rules.border_sector)
            .map_or(clip, |hit| hit - params.marker_offset)
    }
}

/// Where the segment from `centre` to `point` crosses the edge of `rect` that faces `point`,
/// held to that edge.
fn border_intersection(rect: Rect, point: Vec2, centre: Vec2, sector: f32) -> Option<Vec2> {
    let top_left = rect.left_top().to_vec2();
    let top_right = rect.right_top().to_vec2();
    let bottom_left = rect.left_bottom().to_vec2();
    let bottom_right = rect.right_bottom().to_vec2();
    let angle = (point.y - centre.y).atan2(point.x - centre.x);
    let near = std::f32::consts::PI * sector;
    let far = std::f32::consts::PI - near;
    if (near..=far).contains(&angle) {
        crossing(bottom_left, bottom_right, centre, point)
            .map(|hit| Vec2::new(hit.x.clamp(bottom_left.x, bottom_right.x), hit.y))
    } else if (-near..=near).contains(&angle) {
        crossing(top_right, bottom_right, centre, point)
            .map(|hit| Vec2::new(hit.x, hit.y.clamp(top_right.y, bottom_right.y)))
    } else if (-far..=-near).contains(&angle) {
        crossing(top_left, top_right, centre, point)
            .map(|hit| Vec2::new(hit.x.clamp(top_left.x, top_right.x), hit.y))
    } else {
        crossing(top_left, bottom_left, centre, point)
            .map(|hit| Vec2::new(hit.x, hit.y.clamp(top_left.y, bottom_left.y)))
    }
}

/// Where the segment `from`-`to` meets the edge `edge_start`-`edge_end`, when it does.
fn crossing(edge_start: Vec2, edge_end: Vec2, from: Vec2, to: Vec2) -> Option<Vec2> {
    let edge = edge_end - edge_start;
    let line = to - from;
    let denominator = edge.x * line.y - edge.y * line.x;
    if denominator == 0.0 {
        return None;
    }
    let offset = from - edge_start;
    let along = (offset.x * edge.y - offset.y * edge.x) / denominator;
    (0.0..=1.0).contains(&along).then(|| from + line * along)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// Synthetic authored values, not the game's.
    pub fn params() -> ModeParams {
        ModeParams {
            min_scale: 1.0,
            max_scale: 3.0,
            start_zoom: 0.25,
            wheel_speed: 0.5,
            drag_speed: 2.0,
            marker_scales: MarkerScales {
                min: 4.0,
                max: 2.0,
                selected_min: 5.0,
                selected_max: 3.0,
            },
            pan_gutters: PanGutters {
                left: 0.1,
                right: -0.1,
                top: 0.2,
                bottom: -0.2,
            },
            border: Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(200.0, 100.0)),
            window: Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(190.0, 90.0)),
            texture_offset: Vec2::new(0.0, 5.0),
            marker_offset: Vec2::new(0.0, 5.0),
        }
    }

    pub fn rules() -> Rules {
        Rules {
            select_radius: 10.0,
            close_enough: 0.001,
            border_sector: 0.25,
            clamp_inset: ClampInset {
                top: 6.0,
                bottom: 3.0,
                left: 2.0,
                right: 4.0,
            },
        }
    }

    pub const TEXTURE: Vec2 = Vec2::new(300.0, 240.0);

    pub fn view() -> MapView {
        let mut view = MapView::new(params(), params(), rules());
        view.set_texture(TEXTURE);
        view
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn the_holder_scale_runs_linearly_from_the_minimum_to_the_maximum() {
        let mut view = view();
        assert!(close(view.scale(), 1.5));
        view.holders[0].map_scale = 1.0;
        assert!(close(view.scale(), 3.0));
        view.holders[0].map_scale = 0.0;
        assert!(close(view.scale(), 1.0));
    }

    #[test]
    fn a_marker_is_drawn_at_the_holder_scale_times_its_clip_scale() {
        let view = view();
        assert!(close(view.clip_scale(false), 3.5));
        assert!(close(view.clip_scale(true), 4.5));
        assert!(close(view.marker_scale(false), 1.5 * 3.5));
    }

    #[test]
    fn the_texture_is_shown_at_its_native_size_times_the_scale() {
        let rect = view().texture_rect();
        assert!(close(rect.width(), TEXTURE.x * 1.5));
        assert!(close(rect.height(), TEXTURE.y * 1.5));
    }

    #[test]
    fn the_first_texture_size_centres_both_holders_on_its_middle() {
        let view = view();
        for holder in view.holders {
            let border = params().border;
            let centre = border.min.to_vec2() + border.size() / 2.0;
            let middle = holder.origin + TEXTURE / 2.0 * 1.5;
            assert!(close(middle.x, centre.x) && close(middle.y, centre.y));
        }
    }

    #[test]
    fn panning_stops_at_the_gutters() {
        let mut view = view();
        view.move_map(Vec2::splat(1.0e5));
        let border = params().border.size();
        assert!(close(view.holders[0].origin.x, border.x * 0.1));
        assert!(close(view.holders[0].origin.y, border.y * 0.2));
        view.move_map(Vec2::splat(-1.0e5));
        let size = TEXTURE * 1.5;
        assert!(close(view.holders[0].origin.x, -size.x + border.x * 1.1));
        assert!(close(view.holders[0].origin.y, -size.y + border.y * 1.2));
    }

    #[test]
    fn a_drag_moves_by_the_drag_speed() {
        let mut view = view();
        let before = view.holders[0].origin;
        view.drag(Vec2::new(-3.0, -2.0));
        assert!(close(view.holders[0].origin.x - before.x, -6.0));
        assert!(close(view.holders[0].origin.y - before.y, -4.0));
    }

    #[test]
    fn the_wheel_changes_the_scale_parameter_by_the_delta_times_the_speed() {
        let mut view = view();
        assert!(view.wheel(0.5, false));
        assert!(close(view.holders[0].map_scale, 0.5));
        assert!(view.wheel(10.0, false));
        assert!(close(view.holders[0].map_scale, 1.0));
        assert!(!view.wheel(1.0, false), "already at the top");
        assert!(view.wheel(-10.0, false));
        assert!(close(view.holders[0].map_scale, 0.0));
        assert!(!view.wheel(-1.0, false), "already at the bottom");
    }

    #[test]
    fn zooming_is_ignored_while_the_map_moves() {
        let mut view = view();
        assert!(!view.wheel(1.0, true));
        assert!(close(view.holders[0].map_scale, 0.25));
    }

    #[test]
    fn zooming_keeps_the_texel_at_the_border_centre_fixed() {
        let mut view = view();
        view.move_map(Vec2::new(-30.0, -20.0));
        let border = params().border.size();
        let texel = |view: &MapView| (border / 2.0 - view.holders[0].origin) / view.scale();
        let before = texel(&view);
        view.zoom(0.1, false);
        let after = texel(&view);
        assert!(close(before.x, after.x) && close(before.y, after.y));
    }

    #[test]
    fn centring_puts_the_texel_in_the_middle_of_the_border() {
        let mut view = view();
        view.centre_on(Vec2::new(100.0, 90.0));
        let border = params().border;
        let at = view.holders[0].origin + Vec2::new(100.0, 90.0) * view.scale();
        let middle = border.min.to_vec2() + border.size() / 2.0;
        assert!(close(at.x, middle.x) && close(at.y, middle.y));
    }

    #[test]
    fn showing_a_mode_restarts_its_zoom_and_keeps_its_position() {
        let mut view = view();
        view.wheel(1.0, false);
        view.move_map(Vec2::new(-10.0, 0.0));
        let origin = view.holders[0].origin;
        view.show(MapMode::Local);
        view.show(MapMode::World);
        assert!(close(view.holders[0].map_scale, 0.25));
        assert_eq!(view.holders[0].origin, origin);
    }

    #[test]
    fn the_nearest_marker_inside_the_square_wins_and_the_corner_beyond_the_radius_does_not() {
        let view = view();
        let holder = view.holders[0].origin;
        let scale = view.scale();
        let pointer = (holder + Vec2::new(60.0, 40.0) * scale).to_pos2();
        let at = |x: f32, y: f32| Vec2::new(x, y - 5.0);
        let candidates = [
            (0, at(60.0 + 9.0, 40.0 + 9.0)),
            (1, at(60.0 + 3.0, 40.0)),
            (2, at(60.0 + 12.0, 40.0)),
        ];
        assert_eq!(view.nearest(pointer, candidates), Some(1));
        assert_eq!(view.nearest(pointer, [candidates[2]]), None);
        assert_eq!(view.nearest(pointer, []), None);
    }

    #[test]
    fn the_pick_square_is_in_holder_units_whatever_the_scale() {
        let mut view = view();
        let hit = |view: &MapView, offset: f32| {
            let pointer = view.marker_to_stage(Vec2::new(60.0, 35.0));
            view.nearest(pointer, [(0, Vec2::new(60.0 + offset, 35.0))])
        };
        assert_eq!(hit(&view, 9.0), Some(0));
        assert_eq!(hit(&view, 11.0), None);
        view.wheel(2.0, false);
        assert_eq!(hit(&view, 9.0), Some(0));
        assert_eq!(hit(&view, 11.0), None);
    }

    #[test]
    fn stage_and_marker_positions_round_trip() {
        let view = view();
        let stage = view.marker_to_stage(Vec2::new(42.0, 17.0));
        let back = view.stage_to_marker(stage);
        assert!(close(back.x, 42.0) && close(back.y, 17.0));
    }

    #[test]
    fn a_marker_inside_the_inset_border_stays_and_one_outside_lands_on_its_edge() {
        let mut view = view();
        view.centre_on(Vec2::new(150.0, 120.0));
        let inside = view.stage_to_marker(Pos2::new(100.0, 70.0));
        assert_eq!(view.clamped(inside), inside);

        let far_right = view.stage_to_marker(Pos2::new(5000.0, 70.0));
        let clamped = view.clamped(far_right);
        let stage = view.marker_to_stage(clamped);
        let border = params().border;
        assert!(close(stage.x, border.max.x - rules().clamp_inset.right));
        assert!(border.min.y <= stage.y && stage.y <= border.max.y);

        let far_above = view.stage_to_marker(Pos2::new(100.0, -5000.0));
        let stage = view.marker_to_stage(view.clamped(far_above));
        assert!(close(stage.y, border.min.y + rules().clamp_inset.top));

        let far_below = view.stage_to_marker(Pos2::new(100.0, 5000.0));
        let stage = view.marker_to_stage(view.clamped(far_below));
        assert!(close(stage.y, border.max.y - rules().clamp_inset.bottom));

        let far_left = view.stage_to_marker(Pos2::new(-5000.0, 70.0));
        let stage = view.marker_to_stage(view.clamped(far_left));
        assert!(close(stage.x, border.min.x + rules().clamp_inset.left));
    }

    #[test]
    fn a_marker_straight_beyond_a_corner_is_held_to_that_corner() {
        let mut view = view();
        view.centre_on(Vec2::new(150.0, 120.0));
        let outside = view.stage_to_marker(Pos2::new(5000.0, 5000.0));
        let stage = view.marker_to_stage(view.clamped(outside));
        let border = params().border;
        assert!(close(stage.y, border.max.y - rules().clamp_inset.bottom));
        assert!(stage.x <= border.max.x);
    }

    #[test]
    fn parallel_lines_have_no_crossing() {
        let at = |x: f32, y: f32| Vec2::new(x, y);
        assert_eq!(
            crossing(at(0.0, 0.0), at(10.0, 0.0), at(0.0, 1.0), at(10.0, 1.0)),
            None
        );
        assert_eq!(
            crossing(at(0.0, 0.0), at(10.0, 0.0), at(5.0, -5.0), at(5.0, 5.0)),
            Some(at(5.0, 0.0))
        );
        assert_eq!(
            crossing(at(0.0, 0.0), at(10.0, 0.0), at(5.0, 1.0), at(5.0, 5.0)),
            None,
            "the segment stops short of the edge"
        );
    }
}
