//! The large loading screen's picture: the stage and the model drawn into a linear image with
//! the camera the stage frames, then graded onto the menu's target.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::LoadingArtRenderer;
use super::grade::{GradeLook, LoadScreenLut, load_screen_cinematic};
use super::grade_pass::GradePass;
use super::stage::{CpuStage, StageError, keep_usable, stage_paths};
use super::stage_renderer::{LoadedStage, StageRenderer};
use crate::rendering::lighting::tonemap_settings::ImageSpaceCinematic;
use crate::rendering::post::tonemap::lut_library::load_table;

/// The format of the linear picture the stage and the model are drawn into.
pub const PICTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Where the grade's data comes from.
#[derive(Debug, Clone)]
pub struct GradeSource {
    pub lut: LoadScreenLut,
    pub db_path: String,
}

enum StageState {
    NotLoaded,
    Ready(LoadedStage),
    /// Reported when it failed; not retried.
    Unavailable(String),
}

pub struct LoadingScene {
    device: wgpu::Device,
    queue: wgpu::Queue,
    asset_root: PathBuf,
    grade_source: GradeSource,
    stage_renderer: StageRenderer,
    stage: StageState,
    grade: GradePass,
    picture: Option<Picture>,
}

impl LoadingScene {
    /// A scene graded onto targets of `target_format`; nothing is read before [`Self::prepare`].
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target_format: wgpu::TextureFormat,
        asset_root: &Path,
        grade_source: GradeSource,
    ) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            asset_root: asset_root.to_path_buf(),
            grade_source,
            stage_renderer: StageRenderer::new(device, PICTURE_FORMAT),
            stage: StageState::NotLoaded,
            grade: GradePass::new(device, queue, target_format),
            picture: None,
        }
    }

    /// Loads the stage and the grade on the first call. Fails, now and on every later call,
    /// when no width of the stage is usable: without it there is no camera to frame the model.
    /// A grade that cannot be read is logged and left neutral.
    pub fn prepare(&mut self) -> Result<(), StageError> {
        if matches!(self.stage, StageState::NotLoaded) {
            self.stage = match self.load_stage() {
                Ok(stage) => StageState::Ready(stage),
                Err(error) => StageState::Unavailable(error.to_string()),
            };
            if matches!(self.stage, StageState::Ready(_)) {
                self.load_grade();
            }
        }
        match &self.stage {
            StageState::Ready(_) => Ok(()),
            StageState::Unavailable(reason) => Err(StageError::NoneUsable(reason.clone())),
            StageState::NotLoaded => unreachable!("the stage was loaded above"),
        }
    }

    fn load_stage(&mut self) -> Result<LoadedStage, StageError> {
        let stages = load_stages(&self.asset_root)?;
        self.stage_renderer
            .upload(&self.device, &self.queue, &stages, &self.asset_root)
    }

    fn load_grade(&mut self) {
        let cinematic = load_screen_cinematic(&self.grade_source.db_path).unwrap_or_else(|error| {
            tracing::error!(
                "the loading screens' image space is unavailable ({error}); they are graded without its cinematic stage"
            );
            ImageSpaceCinematic::NEUTRAL
        });
        self.grade.set_look(GradeLook { cinematic });
        let key: Arc<str> = self.grade_source.lut.key.as_str().into();
        match load_table(&self.asset_root.to_string_lossy(), &key) {
            Ok(table) => self.grade.set_lut(&self.queue, &table),
            Err(error) => {
                tracing::error!("{error}; loading screens are graded without a colour LUT");
            }
        }
    }

    /// Draws the stage and `art`'s model into the linear picture and grades it over what
    /// `target`, of `size` pixels, holds at `fade`. Nothing is drawn before the stage is
    /// prepared or for an empty target.
    pub fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        art: &mut LoadingArtRenderer,
        target: &wgpu::TextureView,
        size: (u32, u32),
        fade: f32,
    ) {
        let StageState::Ready(stage) = &self.stage else {
            return;
        };
        if size.0 == 0 || size.1 == 0 {
            return;
        }
        let aspect = size.0 as f32 / size.1 as f32;
        let Some(framing) = stage.framing_for(aspect) else {
            return;
        };
        let picture = Picture::view_for(&mut self.picture, &self.device, size);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Loading Stage Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: picture,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            self.stage_renderer
                .draw(&self.queue, &mut pass, stage, aspect);
        }
        art.render(encoder, picture, size, framing.vertical_fov_radians);
        self.grade
            .draw(&self.device, &self.queue, encoder, picture, target, fade);
    }
}

/// Every width of the stage that loads; the ones that do not are logged. An error when none do.
fn load_stages(asset_root: &Path) -> Result<Vec<CpuStage>, StageError> {
    keep_usable(
        stage_paths(asset_root)
            .iter()
            .map(|path| CpuStage::load(path)),
    )
}

/// The linear picture, sized like the target it is graded onto.
struct Picture {
    size: (u32, u32),
    view: wgpu::TextureView,
}

impl Picture {
    fn view_for<'a>(
        slot: &'a mut Option<Self>,
        device: &wgpu::Device,
        size: (u32, u32),
    ) -> &'a wgpu::TextureView {
        if slot.as_ref().is_some_and(|picture| picture.size != size) {
            *slot = None;
        }
        &slot
            .get_or_insert_with(|| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("loading_picture"),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: PICTURE_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                Self {
                    size,
                    view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
                }
            })
            .view
    }
}
