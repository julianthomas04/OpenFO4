//! The converted main menu music reaches the audio output as audible frames when the game flow
//! enters the menu. The real-install test is ignored by default (needs an installation). Run with
//! `cargo test --test menu_music_audio -- --ignored` with `OPENFO4_INSTALL_DIR` set.

use std::path::Path;
use std::time::Duration;

use openfo4::assets::layout;
use openfo4::assets::{AssetRootEnv, resolve_asset_root};
use openfo4::audio::test_support::{mock_manager_settings, render_buffer, sine_flac, wait_until};
use openfo4::audio::{AudioSystem, MenuMusicSettings};
use openfo4::game_flow::Phase;

const SAMPLE_RATE: u32 = 44_100;
const MAIN_THEME: &str = "\\Data\\Music\\Special\\MUS_MainTheme.xwm";
const AUDIBLE_PEAK: f32 = 0.001;

/// Plays the menu music from the converted audio below `assets` through the mock backend and
/// returns the loudest sample reached.
fn peak_of_menu_music(assets: &Path) -> f32 {
    let (manager_settings, meter) = mock_manager_settings(SAMPLE_RATE);
    let settings = MenuMusicSettings {
        enabled: true,
        file: MAIN_THEME.to_string(),
        fade: Duration::from_millis(10),
        attenuation_millibels: 0,
    };
    let mut audio = AudioSystem::new(manager_settings, assets, settings).unwrap();
    audio.sync(Phase::MainMenu).unwrap();

    let mut peak = 0.0_f32;
    wait_until("the music to become audible", || {
        render_buffer(&mut audio);
        peak = peak.max(meter.take());
        peak > AUDIBLE_PEAK
    });
    peak
}

#[test]
fn a_decoded_synthetic_file_is_audible() {
    let assets = tempfile::tempdir().unwrap();
    let folder = assets.path().join(layout::AUDIO_DIR).join("music/special");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("mus_maintheme.flac"),
        sine_flac(SAMPLE_RATE, 4000),
    )
    .unwrap();

    assert!(peak_of_menu_music(assets.path()) > AUDIBLE_PEAK);
}

#[test]
#[ignore = "needs a converted installation in OPENFO4_INSTALL_DIR"]
fn the_installed_main_menu_theme_decodes_to_audible_frames() {
    let assets = resolve_asset_root(
        &AssetRootEnv::from_process(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    )
    .expect("set OPENFO4_INSTALL_DIR to a converted installation");

    assert!(peak_of_menu_music(&assets) > AUDIBLE_PEAK);
}
