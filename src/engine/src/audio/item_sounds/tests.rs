use std::path::Path;

use super::*;
use crate::assets::db::item_sounds::ItemSoundCatalog;
use crate::assets::db::sound_catalog::{COMPOUND_DESCRIPTOR, SoundDescriptor};
use crate::audio::file_probe::ProbeError;
use crate::audio::test_support::item_sounds::{
    JACKET, JACKET_PUT_DOWN, JACKET_PUT_DOWN_FILE, NOTE, PISTOL, PISTOL_PICKUP, PISTOL_PICKUP_FILE,
    SCRAP, TONIC, TONIC_PICKUP, TONIC_PICKUP_FILE, TONIC_PUT_DOWN, TONIC_PUT_DOWN_FILE,
    UNKNOWN_ITEM, all_files, catalog, default_descriptor, default_file,
};
use crate::audio::test_support::{
    HALF_SECOND_SAMPLES, SAMPLE_RATE, SEED, SharedBuffer, sine_flac, write_audio_file,
};

fn install() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for file in all_files() {
        write_audio_file(
            root.path(),
            &file,
            &sine_flac(SAMPLE_RATE, HALF_SECOND_SAMPLES),
        );
    }
    root
}

fn open(root: &Path, catalog: ItemSoundCatalog) -> Result<ItemSounds, ItemSoundError> {
    ItemSounds::new(catalog, &root.join("audio"), SEED)
}

fn modify_descriptor(
    catalog: &mut ItemSoundCatalog,
    key: &str,
    change: impl FnOnce(&mut SoundDescriptor),
) {
    change(catalog.sounds.descriptors.get_mut(key).unwrap());
}

fn played_file(sounds: &mut ItemSounds, item: &str, kind: ItemSoundKind) -> String {
    sounds.sound(item, kind).unwrap().unwrap().file_key
}

mod descriptor_choice {
    use super::*;

    fn choice(item: &str, kind: ItemSoundKind) -> Option<String> {
        descriptor_for(&catalog(), item, kind).map(String::from)
    }

    #[test]
    fn an_own_sound_wins_on_its_side() {
        assert_eq!(
            choice(TONIC, ItemSoundKind::PickUp).as_deref(),
            Some(TONIC_PICKUP)
        );
        assert_eq!(
            choice(TONIC, ItemSoundKind::PutDown).as_deref(),
            Some(TONIC_PUT_DOWN)
        );
    }

    #[test]
    fn an_item_with_a_pickup_sound_only_falls_back_on_the_put_down_side_alone() {
        assert_eq!(
            choice(PISTOL, ItemSoundKind::PickUp).as_deref(),
            Some(PISTOL_PICKUP)
        );
        assert_eq!(
            choice(PISTOL, ItemSoundKind::PutDown),
            Some(default_descriptor("PDSW"))
        );
    }

    #[test]
    fn an_item_with_a_put_down_sound_only_falls_back_on_the_pickup_side_alone() {
        assert_eq!(
            choice(JACKET, ItemSoundKind::PutDown).as_deref(),
            Some(JACKET_PUT_DOWN)
        );
        assert_eq!(
            choice(JACKET, ItemSoundKind::PickUp),
            Some(default_descriptor("PUSA"))
        );
    }

    #[test]
    fn an_item_without_sounds_plays_the_defaults_of_its_slot_on_both_sides() {
        for (item, pickup, put_down) in [(NOTE, "PUSB", "PDSB"), (SCRAP, "PUSG", "PDSG")] {
            assert_eq!(
                choice(item, ItemSoundKind::PickUp),
                Some(default_descriptor(pickup))
            );
            assert_eq!(
                choice(item, ItemSoundKind::PutDown),
                Some(default_descriptor(put_down))
            );
        }
    }

    #[test]
    fn an_unknown_item_has_no_descriptor() {
        assert_eq!(choice(UNKNOWN_ITEM, ItemSoundKind::PickUp), None);
    }

    #[test]
    fn a_slot_without_default_leaves_only_the_side_that_falls_back_on_it_without_descriptor() {
        let mut catalog = catalog();
        catalog.defaults.remove("PDSW");
        assert_eq!(
            descriptor_for(&catalog, PISTOL, ItemSoundKind::PutDown),
            None
        );
        assert_eq!(
            descriptor_for(&catalog, PISTOL, ItemSoundKind::PickUp),
            Some(PISTOL_PICKUP)
        );
    }
}

mod startup {
    use super::*;

    #[test]
    fn a_complete_install_opens() {
        assert!(open(install().path(), catalog()).is_ok());
    }

    #[test]
    fn a_missing_audio_folder_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        assert!(matches!(
            open(root.path(), catalog()),
            Err(ItemSoundError::Probe(ProbeError::DirectoryMissing(_)))
        ));
    }

    #[test]
    fn a_file_whose_header_cannot_be_opened_is_an_error() {
        let root = install();
        write_audio_file(root.path(), PISTOL_PICKUP_FILE, &[]);
        assert!(matches!(
            open(root.path(), catalog()),
            Err(ItemSoundError::Probe(ProbeError::Open { .. }))
        ));
    }

    #[test]
    fn an_absent_file_is_logged_once_and_skipped() {
        let root = install();
        std::fs::remove_file(root.path().join("audio").join(TONIC_PICKUP_FILE)).unwrap();
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4=warn");
        let mut sounds = open(root.path(), catalog()).unwrap();
        let logged = log.drain();
        let missing: Vec<&str> = logged
            .lines()
            .filter(|line| line.contains("is missing"))
            .collect();
        assert_eq!(missing.len(), 1, "{logged}");
        assert!(missing[0].contains(TONIC_PICKUP_FILE), "{logged}");
        assert_eq!(sounds.sound(TONIC, ItemSoundKind::PickUp).unwrap(), None);
        assert_eq!(
            played_file(&mut sounds, TONIC, ItemSoundKind::PutDown),
            TONIC_PUT_DOWN_FILE
        );
    }

    #[test]
    fn a_default_file_absent_silences_only_the_sides_that_fall_back_on_it() {
        let root = install();
        std::fs::remove_file(root.path().join("audio").join(default_file("PDSW"))).unwrap();
        let mut sounds = open(root.path(), catalog()).unwrap();
        assert_eq!(sounds.sound(PISTOL, ItemSoundKind::PutDown).unwrap(), None);
        assert_eq!(
            played_file(&mut sounds, PISTOL, ItemSoundKind::PickUp),
            PISTOL_PICKUP_FILE
        );
    }

    #[test]
    fn a_compound_descriptor_is_an_error() {
        let mut catalog = catalog();
        modify_descriptor(&mut catalog, PISTOL_PICKUP, |descriptor| {
            descriptor.descriptor_type = COMPOUND_DESCRIPTOR;
        });
        assert!(matches!(
            open(install().path(), catalog),
            Err(ItemSoundError::Resolve(
                SoundError::UnsupportedDescriptor { .. }
            ))
        ));
    }

    #[test]
    fn a_looping_descriptor_is_an_error() {
        let mut catalog = catalog();
        modify_descriptor(&mut catalog, PISTOL_PICKUP, |descriptor| {
            descriptor.looping_flags = 0x0800;
        });
        assert!(matches!(
            open(install().path(), catalog),
            Err(ItemSoundError::Resolve(SoundError::Looping(key))) if key == PISTOL_PICKUP
        ));
    }

    #[test]
    fn the_low_looping_flags_one_and_two_are_not_looping() {
        let mut catalog = catalog();
        for (key, flags) in [(PISTOL_PICKUP, 1), (TONIC_PICKUP, 2)] {
            modify_descriptor(&mut catalog, key, |descriptor| {
                descriptor.looping_flags = flags;
            });
        }
        assert!(open(install().path(), catalog).is_ok());
    }

    #[test]
    fn a_descriptor_without_output_model_is_an_error() {
        let mut catalog = catalog();
        modify_descriptor(&mut catalog, JACKET_PUT_DOWN, |descriptor| {
            descriptor.output_model = None;
        });
        assert!(matches!(
            open(install().path(), catalog),
            Err(ItemSoundError::Resolve(SoundError::NoOutputModel(_)))
        ));
    }

    #[test]
    fn a_default_naming_an_unknown_descriptor_is_an_error() {
        let mut catalog = catalog();
        catalog.defaults.insert("PUSG", "no_such_sound".to_string());
        assert!(matches!(
            open(install().path(), catalog),
            Err(ItemSoundError::Resolve(SoundError::UnknownRecord { .. }))
        ));
    }
}

mod playing {
    use super::*;

    fn sounds() -> ItemSounds {
        open(install().path(), catalog()).unwrap()
    }

    #[test]
    fn each_side_plays_the_file_of_its_resolved_descriptor() {
        let mut sounds = sounds();
        let cases = [
            (
                PISTOL,
                ItemSoundKind::PickUp,
                PISTOL_PICKUP_FILE.to_string(),
            ),
            (PISTOL, ItemSoundKind::PutDown, default_file("PDSW")),
            (JACKET, ItemSoundKind::PickUp, default_file("PUSA")),
            (
                JACKET,
                ItemSoundKind::PutDown,
                JACKET_PUT_DOWN_FILE.to_string(),
            ),
            (NOTE, ItemSoundKind::PickUp, default_file("PUSB")),
            (SCRAP, ItemSoundKind::PutDown, default_file("PDSG")),
        ];
        for (item, kind, file) in cases {
            assert_eq!(
                played_file(&mut sounds, item, kind),
                file,
                "{item} {kind:?}"
            );
        }
    }

    #[test]
    fn a_repeated_pickup_plays_again() {
        let mut sounds = sounds();
        for _ in 0..3 {
            assert!(
                sounds
                    .sound(SCRAP, ItemSoundKind::PickUp)
                    .unwrap()
                    .is_some()
            );
        }
    }

    #[test]
    fn an_unknown_item_is_silent_and_warned_once_per_item() {
        let mut sounds = sounds();
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4::audio::item_sounds=warn");
        for kind in [
            ItemSoundKind::PickUp,
            ItemSoundKind::PutDown,
            ItemSoundKind::PickUp,
        ] {
            assert_eq!(sounds.sound(UNKNOWN_ITEM, kind).unwrap(), None);
        }
        let first = log.drain();
        assert_eq!(first.lines().count(), 1, "{first}");
        assert!(first.contains(UNKNOWN_ITEM), "{first}");
        sounds
            .sound("another_unknown", ItemSoundKind::PickUp)
            .unwrap();
        assert_eq!(log.drain().lines().count(), 1);
    }

    #[test]
    fn a_known_item_logs_nothing() {
        let mut sounds = sounds();
        let log = SharedBuffer::default();
        let _guard = log.capture("openfo4::audio::item_sounds=warn");
        sounds.sound(PISTOL, ItemSoundKind::PickUp).unwrap();
        assert_eq!(log.drain(), "");
    }

    #[test]
    fn a_slot_default_that_vanished_is_an_error_naming_the_slot() {
        let mut sounds = sounds();
        sounds.catalog.defaults.remove("PDSW");
        let error = sounds.sound(PISTOL, ItemSoundKind::PutDown).unwrap_err();
        assert!(
            matches!(
                &error,
                ItemSoundError::Resolve(SoundError::UnknownRecord { key, .. }) if key == "PDSW"
            ),
            "{error}"
        );
    }
}
