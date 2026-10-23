//! A text field of the movie with the face and colour it draws in, resolved once at load.

use egui::Color32;

use super::rgba;
use crate::rendering::pipboy::fonts::{Weight, weight_for_class};
use crate::rendering::pipboy::paint::tinted;
use crate::rendering::pipboy::scaleform_text::FieldStyle;
use crate::ui_layout::scaleform_text::{FieldGeometry, FontMetrics};
use crate::ui_layout::values::TextField;
use crate::ui_layout::{TextKey, UiLayout, UiLayoutError};

#[derive(Clone, Debug)]
pub struct Text {
    pub field: TextField,
    pub weight: Weight,
    pub colour: Color32,
    pub metrics: FontMetrics,
}

impl Text {
    /// The field at `key`; a font class that is not a loaded face fails here, at boot.
    pub fn load(layout: &UiLayout, key: &TextKey) -> Result<Self, UiLayoutError> {
        let field = layout.get(key)?;
        Ok(Self {
            weight: weight_for_class(layout, &field.font)?,
            colour: field.color.map_or(Color32::WHITE, rgba),
            metrics: layout.font_metrics(&field.font)?,
            field,
        })
    }

    pub fn geometry(&self) -> FieldGeometry {
        FieldGeometry::of(&self.field, self.metrics)
    }

    /// The face and the field's colour multiplied with `tint`, the colour of the set the chart
    /// is drawn in.
    pub fn style(&self, tint: Color32) -> FieldStyle {
        FieldStyle::new(self.weight, tinted(self.colour, tint))
    }

    /// The field's y in its parent.
    pub fn local_y(&self) -> f32 {
        self.field.placement.local.matrix.ty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::pipboy::test_layout::pipboy_layout;

    #[test]
    fn a_text_field_carries_its_face_and_colour() {
        let key = TextKey::new(super::super::MOVIE, "[LevelUpMenu]/PerkInfo_mc/PerkName_tf");
        let text = Text::load(&pipboy_layout(), &key).unwrap();
        assert_eq!(text.style(Color32::WHITE).colour, text.colour);
        let tint = Color32::from_rgb(20, 255, 40);
        assert_eq!(text.style(tint).colour, tinted(text.colour, tint));
    }

    #[test]
    fn a_missing_field_is_an_error() {
        let key = TextKey::new(super::super::MOVIE, "[LevelUpMenu]/Nothing");
        assert!(Text::load(&pipboy_layout(), &key).is_err());
    }
}
