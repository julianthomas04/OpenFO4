//! Rasterised Pip-Boy art: SVG frames from the art library turned into egui textures at the
//! current stage scale.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use egui::{ColorImage, Pos2, Rect, TextureHandle, TextureId, TextureOptions, Vec2};
use resvg::usvg;

use crate::assets::pipboy_art::{ArtClip, ArtLibrary, FixedArt};
use crate::rendering::svg_raster::{self, BadRasterSize};

/// Texture memory the cache keeps before it drops the least recently used frames; large enough
/// for several whole animated clips at the capped scale.
const DEFAULT_BUDGET_BYTES: usize = 128 * 1024 * 1024;
/// Animated clips are rasterised at most this many pixels per stage pixel; the GPU upscales them.
const ANIMATED_SCALE_CAP: f32 = 1.5;
/// The stage scale is rounded to a multiple of `1 / SCALE_STEPS` when picking a raster scale.
const SCALE_STEPS: f32 = 16.0;

/// One rasterised frame, ready to paint. `rect` is in stage pixels relative to the clip's
/// registration point; add the placement point to draw it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArtSprite {
    pub texture: TextureId,
    pub rect: Rect,
}

#[derive(Debug, thiserror::Error)]
pub enum RasterError {
    #[error("frame '{path}' could not be read: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("frame is not valid SVG: {0}")]
    Parse(#[from] usvg::Error),
    #[error("frame would rasterise to {width}x{height} pixels")]
    BadSize { width: u32, height: u32 },
}

impl From<BadRasterSize> for RasterError {
    fn from(BadRasterSize { width, height }: BadRasterSize) -> Self {
        Self::BadSize { width, height }
    }
}

/// A frame rasterised on the CPU, ready to become a texture.
struct Raster {
    image: ColorImage,
    /// Placement rectangle relative to the registration point, in stage pixels.
    rect: Rect,
}

/// Rasterises an SVG whose `viewBox` is `bounds` (stage pixels) at `scale` raster pixels per stage
/// pixel.
fn rasterise(svg: &[u8], bounds: [f32; 4], scale: f32) -> Result<Raster, RasterError> {
    let [x_min, y_min, x_max, y_max] = bounds;
    let size = Vec2::new(x_max - x_min, y_max - y_min) * scale;
    let (width, height) = (size.x.ceil() as u32, size.y.ceil() as u32);
    let tree = usvg::Tree::from_data(svg, &usvg::Options::default())?;
    let pixmap = svg_raster::render(&tree, width, height)?;
    Ok(Raster {
        image: ColorImage::from_rgba_premultiplied(
            [width as usize, height as usize],
            pixmap.data(),
        ),
        rect: Rect::from_min_size(
            Pos2::new(x_min, y_min),
            Vec2::new(width as f32, height as f32) / scale,
        ),
    })
}

struct Entry {
    texture: TextureHandle,
    rect: Rect,
    bytes: usize,
    last_used: u64,
}

/// A colour matrix applied to rasterised art: output channel `i` is `rows[i]` dotted with the
/// input `(r, g, b, a, 1)`, all in 0..1 units on non-premultiplied colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColourMatrix {
    pub rows: [[f32; 5]; 4],
}

impl ColourMatrix {
    fn cache_id(&self) -> u32 {
        self.rows
            .iter()
            .flatten()
            .fold(0x811c_9dc5_u32, |hash, value| {
                (hash ^ value.to_bits()).wrapping_mul(0x0100_0193)
            })
    }

    fn apply(&self, image: &mut ColorImage) {
        for pixel in &mut image.pixels {
            let [r, g, b, a] = pixel.to_srgba_unmultiplied().map(|c| f32::from(c) / 255.0);
            let input = [r, g, b, a, 1.0];
            let [nr, ng, nb, na] = self.rows.map(|row| {
                row.iter()
                    .zip(input)
                    .map(|(weight, value)| weight * value)
                    .sum::<f32>()
                    .clamp(0.0, 1.0)
            });
            *pixel = egui::Color32::from_rgba_unmultiplied(
                (nr * 255.0).round() as u8,
                (ng * 255.0).round() as u8,
                (nb * 255.0).round() as u8,
                (na * 255.0).round() as u8,
            );
        }
    }
}

/// Which frame of which clip at which raster scale and colour matrix: the clip's index in the
/// cache, the first timeline slot showing the same file, the raster scale's bits, the symbol
/// scale's bits and the matrix id (`0` for none).
type FrameKey = (u32, u32, u32, u32, u32);

/// What `ArtCache::raster_frame` is asked for.
struct FrameRequest<'a> {
    clip_id: &'a str,
    /// First timeline slot showing the same frame file; repeated frames share one texture.
    slot: usize,
    bounds: [f32; 4],
    symbol_scale: f32,
    animated: bool,
    filter: Option<ColourMatrix>,
}

/// Textures keyed by frame and raster scale, evicted least recently used past a byte budget.
///
/// The stage scale is quantised so a window drag does not re-rasterise on every pixel, and
/// animated clips are rasterised no larger than [`ANIMATED_SCALE_CAP`] (the GPU upscales them) so
/// a whole looping clip fits the budget.
pub struct ArtCache {
    /// The stage scale in `1 / SCALE_STEPS` units.
    stage_steps: u32,
    clip_indices: HashMap<String, u32>,
    entries: HashMap<FrameKey, Entry>,
    bytes: usize,
    budget: usize,
    clock: u64,
    rasterised: usize,
    failed: HashSet<(u32, u32)>,
}

impl Default for ArtCache {
    fn default() -> Self {
        Self {
            stage_steps: SCALE_STEPS as u32,
            clip_indices: HashMap::new(),
            entries: HashMap::new(),
            bytes: 0,
            budget: DEFAULT_BUDGET_BYTES,
            clock: 0,
            rasterised: 0,
            failed: HashSet::new(),
        }
    }
}

impl ArtCache {
    /// Window pixels per stage pixel. Frames keep their textures across small changes; a change
    /// of a whole quantisation step lets frames that failed at the old scale be tried again.
    pub fn set_scale(&mut self, scale: f32) {
        let steps = (scale * SCALE_STEPS).round().max(1.0) as u32;
        if steps != self.stage_steps {
            self.stage_steps = steps;
            self.failed.clear();
        }
    }

    pub fn texture_count(&self) -> usize {
        self.entries.len()
    }

    /// How many frames were rasterised since the cache was created.
    pub fn rasterised_frames(&self) -> usize {
        self.rasterised
    }

    /// Frame `frame` of a required clip; `None` when it could not be rasterised (logged once).
    pub fn fixed(
        &mut self,
        ctx: &egui::Context,
        library: &ArtLibrary,
        art: FixedArt,
        frame: usize,
    ) -> Option<ArtSprite> {
        self.sprite(ctx, library, library.fixed(art), frame, 1.0)
    }

    /// Like [`Self::fixed`] drawn at `symbol_scale`, rasterised at the final size so it stays sharp.
    pub fn fixed_scaled(
        &mut self,
        ctx: &egui::Context,
        library: &ArtLibrary,
        art: FixedArt,
        frame: usize,
        symbol_scale: f32,
    ) -> Option<ArtSprite> {
        self.sprite(ctx, library, library.fixed(art), frame, symbol_scale)
    }

    /// Frame `frame` of a data-keyed clip (perk, quest, marker, perk card); `None` when the frame
    /// could not be rasterised (logged once per frame).
    pub fn clip(
        &mut self,
        ctx: &egui::Context,
        library: &ArtLibrary,
        clip: &ArtClip,
        frame: usize,
    ) -> Option<ArtSprite> {
        self.sprite(ctx, library, clip, frame, 1.0)
    }

    pub fn clip_scaled(
        &mut self,
        ctx: &egui::Context,
        library: &ArtLibrary,
        clip: &ArtClip,
        frame: usize,
        symbol_scale: f32,
    ) -> Option<ArtSprite> {
        self.sprite(ctx, library, clip, frame, symbol_scale)
    }

    /// Like [`Self::clip_scaled`] with `filter` applied to the rasterised frame.
    pub fn clip_scaled_filtered(
        &mut self,
        ctx: &egui::Context,
        library: &ArtLibrary,
        clip: &ArtClip,
        frame: usize,
        symbol_scale: f32,
        filter: ColourMatrix,
    ) -> Option<ArtSprite> {
        self.sprite_filtered(ctx, library, clip, frame, symbol_scale, Some(filter))
    }

    fn sprite(
        &mut self,
        ctx: &egui::Context,
        library: &ArtLibrary,
        clip: &ArtClip,
        frame: usize,
        symbol_scale: f32,
    ) -> Option<ArtSprite> {
        self.sprite_filtered(ctx, library, clip, frame, symbol_scale, None)
    }

    fn sprite_filtered(
        &mut self,
        ctx: &egui::Context,
        library: &ArtLibrary,
        clip: &ArtClip,
        frame: usize,
        symbol_scale: f32,
        filter: Option<ColourMatrix>,
    ) -> Option<ArtSprite> {
        let index = frame % clip.frames.len();
        let file = &clip.frames[index];
        let request = FrameRequest {
            clip_id: &clip.id,
            slot: clip
                .frames
                .iter()
                .position(|other| other == file)
                .unwrap_or(index),
            bounds: clip.bounds,
            symbol_scale,
            animated: clip.frames.len() > 1,
            filter,
        };
        self.raster_frame(ctx, &request, || library.frame_path(clip, index))
    }

    /// Raster pixels per stage pixel; the symbol scale is already in the rasterised bounds.
    fn raster_scale(&self, animated: bool) -> f32 {
        let scale = self.stage_steps as f32 / SCALE_STEPS;
        if animated {
            scale.min(ANIMATED_SCALE_CAP)
        } else {
            scale
        }
    }

    fn clip_index(&mut self, clip_id: &str) -> u32 {
        if let Some(index) = self.clip_indices.get(clip_id) {
            return *index;
        }
        let index = self.clip_indices.len() as u32;
        self.clip_indices.insert(clip_id.to_owned(), index);
        index
    }

    /// The cached frame, or the frame read from `path()` and rasterised.
    fn raster_frame(
        &mut self,
        ctx: &egui::Context,
        request: &FrameRequest<'_>,
        path: impl FnOnce() -> PathBuf,
    ) -> Option<ArtSprite> {
        let raster_scale = self.raster_scale(request.animated);
        let clip = self.clip_index(request.clip_id);
        let slot = request.slot as u32;
        let filter_id = request.filter.as_ref().map_or(0, ColourMatrix::cache_id);
        let key = (
            clip,
            slot,
            raster_scale.to_bits(),
            request.symbol_scale.to_bits(),
            filter_id,
        );
        self.clock += 1;
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.last_used = self.clock;
            return Some(ArtSprite {
                texture: entry.texture.id(),
                rect: entry.rect,
            });
        }
        if self.failed.contains(&(clip, slot)) {
            return None;
        }
        let path = path();
        let raster = std::fs::read(&path)
            .map_err(|source| RasterError::Read {
                path: path.clone(),
                source,
            })
            .and_then(|svg| {
                rasterise(
                    &svg,
                    request.bounds.map(|edge| edge * request.symbol_scale),
                    raster_scale,
                )
            })
            .map(|mut raster| {
                if let Some(filter) = &request.filter {
                    filter.apply(&mut raster.image);
                }
                raster
            });
        match raster {
            Ok(raster) => {
                self.rasterised += 1;
                Some(self.insert(ctx, key, &path, raster))
            }
            Err(error) => {
                tracing::error!("Pip-Boy art clip '{}': {error}", request.clip_id);
                self.failed.insert((clip, slot));
                None
            }
        }
    }

    fn insert(
        &mut self,
        ctx: &egui::Context,
        key: FrameKey,
        path: &Path,
        raster: Raster,
    ) -> ArtSprite {
        let bytes = raster.image.pixels.len() * 4;
        let texture = ctx.load_texture(
            path.display().to_string(),
            raster.image,
            TextureOptions::LINEAR,
        );
        let sprite = ArtSprite {
            texture: texture.id(),
            rect: raster.rect,
        };
        self.bytes += bytes;
        self.entries.insert(
            key,
            Entry {
                texture,
                rect: raster.rect,
                bytes,
                last_used: self.clock,
            },
        );
        self.evict_to_budget();
        sprite
    }

    fn evict_to_budget(&mut self) {
        while self.bytes > self.budget && self.entries.len() > 1 {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| *key);
            let Some(entry) = oldest.and_then(|key| self.entries.remove(&key)) else {
                break;
            };
            self.bytes -= entry.bytes;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT_HALF_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="-10 -10 20 20"><rect x="-10" y="-10" width="10" height="20" fill="white"/></svg>"#;
    const BOUNDS: [f32; 4] = [-10.0, -10.0, 10.0, 10.0];

    fn pixel(raster: &Raster, x: usize, y: usize) -> egui::Color32 {
        raster.image.pixels[y * raster.image.width() + x]
    }

    #[test]
    fn rasterising_places_the_drawn_half_and_reports_the_registration_relative_rect() {
        let raster = rasterise(LEFT_HALF_SVG.as_bytes(), BOUNDS, 2.0).unwrap();

        assert_eq!(raster.image.size, [40, 40]);
        assert_eq!(pixel(&raster, 5, 20), egui::Color32::WHITE);
        assert_eq!(pixel(&raster, 35, 20), egui::Color32::TRANSPARENT);
        assert_eq!(
            raster.rect,
            Rect::from_min_size(Pos2::new(-10.0, -10.0), Vec2::new(20.0, 20.0))
        );
    }

    #[test]
    fn a_higher_scale_rasterises_more_pixels_for_the_same_stage_rect() {
        let raster = rasterise(LEFT_HALF_SVG.as_bytes(), BOUNDS, 3.0).unwrap();
        assert_eq!(raster.image.size, [60, 60]);
        assert_eq!(raster.rect.size(), Vec2::new(20.0, 20.0));
    }

    #[test]
    fn malformed_svg_and_degenerate_bounds_are_distinct_errors() {
        assert!(matches!(
            rasterise(b"not svg", BOUNDS, 1.0),
            Err(RasterError::Parse(_))
        ));
        assert!(matches!(
            rasterise(LEFT_HALF_SVG.as_bytes(), [0.0, 0.0, 0.0, 5.0], 1.0),
            Err(RasterError::BadSize { width: 0, .. })
        ));
        assert!(matches!(
            rasterise(LEFT_HALF_SVG.as_bytes(), [0.0, 0.0, 9000.0, 5.0], 1.0),
            Err(RasterError::BadSize { .. })
        ));
    }

    #[test]
    fn a_colour_matrix_mixes_unpremultiplied_channels_and_keeps_alpha() {
        let mut image = ColorImage::new(
            [2, 1],
            vec![
                egui::Color32::from_rgba_unmultiplied(200, 100, 0, 128),
                egui::Color32::TRANSPARENT,
            ],
        );
        let matrix = ColourMatrix {
            rows: [
                [0.0, 1.0, 0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0, 0.0, 0.1],
                [0.0, 0.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 1.0, 0.0],
            ],
        };

        matrix.apply(&mut image);

        assert_eq!(
            image.pixels[0].to_srgba_unmultiplied(),
            [100, 225, 0, 128],
            "green moves to red, red plus the offset to green"
        );
        assert_eq!(image.pixels[1], egui::Color32::TRANSPARENT);
    }

    #[test]
    fn different_colour_matrices_have_different_cache_ids() {
        let identity = ColourMatrix {
            rows: [
                [1.0, 0.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 1.0, 0.0],
            ],
        };
        let mut brighter = identity;
        brighter.rows[0][4] = 0.25;
        assert_ne!(identity.cache_id(), brighter.cache_id());
        assert_eq!(identity.cache_id(), identity.cache_id());
    }

    struct Fixture {
        dir: tempfile::TempDir,
        cache: ArtCache,
        ctx: egui::Context,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            Self {
                dir,
                cache: ArtCache::default(),
                ctx: egui::Context::default(),
            }
        }

        fn write(&self, name: &str, svg: &str) -> PathBuf {
            let path = self.dir.path().join(name);
            std::fs::write(&path, svg).unwrap();
            path
        }

        fn request(
            clip_id: &str,
            slot: usize,
            symbol_scale: f32,
            animated: bool,
        ) -> FrameRequest<'_> {
            FrameRequest {
                clip_id,
                slot,
                bounds: BOUNDS,
                symbol_scale,
                animated,
                filter: None,
            }
        }

        fn sprite(
            &mut self,
            id: &str,
            slot: usize,
            path: &Path,
            symbol_scale: f32,
        ) -> Option<ArtSprite> {
            self.cache.raster_frame(
                &self.ctx,
                &Self::request(id, slot, symbol_scale, false),
                || path.to_path_buf(),
            )
        }

        fn animated(&mut self, id: &str, slot: usize, path: &Path) -> Option<ArtSprite> {
            self.cache
                .raster_frame(&self.ctx, &Self::request(id, slot, 1.0, true), || {
                    path.to_path_buf()
                })
        }
    }

    #[test]
    fn a_frame_is_rasterised_once_per_scale_and_reused() {
        let mut fixture = Fixture::new();
        let path = fixture.write("a.svg", LEFT_HALF_SVG);
        fixture.cache.set_scale(2.0);

        let first = fixture.sprite("a", 0, &path, 1.0).unwrap();
        let again = fixture.sprite("a", 0, &path, 1.0).unwrap();
        assert_eq!(first, again);
        assert_eq!(fixture.cache.texture_count(), 1);

        let bigger = fixture.sprite("a", 0, &path, 1.5).unwrap();
        assert_ne!(bigger.texture, first.texture);
        assert_eq!(fixture.cache.texture_count(), 2);
    }

    #[test]
    fn a_window_drag_inside_one_quantisation_step_keeps_the_textures() {
        let mut fixture = Fixture::new();
        let path = fixture.write("a.svg", LEFT_HALF_SVG);
        fixture.cache.set_scale(2.0);
        let first = fixture.sprite("a", 0, &path, 1.0).unwrap();

        fixture.cache.set_scale(2.02);
        let dragged = fixture.sprite("a", 0, &path, 1.0).unwrap();

        assert_eq!(first, dragged);
        assert_eq!(fixture.cache.rasterised_frames(), 1);
    }

    #[test]
    fn a_scale_change_rasterises_afresh_without_dropping_the_old_textures_at_once() {
        let mut fixture = Fixture::new();
        let path = fixture.write("a.svg", LEFT_HALF_SVG);
        fixture.cache.set_scale(2.0);
        fixture.sprite("a", 0, &path, 1.0).unwrap();

        fixture.cache.set_scale(3.0);
        fixture.sprite("a", 0, &path, 1.0).unwrap();

        assert_eq!(fixture.cache.rasterised_frames(), 2);
        assert_eq!(fixture.cache.texture_count(), 2);
    }

    #[test]
    fn animated_clips_are_rasterised_no_larger_than_the_cap_and_icons_are_not() {
        let mut fixture = Fixture::new();
        let path = fixture.write("a.svg", LEFT_HALF_SVG);
        fixture.cache.set_scale(3.0);

        let animated = fixture.animated("clip", 0, &path).unwrap();
        assert_eq!(fixture.cache.bytes, 30 * 30 * 4);
        let icon = fixture.sprite("icon", 0, &path, 1.0).unwrap();
        assert_eq!(fixture.cache.bytes, 30 * 30 * 4 + 60 * 60 * 4);
        assert_eq!(animated.rect.size(), icon.rect.size());
    }

    #[test]
    fn a_looping_clip_that_fits_the_budget_is_rasterised_only_on_its_first_loop() {
        let mut fixture = Fixture::new();
        let paths: Vec<_> = (0..4)
            .map(|i| fixture.write(&format!("{i}.svg"), LEFT_HALF_SVG))
            .collect();
        fixture.cache.set_scale(1.0);

        for _loop in 0..3 {
            for (slot, path) in paths.iter().enumerate() {
                fixture.animated("loop", slot, path).unwrap();
            }
        }

        assert_eq!(fixture.cache.rasterised_frames(), 4);
    }

    #[test]
    fn past_the_budget_the_least_recently_used_frame_goes_first() {
        let mut fixture = Fixture::new();
        let paths: Vec<_> = (0..3)
            .map(|i| fixture.write(&format!("{i}.svg"), LEFT_HALF_SVG))
            .collect();
        fixture.cache.set_scale(1.0);
        fixture.cache.budget = 2 * 20 * 20 * 4;

        fixture.sprite("c", 0, &paths[0], 1.0).unwrap();
        fixture.sprite("c", 1, &paths[1], 1.0).unwrap();
        fixture.sprite("c", 0, &paths[0], 1.0).unwrap();
        fixture.sprite("c", 2, &paths[2], 1.0).unwrap();

        assert_eq!(fixture.cache.texture_count(), 2);
        assert_eq!(fixture.cache.rasterised_frames(), 3);
        fixture.sprite("c", 0, &paths[0], 1.0).unwrap();
        assert_eq!(
            fixture.cache.rasterised_frames(),
            3,
            "the recently used frame stayed cached"
        );
        fixture.sprite("c", 1, &paths[1], 1.0).unwrap();
        assert_eq!(
            fixture.cache.rasterised_frames(),
            4,
            "the least recently used frame was evicted"
        );
    }

    #[test]
    fn a_failed_frame_is_not_retried_nor_does_it_poison_the_other_frames_of_its_clip() {
        let mut fixture = Fixture::new();
        let broken = fixture.write("broken.svg", "<svg");
        let good = fixture.write("good.svg", LEFT_HALF_SVG);
        let missing = fixture.dir.path().join("missing.svg");

        assert!(fixture.sprite("clip", 0, &broken, 1.0).is_none());
        assert!(fixture.sprite("clip", 1, &missing, 1.0).is_none());
        assert!(fixture.sprite("clip", 2, &good, 1.0).is_some());
        std::fs::write(&broken, LEFT_HALF_SVG).unwrap();
        assert!(fixture.sprite("clip", 0, &broken, 1.0).is_none());
        assert_eq!(fixture.cache.texture_count(), 1);
    }

    #[test]
    fn a_scale_step_lets_failed_frames_be_tried_again() {
        let mut fixture = Fixture::new();
        let path = fixture.write("a.svg", "<svg");
        assert!(fixture.sprite("clip", 0, &path, 1.0).is_none());
        std::fs::write(&path, LEFT_HALF_SVG).unwrap();
        assert!(fixture.sprite("clip", 0, &path, 1.0).is_none());

        fixture.cache.set_scale(2.0);

        assert!(fixture.sprite("clip", 0, &path, 1.0).is_some());
    }
}
