//! The moon and the stars of the exterior sky, drawn right after the sky gradient.
//!
//! Both layers sit at reverse-Z depth 0 like the gradient, so they show only where no geometry
//! wrote depth. They bind their own group at [`layer::LAYER_GROUP`], which the world pass uses
//! for the texture feedback buffer, so [`NightSky::draw`] takes that bind group and binds it
//! again when it is done.

mod assets;
mod layer;
mod moon;
mod stars;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use glam::Vec3;

pub use assets::{AssetFailure, AssetFailureKind, AssetState, SkyAssetError, SkyAssetStatus};
pub use moon::MoonSizes;

use assets::{load_star_geometry, load_texture, star_texture_key};
use moon::{MoonLayer, MoonUniform};
use stars::{StarLayer, StarUniform};

use crate::rendering::lighting::time_of_day::unmeasured::moon_angular_diameter_deg;
use crate::rendering::lighting::time_of_day::{MoonKind, MoonPhase, SkyObjects, StarRotation};
use crate::rendering::render_pipelines::world_pass_layout::FEEDBACK_GROUP;

const _: () = assert!(layer::LAYER_GROUP == FEEDBACK_GROUP);

/// The moon of the frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoonInput {
    pub kind: MoonKind,
    pub phase: MoonPhase,
    /// Unit vector in engine space, towards the moon.
    pub direction: Vec3,
    /// 0 to 1: how far the moon has faded in above the horizon.
    pub visibility: f32,
}

/// What the sky shows this frame, as plain data. It mirrors [`SkyObjects`] without the sun
/// disc, which the sky gradient draws; the renderer stays testable without the lighting state.
#[derive(Debug, Clone, PartialEq)]
pub struct NightSkyInput {
    /// `None` where the climate shows no moon: nothing is drawn or loaded for it.
    pub moon: Option<MoonInput>,
    pub star_rotation: StarRotation,
    /// 0 hides the stars, 1 shows them fully.
    pub star_alpha: f32,
    /// The climate sky model (`sky\stars.nif`); `None` draws no stars.
    pub sky_model: Option<Arc<str>>,
}

impl From<&SkyObjects> for NightSkyInput {
    fn from(objects: &SkyObjects) -> Self {
        Self {
            moon: objects.moon.map(|moon| MoonInput {
                kind: moon.kind,
                phase: moon.phase,
                direction: moon.direction,
                visibility: moon.visibility,
            }),
            star_rotation: objects.star_rotation,
            star_alpha: objects.star_alpha,
            sky_model: objects.sky_model_path.clone(),
        }
    }
}

/// The sky model the star dome was last loaded for.
enum InstalledModel {
    NothingYet,
    /// `None` is a climate without a sky model.
    Requested(Option<Arc<str>>),
}

impl InstalledModel {
    fn is(&self, model: Option<&str>) -> bool {
        matches!(self, Self::Requested(installed) if installed.as_deref() == model)
    }
}

pub struct NightSky {
    asset_root: String,
    moon_sizes: MoonSizes,
    moon: MoonLayer,
    stars: StarLayer,
    installed_model: InstalledModel,
    status: SkyAssetStatus,
}

impl NightSky {
    /// `frame_layout` is the layout of the frame group (group 1) the world pass binds.
    /// Every asset the first frame needs loads here: the star texture, the moon textures of
    /// `initial_moon` and the dome of `initial_sky_model`. After that only a change of moon
    /// kind or sky model in [`Self::update`] loads again.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        asset_root: &str,
        frame_layout: &wgpu::BindGroupLayout,
        moon_sizes: MoonSizes,
        initial_moon: Option<MoonKind>,
        initial_sky_model: Option<&Arc<str>>,
    ) -> Self {
        let mut stars = StarLayer::new(device, frame_layout);
        let star_texture = load_texture(device, queue, asset_root, &star_texture_key());
        let status = SkyAssetStatus {
            star_texture: stars.install_texture(device, star_texture),
            ..SkyAssetStatus::default()
        };
        let mut sky = Self {
            asset_root: asset_root.to_owned(),
            moon_sizes,
            moon: MoonLayer::new(device, frame_layout),
            stars,
            installed_model: InstalledModel::NothingYet,
            status,
        };
        sky.load_changed_assets(device, queue, initial_moon, initial_sky_model);
        sky
    }

    /// Positions the moon and the stars for this frame. `radiance_scale` is the sky radiance
    /// scale the sky colours are multiplied by.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        input: &NightSkyInput,
        radiance_scale: f32,
    ) {
        self.load_changed_assets(
            device,
            queue,
            input.moon.map(|moon| moon.kind),
            input.sky_model.as_ref(),
        );
        match input.moon {
            Some(moon) => {
                let diameter_deg = moon_angular_diameter_deg(self.moon_sizes.of(moon.kind));
                let uniform = MoonUniform::new(
                    moon.direction,
                    diameter_deg,
                    moon.visibility,
                    radiance_scale,
                );
                self.moon.update(queue, moon.phase, uniform);
            }
            None => self.moon.hide(),
        }
        let stars = StarUniform::new(input.star_rotation, input.star_alpha, radiance_scale);
        self.stars.update(queue, &stars);
    }

    fn load_changed_assets(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        moon: Option<MoonKind>,
        sky_model: Option<&Arc<str>>,
    ) {
        if let Some(kind) = moon.filter(|&kind| self.moon.kind() != Some(kind)) {
            self.status.moon = self.moon.load_kind(device, queue, &self.asset_root, kind);
        }
        if !self.installed_model.is(sky_model.map(|model| &**model)) {
            let geometry = sky_model.map(|path| load_star_geometry(&self.asset_root, path));
            self.status.star_mesh = self.stars.install_mesh(device, geometry);
            self.installed_model = InstalledModel::Requested(sky_model.cloned());
        }
    }

    /// Draws the stars, then the moon over them. The layers take the world pass feedback group
    /// slot, so `feedback` is bound there again afterwards and the pass is as it was.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, feedback: &wgpu::BindGroup) {
        self.stars.draw(pass);
        self.moon.draw(pass);
        pass.set_bind_group(FEEDBACK_GROUP, feedback, &[]);
    }

    pub fn asset_status(&self) -> &SkyAssetStatus {
        &self.status
    }
}
