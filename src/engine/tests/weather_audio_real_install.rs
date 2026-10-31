//! The weather sounds of a converted installation: every weather of the Commonwealth climate asks
//! for loops and thunder whose files exist.
//!
//! Ignored by default (needs an installation converted with the weather tables, the sound tables
//! and the audio). Run with
//! `cargo test --test weather_audio_real_install -- --ignored --nocapture`.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use openfo4::assets::db::DbClient;
use openfo4::assets::layout::audio_dir;
use openfo4::audio::conditions::GameTime;
use openfo4::audio::weather::{
    ThunderStrike, WeatherAudio, WeatherAudioInput, WeatherPlay, WeatherSide, WeatherSoundTable,
    load_weather_audio,
};
use openfo4::game_flow::spawn_locations::COMMONWEALTH;
use openfo4::rendering::weather::WeatherLibrary;
use openfo4::weather::{SoundKind, TransitionSide, WeatherId};

const NOON: f32 = 12.0;
const MIDWAY: f32 = 0.5;

fn database_path() -> String {
    common::database_path().to_str().unwrap().to_string()
}

fn audio_folder() -> PathBuf {
    PathBuf::from(audio_dir(common::asset_root().to_str().unwrap()))
}

fn library() -> WeatherLibrary {
    let db = DbClient::new(&database_path()).unwrap();
    WeatherLibrary::load(&db, COMMONWEALTH).unwrap()
}

fn weather_audio(library: &WeatherLibrary) -> WeatherAudio {
    let table = WeatherSoundTable::new(
        library
            .weathers()
            .iter()
            .map(|weather| weather.sounds.clone())
            .collect(),
    );
    load_weather_audio(&database_path(), table, &audio_folder()).unwrap()
}

fn ids(library: &WeatherLibrary) -> impl Iterator<Item = WeatherId> {
    (0..library.weathers().len()).map(WeatherId::new)
}

fn side(library: &WeatherLibrary, weather: WeatherId, transition: TransitionSide) -> WeatherSide {
    let def = library.get(weather).unwrap();
    WeatherSide::new(weather, transition, def.precipitation_fade, def.wind.speed)
}

fn outdoors(sides: Vec<WeatherSide>, strikes: Vec<ThunderStrike>) -> WeatherAudioInput {
    WeatherAudioInput {
        sides,
        strikes,
        outdoor: true,
    }
}

fn update(audio: &mut WeatherAudio, input: &WeatherAudioInput) -> WeatherPlay {
    audio
        .update(Some(input), GameTime::at_hour(NOON), Duration::ZERO)
        .unwrap()
}

fn assert_files_exist(what: &str, files: impl IntoIterator<Item = String>) {
    let audio = audio_folder();
    for file_key in files {
        let path = audio.join(&file_key);
        assert!(path.exists(), "{what}: {} is missing", path.display());
    }
}

#[test]
#[ignore = "needs a converted installation"]
fn every_weather_of_the_commonwealth_opens_and_asks_for_loops_whose_files_exist() {
    let library = library();
    let mut audio = weather_audio(&library);

    assert!(
        audio.missing_files().is_empty(),
        "weather sound files are missing: {:?}",
        audio.missing_files()
    );
    for weather in ids(&library) {
        let def = library.get(weather).unwrap();
        let input = outdoors(
            vec![side(&library, weather, TransitionSide::Settled)],
            vec![],
        );

        let play = update(&mut audio, &input);

        println!(
            "{} ({}): {:?}",
            def.editor_id.as_deref().unwrap_or("<no editor id>"),
            def.form_key,
            play.loops
        );
        let has_default_sound = def
            .sounds
            .iter()
            .any(|sound| sound.kind == SoundKind::Default);
        assert!(
            !has_default_sound || !play.loops.is_empty(),
            "{}: a default sound but no loop",
            def.form_key
        );
        for target in &play.loops {
            assert!(target.gain_db.is_finite(), "{}: {target:?}", def.form_key);
        }
        assert_files_exist(
            &def.form_key,
            play.loops.into_iter().map(|target| target.file_key),
        );
    }
}

#[test]
#[ignore = "needs a converted installation"]
fn every_storm_of_the_commonwealth_plays_thunder_layers_whose_files_exist() {
    let library = library();
    let mut audio = weather_audio(&library);
    let mut storms = 0;
    for weather in ids(&library) {
        let def = library.get(weather).unwrap();
        if !def
            .sounds
            .iter()
            .any(|sound| sound.kind == SoundKind::Thunder)
        {
            continue;
        }
        storms += 1;
        let strike = ThunderStrike {
            weather,
            delay_s: 0.0,
            intensity: 1.0,
        };
        let input = outdoors(
            vec![side(&library, weather, TransitionSide::Settled)],
            vec![strike],
        );

        let play = update(&mut audio, &input);

        println!(
            "{} ({}): {:?}",
            def.editor_id.as_deref().unwrap_or("<no editor id>"),
            def.form_key,
            play.thunder
        );
        assert!(!play.thunder.is_empty(), "{}: silent thunder", def.form_key);
        for layer in &play.thunder {
            assert!(layer.mix.gain_db.is_finite(), "{}: {layer:?}", def.form_key);
        }
        assert_files_exist(
            &def.form_key,
            play.thunder.into_iter().map(|layer| layer.file_key),
        );
    }
    assert!(storms > 0, "the climate has no storm");
}

#[test]
#[ignore = "needs a converted installation"]
fn every_transition_between_commonwealth_weathers_asks_for_audible_loops() {
    let library = library();
    let mut audio = weather_audio(&library);
    for to in ids(&library) {
        for from in ids(&library).filter(|&from| from != to) {
            let input = outdoors(
                vec![
                    side(&library, to, TransitionSide::Incoming { progress: MIDWAY }),
                    side(
                        &library,
                        from,
                        TransitionSide::Outgoing { progress: MIDWAY },
                    ),
                ],
                vec![],
            );

            let play = update(&mut audio, &input);

            for target in &play.loops {
                assert!(
                    target.gain_db.is_finite(),
                    "{} to {}: {target:?}",
                    library.get(from).unwrap().form_key,
                    library.get(to).unwrap().form_key
                );
            }
        }
    }
}
