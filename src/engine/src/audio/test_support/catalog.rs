//! Builders for catalog records, so tests can state a track or music type in one line.

use std::collections::HashMap;

use crate::assets::db::music::{
    Condition, MusicCatalog, MusicTrack, MusicTrackKind, MusicType, SoundCategory,
};
use crate::audio::place::PlaceSnapshot;

/// A single track whose form key is its file key.
pub fn single(file: &str, conditions: Vec<Condition>) -> MusicTrack {
    MusicTrack {
        form_key: file.to_string(),
        editor_id: None,
        kind: MusicTrackKind::Single {
            file_key: file.to_string(),
            duration: None,
            fade_out: None,
        },
        conditions,
    }
}

pub fn silent(form_key: &str, seconds: f32) -> MusicTrack {
    MusicTrack {
        form_key: form_key.to_string(),
        editor_id: None,
        kind: MusicTrackKind::Silent { duration: seconds },
        conditions: vec![],
    }
}

pub fn palette(
    form_key: &str,
    layers: &[&[&str]],
    duration: f32,
    fade_out: Option<f32>,
) -> MusicTrack {
    MusicTrack {
        form_key: form_key.to_string(),
        editor_id: None,
        kind: MusicTrackKind::Palette {
            layers: layers
                .iter()
                .map(|layer| layer.iter().map(ToString::to_string).collect())
                .collect(),
            duration,
            fade_out,
        },
        conditions: vec![],
    }
}

pub fn music_type(form_key: &str, flags: u32, tracks: &[&str]) -> MusicType {
    MusicType {
        form_key: form_key.to_string(),
        editor_id: Some(form_key.to_string()),
        flags,
        priority: 80,
        ducking_centidb: 0,
        fade_seconds: 1.0,
        tracks: tracks.iter().map(ToString::to_string).collect(),
    }
}

/// A music type that fades over `fade_seconds`.
pub fn timed_music_type(
    form_key: &str,
    flags: u32,
    fade_seconds: f32,
    tracks: &[&str],
) -> MusicType {
    MusicType {
        fade_seconds,
        ..music_type(form_key, flags, tracks)
    }
}

pub fn track_map(tracks: Vec<(&str, MusicTrack)>) -> HashMap<String, MusicTrack> {
    tracks
        .into_iter()
        .map(|(key, track)| (key.to_string(), track))
        .collect()
}

pub const MASTER_CATEGORY_MULTIPLIER: f32 = 0.8;
pub const MUSIC_CATEGORY_MULTIPLIER: f32 = 0.5;

fn category(
    form_key: &str,
    editor_id: &str,
    parent: Option<&str>,
    multiplier: Option<f32>,
) -> SoundCategory {
    SoundCategory {
        form_key: form_key.to_string(),
        editor_id: Some(editor_id.to_string()),
        parent: parent.map(str::to_string),
        static_multiplier: multiplier,
    }
}

/// The music category below a master category, as the game data chains them.
pub fn sound_categories() -> Vec<SoundCategory> {
    vec![
        category(
            "master",
            "_AudioCategoryMaster",
            None,
            Some(MASTER_CATEGORY_MULTIPLIER),
        ),
        category(
            "music",
            "AudioCategoryMUS",
            Some("master"),
            Some(MUSIC_CATEGORY_MULTIPLIER),
        ),
    ]
}

/// A catalog in which each worldspace plays one music type.
pub fn world_catalog(
    worldspace_music: &[(&str, &str)],
    music_types: Vec<MusicType>,
    tracks: Vec<MusicTrack>,
) -> MusicCatalog {
    MusicCatalog {
        music_types,
        tracks,
        worldspace_music: worldspace_music
            .iter()
            .map(|(worldspace, music_type)| (worldspace.to_string(), music_type.to_string()))
            .collect(),
        sound_categories: sound_categories(),
        ..MusicCatalog::default()
    }
}

/// An exterior place in `worldspace`; places in different worldspaces differ.
pub fn exterior_place(worldspace: &str) -> PlaceSnapshot {
    PlaceSnapshot {
        worldspace: worldspace.to_string(),
        interior_cell: None,
        grid: (0, 0),
        point_beth: (0.0, 0.0),
    }
}
