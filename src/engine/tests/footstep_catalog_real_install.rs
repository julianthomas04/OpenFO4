//! The footstep catalog of a converted installation: the default set resolves to the vanilla dirt step sound.
//!
//! Ignored by default (needs an installation whose database was converted with the footstep tables).
//! Run with `cargo test --test footstep_catalog_real_install -- --ignored --nocapture`.

mod common;

use openfo4::assets::db::DbClient;
use openfo4::assets::db::footsteps::{FootstepCatalog, FootstepSet};
use openfo4::footsteps::events::{FootTag, Gait};

const DEFAULT_SET_EDITOR_ID: &str = "DefaultFootstepSetXXX";
const DIRT_MATERIAL: &str = "012F38:Fallout4.esm";
const DIRT_WALK_RIGHT_DATA_SET: &str = "DefaultFootstepWalkRImpactset";
const DIRT_WALK_RIGHT_IMPACT: &str = "FSTDirtWalkRImpact";
const DIRT_WALK_RIGHT_SOUND: &str = "FSTPlayerDirtWalkR";

fn catalog() -> FootstepCatalog {
    let db = DbClient::new(common::database_path().to_str().unwrap()).unwrap();
    db.get_footstep_catalog().unwrap()
}

fn tags(catalog: &FootstepCatalog, set: &FootstepSet, gait: Gait) -> Vec<FootTag> {
    set.gaits[&gait]
        .iter()
        .map(|key| catalog.footsteps[key].tag)
        .collect()
}

#[test]
#[ignore = "needs a converted installation"]
fn default_footstep_set_covers_run_sprint_and_sneak() {
    let catalog = catalog();
    let set = &catalog.footstep_sets[&catalog.default_footstep_set];
    assert_eq!(set.editor_id.as_deref(), Some(DEFAULT_SET_EDITOR_ID));

    for (gait, expected) in [
        (Gait::Run, [FootTag::FootRight, FootTag::FootLeft]),
        (Gait::Sneak, [FootTag::FootRight, FootTag::FootLeft]),
        (
            Gait::Sprint,
            [FootTag::FootSprintRight, FootTag::FootSprintLeft],
        ),
    ] {
        let found = tags(&catalog, set, gait);
        for tag in expected {
            assert!(found.contains(&tag), "{gait:?} lacks {tag:?}: {found:?}");
        }
    }
}

#[test]
#[ignore = "needs a converted installation"]
fn dirt_on_the_walk_right_impact_data_set_plays_the_player_dirt_sound() {
    let catalog = catalog();
    let set = &catalog.footstep_sets[&catalog.default_footstep_set];
    let dirt = catalog.material_hashes[DIRT_MATERIAL];

    let data_set = set
        .gaits
        .values()
        .flatten()
        .filter_map(|key| catalog.footsteps[key].impact_data_set.as_ref())
        .map(|key| &catalog.impact_data_sets[key])
        .find(|data_set| data_set.editor_id.as_deref() == Some(DIRT_WALK_RIGHT_DATA_SET))
        .expect("the default set uses the walk right impact data set");
    let entry = data_set
        .entries
        .iter()
        .find(|entry| entry.material == dirt)
        .expect("the data set lists dirt");
    let impact = &catalog.impacts[&entry.impact];
    assert_eq!(impact.editor_id.as_deref(), Some(DIRT_WALK_RIGHT_IMPACT));
    let sound = &catalog.sound_descriptors[impact.player_sound.as_ref().unwrap()];
    assert_eq!(sound.editor_id.as_deref(), Some(DIRT_WALK_RIGHT_SOUND));
}
