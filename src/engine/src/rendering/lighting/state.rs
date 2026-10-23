use glam::{UVec2, UVec4, Vec3, Vec4};

use super::ambient::{AmbientCube, EXTERIOR_AMBIENT_FILL};
use super::cell_lighting::CellLighting;
use super::environment::{Environment, InteriorLighting};
use super::exterior_weather::{ExteriorWeather, ImageSpaceHdr};
use super::fog::{Fog, FogParams};
use super::frame_uniform::{FrameUniform, wrapped_time};
use super::interior_catalog::InteriorLightingCatalog;
use super::tonemap_settings::TonemapSettings;
use crate::rendering::camera::Camera;
use crate::rendering::debug_view::DebugView;
use crate::rendering::local_lights::LocalLightSettings;
use crate::rendering::local_lights::cluster_grid::{ClusterDepth, cluster_grid};
use crate::rendering::post::auto_exposure::AutoExposureParams;
use crate::rendering::post::depth_of_field::ImageSpaceDepthOfField;
use crate::rendering::shadows::{CascadeSet, ShadowState, lamps::LampShadowState};
use crate::rendering::water::CameraWater;

const SUN_ANGULAR_RADIUS_DEG: f32 = 0.5;
/// The Creation Kit states the eye adaptation speed per frame at 30 fps.
const ADAPTATION_REFERENCE_FPS: f32 = 30.0;
/// Compensates for the unproven AutoExposureMin/Max semantics; calibrated against the vanilla reference screenshot.
const EXPOSURE_CALIBRATION: f32 = 0.38;

#[derive(Debug, Copy, Clone)]
pub struct SunSettings {
    /// 0 = north (engine -Z), increasing clockwise towards +X (east).
    pub azimuth_deg: f32,
    pub elevation_deg: f32,
    pub color: Vec3,
    pub disc_color: Vec3,
    pub intensity: f32,
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

    /// No direct light once the sun is at or below the horizon.
    pub fn illuminance(&self) -> Vec3 {
        self.above_horizon_scale() * self.color * self.intensity
    }

    pub fn disc_illuminance(&self) -> Vec3 {
        self.above_horizon_scale() * self.disc_color * self.intensity
    }

    fn above_horizon_scale(&self) -> f32 {
        f32::from(self.direction_to_sun().y > 0.0)
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

/// Exterior fog. While linked, both fog colours follow the sky horizon, so
/// distant geometry fades into the sky's horizon band.
#[derive(Debug, Copy, Clone)]
pub struct FogSettings {
    pub params: FogParams,
    pub link_to_horizon: bool,
}

impl FogSettings {
    pub fn fog(&self, horizon_radiance: Vec3) -> Fog {
        let mut params = self.params;
        if self.link_to_horizon {
            params.near_color = horizon_radiance;
            params.far_color = horizon_radiance;
        }
        Fog::Enabled(params)
    }
}

impl Default for FogSettings {
    fn default() -> Self {
        let horizon = SkySettings::default().horizon;
        Self {
            params: FogParams {
                near_color: horizon,
                far_color: horizon,
                near: 200.0,
                far: 12000.0,
                power: 1.0,
                max: 0.8,
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
            middle_gray: self.hdr.middle_gray,
            min: self.hdr.auto_exposure_min,
            max: self.hdr.auto_exposure_max,
            ev_scale: self.ev.exp2() * EXPOSURE_CALIBRATION,
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
/// loaded and in use, the sky cube otherwise, and applies the fill calibration
/// to whichever it picked. Resolved cell lighting and the neutral interior
/// upload their cubes as they are.
fn uploaded_ambient(
    shading: Shading<'_>,
    weather_cube: Option<AmbientCube>,
    use_weather_ambient: bool,
    sky: &SkySettings,
) -> AmbientCube {
    match shading {
        Shading::Exterior => {
            let source = match weather_cube {
                Some(cube) if use_weather_ambient => cube,
                Some(_) | None => sky.ambient_cube(),
            };
            source.scaled(EXTERIOR_AMBIENT_FILL)
        }
        Shading::Cell(cell) => cell.ambient_cube,
        Shading::Neutral => sky.ambient_cube(),
    }
}

/// Everything in the frame uniform that differs between exterior and interior
/// shading, except the ambient cube, which `uploaded_ambient` chooses.
struct SceneTerms {
    to_sun: Vec3,
    sun_illuminance: Vec3,
    /// The HDR sunlight scale already folded into `sun_illuminance` (1 when none).
    sunlight_scale: f32,
    sun_disc_illuminance: Vec3,
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
    SunBelowHorizon,
}

impl ShadowStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled in the panel",
            Self::Interior => "off in interiors",
            Self::SunBelowHorizon => "sun below the horizon",
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
    exterior_weather: Option<ExteriorWeather>,
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
            exterior_weather: None,
            environment: Environment::Exterior,
        }
    }

    /// Takes the weather's sun and sky colours and its image space's light
    /// scales and exposure as the new panel values, and its directional
    /// ambient as the exterior ambient.
    pub fn set_exterior_weather(&mut self, weather: ExteriorWeather) {
        self.sun.intensity = weather.hdr.sunlight_scale;
        self.sky.radiance_scale = weather.hdr.sky_scale;
        self.exposure.hdr = weather.hdr;
        self.sun.color = weather.sun_light;
        self.sun.disc_color = weather.sun_disc;
        self.sky.zenith = weather.sky_upper;
        self.sky.horizon = weather.horizon;
        self.sky.ground = weather.sky_lower;
        match weather.fog {
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
        self.exterior_weather = Some(weather);
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
        } else if self.sun.direction_to_sun().y <= 0.0 {
            ShadowStatus::SunBelowHorizon
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
        let fog = scene.fog.uniform_fields();
        let shadow = self.shadows.uniform_fields(cascades);

        FrameUniform {
            view_proj,
            inv_view_proj: view_proj.inverse(),
            camera_pos: camera.eye.extend(1.0),
            to_sun: scene
                .to_sun
                .extend(SUN_ANGULAR_RADIUS_DEG.to_radians().cos()),
            sun_illuminance: scene.sun_illuminance.extend(scene.sunlight_scale),
            sun_disc_illuminance: scene.sun_disc_illuminance.extend(0.0),
            ambient_cube: self.uploaded_ambient().uniform_faces(),
            sky_zenith: scene.sky_zenith.extend(0.0),
            sky_horizon: scene.sky_horizon.extend(0.0),
            sky_ground: scene.sky_ground.extend(0.0),
            fog_near_color: fog.near_color,
            fog_far_color: fog.far_color,
            fog_params: fog.params,
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
            tonemap: self.tonemap.uniform(),
            texture_feedback,
            time_water: Vec4::new(
                wrapped_time(elapsed_seconds),
                camera_water.surface_height,
                f32::from(u8::from(camera_water.underwater)),
                camera_water.fraction,
            ),
        }
    }

    fn scene_terms(&self) -> SceneTerms {
        match self.shading() {
            Shading::Exterior => self.exterior_terms(),
            Shading::Neutral => SceneTerms {
                sun_illuminance: Vec3::ZERO,
                sun_disc_illuminance: Vec3::ZERO,
                sky_zenith: Vec3::ZERO,
                sky_horizon: Vec3::ZERO,
                sky_ground: Vec3::ZERO,
                fog: Fog::Disabled,
                ..self.exterior_terms()
            },
            Shading::Cell(cell) => SceneTerms {
                to_sun: cell.directional.map_or(Vec3::Y, |light| light.to_light),
                sun_illuminance: cell
                    .directional
                    .map_or(Vec3::ZERO, |light| light.color * light.fade.max(0.0)),
                sunlight_scale: 1.0,
                sun_disc_illuminance: Vec3::ZERO,
                sky_zenith: Vec3::ZERO,
                sky_horizon: Vec3::ZERO,
                sky_ground: Vec3::ZERO,
                fog: cell.fog,
            },
        }
    }

    fn uploaded_ambient(&self) -> AmbientCube {
        uploaded_ambient(
            self.shading(),
            self.exterior_weather
                .as_ref()
                .map(|weather| weather.ambient_cube),
            self.use_weather_ambient,
            &self.sky,
        )
    }

    fn exterior_terms(&self) -> SceneTerms {
        SceneTerms {
            to_sun: self.sun.direction_to_sun(),
            sun_illuminance: self.sun.illuminance(),
            sunlight_scale: self.sun.intensity,
            sun_disc_illuminance: self.sun.disc_illuminance(),
            sky_zenith: self.sky.zenith * self.sky.radiance_scale,
            sky_horizon: self.sky.horizon * self.sky.radiance_scale,
            sky_ground: self.sky.ground * self.sky.radiance_scale,
            fog: self.fog.fog(self.sky.horizon * self.sky.radiance_scale),
        }
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
    use super::*;
    use crate::rendering::game_settings::GameSettings;
    use crate::rendering::test_support::ini_with;

    fn sun(azimuth_deg: f32, elevation_deg: f32) -> SunSettings {
        SunSettings {
            azimuth_deg,
            elevation_deg,
            color: Vec3::new(1.0, 0.5, 0.25),
            disc_color: Vec3::new(2.0, 2.0, 2.0),
            intensity: 3.0,
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
        assert_eq!(sun.disc_illuminance(), Vec3::splat(6.0));
    }

    #[test]
    fn a_sun_at_or_below_the_horizon_gives_no_direct_light() {
        for elevation in [0.0, -0.1, -45.0] {
            let sun = sun(0.0, elevation);
            assert_eq!(sun.illuminance(), Vec3::ZERO, "{elevation}");
            assert_eq!(sun.disc_illuminance(), Vec3::ZERO, "{elevation}");
        }
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
        AmbientCube(cube.0.map(|face| face * 1.5))
    }

    fn weather_cube() -> AmbientCube {
        distinct_cube(0.1)
    }

    fn cell_cube() -> AmbientCube {
        distinct_cube(0.3)
    }

    fn weather() -> ExteriorWeather {
        ExteriorWeather {
            form_key: "TestWeather".to_string(),
            editor_id: None,
            sun_light: Vec3::ONE,
            sun_disc: Vec3::ONE,
            sky_upper: Vec3::new(0.1, 0.2, 0.9),
            sky_lower: Vec3::new(0.3, 0.2, 0.1),
            horizon: Vec3::new(0.6, 0.7, 0.8),
            ambient_cube: weather_cube(),
            ambient_specular: Vec3::ZERO,
            hdr: ImageSpaceHdr::default(),
            depth_of_field: None,
            fog: None,
        }
    }

    fn resolved_cell() -> InteriorLighting {
        InteriorLighting::Cell(ResolvedLighting {
            lighting: CellLighting {
                ambient_color: Vec3::splat(0.2),
                ambient_cube: cell_cube(),
                directional: None,
                fog: Fog::Disabled,
                clip_distance: None,
            },
            template: TemplateStatus::NoInheritance,
        })
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

    fn lighting_state() -> LightingState {
        let settings = GameSettings::from_ini(&ini_with("")).unwrap();
        LightingState::new(
            ShadowState::new(settings.shadows),
            LampShadowState::new(settings.lamp_shadows),
            LocalLightSettings::new(settings.light_fade),
        )
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
            state.set_exterior_weather(weather());
        }
        state.environment = environment;
        state.use_weather_ambient = use_weather_ambient;
        state.use_cell_lighting = use_cell_lighting;
        state
    }

    fn uploaded_cube(state: &LightingState) -> [Vec4; AMBIENT_CUBE_FACES] {
        let camera = Camera {
            eye: Vec3::new(0.0, 10.0, 5.0),
            target: Vec3::ZERO,
            up: Vec3::Y,
            aspect: 1.0,
            fovy: 1.0,
            znear: 0.5,
            zfar: 1000.0,
        };
        state
            .frame_uniform(&FrameInputs {
                camera: &camera,
                cascades: None,
                viewport: UVec2::new(64, 64),
                max_anisotropy: 1,
                texture_feedback: UVec4::ZERO,
                elapsed_seconds: 0.0,
                camera_water: CameraWater::default(),
            })
            .ambient_cube
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
    fn linked_fog_takes_the_horizon_radiance_for_both_colours() {
        let settings = FogSettings::default();
        let Fog::Enabled(params) = settings.fog(Vec3::new(9.0, 8.0, 7.0)) else {
            panic!("exterior fog is always enabled");
        };
        assert_eq!(params.near_color, Vec3::new(9.0, 8.0, 7.0));
        assert_eq!(params.far_color, Vec3::new(9.0, 8.0, 7.0));
        assert_eq!(params.near, settings.params.near);
    }

    #[test]
    fn unlinked_fog_keeps_its_own_colours() {
        let settings = FogSettings {
            link_to_horizon: false,
            params: FogParams {
                near_color: Vec3::new(0.3, 0.3, 0.3),
                ..FogSettings::default().params
            },
        };
        let Fog::Enabled(params) = settings.fog(Vec3::splat(9.0)) else {
            panic!("exterior fog is always enabled");
        };
        assert_eq!(params.near_color, Vec3::new(0.3, 0.3, 0.3));
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
        assert_eq!(exposure(0.0, true, 3.0).gpu_params(0.0).adaptation, 0.0);
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
        assert_eq!((on.min, on.max, on.middle_gray), (1.0, 4.0, 0.18));
    }
}
