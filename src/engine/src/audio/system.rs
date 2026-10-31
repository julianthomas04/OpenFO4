use std::path::{Path, PathBuf};
use std::time::Duration;

use kira::sound::PlaybackState;
use kira::{AudioManager, AudioManagerSettings, DefaultBackend};

use super::backend::AudioBackend;
use super::backend_errors::BackendErrorReporter;
use super::conditions::GameTime;
use super::error::AudioError;
use super::footsteps::Footsteps;
use super::music::{MusicCommand, MusicDirector};
use super::music_output::{MusicOutput, Playing, SoundSettings, open_music_file};
use super::place::PlaceSnapshot;
use super::settings::MenuMusicSettings;
use super::sfx_output::SfxOutput;
use super::weather::{WeatherAudio, WeatherAudioInput, WeatherPlay, report_disabled};
use super::weather_output::WeatherOutput;
use super::world_music::WorldMusic;
use super::world_playback::WorldPlayback;
use crate::assets::layout;
use crate::footsteps::FootstepEvent;
use crate::game_flow::Phase;

/// What the audio needs to know once per frame.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioFrame {
    pub phase: Phase,
    pub dt: Duration,
    /// The game clock, for the music track conditions.
    pub time: GameTime,
    /// The player's place when it was polled this frame.
    pub place: Option<PlaceSnapshot>,
    /// The weather the player is under; `None` where there is none, which leaves the weather
    /// silent.
    pub weather: Option<WeatherAudioInput>,
}

/// A frame without a place, time or weather, for tests that only follow the phase.
#[cfg(any(test, feature = "test-support"))]
impl From<Phase> for AudioFrame {
    fn from(phase: Phase) -> Self {
        Self {
            phase,
            dt: Duration::ZERO,
            time: GameTime::at_hour(0.0),
            place: None,
            weather: None,
        }
    }
}

/// The audio output and what plays on it. Generic over kira's backend so tests can drive a mock.
pub struct AudioSystem<B: AudioBackend = DefaultBackend> {
    manager: AudioManager<B>,
    output: MusicOutput,
    sfx: SfxOutput,
    weather_output: WeatherOutput,
    footsteps: Option<Footsteps>,
    /// `None` while there are no weather sounds, and for good once their data failed.
    weather: Option<WeatherAudio>,
    /// The phase of the latest frame; footsteps and weather play only in a gameplay phase.
    phase: Phase,
    menu: Option<Playing>,
    world: Option<WorldPlayback>,
    director: MusicDirector,
    settings: MenuMusicSettings,
    audio_dir: PathBuf,
    menu_path: PathBuf,
    /// The time the frames have added up to; plan timers run on it.
    clock: Duration,
    backend_errors: BackendErrorReporter,
}

impl AudioSystem {
    /// Opens the default audio device. A device that cannot be opened is logged and leaves the
    /// game silent (`None`); a music file that is missing or cannot be decoded is a broken
    /// install and an error.
    pub fn open_default(
        asset_root: &Path,
        settings: MenuMusicSettings,
    ) -> Result<Option<Self>, AudioError> {
        match Self::new(AudioManagerSettings::default(), asset_root, settings) {
            Err(AudioError::Device(cause)) => {
                tracing::error!("audio is disabled, cannot open the audio device: {cause}");
                Ok(None)
            }
            other => other.map(Some),
        }
    }
}

impl<B: AudioBackend> AudioSystem<B> {
    pub fn new(
        manager_settings: AudioManagerSettings<B>,
        asset_root: &Path,
        settings: MenuMusicSettings,
    ) -> Result<Self, AudioError> {
        let audio_dir = PathBuf::from(layout::audio_dir(&asset_root.to_string_lossy()));
        let menu_path = audio_dir.join(layout::audio_key(&settings.file)?);
        if settings.enabled {
            open_music_file(&menu_path, true)?;
        }
        let mut manager = AudioManager::new(manager_settings)
            .map_err(|error| AudioError::Device(B::describe_setup_error(&error)))?;
        let output = MusicOutput::new(&mut manager)?;
        let sfx = SfxOutput::new(&mut manager, audio_dir.clone())?;
        let weather_output = WeatherOutput::new(&mut manager, audio_dir.clone())?;
        Ok(Self {
            manager,
            output,
            sfx,
            weather_output,
            footsteps: None,
            weather: None,
            phase: Phase::MainMenu,
            menu: None,
            world: None,
            director: MusicDirector::new(settings.enabled),
            settings,
            audio_dir,
            menu_path,
            clock: Duration::ZERO,
            backend_errors: BackendErrorReporter::default(),
        })
    }

    /// Lets the world music play from `Phase::Revealing` on.
    pub fn set_world_music(&mut self, music: WorldMusic) {
        self.world = Some(WorldPlayback::new(music, self.audio_dir.clone()));
    }

    /// Lets `play_footstep` play the footstep sounds.
    pub fn set_footsteps(&mut self, footsteps: Footsteps) {
        self.footsteps = Some(footsteps);
    }

    /// Lets the frames' weather play as loops and thunder; replaces the weather sounds set before.
    pub fn set_weather(&mut self, weather: WeatherAudio) {
        self.weather = Some(weather);
    }

    /// Plays the sounds of one footstep; `material` is the hash of the surface's material (see
    /// `audio::footsteps`). The step is dropped outside the gameplay phases (`Revealing` and
    /// `InGame`, where the world music plays too) and while no footsteps are set. An error is
    /// fatal: footstep data that cannot be used, or a file that cannot be decoded or played.
    pub fn play_footstep(
        &mut self,
        event: FootstepEvent,
        material: Option<u32>,
    ) -> Result<(), AudioError> {
        if !in_world(self.phase) {
            return Ok(());
        }
        let Some(footsteps) = &mut self.footsteps else {
            return Ok(());
        };
        for layer in footsteps.step(event, material)? {
            self.sfx.play_layer(&layer)?;
        }
        Ok(())
    }

    /// Applies what the game flow's phase, the player's place and the weather ask of the audio;
    /// call once per frame. Weather sounds play in the gameplay phases only: in any other phase, or
    /// without a weather, the loops fade out and the thunder on its way is dropped. An error is
    /// fatal: a music file cannot be played or turned out damaged, or the music data is unusable.
    /// The weather sounds never are: a weather sound file that fails is logged and left out, and
    /// weather sound data that fails turns the weather sounds off (see `WeatherOutput`).
    pub fn sync(&mut self, frame: impl Into<AudioFrame>) -> Result<(), AudioError> {
        let frame = frame.into();
        self.clock += frame.dt;
        self.phase = frame.phase;
        self.report_backend_errors();
        self.check_music_streams()?;
        for command in self.director.update(frame.phase) {
            self.apply(command)?;
        }
        if let Some(world) = &mut self.world {
            world.sync(
                &mut self.output,
                self.clock,
                frame.time,
                frame.place.as_ref(),
            )?;
        }
        self.sync_weather(frame.weather.as_ref(), frame.time);
        Ok(())
    }

    pub fn music_state(&self) -> Option<PlaybackState> {
        self.menu.as_ref().map(Playing::state)
    }

    /// The playback position of the menu music in seconds.
    pub fn music_position(&self) -> Option<f64> {
        self.menu.as_ref().map(Playing::position)
    }

    pub fn world_music(&self) -> Option<&WorldMusic> {
        self.world.as_ref().map(WorldPlayback::music)
    }

    /// How many world music sounds are playing or fading in the current plan.
    pub fn world_sound_count(&self) -> usize {
        self.world.as_ref().map_or(0, WorldPlayback::sound_count)
    }

    pub fn manager_mut(&mut self) -> &mut AudioManager<B> {
        &mut self.manager
    }

    #[cfg(test)]
    pub(super) fn world_sound_file_names(&self) -> Vec<String> {
        self.world
            .as_ref()
            .map_or_else(Vec::new, WorldPlayback::sound_file_names)
    }

    #[cfg(test)]
    pub(super) fn decoded_sfx_count(&self) -> usize {
        self.sfx.decoded_count()
    }

    #[cfg(test)]
    pub(super) fn playing_loop_files(&self) -> Vec<&str> {
        self.weather_output.playing_files()
    }

    /// The weather sounds that are playing or fading out.
    #[cfg(test)]
    pub(super) fn weather_sound_count(&self) -> usize {
        self.weather_output.sound_count()
    }

    #[cfg(test)]
    pub(super) fn retiring_music_count(&self) -> usize {
        self.output.retiring_count()
    }

    fn apply(&mut self, command: MusicCommand) -> Result<(), AudioError> {
        match command {
            MusicCommand::StartMenu => self.start_menu(),
            MusicCommand::StopMenu => {
                self.fade_out_menu();
                Ok(())
            }
            MusicCommand::StartWorld => {
                if let Some(world) = &mut self.world {
                    world.start();
                }
                Ok(())
            }
            MusicCommand::StopWorld => {
                if let Some(world) = &mut self.world {
                    world.stop(&mut self.output, self.settings.fade);
                }
                Ok(())
            }
        }
    }

    fn start_menu(&mut self) -> Result<(), AudioError> {
        let settings = SoundSettings {
            looped: true,
            volume: self.settings.attenuation(),
            fade_in: self.settings.fade,
        };
        let sound = self.output.play(&self.menu_path, settings)?;
        self.output.retire(self.menu.replace(sound));
        Ok(())
    }

    fn fade_out_menu(&mut self) {
        let fade = self.settings.fade;
        if let Some(menu) = &mut self.menu {
            menu.stop(fade);
        }
    }

    fn sync_weather(&mut self, input: Option<&WeatherAudioInput>, time: GameTime) {
        self.weather_output.check();
        let play = self.weather_play(input, time);
        self.weather_output.follow(&play.loops);
        self.weather_output.play_thunder(&play.thunder);
    }

    /// What the weather sounds play now. Weather sound data that cannot be played turns the
    /// weather sounds off for the rest of the game, after one error in the log.
    fn weather_play(&mut self, input: Option<&WeatherAudioInput>, time: GameTime) -> WeatherPlay {
        let Some(weather) = &mut self.weather else {
            return WeatherPlay::default();
        };
        let input = input.filter(|_| in_world(self.phase));
        match weather.update(input, time, self.clock) {
            Ok(play) => play,
            Err(error) => {
                report_disabled(&error);
                self.weather = None;
                WeatherPlay::default()
            }
        }
    }

    fn check_music_streams(&mut self) -> Result<(), AudioError> {
        let world = self.world.iter_mut().flat_map(WorldPlayback::sounds_mut);
        self.output.check(self.menu.iter_mut().chain(world))
    }

    fn report_backend_errors(&mut self) {
        let backend = self.manager.backend_mut();
        let errors: Vec<_> = std::iter::from_fn(|| backend.pop_stream_error()).collect();
        let discarded = backend.stream_errors_discarded();
        for line in self.backend_errors.lines(errors, discarded) {
            tracing::error!("{line}");
        }
    }
}

/// The phases the world plays in: its music, its footsteps and its weather.
fn in_world(phase: Phase) -> bool {
    matches!(phase, Phase::Revealing | Phase::InGame)
}
