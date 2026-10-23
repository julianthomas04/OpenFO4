//! Every spawn picker location resolves against the converted game data. Ignored by default
//! (needs an installation). Run with
//! `OPENFO4_DB=<install>/assets/data/data.db cargo test --test spawn_locations_real_install -- --ignored`.

use openfo4::assets::db::DbClient;
use openfo4::game_flow::spawn_locations::{COMMONWEALTH, SpawnPlace, resolve_spawn_locations};

const PICKER_NAMES: [&str; 7] = [
    "RED ROCKET",
    "DIAMOND CITY",
    "THE CASTLE",
    "SANCTUARY HILLS",
    "MUSEUM OF FREEDOM",
    "GOODNEIGHBOR",
    "BUNKER HILL",
];

#[test]
#[ignore = "needs a converted installation (OPENFO4_DB)"]
fn the_picker_resolves_every_place_against_the_installation() {
    let db_path = std::env::var("OPENFO4_DB").expect("set OPENFO4_DB to data.db");
    let db = DbClient::new(&db_path).unwrap();

    let locations = resolve_spawn_locations(&db).unwrap_or_else(|error| panic!("{error}"));

    let names: Vec<_> = locations.iter().map(|location| location.name).collect();
    assert_eq!(names, PICKER_NAMES);
    for location in &locations {
        assert!(
            location.position.iter().all(|c| c.is_finite()),
            "{location:?}"
        );
        assert!(location.heading.is_some(), "{location:?}");
    }
    for location in locations
        .iter()
        .filter(|location| location.name != "MUSEUM OF FREEDOM")
    {
        assert_eq!(
            location.place,
            SpawnPlace::Exterior {
                worldspace: COMMONWEALTH.to_owned()
            },
            "{}",
            location.name
        );
    }
    let museum = locations
        .iter()
        .find(|location| location.name == "MUSEUM OF FREEDOM")
        .expect("the picker lists the Museum of Freedom");
    assert_eq!(
        museum.place,
        SpawnPlace::Interior {
            cell_form_key: "017DF7:Fallout4.esm".to_owned()
        }
    );
}
