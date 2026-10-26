//! The footstep sounds of a converted installation: dirt and stone resolve to existing files for
//! run, sprint and sneak.
//!
//! Ignored by default (needs an installation converted with the footstep tables and audio). Run
//! with `cargo test --test footstep_audio_real_install -- --ignored --nocapture`.

mod common;

use std::collections::HashSet;
use std::path::PathBuf;

use openfo4::assets::db::DbClient;
use openfo4::assets::db::footsteps::FootstepCatalog;
use openfo4::assets::layout::audio_dir;
use openfo4::audio::conditions::FreshWorldContext;
use openfo4::audio::footsteps::{Footsteps, Resolution, resolve};
use openfo4::footsteps::{FootTag, FootstepEvent, Gait, SurfaceMaterial};
use openfo4::random::SplitMix64;

const DIRT_MATERIAL: &str = "012F38:Fallout4.esm";
const STONE_IMPACT_NAME: &str = "Stone";
/// The stair sounds of stone have no source file in the game.
const STAIRS_IMPACT_NAME: &str = "Stairs";
const SEED: u64 = 1;

/// Folders of the default set's source files that no game archive contains.
const UNSHIPPED_FOLDERS: [&str; 7] = [
    "sound/fx/fst/player/waterpuddle/",
    "sound/fx/fst/water/land/",
    "sound/fx/fst/grass/jumpdown/",
    "sound/fx/fst/grass/jumpup/",
    "sound/fx/fst/stonesolid/land/",
    "sound/fx/fst/stonesolid/stairs/l/",
    "sound/fx/fst/wood/stairs/l/",
];

fn catalog() -> FootstepCatalog {
    let db = DbClient::new(common::database_path().to_str().unwrap()).unwrap();
    db.get_footstep_catalog().unwrap()
}

fn audio_folder() -> PathBuf {
    PathBuf::from(audio_dir(common::asset_root().to_str().unwrap()))
}

/// The material of the first entry of the run footstep's impact data set whose impact names stone.
fn stone_material(catalog: &FootstepCatalog) -> u32 {
    let set = &catalog.footstep_sets[&catalog.default_footstep_set];
    let footstep = set.gaits[&Gait::Run]
        .iter()
        .map(|key| &catalog.footsteps[key])
        .find(|footstep| footstep.tag == FootTag::FootRight)
        .expect("the run gait has a right foot");
    let data_set = &catalog.impact_data_sets[footstep.impact_data_set.as_ref().unwrap()];
    data_set
        .entries
        .iter()
        .find(|entry| {
            catalog.impacts[&entry.impact]
                .editor_id
                .as_deref()
                .is_some_and(|name| {
                    name.contains(STONE_IMPACT_NAME) && !name.contains(STAIRS_IMPACT_NAME)
                })
        })
        .expect("the run impact data set lists a stone impact")
        .material
}

#[test]
#[ignore = "needs a converted installation"]
fn dirt_and_stone_resolve_to_existing_files_for_every_gait() {
    let catalog = catalog();
    let audio = audio_folder();
    let context = FreshWorldContext::new(0.0, []);
    let materials = [
        ("dirt", catalog.material_hashes[DIRT_MATERIAL]),
        ("stone", stone_material(&catalog)),
    ];
    let steps = [
        (Gait::Run, FootTag::FootRight),
        (Gait::Run, FootTag::FootLeft),
        (Gait::Sprint, FootTag::FootSprintRight),
        (Gait::Sprint, FootTag::FootSprintLeft),
        (Gait::Sneak, FootTag::FootRight),
        (Gait::Sneak, FootTag::FootLeft),
    ];
    let mut rng = SplitMix64::new(SEED);
    for (name, material) in materials {
        for (gait, tag) in steps {
            let event = FootstepEvent {
                tag,
                gait,
                surface: SurfaceMaterial::Hash(material),
            };
            let resolution = resolve(
                &catalog,
                &context,
                event,
                Some(material),
                &HashSet::new(),
                &mut rng,
            )
            .unwrap_or_else(|error| panic!("{name} {gait:?} {tag:?}: {error}"));
            let Resolution::Play(layers) = resolution else {
                panic!("{name} {gait:?} {tag:?} is silent: {resolution:?}");
            };
            for layer in &layers {
                let path = audio.join(&layer.file_key);
                assert!(
                    path.exists(),
                    "{name} {gait:?} {tag:?}: {path:?} is missing"
                );
                println!(
                    "{name} {gait:?} {tag:?}: {} {:?}",
                    layer.file_key, layer.mix
                );
            }
        }
    }
}

#[test]
#[ignore = "needs a converted installation"]
fn the_default_set_opens_and_only_the_sources_the_game_never_shipped_are_missing() {
    let footsteps = Footsteps::new(catalog(), &audio_folder(), SEED).unwrap();
    let missing = footsteps.missing_files();
    for file_key in missing {
        assert!(
            UNSHIPPED_FOLDERS
                .iter()
                .any(|folder| file_key.starts_with(folder)),
            "{file_key} is missing but is not in a folder the game leaves unshipped"
        );
    }
    assert!(
        !missing.is_empty(),
        "the game leaves source files unshipped"
    );
}
