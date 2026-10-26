//! Ambient world music: which music type plays at the player's place and what it plays next.
//!
//! Built once at startup from the catalog. Startup fails loudly when a track that can play uses an
//! unsupported condition, a palette nests another palette, the audio folder is missing, a music
//! file that is present cannot be opened, or the music volume cannot be read from the sound
//! categories; a music file that is absent is logged once and left out of every pick.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use kira::Decibels;
use kira::sound::FromFileError;

use super::conditions::{self, ConditionError, FreshWorldContext, GameTime, global_entries};
use super::decibels::amplitude_to_db;
use super::file_probe::{ProbeError, probe_files};
use super::music_output::open_stream;
use super::music_stack::pick;
use super::place::{Candidate, PlaceError, PlaceIndex, PlaceSnapshot};
use super::scheduler::{LOG_TARGET, PlayPlan, SchedulerError, TrackScheduler};
use crate::assets::db::music::{
    MusicCatalog, MusicTrack, MusicTrackKind, MusicType, SoundCategory,
};
use crate::random::SplitMix64;

pub const MUSIC_CATEGORY_EDITOR_ID: &str = "AudioCategoryMUS";

#[derive(Debug, thiserror::Error)]
pub enum WorldMusicError {
    #[error(transparent)]
    Place(#[from] PlaceError),
    #[error(transparent)]
    Scheduler(#[from] SchedulerError),
    #[error("music track {track} cannot be played: {source}")]
    Condition {
        track: String,
        source: ConditionError,
    },
    #[error("cannot read the music data: {0:#}")]
    Catalog(anyhow::Error),
    #[error("music type {music_type} has the unusable fade time {seconds}")]
    Fade { music_type: String, seconds: f32 },
    #[error(
        "the audio folder {0} does not exist; the installer converts the game's audio into it, so run the installer again"
    )]
    AudioDirectoryMissing(PathBuf),
    #[error("cannot check the music file {path}: {source}")]
    FileCheck {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "music {path} cannot be opened or decoded; the installer converts it \
         from the game, so run the installer again: {source}"
    )]
    MusicFile {
        path: PathBuf,
        source: FromFileError,
    },
    #[error("the sound category {MUSIC_CATEGORY_EDITOR_ID} does not exist")]
    MusicCategoryMissing,
    #[error("the sound category {0} has no volume multiplier")]
    CategoryWithoutMultiplier(String),
    #[error("the sound category {category} names the missing parent {parent}")]
    CategoryParentMissing { category: String, parent: String },
    #[error("the sound category {0} is its own ancestor")]
    CategoryCycle(String),
}

impl From<ProbeError> for WorldMusicError {
    fn from(error: ProbeError) -> Self {
        match error {
            ProbeError::DirectoryMissing(path) => Self::AudioDirectoryMissing(path),
            ProbeError::Check { path, source } => Self::FileCheck { path, source },
            ProbeError::Open { path, source, .. } => Self::MusicFile { path, source },
        }
    }
}

/// A change of the playing music type. `plan` is the new type's first pick, if it has one.
///
/// An abrupt change crossfades: the old sounds fade out over `fade` while `plan` fades in over
/// the same time. Any other change waits for the sounds that play to end, then starts `plan`; a
/// start from silence fades in over `fade`.
#[derive(Debug, Clone, PartialEq)]
pub struct MusicChange {
    pub fade: Duration,
    pub abrupt: bool,
    pub plan: Option<PlayPlan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    Unset,
    NoCandidate,
    Type(String),
}

type MusicTypes = HashMap<String, MusicType>;
type Tracks = HashMap<String, MusicTrack>;

pub struct WorldMusic {
    index: PlaceIndex,
    music_types: MusicTypes,
    tracks: Tracks,
    scheduler: TrackScheduler,
    context: FreshWorldContext,
    rng: SplitMix64,
    gain: f32,
    last_place: Option<PlaceSnapshot>,
    selection: Selection,
    plan: Option<PlayPlan>,
}

impl WorldMusic {
    /// `audio_dir` is where the converted music files live.
    pub fn new(
        catalog: MusicCatalog,
        audio_dir: &Path,
        seed: u64,
    ) -> Result<Self, WorldMusicError> {
        let gain = music_category_gain(&catalog.sound_categories)?;
        let index = PlaceIndex::from_catalog(&catalog)?;
        let context = FreshWorldContext::new(0.0, global_entries(&catalog));
        let reachable_types = reachable_type_keys(&catalog);
        let music_types = keyed(catalog.music_types, |music_type| {
            music_type.form_key.clone()
        });
        let tracks = keyed(catalog.tracks, |track| track.form_key.clone());
        let reachable = reachable_tracks(&reachable_types, &music_types, &tracks)?;
        validate_reachable(
            &reachable_types,
            &music_types,
            &reachable,
            &tracks,
            &context,
        )?;
        let missing_files = probe_files(
            "music file",
            single_file_keys(&reachable),
            audio_dir,
            |path| open_stream(path, false).map(|_| ()),
        )?;
        Ok(Self {
            index,
            music_types,
            tracks,
            scheduler: TrackScheduler::new(missing_files),
            context,
            rng: SplitMix64::new(seed),
            gain,
            last_place: None,
            selection: Selection::Unset,
            plan: None,
        })
    }

    /// The music volume from the sound category chain, as a level for every world music sound.
    pub fn volume(&self) -> Decibels {
        Decibels(amplitude_to_db(self.gain))
    }

    pub fn missing_files(&self) -> &HashSet<String> {
        self.scheduler.missing_files()
    }

    pub fn music_type(&self, form_key: &str) -> Option<&MusicType> {
        self.music_types.get(form_key)
    }

    /// The music type that plays now; `None` before the first place and when nothing applies.
    pub fn current_music_type(&self) -> Option<&MusicType> {
        match &self.selection {
            Selection::Type(key) => self.music_types.get(key),
            Selection::Unset | Selection::NoCandidate => None,
        }
    }

    pub fn current_plan(&self) -> Option<&PlayPlan> {
        self.plan.as_ref()
    }

    /// Every music type the place offers, and the one the priority stack picks.
    pub fn candidates(
        &self,
        place: &PlaceSnapshot,
    ) -> Result<(Vec<Candidate>, Option<Candidate>), WorldMusicError> {
        let candidates = self.index.resolve(place)?;
        let picked = pick(&candidates);
        Ok((candidates, picked))
    }

    fn type_name(&self, form_key: &str) -> String {
        self.music_types
            .get(form_key)
            .and_then(|music_type| music_type.editor_id.as_deref())
            .unwrap_or(form_key)
            .to_string()
    }

    fn selection_name(&self, selection: &Selection) -> String {
        match selection {
            Selection::Unset => "(none yet)".to_string(),
            Selection::NoCandidate => "(no music type)".to_string(),
            Selection::Type(key) => self.type_name(key),
        }
    }

    /// Forgets the place and what plays, so the next place starts the music afresh.
    pub fn reset(&mut self) {
        self.last_place = None;
        self.selection = Selection::Unset;
        self.plan = None;
    }

    /// `Some` when the music type at `place` differs from the one that plays. A snapshot equal to
    /// the previous one is not resolved again; the snapshot includes the exact point, so a moving
    /// player is resolved on every poll, and only a changed music type yields a change.
    pub fn update_place(
        &mut self,
        place: &PlaceSnapshot,
        time: GameTime,
    ) -> Result<Option<MusicChange>, WorldMusicError> {
        if self.last_place.as_ref() == Some(place) {
            return Ok(None);
        }
        self.last_place = Some(place.clone());
        let (candidates, picked) = self.candidates(place)?;
        let selection = picked.map_or(Selection::NoCandidate, |candidate| {
            Selection::Type(candidate.music_type)
        });
        if selection == self.selection {
            return Ok(None);
        }
        let outgoing_fade = self.current_music_type().map(fade_of).transpose()?;
        tracing::debug!(
            target: LOG_TARGET,
            from = %self.selection_name(&self.selection),
            to = %self.selection_name(&selection),
            candidates = ?candidates
                .iter()
                .map(|candidate| format!(
                    "{} ({:?}, {})",
                    self.type_name(&candidate.music_type),
                    candidate.source,
                    candidate.priority
                ))
                .collect::<Vec<_>>(),
            point = ?place.point_beth,
            "music type changed"
        );
        self.selection = selection;
        self.plan = None;
        let Selection::Type(key) = &self.selection else {
            return Ok(outgoing_fade.map(|fade| MusicChange {
                fade,
                abrupt: false,
                plan: None,
            }));
        };
        self.context.set_time(time);
        let music_type = lookup_type(&self.music_types, key)?;
        let fade = fade_of(music_type)?;
        self.plan =
            self.scheduler
                .next_plan(music_type, &self.tracks, &self.context, &mut self.rng)?;
        Ok(Some(MusicChange {
            fade,
            abrupt: self.scheduler.is_abrupt(music_type),
            plan: self.plan.clone(),
        }))
    }

    /// The plan after the previous one ended. `None` is silence: the type does not cycle, or
    /// nothing is eligible.
    pub fn next_plan(&mut self, time: GameTime) -> Result<Option<PlayPlan>, WorldMusicError> {
        self.plan = None;
        let Selection::Type(key) = &self.selection else {
            return Ok(None);
        };
        let music_type = lookup_type(&self.music_types, key)?;
        if !self.scheduler.cycles(music_type) {
            tracing::debug!(
                target: LOG_TARGET,
                music_type = %self.type_name(key),
                "music type does not cycle: silence until the type changes"
            );
            return Ok(None);
        }
        self.context.set_time(time);
        self.plan =
            self.scheduler
                .next_plan(music_type, &self.tracks, &self.context, &mut self.rng)?;
        Ok(self.plan.clone())
    }
}

fn keyed<T>(items: Vec<T>, key: impl Fn(&T) -> String) -> HashMap<String, T> {
    items.into_iter().map(|item| (key(&item), item)).collect()
}

fn lookup_type<'a>(music_types: &'a MusicTypes, key: &str) -> Result<&'a MusicType, PlaceError> {
    music_types
        .get(key)
        .ok_or_else(|| PlaceError::UnknownMusicType(key.to_string()))
}

fn lookup_track<'a>(tracks: &'a Tracks, key: &str) -> Result<&'a MusicTrack, SchedulerError> {
    tracks
        .get(key)
        .ok_or_else(|| SchedulerError::UnknownTrack(key.to_string()))
}

fn fade_of(music_type: &MusicType) -> Result<Duration, WorldMusicError> {
    Duration::try_from_secs_f32(music_type.fade_seconds).map_err(|_| WorldMusicError::Fade {
        music_type: music_type.form_key.clone(),
        seconds: music_type.fade_seconds,
    })
}

/// The multiplier of the music category times those of its ancestors up to the root.
fn music_category_gain(categories: &[SoundCategory]) -> Result<f32, WorldMusicError> {
    let by_key: HashMap<&str, &SoundCategory> = categories
        .iter()
        .map(|category| (category.form_key.as_str(), category))
        .collect();
    let mut category = categories
        .iter()
        .find(|category| category.editor_id.as_deref() == Some(MUSIC_CATEGORY_EDITOR_ID))
        .ok_or(WorldMusicError::MusicCategoryMissing)?;
    let mut visited = HashSet::new();
    let mut gain = 1.0;
    loop {
        let name = category
            .editor_id
            .clone()
            .unwrap_or_else(|| category.form_key.clone());
        if !visited.insert(category.form_key.as_str()) {
            return Err(WorldMusicError::CategoryCycle(name));
        }
        gain *= category
            .static_multiplier
            .ok_or_else(|| WorldMusicError::CategoryWithoutMultiplier(name.clone()))?;
        let Some(parent) = &category.parent else {
            return Ok(gain);
        };
        category =
            by_key
                .get(parent.as_str())
                .ok_or_else(|| WorldMusicError::CategoryParentMissing {
                    category: name,
                    parent: parent.clone(),
                })?;
    }
}

/// The music types some place names.
fn reachable_type_keys(catalog: &MusicCatalog) -> BTreeSet<String> {
    catalog
        .cell_places
        .iter()
        .filter_map(|place| place.music_type.clone())
        .chain(catalog.location_music.values().cloned())
        .chain(catalog.worldspace_music.values().cloned())
        .chain(
            catalog
                .region_music
                .iter()
                .map(|region| region.music_type.clone()),
        )
        .collect()
}

/// Every track that can play: the tracks of each reachable music type, and the sub-tracks of their
/// palettes.
fn reachable_tracks<'a>(
    reachable_types: &BTreeSet<String>,
    music_types: &MusicTypes,
    tracks: &'a Tracks,
) -> Result<Vec<&'a MusicTrack>, WorldMusicError> {
    let mut pending: Vec<&str> = Vec::new();
    for key in reachable_types {
        pending.extend(
            lookup_type(music_types, key)?
                .tracks
                .iter()
                .map(String::as_str),
        );
    }
    let mut seen = HashSet::new();
    let mut reachable = Vec::new();
    while let Some(key) = pending.pop() {
        if !seen.insert(key) {
            continue;
        }
        let track = lookup_track(tracks, key)?;
        if let MusicTrackKind::Palette { layers, .. } = &track.kind {
            pending.extend(layers.iter().flatten().map(String::as_str));
        }
        reachable.push(track);
    }
    reachable.sort_by(|a, b| a.form_key.cmp(&b.form_key));
    Ok(reachable)
}

fn track_name(track: &MusicTrack) -> String {
    match &track.editor_id {
        Some(editor_id) => format!("{editor_id} ({})", track.form_key),
        None => track.form_key.clone(),
    }
}

fn validate_reachable(
    reachable_types: &BTreeSet<String>,
    music_types: &MusicTypes,
    reachable: &[&MusicTrack],
    tracks: &Tracks,
    context: &FreshWorldContext,
) -> Result<(), WorldMusicError> {
    for track in reachable {
        conditions::evaluate(&track.conditions, context).map_err(|source| {
            WorldMusicError::Condition {
                track: track_name(track),
                source,
            }
        })?;
        if let MusicTrackKind::Palette { layers, .. } = &track.kind {
            for key in layers.iter().flatten() {
                if matches!(
                    lookup_track(tracks, key)?.kind,
                    MusicTrackKind::Palette { .. }
                ) {
                    return Err(SchedulerError::NestedPalette(key.clone()).into());
                }
            }
        }
    }
    for key in reachable_types {
        fade_of(lookup_type(music_types, key)?)?;
    }
    Ok(())
}

/// The file keys of the single tracks among `reachable`.
fn single_file_keys<'a>(reachable: &[&'a MusicTrack]) -> Vec<&'a str> {
    reachable
        .iter()
        .filter_map(|track| match &track.kind {
            MusicTrackKind::Single { file_key, .. } => Some(file_key.as_str()),
            MusicTrackKind::Silent { .. } | MusicTrackKind::Palette { .. } => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::db::music::{CellPlace, Comparand, CompareOp};
    use crate::audio::conditions::GET_STAGE;
    use crate::audio::scheduler::{FLAG_CYCLE_TRACKS, LayerStep};
    use crate::audio::test_support::catalog::{
        MASTER_CATEGORY_MULTIPLIER, MUSIC_CATEGORY_MULTIPLIER, exterior_place, palette, silent,
        single, timed_music_type as typed, world_catalog,
    };
    use crate::audio::test_support::condition::{condition, day, night};
    use crate::audio::test_support::{
        HALF_SECOND_SAMPLES, SAMPLE_RATE, SEED, SharedBuffer, sine_flac, undecodable_flac,
    };

    const CYCLES: u32 = FLAG_CYCLE_TRACKS;

    fn present_dir(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for file in files {
            write_file(
                dir.path(),
                file,
                &sine_flac(SAMPLE_RATE, HALF_SECOND_SAMPLES),
            );
        }
        dir
    }

    fn write_file(dir: &Path, file: &str, bytes: &[u8]) {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn two_types() -> MusicCatalog {
        world_catalog(
            &[("ws1", "A"), ("ws2", "B")],
            vec![
                typed("A", CYCLES, 4.0, &["music/a.flac"]),
                typed("B", 0, 2.0, &["music/b.flac"]),
            ],
            vec![
                single("music/a.flac", vec![]),
                single("music/b.flac", vec![]),
            ],
        )
    }

    fn world(catalog: MusicCatalog) -> WorldMusic {
        let dir = present_dir(&["music/a.flac", "music/b.flac"]);
        WorldMusic::new(catalog, dir.path(), SEED).unwrap()
    }

    fn build_error(catalog: MusicCatalog) -> WorldMusicError {
        let dir = present_dir(&["music/a.flac", "music/b.flac"]);
        WorldMusic::new(catalog, dir.path(), SEED).err().unwrap()
    }

    fn single_file(plan: Option<&PlayPlan>) -> &str {
        match plan {
            Some(PlayPlan::Single { file_key }) => file_key,
            other => panic!("expected a single track, got {other:?}"),
        }
    }

    fn update(music: &mut WorldMusic, worldspace: &str) -> Option<MusicChange> {
        music
            .update_place(&exterior_place(worldspace), GameTime::at_hour(9.0))
            .unwrap()
    }

    #[test]
    fn the_first_place_starts_its_music_type_with_that_types_fade() {
        let mut music = world(two_types());

        let change = update(&mut music, "ws1").unwrap();

        assert_eq!(change.fade, Duration::from_secs(4));
        assert_eq!(single_file(change.plan.as_ref()), "music/a.flac");
        assert_eq!(music.current_music_type().unwrap().form_key, "A");
        assert_eq!(music.current_plan(), change.plan.as_ref());
    }

    #[test]
    fn a_change_of_type_uses_the_new_types_fade() {
        let mut music = world(two_types());
        update(&mut music, "ws1");

        let change = update(&mut music, "ws2").unwrap();

        assert_eq!(change.fade, Duration::from_secs(2));
        assert_eq!(single_file(change.plan.as_ref()), "music/b.flac");
    }

    #[test]
    fn a_place_with_the_same_type_or_the_same_snapshot_changes_nothing() {
        let mut catalog = two_types();
        catalog.worldspace_music.insert("ws3".into(), "A".into());
        let mut music = world(catalog);
        update(&mut music, "ws1");

        assert_eq!(update(&mut music, "ws1"), None);
        assert_eq!(update(&mut music, "ws3"), None);
    }

    #[test]
    fn a_moving_point_in_the_same_music_type_changes_nothing() {
        let mut music = world(two_types());
        update(&mut music, "ws1");
        let mut moved = exterior_place("ws1");
        moved.point_beth = (10.0, 20.0);

        assert_eq!(
            music.update_place(&moved, GameTime::at_hour(9.0)).unwrap(),
            None
        );
    }

    #[test]
    fn a_place_without_any_music_type_fades_out_over_the_old_types_fade() {
        let mut music = world(two_types());
        update(&mut music, "ws2");

        let change = update(&mut music, "nowhere").unwrap();

        assert_eq!(
            change,
            MusicChange {
                fade: Duration::from_secs(2),
                abrupt: false,
                plan: None
            }
        );
        assert!(music.current_music_type().is_none());
    }

    #[test]
    fn a_first_place_without_any_music_type_has_nothing_to_fade() {
        let mut music = world(two_types());
        assert_eq!(update(&mut music, "nowhere"), None);
    }

    #[test]
    fn a_music_type_without_tracks_changes_to_silence_with_its_own_fade() {
        let mut catalog = two_types();
        catalog.music_types.push(typed("NONE", 0, 3.0, &[]));
        catalog
            .worldspace_music
            .insert("quiet".into(), "NONE".into());
        let mut music = world(catalog);
        update(&mut music, "ws1");

        let change = update(&mut music, "quiet").unwrap();

        assert_eq!(
            change,
            MusicChange {
                fade: Duration::from_secs(3),
                abrupt: false,
                plan: None
            }
        );
    }

    #[test]
    fn only_a_cycling_type_asks_for_a_next_plan() {
        let mut music = world(two_types());
        update(&mut music, "ws1");
        assert!(music.next_plan(GameTime::at_hour(9.0)).unwrap().is_some());

        update(&mut music, "ws2");
        assert_eq!(music.next_plan(GameTime::at_hour(9.0)).unwrap(), None);
        assert_eq!(music.current_plan(), None);
    }

    #[test]
    fn the_hour_decides_which_track_is_eligible() {
        let catalog = world_catalog(
            &[("ws1", "A")],
            vec![typed(
                "A",
                CYCLES,
                1.0,
                &["music/day.flac", "music/night.flac"],
            )],
            vec![
                single("music/day.flac", day()),
                single("music/night.flac", night()),
            ],
        );
        let dir = present_dir(&["music/day.flac", "music/night.flac"]);
        let mut music = WorldMusic::new(catalog, dir.path(), SEED).unwrap();
        update(&mut music, "ws1");

        assert_eq!(single_file(music.current_plan()), "music/day.flac");
        assert_eq!(
            single_file(music.next_plan(GameTime::at_hour(23.0)).unwrap().as_ref()),
            "music/night.flac"
        );
    }

    #[test]
    fn reset_makes_the_next_place_start_afresh() {
        let mut music = world(two_types());
        update(&mut music, "ws1");
        music.reset();

        assert!(update(&mut music, "ws1").is_some());
    }

    #[test]
    fn a_palette_plan_lists_the_sub_tracks_of_each_layer_in_order() {
        let catalog = world_catalog(
            &[("ws1", "A")],
            vec![typed("A", 0, 1.0, &["pal"])],
            vec![
                palette(
                    "pal",
                    &[&["music/a.flac", "pause", "music/b.flac"]],
                    60.0,
                    None,
                ),
                silent("pause", 4.0),
                single("music/a.flac", vec![]),
                single("music/b.flac", vec![]),
            ],
        );
        let mut music = world(catalog);

        let change = update(&mut music, "ws1").unwrap();

        let Some(PlayPlan::Palette { layers, .. }) = change.plan else {
            panic!("expected a palette plan, got {:?}", change.plan);
        };
        assert_eq!(
            layers,
            [vec![
                LayerStep::Sound {
                    file_key: "music/a.flac".to_string()
                },
                LayerStep::Gap { seconds: 4.0 },
                LayerStep::Sound {
                    file_key: "music/b.flac".to_string()
                },
            ]]
        );
    }

    #[test]
    fn an_unsupported_function_on_a_reachable_track_is_fatal_and_names_the_track() {
        let mut catalog = two_types();
        catalog.tracks[0].editor_id = Some("MUSTrackA".into());
        catalog.tracks[0].conditions = vec![condition(47, CompareOp::Equal, Comparand::Value(1.0))];

        let error = build_error(catalog);

        assert!(matches!(error, WorldMusicError::Condition { .. }));
        let message = error.to_string();
        assert!(
            message.contains("MUSTrackA") && message.contains("47"),
            "{message}"
        );
    }

    #[test]
    fn an_unsupported_function_on_a_palette_sub_track_is_fatal() {
        let mut sub = single("music/layer.flac", vec![]);
        sub.conditions = vec![condition(
            GET_STAGE,
            CompareOp::Equal,
            Comparand::Value(0.0),
        )];
        let catalog = world_catalog(
            &[("ws1", "A")],
            vec![typed("A", 0, 1.0, &["pal"])],
            vec![palette("pal", &[&["music/layer.flac"]], 10.0, None), sub],
        );

        let error = build_error(catalog);

        assert!(error.to_string().contains("music/layer.flac"), "{error}");
    }

    #[test]
    fn a_palette_inside_a_palette_is_fatal_at_startup_even_when_it_is_never_picked() {
        let catalog = world_catalog(
            &[("ws1", "A")],
            vec![typed("A", 0, 1.0, &["outer"])],
            vec![
                palette("outer", &[&["inner"]], 10.0, None),
                palette("inner", &[], 10.0, None),
            ],
        );

        let error = build_error(catalog);

        assert!(
            matches!(
                error,
                WorldMusicError::Scheduler(SchedulerError::NestedPalette(ref key)) if key == "inner"
            ),
            "{error}"
        );
    }

    #[test]
    fn a_listed_track_without_a_record_or_a_place_music_type_without_a_record_is_fatal() {
        let mut catalog = two_types();
        catalog.music_types[0].tracks.push("ghost".to_string());
        assert!(matches!(
            build_error(catalog),
            WorldMusicError::Scheduler(SchedulerError::UnknownTrack(ref key)) if key == "ghost"
        ));

        let mut catalog = two_types();
        catalog
            .worldspace_music
            .insert("ws9".into(), "GHOST".into());
        assert!(matches!(
            build_error(catalog),
            WorldMusicError::Place(PlaceError::UnknownMusicType(ref key)) if key == "GHOST"
        ));
    }

    #[test]
    fn an_unreachable_track_with_an_unsupported_function_is_ignored() {
        let mut catalog = two_types();
        let mut stray = single("music/stray.flac", vec![]);
        stray.conditions = vec![condition(47, CompareOp::Equal, Comparand::Value(1.0))];
        catalog.tracks.push(stray);
        catalog
            .music_types
            .push(typed("C", 0, 1.0, &["music/stray.flac"]));
        let dir = present_dir(&["music/a.flac", "music/b.flac"]);

        assert!(WorldMusic::new(catalog, dir.path(), SEED).is_ok());
    }

    fn here_and_gone() -> MusicCatalog {
        world_catalog(
            &[("ws1", "A")],
            vec![typed(
                "A",
                CYCLES,
                1.0,
                &["music/here.flac", "music/gone.flac"],
            )],
            vec![
                single("music/here.flac", vec![]),
                single("music/gone.flac", vec![]),
            ],
        )
    }

    #[test]
    fn a_music_file_that_is_absent_is_excluded_from_every_pick() {
        let dir = present_dir(&["music/here.flac"]);
        let mut music = WorldMusic::new(here_and_gone(), dir.path(), SEED).unwrap();
        update(&mut music, "ws1");

        assert_eq!(
            music.missing_files(),
            &HashSet::from(["music/gone.flac".to_string()])
        );
        for _ in 0..50 {
            assert_eq!(
                single_file(music.next_plan(GameTime::at_hour(9.0)).unwrap().as_ref()),
                "music/here.flac"
            );
        }
    }

    #[test]
    fn a_music_file_used_by_several_tracks_is_logged_once_when_absent() {
        let mut catalog = here_and_gone();
        let mut again = single("music/gone.flac", vec![]);
        again.form_key = "gone-again".to_string();
        catalog.tracks.push(again);
        catalog.music_types[0].tracks.push("gone-again".to_string());
        let dir = present_dir(&["music/here.flac"]);
        let logs = SharedBuffer::default();

        {
            let _capture = logs.capture("error");
            WorldMusic::new(catalog, dir.path(), SEED).unwrap();
        }

        let lines = logs.drain();
        assert_eq!(lines.matches("gone.flac").count(), 1, "{lines}");
        assert!(lines.contains("missing"), "{lines}");
    }

    #[test]
    fn a_music_file_that_is_present_but_cannot_be_opened_is_fatal() {
        let dir = present_dir(&["music/here.flac"]);
        write_file(dir.path(), "music/gone.flac", b"");

        let empty = WorldMusic::new(here_and_gone(), dir.path(), SEED)
            .err()
            .unwrap();

        assert!(
            matches!(&empty, WorldMusicError::MusicFile { path, .. } if path.ends_with("gone.flac")),
            "{empty}"
        );

        write_file(
            dir.path(),
            "music/gone.flac",
            b"this is not a flac file at all",
        );
        let garbage = WorldMusic::new(here_and_gone(), dir.path(), SEED)
            .err()
            .unwrap();
        assert!(
            matches!(garbage, WorldMusicError::MusicFile { .. }),
            "{garbage}"
        );
        assert!(
            garbage.to_string().contains("run the installer"),
            "{garbage}"
        );
    }

    #[test]
    fn a_music_file_that_opens_but_cannot_decode_passes_the_startup_check() {
        let dir = present_dir(&["music/here.flac"]);
        write_file(
            dir.path(),
            "music/gone.flac",
            &undecodable_flac(SAMPLE_RATE, HALF_SECOND_SAMPLES),
        );

        assert!(WorldMusic::new(here_and_gone(), dir.path(), SEED).is_ok());
    }

    #[test]
    fn a_missing_audio_folder_is_fatal() {
        let dir = present_dir(&[]);
        let absent = dir.path().join("audio");

        let error = WorldMusic::new(two_types(), &absent, SEED).err().unwrap();

        assert!(
            matches!(error, WorldMusicError::AudioDirectoryMissing(ref path) if *path == absent),
            "{error}"
        );
        assert!(error.to_string().contains("run the installer"), "{error}");
    }

    #[test]
    fn silent_tracks_need_no_music_file() {
        let catalog = world_catalog(
            &[("ws1", "A")],
            vec![typed("A", 0, 1.0, &["pause"])],
            vec![silent("pause", 5.0)],
        );
        let dir = present_dir(&[]);

        let music = WorldMusic::new(catalog, dir.path(), SEED).unwrap();

        assert!(music.missing_files().is_empty());
    }

    #[test]
    fn a_music_type_with_an_unusable_fade_is_fatal() {
        let mut catalog = two_types();
        catalog.music_types[0].fade_seconds = -1.0;

        assert!(matches!(build_error(catalog), WorldMusicError::Fade { .. }));
    }

    #[test]
    fn the_volume_is_the_music_multiplier_times_those_of_its_ancestors() {
        let music = world(two_types());

        let expected = MASTER_CATEGORY_MULTIPLIER * MUSIC_CATEGORY_MULTIPLIER;
        assert!((music.volume().as_amplitude() - expected).abs() < 1e-4);
    }

    #[test]
    fn missing_category_data_is_fatal() {
        let build = |edit: fn(&mut Vec<SoundCategory>)| {
            let mut catalog = two_types();
            edit(&mut catalog.sound_categories);
            build_error(catalog)
        };

        assert!(matches!(
            build(Vec::clear),
            WorldMusicError::MusicCategoryMissing
        ));
        assert!(matches!(
            build(|c| c[1].static_multiplier = None),
            WorldMusicError::CategoryWithoutMultiplier(_)
        ));
        assert!(matches!(
            build(|c| c[0].static_multiplier = None),
            WorldMusicError::CategoryWithoutMultiplier(_)
        ));
        assert!(matches!(
            build(|c| c[1].parent = Some("elsewhere".into())),
            WorldMusicError::CategoryParentMissing { .. }
        ));
        assert!(matches!(
            build(|c| c[0].parent = Some("music".into())),
            WorldMusicError::CategoryCycle(_)
        ));
    }

    #[test]
    fn an_exterior_cell_without_a_position_is_a_fatal_place_error() {
        let mut catalog = two_types();
        catalog.cell_places.push(CellPlace {
            cell: "c".into(),
            music_type: Some("A".into()),
            location: None,
            worldspace: None,
            grid: None,
            is_interior: false,
        });

        assert!(matches!(build_error(catalog), WorldMusicError::Place(_)));
    }
}
