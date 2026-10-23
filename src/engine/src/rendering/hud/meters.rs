//! HP and AP meter bars of the HUD movie: a solid fill bar, the bracket that frames the bar's
//! maximum extent and a text label. Every geometry value comes from the converted `HUDMenu`
//! movie; HP fills from the left at the bottom-left, AP from the right at the bottom-right.

use super::field_text::{ScreenMap, Shadow, TextItem};
use super::layout::{Anchor, Layout};
use super::quad::{PxRect, QuadBuilder};
use super::shadow;
use super::shape::{StageRect, shown_fill, stage_rect};
use crate::rendering::ui_colour::UiTint;
use crate::sim::MeterFill;
use crate::ui_layout::scaleform_text::FieldGeometry;
use crate::ui_layout::set_text::SetTextKeys;
use crate::ui_layout::{
    LiteralKey, ShapeKey, TextKey, TranslateKey, UiLayout, UiLayoutError, key_check_fields,
};

pub const MOVIE: &str = "HUDMenu";

pub static SET_TEXT: SetTextKeys = SetTextKeys::new(MOVIE);

/// Where the meter's fill grows from, as the movie's meter component states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Justify {
    Left,
    Right,
}

pub struct MeterKeys {
    bar: ShapeKey,
    bracket: ShapeKey,
    label: TextKey,
    label_text: TranslateKey,
    justification: LiteralKey,
}

pub static HP_KEYS: MeterKeys = MeterKeys {
    bar: ShapeKey::new(
        MOVIE,
        "LeftMeters_mc/HPMeter_mc/MeterBar_mc/MeterBarInternal_mc/@1/@1",
    ),
    bracket: ShapeKey::new(MOVIE, "LeftMeters_mc/HPMeter_mc/Bracket_mc/@1"),
    label: TextKey::new(MOVIE, "LeftMeters_mc/HPMeter_mc/DisplayText_tf"),
    label_text: TranslateKey::text("$HP"),
    justification: LiteralKey::texts(
        MOVIE,
        "HUDMenu_fla.HPMeter_41.__setProp_MeterBar_mc_HPMeter_MeterBar_mc_0#MeterBar_mc.Justification=",
        1,
    ),
};

pub static AP_KEYS: MeterKeys = MeterKeys {
    bar: ShapeKey::new(
        MOVIE,
        "RightMeters_mc/ActionPointMeter_mc/MeterBar_mc/MeterBarInternal_mc/@1/@1",
    ),
    bracket: ShapeKey::new(MOVIE, "RightMeters_mc/ActionPointMeter_mc/Bracket_mc/@1"),
    label: TextKey::new(MOVIE, "RightMeters_mc/ActionPointMeter_mc/DisplayText_tf"),
    label_text: TranslateKey::text("$AP"),
    justification: LiteralKey::texts(
        MOVIE,
        "ActionPointMeter.__setProp_MeterBar_mc_ActionPointMeter_MeterBar_mc_0#MeterBar_mc.Justification=",
        1,
    ),
};

key_check_fields!(MeterKeys {
    bar,
    bracket,
    label,
    label_text,
    justification
});

#[derive(Debug, Clone, PartialEq)]
pub struct LabelSpec {
    /// The text as the game's text-setting rule shows it.
    pub text: String,
    pub family: String,
    pub geometry: FieldGeometry,
    pub colour: [u8; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct MeterSpec {
    pub corner: Anchor,
    pub bar: StageRect,
    /// The bar's distance from the stage edge on its corner's side, and from the stage bottom.
    pub inset_x: f32,
    pub inset_bottom: f32,
    /// How far the bracket's feet stand outside the bar, and the thickness of its rail and feet.
    /// Stand-in S3: the layout holds the bracket shape's bounds only, so the foot width is taken
    /// to be the rail thickness.
    pub overhang: f32,
    pub thickness: f32,
    pub fill: [f32; 4],
    pub justify: Justify,
    pub label: LabelSpec,
}

const EDGE_TOLERANCE_PX: f32 = 1e-3;

impl MeterSpec {
    pub fn resolve(
        layout: &UiLayout,
        keys: &MeterKeys,
        corner: Anchor,
        stage: [f32; 4],
    ) -> Result<Self, UiLayoutError> {
        let [stage_left, _, stage_right, stage_bottom] = stage;
        let bar_shape = layout.get(&keys.bar)?;
        let bar = stage_rect(MOVIE, keys.bar.path, &bar_shape)?;
        let bracket = stage_rect(MOVIE, keys.bracket.path, &layout.get(&keys.bracket)?)?;
        let overhang = bar.x - bracket.x;
        let thickness = bracket.bottom() - bar.bottom();
        let symmetric = (bracket.right() - bar.right() - overhang).abs() <= EDGE_TOLERANCE_PX;
        let level = (bracket.y - bar.y).abs() <= EDGE_TOLERANCE_PX;
        if overhang < 0.0 || thickness <= 0.0 || !symmetric || !level {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                keys.bracket.path,
                "the bracket is not a rail below the bar with feet standing equally outside it",
            ));
        }
        let justify = match layout.lit_string(&keys.justification, 0)?.as_str() {
            "left" => Justify::Left,
            "right" => Justify::Right,
            other => {
                return Err(UiLayoutError::unresolved(
                    MOVIE,
                    keys.justification.path,
                    format!("justification '{other}' is neither left nor right"),
                ));
            }
        };
        let inset_x = match corner {
            Anchor::Bl => bar.x - stage_left,
            Anchor::Br => stage_right - bar.right(),
        };
        Ok(Self {
            corner,
            bar,
            inset_x,
            inset_bottom: stage_bottom - bar.bottom(),
            overhang,
            thickness,
            fill: shown_fill(MOVIE, keys.bar.path, &bar_shape)?,
            justify,
            label: LabelSpec::resolve(layout, keys)?,
        })
    }
}

impl LabelSpec {
    fn resolve(layout: &UiLayout, keys: &MeterKeys) -> Result<Self, UiLayoutError> {
        let field = layout.get(&keys.label)?;
        let colour = field.color.ok_or_else(|| {
            UiLayoutError::unresolved(MOVIE, keys.label.path, "the text field has no colour")
        })?;
        let rules = SET_TEXT.resolve(layout)?;
        Ok(Self {
            text: rules.apply(layout, keys.label_text.path, false, false),
            family: layout.font_family(&field.font)?,
            geometry: FieldGeometry::of(&field, layout.font_metrics(&field.font)?),
            colour: [colour[0], colour[1], colour[2]],
        })
    }
}

/// The meter bar in framebuffer pixels.
pub fn bar_rect(layout: &Layout, spec: &MeterSpec) -> PxRect {
    layout.place(
        spec.corner,
        spec.bar.w,
        spec.bar.h,
        spec.inset_x,
        spec.inset_bottom,
    )
}

/// The label of a meter whose bar is at `bar`: the field keeps its place relative to the bar.
pub fn label_item(layout: &Layout, spec: &MeterSpec, bar: PxRect, tint: &UiTint) -> TextItem {
    let scale = layout.scale();
    let label = &spec.label;
    let [_, _, _, alpha] = tint.text_rgba();
    let [r, g, b] = tint.apply_to(label.colour);
    TextItem {
        text: label.text.clone(),
        family: label.family.clone(),
        geometry: label.geometry,
        map: ScreenMap {
            origin: (bar.x - scale * spec.bar.x, bar.y - scale * spec.bar.y),
            scale,
        },
        shrink: false,
        colour: [r, g, b, alpha],
        shadow: Some(Shadow {
            offset: label.geometry.size * scale * shadow::TEXT_OFFSET_EM,
            colour: [0, 0, 0, tint.shadow_alpha(shadow::TEXT_ALPHA)],
        }),
    }
}

/// Emits a meter: the bracket, then the fill.
pub fn emit(
    qb: &mut QuadBuilder,
    layout: &Layout,
    spec: &MeterSpec,
    bar: PxRect,
    meter: MeterFill,
) {
    let scale = layout.scale();
    let thickness = scale * spec.thickness;
    let overhang = scale * spec.overhang;
    let colour = spec.fill;
    qb.rect(
        PxRect {
            x: bar.x - overhang,
            y: bar.y,
            w: thickness,
            h: bar.h,
        },
        colour,
    );
    qb.rect(
        PxRect {
            x: bar.x + bar.w,
            y: bar.y,
            w: thickness,
            h: bar.h,
        },
        colour,
    );
    qb.rect(
        PxRect {
            x: bar.x - overhang,
            y: bar.y + bar.h,
            w: bar.w + 2.0 * overhang,
            h: thickness,
        },
        colour,
    );

    let fill_w = bar.w * meter.fraction;
    let fill_x = match spec.justify {
        Justify::Left => bar.x,
        Justify::Right => bar.x + bar.w - fill_w,
    };
    qb.rect(
        PxRect {
            x: fill_x,
            w: fill_w,
            ..bar
        },
        colour,
    );
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the fixture arithmetic stays on exactly representable values"
)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::rendering::hud::quad::HudVertex;
    use crate::ui_layout::test_support::{
        assert_close, font_config_json, instance, movie_json, set_text_parts, text_character,
    };

    fn translate() -> String {
        json!({
            "format": 1, "extractor": 1, "kind": "translate", "language": "en",
            "entries": {"$HP": "Hit", "$AP": "Action"}, "unresolved": {},
        })
        .to_string()
    }

    fn placed(char: u32, kind: &str, matrix: [f64; 6]) -> Value {
        let mut value = instance(char, kind, 0.0, 0.0);
        value["world"]["matrix"] = json!(matrix);
        value["local"]["matrix"] = json!(matrix);
        value
    }

    fn justification(class: &str, side: &str) -> (String, Value) {
        (
            format!(
                "{class}.__setProp_MeterBar_mc_{}_MeterBar_mc_0#MeterBar_mc.Justification=",
                class.rsplit('.').next().unwrap().split('_').next().unwrap()
            ),
            json!({"values": [side]}),
        )
    }

    const HP_CLASS: &str = "HUDMenu_fla.HPMeter_41";
    const AP_CLASS: &str = "ActionPointMeter";

    /// A 640 x 360 stage with a 100 x 6 bar at the bottom of each side.
    fn parts(bracket_right: f64) -> Value {
        let set_text = set_text_parts();
        let mut literals = set_text["literals"].clone();
        for (key, value) in [
            justification(HP_CLASS, "left"),
            justification(AP_CLASS, "right"),
        ] {
            literals[key] = value;
        }
        let bar = |tx| placed(2, "shape", [100.0 / 6.0, 0.0, 0.0, 1.0, tx, 303.0]);
        let bracket = |tx| placed(3, "shape", [1.0, 0.0, 0.0, 1.0, tx, 300.0]);
        let label = |tx| placed(1, "text", [1.0, 0.0, 0.0, 1.0, tx, 290.0]);
        json!({
            "fps": 30, "stage": [0, 0, 640, 360],
            "constants": set_text["constants"],
            "literals": literals,
            "characters": {
                "1": text_character([-2.0, -2.0, 40.0, 30.0]),
                "2": {"kind": "shape", "bounds": [-3, -3, 3, 3],
                      "fills": [{"type": "solid", "color": [255, 255, 255, 255]}], "lines": [], "paths": []},
                "3": {"kind": "shape", "bounds": [-2, 0, bracket_right, 8],
                      "fills": [{"type": "solid", "color": [255, 255, 255, 255]}], "lines": [], "paths": []},
            },
            "instances": {
                "LeftMeters_mc/HPMeter_mc/MeterBar_mc/MeterBarInternal_mc/@1/@1": bar(100.0),
                "LeftMeters_mc/HPMeter_mc/Bracket_mc/@1": bracket(50.0),
                "LeftMeters_mc/HPMeter_mc/DisplayText_tf": label(10.0),
                "RightMeters_mc/ActionPointMeter_mc/MeterBar_mc/MeterBarInternal_mc/@1/@1": bar(530.0),
                "RightMeters_mc/ActionPointMeter_mc/Bracket_mc/@1": bracket(480.0),
                "RightMeters_mc/ActionPointMeter_mc/DisplayText_tf": label(590.0),
            },
        })
    }

    fn layout(parts: &Value) -> UiLayout {
        UiLayout::from_documents(
            &translate(),
            &font_config_json(),
            &[(MOVIE, &movie_json(MOVIE, parts))],
        )
        .unwrap()
    }

    const STAGE: [f32; 4] = [0.0, 0.0, 640.0, 360.0];

    fn hp(layout: &UiLayout) -> Result<MeterSpec, UiLayoutError> {
        MeterSpec::resolve(layout, &HP_KEYS, Anchor::Bl, STAGE)
    }

    fn ap(layout: &UiLayout) -> Result<MeterSpec, UiLayoutError> {
        MeterSpec::resolve(layout, &AP_KEYS, Anchor::Br, STAGE)
    }

    #[test]
    fn a_meter_is_read_from_its_bar_bracket_and_label() {
        let spec = hp(&layout(&parts(102.0))).unwrap();
        assert_close(spec.bar.x, 50.0);
        assert_close(spec.bar.w, 100.0);
        assert_close(spec.inset_x, 50.0);
        assert_close(spec.inset_bottom, 54.0);
        assert_close(spec.overhang, 2.0);
        assert_close(spec.thickness, 2.0);
        assert_eq!(spec.justify, Justify::Left);
        assert_eq!(spec.label.text, "Hit");
        assert_eq!(spec.label.family, "Fixture Sans");
    }

    #[test]
    fn the_right_meter_measures_its_inset_from_the_right_edge() {
        let spec = ap(&layout(&parts(102.0))).unwrap();
        assert_close(spec.inset_x, 60.0);
        assert_eq!(spec.justify, Justify::Right);
        assert_eq!(spec.label.text, "Action");
    }

    #[test]
    fn a_bracket_that_overhangs_unequally_is_rejected_naming_it() {
        let error = hp(&layout(&parts(110.0))).unwrap_err();
        assert!(error.to_string().contains("Bracket_mc"), "{error}");
    }

    #[test]
    fn a_justification_other_than_left_or_right_is_rejected() {
        let mut broken = parts(102.0);
        let (key, value) = justification(HP_CLASS, "middle");
        broken["literals"][key] = value;
        let error = hp(&layout(&broken)).unwrap_err();
        assert!(error.to_string().contains("middle"), "{error}");
    }

    #[test]
    fn the_keys_of_a_movie_without_the_meters_are_all_listed() {
        let mut empty = parts(102.0);
        empty["instances"] = json!({});
        let error = layout(&empty)
            .validate(&[&HP_KEYS, &AP_KEYS])
            .unwrap_err()
            .to_string();
        assert!(error.contains("LeftMeters_mc"), "{error}");
        assert!(error.contains("RightMeters_mc"), "{error}");
    }

    fn pixel_layout() -> Layout {
        Layout::for_stage(640.0, 360.0, 1280.0, 720.0)
    }

    #[test]
    fn the_bar_sits_at_its_inset_from_the_corner_scaled_to_the_screen() {
        let spec = hp(&layout(&parts(102.0))).unwrap();
        let bar = bar_rect(&pixel_layout(), &spec);
        assert_close(bar.x, 100.0);
        assert_close(bar.y, 600.0);
        assert_close(bar.w, 200.0);
        assert_close(bar.h, 12.0);
    }

    #[test]
    fn the_label_keeps_its_place_relative_to_the_bar() {
        let spec = hp(&layout(&parts(102.0))).unwrap();
        let screen = pixel_layout();
        let bar = bar_rect(&screen, &spec);
        let tint = UiTint {
            color: [1.0; 3],
            opacity: 1.0,
        };
        let item = label_item(&screen, &spec, bar, &tint);
        let (left, top) = item.map.point(item.geometry.left, item.geometry.top);
        assert_close(left, 2.0 * 8.0);
        assert_close(top, 2.0 * 288.0);
        assert_eq!(item.colour, [255, 255, 255, 255]);
        assert!(item.shadow.is_some());
    }

    fn emitted(spec: &MeterSpec, fraction: f32) -> Vec<HudVertex> {
        let screen = pixel_layout();
        let bar = bar_rect(&screen, spec);
        let mut verts = Vec::new();
        let mut quads = QuadBuilder::new(&mut verts, 1280.0, 720.0, 0.0, [0.0; 4]);
        emit(&mut quads, &screen, spec, bar, MeterFill { fraction });
        quads.finish();
        verts
    }

    fn x_extent(quad: &[HudVertex]) -> (f32, f32) {
        let clip: Vec<f32> = quad.iter().map(|vertex| vertex.pos[0]).collect();
        let to_pixels = |clip: f32| f32::midpoint(clip, 1.0) * 1280.0;
        (
            to_pixels(clip.iter().copied().fold(f32::MAX, f32::min)),
            to_pixels(clip.iter().copied().fold(f32::MIN, f32::max)),
        )
    }

    #[test]
    fn a_left_justified_fill_grows_from_the_left_edge() {
        let spec = hp(&layout(&parts(102.0))).unwrap();
        let verts = emitted(&spec, 0.5);
        assert_eq!(verts.len(), 24);
        let (left, right) = x_extent(&verts[18..]);
        assert_close(left, 100.0);
        assert_close(right, 200.0);
    }

    #[test]
    fn a_right_justified_fill_drains_towards_the_right_edge() {
        let spec = ap(&layout(&parts(102.0))).unwrap();
        let verts = emitted(&spec, 0.25);
        let (left, right) = x_extent(&verts[18..]);
        assert_close(right, 1160.0);
        assert_close(right - left, 50.0);
    }

    #[test]
    fn the_bracket_frames_the_bar_with_feet_and_a_rail() {
        let spec = hp(&layout(&parts(102.0))).unwrap();
        let verts = emitted(&spec, 1.0);
        let (foot_left, _) = x_extent(&verts[0..6]);
        let (_, foot_right) = x_extent(&verts[6..12]);
        let (rail_left, rail_right) = x_extent(&verts[12..18]);
        assert_close(foot_left, 96.0);
        assert_close(foot_right, 304.0);
        assert_close(rail_left, 96.0);
        assert_close(rail_right, 304.0);
    }
}
