//! The serde shapes of the layout documents (format 1) and the format gate.

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use super::error::UiLayoutError;
use super::values::{Align, Fill, Line, LocalPlacement, Scalar, ShapePath, TrackKey, World};

pub const SUPPORTED_FORMAT: u32 = 1;

pub const MOVIE_KIND: &str = "movie";
pub const TRANSLATE_KIND: &str = "translate";
pub const FONT_CONFIG_KIND: &str = "fontconfig";

#[derive(Deserialize)]
struct Header {
    format: Option<u32>,
    kind: Option<String>,
}

/// Parses `json` as a document of `kind`, rejecting every format other than [`SUPPORTED_FORMAT`]
/// before the body is read.
pub fn parse<T: serde::de::DeserializeOwned>(
    document: &str,
    kind: &str,
    json: &str,
) -> Result<T, UiLayoutError> {
    let bad = |reason: String| UiLayoutError::BadFormat {
        movie: document.to_owned(),
        reason,
    };
    let header: Header = serde_json::from_str(json).map_err(|error| bad(error.to_string()))?;
    match header.format {
        Some(SUPPORTED_FORMAT) => {}
        Some(other) => {
            return Err(bad(format!(
                "format {other}, the engine reads format {SUPPORTED_FORMAT}"
            )));
        }
        None => return Err(bad("it has no format number".to_owned())),
    }
    if header.kind.as_deref() != Some(kind) {
        return Err(bad(format!(
            "its kind is {:?}, expected '{kind}'",
            header.kind
        )));
    }
    serde_json::from_str(json).map_err(|error| bad(error.to_string()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovieDocument {
    pub stage: [f32; 4],
    pub fps: f32,
    pub frame_count: u32,
    pub labels: BTreeMap<String, u32>,
    pub symbols: HashMap<String, u32>,
    pub characters: HashMap<String, CharacterDocument>,
    pub instances: HashMap<String, InstanceDocument>,
    pub constants: HashMap<String, Scalar>,
    pub literals: HashMap<String, LiteralDocument>,
    pub unresolved: HashMap<String, String>,
    #[serde(default)]
    pub super_classes: HashMap<String, String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CharacterDocument {
    Shape(ShapeDocument),
    Sprite(SpriteDocument),
    Text(Box<TextDocument>),
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeDocument {
    pub bounds: [f32; 4],
    pub edge_bounds: Option<[f32; 4]>,
    pub fills: Vec<Fill>,
    pub lines: Vec<Line>,
    pub paths: Vec<ShapePath>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpriteDocument {
    /// The class the sprite's symbol is exported under.
    pub class: Option<String>,
    pub frame_count: u32,
    pub labels: BTreeMap<String, u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDocument {
    pub bounds: [f32; 4],
    pub font: Option<String>,
    pub size: Option<f32>,
    pub color: Option<[u8; 4]>,
    pub align: Option<Align>,
    pub left_margin: Option<f32>,
    pub right_margin: Option<f32>,
    pub indent: Option<f32>,
    pub leading: Option<f32>,
    pub flags: TextFlags,
    pub initial_text: Option<String>,
    pub html_format: Option<HashMap<String, Scalar>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextFlags {
    pub word_wrap: bool,
    pub multiline: bool,
    pub auto_size: bool,
    pub html: bool,
}

#[derive(Deserialize)]
pub struct InstanceDocument {
    pub char: u32,
    pub kind: String,
    pub depth: i32,
    pub name: Option<String>,
    pub frames: [u32; 2],
    pub local: LocalPlacement,
    pub world: World,
    pub track: Option<Vec<TrackKey>>,
}

#[derive(Deserialize)]
pub struct LiteralDocument {
    pub values: Vec<Scalar>,
}

#[derive(Deserialize)]
pub struct TranslateDocument {
    pub entries: HashMap<String, String>,
    pub unresolved: HashMap<String, String>,
}

#[derive(Deserialize)]
pub struct FontConfigDocument {
    pub maps: Vec<FontMap>,
    pub fonts: Vec<FontFace>,
}

/// One face's vertical metrics in its font library's units.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FontFace {
    pub family: String,
    pub style: String,
    pub em_size: f32,
    pub ascent: f32,
    pub descent: f32,
}

#[derive(Deserialize)]
pub struct FontMap {
    pub class: String,
    pub family: String,
    pub style: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installer_source;

    #[test]
    fn the_supported_format_is_the_one_the_installer_writes() {
        let installer =
            include_str!("../../../installer/file_converters/ui_layout/ui_document_header.cs");
        assert_eq!(
            installer_source::constant(installer, "Format"),
            SUPPORTED_FORMAT.to_string()
        );
    }
}
