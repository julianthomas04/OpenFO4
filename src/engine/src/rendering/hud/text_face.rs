//! A HUD text field resolved from the layout: its face, place and colour, and the text item it
//! draws as.

use super::field_text::{ScreenMap, TextItem};
use super::shadow;
use crate::rendering::ui_colour::UiTint;
use crate::ui_layout::scaleform_text::FieldGeometry;
use crate::ui_layout::{TextKey, UiLayout, UiLayoutError};

#[derive(Debug, Clone, PartialEq)]
pub struct TextFace {
    pub family: String,
    pub geometry: FieldGeometry,
    pub colour: [u8; 3],
}

impl TextFace {
    /// The field `key`, in the font class `font` when the movie's code sets one.
    pub fn resolve(
        layout: &UiLayout,
        key: &TextKey,
        font: Option<&str>,
    ) -> Result<Self, UiLayoutError> {
        let field = layout.get(key)?;
        let font = font.unwrap_or(field.font.as_str());
        let colour = field.color.ok_or_else(|| {
            UiLayoutError::unresolved(key.movie, key.path, "the text field has no colour")
        })?;
        Ok(Self {
            family: layout.font_family(font)?,
            geometry: FieldGeometry::of(&field, layout.font_metrics(font)?),
            colour: [colour[0], colour[1], colour[2]],
        })
    }

    /// `text` in the field `geometry` (the face's own, or moved), single line, with the HUD's text
    /// colour and shadow.
    pub fn item(
        &self,
        geometry: FieldGeometry,
        text: String,
        map: ScreenMap,
        tint: &UiTint,
    ) -> TextItem {
        let [_, _, _, alpha] = tint.text_rgba();
        let [r, g, b] = tint.apply_to(self.colour);
        TextItem {
            text,
            family: self.family.clone(),
            geometry,
            map,
            shrink: false,
            colour: [r, g, b, alpha],
            shadow: Some(shadow::text(geometry.size, map.scale, tint)),
            wrap_width: None,
            max_height: None,
        }
    }
}
