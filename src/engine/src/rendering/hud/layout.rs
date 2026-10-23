//! Stage layout. A movie is authored on a stage of its own size (the HUD movie's stage is a
//! reference resolution) and uniformly scaled to the live framebuffer, with elements pinned to
//! screen corners or the centre the way the game anchors its groups to the safe rect.
//!
//! The menus use the same uniform scale for their stage and centre it on the framebuffer
//! ([`Layout::stage_point`]).

use super::quad::PxRect;

/// The corners the HUD meters are pinned to (the crosshair uses [`Layout::center`]).
///
/// Stand-in S1: the converted layout positions the meter groups on the stage but not how the
/// game anchors them on other aspect ratios, so the meters are pinned to the framebuffer corners.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// Bottom-left (HP meter).
    Bl,
    /// Bottom-right (AP meter).
    Br,
}

/// Maps a stage onto the live framebuffer with a uniform, aspect-preserving scale (FO4 scales the UI uniformly, it does
/// not stretch).
pub struct Layout {
    fb_w: f32,
    fb_h: f32,
    stage_w: f32,
    stage_h: f32,
    scale: f32,
}

impl Layout {
    /// A layout of a stage of `stage_w` × `stage_h` pixels.
    pub fn for_stage(stage_w: f32, stage_h: f32, fb_w: f32, fb_h: f32) -> Self {
        let scale = (fb_w / stage_w).min(fb_h / stage_h).max(f32::MIN_POSITIVE);
        Self {
            fb_w,
            fb_h,
            stage_w,
            stage_h,
            scale,
        }
    }

    /// Framebuffer pixels per stage pixel.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Scale a stage length to framebuffer pixels.
    pub fn s(&self, v: f32) -> f32 {
        v * self.scale
    }

    /// Screen center in framebuffer pixels (crosshair origin).
    pub fn center(&self) -> (f32, f32) {
        (self.fb_w * 0.5, self.fb_h * 0.5)
    }

    /// The framebuffer position of the stage point `(x, y)` when the stage is scaled and
    /// centred on the framebuffer; the rest of the framebuffer is letterbox.
    pub fn stage_point(&self, x: f32, y: f32) -> (f32, f32) {
        (
            (self.fb_w - self.stage_w * self.scale) / 2.0 + x * self.scale,
            (self.fb_h - self.stage_h * self.scale) / 2.0 + y * self.scale,
        )
    }

    /// The whole framebuffer in stage coordinates, as `[xmin, ymin, xmax, ymax]`: the stage
    /// plus the letterbox, which the movie sees as its visible rectangle.
    pub fn visible_stage_rect(&self) -> [f32; 4] {
        let (left, top) = self.stage_point(0.0, 0.0);
        [
            -left / self.scale,
            -top / self.scale,
            (self.fb_w - left) / self.scale,
            (self.fb_h - top) / self.scale,
        ]
    }

    /// Place a box of stage size `vw`×`vh` at `anchor`, inset `ox`/`oy`
    /// stage px from the anchored corner. Returns a framebuffer-px rect.
    pub fn place(&self, anchor: Anchor, vw: f32, vh: f32, ox: f32, oy: f32) -> PxRect {
        let (w, h, ox, oy) = (self.s(vw), self.s(vh), self.s(ox), self.s(oy));
        let (x, y) = match anchor {
            Anchor::Bl => (ox, self.fb_h - oy - h),
            Anchor::Br => (self.fb_w - ox - w, self.fb_h - oy - h),
        };
        PxRect { x, y, w, h }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the arithmetic below stays on exactly representable values"
)]
mod tests {
    use super::*;

    #[test]
    fn the_visible_rect_is_the_screen_in_stage_units() {
        let same = Layout::for_stage(1280.0, 720.0, 2560.0, 1440.0);
        assert_eq!(same.visible_stage_rect(), [0.0, 0.0, 1280.0, 720.0]);
        let wide = Layout::for_stage(1280.0, 720.0, 3000.0, 1440.0);
        assert_eq!(wide.visible_stage_rect(), [-110.0, 0.0, 1390.0, 720.0]);
        let tall = Layout::for_stage(1280.0, 720.0, 1280.0, 1000.0);
        assert_eq!(tall.visible_stage_rect(), [0.0, -140.0, 1280.0, 860.0]);
    }

    #[test]
    fn a_stage_is_scaled_to_the_framebuffer() {
        let layout = Layout::for_stage(640.0, 360.0, 1920.0, 1080.0);
        assert_eq!(layout.scale(), 3.0);
        assert_eq!(layout.s(10.0), 30.0);
    }

    #[test]
    fn the_smaller_axis_ratio_decides_the_scale() {
        assert_eq!(
            Layout::for_stage(1280.0, 720.0, 3440.0, 1440.0).scale(),
            2.0
        );
        assert_eq!(
            Layout::for_stage(1280.0, 720.0, 1280.0, 1000.0).scale(),
            1.0
        );
    }

    #[test]
    fn a_stage_of_another_size_scales_by_its_own_size() {
        assert_eq!(Layout::for_stage(640.0, 360.0, 1920.0, 1080.0).scale(), 3.0);
    }

    #[test]
    fn a_stage_point_on_a_16_by_9_screen_has_no_letterbox() {
        let layout = Layout::for_stage(640.0, 360.0, 1920.0, 1080.0);
        assert_eq!(layout.stage_point(0.0, 0.0), (0.0, 0.0));
        assert_eq!(layout.stage_point(100.0, 200.0), (300.0, 600.0));
    }

    #[test]
    fn a_stage_point_on_an_ultrawide_screen_is_pillarboxed() {
        let layout = Layout::for_stage(640.0, 360.0, 3440.0, 1440.0);
        assert_eq!(layout.stage_point(0.0, 0.0), (440.0, 0.0));
    }

    #[test]
    fn a_stage_point_on_a_tall_screen_is_letterboxed() {
        let layout = Layout::for_stage(640.0, 360.0, 1280.0, 1000.0);
        assert_eq!(layout.stage_point(0.0, 0.0), (0.0, 140.0));
    }
}
