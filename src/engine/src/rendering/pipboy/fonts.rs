//! The two Pip-Boy typefaces, located among the installer-extracted fonts and registered as named
//! egui families in the Pip-Boy context only.

use std::path::{Path, PathBuf};

use egui::{FontData, FontDefinitions, FontFamily, FontId};
use glyphon::fontdb;

use crate::ui_layout::faces::{BOLD_FACE, REGULAR_FACE};
use crate::ui_layout::{UiLayout, UiLayoutError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    Regular,
    Bold,
}

impl Weight {
    fn family_name(self) -> &'static str {
        match self {
            Weight::Regular => "pipboy-regular",
            Weight::Bold => "pipboy-bold",
        }
    }
}

/// The loaded face a font class of the game's font configuration stands for.
pub fn weight_for_class(layout: &UiLayout, class: &str) -> Result<Weight, UiLayoutError> {
    let family = layout.font_family(class)?;
    match family.as_str() {
        REGULAR_FACE => Ok(Weight::Regular),
        BOLD_FACE => Ok(Weight::Bold),
        _ => Err(UiLayoutError::unresolved(
            crate::ui_layout::FONT_CONFIG_DOCUMENT,
            class,
            format!("family '{family}' is not a loaded Pip-Boy face"),
        )),
    }
}

pub fn font_id(weight: Weight, size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(weight.family_name().into()))
}

#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("Pip-Boy: font folder '{dir}' holds no face named '{face}'; the install is incomplete")]
    MissingFace { dir: PathBuf, face: &'static str },
    #[error("Pip-Boy: font '{face}' in '{dir}' could not be read")]
    Unreadable { dir: PathBuf, face: &'static str },
}

/// The font file bytes of both faces.
pub struct PipBoyFonts {
    regular: Vec<u8>,
    bold: Vec<u8>,
}

impl PipBoyFonts {
    pub fn new(regular: Vec<u8>, bold: Vec<u8>) -> Self {
        Self { regular, bold }
    }

    /// Finds both faces among the TTFs of `dir`.
    pub fn load(dir: &Path) -> Result<Self, FontError> {
        let mut database = fontdb::Database::new();
        database.load_fonts_dir(dir);
        Ok(Self {
            regular: face_bytes(&database, dir, REGULAR_FACE, Some(BOLD_FACE))?,
            bold: face_bytes(&database, dir, BOLD_FACE, None)?,
        })
    }

    pub fn definitions(self) -> FontDefinitions {
        let mut definitions = FontDefinitions::empty();
        for (weight, bytes) in [(Weight::Regular, self.regular), (Weight::Bold, self.bold)] {
            let name = weight.family_name();
            definitions.font_data.insert(
                name.to_owned(),
                std::sync::Arc::new(FontData::from_owned(bytes)),
            );
            definitions
                .families
                .insert(FontFamily::Name(name.into()), vec![name.to_owned()]);
        }
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            definitions
                .families
                .insert(family, vec![Weight::Regular.family_name().to_owned()]);
        }
        definitions
    }
}

/// The data of the face listing family `family` and not `excluded`.
fn face_bytes(
    database: &fontdb::Database,
    dir: &Path,
    family: &'static str,
    excluded: Option<&str>,
) -> Result<Vec<u8>, FontError> {
    let lists = |face: &fontdb::FaceInfo, name: &str| face.families.iter().any(|(f, _)| f == name);
    let face = database
        .faces()
        .find(|face| lists(face, family) && excluded.is_none_or(|other| !lists(face, other)))
        .ok_or_else(|| FontError::MissingFace {
            dir: dir.to_path_buf(),
            face: family,
        })?;
    database
        .with_face_data(face.id, |data, _| data.to_vec())
        .ok_or_else(|| FontError::Unreadable {
            dir: dir.to_path_buf(),
            face: family,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundled_font_bytes() -> Vec<u8> {
        let defaults = FontDefinitions::default();
        defaults.font_data["Ubuntu-Light"].font.to_vec()
    }

    #[test]
    fn a_folder_without_the_faces_is_a_named_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("other.ttf"), bundled_font_bytes()).unwrap();

        let error = PipBoyFonts::load(dir.path()).err().unwrap();

        assert!(matches!(
            error,
            FontError::MissingFace {
                face: REGULAR_FACE,
                ..
            }
        ));
        assert!(error.to_string().contains("Roboto Condensed"));
    }

    #[test]
    fn a_missing_folder_is_the_same_missing_face_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        assert!(matches!(
            PipBoyFonts::load(&missing),
            Err(FontError::MissingFace { .. })
        ));
    }

    #[test]
    fn face_lookup_picks_the_face_by_family_name() {
        let mut database = fontdb::Database::new();
        database.load_font_data(bundled_font_bytes());
        let dir = Path::new("test");

        let bytes = face_bytes(&database, dir, "Ubuntu", None).unwrap();

        assert_eq!(bytes, bundled_font_bytes());
        assert!(face_bytes(&database, dir, "Ubuntu", Some("Ubuntu")).is_err());
    }

    #[test]
    fn definitions_register_both_named_families_and_default_text_to_regular() {
        let fonts = PipBoyFonts::new(bundled_font_bytes(), bundled_font_bytes());
        let definitions = fonts.definitions();

        for weight in [Weight::Regular, Weight::Bold] {
            let family = font_id(weight, 20.0).family;
            assert_eq!(definitions.families[&family], vec![weight.family_name()]);
        }
        assert_eq!(
            definitions.families[&FontFamily::Proportional],
            vec!["pipboy-regular"]
        );
    }
}
