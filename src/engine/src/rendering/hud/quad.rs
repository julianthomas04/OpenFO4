//! HUD geometry primitives: a 2D colored vertex and a builder that expands
//! axis-aligned rectangles into clip-space triangles. Every v1 HUD element
//! (meter fill, bracket, crosshair) is a solid rectangle, matching FO4's HUD
//! primitives (all built from one white square shape). No textures.
//!
//! The builder also adds a subtle **drop shadow** behind every rect: FO4's HUD
//! shadow is a Scaleform/GFx render effect with no parameters stored with the
//! HUD movie, so this is an engine-side
//! approximation — a dark offset copy — not extracted FO4 data.

const VERTS_PER_RECT: usize = 6;

/// A rectangle in framebuffer pixels (origin top-left, y down).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PxRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl PxRect {
    /// The left, top, right and bottom edges as clip-space coordinates of a target of `target`
    /// pixels.
    pub fn clip_edges(self, target: (u32, u32)) -> [f32; 4] {
        let (width, height) = (target.0 as f32, target.1 as f32);
        [
            self.x / width * 2.0 - 1.0,
            1.0 - self.y / height * 2.0,
            (self.x + self.w) / width * 2.0 - 1.0,
            1.0 - (self.y + self.h) / height * 2.0,
        ]
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HudVertex {
    /// Clip-space position (x,y in [-1, 1], y up).
    pub pos: [f32; 2],
    /// Linear-RGBA "art" color, multiplied by the HUD tint in the shader by `tint_weight`.
    pub color: [f32; 4],
    /// How much of the HUD tint multiplies `color`: [`TINTED`] for art, [`UNTINTED`] for a colour
    /// that must reach the screen as it is (the opacity still applies).
    pub tint_weight: f32,
}

pub const TINTED: f32 = 1.0;
pub const UNTINTED: f32 = 0.0;

impl HudVertex {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        const ATTRS: [wgpu::VertexAttribute; 3] =
            wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4, 2 => Float32];
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<HudVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRS,
        }
    }
}

/// Accumulates HUD rectangles as clip-space triangles into a retained vertex
/// vector (cleared and refilled each frame, capacity preserved — no per-frame
/// allocation once warmed). `cull_mode: None` makes winding irrelevant.
pub struct QuadBuilder<'a> {
    verts: &'a mut Vec<HudVertex>,
    fb_w: f32,
    fb_h: f32,
    shadow_offset: f32,
    shadow_color: [f32; 4],
    /// Where the flat rectangles start, once the first one is pushed.
    shadowed_end: Option<usize>,
}

impl<'a> QuadBuilder<'a> {
    pub fn new(
        verts: &'a mut Vec<HudVertex>,
        fb_w: f32,
        fb_h: f32,
        shadow_offset: f32,
        shadow_color: [f32; 4],
    ) -> Self {
        verts.clear();
        Self {
            verts,
            fb_w,
            fb_h,
            shadow_offset,
            shadow_color,
            shadowed_end: None,
        }
    }

    /// Framebuffer-pixel point -> clip space (y-down pixels to y-up clip).
    fn to_clip(&self, x: f32, y: f32) -> [f32; 2] {
        [x / self.fb_w * 2.0 - 1.0, 1.0 - y / self.fb_h * 2.0]
    }

    fn raw_rect(&mut self, r: PxRect, color: [f32; 4], tint_weight: f32) {
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let tl = self.to_clip(r.x, r.y);
        let tr = self.to_clip(r.x + r.w, r.y);
        let bl = self.to_clip(r.x, r.y + r.h);
        let br = self.to_clip(r.x + r.w, r.y + r.h);
        let v = |pos| HudVertex {
            pos,
            color,
            tint_weight,
        };
        self.verts
            .extend_from_slice(&[v(tl), v(bl), v(br), v(tl), v(br), v(tr)]);
    }

    /// Push one filled rectangle. Its drop shadow is added by [`Self::finish`].
    /// Empty/inverted rectangles emit nothing.
    pub fn rect(&mut self, r: PxRect, color: [f32; 4]) {
        self.rect_weighted(r, color, TINTED);
    }

    /// [`Self::rect`] whose colour the HUD tint does not multiply.
    pub fn rect_untinted(&mut self, r: PxRect, color: [f32; 4]) {
        self.rect_weighted(r, color, UNTINTED);
    }

    fn rect_weighted(&mut self, r: PxRect, color: [f32; 4], tint_weight: f32) {
        debug_assert!(
            self.shadowed_end.is_none(),
            "a shadowed rectangle must be pushed before every flat one"
        );
        self.raw_rect(r, color, tint_weight);
    }

    /// Push one filled rectangle that casts no shadow and is drawn beneath every shadow and every
    /// shadowed rectangle (a translucent backdrop). Must come after the last [`Self::rect`].
    pub fn rect_flat(&mut self, r: PxRect, color: [f32; 4]) {
        self.shadowed_end.get_or_insert(self.verts.len());
        self.raw_rect(r, color, TINTED);
    }

    /// Puts a drop shadow (a dark copy offset down-right) behind every shadowed rectangle, all of
    /// them before every shadowed rectangle, so a shadow never covers a neighbouring element; the
    /// flat rectangles come before the shadows. A shadow's alpha scales with its element's own alpha so a semi-transparent piece (e.g. the crosshair
    /// dot fading out) casts a proportionally fainter shadow instead of a full-strength ghost.
    pub fn finish(self) {
        let shadowed = self.shadowed_end.unwrap_or(self.verts.len());
        if self.shadow_offset > 0.0 {
            let dx = self.shadow_offset / self.fb_w * 2.0;
            let dy = -self.shadow_offset / self.fb_h * 2.0;
            for index in (0..shadowed).step_by(VERTS_PER_RECT) {
                let alpha = self.shadow_color[3] * self.verts[index].color[3];
                if alpha <= 0.0 {
                    continue;
                }
                let mut color = self.shadow_color;
                color[3] = alpha;
                for vertex in index..index + VERTS_PER_RECT {
                    let pos = self.verts[vertex].pos;
                    self.verts.push(HudVertex {
                        pos: [pos[0] + dx, pos[1] + dy],
                        color,
                        tint_weight: TINTED,
                    });
                }
            }
        }
        self.verts.rotate_left(shadowed);
    }
}

/// The colour the HUD shader outputs for `vertex` under `tint`: `mix(color, color * tint,
/// tint_weight)`, before the opacity.
#[cfg(test)]
pub(crate) fn shader_rgb(vertex: &HudVertex, tint: [f32; 3]) -> [f32; 3] {
    let weight = vertex.tint_weight;
    std::array::from_fn(|channel| {
        let color = vertex.color[channel];
        color * (1.0 - weight) + color * tint[channel] * weight
    })
}

/// The rectangles of `verts` (as [`QuadBuilder`] writes them) in framebuffer pixels, with their
/// colour.
#[cfg(test)]
pub(crate) fn recorded_rects(verts: &[HudVertex], fb: (f32, f32)) -> Vec<(PxRect, [f32; 4])> {
    verts
        .chunks(VERTS_PER_RECT)
        .map(|rect| {
            let to_px = |pos: [f32; 2]| {
                (
                    f32::midpoint(pos[0], 1.0) * fb.0,
                    f32::midpoint(-pos[1], 1.0) * fb.1,
                )
            };
            let (left, top) = to_px(rect[0].pos);
            let (right, bottom) = to_px(rect[2].pos);
            let px = PxRect {
                x: left,
                y: top,
                w: right - left,
                h: bottom - top,
            };
            (px, rect[0].color)
        })
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the fixture arithmetic stays on exactly representable values"
)]
mod tests {
    use super::*;

    const SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.5];

    fn rect(x: f32) -> PxRect {
        PxRect {
            x,
            y: 10.0,
            w: 20.0,
            h: 20.0,
        }
    }

    const HUD_TINT: [f32; 3] = [0.5, 0.25, 1.0];

    fn assert_rgb_eq(actual: [f32; 3], expected: [f32; 3]) {
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-6, "{actual} vs {expected}");
        }
    }

    #[test]
    fn the_two_tint_weights_are_the_ends_of_the_shaders_mix() {
        assert_eq!(TINTED.to_bits(), 1.0_f32.to_bits());
        assert_eq!(UNTINTED.to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    fn a_tinted_vertex_comes_out_multiplied_by_the_hud_tint_and_an_untinted_one_unchanged() {
        let art = [0.8, 0.6, 0.4, 1.0];
        let mut tinted = Vec::new();
        let mut qb = QuadBuilder::new(&mut tinted, 100.0, 100.0, 0.0, SHADOW);
        qb.rect(rect(0.0), art);
        qb.finish();
        let mut untinted = Vec::new();
        let mut qb = QuadBuilder::new(&mut untinted, 100.0, 100.0, 0.0, SHADOW);
        qb.rect_untinted(rect(0.0), art);
        qb.finish();
        for vertex in &tinted {
            assert_rgb_eq(shader_rgb(vertex, HUD_TINT), [0.4, 0.15, 0.4]);
        }
        for vertex in &untinted {
            assert_rgb_eq(shader_rgb(vertex, HUD_TINT), [0.8, 0.6, 0.4]);
        }
    }

    #[test]
    fn the_shadow_of_an_untinted_rectangle_is_still_multiplied_by_the_hud_tint() {
        let mut verts = Vec::new();
        let mut qb = QuadBuilder::new(&mut verts, 100.0, 100.0, 2.0, [1.0; 4]);
        qb.rect_untinted(rect(0.0), [0.8, 0.6, 0.4, 1.0]);
        qb.finish();
        let (shadow, shape) = verts.split_at(VERTS_PER_RECT);
        for vertex in shadow {
            assert_rgb_eq(shader_rgb(vertex, HUD_TINT), HUD_TINT);
        }
        for vertex in shape {
            assert_rgb_eq(shader_rgb(vertex, HUD_TINT), [0.8, 0.6, 0.4]);
        }
    }

    #[test]
    fn every_shadow_is_drawn_before_every_shape() {
        let mut verts = Vec::new();
        let mut quads = QuadBuilder::new(&mut verts, 100.0, 100.0, 2.0, SHADOW);
        quads.rect(rect(0.0), [1.0, 0.0, 0.0, 1.0]);
        quads.rect(rect(30.0), [0.0, 1.0, 0.0, 1.0]);
        quads.finish();
        assert_eq!(verts.len(), 24);
        assert!(verts[..12].iter().all(|vertex| vertex.color == SHADOW));
        assert_eq!(verts[12].color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(verts[18].color, [0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn a_shadow_is_the_shape_moved_down_and_right_with_its_alpha_scaled() {
        let mut verts = Vec::new();
        let mut quads = QuadBuilder::new(&mut verts, 100.0, 100.0, 5.0, SHADOW);
        quads.rect(rect(0.0), [1.0, 1.0, 1.0, 0.5]);
        quads.finish();
        let (shadow, shape) = (verts[0], verts[6]);
        assert!((shadow.pos[0] - shape.pos[0] - 0.1).abs() < 1e-6);
        assert!((shape.pos[1] - shadow.pos[1] - 0.1).abs() < 1e-6);
        assert!((shadow.color[3] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn a_flat_rectangle_casts_no_shadow_and_lies_beneath_every_shadow_and_shape() {
        let (red, green) = ([1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]);
        let mut verts = Vec::new();
        let mut quads = QuadBuilder::new(&mut verts, 100.0, 100.0, 2.0, SHADOW);
        quads.rect(rect(0.0), red);
        quads.rect_flat(rect(30.0), green);
        quads.finish();
        assert_eq!(verts.len(), 18);
        assert!(verts[..6].iter().all(|vertex| vertex.color == green));
        assert!(verts[6..12].iter().all(|vertex| vertex.color == SHADOW));
        assert!(verts[12..].iter().all(|vertex| vertex.color == red));
    }

    #[test]
    fn flat_rectangles_keep_their_push_order() {
        let (first, second) = ([1.0, 0.0, 0.0, 0.5], [0.0, 0.0, 1.0, 0.5]);
        let mut verts = Vec::new();
        let mut quads = QuadBuilder::new(&mut verts, 100.0, 100.0, 0.0, SHADOW);
        quads.rect(rect(0.0), [1.0; 4]);
        quads.rect_flat(rect(30.0), first);
        quads.rect_flat(rect(60.0), second);
        quads.finish();
        assert_eq!(verts.len(), 18);
        assert_eq!(verts[0].color, first);
        assert_eq!(verts[6].color, second);
        assert_eq!(verts[12].color, [1.0; 4]);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "shadowed rectangle must be pushed before every flat one")]
    fn a_shadowed_rectangle_after_a_flat_one_is_refused() {
        let mut verts = Vec::new();
        let mut quads = QuadBuilder::new(&mut verts, 100.0, 100.0, 2.0, SHADOW);
        quads.rect_flat(rect(0.0), [1.0; 4]);
        quads.rect(rect(30.0), [1.0; 4]);
    }

    #[test]
    fn invisible_shapes_and_a_zero_offset_cast_no_shadow() {
        let mut verts = Vec::new();
        let mut quads = QuadBuilder::new(&mut verts, 100.0, 100.0, 2.0, SHADOW);
        quads.rect(rect(0.0), [1.0, 1.0, 1.0, 0.0]);
        quads.finish();
        assert_eq!(verts.len(), 6);

        let mut quads = QuadBuilder::new(&mut verts, 100.0, 100.0, 0.0, SHADOW);
        quads.rect(rect(0.0), [1.0, 1.0, 1.0, 1.0]);
        quads.finish();
        assert_eq!(verts.len(), 6);
    }
}
