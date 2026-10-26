//! A small weather sound catalog for tests: a clear weather with a wind bed, a rain weather that
//! shares the bed and adds rain, gusts and a thunder sound of two layers, a storm with a bed of its
//! own and the same thunder, and a weather with nothing but thunder. The beds play only outdoors,
//! as the exterior beds of the game do. Every descriptor takes the footstep catalog's output model
//! and categories.

use crate::assets::db::music::{Comparand, CompareOp, Condition};
use crate::assets::db::sound_catalog::{SoundCatalog, SoundDescriptor};
use crate::audio::conditions::IS_IN_INTERIOR;
use crate::audio::test_support::condition::condition;
use crate::audio::test_support::footsteps::{self, compound, standard};
use crate::audio::weather::WeatherSoundTable;
use crate::weather::{SoundKind, WeatherId, WeatherSound};

pub const CLEAR: WeatherId = WeatherId::new(0);
pub const RAIN: WeatherId = WeatherId::new(1);
pub const STORM: WeatherId = WeatherId::new(2);
pub const ONLY_THUNDER: WeatherId = WeatherId::new(3);

/// The bed of the clear and the rain weather.
pub const WIND_BED: &str = "wind_bed";
pub const RAIN_BED: &str = "rain_bed";
pub const GUSTS: &str = "gusts";
pub const STORM_BED: &str = "storm_bed";
/// The compound thunder of the rain, the storm and the thunder-only weather.
pub const THUNDER: &str = "thunder";
pub const THUNDER_HIGH: &str = "thunder_high";
pub const THUNDER_LOW: &str = "thunder_low";

pub const WIND_FILE: &str = "sound/fx/amb/wind_bed.flac";
pub const RAIN_FILES: [&str; 2] = ["sound/fx/amb/rain_a.flac", "sound/fx/amb/rain_b.flac"];
pub const GUSTS_FILE: &str = "sound/fx/amb/gusts.flac";
pub const STORM_FILE: &str = "sound/fx/amb/storm_bed.flac";
pub const THUNDER_HIGH_FILES: [&str; 2] = [
    "sound/fx/amb/thunder_high_a.flac",
    "sound/fx/amb/thunder_high_b.flac",
];
pub const THUNDER_LOW_FILE: &str = "sound/fx/amb/thunder_low.flac";

/// The loop type and the envelope flags the exterior beds of the game carry.
const LOOPING_FLAGS: u32 = 0x0804;

/// Every file the catalog's weathers can reach.
pub fn all_files() -> Vec<&'static str> {
    let mut files = vec![WIND_FILE, GUSTS_FILE, STORM_FILE, THUNDER_LOW_FILE];
    files.extend(RAIN_FILES);
    files.extend(THUNDER_HIGH_FILES);
    files
}

fn exterior_only() -> Vec<Condition> {
    vec![condition(
        IS_IN_INTERIOR,
        CompareOp::Equal,
        Comparand::Value(0.0),
    )]
}

fn looping(key: &str, files: &[&str], conditions: Vec<Condition>) -> SoundDescriptor {
    SoundDescriptor {
        looping_flags: LOOPING_FLAGS,
        ..standard(key, files, conditions)
    }
}

pub fn sound_catalog() -> SoundCatalog {
    let footsteps = footsteps::catalog().sounds;
    let descriptors = [
        looping(WIND_BED, &[WIND_FILE], exterior_only()),
        looping(RAIN_BED, &RAIN_FILES, exterior_only()),
        looping(GUSTS, &[GUSTS_FILE], exterior_only()),
        looping(STORM_BED, &[STORM_FILE], exterior_only()),
        standard(THUNDER_HIGH, &THUNDER_HIGH_FILES, vec![]),
        standard(THUNDER_LOW, &[THUNDER_LOW_FILE], vec![]),
        compound(THUNDER, &[THUNDER_HIGH, THUNDER_LOW]),
    ];
    SoundCatalog {
        descriptors: descriptors
            .into_iter()
            .map(|descriptor| (descriptor.form_key.clone(), descriptor))
            .collect(),
        output_models: footsteps.output_models,
        categories: footsteps.categories,
    }
}

fn sound(kind: SoundKind, descriptor: &str) -> WeatherSound {
    WeatherSound {
        kind,
        descriptor: descriptor.to_string(),
    }
}

pub fn table() -> WeatherSoundTable {
    WeatherSoundTable::new(vec![
        vec![sound(SoundKind::Default, WIND_BED)],
        vec![
            sound(SoundKind::Default, WIND_BED),
            sound(SoundKind::Precipitation, RAIN_BED),
            sound(SoundKind::Wind, GUSTS),
            sound(SoundKind::Thunder, THUNDER),
        ],
        vec![
            sound(SoundKind::Default, STORM_BED),
            sound(SoundKind::Thunder, THUNDER),
        ],
        vec![sound(SoundKind::Thunder, THUNDER)],
    ])
}

/// The gain every layer of the catalog has when its variance draws sit at their middle and no
/// level is applied.
pub fn steady_gain_db() -> f32 {
    footsteps::expected_gain_db()
}
