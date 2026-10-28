use std::sync::Arc;

use glam::{UVec2, UVec4, Vec3, Vec4};

use super::ambient::{AmbientCube, EXTERIOR_AMBIENT_FILL};
use super::cell_lighting::CellLighting;
use super::environment::{Environment, InteriorLighting};
use super::exterior_weather::{ExteriorWeather, ExteriorWeatherSlots, ImageSpaceHdr};
use super::fog::{Fog, FogColors, FogParams, FogTime, HeightBands, SunlightTerm};
use super::frame_uniform::{FrameUniform, wrapped_time};
use super::interior_catalog::InteriorLightingCatalog;
use super::lightning::tuning::{FLASH_AMBIENT_GAIN, FLASH_SKY_GAIN};
use super::sky_clock::{ClockedSky, SkyClock};
use super::sky_measured::SKY_BRIGHTNESS;
use super::time_of_day::{
    DayTime, MoonKind, SkyGameConfig, SkyMoment, SkyObjects, SlotBlend, sun_light_weight,
};
use super::tonemap_settings::TonemapSettings;
use crate::rendering::camera::Camera;
use crate::rendering::debug_view::DebugView;
use crate::rendering::local_lights::LocalLightSettings;
use crate::rendering::local_lights::cluster_grid::{ClusterDepth, cluster_grid};
use crate::rendering::post::auto_exposure::AutoExposureParams;
use crate::rendering::post::depth_of_field::ImageSpaceDepthOfField;
use crate::rendering::post::night_sky::SunInput;
use crate::rendering::shadows::{CascadeSet, ShadowState, lamps::LampShadowState};
use crate::rendering::water::CameraWater;

/// Angular radius of the real sun's disc; sun shadow edges soften with it.
const SUN_ANGULAR_RADIUS_DEG: f32 = 0.266;
/// The Creation Kit states the eye adaptation speed per frame at 30 fps.
const ADAPTATION_REFERENCE_FPS: f32 = 30.0;
/// The automatic exposure settles at this share of the middle grey over the mean luminance of
/// the picture, within the image space's limits.
const AUTO_EXPOSURE_KEY: f32 = 0.5;
/// Towards a darker picture the adapted luminance falls by at most this much per second in natural
/// log, whatever the image space's eye adapt speed.
const DARKENING_LOG_RATE: f32 = 0.8;

/// The exterior's one directional light and the sun disc the sky paints. With the clock in
/// charge the direction and strength are the measured light's, which is never the disc's: it
/// stays up day and night.
#[derive(Debug, Copy, Clone)]
pub struct SunSettings {
    /// 0 = north (engine -Z), increasing clockwise towards +X (east).
    pub azimuth_deg: f32,
    pub elevation_deg: f32,
    pub color: Vec3,
    pub disc_color: Vec3,
    pub intensity: f32,
    /// 0 to 1, multiplies the direct light: the clock's light strength, or the manual sun's
    /// fade with its elevation.
    pub light_strength: f32,
    /// 0 to 1, multiplies the sun disc: the sun's alpha around sunrise and sunset.
    pub daylight: f32,
}

impl SunSettings {
    pub fn direction_to_sun(&self) -> Vec3 {
        let azimuth = self.azimuth_deg.to_radians();
        let elevation = self.elevation_deg.to_radians();
        Vec3::new(
            azimuth.sin() * elevation.cos(),
            elevation.sin(),
            -azimuth.cos() * elevation.cos(),
        )
    }

    /// Points the light along `direction`, a unit vector in engine space; the inverse of
    /// [`Self::direction_to_sun`].
    pub fn set_direction(&mut self, direction: Vec3) {
        self.azimuth_deg = direction
            .x
            .atan2(-direction.z)
            .to_degrees()
            .rem_euclid(360.0);
        self.elevation_deg = direction.y.clamp(-1.0, 1.0).asin().to_degrees();
    }

    /// Lets the light and the disc fade in with the elevation, as the clock does for the sun.
    pub fn fade_by_elevation(&mut self) {
        let weight = sun_light_weight(self.elevation_deg);
        self.light_strength = weight;
        self.daylight = weight;
    }

    pub fn illuminance(&self) -> Vec3 {
        self.light_strength * self.color * self.intensity
    }
}

impl Default for SunSettings {
    fn default() -> Self {
        Self {
            azimuth_deg: 220.0,
            elevation_deg: 40.0,
            color: Vec3::new(1.0, 0.90, 0.75),
            disc_color: Vec3::new(1.0, 0.90, 0.75),
            intensity: 1.5,
            light_strength: 1.0,
            daylight: 1.0,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct SkySettings {
    pub zenith: Vec3,
    pub horizon: Vec3,
    pub ground: Vec3,
    pub ambient_intensity: f32,
    /// Multiplies the sky radiance, not the ambient and not the fog.
    pub radiance_scale: f32,
}

impl SkySettings {
    /// +Y sees the zenith, the sides the horizon, -Y the ground bounce.
    pub fn ambient_cube(&self) -> AmbientCube {
        let [zenith, horizon, ground] =
            [self.zenith, self.horizon, self.ground].map(|color| color * self.ambient_intensity);
        AmbientCube([horizon, horizon, zenith, ground, horizon, horizon])
    }

    /// The light the sky shows for one of its colours.
    pub fn radiance(&self, color: Vec3) -> Vec3 {
        color * SKY_BRIGHTNESS * self.radiance_scale
    }

    /// The colour fog linked to the horizon takes: the horizon as the sky shows it at radiance
    /// scale 1.
    pub fn linked_fog_color(&self) -> Vec3 {
        self.horizon * SKY_BRIGHTNESS
    }
}

impl Default for SkySettings {
    fn default() -> Self {
        Self {
            zenith: Vec3::new(0.18, 0.36, 0.85),
            horizon: Vec3::new(0.65, 0.75, 0.90),
            ground: Vec3::new(0.20, 0.18, 0.15),
            ambient_intensity: 0.40,
            radiance_scale: 1.0,
        }
    }
}

/// Exterior fog. While linked, every fog colour follows the sky horizon as the sky shows it at
/// radiance scale 1, so distant geometry fades into the sky's horizon band.
#[derive(Debug, Copy, Clone)]
pub struct FogSettings {
    pub params: FogParams,
    pub link_to_horizon: bool,
}

impl FogSettings {
    pub fn fog(&self, horizon: Vec3) -> Fog {
        let mut params = self.params;
        if self.link_to_horizon {
            params.colors = FogColors::uniform(horizon);
        }
        Fog::Enabled(params)
    }
}

impl Default for FogSettings {
    fn default() -> Self {
        let horizon = SkySettings::default().horizon;
        Self {
            params: FogParams {
                colors: FogColors::uniform(horizon),
                near: 200.0,
                far: 12000.0,
                power: 1.0,
                max: 0.8,
                height: HeightBands::NONE,
                sunlight: SunlightTerm::NONE,
            },
            link_to_horizon: true,
        }
    }
}

/// The GPU auto-exposure state, a few frames stale on the CPU; the layout of
/// `State` in `shaders/auto_exposure.wgsl`.
#[repr(C)]
#[derive(Debug, Copy, Clone, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ExposureReadout {
    pub adapted_luminance: f32,
    pub exposure: f32,
}

#[derive(Debug, Copy, Clone)]
pub struct ExposureSettings {
    /// An offset on the auto exposure, the whole exposure while `auto` is off.
    pub ev: f32,
    pub auto: bool,
    pub hdr: ImageSpaceHdr,
    pub readout: ExposureReadout,
}

impl Default for ExposureSettings {
    fn default() -> Self {
        Self {
            ev: 0.0,
            auto: true,
            hdr: ImageSpaceHdr::default(),
            readout: ExposureReadout::default(),
        }
    }
}

impl ExposureSettings {
    pub fn gpu_params(&self, dt: f32) -> AutoExposureParams {
        AutoExposureParams {
            adaptation: 1.0
                - (-self.hdr.eye_adapt_speed / 100.0 * dt * ADAPTATION_REFERENCE_FPS).exp(),
            darkening: (-DARKENING_LOG_RATE * dt).exp(),
            target_luminance: AUTO_EXPOSURE_KEY * self.hdr.middle_gray,
            min: self.hdr.auto_exposure_min,
            max: self.hdr.auto_exposure_max,
            ev_scale: self.ev.exp2(),
            auto: u32::from(self.auto),
        }
    }
}

/// Which lighting model the current frame renders with, after the environment
/// and the XCLL A/B toggle are taken into account.
#[derive(Debug, Copy, Clone)]
pub enum Shading<'a> {
    Exterior,
    Cell(&'a CellLighting),
    Neutral,
}

fn shading_for(environment: &Environment, use_cell_lighting: bool) -> Shading<'_> {
    match environment {
        Environment::Interior { lighting, .. } if use_cell_lighting => match lighting {
            InteriorLighting::Cell(resolved) => Shading::Cell(&resolved.lighting),
            InteriorLighting::Neutral(_) => Shading::Neutral,
            InteriorLighting::CatalogUnavailable => Shading::Exterior,
        },
        Environment::Interior { .. } | Environment::Exterior => Shading::Exterior,
    }
}

/// The ambient cube a frame uploads. Exterior shading (which includes interiors
/// that fall back to the exterior terms) takes the weather's cube while one is
/// loaded and in use, the sky cube otherwise, applies the fill calibration to
/// whichever it picked and adds the light of the lightning flash. Resolved cell
/// lighting and the neutral interior upload their cubes as they are.
fn uploaded_ambient(
    shading: Shading<'_>,
    weather_cube: Option<AmbientCube>,
    use_weather_ambient: bool,
    sky: &SkySettings,
    lightning: Vec3,
) -> AmbientCube {
    match shading {
        Shading::Exterior => {
            let source = match weather_cube {
                Some(cube) if use_weather_ambient => cube,
                Some(_) | None => sky.ambient_cube(),
            };
            source
                .scaled(EXTERIOR_AMBIENT_FILL)
                .added(lightning * FLASH_AMBIENT_GAIN)
        }
        Shading::Cell(cell) => cell.ambient_cube,
        Shading::Neutral => sky.ambient_cube(),
    }
}

/// Everything in the frame uniform that differs between exterior and interior
/// shading, except the ambient cube, which `uploaded_ambient` chooses.
struct SceneTerms {
    to_sun: Vec3,
    /// The sun's alpha, for the water.
    daylight: f32,
    sun_illuminance: Vec3,
    /// The HDR sunlight scale already folded into `sun_illuminance` (1 when none).
    sunlight_scale: f32,
    sky_zenith: Vec3,
    sky_horizon: Vec3,
    sky_ground: Vec3,
    fog: Fog,
}

/// What the frame uniform needs beyond the lighting state itself.
pub struct FrameInputs<'a> {
    pub camera: &'a Camera,
    pub cascades: Option<&'a CascadeSet>,
    /// Framebuffer size in pixels.
    pub viewport: UVec2,
    /// Anisotropy clamp of the material sampler, for the sampled-mip view.
    pub max_anisotropy: u32,
    /// Texture feedback and mip view parameters (`TextureFeedback::frame_params`).
    pub texture_feedback: UVec4,
    /// Seconds since start; wrapped for the uniform by `frame_uniform`.
    pub elapsed_seconds: f32,
    pub camera_water: CameraWater,
}

/// Whether sun shadows render this frame, and why not.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ShadowStatus {
    Active,
    Disabled,
    Interior,
    NoDirectLight,
}

impl ShadowStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled in the panel",
            Self::Interior => "off in interiors",
            Self::NoDirectLight => "no direct light (sun and moon down)",
        }
    }
}

/// CPU authority for everything lighting-related; `frame_uniform` is the only
/// place that turns it into GPU data.
#[derive(Debug, Clone)]
pub struct LightingState {
    pub sun: SunSettings,
    pub sky: SkySettings,
    pub fog: FogSettings,
    pub exposure: ExposureSettings,
    pub tonemap: TonemapSettings,
    pub debug_view: DebugView,
    pub use_cell_lighting: bool,
    pub shadows: ShadowState,
    pub lamp_shadows: LampShadowState,
    pub local_lights: LocalLightSettings,
    /// While on and a weather is loaded, exterior ambient comes from the
    /// weather instead of the sky settings.
    pub use_weather_ambient: bool,
    /// While on, the clock no longer drives the sun, sky, fog, exposure and ambient: they keep the
    /// values the panels hold. The slot, moon and star readouts keep following the clock.
    pub sun_override: bool,
    /// The light of the lightning flash: not a weather value, so setting the weather leaves it.
    lightning: Vec3,
    exterior_weather: Option<ExteriorWeather>,
    /// How the time of day weighed the halves of the exterior weather's fog.
    fog_time: FogTime,
    sky_clock: Option<SkyClock>,
    sky_moment: Option<SkyMoment>,
    sky_objects: Option<SkyObjects>,
    environment: Environment,
}

impl LightingState {
    pub fn new(
        shadows: ShadowState,
        lamp_shadows: LampShadowState,
        local_lights: LocalLightSettings,
    ) -> Self {
        Self {
            sun: SunSettings::default(),
            sky: SkySettings::default(),
            fog: FogSettings::default(),
            exposure: ExposureSettings::default(),
            tonemap: TonemapSettings::default(),
            debug_view: DebugView::initial(),
            use_cell_lighting: true,
            shadows,
            lamp_shadows,
            local_lights,
            use_weather_ambient: true,
            sun_override: false,
            lightning: Vec3::ZERO,
            exterior_weather: None,
            fog_time: FogTime::DAY,
            sky_clock: None,
            sky_moment: None,
            sky_objects: None,
            environment: Environment::Exterior,
        }
    }

    /// Takes the weather's sun and sky colours and its image space's light
    /// scales and exposure as the new panel values, its directional
    /// ambient as the exterior ambient, and its fog as `fog_time` resolves it.
    pub fn set_exterior_weather(&mut self, weather: ExteriorWeather, fog_time: FogTime) {
        self.sun.intensity = weather.hdr.sunlight_scale;
        self.sky.radiance_scale = weather.hdr.sky_scale;
        self.exposure.hdr = weather.hdr;
        self.tonemap.cinematic = weather.cinematic;
        self.sun.color = weather.sun_light;
        self.sun.disc_color = weather.sun_disc;
        self.sky.zenith = weather.sky_upper;
        self.sky.horizon = weather.horizon;
        self.sky.ground = weather.sky_lower;
        match weather.fog.map(|fog| fog.resolve(fog_time)) {
            Some(Fog::Enabled(fog)) => {
                self.fog.params = fog;
                self.fog.link_to_horizon = false;
            }
            Some(Fog::Disabled) => {
                self.fog.params.near = 0.0;
                self.fog.params.far = 0.0;
                self.fog.link_to_horizon = false;
            }
            None => {}
        }
        self.fog_time = fog_time;
        self.exterior_weather = Some(weather);
    }

    /// Hands the exterior its weather and the settings the clock needs. The weather's Day slot
    /// applies until the first [`Self::set_time_of_day`].
    pub fn set_exterior_sky(&mut self, slots: ExteriorWeatherSlots, game: SkyGameConfig) {
        let clock = SkyClock::new(slots, game);
        self.set_exterior_weather(clock.initial_weather(), FogTime::DAY);
        self.sky_clock = Some(clock);
    }

    /// Moves the exterior to `day_time`: samples the weather, places the sun, moon and stars,
    /// and lights the world with the clock's light. Interior shading has no sky, so it clears
    /// the sky readouts. The readouts update either way, but with [`Self::sun_override`] on the
    /// sun, sky, fog and exposure keep their panel values. Whenever the clock does not drive the
    /// sun (overridden, or no [`Self::set_exterior_sky`] yet) the panel's sun fades with its
    /// elevation, so it never lights the world from below the horizon.
    pub fn set_time_of_day(&mut self, day_time: DayTime) {
        self.apply_time_of_day(day_time, None);
    }

    /// [`Self::set_time_of_day`] with `weather` instead of the one weather of the clock: the
    /// weather of the weather cycle, sampled at [`Self::slot_blend_at`] of the same time. Without
    /// a clock there is nothing to light, as for `set_time_of_day`.
    pub fn set_time_of_day_with_weather(&mut self, day_time: DayTime, weather: &ExteriorWeather) {
        self.apply_time_of_day(day_time, Some(weather));
    }

    /// The slots and the way between them that a weather is sampled at for `day_time`; `None`
    /// until [`Self::set_exterior_sky`] gave the exterior its clock.
    pub fn slot_blend_at(&self, day_time: DayTime) -> Option<SlotBlend> {
        self.sky_clock
            .as_ref()
            .map(|clock| clock.slot_blend(day_time))
    }

    fn apply_time_of_day(&mut self, day_time: DayTime, given: Option<&ExteriorWeather>) {
        if !matches!(self.shading(), Shading::Exterior) {
            self.sky_moment = None;
            self.sky_objects = None;
            return;
        }
        if self.sun_override || self.sky_clock.is_none() {
            self.sun.fade_by_elevation();
        }
        let Some(clock) = &self.sky_clock else {
            return;
        };
        let ClockedSky {
            moment,
            weather,
            sun_alpha,
            mut objects,
        } = match given {
            Some(weather) => clock.at_weather(day_time, weather),
            None => clock.at(day_time),
        };
        self.sky_moment = Some(moment);
        let fog_time = clock.fog_time(day_time);
        if self.sun_override {
            objects.sun_disc_direction = self.sun.direction_to_sun();
        } else {
            self.set_exterior_weather(weather, fog_time);
            self.sun.set_direction(moment.light.direction);
            self.sun.light_strength = moment.light.strength;
            self.sun.daylight = sun_alpha;
        }
        self.sky_objects = Some(objects);
    }

    /// Sets the light of the lightning flash, once per frame: what
    /// [`super::lightning::LightningFlashes::update`] returns. The exterior sky terms, the fog and
    /// the exterior ambient add it to the weather's values; the sun and the exposure ignore it.
    /// It is no weather value, so setting the weather or the time of day leaves it as it is.
    /// Cell lighting and the neutral interior ignore it.
    pub fn set_lightning(&mut self, flash: Vec3) {
        self.lightning = flash;
    }

    /// The radiance of the sky at the horizon as the exterior is lit with it, lightning flash
    /// included: the light the weather in the open air catches.
    pub fn horizon_radiance(&self) -> Vec3 {
        self.exterior_terms().sky_horizon
    }

    /// The flash lighting this frame, for the layers that draw weather: none where the frame is
    /// not shaded with the exterior terms.
    pub fn lightning(&self) -> Vec3 {
        match self.shading() {
            Shading::Exterior => self.lightning,
            Shading::Cell(_) | Shading::Neutral => Vec3::ZERO,
        }
    }

    /// The kind of the moon and the sky model the night sky loads first.
    pub fn initial_night_sky(&self) -> (Option<MoonKind>, Option<&Arc<str>>) {
        self.sky_clock.as_ref().map_or((None, None), |clock| {
            (clock.moon_kind(), clock.sky_model_path())
        })
    }

    /// The game settings of the sky, once the exterior has its clock.
    pub fn sky_game_config(&self) -> Option<&SkyGameConfig> {
        self.sky_clock.as_ref().map(SkyClock::game)
    }

    /// Where the clock put the sky at the last exterior update; `None` before the first.
    pub fn sky_moment(&self) -> Option<&SkyMoment> {
        self.sky_moment.as_ref()
    }

    /// What the sky renderer draws, as of the last exterior update; `None` before the first.
    pub fn sky_objects(&self) -> Option<&SkyObjects> {
        self.sky_objects.as_ref()
    }

    /// The sun disc's direction: the clock's sun, or the manual one while overridden.
    fn sun_disc_direction(&self) -> Vec3 {
        match &self.sky_objects {
            Some(objects) if !self.sun_override => objects.sun_disc_direction,
            Some(_) | None => self.sun.direction_to_sun(),
        }
    }

    /// The sun the sky draws: the disc's place and alpha, the disc colour (the panel's while the
    /// sun is overridden) and the weather's glare colour, which the panel has no control for.
    pub fn sun_input(&self) -> SunInput {
        SunInput {
            direction: self.sun_disc_direction(),
            alpha: self.sun.daylight,
            disc_color: self.sun.disc_color,
            glare_color: self
                .exterior_weather
                .as_ref()
                .map_or(self.sun.disc_color, |weather| weather.sun_glare),
        }
    }

    /// How the time of day last weighed the halves of the exterior weather's fog.
    pub fn fog_time(&self) -> FogTime {
        self.fog_time
    }

    pub fn exterior_weather(&self) -> Option<&ExteriorWeather> {
        self.exterior_weather.as_ref()
    }

    /// The depth of field and vignette the image-space manager holds: the weather's image
    /// space in the exterior. UNKNOWN for interiors (the image space of a cell is not
    /// exported yet): they have none.
    pub fn depth_of_field(&self) -> Option<&ImageSpaceDepthOfField> {
        match self.environment {
            Environment::Exterior => self.exterior_weather.as_ref()?.depth_of_field.as_ref(),
            Environment::Interior { .. } => None,
        }
    }

    pub fn environment(&self) -> &Environment {
        &self.environment
    }

    /// Rebuilds the environment only when the loaded interior changes.
    pub fn sync_environment(
        &mut self,
        interior: Option<&str>,
        catalog: Option<&InteriorLightingCatalog>,
    ) {
        if self.environment.cell_key() == interior {
            return;
        }
        self.environment = Environment::derive(interior, catalog);
        self.environment.log_entered();
    }

    /// The fog clip distance (engine units) of the loaded interior's lighting,
    /// template inheritance applied, if it has a positive one.
    pub fn interior_clip_distance(&self) -> Option<f32> {
        match &self.environment {
            Environment::Interior {
                lighting: InteriorLighting::Cell(resolved),
                ..
            } => resolved.lighting.clip_distance,
            Environment::Interior { .. } | Environment::Exterior => None,
        }
    }

    pub fn shading(&self) -> Shading<'_> {
        shading_for(&self.environment, self.use_cell_lighting)
    }

    pub fn draws_sky(&self) -> bool {
        matches!(self.shading(), Shading::Exterior)
    }

    /// Linear colour the HDR target is cleared to: the far fog colour inside a
    /// cell, black where fog is disabled or the sky paints over it.
    pub fn background_color(&self) -> Vec3 {
        match self.shading() {
            Shading::Cell(cell) => cell.fog.uniform_fields().far_color.truncate(),
            Shading::Exterior | Shading::Neutral => Vec3::ZERO,
        }
    }

    pub fn shadow_status(&self) -> ShadowStatus {
        if !self.shadows.settings.enabled {
            ShadowStatus::Disabled
        } else if !matches!(self.environment, Environment::Exterior) {
            ShadowStatus::Interior
        } else if self.sun.light_strength <= 0.0 {
            ShadowStatus::NoDirectLight
        } else {
            ShadowStatus::Active
        }
    }

    /// The cascades to render and sample this frame; `None` while inactive.
    pub fn shadow_cascades(&mut self, camera: &Camera, rendered: u32) -> Option<CascadeSet> {
        if self.shadow_status() != ShadowStatus::Active {
            self.shadows.discard_cached_cascades();
            return None;
        }
        Some(
            self.shadows
                .cascades(camera, self.sun.direction_to_sun(), rendered),
        )
    }

    pub fn frame_uniform(&self, inputs: &FrameInputs) -> FrameUniform {
        let FrameInputs {
            camera,
            cascades,
            viewport,
            max_anisotropy,
            texture_feedback,
            elapsed_seconds,
            camera_water,
        } = *inputs;
        let view_proj = camera.build_view_projection_matrix();
        let viewport = viewport.max(UVec2::ONE).as_vec2();
        let scene = self.scene_terms();
        let shadow = self.shadows.uniform_fields(cascades);
        let tonemap = self.tonemap.uniform(self.exposure.hdr.tonemap_e);

        let mut frame = FrameUniform {
            view_proj,
            inv_view_proj: view_proj.inverse(),
            camera_pos: camera.eye.extend(1.0),
            to_sun: scene
                .to_sun
                .extend(SUN_ANGULAR_RADIUS_DEG.to_radians().cos()),
            sun_illuminance: scene.sun_illuminance.extend(scene.sunlight_scale),
            daylight: Vec4::new(scene.daylight, 0.0, 0.0, 0.0),
            ambient_cube: self.uploaded_ambient().uniform_faces(),
            sky_zenith: scene.sky_zenith.extend(0.0),
            sky_horizon: scene.sky_horizon.extend(0.0),
            sky_ground: scene.sky_ground.extend(0.0),
            cascade_view_proj: shadow.cascade_view_proj,
            cascade_spheres: shadow.cascade_spheres,
            shadow_params: shadow.shadow_params,
            cascade_texel_world: shadow.cascade_texel_world,
            shadow_filter: shadow.shadow_filter,
            flags: UVec4::new(
                u32::from(cascades.is_some()),
                shadow.cascade_count,
                self.debug_view as u32,
                max_anisotropy,
            ),
            camera_forward: camera.forward().extend(0.0),
            viewport: Vec4::new(
                viewport.x,
                viewport.y,
                viewport.x.recip(),
                viewport.y.recip(),
            ),
            cluster_grid: cluster_grid(&self.local_lights),
            cluster_z: ClusterDepth::for_settings(&self.local_lights).uniform(),
            tonemap: tonemap.tonemap,
            cinematic: tonemap.cinematic,
            tint: tonemap.tint,
            texture_feedback,
            time_water: Vec4::new(
                wrapped_time(elapsed_seconds),
                camera_water.surface_height,
                f32::from(u8::from(camera_water.underwater)),
                camera_water.fraction,
            ),
            ..bytemuck::Zeroable::zeroed()
        };
        scene.fog.uniform_fields().write_to(&mut frame);
        frame
    }

    fn scene_terms(&self) -> SceneTerms {
        match self.shading() {
            Shading::Exterior => self.exterior_terms(),
            Shading::Neutral => SceneTerms {
                sun_illuminance: Vec3::ZERO,
                sky_zenith: Vec3::ZERO,
                sky_horizon: Vec3::ZERO,
                sky_ground: Vec3::ZERO,
                fog: Fog::Disabled,
                ..self.exterior_terms()
            },
            Shading::Cell(cell) => {
                let to_sun = cell.directional.map_or(Vec3::Y, |light| light.to_light);
                SceneTerms {
                    to_sun,
                    daylight: f32::from(to_sun.y > 0.0),
                    sun_illuminance: cell
                        .directional
                        .map_or(Vec3::ZERO, |light| light.color * light.fade.max(0.0)),
                    sunlight_scale: 1.0,
                    sky_zenith: Vec3::ZERO,
                    sky_horizon: Vec3::ZERO,
                    sky_ground: Vec3::ZERO,
                    fog: cell.fog,
                }
            }
        }
    }

    fn uploaded_ambient(&self) -> AmbientCube {
        uploaded_ambient(
            self.shading(),
            self.exterior_weather
                .as_ref()
                .map(|weather| weather.ambient.cube()),
            self.use_weather_ambient,
            &self.sky,
            self.lightning,
        )
    }

    fn exterior_terms(&self) -> SceneTerms {
        let sky_flash = self.lightning * FLASH_SKY_GAIN;
        SceneTerms {
            to_sun: self.sun.direction_to_sun(),
            daylight: self.sun.daylight,
            sun_illuminance: self.sun.illuminance(),
            sunlight_scale: self.sun.intensity,
            sky_zenith: self.sky.radiance(self.sky.zenith) + sky_flash,
            sky_horizon: self.sky.radiance(self.sky.horizon) + sky_flash,
            sky_ground: self.sky.radiance(self.sky.ground) + sky_flash,
            fog: self
                .fog
                .fog(self.sky.linked_fog_color())
                .brightened(sky_flash),
        }
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Lighting states for the tests of what drives the lighting.

    use super::super::environment::NeutralReason;
    use super::super::time_of_day::test_support::vanilla_game_config;
    use super::*;
    use crate::rendering::game_settings::GameSettings;
    use crate::rendering::test_support::ini_with;

    /// A state in the exterior with no weather and no sky clock yet.
    pub(crate) fn lighting_state() -> LightingState {
        let settings = GameSettings::from_ini(&ini_with("")).unwrap();
        LightingState::new(
            ShadowState::new(settings.shadows),
            LampShadowState::new(settings.lamp_shadows),
            LocalLightSettings::new(settings.light_fade),
        )
    }

    /// A state in the exterior whose sky clock samples `slots` with the vanilla sky settings.
    pub(crate) fn exterior_state(slots: ExteriorWeatherSlots) -> LightingState {
        let mut state = lighting_state();
        state.set_exterior_sky(slots, vanilla_game_config());
        state
    }

    /// `state` inside an interior that has no lighting of its own, which is shaded with neutral
    /// light and draws no sky.
    pub(crate) fn in_neutral_interior(mut state: LightingState) -> LightingState {
        state.environment = Environment::Interior {
            cell: "TestCell".to_owned(),
            lighting: InteriorLighting::Neutral(NeutralReason::NotInCatalog),
        };
        state
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests compare exact sentinels and copied values"
)]
mod tests {
    use super::super::ambient::AMBIENT_CUBE_FACES;
    use super::super::cell_lighting::{ResolvedLighting, TemplateStatus};
    use super::super::environment::NeutralReason;
    use super::super::exterior_weather::WeatherAmbient;
    use super::super::fog::measured::{BlendedFogColors, DAY_COLOUR_FACTORS};
    use super::super::fog::{ExteriorFog, FogRange};
    use super::super::tonemap_settings::ImageSpaceCinematic;
    use super::fixtures::lighting_state;
    use super::*;

    fn sun(azimuth_deg: f32, elevation_deg: f32) -> SunSettings {
        SunSettings {
            azimuth_deg,
            elevation_deg,
            color: Vec3::new(1.0, 0.5, 0.25),
            disc_color: Vec3::new(2.0, 2.0, 2.0),
            intensity: 3.0,
            light_strength: 1.0,
            daylight: 1.0,
        }
    }

    #[test]
    fn the_sun_at_azimuth_zero_is_north_which_is_engine_minus_z() {
        let to_sun = sun(0.0, 0.0).direction_to_sun();
        assert!(
            to_sun.abs_diff_eq(Vec3::new(0.0, 0.0, -1.0), 1e-6),
            "{to_sun}"
        );
    }

    #[test]
    fn azimuth_increases_clockwise_from_north_towards_east() {
        let east = sun(90.0, 0.0).direction_to_sun();
        let south = sun(180.0, 0.0).direction_to_sun();
        assert!(east.abs_diff_eq(Vec3::X, 1e-6), "{east}");
        assert!(south.abs_diff_eq(Vec3::Z, 1e-6), "{south}");
    }

    #[test]
    fn elevation_raises_the_sun_towards_the_zenith() {
        assert!(sun(0.0, 90.0).direction_to_sun().abs_diff_eq(Vec3::Y, 1e-6));
        let halfway = sun(0.0, 30.0).direction_to_sun();
        assert!((halfway.y - 0.5).abs() < 1e-6);
        assert!((halfway.length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_risen_sun_lights_with_colour_times_intensity() {
        let sun = sun(0.0, 10.0);
        assert_eq!(sun.illuminance(), Vec3::new(3.0, 1.5, 0.75));
    }

    #[test]
    fn the_light_strength_scales_the_direct_light() {
        let mut sun = sun(0.0, 10.0);
        sun.light_strength = 0.5;
        assert_eq!(sun.illuminance(), Vec3::new(1.5, 0.75, 0.375));
    }

    #[test]
    fn the_daylight_leaves_the_direct_light_alone() {
        let mut sun = sun(0.0, 10.0);
        sun.daylight = 0.25;
        assert_eq!(sun.illuminance(), Vec3::new(3.0, 1.5, 0.75));
    }

    #[test]
    fn no_strength_gives_no_light_whatever_the_elevation() {
        for elevation in [-45.0, 0.0, 60.0] {
            let mut sun = sun(0.0, elevation);
            sun.light_strength = 0.0;
            assert_eq!(sun.illuminance(), Vec3::ZERO, "{elevation}");
        }
    }

    #[test]
    fn fading_by_elevation_lights_nothing_at_the_horizon_and_everything_from_five_degrees() {
        let faded = |elevation| {
            let mut sun = sun(0.0, elevation);
            sun.fade_by_elevation();
            sun
        };
        for elevation in [-45.0, -0.1, 0.0] {
            let sun = faded(elevation);
            assert_eq!(sun.illuminance(), Vec3::ZERO, "{elevation}");
            assert_eq!(sun.daylight, 0.0, "{elevation}");
        }
        let halfway = faded(2.5);
        assert!((halfway.light_strength - 0.5).abs() < 1e-6);
        assert_eq!(halfway.daylight, halfway.light_strength);
        for elevation in [5.0, 40.0] {
            assert_eq!(faded(elevation).illuminance(), Vec3::new(3.0, 1.5, 0.75));
        }
    }

    #[test]
    fn setting_a_direction_is_the_inverse_of_reading_it() {
        let mut sun = sun(0.0, 0.0);
        for (azimuth, elevation) in [
            (0.0_f32, 0.0_f32),
            (90.0, 30.0),
            (180.0, -20.0),
            (270.0, 60.0),
            (359.0, 5.0),
        ] {
            let direction = Vec3::new(
                f32::sin(azimuth.to_radians()) * f32::cos(elevation.to_radians()),
                f32::sin(elevation.to_radians()),
                -f32::cos(azimuth.to_radians()) * f32::cos(elevation.to_radians()),
            );
            sun.set_direction(direction);
            assert!((sun.azimuth_deg - azimuth).abs() < 1e-3, "{azimuth}");
            assert!((sun.elevation_deg - elevation).abs() < 1e-3, "{elevation}");
            assert!(sun.direction_to_sun().abs_diff_eq(direction, 1e-5));
        }
    }

    #[test]
    fn setting_a_direction_keeps_the_azimuth_in_range_and_survives_straight_up() {
        let mut sun = sun(0.0, 0.0);
        sun.set_direction(Vec3::new(-1.0, 0.0, 0.0));
        assert!((sun.azimuth_deg - 270.0).abs() < 1e-3);
        sun.set_direction(Vec3::Y);
        assert!((sun.elevation_deg - 90.0).abs() < 1e-3);
        sun.set_direction(Vec3::new(0.0, 1.000_001, 0.0));
        assert!(sun.elevation_deg.is_finite());
    }

    #[test]
    fn the_sky_ambient_cube_sees_zenith_above_ground_below_and_horizon_around() {
        let sky = SkySettings {
            zenith: Vec3::new(1.0, 0.0, 0.0),
            horizon: Vec3::new(0.0, 1.0, 0.0),
            ground: Vec3::new(0.0, 0.0, 1.0),
            ambient_intensity: 1.0,
            radiance_scale: 1.0,
        };
        let cube = sky.ambient_cube().0;
        // Faces are +X, -X, +Y, -Y, +Z, -Z.
        assert_eq!(cube[2], Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(cube[3], Vec3::new(0.0, 0.0, 1.0));
        for side in [0, 1, 4, 5] {
            assert_eq!(cube[side], Vec3::new(0.0, 1.0, 0.0), "face {side}");
        }
    }

    #[test]
    fn zero_ambient_intensity_blacks_out_the_sky_ambient_cube() {
        let sky = SkySettings {
            ambient_intensity: 0.0,
            ..SkySettings::default()
        };
        assert!(sky.ambient_cube().0.iter().all(|face| *face == Vec3::ZERO));
    }

    /// A cube whose faces all differ, so a swapped or dropped face shows.
    fn distinct_cube(base: f32) -> AmbientCube {
        AmbientCube(std::array::from_fn(|face| {
            Vec3::new(base + face as f32, 0.5 * base, 2.0 * base + face as f32)
        }))
    }

    fn fill(cube: AmbientCube) -> AmbientCube {
        AmbientCube(cube.0.map(|face| face * EXTERIOR_AMBIENT_FILL))
    }

    fn weather_cube() -> AmbientCube {
        distinct_cube(0.1)
    }

    fn cell_cube() -> AmbientCube {
        distinct_cube(0.3)
    }

    fn weather() -> ExteriorWeather {
        ExteriorWeather {
            form_key: "TestWeather".into(),
            editor_id: None,
            sun_light: Vec3::ONE,
            sun_disc: Vec3::ONE,
            sun_glare: Vec3::ONE,
            sky_upper: Vec3::new(0.1, 0.2, 0.9),
            sky_lower: Vec3::new(0.3, 0.2, 0.1),
            horizon: Vec3::new(0.6, 0.7, 0.8),
            stars: Vec3::ZERO,
            ambient: WeatherAmbient {
                colors: weather_cube(),
                scale: 1.0,
            },
            ambient_specular: Vec3::ZERO,
            water_multiplier: Vec3::new(0.2, 0.4, 0.6),
            hdr: ImageSpaceHdr::default(),
            cinematic: ImageSpaceCinematic::NEUTRAL,
            depth_of_field: None,
            fog: None,
        }
    }

    fn resolved_cell() -> InteriorLighting {
        InteriorLighting::Cell(Box::new(ResolvedLighting {
            lighting: CellLighting {
                ambient_color: Vec3::splat(0.2),
                ambient_cube: cell_cube(),
                directional: None,
                fog: Fog::Disabled,
                clip_distance: None,
            },
            template: TemplateStatus::NoInheritance,
        }))
    }

    fn neutral() -> InteriorLighting {
        InteriorLighting::Neutral(NeutralReason::NotInCatalog)
    }

    fn interior(lighting: InteriorLighting) -> Environment {
        Environment::Interior {
            cell: "TestCell".to_string(),
            lighting,
        }
    }

    /// A state in `environment`, with a weather loaded or not and the two panel toggles set.
    fn state_with(
        environment: Environment,
        has_weather: bool,
        use_weather_ambient: bool,
        use_cell_lighting: bool,
    ) -> LightingState {
        let mut state = lighting_state();
        if has_weather {
            state.set_exterior_weather(weather(), FogTime::DAY);
        }
        state.environment = environment;
        state.use_weather_ambient = use_weather_ambient;
        state.use_cell_lighting = use_cell_lighting;
        state
    }

    #[test]
    fn without_a_sky_clock_a_sun_below_the_horizon_gives_no_light_and_no_shadows() {
        let mut state = state_with(Environment::Exterior, true, true, false);
        state.sun.elevation_deg = -20.0;
        state.set_time_of_day(DayTime::clamped(0, 12.0));
        assert_eq!(state.sun.illuminance(), Vec3::ZERO);
        assert_eq!(frame_of(&state).daylight.x, 0.0);
        assert_eq!(state.shadow_status(), ShadowStatus::NoDirectLight);
        state.sun.elevation_deg = 40.0;
        state.set_time_of_day(DayTime::clamped(0, 12.0));
        assert_eq!(state.shadow_status(), ShadowStatus::Active);
    }

    fn test_camera() -> Camera {
        Camera {
            eye: Vec3::new(0.0, 10.0, 5.0),
            target: Vec3::ZERO,
            up: Vec3::Y,
            aspect: 1.0,
            fovy: 1.0,
            znear: 0.5,
            zfar: 1000.0,
        }
    }

    fn frame_of(state: &LightingState) -> FrameUniform {
        state.frame_uniform(&FrameInputs {
            camera: &test_camera(),
            cascades: None,
            viewport: UVec2::new(64, 64),
            max_anisotropy: 1,
            texture_feedback: UVec4::ZERO,
            elapsed_seconds: 0.0,
            camera_water: CameraWater::default(),
        })
    }

    fn uploaded_cube(state: &LightingState) -> [Vec4; AMBIENT_CUBE_FACES] {
        frame_of(state).ambient_cube
    }

    #[test]
    fn exterior_with_the_weather_cube_in_use_uploads_it_with_the_fill() {
        let state = state_with(Environment::Exterior, true, true, true);
        assert_eq!(uploaded_cube(&state), fill(weather_cube()).uniform_faces());
    }

    #[test]
    fn exterior_with_the_weather_ambient_switched_off_uploads_the_sky_cube_with_the_fill() {
        let state = state_with(Environment::Exterior, true, false, true);
        assert_eq!(
            uploaded_cube(&state),
            fill(state.sky.ambient_cube()).uniform_faces()
        );
    }

    #[test]
    fn exterior_without_a_weather_uploads_the_sky_cube_with_the_fill() {
        for use_weather_ambient in [true, false] {
            let state = state_with(Environment::Exterior, false, use_weather_ambient, true);
            assert_eq!(
                uploaded_cube(&state),
                fill(state.sky.ambient_cube()).uniform_faces(),
                "{use_weather_ambient}"
            );
        }
    }

    #[test]
    fn interior_with_resolved_cell_lighting_uploads_its_cube_unscaled() {
        for (has_weather, use_weather_ambient) in [(true, true), (false, false)] {
            let state = state_with(
                interior(resolved_cell()),
                has_weather,
                use_weather_ambient,
                true,
            );
            assert_eq!(uploaded_cube(&state), cell_cube().uniform_faces());
        }
    }

    #[test]
    fn neutral_interior_uploads_the_sky_cube_unscaled() {
        for (has_weather, use_weather_ambient) in [(true, true), (false, false)] {
            let state = state_with(interior(neutral()), has_weather, use_weather_ambient, true);
            assert_eq!(
                uploaded_cube(&state),
                state.sky.ambient_cube().uniform_faces()
            );
        }
    }

    #[test]
    fn interior_without_a_lighting_catalog_is_shaded_as_exterior_with_the_fill() {
        let environment = || interior(InteriorLighting::CatalogUnavailable);
        let with_weather = state_with(environment(), true, true, true);
        assert_eq!(
            uploaded_cube(&with_weather),
            fill(weather_cube()).uniform_faces()
        );
        let without_weather = state_with(environment(), false, true, true);
        assert_eq!(
            uploaded_cube(&without_weather),
            fill(without_weather.sky.ambient_cube()).uniform_faces()
        );
    }

    #[test]
    fn interior_with_cell_lighting_switched_off_is_shaded_as_exterior_with_the_fill() {
        let environments = [
            interior(resolved_cell()),
            interior(neutral()),
            interior(InteriorLighting::CatalogUnavailable),
        ];
        for environment in environments {
            let with_weather = state_with(environment.clone(), true, true, false);
            assert_eq!(
                uploaded_cube(&with_weather),
                fill(weather_cube()).uniform_faces(),
                "{environment:?}"
            );
            let without_weather = state_with(environment.clone(), false, true, false);
            assert_eq!(
                uploaded_cube(&without_weather),
                fill(without_weather.sky.ambient_cube()).uniform_faces(),
                "{environment:?}"
            );
        }
    }

    #[test]
    fn linked_fog_takes_the_horizon_radiance_for_every_colour() {
        let settings = FogSettings::default();
        let radiance = Vec3::new(9.0, 8.0, 7.0);
        let Fog::Enabled(params) = settings.fog(radiance) else {
            panic!("exterior fog is always enabled");
        };
        assert_eq!(params.colors, FogColors::uniform(radiance));
        assert_eq!(params.near, settings.params.near);
    }

    #[test]
    fn unlinked_fog_keeps_its_own_colours() {
        let settings = FogSettings {
            link_to_horizon: false,
            params: FogParams {
                colors: FogColors {
                    near: Vec3::new(0.3, 0.3, 0.3),
                    far: Vec3::new(0.5, 0.25, 0.125),
                    near_high: Vec3::splat(0.75),
                    far_high: Vec3::splat(1.5),
                },
                ..FogSettings::default().params
            },
        };
        let Fog::Enabled(params) = settings.fog(Vec3::splat(9.0)) else {
            panic!("exterior fog is always enabled");
        };
        assert_eq!(params, settings.params);
    }

    fn exposure(ev: f32, auto: bool, adapt_speed: f32) -> ExposureSettings {
        ExposureSettings {
            ev,
            auto,
            hdr: ImageSpaceHdr {
                eye_adapt_speed: adapt_speed,
                middle_gray: 0.18,
                auto_exposure_min: 1.0,
                auto_exposure_max: 4.0,
                ..ImageSpaceHdr::default()
            },
            readout: ExposureReadout::default(),
        }
    }

    #[test]
    fn adaptation_is_the_exponential_approach_of_one_frame_at_thirty_fps() {
        // speed 3 -> 3 % per reference frame; one 30 fps frame: 1 - exp(-0.03).
        let params = exposure(0.0, true, 3.0).gpu_params(1.0 / 30.0);
        assert!(
            (params.adaptation - 0.029_554).abs() < 1e-5,
            "{}",
            params.adaptation
        );
    }

    #[test]
    fn adaptation_is_frame_rate_independent() {
        let once = exposure(0.0, true, 3.0).gpu_params(1.0 / 30.0).adaptation;
        let half = exposure(0.0, true, 3.0).gpu_params(1.0 / 60.0).adaptation;
        // Two half-steps of the remaining distance equal one full step.
        assert!((1.0 - (1.0 - half) * (1.0 - half) - once).abs() < 1e-6);
    }

    #[test]
    fn no_time_passed_means_no_adaptation() {
        let params = exposure(0.0, true, 3.0).gpu_params(0.0);
        assert_eq!((params.adaptation, params.darkening), (0.0, 1.0));
    }

    #[test]
    fn darkening_falls_at_its_log_rate_whatever_the_eye_adapt_speed() {
        let slow = exposure(0.0, true, 1.0).gpu_params(0.5).darkening;
        let fast = exposure(0.0, true, 5.0).gpu_params(0.5).darkening;
        assert_eq!(slow, fast);
        assert!((slow - (-0.4_f32).exp()).abs() < 1e-6, "{slow}");
    }

    #[test]
    fn darkening_is_frame_rate_independent() {
        let once = exposure(0.0, true, 3.0).gpu_params(1.0 / 30.0).darkening;
        let half = exposure(0.0, true, 3.0).gpu_params(1.0 / 60.0).darkening;
        assert!((half * half - once).abs() < 1e-6);
    }

    #[test]
    fn adapting_at_once_also_darkens_at_once() {
        let params = exposure(0.0, true, 3.0)
            .gpu_params(1.0 / 60.0)
            .adapted_at_once();
        assert_eq!((params.adaptation, params.darkening), (1.0, 0.0));
    }

    #[test]
    fn the_ev_offset_doubles_the_exposure_scale_per_stop() {
        let base = exposure(0.0, true, 3.0).gpu_params(0.0).ev_scale;
        let plus_one = exposure(1.0, true, 3.0).gpu_params(0.0).ev_scale;
        let minus_two = exposure(-2.0, true, 3.0).gpu_params(0.0).ev_scale;
        assert!((plus_one - 2.0 * base).abs() < 1e-6);
        assert!((minus_two - 0.25 * base).abs() < 1e-6);
    }

    #[test]
    fn the_auto_flag_and_image_space_limits_reach_the_gpu_params() {
        let on = exposure(0.0, true, 3.0).gpu_params(0.0);
        let off = exposure(0.0, false, 3.0).gpu_params(0.0);
        assert_eq!((on.auto, off.auto), (1, 0));
        assert_eq!((on.min, on.max), (1.0, 4.0));
    }

    #[test]
    fn the_auto_exposure_targets_its_key_share_of_the_middle_grey() {
        let params = exposure(0.0, true, 3.0).gpu_params(0.0);
        assert!((params.target_luminance - AUTO_EXPOSURE_KEY * 0.18).abs() < 1e-7);
    }

    #[test]
    fn without_an_ev_offset_the_exposure_is_the_image_space_exposure() {
        assert_eq!(exposure(0.0, true, 3.0).gpu_params(0.0).ev_scale, 1.0);
    }

    mod time_of_day {
        use glam::Vec3;

        use super::super::super::exterior_weather::fixtures::{
            per_slot, rgb, weather_slots, without_moon,
        };
        use super::super::super::lightning::tuning::FLASH_SKY_GAIN;
        use super::super::super::time_of_day::{MoonKind, MoonPhase, TimeSlot};
        use super::super::fixtures::exterior_state;
        use super::*;

        const DAY_SUNLIGHT: (u8, u8, u8) = (255, 255, 200);
        const NIGHT_SUNLIGHT: (u8, u8, u8) = (60, 70, 120);
        const NIGHT_SUNLIGHT_SCALE: f32 = 0.5;

        fn colour(rgb8: (u8, u8, u8)) -> Vec3 {
            let linear = |channel: u8| {
                let encoded = f32::from(channel) / 255.0;
                if encoded <= 0.04045 {
                    encoded / 12.92
                } else {
                    ((encoded + 0.055) / 1.055).powf(2.4)
                }
            };
            Vec3::new(linear(rgb8.0), linear(rgb8.1), linear(rgb8.2))
        }

        fn assert_colour(actual: Vec3, rgb8: (u8, u8, u8)) {
            let expected = colour(rgb8);
            assert!(
                actual.abs_diff_eq(expected, 1e-6),
                "{actual} is not {expected}"
            );
        }

        fn hdr(slot: TimeSlot) -> ImageSpaceHdr {
            ImageSpaceHdr {
                sunlight_scale: if slot == TimeSlot::Night {
                    NIGHT_SUNLIGHT_SCALE
                } else {
                    2.0
                },
                ..ImageSpaceHdr::default()
            }
        }

        fn slots() -> ExteriorWeatherSlots {
            weather_slots(
                |colors| {
                    colors["Sunlight"] = per_slot(|slot| {
                        let (r, g, b) = if slot == TimeSlot::Night {
                            NIGHT_SUNLIGHT
                        } else {
                            DAY_SUNLIGHT
                        };
                        rgb(r, g, b)
                    });
                    colors["Stars"] = per_slot(|slot| {
                        if slot == TimeSlot::Night {
                            rgb(255, 255, 255)
                        } else {
                            rgb(0, 0, 0)
                        }
                    });
                },
                hdr,
            )
        }

        fn state() -> LightingState {
            exterior_state(slots())
        }

        fn at_hour(state: &mut LightingState, days: u32, hour: f32) {
            state.set_time_of_day(DayTime::new(days, hour).unwrap());
        }

        #[test]
        fn setting_the_exterior_sky_applies_the_day_slot_before_any_clock_time() {
            let state = state();
            assert_colour(state.sun.color, DAY_SUNLIGHT);
            assert_eq!(state.sun.intensity, 2.0);
            assert!(state.sky_moment().is_none());
            assert!(state.sky_objects().is_none());
        }

        #[test]
        fn at_noon_the_sun_lights_with_the_day_colour_and_the_day_light_scale() {
            let mut state = state();
            at_hour(&mut state, 0, 13.0);
            let sun = state.sun;
            assert_colour(sun.color, DAY_SUNLIGHT);
            assert_eq!(sun.intensity, 2.0);
            assert_eq!(sun.light_strength, 1.0);
            assert_eq!(sun.daylight, 1.0);
            assert!(sun.elevation_deg > 20.0, "{}", sun.elevation_deg);
            let moment = state.sky_moment().unwrap();
            assert!(
                sun.direction_to_sun()
                    .abs_diff_eq(moment.light.direction, 1e-4)
            );
            assert!(
                sun.illuminance()
                    .abs_diff_eq(colour(DAY_SUNLIGHT) * 2.0, 1e-5),
                "colour times the light scale times the strength"
            );
        }

        #[test]
        fn at_solar_midnight_the_light_shines_from_the_south_with_the_night_sunlight_colour() {
            let mut state = state();
            at_hour(&mut state, 0, 1.0);
            let sun = state.sun;
            assert!(
                (sun.azimuth_deg - 180.0).abs() < 1e-3,
                "{}",
                sun.azimuth_deg
            );
            assert!(sun.elevation_deg > 20.0, "{}", sun.elevation_deg);
            assert_colour(sun.color, NIGHT_SUNLIGHT);
            assert!((sun.light_strength - 1.0).abs() < 1e-6);
            assert!(
                sun.illuminance()
                    .abs_diff_eq(colour(NIGHT_SUNLIGHT) * NIGHT_SUNLIGHT_SCALE, 1e-5),
            );
        }

        #[test]
        fn at_night_the_disc_stays_with_the_sun_below_the_horizon_and_is_dark() {
            let mut state = state();
            at_hour(&mut state, 0, 0.0);
            let frame = frame_of(&state);
            let sun_direction = state.sky_moment().unwrap().sun.direction();
            assert!(sun_direction.y < 0.0);
            assert!(state.sun_input().direction.abs_diff_eq(sun_direction, 1e-5));
            assert!(frame.to_sun.truncate().y > 0.0, "the light stays up");
            assert_eq!(frame.daylight.x, 0.0);
            assert_eq!(state.sun_input().alpha, 0.0);
        }

        #[test]
        fn at_noon_the_disc_stands_twenty_degrees_above_the_light() {
            let mut state = state();
            at_hour(&mut state, 0, 13.0);
            let frame = frame_of(&state);
            let disc = state.sun_input().direction;
            let light = frame.to_sun.truncate();
            let gap = disc.angle_between(light).to_degrees();
            assert!((19.0..22.0).contains(&gap), "{gap}");
            assert!(disc.y > light.y);
            assert_eq!(frame.daylight.x, 1.0);
        }

        #[test]
        fn the_sun_alpha_ramps_around_the_sunrise_midpoint_over_the_game_setting() {
            // The default climate rises at 07:00; the ramp is 0.15 h wide.
            let mut state = state();
            for (hour, alpha) in [(6.9, 0.0), (7.0, 0.5), (7.1, 1.0), (13.0, 1.0)] {
                at_hour(&mut state, 0, hour);
                let daylight = frame_of(&state).daylight.x;
                assert!((daylight - alpha).abs() < 1e-4, "{hour}: {daylight}");
            }
        }

        #[test]
        fn the_sun_alpha_falls_around_the_sunset_midpoint() {
            let mut state = state();
            for (hour, alpha) in [(18.9, 1.0), (19.0, 0.5), (19.1, 0.0)] {
                at_hour(&mut state, 0, hour);
                let daylight = frame_of(&state).daylight.x;
                assert!((daylight - alpha).abs() < 1e-4, "{hour}: {daylight}");
            }
        }

        #[test]
        fn the_readouts_name_the_slot_blend_and_the_moon_phase() {
            let mut state = state();
            at_hour(&mut state, 0, 5.25);
            let moment = state.sky_moment().unwrap();
            assert_eq!(
                (moment.blend.from, moment.blend.to),
                (TimeSlot::EarlySunrise, TimeSlot::Sunrise)
            );
            assert!((moment.blend.t - 0.5).abs() < 1e-5);
            assert_eq!(moment.moon.unwrap().phase, MoonPhase::Full);
            at_hour(&mut state, 4, 5.25);
            assert_eq!(
                state.sky_moment().unwrap().moon.unwrap().phase,
                MoonPhase::ThreeWan
            );
        }

        #[test]
        fn the_sky_objects_carry_the_moon_the_stars_and_the_climate_model() {
            let mut state = state();
            at_hour(&mut state, 0, 0.0);
            let objects = state.sky_objects().unwrap();
            let moon = objects.moon.unwrap();
            assert_eq!(moon.kind, MoonKind::Masser);
            assert_eq!(moon.visibility, 1.0);
            assert!((moon.direction.length() - 1.0).abs() < 1e-5);
            assert_eq!(objects.star_alpha, 1.0);
            assert_eq!(objects.sky_model_path.as_deref(), Some("Sky\\Stars.nif"));
            assert!((objects.star_rotation.axis.length() - 1.0).abs() < 1e-5);
        }

        #[test]
        fn the_stars_are_dark_by_day_and_follow_the_sampled_stars_colour() {
            let mut state = state();
            at_hour(&mut state, 0, 13.0);
            assert_eq!(state.sky_objects().unwrap().star_alpha, 0.0);
            // Halfway between Night and Day the Stars colour is half way too, in linear space.
            at_hour(&mut state, 0, 2.5);
            let before_dawn = state.sky_objects().unwrap().star_alpha;
            at_hour(&mut state, 0, 3.75);
            let at_dawn = state.sky_objects().unwrap().star_alpha;
            assert_eq!(before_dawn, 1.0);
            assert!(at_dawn < before_dawn && at_dawn > 0.0, "{at_dawn}");
        }

        #[test]
        fn a_climate_without_a_moon_has_no_moon_object_and_the_same_night_light() {
            let mut moonless = exterior_state(without_moon(slots()));
            let mut state = state();
            at_hour(&mut moonless, 0, 0.0);
            at_hour(&mut state, 0, 0.0);
            assert!(moonless.sky_objects().unwrap().moon.is_none());
            assert!(moonless.sky_moment().unwrap().moon.is_none());
            assert_eq!(moonless.sun.illuminance(), state.sun.illuminance());
            assert!(moonless.sun.illuminance().max_element() > 0.0);
        }

        #[test]
        fn the_direct_light_never_pops_over_a_whole_day() {
            let mut state = state();
            let mut previous: Option<Vec3> = None;
            for step in 0..2400 {
                at_hour(&mut state, 0, step as f32 * 0.01);
                let light = state.sun.illuminance();
                if let Some(previous) = previous {
                    assert!(
                        (light - previous).abs().max_element() < 0.1,
                        "hour {}: {previous} -> {light}",
                        step as f32 * 0.01
                    );
                }
                previous = Some(light);
            }
        }

        #[test]
        fn the_weather_follows_the_clock_across_the_day() {
            let mut state = state();
            at_hour(&mut state, 0, 13.0);
            let noon_exposure = state.exposure.hdr;
            let noon_sky = state.sky.zenith;
            at_hour(&mut state, 0, 0.0);
            assert_eq!(state.exposure.hdr.sunlight_scale, NIGHT_SUNLIGHT_SCALE);
            assert_eq!(noon_exposure.sunlight_scale, 2.0);
            assert_eq!(state.sky.zenith, noon_sky, "uniform colours stay put");
            assert_colour(state.exterior_weather().unwrap().sun_light, NIGHT_SUNLIGHT);
        }

        #[test]
        fn a_lightning_flash_survives_the_clock_rewriting_the_weather_every_frame() {
            let flash = Vec3::new(0.5, 0.25, 1.0);
            let (mut plain, mut flashing) = (state(), state());
            flashing.set_lightning(flash);
            for hour in [13.0, 0.0, 6.5, 19.0] {
                at_hour(&mut plain, 0, hour);
                at_hour(&mut flashing, 0, hour);
                let (plain, flashing) = (frame_of(&plain), frame_of(&flashing));
                for (lit, unlit) in [
                    (flashing.sky_zenith, plain.sky_zenith),
                    (flashing.sky_horizon, plain.sky_horizon),
                    (flashing.sky_ground, plain.sky_ground),
                ] {
                    let lifted = unlit.truncate() + flash * FLASH_SKY_GAIN;
                    assert!(lit.truncate().abs_diff_eq(lifted, 1e-6), "{hour}: {lit}");
                }
            }
        }

        #[test]
        fn shadows_follow_the_light_day_and_night_with_or_without_a_moon() {
            for slots in [slots(), without_moon(slots())] {
                let mut state = exterior_state(slots);
                state.shadows.settings.enabled = true;
                for hour in [0.0, 7.25, 13.0, 20.0] {
                    at_hour(&mut state, 0, hour);
                    assert_eq!(state.shadow_status(), ShadowStatus::Active, "{hour}");
                    let cascades = state.shadow_cascades(&test_camera(), 0).unwrap();
                    let light = state.sky_moment().unwrap().light.direction;
                    assert!(cascades.to_sun.abs_diff_eq(light, 1e-4), "{hour}");
                }
            }
        }

        #[test]
        fn the_shadow_panel_status_still_reports_a_disabled_setting_first() {
            let mut state = state();
            state.shadows.settings.enabled = false;
            at_hour(&mut state, 0, 0.0);
            assert_eq!(state.shadow_status(), ShadowStatus::Disabled);
        }

        fn manual_sun(state: &mut LightingState) {
            state.sun_override = true;
            state.sun.azimuth_deg = 123.0;
            state.sun.elevation_deg = 33.0;
            state.sun.color = Vec3::new(0.9, 0.1, 0.2);
            state.sun.disc_color = Vec3::new(0.3, 0.4, 0.5);
            state.sun.intensity = 4.5;
            state.sky.zenith = Vec3::new(0.01, 0.02, 0.03);
            state.sky.horizon = Vec3::new(0.04, 0.05, 0.06);
            state.sky.ground = Vec3::new(0.07, 0.08, 0.09);
            state.sky.radiance_scale = 3.5;
        }

        #[test]
        fn the_override_keeps_the_manual_sun_and_sky_whatever_the_time() {
            let mut state = state();
            at_hour(&mut state, 0, 13.0);
            manual_sun(&mut state);
            let (sun, sky) = (state.sun, state.sky);
            for hour in [0.0, 6.5, 13.0, 19.0] {
                at_hour(&mut state, 0, hour);
                assert_eq!(state.sun.azimuth_deg, sun.azimuth_deg, "{hour}");
                assert_eq!(state.sun.elevation_deg, sun.elevation_deg, "{hour}");
                assert_eq!(state.sun.color, sun.color, "{hour}");
                assert_eq!(state.sun.disc_color, sun.disc_color, "{hour}");
                assert_eq!(state.sun.intensity, sun.intensity, "{hour}");
                assert_eq!(state.sky.zenith, sky.zenith, "{hour}");
                assert_eq!(state.sky.horizon, sky.horizon, "{hour}");
                assert_eq!(state.sky.ground, sky.ground, "{hour}");
                assert_eq!(state.sky.radiance_scale, sky.radiance_scale, "{hour}");
            }
        }

        #[test]
        fn the_override_freezes_exposure_and_the_ambient_with_the_rest() {
            let mut state = state();
            at_hour(&mut state, 0, 13.0);
            manual_sun(&mut state);
            let exposure = state.exposure.hdr;
            let ambient = state.exterior_weather().unwrap().ambient;
            at_hour(&mut state, 0, 0.0);
            assert_eq!(state.exposure.hdr, exposure);
            assert_eq!(state.exterior_weather().unwrap().ambient, ambient);
        }

        #[test]
        fn the_readouts_keep_following_the_clock_under_the_override() {
            let mut state = state();
            manual_sun(&mut state);
            at_hour(&mut state, 0, 5.25);
            let moment = *state.sky_moment().unwrap();
            assert_eq!(moment.blend.from, TimeSlot::EarlySunrise);
            let objects = state.sky_objects().unwrap();
            assert!(objects.moon.is_some());
            at_hour(&mut state, 0, 0.0);
            assert_eq!(state.sky_moment().unwrap().blend.from, TimeSlot::Night);
        }

        #[test]
        fn the_override_draws_the_disc_where_the_manual_sun_is() {
            let mut state = state();
            at_hour(&mut state, 0, 0.0);
            manual_sun(&mut state);
            let frame = frame_of(&state);
            let manual = state.sun.direction_to_sun();
            assert!(state.sun_input().direction.abs_diff_eq(manual, 1e-5));
            assert!(frame.to_sun.truncate().abs_diff_eq(manual, 1e-5));
        }

        #[test]
        fn the_override_lights_with_a_strength_that_fades_with_the_manual_elevation() {
            let mut state = state();
            manual_sun(&mut state);
            for (elevation, strength) in [(-10.0, 0.0), (0.0, 0.0), (2.5, 0.5), (40.0, 1.0)] {
                state.sun.elevation_deg = elevation;
                at_hour(&mut state, 0, 13.0);
                assert!(
                    (state.sun.light_strength - strength).abs() < 1e-5,
                    "{elevation}: {}",
                    state.sun.light_strength
                );
                assert_eq!(state.sun.daylight, state.sun.light_strength);
            }
        }

        #[test]
        fn a_manual_sun_below_the_horizon_casts_no_shadows() {
            let mut state = state();
            state.shadows.settings.enabled = true;
            manual_sun(&mut state);
            state.sun.elevation_deg = -5.0;
            at_hour(&mut state, 0, 13.0);
            assert_eq!(state.shadow_status(), ShadowStatus::NoDirectLight);
            state.sun.elevation_deg = 20.0;
            at_hour(&mut state, 0, 13.0);
            assert_eq!(state.shadow_status(), ShadowStatus::Active);
        }

        #[test]
        fn turning_the_override_off_hands_the_lighting_back_to_the_clock() {
            let mut state = state();
            manual_sun(&mut state);
            at_hour(&mut state, 0, 13.0);
            state.sun_override = false;
            at_hour(&mut state, 0, 13.0);
            assert_colour(state.sun.color, DAY_SUNLIGHT);
            assert_eq!(state.sun.intensity, 2.0);
            let light = state.sky_moment().unwrap().light.direction;
            assert!(state.sun.direction_to_sun().abs_diff_eq(light, 1e-4));
        }

        #[test]
        fn interior_shading_leaves_the_lighting_alone_and_clears_the_sky_readouts() {
            for lighting in [resolved_cell(), neutral()] {
                let mut state = state();
                at_hour(&mut state, 0, 13.0);
                let sun = state.sun;
                state.environment = interior(lighting);
                at_hour(&mut state, 0, 0.0);
                assert_eq!(state.sun.color, sun.color);
                assert_eq!(state.sun.azimuth_deg, sun.azimuth_deg);
                assert!(state.sky_moment().is_none());
                assert!(state.sky_objects().is_none());
            }
        }

        #[test]
        fn an_interior_shaded_like_the_exterior_still_follows_the_clock() {
            let mut state = state();
            state.environment = interior(InteriorLighting::CatalogUnavailable);
            at_hour(&mut state, 0, 0.0);
            assert_colour(state.sun.color, NIGHT_SUNLIGHT);
            state.environment = interior(resolved_cell());
            state.use_cell_lighting = false;
            at_hour(&mut state, 0, 13.0);
            assert_colour(state.sun.color, DAY_SUNLIGHT);
        }

        #[test]
        fn without_a_weather_the_clock_changes_nothing() {
            let mut state = lighting_state();
            let before = state.sun;
            at_hour(&mut state, 0, 0.0);
            assert_eq!(state.sun.color, before.color);
            assert_eq!(state.sun.elevation_deg, before.elevation_deg);
            assert!(state.sky_moment().is_none());
            assert!(state.sky_objects().is_none());
            assert!(state.exterior_weather().is_none());
        }

        #[test]
        fn without_a_weather_the_override_still_fades_the_manual_sun() {
            let mut state = lighting_state();
            state.sun_override = true;
            state.sun.elevation_deg = -5.0;
            at_hour(&mut state, 0, 13.0);
            assert_eq!(state.sun.illuminance(), Vec3::ZERO);
            state.shadows.settings.enabled = true;
            assert_eq!(state.shadow_status(), ShadowStatus::NoDirectLight);
        }

        fn time(hour: f32) -> DayTime {
            DayTime::new(0, hour).unwrap()
        }

        /// A weather that differs from `slots()` in everything lighting reads from it.
        fn other_weather(state: &LightingState, hour: f32) -> ExteriorWeather {
            let own = slots().sample(state.slot_blend_at(time(hour)).unwrap());
            ExteriorWeather {
                sun_light: Vec3::new(0.2, 0.4, 0.6),
                sky_upper: Vec3::new(0.1, 0.1, 0.9),
                stars: Vec3::ZERO,
                hdr: ImageSpaceHdr {
                    sunlight_scale: 7.0,
                    sky_scale: 3.0,
                    ..ImageSpaceHdr::default()
                },
                ..own
            }
        }

        #[test]
        fn the_blend_of_a_time_is_the_one_the_clock_places_the_sky_at() {
            let mut state = state();
            for hour in [0.0, 5.5, 6.5, 13.0, 19.25, 23.5] {
                at_hour(&mut state, 0, hour);
                assert_eq!(
                    state.slot_blend_at(time(hour)),
                    state.sky_moment().map(|moment| moment.blend),
                    "{hour}"
                );
            }
        }

        #[test]
        fn without_a_clock_there_is_no_blend_to_sample_a_weather_at() {
            assert_eq!(lighting_state().slot_blend_at(time(12.0)), None);
        }

        #[test]
        fn the_clocks_own_weather_given_explicitly_lights_the_world_exactly_as_the_clock_does() {
            let (mut own, mut given) = (state(), state());
            for hour in [0.0, 5.5, 7.0, 13.0, 19.0, 23.5] {
                at_hour(&mut own, 0, hour);
                let weather = slots().sample(given.slot_blend_at(time(hour)).unwrap());
                given.set_time_of_day_with_weather(time(hour), &weather);
                assert_eq!(
                    bytemuck::bytes_of(&frame_of(&given)),
                    bytemuck::bytes_of(&frame_of(&own)),
                    "{hour}: frame"
                );
                assert_eq!(given.exterior_weather(), own.exterior_weather(), "{hour}");
                assert_eq!(given.sky_objects(), own.sky_objects(), "{hour}");
                assert_eq!(given.sky_moment(), own.sky_moment(), "{hour}");
            }
        }

        #[test]
        fn a_given_weather_lights_the_world_instead_of_the_clocks_own() {
            let mut state = state();
            let given = other_weather(&state, 13.0);

            state.set_time_of_day_with_weather(time(13.0), &given);

            assert_eq!(state.sun.color, given.sun_light);
            assert_eq!(state.sun.intensity, 7.0);
            assert_eq!(state.sky.zenith, given.sky_upper);
            assert_eq!(state.sky.radiance_scale, 3.0);
            assert_eq!(state.exterior_weather(), Some(&given));
        }

        #[test]
        fn the_stars_follow_the_given_weather() {
            let mut state = state();
            let given = other_weather(&state, 0.0);
            at_hour(&mut state, 0, 0.0);
            assert_eq!(state.sky_objects().unwrap().star_alpha, 1.0);

            state.set_time_of_day_with_weather(time(0.0), &given);

            assert_eq!(state.sky_objects().unwrap().star_alpha, 0.0);
        }

        #[test]
        fn a_given_weather_does_not_move_the_sun_the_moon_or_the_stars() {
            let (mut own, mut given) = (state(), state());
            let weather = other_weather(&given, 6.5);
            at_hour(&mut own, 0, 6.5);

            given.set_time_of_day_with_weather(time(6.5), &weather);

            assert_eq!(given.sky_moment(), own.sky_moment());
            assert_eq!(given.sun.azimuth_deg, own.sun.azimuth_deg);
            assert_eq!(given.sun.elevation_deg, own.sun.elevation_deg);
            assert_eq!(given.sun.light_strength, own.sun.light_strength);
            assert_eq!(given.sun.daylight, own.sun.daylight);
            let (given_objects, own_objects) =
                (given.sky_objects().unwrap(), own.sky_objects().unwrap());
            assert_eq!(given_objects.moon, own_objects.moon);
            assert_eq!(given_objects.star_rotation, own_objects.star_rotation);
        }

        #[test]
        fn the_override_keeps_the_panel_values_whatever_weather_is_given() {
            let mut state = state();
            at_hour(&mut state, 0, 13.0);
            manual_sun(&mut state);
            let (sun, sky, exposure) = (state.sun, state.sky, state.exposure.hdr);
            let given = other_weather(&state, 0.0);

            state.set_time_of_day_with_weather(time(0.0), &given);

            assert_eq!(state.sun.color, sun.color);
            assert_eq!(state.sun.intensity, sun.intensity);
            assert_eq!(state.sky.zenith, sky.zenith);
            assert_eq!(state.sky.radiance_scale, sky.radiance_scale);
            assert_eq!(state.exposure.hdr, exposure);
            assert_eq!(
                state.sky_moment().map(|moment| moment.blend),
                state.slot_blend_at(time(0.0)),
                "the readouts keep following the clock"
            );
        }

        #[test]
        fn interior_shading_ignores_a_given_weather_as_it_ignores_the_clocks_own() {
            for lighting in [resolved_cell(), neutral()] {
                let mut state = state();
                at_hour(&mut state, 0, 13.0);
                let sun_color = state.sun.color;
                let given = other_weather(&state, 0.0);
                state.environment = interior(lighting);

                state.set_time_of_day_with_weather(time(0.0), &given);

                assert_eq!(state.sun.color, sun_color);
                assert!(state.sky_moment().is_none());
                assert!(state.sky_objects().is_none());
            }
        }

        #[test]
        fn without_a_clock_a_given_weather_changes_nothing() {
            let mut state = lighting_state();
            let before = state.sun.color;
            let given = ExteriorWeather {
                sun_light: Vec3::new(0.2, 0.4, 0.6),
                ..weather()
            };

            state.set_time_of_day_with_weather(time(0.0), &given);

            assert_eq!(state.sun.color, before);
            assert!(state.exterior_weather().is_none());
            assert!(state.sky_moment().is_none());
        }
    }

    mod lightning {
        use super::*;
        use crate::rendering::lighting::lightning::tuning::{
            ATTACK_S, FLASH_AMBIENT_GAIN, FLASH_LENGTH_S, FLASH_SKY_GAIN,
        };
        use crate::rendering::lighting::lightning::{LightningFlashes, Strike};

        /// Distinct channels, so a swapped or dropped channel shows.
        const FLASH: Vec3 = Vec3::new(0.5, 0.25, 1.0);

        fn exterior() -> LightingState {
            state_with(Environment::Exterior, true, true, true)
        }

        fn with_flash(mut state: LightingState) -> LightingState {
            state.set_lightning(FLASH);
            state
        }

        fn bytes(frame: &FrameUniform) -> &[u8] {
            bytemuck::bytes_of(frame)
        }

        /// `lit` is `unlit` plus `lift` in its colour, with the fourth component as it was.
        fn assert_lifted(lit: Vec4, unlit: Vec4, lift: Vec3) {
            assert!(
                lit.truncate().abs_diff_eq(unlit.truncate() + lift, 1e-6),
                "{lit} is not {unlit} plus {lift}"
            );
            assert_eq!(lit.w, unlit.w);
        }

        fn weather_with_fog(fog: ExteriorFog) -> ExteriorWeather {
            ExteriorWeather {
                fog: Some(fog),
                ..weather()
            }
        }

        fn own_range() -> FogRange {
            FogRange {
                near: 100.0,
                far: 5000.0,
                power: 1.5,
                max: 0.7,
                height: HeightBands::NONE,
            }
        }

        fn own_fog() -> ExteriorFog {
            ExteriorFog {
                colors: BlendedFogColors::from_stored(
                    FogColors {
                        near: Vec3::new(0.1, 0.2, 0.3),
                        far: Vec3::new(0.4, 0.5, 0.6),
                        near_high: Vec3::new(0.7, 0.6, 0.5),
                        far_high: Vec3::new(0.2, 0.3, 0.4),
                    },
                    DAY_COLOUR_FACTORS,
                ),
                sunlight: Vec3::splat(0.5),
                day_weight: 1.0,
                day: Some(own_range()),
                night: Some(FogRange {
                    near: 300.0,
                    far: 9000.0,
                    ..own_range()
                }),
            }
        }

        fn without_fog() -> ExteriorFog {
            ExteriorFog {
                day: None,
                night: None,
                ..own_fog()
            }
        }

        #[test]
        fn a_flash_lifts_the_sky_terms_whatever_the_weathers_sky_scale() {
            let mut state = exterior();
            state.sky.radiance_scale = 3.0;
            let unlit = frame_of(&state);
            let lit = frame_of(&with_flash(state));
            let lift = FLASH * FLASH_SKY_GAIN;
            assert_lifted(lit.sky_zenith, unlit.sky_zenith, lift);
            assert_lifted(lit.sky_horizon, unlit.sky_horizon, lift);
            assert_lifted(lit.sky_ground, unlit.sky_ground, lift);
        }

        #[test]
        fn a_flash_lifts_the_fog_that_follows_the_horizon_with_the_horizon() {
            let state = exterior();
            let unlit = frame_of(&state);
            let lit = frame_of(&with_flash(state));
            let lift = FLASH * FLASH_SKY_GAIN;
            assert_lifted(lit.fog_near_color, unlit.fog_near_color, lift);
            assert_lifted(lit.fog_far_color, unlit.fog_far_color, lift);
            assert_eq!(lit.fog_params, unlit.fog_params);
            assert!(
                lit.fog_far_color
                    .truncate()
                    .abs_diff_eq(lit.sky_horizon.truncate(), 1e-6)
            );
        }

        #[test]
        fn a_flash_lifts_the_colours_of_a_fog_the_weather_brings_and_keeps_its_range() {
            let mut state = lighting_state();
            state.set_exterior_weather(weather_with_fog(own_fog()), FogTime::DAY);
            let unlit = frame_of(&state);
            let lit = frame_of(&with_flash(state));
            let lift = FLASH * FLASH_SKY_GAIN;
            assert_lifted(lit.fog_near_color, unlit.fog_near_color, lift);
            assert_lifted(lit.fog_far_color, unlit.fog_far_color, lift);
            assert_eq!(lit.fog_params, unlit.fog_params);
            assert_eq!(lit.fog_near_color.w, own_range().near);
            assert_eq!(lit.fog_far_color.w, own_range().far);
        }

        #[test]
        fn a_flash_does_not_switch_on_a_fog_the_weather_disables() {
            let mut state = lighting_state();
            state.set_exterior_weather(weather_with_fog(without_fog()), FogTime::DAY);
            let unlit = frame_of(&state);
            let lit = frame_of(&with_flash(state));
            assert_eq!(unlit.fog_params, Vec4::ZERO);
            assert_eq!(lit.fog_params, unlit.fog_params);
            assert_eq!(lit.fog_near_color, unlit.fog_near_color);
            assert_eq!(lit.fog_far_color, unlit.fog_far_color);
        }

        #[test]
        fn a_flash_adds_its_ambient_gain_to_every_face_after_the_fill() {
            let lift = FLASH * FLASH_AMBIENT_GAIN;
            let weather_ambient = with_flash(exterior());
            assert_eq!(
                uploaded_cube(&weather_ambient),
                fill(weather_cube()).added(lift).uniform_faces()
            );
            for (has_weather, use_weather_ambient) in [(true, false), (false, true)] {
                let state = with_flash(state_with(
                    Environment::Exterior,
                    has_weather,
                    use_weather_ambient,
                    true,
                ));
                assert_eq!(
                    uploaded_cube(&state),
                    fill(state.sky.ambient_cube()).added(lift).uniform_faces(),
                    "{has_weather} {use_weather_ambient}"
                );
            }
        }

        #[test]
        fn a_flash_leaves_the_sun_and_the_exposure_alone() {
            const FRAME_SECONDS: f32 = 1.0 / 30.0;
            let state = exterior();
            let unlit = frame_of(&state);
            let unlit_exposure = state.exposure.gpu_params(FRAME_SECONDS);
            let flashing = with_flash(state);
            let lit = frame_of(&flashing);
            assert_eq!(lit.to_sun, unlit.to_sun);
            assert_eq!(lit.sun_illuminance, unlit.sun_illuminance);
            assert_eq!(lit.daylight, unlit.daylight);
            assert_eq!(
                bytemuck::bytes_of(&flashing.exposure.gpu_params(FRAME_SECONDS)),
                bytemuck::bytes_of(&unlit_exposure)
            );
        }

        #[test]
        fn a_flash_does_not_reach_cell_lighting_or_the_neutral_interior() {
            for lighting in [resolved_cell(), neutral()] {
                let state = state_with(interior(lighting), true, true, true);
                let unlit = frame_of(&state);
                let lit = frame_of(&with_flash(state));
                assert_eq!(bytes(&lit), bytes(&unlit));
            }
        }

        #[test]
        fn an_interior_shaded_like_the_exterior_takes_the_flash_with_the_exterior_terms() {
            let shaded_like_the_exterior = [
                state_with(
                    interior(InteriorLighting::CatalogUnavailable),
                    true,
                    true,
                    true,
                ),
                state_with(interior(resolved_cell()), true, true, false),
            ];
            for state in shaded_like_the_exterior {
                let unlit = frame_of(&state);
                let lit = frame_of(&with_flash(state));
                assert_lifted(lit.sky_zenith, unlit.sky_zenith, FLASH * FLASH_SKY_GAIN);
                assert_ne!(lit.ambient_cube, unlit.ambient_cube);
            }
        }

        #[test]
        fn the_flash_is_readable_wherever_the_exterior_terms_apply_and_none_elsewhere() {
            assert_eq!(with_flash(exterior()).lightning(), FLASH);
            let fallback = state_with(
                interior(InteriorLighting::CatalogUnavailable),
                true,
                true,
                true,
            );
            assert_eq!(with_flash(fallback).lightning(), FLASH);
            for lighting in [resolved_cell(), neutral()] {
                let state = state_with(interior(lighting), true, true, true);
                assert_eq!(with_flash(state).lightning(), Vec3::ZERO);
            }
        }

        #[test]
        fn no_flash_is_set_until_one_is_given() {
            assert_eq!(exterior().lightning(), Vec3::ZERO);
        }

        #[test]
        fn a_flash_survives_the_weather_being_set_again_with_other_values() {
            let other = ExteriorWeather {
                sky_upper: Vec3::new(0.7, 0.1, 0.2),
                horizon: Vec3::new(0.3, 0.4, 0.5),
                sky_lower: Vec3::new(0.05, 0.06, 0.07),
                fog: Some(own_fog()),
                ..weather()
            };
            let mut plain = exterior();
            plain.set_exterior_weather(other.clone(), FogTime::DAY);
            let mut flashing = with_flash(exterior());
            flashing.set_exterior_weather(other, FogTime::DAY);
            assert_eq!(flashing.lightning(), FLASH);
            let (lit, unlit) = (frame_of(&flashing), frame_of(&plain));
            let lift = FLASH * FLASH_SKY_GAIN;
            assert_lifted(lit.sky_zenith, unlit.sky_zenith, lift);
            assert_lifted(lit.sky_horizon, unlit.sky_horizon, lift);
            assert_lifted(lit.sky_ground, unlit.sky_ground, lift);
            assert_lifted(lit.fog_far_color, unlit.fog_far_color, lift);
        }

        #[test]
        fn clearing_the_flash_brings_the_frame_back_bit_for_bit() {
            let mut state = exterior();
            let unlit = frame_of(&state);
            state.set_lightning(FLASH);
            assert_ne!(bytes(&frame_of(&state)), bytes(&unlit));
            state.set_lightning(Vec3::ZERO);
            assert_eq!(bytes(&frame_of(&state)), bytes(&unlit));
        }

        #[test]
        fn without_a_flash_the_sky_is_the_settings_times_the_radiance_scale_and_linked_fog_is_not()
        {
            let mut state = exterior();
            state.sky.radiance_scale = 2.5;
            let frame = frame_of(&state);
            assert_eq!(
                frame.sky_zenith.truncate(),
                state.sky.zenith * SKY_BRIGHTNESS * 2.5
            );
            assert_eq!(
                frame.sky_horizon.truncate(),
                state.sky.horizon * SKY_BRIGHTNESS * 2.5
            );
            assert_eq!(
                frame.sky_ground.truncate(),
                state.sky.ground * SKY_BRIGHTNESS * 2.5
            );
            assert_eq!(
                frame.fog_near_color.truncate(),
                state.sky.horizon * SKY_BRIGHTNESS
            );
            assert_eq!(
                frame.fog_far_color.truncate(),
                state.sky.horizon * SKY_BRIGHTNESS
            );
        }

        #[test]
        fn the_fog_of_a_weather_keeps_its_colours_whatever_the_sky_scale() {
            let mut state = exterior();
            state.set_exterior_weather(weather_with_fog(own_fog()), FogTime::DAY);
            state.sky.radiance_scale = 2.4;
            let frame = frame_of(&state);
            let Fog::Enabled(fog) = own_fog().resolve(FogTime::DAY) else {
                panic!("the day half has fog");
            };
            assert_eq!(frame.fog_near_color.truncate(), fog.colors.near);
            assert_eq!(frame.fog_far_color.truncate(), fog.colors.far);
            assert_eq!(frame.fog_near_high_color.truncate(), fog.colors.near_high);
            assert_eq!(frame.fog_far_high_color.truncate(), fog.colors.far_high);
        }

        #[test]
        fn the_clock_resolves_the_weathers_fog_for_the_time_of_day() {
            let mut state = exterior();
            state.set_exterior_weather(weather_with_fog(own_fog()), FogTime::DAY);
            let night = FogTime { night_weight: 1.0 };
            state.set_exterior_weather(weather_with_fog(own_fog()), night);
            assert_eq!(state.fog_time(), night);
            assert_eq!(Fog::Enabled(state.fog.params), own_fog().resolve(night));
            assert_eq!(state.fog.params.near, 300.0);
        }

        #[test]
        fn a_strike_lifts_the_frame_while_its_flash_lasts_and_then_leaves_it_as_it_was() {
            let mut state = exterior();
            let unlit = frame_of(&state);
            let mut flashes = LightningFlashes::default();
            let strike = Strike::from_srgb8(0.5, [219, 220, 238]);
            let mut frame_after = |dt_s: f32, strikes: &[Strike]| {
                state.set_lightning(flashes.update(dt_s, strikes));
                frame_of(&state)
            };
            frame_after(0.0, &[strike]);
            let peak = frame_after(ATTACK_S, &[]);
            let lift = strike.colour * strike.intensity * FLASH_SKY_GAIN;
            assert_lifted(peak.sky_zenith, unlit.sky_zenith, lift);
            let over = frame_after(FLASH_LENGTH_S, &[]);
            assert_eq!(bytes(&over), bytes(&unlit));
        }
    }
}
