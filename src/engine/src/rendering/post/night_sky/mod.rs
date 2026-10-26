//! The moon, the stars, the sun and the clouds of the exterior sky, drawn right after the sky
//! gradient.
//!
//! All layers sit at reverse-Z depth 0 like the gradient, so they show only where no geometry
//! wrote depth. They bind their own group at [`layer::LAYER_GROUP`], which the world pass uses
//! for the texture feedback buffer, so [`NightSky::draw`] takes that bind group and binds it
//! again when it is done. Below that group their pipelines share the world pass's layouts, as every
//! world pass pipeline must: after a switch from a pipeline whose lower groups differ, the next
//! draw produces nothing on Vulkan, without any validation error.

mod assets;
mod cloud_geometry;
mod cloud_scroll;
mod clouds;
#[cfg(test)]
mod clouds_tests;
mod layer;
mod moon;
mod quad;
mod stars;
mod sun;
#[cfg(test)]
mod sun_tests;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use glam::Vec3;

pub use assets::{AssetFailure, AssetFailureKind, AssetState, SkyAssetError, SkyAssetStatus};
pub use clouds::{CloudLayerDraw, CloudsInput};
use layer::WorldGroupLayouts;
pub use moon::MoonSizes;
pub use sun::{SunInput, SunSizes};

use assets::{load_star_geometry, load_texture, star_texture_key};
use cloud_geometry::{DEFAULT_MODEL, load_cloud_geometry};
use clouds::CloudLayer;
use moon::{MoonLayer, MoonUniform};
use stars::{StarLayer, StarUniform};
use sun::{SunLayer, SunUniforms};

use crate::assets::layout::named_texture_key;
use crate::rendering::lighting::time_of_day::unmeasured::moon_angular_diameter_deg;
use crate::rendering::lighting::time_of_day::{
    ClimateSunTextures, MoonKind, MoonPhase, SkyObjects, StarRotation,
};
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

/// The sizes of the sky objects the game settings give.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkySizes {
    pub moon: MoonSizes,
    pub sun: SunSizes,
}

impl SkySizes {
    #[cfg(test)]
    pub const VANILLA: Self = Self {
        moon: MoonSizes::VANILLA,
        sun: SunSizes::VANILLA,
    };
}

/// What the sky shows this frame, as plain data: [`SkyObjects`] and the sun's look; the
/// renderer stays testable without the lighting state.
#[derive(Debug, Clone, PartialEq)]
pub struct NightSkyInput {
    /// `None` where the climate shows no moon: nothing is drawn or loaded for it.
    pub moon: Option<MoonInput>,
    pub star_rotation: StarRotation,
    /// 0 hides the stars, 1 shows them fully.
    pub star_alpha: f32,
    /// The climate sky model (`sky\stars.nif`); `None` draws no stars.
    pub sky_model: Option<Arc<str>>,
    pub sun: SunInput,
    /// The climate's sun textures; one it names none of is not drawn or loaded.
    pub sun_textures: ClimateSunTextures,
}

impl NightSkyInput {
    /// `sun` is where the sun stands and how it looks; `objects` holds the rest.
    pub fn new(objects: &SkyObjects, sun: SunInput) -> Self {
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
            sun,
            sun_textures: objects.sun_textures.clone(),
        }
    }
}

/// What a layer last loaded its assets for.
enum Installed<T> {
    NothingYet,
    Requested(T),
}

impl<T: PartialEq> Installed<T> {
    fn is(&self, wanted: &T) -> bool {
        matches!(self, Self::Requested(installed) if installed == wanted)
    }
}

pub struct NightSky {
    asset_root: String,
    sizes: SkySizes,
    moon: MoonLayer,
    stars: StarLayer,
    sun: SunLayer,
    clouds: CloudLayer,
    /// `None` is a climate without a sky model.
    installed_model: Installed<Option<Arc<str>>>,
    installed_sun_textures: Installed<ClimateSunTextures>,
    /// Whether the cloud model was requested, which the first layer that shows does.
    cloud_model_requested: bool,
    status: SkyAssetStatus,
}

impl NightSky {
    /// `world_groups` are the layouts of the groups the world pass binds below the layer group.
    /// Every asset the first frame needs loads here: the star texture, the moon textures of
    /// `initial_moon` and the dome of `initial_sky_model`. The sun textures load with the first
    /// [`Self::update`]. After that only a change of moon kind, sky model or sun textures in
    /// [`Self::update`] loads again.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        asset_root: &str,
        world_groups: WorldGroupLayouts<'_>,
        sizes: SkySizes,
        initial_moon: Option<MoonKind>,
        initial_sky_model: Option<&Arc<str>>,
    ) -> Self {
        let mut stars = StarLayer::new(device, world_groups);
        let star_texture = load_texture(device, queue, asset_root, &star_texture_key());
        let status = SkyAssetStatus {
            star_texture: stars.install_texture(device, star_texture),
            ..SkyAssetStatus::default()
        };
        let mut sky = Self {
            asset_root: asset_root.to_owned(),
            sizes,
            moon: MoonLayer::new(device, world_groups),
            stars,
            sun: SunLayer::new(device, world_groups),
            clouds: CloudLayer::new(device, world_groups),
            installed_model: Installed::NothingYet,
            installed_sun_textures: Installed::NothingYet,
            cloud_model_requested: false,
            status,
        };
        sky.load_changed_assets(device, queue, initial_moon, initial_sky_model);
        sky
    }

    /// Positions the moon, the stars and the sun for this frame. `radiance_scale` is the sky radiance
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
        self.load_changed_sun_textures(device, queue, &input.sun_textures);
        match input.moon {
            Some(moon) => {
                let diameter_deg = moon_angular_diameter_deg(self.sizes.moon.of(moon.kind));
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
        self.sun.update(
            queue,
            &SunUniforms::new(&input.sun, self.sizes.sun, radiance_scale),
        );
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
        if !self.installed_model.is(&sky_model.cloned()) {
            let geometry = sky_model.map(|path| load_star_geometry(&self.asset_root, path));
            self.status.star_mesh = self.stars.install_mesh(device, geometry);
            self.installed_model = Installed::Requested(sky_model.cloned());
        }
    }

    fn load_changed_sun_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        textures: &ClimateSunTextures,
    ) {
        if self.installed_sun_textures.is(textures) {
            return;
        }
        let load = |path: &Option<Arc<str>>| {
            path.as_deref()
                .map(|path| load_texture(device, queue, &self.asset_root, &named_texture_key(path)))
        };
        let (disc, glare) = (load(&textures.sun), load(&textures.glare));
        [self.status.sun_texture, self.status.sun_glare_texture] =
            self.sun.install(device, disc, glare);
        self.installed_sun_textures = Installed::Requested(textures.clone());
    }

    /// Readies the clouds of this frame, which cover the stars, the moon and the sun wherever
    /// they are opaque. `radiance_scale` is the sky radiance scale the sky colours are multiplied
    /// by and `dt` the world seconds since the last call, which the layers scroll by. The cloud
    /// model loads when the first layer shows; the textures load the first time a layer shows
    /// them, on the calling thread. Without a call, or with no layers, no clouds are drawn.
    pub fn update_clouds(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        input: &CloudsInput<'_>,
        radiance_scale: f32,
        dt: f32,
    ) {
        self.load_cloud_model_for(device, input);
        let asset_root = &self.asset_root;
        self.clouds
            .update(device, queue, input, radiance_scale, dt, |path| {
                load_texture(device, queue, asset_root, &named_texture_key(path))
            });
    }

    fn load_cloud_model_for(&mut self, device: &wgpu::Device, input: &CloudsInput<'_>) {
        if input.layers.is_empty() || self.cloud_model_requested {
            return;
        }
        let geometry = load_cloud_geometry(&self.asset_root, DEFAULT_MODEL);
        self.status.cloud_mesh = self.clouds.install_mesh(device, geometry);
        self.cloud_model_requested = true;
    }

    /// Draws the stars, then the moon over them, then the sun, then the clouds over all of them.
    /// The layers take the world pass feedback group slot, so `feedback` is bound there again
    /// afterwards and the pass is as it was.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, feedback: &wgpu::BindGroup) {
        self.stars.draw(pass);
        self.moon.draw(pass);
        self.sun.draw(pass);
        self.clouds.draw(pass);
        pass.set_bind_group(FEEDBACK_GROUP, feedback, &[]);
    }

    pub fn asset_status(&self) -> &SkyAssetStatus {
        &self.status
    }
}
