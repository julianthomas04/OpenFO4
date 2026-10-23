//! The main menu's list layout, read from the converted `MainMenu` movie. All lengths are in
//! pixels of the movie's stage, which the engine centres on the screen and scales uniformly.
//!
//! Everything the menu draws sits in the middle of that stage, so a stage centred with letterbox
//! on any aspect ratio keeps the movie's composition; the HUD's corner-pinned safe-rect
//! anchoring has nothing to apply to.

use super::loading_icon::{self, IconPlacement};
use crate::rendering::hud::field_text::ScreenMap;
use crate::rendering::hud::layout::Layout;
use crate::rendering::hud::quad::PxRect;
use crate::rendering::hud::shape::{StageRect, shown_fill, stage_rect};
use crate::ui_layout::scaleform_text::FieldGeometry;
use crate::ui_layout::set_text::{SetTextKeys, SetTextRules};
use crate::ui_layout::{
    LiteralKey, ModuleKeys, NumberKey, PlacementKey, ShapeKey, TextKey, UiLayout, UiLayoutError,
};

pub const MOVIE: &str = "MainMenu";

const LIST_ENTRY_CLASS: &str = "MainMenuListEntry";
const SHRINK_MODE: &str = "shrink";
const MAX_COLOUR: f64 = 16_777_215.0;
/// The composed matrices of the layout are rounded to six decimals.
const ORIGIN_TOLERANCE_PX: f32 = 0.01;

static LIST_PLACEMENT: PlacementKey = PlacementKey::new(MOVIE, "Menu_mc/MainPanel_mc/List_mc");
static ROW_COUNT: LiteralKey = LiteralKey::new(
    MOVIE,
    "MainMenu_fla.MainListPanel_49.__setProp_List_mc_MainListPanel_List_0#List_mc.numListItems=",
    1,
);
static ROW_SPACING: LiteralKey = LiteralKey::new(
    MOVIE,
    "MainMenu_fla.MainListPanel_49.__setProp_List_mc_MainListPanel_List_0#List_mc.verticalSpacing=",
    1,
);
static ENTRY_CLASS: LiteralKey = LiteralKey::texts(
    MOVIE,
    "MainMenu_fla.MainListPanel_49.__setProp_List_mc_MainListPanel_List_0#List_mc.listEntryClass=",
    1,
);
static ENTRY_BORDER: ShapeKey = ShapeKey::new(MOVIE, "[MainMenuListEntry]/border/@1");
static ENTRY_TEXT: TextKey = TextKey::new(MOVIE, "[MainMenuListEntry]/textField");
static ENTRY_COLOURS: LiteralKey = LiteralKey::new(
    MOVIE,
    "MainMenuListEntry.SetEntryText#textField.textColor=",
    4,
);
static ENTRY_AUTO_SIZE: LiteralKey = LiteralKey::texts(
    MOVIE,
    "MainMenuListEntry.<init>#TextFieldEx.setTextAutoSize(1)",
    1,
);
static SELECTED_ALPHA: NumberKey = NumberKey::new(MOVIE, "Shared.GlobalFunc.SELECTED_RECT_ALPHA");
static PANEL_BACKGROUND: ShapeKey =
    ShapeKey::new(MOVIE, "Menu_mc/MainPanel_mc/MainPanelBackground_mc/@1");
pub static SET_TEXT: SetTextKeys = SetTextKeys::new(MOVIE);

/// Index of the enabled row's text colour among the four the entry states assign: the colours
/// are assigned in the order disabled-selected, disabled, selected, unselected.
const SELECTED_COLOUR: usize = 2;
const UNSELECTED_COLOUR: usize = 3;

/// Every layout key the menu reads, and that the movie's pieces fit together as the menu needs.
fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    MenuLayout::resolve(layout).map(drop)
}

pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[
        &LIST_PLACEMENT,
        &ROW_COUNT,
        &ROW_SPACING,
        &ENTRY_CLASS,
        &ENTRY_BORDER,
        &ENTRY_TEXT,
        &ENTRY_COLOURS,
        &ENTRY_AUTO_SIZE,
        &SELECTED_ALPHA,
        &PANEL_BACKGROUND,
        &SET_TEXT,
        &loading_icon::STAGE,
        &loading_icon::LOGO_ORIGIN,
        &loading_icon::LOGO_PLACEMENT,
    ],
    resolve: validate,
};

#[derive(Debug, Clone, PartialEq)]
pub struct EntryLayout {
    pub width: f32,
    pub height: f32,
    /// The entry's text field in the entry's own stage pixels.
    pub text: FieldGeometry,
    /// The font family the game maps the text field's font class to.
    pub font_family: String,
}

/// The colours of an enabled row. The movie also carries the disabled styles; no row of these
/// menus is ever disabled, so they are not read.
#[derive(Debug, Clone, PartialEq)]
pub struct ListColors {
    pub text: [u8; 3],
    pub text_selected: [u8; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectionStyle {
    pub fill: [u8; 3],
    pub alpha: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ListLayout {
    pub x: f32,
    pub y: f32,
    pub visible_rows: usize,
    pub vertical_spacing: f32,
    pub entry: EntryLayout,
    pub colors: ListColors,
    pub selection: SelectionStyle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelBackground {
    pub rect: StageRect,
    pub color: [u8; 3],
    pub alpha: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuLayout {
    pub stage: (f32, f32),
    pub list: ListLayout,
    pub panel_background: PanelBackground,
    pub set_text: SetTextRules,
    /// Where the loading screen's logo sits on the stage the menu shares with it.
    pub loading_icon: IconPlacement,
}

/// One visible list row: its index in the whole list, its rectangle in framebuffer pixels and
/// the map that puts the entry's own stage pixels on the screen.
#[derive(Debug, Clone, Copy)]
pub struct RowRect {
    pub index: usize,
    pub rect: PxRect,
    pub map: ScreenMap,
}

fn row_count(layout: &UiLayout) -> Result<usize, UiLayoutError> {
    match layout.lit_whole(&ROW_COUNT, 0)? {
        0 => Err(UiLayoutError::unresolved(
            ROW_COUNT.movie,
            ROW_COUNT.path,
            "the list shows no rows",
        )),
        rows => Ok(rows as usize),
    }
}

fn colour(key: &LiteralKey, value: f64) -> Result<[u8; 3], UiLayoutError> {
    if value >= 0.0 && value.fract() == 0.0 && value <= MAX_COLOUR {
        let rgb = value as u32;
        Ok([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8])
    } else {
        Err(UiLayoutError::unresolved(
            key.movie,
            key.path,
            format!("{value} is not a colour number"),
        ))
    }
}

fn bytes(rgba: [f32; 4]) -> ([u8; 3], f32) {
    let channel = |value: f32| (value * 255.0).round() as u8;
    (
        [channel(rgba[0]), channel(rgba[1]), channel(rgba[2])],
        rgba[3],
    )
}

impl MenuLayout {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let movie = layout.movie(MOVIE)?;
        let [xmin, ymin, xmax, ymax] = movie.stage().rect;
        if xmin != 0.0 || ymin != 0.0 {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                "stage",
                format!("the stage starts at ({xmin}, {ymin}), not at the origin"),
            ));
        }
        let entry_class = layout.lit_string(&ENTRY_CLASS, 0)?;
        if entry_class != LIST_ENTRY_CLASS {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                ENTRY_CLASS.path,
                format!("the list shows '{entry_class}' entries, not '{LIST_ENTRY_CLASS}'"),
            ));
        }
        let auto_size = layout.lit_string(&ENTRY_AUTO_SIZE, 0)?;
        if auto_size != SHRINK_MODE {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                ENTRY_AUTO_SIZE.path,
                format!("the entry text auto-sizes by '{auto_size}', not '{SHRINK_MODE}'"),
            ));
        }
        let border = layout.get(&ENTRY_BORDER)?;
        let border_rect = stage_rect(MOVIE, ENTRY_BORDER.path, &border)?;
        if border_rect.x.abs() > ORIGIN_TOLERANCE_PX || border_rect.y.abs() > ORIGIN_TOLERANCE_PX {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                ENTRY_BORDER.path,
                "the entry's border does not start at the entry's origin",
            ));
        }
        let text = layout.get(&ENTRY_TEXT)?;
        let list_origin = layout.get(&LIST_PLACEMENT)?.world.matrix;
        let panel = layout.get(&PANEL_BACKGROUND)?;
        let (panel_rgb, panel_alpha) = bytes(shown_fill(MOVIE, PANEL_BACKGROUND.path, &panel)?);
        let (fill_rgb, _) = bytes(shown_fill(MOVIE, ENTRY_BORDER.path, &border)?);
        let selected_alpha = layout.get(&SELECTED_ALPHA)?;
        if !(0.0..=1.0).contains(&selected_alpha) {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                SELECTED_ALPHA.path,
                format!("{selected_alpha} is not between 0 and 1"),
            ));
        }
        Ok(Self {
            stage: (xmax, ymax),
            list: ListLayout {
                x: list_origin.tx,
                y: list_origin.ty,
                visible_rows: row_count(layout)?,
                vertical_spacing: layout.lit_number(&ROW_SPACING, 0)? as f32,
                entry: EntryLayout {
                    width: border_rect.w,
                    height: border_rect.h,
                    text: FieldGeometry::of(&text, layout.font_metrics(&text.font)?),
                    font_family: layout.font_family(&text.font)?,
                },
                colors: ListColors {
                    text: colour(
                        &ENTRY_COLOURS,
                        layout.lit_number(&ENTRY_COLOURS, UNSELECTED_COLOUR)?,
                    )?,
                    text_selected: colour(
                        &ENTRY_COLOURS,
                        layout.lit_number(&ENTRY_COLOURS, SELECTED_COLOUR)?,
                    )?,
                },
                selection: SelectionStyle {
                    fill: fill_rgb,
                    alpha: selected_alpha as f32,
                },
            },
            panel_background: PanelBackground {
                rect: stage_rect(MOVIE, PANEL_BACKGROUND.path, &panel)?,
                color: panel_rgb,
                alpha: panel_alpha,
            },
            set_text: SET_TEXT.resolve(layout)?,
            loading_icon: IconPlacement::resolve(layout, (xmax, ymax))?,
        })
    }

    /// The stage scaled uniformly to fit a framebuffer of `fb_w` x `fb_h` pixels and centred on
    /// it; the rest is letterbox.
    pub fn fit(&self, fb_w: u32, fb_h: u32) -> Layout {
        Layout::for_stage(self.stage.0, self.stage.1, fb_w as f32, fb_h as f32)
    }

    /// The rectangles of the visible rows, top to bottom, for a window starting at list index
    /// `first_visible`. Drawing and hit-testing both use these.
    pub fn row_rects(
        &self,
        fb_w: u32,
        fb_h: u32,
        first_visible: usize,
    ) -> impl Iterator<Item = RowRect> + '_ {
        let fit = self.fit(fb_w, fb_h);
        let list = &self.list;
        let pitch = list.entry.height + list.vertical_spacing;
        (0..list.visible_rows).map(move |slot| {
            let row_y = list.y + slot as f32 * pitch;
            let (x, y) = fit.stage_point(list.x, row_y);
            RowRect {
                index: first_visible + slot,
                rect: PxRect {
                    x,
                    y,
                    w: fit.s(list.entry.width),
                    h: fit.s(list.entry.height),
                },
                map: ScreenMap {
                    origin: (x, y),
                    scale: fit.scale(),
                },
            }
        })
    }

    /// The panel behind the list, in framebuffer pixels.
    pub fn panel_rect(&self, fb_w: u32, fb_h: u32) -> PxRect {
        let fit = self.fit(fb_w, fb_h);
        let panel = &self.panel_background.rect;
        let (x, y) = fit.stage_point(panel.x, panel.y);
        PxRect {
            x,
            y,
            w: fit.s(panel.w),
            h: fit.s(panel.h),
        }
    }

    /// The index of the list row under the pixel `(x, y)`; the gaps between rows hit nothing.
    pub fn row_at(
        &self,
        fb_w: u32,
        fb_h: u32,
        first_visible: usize,
        x: f32,
        y: f32,
    ) -> Option<usize> {
        self.row_rects(fb_w, fb_h, first_visible)
            .find(|row| contains(row.rect, x, y))
            .map(|row| row.index)
    }
}

fn contains(rect: PxRect, x: f32, y: f32) -> bool {
    x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h
}

/// A synthetic menu movie in the layout format, for the tests of this and sibling modules.
#[cfg(test)]
pub(super) mod test_support {
    use std::sync::Arc;

    use serde_json::{Value, json};

    use super::{MOVIE, MenuLayout};
    use crate::rendering::menu::loading_icon;
    use crate::ui_layout::UiLayout;
    use crate::ui_layout::test_support::{fixture_face, instance, movie_json, set_text_parts};

    pub const FIXTURE_CLASS: &str = "$TEST_Font";
    pub const ROWS: usize = 4;

    fn translate() -> String {
        json!({
            "format": 1, "extractor": 1, "kind": "translate", "language": "en",
            "entries": {"$TEST_LABEL": "Translated label", "$PLAY": "PLAY", "$QUIT": "QUIT", "$BACK": "BACK"}, "unresolved": {},
        })
        .to_string()
    }

    fn font_config(family: &str) -> String {
        json!({
            "format": 1, "extractor": 1, "kind": "fontconfig", "language": "en",
            "fontLibraries": [],
            "maps": [{"class": FIXTURE_CLASS, "family": family, "style": "Normal"}],
            "fonts": [fixture_face(family, "Normal")],
            "validNameChars": null, "validBookChars": null,
        })
        .to_string()
    }

    fn placed(char: u32, kind: &str, matrix: [f64; 6]) -> Value {
        let mut value = instance(char, kind, 0.0, 0.0);
        value["world"]["matrix"] = json!(matrix);
        value["local"]["matrix"] = json!(matrix);
        value
    }

    fn rule_literals() -> Value {
        set_text_parts()["literals"].clone()
    }

    fn list_literal(property: &str, value: &Value) -> (String, Value) {
        (
            format!(
                "MainMenu_fla.MainListPanel_49.__setProp_List_mc_MainListPanel_List_0#List_mc.{property}="
            ),
            json!({"values": [value]}),
        )
    }

    /// The movie parts of a menu whose rows are 120 x 30 at (100, 200).
    pub fn parts() -> Value {
        let mut literals = rule_literals();
        for (key, value) in [
            list_literal("numListItems", &json!(ROWS)),
            list_literal("verticalSpacing", &json!(2)),
            list_literal("listEntryClass", &json!("MainMenuListEntry")),
        ] {
            literals[key] = value;
        }
        literals["MainMenuListEntry.SetEntryText#textField.textColor="] =
            json!({"values": [2_236_962, 4_473_924, 0, 16_777_215]});
        literals["MainMenuListEntry.<init>#TextFieldEx.setTextAutoSize(1)"] =
            json!({"values": ["shrink"]});
        let text = json!({
            "kind": "text", "bounds": [-2, -2, 102, 26], "font": FIXTURE_CLASS, "size": 20,
            "color": [255, 255, 255, 255], "align": "left", "leftMargin": 0, "rightMargin": 0,
            "indent": 0, "leading": 0,
            "flags": {"wordWrap": false, "multiline": false, "autoSize": false, "html": false},
        });
        let square = |colour: [u8; 4]| {
            json!({"kind": "shape", "bounds": [-5.5, -5.5, 5.5, 5.5],
                   "fills": [{"type": "solid", "color": colour}], "lines": [], "paths": []})
        };
        let border = placed(
            2,
            "shape",
            [120.0 / 11.0, 0.0, 0.0, 30.0 / 11.0, 60.0, 15.0],
        );
        json!({
            "fps": 30, "stage": [0, 0, 1280, 720],
            "constants": {
                "Shared.GlobalFunc.SELECTED_RECT_ALPHA": 1,
                "Shared.GlobalFunc.MAX_TRUNCATED_TEXT_LENGTH": crate::ui_layout::test_support::FIXTURE_TEXT_LIMIT,
            },
            "literals": literals,
            "characters": {
                "1": text,
                "2": square([255, 255, 255, 255]),
                "3": {"kind": "shape", "bounds": [0, 0, 140, 150],
                      "fills": [{"type": "solid", "color": [165, 165, 165, 2]}], "lines": [], "paths": []},
            },
            "instances": {
                "Menu_mc/MainPanel_mc/List_mc": placed(9, "sprite", [1.0, 0.0, 0.0, 1.0, 100.0, 200.0]),
                "[MainMenuListEntry]/border/@1": border,
                "[MainMenuListEntry]/textField": placed(1, "text", [1.0, 0.0, 0.0, 1.0, 10.0, 4.0]),
                "Menu_mc/MainPanel_mc/MainPanelBackground_mc/@1":
                    placed(3, "shape", [1.0, 0.0, 0.0, 1.0, 90.0, 190.0]),
            },
        })
    }

    pub fn ui_layout_with(parts: &Value, family: &str) -> Arc<UiLayout> {
        let loading = movie_json(loading_icon::MOVIE, &loading_icon::test_support::parts());
        build(parts, family, &[(loading_icon::MOVIE, &loading)])
    }

    /// The fixture menu with no loading screen movie installed.
    pub fn ui_layout_without_loading_screen(family: &str) -> Arc<UiLayout> {
        build(&parts(), family, &[])
    }

    fn build(parts: &Value, family: &str, others: &[(&str, &str)]) -> Arc<UiLayout> {
        let menu = movie_json(MOVIE, parts);
        let movies: Vec<(&str, &str)> = std::iter::once((MOVIE, menu.as_str()))
            .chain(others.iter().copied())
            .collect();
        Arc::new(
            UiLayout::from_documents(&translate(), &font_config(family), &movies)
                .expect("the fixture documents are valid"),
        )
    }

    pub fn ui_layout(family: &str) -> Arc<UiLayout> {
        ui_layout_with(&parts(), family)
    }

    pub fn standard() -> MenuLayout {
        MenuLayout::resolve(&ui_layout("Test Family")).expect("the fixture resolves")
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the layout arithmetic below stays on exactly representable values"
)]
mod tests {
    use super::test_support::{
        ROWS, parts, standard, ui_layout, ui_layout_with, ui_layout_without_loading_screen,
    };
    use super::*;
    use crate::rendering::game_settings::SafeZone;

    #[test]
    fn the_fixture_menu_resolves_into_typed_values() {
        let layout = standard();
        assert_eq!(layout.list.visible_rows, ROWS);
        assert_eq!((layout.list.x, layout.list.y), (100.0, 200.0));
        assert_eq!(layout.list.vertical_spacing, 2.0);
        assert_eq!(layout.list.entry.font_family, "Test Family");
        assert!((layout.list.entry.width - 120.0).abs() < 1e-3);
        assert!((layout.list.entry.height - 30.0).abs() < 1e-3);
        assert_eq!(layout.list.colors.text, [255, 255, 255]);
        assert_eq!(layout.list.colors.text_selected, [0, 0, 0]);
        assert_eq!(layout.list.selection.alpha, 1.0);
        assert_eq!(layout.panel_background.rect.x, 90.0);
        assert_eq!(layout.panel_background.color, [165, 165, 165]);
        assert!((layout.panel_background.alpha - 2.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn the_entry_text_field_keeps_its_gutter_and_size() {
        let text = standard().list.entry.text;
        assert_eq!(text.size, 20.0);
        assert_eq!(text.content_left(), 10.0);
    }

    #[test]
    fn rows_follow_the_entry_height_plus_the_spacing() {
        let layout = standard();
        let rows: Vec<_> = layout.row_rects(1280, 720, 3).collect();
        assert_eq!(rows.len(), ROWS);
        assert_eq!(rows[0].index, 3);
        assert!((rows[1].rect.y - rows[0].rect.y - 32.0).abs() < 1e-3);
        assert_eq!(rows[0].rect.x, 100.0);
        assert_eq!(rows[0].map.origin, (rows[0].rect.x, rows[0].rect.y));
    }

    #[test]
    fn a_stage_smaller_than_the_screen_is_centred_and_the_rows_scale_with_it() {
        let layout = standard();
        let first = layout.row_rects(2560, 1440, 0).next().unwrap();
        assert_eq!(first.rect.x, 200.0);
        assert_eq!(first.map.scale, 2.0);
    }

    #[test]
    fn the_gaps_between_rows_hit_nothing() {
        let layout = standard();
        let row = layout.row_rects(1280, 720, 0).next().unwrap().rect;
        assert_eq!(
            layout.row_at(1280, 720, 5, row.x + 1.0, row.y + 1.0),
            Some(5)
        );
        let gap = row.y + row.h + 1.0;
        assert_eq!(layout.row_at(1280, 720, 0, row.x + 1.0, gap), None);
        assert_eq!(layout.row_at(1280, 720, 0, row.x - 1.0, row.y + 1.0), None);
    }

    #[test]
    fn the_panel_rect_is_scaled_and_centred_like_the_rows() {
        let panel = standard().panel_rect(2560, 1440);
        assert_eq!(
            (panel.x, panel.y, panel.w, panel.h),
            (180.0, 380.0, 280.0, 300.0)
        );
    }

    #[test]
    fn every_failure_of_a_movie_without_the_menu_is_listed() {
        let mut empty = parts();
        empty["instances"] = serde_json::json!({});
        let layout = ui_layout_with(&empty, "Test Family");
        let error = layout.validate(&[&LAYOUT_KEYS]).unwrap_err().to_string();
        assert!(error.contains("[MainMenuListEntry]/textField"), "{error}");
    }

    #[test]
    fn the_loading_logo_is_resolved_with_the_menu() {
        let layout = standard();
        let fit = layout.fit(640, 360);
        let rect = layout.loading_icon.screen_rect(
            loading_icon::test_support::BOUNDS,
            &fit,
            SafeZone { x: 64.0, y: 36.0 },
        );
        assert_eq!((rect.x, rect.y), (598.0, 332.0));
    }

    #[test]
    fn a_menu_without_the_loading_screen_names_the_missing_movie() {
        let layout = ui_layout_without_loading_screen("Test Family");
        let error = MenuLayout::resolve(&layout).unwrap_err();
        assert!(error.to_string().contains("LoadingMenu"), "{error}");
    }

    #[test]
    fn a_list_of_another_entry_class_is_rejected() {
        let mut other = parts();
        other["literals"]["MainMenu_fla.MainListPanel_49.__setProp_List_mc_MainListPanel_List_0#List_mc.listEntryClass="] =
            serde_json::json!({"values": ["SomethingElse"]});
        let error = MenuLayout::resolve(&ui_layout_with(&other, "Test Family")).unwrap_err();
        assert!(error.to_string().contains("SomethingElse"), "{error}");
    }

    #[test]
    fn a_selection_alpha_outside_the_unit_range_is_rejected() {
        let mut other = parts();
        other["constants"]["Shared.GlobalFunc.SELECTED_RECT_ALPHA"] = serde_json::json!(2);
        let error = MenuLayout::resolve(&ui_layout_with(&other, "Test Family")).unwrap_err();
        assert!(error.to_string().contains("SELECTED_RECT_ALPHA"), "{error}");
    }

    #[test]
    fn the_fixture_validates_as_a_whole() {
        ui_layout("Test Family").validate(&[&LAYOUT_KEYS]).unwrap();
    }
}

#[cfg(test)]
mod real_install {
    use super::*;
    use crate::rendering::hud::real_install::installed_layout;

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the installer's layout documents"]
    fn the_installed_movie_satisfies_the_menu_keys() {
        let layout = installed_layout();
        layout.validate(&[&LAYOUT_KEYS]).unwrap();
        println!("{:#?}", MenuLayout::resolve(&layout).unwrap());
    }
}
