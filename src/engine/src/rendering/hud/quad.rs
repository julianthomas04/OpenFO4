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
#[derive(Copy, Clone, Debug)]
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
    /// Linear-RGBA "art" color, multiplied by the HUD tint in the shader.
    pub color: [f32; 4],
}

impl HudVertex {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        const ATTRS: [wgpu::VertexAttribute; 2] =
            wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];
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
        }
    }

    /// Framebuffer-pixel point -> clip space (y-down pixels to y-up clip).
    fn to_clip(&self, x: f32, y: f32) -> [f32; 2] {
        [x / self.fb_w * 2.0 - 1.0, 1.0 - y / self.fb_h * 2.0]
    }

    fn raw_rect(&mut self, r: PxRect, color: [f32; 4]) {
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let tl = self.to_clip(r.x, r.y);
        let tr = self.to_clip(r.x + r.w, r.y);
        let bl = self.to_clip(r.x, r.y + r.h);
        let br = self.to_clip(r.x + r.w, r.y + r.h);
        let v = |pos| HudVertex { pos, color };
        self.verts
            .extend_from_slice(&[v(tl), v(bl), v(br), v(tl), v(br), v(tr)]);
    }

    /// Push one filled rectangle. Its drop shadow is added by [`Self::finish`].
    /// Empty/inverted rectangles emit nothing.
    pub fn rect(&mut self, r: PxRect, color: [f32; 4]) {
        self.raw_rect(r, color);
    }

    /// Puts a drop shadow (a dark copy offset down-right) behind every pushed rectangle, all of
    /// them before every rectangle, so a shadow never covers a neighbouring element. A shadow's
    /// alpha scales with its element's own alpha so a semi-transparent piece (e.g. the crosshair
    /// dot fading out) casts a proportionally fainter shadow instead of a full-strength ghost.
    pub fn finish(self) {
        let shapes = self.verts.len();
        if self.shadow_offset > 0.0 {
            let dx = self.shadow_offset / self.fb_w * 2.0;
            let dy = -self.shadow_offset / self.fb_h * 2.0;
            for index in (0..shapes).step_by(VERTS_PER_RECT) {
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
                    });
                }
            }
        }
        self.verts.rotate_left(shapes);
    }
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
