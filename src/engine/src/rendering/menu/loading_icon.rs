//! The loading screen's animated Vault-Tec logo.
//!
//! The installer exports the logo sprite's frames as white-mask art in the sprite's own
//! coordinates; the loading screen's layout document places that sprite on the stage. The
//! loading screen's script then locks the logo's registration point to the bottom-right corner of
//! the screen's safe rectangle, which moves the sprite. The frame
//! shown follows the seconds since loading began, and is rasterised at the size it has on screen
//! into one texture that is drawn tinted with the HUD colour.

use std::path::PathBuf;

use bytemuck::{Pod, Zeroable};
use resvg::usvg;

use crate::assets::pipboy_art::{ArtClip, ArtLibrary, FixedArt};
use crate::rendering::game_settings::SafeZone;
use crate::rendering::hud::layout::Layout;
use crate::rendering::hud::quad::PxRect;
use crate::rendering::shaders;
use crate::rendering::svg_raster::{self, BadRasterSize};
use crate::ui_layout::values::{ColorTransform, Matrix};
use crate::ui_layout::{PlacementKey, StageKey, UiLayout, UiLayoutError};

pub const MOVIE: &str = "LoadingMenu";

/// The sprite inside the logo's instance: the one the exported frames are of.
/// The logo's own clip, whose registration point the loading screen's script moves.
pub static LOGO_ORIGIN: PlacementKey =
    PlacementKey::new(MOVIE, "FilterHolder_mc/Menu_mc/VaultTecLogo_mc");
pub static LOGO_PLACEMENT: PlacementKey =
    PlacementKey::new(MOVIE, "FilterHolder_mc/Menu_mc/VaultTecLogo_mc/@1");
pub static STAGE: StageKey = StageKey::of(MOVIE);

const TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Where the logo sprite sits on the loading screen's stage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IconPlacement {
    matrix: Matrix,
    /// The registration point of the logo's clip on the stage as authored.
    origin: (f32, f32),
}

impl IconPlacement {
    /// Reads the placement from the loading screen's layout. The loading screen must share the
    /// menu's `menu_stage`, so one stage fit serves both, and the logo must be placed by a
    /// plain, unmirrored scale and offset with no colour transform. The logo's clip must be
    /// placed by a plain offset, so that moving its registration point moves the art alone.
    pub fn resolve(layout: &UiLayout, menu_stage: (f32, f32)) -> Result<Self, UiLayoutError> {
        let [xmin, ymin, xmax, ymax] = layout.get(&STAGE)?.rect;
        if (xmin, ymin, xmax, ymax) != (0.0, 0.0, menu_stage.0, menu_stage.1) {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                "stage",
                format!(
                    "the stage is ({xmin}, {ymin}) to ({xmax}, {ymax}), not the menu's {} x {}",
                    menu_stage.0, menu_stage.1
                ),
            ));
        }
        let origin = layout.get(&LOGO_ORIGIN)?.world.matrix;
        if (origin.a, origin.b, origin.c, origin.d) != (1.0, 0.0, 0.0, 1.0) {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                LOGO_ORIGIN.path,
                "the logo's clip is scaled, rotated or skewed",
            ));
        }
        let world = layout.get(&LOGO_PLACEMENT)?.world;
        let unresolved = |reason| UiLayoutError::unresolved(MOVIE, LOGO_PLACEMENT.path, reason);
        if !world.matrix.is_axis_aligned() {
            return Err(unresolved("the logo is rotated or skewed"));
        }
        if world.matrix.a <= 0.0 || world.matrix.d <= 0.0 {
            return Err(unresolved("the logo is mirrored or collapsed"));
        }
        if world.color != ColorTransform::IDENTITY {
            return Err(unresolved("the logo is recoloured"));
        }
        Ok(Self {
            matrix: world.matrix,
            origin: (origin.tx, origin.ty),
        })
    }

    /// The stage offset the loading screen's script moves the logo by: it puts the clip's
    /// registration point on the bottom-right corner of the visible rectangle, inset by the
    /// safe zone.
    fn safe_rect_shift(&self, fit: &Layout, safe_zone: SafeZone) -> (f32, f32) {
        let [_, _, right, bottom] = fit.visible_stage_rect();
        (
            right - safe_zone.x - self.origin.0,
            bottom - safe_zone.y - self.origin.1,
        )
    }

    /// The framebuffer rectangle the art with symbol-local `bounds` covers, with the stage fitted
    /// by `fit` and the logo locked to the bottom-right corner of the safe zone.
    pub fn screen_rect(&self, bounds: [f32; 4], fit: &Layout, safe_zone: SafeZone) -> PxRect {
        let [xmin, ymin, xmax, ymax] = bounds;
        let (shift_x, shift_y) = self.safe_rect_shift(fit, safe_zone);
        let (left, top) = self.matrix.apply(xmin, ymin);
        let (right, bottom) = self.matrix.apply(xmax, ymax);
        let (left, right) = (left + shift_x, right + shift_x);
        let (top, bottom) = (top + shift_y, bottom + shift_y);
        let (x, y) = fit.stage_point(left, top);
        PxRect {
            x,
            y,
            w: fit.s(right - left),
            h: fit.s(bottom - top),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoadingIconError {
    #[error("the loading icon frame '{path}' cannot be read: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("the loading icon frame '{path}' is not valid SVG: {source}")]
    Parse { path: PathBuf, source: usvg::Error },
}

/// The uniform block of `menu_icon.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
struct IconParams {
    rect: [f32; 4],
    colour: [f32; 4],
}

/// The texture the current frame is rasterised into, and what it shows.
struct Surface {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    size: (u32, u32),
    shown_frame: Option<usize>,
}

/// `rect` moved and sized onto whole pixels, so the raster maps texel for pixel; `None` when it
/// covers no pixel.
pub fn snap_to_pixels(rect: PxRect) -> Option<PxRect> {
    let (width, height) = (rect.w.round(), rect.h.round());
    (width >= 1.0 && height >= 1.0).then(|| PxRect {
        x: rect.x.round(),
        y: rect.y.round(),
        w: width,
        h: height,
    })
}

/// What the logo looks like this frame.
#[derive(Debug, Clone, Copy)]
pub struct IconScene {
    /// The framebuffer size in pixels.
    pub target: (u32, u32),
    /// The framebuffer rectangle the art covers, before it is snapped to whole pixels.
    pub rect: PxRect,
    /// Seconds since the clip started playing.
    pub seconds: f64,
    /// The HUD colour as encoded red, green, blue and opacity bytes.
    pub tint: [u8; 4],
    /// The fade of the whole logo, 0 to 1.
    pub fade: f32,
}

pub struct LoadingIcon {
    clip: ArtClip,
    frames: Vec<usvg::Tree>,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    surface: Option<Surface>,
}

impl LoadingIcon {
    /// Reads and parses every frame of the logo clip, so a missing or broken frame stops the
    /// start instead of the loading screen.
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        art: &ArtLibrary,
    ) -> Result<Self, LoadingIconError> {
        let clip = art.fixed(FixedArt::LoadingIcon).clone();
        let frames = (0..clip.frames.len())
            .map(|index| {
                let path = art.frame_path(&clip, index);
                let svg = std::fs::read(&path).map_err(|source| LoadingIconError::Read {
                    path: path.clone(),
                    source,
                })?;
                usvg::Tree::from_data(&svg, &usvg::Options::default())
                    .map_err(|source| LoadingIconError::Parse { path, source })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("menu_icon_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("menu_icon_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shader = shaders::MENU_ICON.compile(device);
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Menu Icon Pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("menu_icon_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("menu_icon_params"),
            size: std::mem::size_of::<IconParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            clip,
            frames,
            pipeline,
            bind_group_layout,
            sampler,
            params,
            surface: None,
        })
    }

    /// The symbol-local bounds of the art.
    pub fn bounds(&self) -> [f32; 4] {
        self.clip.bounds
    }

    /// The 0-based frame shown `seconds` after the clip started.
    pub fn frame_at(&self, seconds: f64) -> usize {
        self.clip.frame_at(seconds, None).min(self.frames.len() - 1)
    }

    /// Rasterises the frame `scene` shows at its on-screen size, snapped to whole pixels, when
    /// that is not what the texture holds, and writes the quad's parameters. False when the logo
    /// covers no pixel; an error when it covers more than art can be rasterised at.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &IconScene,
    ) -> Result<bool, BadRasterSize> {
        let Some(rect) = snap_to_pixels(scene.rect) else {
            return Ok(false);
        };
        let size = (rect.w as u32, rect.h as u32);
        svg_raster::check_size(size.0, size.1)?;
        if self.surface.as_ref().is_none_or(|s| s.size != size) {
            self.surface = Some(self.create_surface(device, size));
        }
        let frame = self.frame_at(scene.seconds);
        let Some(surface) = self.surface.as_mut() else {
            return Ok(false);
        };
        if surface.shown_frame != Some(frame) {
            let pixmap = svg_raster::render(&self.frames[frame], size.0, size.1)?;
            queue.write_texture(
                surface.texture.as_image_copy(),
                pixmap.data(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(size.0 * 4),
                    rows_per_image: Some(size.1),
                },
                wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
            );
            surface.shown_frame = Some(frame);
        }
        let [red, green, blue, opacity] = scene.tint.map(|byte| f32::from(byte) / 255.0);
        let params = IconParams {
            rect: rect.clip_edges(scene.target),
            colour: [red, green, blue, opacity * scene.fade.clamp(0.0, 1.0)],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        Ok(true)
    }

    /// Draws the logo; call only after [`Self::prepare`] returned true.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(surface) = &self.surface else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &surface.bind_group, &[]);
        pass.draw(0..4, 0..1);
    }

    fn create_surface(&self, device: &wgpu::Device, size: (u32, u32)) -> Surface {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("menu_icon_texture"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TEXTURE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("menu_icon_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        Surface {
            texture,
            bind_group,
            size,
            shown_frame: None,
        }
    }
}

/// A synthetic loading screen and logo art, for the tests of this and sibling modules.
#[cfg(test)]
pub(super) mod test_support {
    use std::path::Path;

    use serde_json::{Value, json};

    use crate::assets::layout::{INTERFACE_DIR, PIPBOY_ART_MANIFEST_FILE, PIPBOY_DIR};
    use crate::assets::pipboy_art::{ArtLibrary, FixedArt, MANIFEST_FORMAT};

    pub const FRAMES_PER_SECOND: f32 = 2.0;
    /// The logo's symbol-local bounds in the fixture art.
    pub const BOUNDS: [f32; 4] = [0.0, 0.0, 40.0, 40.0];
    /// Where the fixture layout puts the logo's sprite: scale 0.5 at stage (600, 300).
    pub const PLACEMENT: [f64; 6] = [0.5, 0.0, 0.0, 0.5, 600.0, 300.0];
    /// Where the fixture layout puts the logo's clip: the corner of the art, as authored.
    pub const ORIGIN: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 620.0, 320.0];

    /// The loading screen movie parts: the logo sprite placed at [`PLACEMENT`].
    pub fn parts() -> Value {
        let mut placed = crate::ui_layout::test_support::instance(14, "sprite", 0.0, 0.0);
        placed["world"]["matrix"] = json!(PLACEMENT);
        placed["local"]["matrix"] = json!(PLACEMENT);
        let mut clip = crate::ui_layout::test_support::instance(15, "sprite", 0.0, 0.0);
        clip["world"]["matrix"] = json!(ORIGIN);
        clip["local"]["matrix"] = json!(ORIGIN);
        json!({
            "stage": [0, 0, 1280, 720],
            "instances": {
                "FilterHolder_mc/Menu_mc/VaultTecLogo_mc": clip,
                "FilterHolder_mc/Menu_mc/VaultTecLogo_mc/@1": placed,
            },
        })
    }

    pub fn svg(left: u32) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40" viewBox="0 0 40 40"><rect x="{left}" width="20" height="40" fill="white"/></svg>"#
        )
    }

    /// An art library whose logo clip shows `frames` (SVG texts) in turn, [`FRAMES_PER_SECOND`]
    /// frames a second, and whose every other required clip shares one frame.
    pub fn library(dir: &Path, frames: &[String]) -> ArtLibrary {
        library_at(dir, frames, FRAMES_PER_SECOND)
    }

    /// Like [`library`] with the logo clip playing at `fps`.
    pub fn library_at(dir: &Path, frames: &[String], fps: f32) -> ArtLibrary {
        let root = dir.join(INTERFACE_DIR).join(PIPBOY_DIR);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("shared.svg"), svg(0)).unwrap();
        let logo_files: Vec<String> = (0..frames.len()).map(|i| format!("logo_{i}.svg")).collect();
        for (file, svg) in logo_files.iter().zip(frames) {
            std::fs::write(root.join(file), svg).unwrap();
        }
        let clips: Vec<Value> = FixedArt::all()
            .map(|art| {
                let (fps, files) = if art == FixedArt::LoadingIcon {
                    (fps, logo_files.clone())
                } else {
                    (30.0, vec!["shared.svg".to_owned()])
                };
                json!({
                    "id": art.clip_id(), "source": "Test.swf", "colour": "mask", "fps": fps,
                    "script": [], "bounds": BOUNDS, "frames": files,
                })
            })
            .collect();
        let manifest = json!({ "format": MANIFEST_FORMAT, "clips": clips });
        std::fs::write(root.join(PIPBOY_ART_MANIFEST_FILE), manifest.to_string()).unwrap();
        ArtLibrary::load(dir).unwrap()
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the placement arithmetic below stays on exactly representable values"
)]
mod tests {
    use serde_json::json;

    use super::test_support::{BOUNDS, ORIGIN, library, library_at, parts, svg};
    use super::*;
    use crate::ui_layout::test_support::movie_json;

    fn layout_of(loading_menu: &serde_json::Value) -> UiLayout {
        UiLayout::from_documents(
            &crate::ui_layout::test_support::translate_json(),
            &crate::ui_layout::test_support::font_config_json(),
            &[(MOVIE, &movie_json(MOVIE, loading_menu))],
        )
        .unwrap()
    }

    const SAFE: SafeZone = SafeZone { x: 64.0, y: 36.0 };

    fn placement() -> IconPlacement {
        IconPlacement::resolve(&layout_of(&parts()), (1280.0, 720.0)).unwrap()
    }

    #[test]
    fn the_logos_registration_point_is_locked_to_the_safe_zones_bottom_right_corner() {
        let fit = Layout::for_stage(1280.0, 720.0, 1280.0, 720.0);
        let rect = placement().screen_rect(BOUNDS, &fit, SAFE);
        assert_eq!(
            (rect.x + rect.w, rect.y + rect.h),
            (1280.0 - SAFE.x, 720.0 - SAFE.y)
        );
        assert_eq!(
            (rect.x, rect.y, rect.w, rect.h),
            (1196.0, 664.0, 20.0, 20.0)
        );
    }

    #[test]
    fn the_lock_moves_the_art_by_the_registration_points_shift_and_not_the_authored_spot() {
        let fit = Layout::for_stage(1280.0, 720.0, 2560.0, 1440.0);
        let rect = placement().screen_rect(BOUNDS, &fit, SAFE);
        let authored_right = ORIGIN[4] as f32 * 2.0;
        assert_eq!(
            rect.x + rect.w - authored_right,
            2.0 * (1280.0 - 64.0 - 620.0)
        );
        assert_eq!(rect.w, 40.0);
    }

    #[test]
    fn a_wider_screen_pins_the_logo_to_the_screens_edge_and_keeps_the_scale() {
        let fit = Layout::for_stage(1280.0, 720.0, 2560.0, 1440.0);
        let wide = Layout::for_stage(1280.0, 720.0, 3000.0, 1440.0);
        let normal = placement().screen_rect(BOUNDS, &fit, SAFE);
        let letterboxed = placement().screen_rect(BOUNDS, &wide, SAFE);
        assert_eq!((letterboxed.w, letterboxed.h), (normal.w, normal.h));
        assert_eq!(letterboxed.x - normal.x, 440.0);
        assert_eq!(letterboxed.y, normal.y);
        assert_eq!(3000.0 - (letterboxed.x + letterboxed.w), 2.0 * SAFE.x);
    }

    #[test]
    fn a_taller_screen_pins_the_logo_to_the_screens_bottom() {
        let fit = Layout::for_stage(1280.0, 720.0, 1280.0, 1000.0);
        let rect = placement().screen_rect(BOUNDS, &fit, SAFE);
        assert_eq!(1000.0 - (rect.y + rect.h), SAFE.y);
    }

    #[test]
    fn art_bounds_not_at_the_origin_move_the_rectangle_by_the_scaled_offset() {
        let fit = Layout::for_stage(1280.0, 720.0, 1280.0, 720.0);
        let rect = placement().screen_rect([-20.0, -10.0, 20.0, 30.0], &fit, SAFE);
        assert_eq!(
            (rect.x, rect.y, rect.w, rect.h),
            (1186.0, 659.0, 20.0, 20.0)
        );
    }

    #[test]
    fn a_scaled_logo_clip_is_rejected() {
        let mut movie = parts();
        movie["instances"]["FilterHolder_mc/Menu_mc/VaultTecLogo_mc"]["world"]["matrix"] =
            json!([2.0, 0, 0, 2.0, 620, 320]);
        let error = IconPlacement::resolve(&layout_of(&movie), (1280.0, 720.0)).unwrap_err();
        assert!(error.to_string().contains("scaled"), "{error}");
    }

    #[test]
    fn a_loading_screen_missing_the_logo_names_the_key() {
        let layout = layout_of(&json!({}));
        let error = IconPlacement::resolve(&layout, (1280.0, 720.0)).unwrap_err();
        assert!(error.to_string().contains("VaultTecLogo_mc"), "{error}");
    }

    #[test]
    fn a_stage_other_than_the_menus_is_rejected() {
        let error = IconPlacement::resolve(&layout_of(&parts()), (1920.0, 1080.0)).unwrap_err();
        assert!(error.to_string().contains("stage"), "{error}");
    }

    #[test]
    fn a_rotated_mirrored_or_recoloured_logo_is_rejected() {
        let cases = [
            ("matrix", json!([0.5, 0.2, 0.0, 0.5, 600, 300]), "rotated"),
            ("matrix", json!([-0.5, 0, 0, 0.5, 600, 300]), "mirrored"),
            (
                "color",
                json!({"mult": [1, 1, 1, 0.5], "add": [0, 0, 0, 0]}),
                "recoloured",
            ),
        ];
        for (field, value, reason) in cases {
            let mut movie = parts();
            movie["instances"]["FilterHolder_mc/Menu_mc/VaultTecLogo_mc/@1"]["world"][field] =
                value;
            let error = IconPlacement::resolve(&layout_of(&movie), (1280.0, 720.0)).unwrap_err();
            assert!(error.to_string().contains(reason), "{error}");
        }
    }

    #[test]
    fn a_missing_frame_file_stops_the_load_with_its_path() {
        let Some((device, _queue)) = crate::rendering::gpu_test::test_device("loading icon") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let art = library(dir.path(), &[svg(0), svg(20)]);
        std::fs::remove_file(
            dir.path()
                .join("interface")
                .join("pipboy")
                .join("logo_1.svg"),
        )
        .unwrap();
        let error = LoadingIcon::new(&device, wgpu::TextureFormat::Rgba8Unorm, &art)
            .err()
            .expect("the frame is gone");
        assert!(matches!(error, LoadingIconError::Read { .. }), "{error}");
        assert!(error.to_string().contains("logo_1.svg"), "{error}");
    }

    #[test]
    fn a_frame_that_is_not_svg_stops_the_load_as_a_parse_error() {
        let Some((device, _queue)) = crate::rendering::gpu_test::test_device("loading icon") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let art = library(dir.path(), &[svg(0), "not svg".to_owned()]);
        let error = LoadingIcon::new(&device, wgpu::TextureFormat::Rgba8Unorm, &art)
            .err()
            .expect("the frame is broken");
        assert!(matches!(error, LoadingIconError::Parse { .. }), "{error}");
    }

    #[test]
    fn a_fractional_placement_snaps_to_whole_pixels_of_the_raster_size() {
        let fit = Layout::for_stage(1280.0, 720.0, 1000.0, 562.5);
        let rect = placement().screen_rect(BOUNDS, &fit, SAFE);
        assert_eq!(
            (rect.x, rect.y, rect.w, rect.h),
            (934.375, 518.75, 15.625, 15.625)
        );
        let snapped = snap_to_pixels(rect).unwrap();
        assert_eq!(
            (snapped.x, snapped.y, snapped.w, snapped.h),
            (934.0, 519.0, 16.0, 16.0)
        );
    }

    #[test]
    fn a_rectangle_under_half_a_pixel_covers_no_pixel() {
        let rect = PxRect {
            x: 3.0,
            y: 3.0,
            w: 0.4,
            h: 20.0,
        };
        assert!(snap_to_pixels(rect).is_none());
    }

    #[test]
    fn an_oversized_logo_is_an_error_and_not_shrunk() {
        let Some((device, queue)) = crate::rendering::gpu_test::test_device("loading icon") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let art = library(dir.path(), &[svg(0)]);
        let mut icon = LoadingIcon::new(&device, wgpu::TextureFormat::Rgba8Unorm, &art).unwrap();
        let scene = |edge: f32| IconScene {
            target: (100, 100),
            rect: PxRect {
                x: 0.0,
                y: 0.0,
                w: edge,
                h: 10.0,
            },
            seconds: 0.0,
            tint: [255; 4],
            fade: 1.0,
        };
        assert_eq!(icon.prepare(&device, &queue, &scene(8192.0)), Ok(true));
        assert_eq!(
            icon.prepare(&device, &queue, &scene(8193.0)),
            Err(BadRasterSize {
                width: 8193,
                height: 10
            })
        );
        assert_eq!(icon.prepare(&device, &queue, &scene(0.2)), Ok(false));
    }

    #[test]
    fn the_frame_follows_the_real_frame_rate_through_a_whole_loop_and_wraps() {
        let Some((device, _queue)) = crate::rendering::gpu_test::test_device("loading icon") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let frames: Vec<String> = (0..57).map(|_| svg(0)).collect();
        let art = library_at(dir.path(), &frames, 30.0);
        let icon = LoadingIcon::new(&device, wgpu::TextureFormat::Rgba8Unorm, &art).unwrap();
        let at = |frame: f64| icon.frame_at((frame + 0.5) / 30.0);
        assert_eq!(
            [0.0, 1.0, 2.0, 28.0, 55.0, 56.0].map(at),
            [0, 1, 2, 28, 55, 56]
        );
        assert_eq!([57.0, 58.0, 113.0, 114.0].map(at), [0, 1, 56, 0]);
        assert_eq!(icon.frame_at(10_000.0 / 30.0 + 0.001), 10_000 % 57);
    }

    #[test]
    fn the_frame_follows_the_seconds_since_loading_began_and_loops() {
        let Some((device, _queue)) = crate::rendering::gpu_test::test_device("loading icon") else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let art = library(dir.path(), &[svg(0), svg(20), svg(0)]);
        let icon = LoadingIcon::new(&device, wgpu::TextureFormat::Rgba8Unorm, &art).unwrap();
        let frames: Vec<usize> = [0.0, 0.49, 0.5, 1.0, 1.49, 1.5, 3.0]
            .map(|seconds| icon.frame_at(seconds))
            .into();
        assert_eq!(frames, [0, 0, 1, 2, 2, 0, 0]);
    }
}
