//! The rollover widget of the HUD movie: the name of what the crosshair rests on, centred above a
//! one-hint button bar (the key and its verb). The movie places both relative to the centre of the
//! stage, where the crosshair is; the hint row is centred on the bar by the rules of the shared
//! button hint component. The widget draws neither brackets nor a shaded background: the movie
//! turns both off for its hint bar, and boot refuses a layout that turns one on.

use std::sync::Arc;

use super::field_text::{ScreenMap, TextExtent, TextItem};
use super::layout::Layout;
use super::meters;
use super::text_face::TextFace;
use crate::rendering::door_prompt::RolloverPrompt;
use crate::rendering::ui_colour::UiTint;
use crate::ui_layout::button_hint::{ButtonHintKeys, ButtonHintRules};
use crate::ui_layout::scaleform_text::{AutoSize, VerticalAlign};
use crate::ui_layout::set_text::{SetTextRules, upper_case};
use crate::ui_layout::{
    LiteralKey, PlacementKey, TextKey, UiLayout, UiLayoutError, key_check_fields,
};

const MOVIE: &str = meters::MOVIE;
const BAR_PATH: &str = "CenterGroup_mc/RolloverWidget_mc/ButtonHintBar_mc";

pub struct RolloverKeys {
    centre: PlacementKey,
    name: TextKey,
    bar: PlacementKey,
    hint: ButtonHintKeys,
    key_name: LiteralKey,
    name_vertical_align: LiteralKey,
    shows_brackets: LiteralKey,
    shows_backdrop: LiteralKey,
}

pub static KEYS: RolloverKeys = RolloverKeys {
    centre: PlacementKey::new(MOVIE, "CenterGroup_mc"),
    name: TextKey::new(MOVIE, "CenterGroup_mc/RolloverWidget_mc/RolloverName_tf"),
    bar: PlacementKey::new(MOVIE, BAR_PATH),
    hint: ButtonHintKeys::of(MOVIE),
    key_name: LiteralKey::texts(MOVIE, "RolloverWidget.<init>#new BSButtonHintData(1)", 2),
    name_vertical_align: LiteralKey::texts(
        MOVIE,
        "RolloverWidget.<init>#TextFieldEx.setVerticalAlign(1)",
        1,
    ),
    shows_brackets: LiteralKey::flags(
        MOVIE,
        "RolloverWidget.__setProp_ButtonHintBar_mc_RolloverWidget_ButtonHintBar_mc_0#ButtonHintBar_mc.bShowBrackets=",
        1,
    ),
    shows_backdrop: LiteralKey::flags(
        MOVIE,
        "RolloverWidget.__setProp_ButtonHintBar_mc_RolloverWidget_ButtonHintBar_mc_0#ButtonHintBar_mc.bUseShadedBackground=",
        1,
    ),
};

key_check_fields!(RolloverKeys {
    centre,
    name,
    bar,
    hint,
    key_name,
    name_vertical_align,
    shows_brackets,
    shows_backdrop,
});

/// The vertical alignment the constructor sets on the name field: the movie's `VALIGN_*` constant
/// values. Centring is not drawn.
fn vertical_align(layout: &UiLayout) -> Result<VerticalAlign, UiLayoutError> {
    let key = &KEYS.name_vertical_align;
    match layout.lit_string(key, 0)?.as_str() {
        "top" | "none" => Ok(VerticalAlign::Top),
        "bottom" => Ok(VerticalAlign::Bottom),
        other => Err(UiLayoutError::unresolved(
            MOVIE,
            key.path,
            format!("the vertical alignment '{other}' is not one the HUD draws"),
        )),
    }
}

/// Stand-in S4: the name is shown upper-case (observed), but the movie's code hands it to the
/// field as it is, so the case change is not in the layout data.
fn name_text(name: &str) -> String {
    upper_case(name)
}

/// The rollover's values from the layout, resolved once. Positions are stage pixels of the HUD
/// movie.
#[derive(Debug, Clone, PartialEq)]
pub struct RolloverSpec {
    /// The stage point the crosshair sits at.
    centre: (f32, f32),
    name: TextFace,
    key: TextFace,
    label: TextFace,
    /// The registration point of the hint bar.
    bar: (f32, f32),
    key_name: String,
    hint: ButtonHintRules,
    set_text: SetTextRules,
}

impl RolloverSpec {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        for setting in [&KEYS.shows_brackets, &KEYS.shows_backdrop] {
            if layout.lit_flag(setting, 0)? {
                return Err(UiLayoutError::unresolved(
                    MOVIE,
                    setting.path,
                    "the rollover's hint bar draws brackets or a shaded background, which the HUD does not draw",
                ));
            }
        }
        let hint = KEYS.hint.resolve(layout)?;
        let centre = layout.get(&KEYS.centre)?.world.matrix;
        let bar = layout.get(&KEYS.bar)?.world.matrix;
        let key_face = TextFace::resolve(layout, &KEYS.hint.key_field, Some(&hint.key_font))?;
        let name = TextFace::resolve(layout, &KEYS.name, None)?;
        Ok(Self {
            centre: (centre.tx, centre.ty),
            name: TextFace {
                geometry: name.geometry.with_valign(vertical_align(layout)?),
                ..name
            },
            key: TextFace {
                geometry: key_face.geometry.translated(0.0, hint.key_y_offset),
                ..key_face
            },
            label: TextFace::resolve(layout, &KEYS.hint.label, None)?,
            bar: (bar.tx, bar.ty),
            key_name: layout.lit_string(&KEYS.key_name, 0)?,
            hint,
            set_text: meters::SET_TEXT.resolve(layout)?,
        })
    }

    /// The families of the faces the widget draws in.
    pub fn families(&self) -> [&str; 3] {
        [&self.name.family, &self.key.family, &self.label.family].map(String::as_str)
    }

    /// The text items of `prompt` on a framebuffer laid out by `layout`. `measure` reports how
    /// much room a text takes, which the hint row needs to centre itself.
    fn items(
        &self,
        prompt: &RolloverPrompt,
        ui: &UiLayout,
        layout: &Layout,
        tint: &UiTint,
        mut measure: impl FnMut(&TextItem) -> TextExtent,
    ) -> Vec<TextItem> {
        let scale = layout.scale();
        let (centre_x, centre_y) = layout.center();
        let map = ScreenMap {
            origin: (
                centre_x - scale * self.centre.0,
                centre_y - scale * self.centre.1,
            ),
            scale,
        };
        let name_geometry = self.name.geometry;
        let name = TextItem {
            wrap_width: name_geometry
                .wraps
                .then(|| layout.s(name_geometry.content_width())),
            ..self
                .name
                .item(name_geometry, name_text(&prompt.name), map, tint)
        };

        let key_text = self.hint.key_caption(&self.key_name);
        let label_text = self.hint.label_text(&self.set_text, ui, &prompt.verb);
        let mut box_width = |face: &TextFace, text: &str| {
            let item = face.item(face.geometry, text.to_owned(), map, tint);
            let natural = measure(&item).width / scale;
            let sized = face.geometry.auto_sized(AutoSize::Left, natural);
            sized.right - sized.left
        };
        let key_box = box_width(&self.key, &key_text);
        let label_box = box_width(&self.label, &label_text);
        let label_shift = self.hint.label_shift(key_box);
        let left = self
            .key
            .geometry
            .left
            .min(self.label.geometry.left + label_shift);
        let right = (self.key.geometry.left + key_box)
            .max(self.label.geometry.left + label_shift + label_box);
        let row_x = self.bar.0 - (right - left) / self.hint.centre_divisor;
        vec![
            name,
            self.key.item(
                self.key.geometry.translated(row_x, self.bar.1),
                key_text,
                map,
                tint,
            ),
            self.label.item(
                self.label
                    .geometry
                    .translated(row_x + label_shift, self.bar.1),
                label_text,
                map,
                tint,
            ),
        ]
    }
}

/// What the widget last built its items for.
struct Shown {
    prompt: RolloverPrompt,
    fb: (u32, u32),
}

/// The rollover's text items, rebuilt only when the prompt or the framebuffer size changes.
pub struct RolloverWidget {
    spec: RolloverSpec,
    ui: Arc<UiLayout>,
    shown: Option<Shown>,
    items: Vec<TextItem>,
}

impl RolloverWidget {
    pub fn new(spec: RolloverSpec, ui: Arc<UiLayout>) -> Self {
        Self {
            spec,
            ui,
            shown: None,
            items: Vec::new(),
        }
    }

    pub fn items(&self) -> &[TextItem] {
        &self.items
    }

    fn is_current(&self, prompt: Option<&RolloverPrompt>, fb: (u32, u32)) -> bool {
        match (&self.shown, prompt) {
            (None, None) => true,
            (Some(shown), Some(prompt)) => shown.prompt == *prompt && shown.fb == fb,
            _ => false,
        }
    }

    /// Shows `prompt` (or nothing) on a framebuffer of `fb` pixels laid out by `layout`; true
    /// when the items changed.
    pub fn update(
        &mut self,
        prompt: Option<&RolloverPrompt>,
        fb: (u32, u32),
        layout: &Layout,
        tint: &UiTint,
        measure: impl FnMut(&TextItem) -> TextExtent,
    ) -> bool {
        if self.is_current(prompt, fb) {
            return false;
        }
        self.items = prompt.map_or_else(Vec::new, |prompt| {
            self.spec.items(prompt, &self.ui, layout, tint, measure)
        });
        self.shown = prompt.map(|prompt| Shown {
            prompt: prompt.clone(),
            fb,
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_layout::test_support::{
        FIXTURE_REGULAR_CLASS, assert_close, instance, set_text_parts, synthesized_layout,
    };
    use serde_json::{Value, json};

    const BAR_SETTING: &str = "RolloverWidget.__setProp_ButtonHintBar_mc_RolloverWidget_ButtonHintBar_mc_0#ButtonHintBar_mc";
    const NAME_PATH: &str = "CenterGroup_mc/RolloverWidget_mc/RolloverName_tf";
    const KEY_PATH: &str = "[Shared.AS3.BSButtonHint]/IconHolderInstance/IconAnimInstance/Icon_tf";
    const LABEL_PATH: &str = "[Shared.AS3.BSButtonHint]/textField_tf";
    const CHAR_WIDTH: f32 = 10.0;
    const FIELD_INSET: f32 = 4.0;
    const CENTRE_DIVISOR: f32 = 3.0;

    #[derive(Clone, Copy)]
    struct Variant {
        brackets: bool,
        backdrop: bool,
        upper_case_labels: bool,
        vertical_align: &'static str,
    }

    const BASE: Variant = Variant {
        brackets: false,
        backdrop: false,
        upper_case_labels: true,
        vertical_align: "bottom",
    };

    fn text_character(bounds: [f32; 4], align: &str, wraps: bool) -> Value {
        json!({
            "kind": "text", "bounds": bounds, "font": FIXTURE_REGULAR_CLASS, "size": 20,
            "color": [255, 255, 255, 255], "align": align,
            "flags": {"wordWrap": wraps, "multiline": wraps, "autoSize": false, "html": false},
        })
    }

    /// The key field sits at y 5 in its clip and the code moves it down by 4; the key is
    /// "K" and its suffix ">", and rows are centred by a divisor of 3.
    fn fixture(variant: Variant) -> Arc<UiLayout> {
        let literal = |values: Value| json!({"values": values});
        let hint = "Shared.AS3.BSButtonHint";
        let bar = "Shared.AS3.BSButtonHintBar.redrawUIComponent#";
        let setting = format!("{BAR_SETTING}.");
        let overlay = json!({
            "characters": {
                "10": text_character([-2.0, -2.0, 102.0, 52.0], "center", true),
                "11": text_character([-2.0, -2.0, 48.0, 24.0], "left", false),
                "12": text_character([-2.0, -2.0, 98.0, 24.0], "left", false),
            },
            "instances": {
                "CenterGroup_mc": instance(1, "sprite", 640.0, 360.0),
                BAR_PATH: instance(1, "sprite", 893.0, 388.0),
                NAME_PATH: instance(10, "text", 733.0, 286.0),
                KEY_PATH: instance(11, "text", 2.0, 5.0),
                LABEL_PATH: instance(12, "text", 2.0, 5.0),
            },
            "literals": {
                "RolloverWidget.<init>#new BSButtonHintData(1)": literal(json!(["K", "R"])),
                "RolloverWidget.<init>#TextFieldEx.setVerticalAlign(1)":
                    literal(json!([variant.vertical_align])),
                format!("{setting}bShowBrackets="): literal(json!([variant.brackets])),
                format!("{setting}bUseShadedBackground="): literal(json!([variant.backdrop])),
                format!("{hint}.GetExpectedFont#$1="): literal(json!(vec![FIXTURE_REGULAR_CLASS; 3])),
                format!("{hint}.get:PCKey#_buttonHintData.PCKey+"): literal(json!([">"])),
                format!("{hint}.UpdateIconTextField#$5="): literal(json!([4, 0])),
                format!("{hint}.redrawPrimaryButton#IconHolderInstance.x="): literal(json!([0])),
                format!("{hint}.redrawTextField#GlobalFunc.SetText(3)"):
                    literal(json!([variant.upper_case_labels])),
                format!("{bar}$6.width+"): literal(json!([20])),
                format!("{bar}-(ButtonHintBarInternal_mc.width)/"):
                    literal(json!([CENTRE_DIVISOR])),
            },
        });
        Arc::new(synthesized_layout(
            &[&KEYS, &meters::SET_TEXT],
            &[(MOVIE, set_text_parts()), (MOVIE, overlay)],
        ))
    }

    fn spec_of(variant: Variant) -> RolloverSpec {
        RolloverSpec::resolve(&fixture(variant)).expect("the fixture resolves")
    }

    fn prompt(name: &str, verb: &str) -> RolloverPrompt {
        RolloverPrompt {
            name: name.into(),
            verb: verb.into(),
        }
    }

    fn fake_measure(item: &TextItem) -> TextExtent {
        TextExtent {
            left: 0.0,
            top: 0.0,
            width: item.text.chars().count() as f32 * CHAR_WIDTH * item.map.scale,
            height: 0.0,
        }
    }

    const TINT: UiTint = UiTint {
        color: [1.0; 3],
        opacity: 1.0,
    };

    fn hud_layout(fb: (u32, u32)) -> Layout {
        Layout::for_stage(1280.0, 720.0, fb.0 as f32, fb.1 as f32)
    }

    fn items_of(variant: Variant, fb: (u32, u32), prompt: &RolloverPrompt) -> Vec<TextItem> {
        spec_of(variant).items(
            prompt,
            &fixture(variant),
            &hud_layout(fb),
            &TINT,
            fake_measure,
        )
    }

    fn items_at(fb: (u32, u32), prompt: &RolloverPrompt) -> Vec<TextItem> {
        items_of(BASE, fb, prompt)
    }

    #[test]
    fn the_spec_reads_the_movie_the_widget_is_authored_in() {
        let spec = spec_of(BASE);
        assert_eq!(spec.centre, (640.0, 360.0));
        assert_eq!(spec.bar, (893.0, 388.0));
        assert_eq!(spec.key_name, "K");
        assert_eq!(spec.name.geometry.valign, VerticalAlign::Bottom);
        assert_eq!(spec.key.geometry.valign, VerticalAlign::Top);
    }

    #[test]
    fn a_layout_that_draws_brackets_for_the_rollover_is_refused() {
        let error = RolloverSpec::resolve(&fixture(Variant {
            brackets: true,
            ..BASE
        }))
        .unwrap_err();
        assert!(error.to_string().contains("bShowBrackets"), "{error}");
    }

    #[test]
    fn a_layout_that_shades_the_rollover_background_is_refused() {
        let error = RolloverSpec::resolve(&fixture(Variant {
            backdrop: true,
            ..BASE
        }))
        .unwrap_err();
        assert!(
            error.to_string().contains("bUseShadedBackground"),
            "{error}"
        );
    }

    #[test]
    fn the_name_field_is_anchored_as_the_constructor_sets() {
        for (literal, expected) in [
            ("bottom", VerticalAlign::Bottom),
            ("top", VerticalAlign::Top),
        ] {
            let spec = spec_of(Variant {
                vertical_align: literal,
                ..BASE
            });
            assert_eq!(spec.name.geometry.valign, expected);
        }
        let error = RolloverSpec::resolve(&fixture(Variant {
            vertical_align: "center",
            ..BASE
        }))
        .unwrap_err();
        assert!(error.to_string().contains("center"), "{error}");
    }

    #[test]
    fn the_name_box_sits_at_the_stage_centre_plus_the_widget_offset() {
        for fb in [(1280_u32, 720_u32), (3440, 1440)] {
            let name = &items_at(fb, &prompt("Door", "Open"))[0];
            let scale = hud_layout(fb).scale();
            let (x, y) = name
                .map
                .point(name.geometry.content_left(), name.geometry.line_top());
            assert_close(x, fb.0 as f32 / 2.0 + 93.0 * scale);
            assert_close(y, fb.1 as f32 / 2.0 - 74.0 * scale);
            assert_close(name.wrap_width.unwrap(), 100.0 * scale);
        }
    }

    #[test]
    fn the_hint_row_starts_its_width_over_the_centre_divisor_left_of_the_bar() {
        let items = items_at((1280, 720), &prompt("Door", "Open"));
        let (key, label) = (&items[1], &items[2]);
        let key_box = 2.0 * CHAR_WIDTH + FIELD_INSET;
        let label_box = 4.0 * CHAR_WIDTH + FIELD_INSET;
        assert_close(
            key.geometry.left,
            893.0 - (key_box + label_box) / CENTRE_DIVISOR,
        );
        assert_close(key.geometry.left + key_box, label.geometry.left);
    }

    #[test]
    fn the_key_sits_where_the_code_moves_it_and_the_label_where_it_is_authored() {
        let items = items_at((1280, 720), &prompt("Door", "Open"));
        let (key, label) = (&items[1], &items[2]);
        assert_close(key.geometry.top, 388.0 + (5.0 - 2.0) + 4.0);
        assert_close(label.geometry.top, 388.0 + (5.0 - 2.0));
    }

    #[test]
    fn a_longer_verb_moves_the_key_left_by_its_extra_width_over_the_divisor() {
        let short = items_at((1280, 720), &prompt("Door", "Open"));
        let long = items_at((1280, 720), &prompt("Door", "Close it"));
        assert_close(
            short[1].geometry.left - long[1].geometry.left,
            4.0 * CHAR_WIDTH / CENTRE_DIVISOR,
        );
    }

    #[test]
    fn name_and_verb_are_upper_case_and_the_key_is_captioned_as_the_movie_says() {
        let items = items_at((1280, 720), &prompt("Concord Speakeasy", "Open"));
        let texts: Vec<&str> = items.iter().map(|item| item.text.as_str()).collect();
        assert_eq!(texts, ["CONCORD SPEAKEASY", "K>", "OPEN"]);
    }

    #[test]
    fn the_verb_keeps_its_case_when_the_component_does_not_upper_case() {
        let items = items_of(
            Variant {
                upper_case_labels: false,
                ..BASE
            },
            (1280, 720),
            &prompt("Door", "Open"),
        );
        assert_eq!(items[2].text, "Open");
    }

    #[test]
    fn a_verb_that_is_a_translate_key_is_translated_like_any_hint_label() {
        let items = items_at((1280, 720), &prompt("Door", "$FIXTURE_A"));
        assert_eq!(items[2].text, "Fixture alpha");
    }

    fn update(
        widget: &mut RolloverWidget,
        prompt: Option<&RolloverPrompt>,
        fb: (u32, u32),
    ) -> bool {
        widget.update(prompt, fb, &hud_layout(fb), &TINT, fake_measure)
    }

    fn widget() -> RolloverWidget {
        RolloverWidget::new(spec_of(BASE), fixture(BASE))
    }

    #[test]
    fn nothing_is_drawn_without_a_prompt() {
        let mut widget = widget();
        assert!(!update(&mut widget, None, (1280, 720)));
        assert_eq!(widget.items(), []);
    }

    #[test]
    fn items_are_rebuilt_only_when_the_prompt_or_the_framebuffer_changes() {
        let mut widget = widget();
        assert!(update(
            &mut widget,
            Some(&prompt("Door", "Open")),
            (1280, 720)
        ));
        assert_eq!(widget.items().len(), 3);
        let same = prompt("Door", "Open");
        assert!(!update(&mut widget, Some(&same), (1280, 720)));
        let closing = prompt("Door", "Close");
        assert!(update(&mut widget, Some(&closing), (1280, 720)));
        assert!(update(&mut widget, Some(&closing), (1920, 1080)));
        assert!(update(&mut widget, None, (1920, 1080)));
        assert_eq!(widget.items(), []);
        assert!(!update(&mut widget, None, (1280, 720)));
    }

    /// The original's `COMMONWEALTH` and `E) OPEN` as measured at 1600x900 and 2560x1440, in
    /// stage units from the crosshair. The ink lies inside the line boxes by the glyphs' side
    /// bearings (up to about two stage units a side) and a frame measures to about one pixel, hence
    /// the tolerances. Baselines are checked against the pixel row nearest the authored one.
    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the converted interface and fonts"]
    fn the_installed_movie_resolves_the_rollover_and_lays_it_out_as_the_original_shows_it() {
        use crate::assets::layout::{ASSETS_DIR, fonts_dir};
        use crate::rendering::hud::field_text::{baselines_of, extent_of};
        use crate::rendering::ui_fonts::UiFonts;

        const BEARINGS: f32 = 3.5;
        const MEASURING: f32 = 1.0;
        const NOMINAL_BASELINES: [f32; 3] = [7.0, 51.0, 50.35];
        let layout = Arc::new(super::super::real_install::installed_layout());
        layout.validate(&[&KEYS]).unwrap();
        let spec = RolloverSpec::resolve(&layout).unwrap();
        let install = std::env::var(crate::assets::INSTALL_DIR_VAR).expect("OPENFO4_INSTALL_DIR");
        let fonts = UiFonts::load(&fonts_dir(&format!("{install}/{ASSETS_DIR}"))).unwrap();
        let mut font_system = fonts.font_system();
        for fb in [(1600, 900), (2560, 1440)] {
            let hud = hud_layout(fb);
            let scale = hud.scale();
            let (centre_x, centre_y) = hud.center();
            let items = spec.items(
                &prompt("Commonwealth", "Open"),
                &layout,
                &hud,
                &TINT,
                |item| extent_of(&mut font_system, item),
            );
            let boxes: Vec<[f32; 4]> = items
                .iter()
                .map(|item| {
                    let extent = extent_of(&mut font_system, item);
                    [
                        (extent.left - centre_x) / scale,
                        (extent.left + extent.width - centre_x) / scale,
                        (extent.top - centre_y) / scale,
                        (extent.top + extent.height - centre_y) / scale,
                    ]
                })
                .collect();
            let baselines: Vec<f32> = items
                .iter()
                .map(|item| (baselines_of(&mut font_system, item)[0] - centre_y) / scale)
                .collect();
            println!("{fb:?}: line boxes (x0, x1, y0, y1) {boxes:?} baselines {baselines:?}");

            let within_ink = |x0: f32, x1: f32, ink: [f32; 2]| {
                assert!(
                    x0 <= ink[0] + MEASURING && x0 >= ink[0] - BEARINGS,
                    "{fb:?}: left {x0} against ink {ink:?}"
                );
                assert!(
                    x1 >= ink[1] - MEASURING && x1 <= ink[1] + BEARINGS,
                    "{fb:?}: right {x1} against ink {ink:?}"
                );
            };
            within_ink(boxes[0][0], boxes[0][1], [186.4, 320.8]);
            within_ink(boxes[1][0], boxes[2][1], [225.6, 280.8]);
            for (index, nominal) in NOMINAL_BASELINES.into_iter().enumerate() {
                let row = (centre_y + nominal * scale).round();
                let expected = (row - centre_y) / scale;
                assert!(
                    (baselines[index] - expected).abs() < 1e-3,
                    "{fb:?}: baseline {index} is {} not {expected} (nominal {nominal})",
                    baselines[index]
                );
            }
            let (band_top, band_bottom) = (36.0, 54.4);
            for (index, box_) in boxes.iter().enumerate().skip(1) {
                assert!(
                    box_[2] <= band_top + MEASURING && box_[3] >= band_bottom - MEASURING,
                    "{fb:?}: hint line box {box_:?} against ink {band_top}..{band_bottom}"
                );
                assert!(
                    baselines[index] > band_top && baselines[index] < band_bottom,
                    "{fb:?}: baseline {} of hint text {index}",
                    baselines[index]
                );
            }
        }
    }
}
