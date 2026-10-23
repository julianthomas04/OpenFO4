//! The game's UI fonts, extracted by the installer into the interface fonts folder, loaded once
//! and shared by every piece of native UI text (the HUD, the menus, the loading screen).
//!
//! [`UiFonts`] is plain data (`Send + Sync`): loading it reads the files and parses their
//! faces, which can happen on a worker thread. Each text renderer then takes its own
//! [`FontSystem`] from it with [`UiFonts::font_system`]; the font bytes are shared, not copied.

use std::path::Path;

use glyphon::{FontSystem, fontdb::Database};

const LOCALE: &str = "en-US";

#[derive(Debug, thiserror::Error)]
pub enum UiFontError {
    #[error(
        "cannot read the UI font folder '{dir}': {source}; the install is incomplete, run the \
         installer (it extracts the game's UI fonts from Interface.ba2)"
    )]
    ReadDir { dir: String, source: std::io::Error },
    #[error(
        "no fonts found in '{dir}': the install is incomplete; run the installer (it extracts \
         the game's UI fonts from Interface.ba2)"
    )]
    NoFonts { dir: String },
    #[error(
        "the UI font family '{family}' is not among the {loaded} font file(s) in '{dir}': the \
         install is incomplete or corrupt"
    )]
    MissingFamily {
        family: String,
        dir: String,
        loaded: usize,
    },
}

pub struct UiFonts {
    db: Database,
    dir: String,
    loaded: usize,
}

impl UiFonts {
    /// Reads every `.ttf` and `.otf` file in `font_dir`. An unreadable folder and a folder
    /// without a usable font are distinct errors; a single unreadable or malformed file is
    /// logged and skipped.
    pub fn load(font_dir: &str) -> Result<Self, UiFontError> {
        let read_dir_error = |source| UiFontError::ReadDir {
            dir: font_dir.to_owned(),
            source,
        };
        let mut files = Vec::new();
        for entry in std::fs::read_dir(font_dir).map_err(read_dir_error)? {
            let path = entry.map_err(read_dir_error)?.path();
            if !is_font_file(&path) {
                continue;
            }
            match std::fs::read(&path) {
                Ok(bytes) => files.push(bytes),
                Err(error) => {
                    tracing::warn!("UI fonts: failed to read {}: {error}", path.display());
                }
            }
        }
        let fonts = Self::from_font_files(font_dir, files)?;
        tracing::info!("UI fonts: loaded {} font(s) from {font_dir}", fonts.loaded);
        Ok(fonts)
    }

    /// Parses the font files `files`, which came from `dir`.
    pub fn from_font_files(dir: &str, files: Vec<Vec<u8>>) -> Result<Self, UiFontError> {
        let mut db = Database::new();
        let mut loaded = 0;
        for bytes in files {
            let faces_before = db.len();
            db.load_font_data(bytes);
            if db.len() == faces_before {
                tracing::warn!("UI fonts: a font file in {dir} has no usable face; skipped");
            } else {
                loaded += 1;
            }
        }
        if loaded == 0 {
            return Err(UiFontError::NoFonts {
                dir: dir.to_owned(),
            });
        }
        Ok(Self {
            db,
            dir: dir.to_owned(),
            loaded,
        })
    }

    /// Fails with [`UiFontError::MissingFamily`] unless a loaded face belongs to `family`.
    pub fn require_family(&self, family: &str) -> Result<(), UiFontError> {
        let present = self
            .db
            .faces()
            .any(|face| face.families.iter().any(|(name, _)| name == family));
        if present {
            Ok(())
        } else {
            Err(UiFontError::MissingFamily {
                family: family.to_owned(),
                dir: self.dir.clone(),
                loaded: self.loaded,
            })
        }
    }

    /// A font system holding exactly these fonts (no system fonts), for one text renderer.
    pub fn font_system(&self) -> FontSystem {
        FontSystem::new_with_locale_and_db(LOCALE.to_owned(), self.db.clone())
    }
}

fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("ttf") || extension.eq_ignore_ascii_case("otf")
        })
}

/// A real font without any game asset: the one egui ships.
#[cfg(test)]
pub(crate) mod test_support {
    use super::UiFonts;

    fn egui_font_bytes() -> Vec<u8> {
        let definitions = egui::FontDefinitions::default();
        let (_, data) = definitions
            .font_data
            .iter()
            .next()
            .expect("egui bundles default fonts");
        data.font.to_vec()
    }

    pub fn ui_fonts() -> UiFonts {
        UiFonts::from_font_files("test fonts", vec![egui_font_bytes()]).expect("a valid font")
    }

    /// The family name of the test font.
    pub fn family() -> String {
        let fonts = ui_fonts();
        let face = fonts.db.faces().next().expect("the font has a face");
        face.families[0].0.clone()
    }

    pub fn font_bytes() -> Vec<u8> {
        egui_font_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{family, font_bytes, ui_fonts};
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("openfo4_ui_fonts_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_loaded_family_is_required_successfully() {
        assert!(ui_fonts().require_family(&family()).is_ok());
    }

    #[test]
    fn a_family_that_was_not_loaded_is_a_missing_family_naming_it() {
        let error = ui_fonts().require_family("No Such Family").unwrap_err();
        assert!(matches!(
            &error,
            UiFontError::MissingFamily { family, loaded: 1, .. } if family == "No Such Family"
        ));
        assert!(error.to_string().contains("No Such Family"), "{error}");
    }

    #[test]
    fn a_font_system_holds_the_loaded_family_and_no_system_fonts() {
        let font_system = ui_fonts().font_system();
        let families: Vec<_> = font_system
            .db()
            .faces()
            .map(|face| face.families[0].0.clone())
            .collect();
        assert!(!families.is_empty());
        assert!(
            families.iter().all(|name| *name == family()),
            "{families:?}"
        );
    }

    #[test]
    fn no_font_files_is_a_no_fonts_error() {
        let error = UiFonts::from_font_files("dir", Vec::new()).err().unwrap();
        assert!(matches!(error, UiFontError::NoFonts { .. }));
    }

    #[test]
    fn a_file_that_is_not_a_font_is_not_counted() {
        let error = UiFonts::from_font_files("dir", vec![b"not a font".to_vec()])
            .err()
            .unwrap();
        assert!(matches!(error, UiFontError::NoFonts { .. }));
    }

    #[test]
    fn a_malformed_file_beside_a_valid_one_is_skipped() {
        let fonts = UiFonts::from_font_files("dir", vec![b"junk".to_vec(), font_bytes()]).unwrap();
        assert_eq!(fonts.loaded, 1);
    }

    #[test]
    fn a_missing_folder_is_a_read_error_naming_it() {
        let error = UiFonts::load("definitely/not/here/fonts").err().unwrap();
        assert!(
            matches!(&error, UiFontError::ReadDir { dir, .. } if dir == "definitely/not/here/fonts")
        );
    }

    #[test]
    fn a_folder_loads_its_font_files_and_ignores_other_files() {
        let dir = temp_dir("load");
        std::fs::write(dir.join("face.TTF"), font_bytes()).unwrap();
        std::fs::write(dir.join("notes.txt"), b"not a font").unwrap();

        let loaded = UiFonts::load(dir.to_str().unwrap());
        std::fs::remove_dir_all(&dir).unwrap();

        let fonts = loaded.unwrap();
        assert_eq!(fonts.loaded, 1);
        assert!(fonts.require_family(&family()).is_ok());
    }

    #[test]
    fn a_folder_without_fonts_is_a_no_fonts_error() {
        let dir = temp_dir("empty");
        std::fs::write(dir.join("notes.txt"), b"not a font").unwrap();

        let loaded = UiFonts::load(dir.to_str().unwrap());
        std::fs::remove_dir_all(&dir).unwrap();

        assert!(matches!(loaded.err().unwrap(), UiFontError::NoFonts { .. }));
    }

    #[test]
    fn the_fonts_can_be_loaded_on_another_thread() {
        let handle = std::thread::spawn(|| UiFonts::from_font_files("dir", vec![font_bytes()]));
        let fonts = handle.join().unwrap().unwrap();
        assert!(fonts.require_family(&family()).is_ok());
    }
}
