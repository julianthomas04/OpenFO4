//! The game INIs the installer copied into a real installation carry every setting the engine
//! needs, so the engine starts. Ignored by default (needs an installation). Run with
//! `OPENFO4_DB=<install>/assets/data/data.db cargo test --test ini_real_install -- --ignored`.

use std::path::Path;

use openfo4::rendering::game_settings::GameSettings;

#[test]
#[ignore = "needs a converted installation (OPENFO4_DB)"]
fn the_installed_game_inis_carry_every_setting_the_engine_reads() {
    let db_path = std::env::var("OPENFO4_DB").expect("set OPENFO4_DB to data.db");
    let asset_root = Path::new(&db_path)
        .parent()
        .and_then(Path::parent)
        .expect("data.db sits in <assets>/data");

    if let Err(error) = GameSettings::load(asset_root) {
        panic!("{error}");
    }
}
