use crate::assets::ini::IniError;
use crate::assets::pipboy_art::ArtManifestError;
use crate::audio::AudioError;
use crate::game_flow::spawn_locations::SpawnLocationError;
use crate::rendering::menu::MenuError;
use crate::rendering::pipboy::perk_grid::PerkGridError;
use crate::rendering::pipboy::world_map_texture::WorldMapTextureError;
use crate::rendering::shaders::startup::ShaderStartupError;
use crate::rendering::ui_fonts::UiFontError;
use crate::rendering::version_overlay::VersionOverlayError;
use crate::spawn_cell::SpawnCellError;
use crate::ui_layout::UiLayoutError;

#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error("cannot create the window: {0}")]
    Window(#[from] winit::error::OsError),
    #[error("cannot create the window surface: {0}")]
    Surface(#[from] wgpu::CreateSurfaceError),
    #[error("no suitable GPU adapter: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    #[error("cannot create the GPU device: {0:#}")]
    Device(anyhow::Error),
    #[error("shader startup failed: {0}")]
    Shaders(#[from] ShaderStartupError),
    #[error("invalid game settings: {0}")]
    Settings(#[from] IniError),
    #[error("spawn override: {0}")]
    SpawnOverride(#[from] SpawnCellError),
    #[error("spawn locations: {0}")]
    SpawnLocations(#[from] SpawnLocationError),
    #[error("user interface fonts: {0}")]
    Fonts(#[from] UiFontError),
    #[error("main menu: {0}")]
    Menu(#[from] MenuError),
    #[error("version overlay: {0}")]
    VersionOverlay(#[from] VersionOverlayError),
    #[error("audio: {0}")]
    Audio(#[from] AudioError),
    #[error("UI layout: {0}")]
    UiLayout(#[from] UiLayoutError),
    #[error("cannot open the asset database: {0:#}")]
    Database(anyhow::Error),
    #[error("cannot start the world streamer: {0:#}")]
    Streamer(anyhow::Error),
    #[error("footsteps: {0:#}")]
    Footsteps(anyhow::Error),
    #[error("cannot create the HUD: {0:#}")]
    Hud(anyhow::Error),
    #[error("cannot create the Pip-Boy: {0:#}")]
    Pipboy(anyhow::Error),
    #[error("cannot start the world clock: {0:#}")]
    WorldClock(anyhow::Error),
    #[error("cannot load the sky's game settings: {0:#}")]
    SkySettings(anyhow::Error),
    #[error("Pip-Boy world map: {0}")]
    PipboyWorldMap(#[from] WorldMapTextureError),
    #[error("Pip-Boy map marker art: {0}")]
    PipboyMarkerArt(#[from] ArtManifestError),
    #[error("Pip-Boy perk chart poster: {0}")]
    PipboyPerkGrid(#[from] PerkGridError),
    #[error("cannot start the startup worker thread: {0}")]
    WorkerSpawn(std::io::Error),
    #[error("the startup worker panicked: {0}")]
    WorkerPanicked(String),
    #[error("the startup worker ended without a result")]
    WorkerDisconnected,
    #[error("startup was cancelled")]
    Cancelled,
}

impl StartupError {
    /// Whether the user ended startup on purpose, so nothing went wrong.
    pub fn is_cancellation(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}
