//! The colour lookup tables the image spaces name, each read from the installed textures the
//! first time an image space grades with it.

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::Arc;

use thiserror::Error;

use super::lut_table::{LutTable, LutTableError};
use crate::assets::resolve::resolve_below;

#[derive(Debug, Error)]
pub enum LutLoadError {
    #[error("colour LUT {key} is not installed: {source}")]
    NotFound { key: Arc<str>, source: io::Error },
    #[error("colour LUT {key} cannot be read: {source}")]
    Unreadable { key: Arc<str>, source: io::Error },
    #[error("colour LUT {key}: {source}")]
    Table {
        key: Arc<str>,
        source: LutTableError,
    },
}

pub struct LutLibrary {
    asset_root: String,
    /// `None` for a table that failed to load; it is reported once and not read again.
    tables: HashMap<Arc<str>, Option<Arc<LutTable>>>,
}

impl LutLibrary {
    pub fn new(asset_root: &str) -> Self {
        Self {
            asset_root: asset_root.to_owned(),
            tables: HashMap::new(),
        }
    }

    /// The table of the texture key `key`, below the asset root; `None` when it cannot load, which
    /// leaves the picture ungraded where the table would grade it.
    pub fn table(&mut self, key: &Arc<str>) -> Option<Arc<LutTable>> {
        if let Some(known) = self.tables.get(key) {
            return known.clone();
        }
        let loaded = match load(&self.asset_root, key) {
            Ok(table) => Some(Arc::new(table)),
            Err(error) => {
                tracing::error!("{error}; the image space grades without it");
                None
            }
        };
        self.tables.insert(Arc::clone(key), loaded.clone());
        loaded
    }
}

fn load(asset_root: &str, key: &Arc<str>) -> Result<LutTable, LutLoadError> {
    let relative = resolve_below(Path::new(asset_root), key).map_err(|source| {
        let key = Arc::clone(key);
        if source.kind() == io::ErrorKind::NotFound {
            LutLoadError::NotFound { key, source }
        } else {
            LutLoadError::Unreadable { key, source }
        }
    })?;
    let bytes = std::fs::read(Path::new(asset_root).join(relative)).map_err(|source| {
        LutLoadError::Unreadable {
            key: Arc::clone(key),
            source,
        }
    })?;
    LutTable::from_dds(&bytes).map_err(|source| LutLoadError::Table {
        key: Arc::clone(key),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::super::lut_table::strip_dds;
    use super::*;

    const KEY: &str = r"textures\effects\luts\lut_cw_clearday.dds";

    fn installed(content: &[u8]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("textures").join("Effects").join("LUTs");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("LUT_CW_ClearDAY.DDS"), content).unwrap();
        root
    }

    fn library(root: &tempfile::TempDir) -> LutLibrary {
        LutLibrary::new(root.path().to_str().unwrap())
    }

    #[test]
    fn a_table_loads_from_the_installed_strip_whatever_the_case_of_its_folders() {
        let root = installed(&strip_dds(&LutTable::identity(), 0));
        let table = library(&root).table(&KEY.into());
        assert_eq!(table.as_deref(), Some(&LutTable::identity()));
    }

    #[test]
    fn a_table_is_read_once() {
        let root = installed(&strip_dds(&LutTable::identity(), 0));
        let mut library = library(&root);
        assert!(library.table(&KEY.into()).is_some());
        std::fs::remove_dir_all(root.path().join("textures")).unwrap();
        assert!(library.table(&KEY.into()).is_some());
    }

    #[test]
    fn a_missing_or_broken_table_is_an_error_naming_it_and_grades_with_none() {
        let root = installed(b"not a dds");
        let missing: Arc<str> = r"textures\effects\luts\missing.dds".into();
        assert!(matches!(
            load(root.path().to_str().unwrap(), &missing),
            Err(LutLoadError::NotFound { key, .. }) if key == missing
        ));
        assert!(matches!(
            load(root.path().to_str().unwrap(), &KEY.into()),
            Err(LutLoadError::Table { .. })
        ));
        let mut library = library(&root);
        assert_eq!(library.table(&missing), None);
        assert_eq!(library.table(&KEY.into()), None);
    }

    /// Developer checks against a real install (`OPENFO4_INSTALL_DIR`), run with `--ignored`.
    mod real_install {
        use glam::Vec3;
        use rusqlite::{Connection, OpenFlags};

        use super::*;
        use crate::rendering::lighting::tonemap_settings::ImageSpaceLut;

        /// The grey curve of `LUT_CW_ClearDAY` the game showed with the LUT alone (generation 8):
        /// display level in, level out.
        const MEASURED_CLEAR_DAY_GREYS: [(f32, f32); 8] = [
            (16.0, 14.0),
            (32.0, 24.0),
            (48.0, 38.0),
            (64.0, 54.0),
            (96.0, 95.0),
            (128.0, 140.0),
            (160.0, 187.0),
            (198.0, 235.0),
        ];
        /// The game's own rounding of the measured levels.
        const MEASURED_LEVELS: f32 = 1.0;
        /// Image space LUT paths of the game data that name no file of its archives.
        const PATHS_WITHOUT_A_FILE: [&str; 2] = [
            "ColorLUT.dds",
            r"Effects\LUTsEffects\LUTs\LUT_CW_ClearDAY.DDS",
        ];

        fn asset_root() -> String {
            let install =
                std::env::var(crate::assets::INSTALL_DIR_VAR).expect("OPENFO4_INSTALL_DIR");
            format!("{install}/{}", crate::assets::layout::ASSETS_DIR)
        }

        fn key(path: &str) -> Arc<str> {
            Arc::clone(&ImageSpaceLut::named(path).tables()[0].0)
        }

        #[test]
        #[ignore = "needs an install (OPENFO4_INSTALL_DIR)"]
        fn every_installed_lut_reads_as_a_table() {
            let root = asset_root();
            let folder = resolve_below(Path::new(&root), r"textures\effects\luts").unwrap();
            let mut read = 0;
            for entry in std::fs::read_dir(Path::new(&root).join(folder)).unwrap() {
                let path = entry.unwrap().path();
                let bytes = std::fs::read(&path).unwrap();
                if let Err(error) = LutTable::from_dds(&bytes) {
                    panic!("{}: {error}", path.display());
                }
                read += 1;
            }
            assert!(read > 0, "no LUT is installed");
        }

        #[test]
        #[ignore = "needs an install (OPENFO4_INSTALL_DIR)"]
        fn the_installed_tables_grade_the_greys_as_the_game_showed() {
            let mut library = LutLibrary::new(&asset_root());
            let identity = library.table(&key(r"Effects\LUTs\ColorLUT.dds")).unwrap();
            assert_eq!(*identity, LutTable::identity());
            let clear_day = library
                .table(&key(r"Effects\LUTs\LUT_CW_ClearDAY.DDS"))
                .unwrap();
            for (level, measured) in MEASURED_CLEAR_DAY_GREYS {
                let graded = clear_day.sample(Vec3::splat(level / 255.0)) * 255.0;
                assert!(
                    graded.abs_diff_eq(Vec3::splat(measured), MEASURED_LEVELS),
                    "{level}: graded {graded}, measured {measured}"
                );
            }
        }

        #[test]
        #[ignore = "needs an install (OPENFO4_INSTALL_DIR)"]
        fn every_lut_an_installed_image_space_names_loads_unless_the_game_has_no_such_file() {
            let root = asset_root();
            let db = Connection::open_with_flags(
                format!("{root}/data/data.db"),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            let mut query = db.prepare("SELECT data FROM image_spaces").unwrap();
            let paths: Vec<String> = query
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .map(|data| {
                    let data: serde_json::Value = serde_json::from_str(&data.unwrap()).unwrap();
                    data["Lut"]
                        .as_str()
                        .expect("every row names its LUT")
                        .to_owned()
                })
                .filter(|path| !path.is_empty())
                .collect();
            assert!(!paths.is_empty(), "no image space names a LUT");
            for path in paths {
                match load(&root, &key(&path)) {
                    Ok(_) => {}
                    Err(LutLoadError::NotFound { .. })
                        if PATHS_WITHOUT_A_FILE.contains(&&*path) => {}
                    Err(error) => panic!("{path}: {error}"),
                }
            }
        }
    }
}
