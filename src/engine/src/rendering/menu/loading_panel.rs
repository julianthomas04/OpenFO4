//! The text panel of a large loading screen: the tip with the box drawn around it, the level, the
//! experience meter and the catalogue number. The loading screen's layout document places every
//! element on the stage; like the rest of the menu, white art and text are multiplied by the HUD
//! colour.
//!
//! The document does not store the box: the screen's script draws it at run time as a 2 pixel
//! white outline without fill around the tip. Its line style and the four numbers of the
//! rectangle the script creates when it starts are in the document's literals. What the
//! rectangle is used for is inferred here: padding around the tip's lines, left, top, right and
//! bottom.

use super::loading_icon::MOVIE;
use super::safe_rect::{Anchor, plain_origin};
use crate::rendering::game_settings::SafeZone;
use crate::rendering::hud::field_text::{ScreenMap, TextExtent, TextItem};
use crate::rendering::hud::layout::Layout;
use crate::rendering::hud::quad::PxRect;
use crate::rendering::hud::shape::{StageRect, shown_fill, stage_rect};
use crate::rendering::ui_colour::UiTint;
use crate::ui_layout::scaleform_text::FieldGeometry;
use crate::ui_layout::{
    KeyGroup, LiteralKey, NumberKey, PlacementKey, ShapeKey, TextKey, TranslateKey, UiLayout,
    UiLayoutError,
};

const MAX_COLOUR: f64 = 16_777_215.0;
const OPAQUE: u8 = 255;

/// Until the engine has a player to show: the level and catalogue number are these and the
/// experience meter is this full.
pub const PLACEHOLDER_LEVEL: u32 = 1;
pub const PLACEHOLDER_CATALOGUE_NUMBER: u32 = 1;
pub const PLACEHOLDER_EXPERIENCE: f32 = 0.0;

pub static LEFT_TEXT: PlacementKey =
    PlacementKey::new(MOVIE, "FilterHolder_mc/Menu_mc/LeftText_mc");
pub static TIP: TextKey = TextKey::new(
    MOVIE,
    "FilterHolder_mc/Menu_mc/LeftText_mc/LoadScreenText_tf",
);
pub static LEVEL: TextKey = TextKey::new(MOVIE, "FilterHolder_mc/Menu_mc/LeftText_mc/LevelText_tf");
pub static CATALOGUE: TextKey =
    TextKey::new(MOVIE, "FilterHolder_mc/Menu_mc/LeftText_mc/VDSGText_tf");
pub static METER_OUTLINE: ShapeKey = ShapeKey::new(
    MOVIE,
    "FilterHolder_mc/Menu_mc/LeftText_mc/LevelMeter_mc/@1",
);
pub static METER_FILL: ShapeKey = ShapeKey::new(
    MOVIE,
    "FilterHolder_mc/Menu_mc/LeftText_mc/LevelMeter_mc/Fill_mc/@1",
);
/// The stage y the script's layout constants name for the tip's bottom. Inferred from a vanilla
/// screenshot, not read from the script: the bottom of the box around the tip's field, outline
/// included, sits on it.
static SLIDE_TEXT_Y: NumberKey = NumberKey::new(MOVIE, "LoadingMenu.SLIDE_TEXT_Y_POSITION");
static LEVEL_PREFIX: TranslateKey = TranslateKey::text("$LEVEL");
static CATALOGUE_PREFIX: TranslateKey = TranslateKey::text("$VDSGPrepend");
static BORDER_WIDTH: LiteralKey = LiteralKey::new(
    MOVIE,
    "LoadingMenu.SetLoadingText#BorderRectangle.graphics.lineStyle(0)",
    1,
);
static BORDER_COLOUR: LiteralKey = LiteralKey::new(
    MOVIE,
    "LoadingMenu.SetLoadingText#BorderRectangle.graphics.lineStyle(1)",
    1,
);
static BORDER_INSET: [LiteralKey; 4] = [
    LiteralKey::new(MOVIE, "LoadingMenu.<init>#new Rectangle(0)", 1),
    LiteralKey::new(MOVIE, "LoadingMenu.<init>#new Rectangle(1)", 1),
    LiteralKey::new(MOVIE, "LoadingMenu.<init>#new Rectangle(2)", 1),
    LiteralKey::new(MOVIE, "LoadingMenu.<init>#new Rectangle(3)", 1),
];

/// Every layout key the panel reads.
pub static KEYS: KeyGroup = KeyGroup(&[
    &LEFT_TEXT,
    &TIP,
    &LEVEL,
    &CATALOGUE,
    &METER_OUTLINE,
    &METER_FILL,
    &BORDER_WIDTH,
    &BORDER_COLOUR,
    &BORDER_INSET,
    &SLIDE_TEXT_Y,
]);

/// Distances in stage pixels from the sides of the tip's lines out to the box's outline.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Padding {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

#[derive(Debug, Clone, PartialEq)]
struct TextStyle {
    geometry: FieldGeometry,
    family: String,
    colour: [u8; 3],
}

#[derive(Debug, Clone, PartialEq)]
struct Outline {
    /// The path the line follows, in stage pixels.
    rect: StageRect,
    /// The line's thickness across and down, in stage pixels.
    thickness: (f32, f32),
    colour: [u8; 3],
}

#[derive(Debug, Clone, PartialEq)]
struct Meter {
    frame: Outline,
    fill: StageRect,
    fill_colour: [u8; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelLayout {
    tip: TextStyle,
    level: TextStyle,
    catalogue: TextStyle,
    /// The translated labels in front of the level and the catalogue number. A line whose label
    /// the translations lack is left out.
    level_prefix: Option<String>,
    catalogue_prefix: Option<String>,
    meter: Meter,
    /// The registration point of the clip holding the panel, on the stage as authored.
    origin: (f32, f32),
    border_width: f32,
    border_colour: [u8; 3],
    pub border_padding: Padding,
    /// The stage y the bottom of the tip's box is laid out at, see [`SLIDE_TEXT_Y`].
    tip_bottom_anchor: f32,
}

/// What the panel shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PanelContent<'a> {
    pub tip: Option<&'a str>,
    pub level: u32,
    /// How full the experience meter is, 0 to 1.
    pub experience: f32,
    pub catalogue_number: u32,
}

impl<'a> PanelContent<'a> {
    /// The panel of a screen with `tip`, showing the placeholders for the player.
    pub fn placeholder(tip: Option<&'a str>) -> Self {
        Self {
            tip,
            level: PLACEHOLDER_LEVEL,
            experience: PLACEHOLDER_EXPERIENCE,
            catalogue_number: PLACEHOLDER_CATALOGUE_NUMBER,
        }
    }
}

/// The text fields of the panel for one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelText {
    pub tip: Option<TextItem>,
    pub level: Option<TextItem>,
    pub catalogue: Option<TextItem>,
}

impl PanelText {
    pub fn into_items(self) -> Vec<TextItem> {
        [self.tip, self.level, self.catalogue]
            .into_iter()
            .flatten()
            .collect()
    }
}

/// A rectangle with its colour, for the quad pass.
pub type ColouredRect = (PxRect, [f32; 4]);

/// The translation of `key`; none, after an error logged once, when the translations lack it.
fn label(layout: &UiLayout, key: &TranslateKey) -> Option<String> {
    layout
        .get(key)
        .map_err(|error| {
            crate::utils::error_once(&format!("loading-panel-label-{}", key.path), || {
                format!("{error}; the loading screen leaves that line out")
            });
        })
        .ok()
}

fn text_style(layout: &UiLayout, key: &TextKey) -> Result<TextStyle, UiLayoutError> {
    let field = layout.get(key)?;
    let [red, green, blue, _] = field
        .color
        .ok_or_else(|| UiLayoutError::unresolved(key.movie, key.path, "the field has no colour"))?;
    Ok(TextStyle {
        geometry: FieldGeometry::of(&field, layout.font_metrics(&field.font)?),
        family: layout.font_family(&field.font)?,
        colour: [red, green, blue],
    })
}

fn rgb_of(key: &LiteralKey, value: f64) -> Result<[u8; 3], UiLayoutError> {
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

fn shown_rgb(
    movie: &str,
    key: &str,
    shape: &crate::ui_layout::values::Shape,
) -> Result<[u8; 3], UiLayoutError> {
    let [red, green, blue, _] = shown_fill(movie, key, shape)?;
    let byte = |channel: f32| (channel * 255.0).round() as u8;
    Ok([byte(red), byte(green), byte(blue)])
}

impl Meter {
    fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let frame = layout.get(&METER_OUTLINE)?;
        let line = frame.lines.first().ok_or_else(|| {
            UiLayoutError::unresolved(
                METER_OUTLINE.movie,
                METER_OUTLINE.path,
                "the meter's frame has no line",
            )
        })?;
        let [red, green, blue, _] = line.color.ok_or_else(|| {
            UiLayoutError::unresolved(
                METER_OUTLINE.movie,
                METER_OUTLINE.path,
                "the meter's line has no colour",
            )
        })?;
        let matrix = frame.placement.world.matrix;
        let fill = layout.get(&METER_FILL)?;
        Ok(Self {
            frame: Outline {
                rect: stage_rect(METER_OUTLINE.movie, METER_OUTLINE.path, &frame)?,
                thickness: (line.width * matrix.a.abs(), line.width * matrix.d.abs()),
                colour: [red, green, blue],
            },
            fill: stage_rect(METER_FILL.movie, METER_FILL.path, &fill)?,
            fill_colour: shown_rgb(METER_FILL.movie, METER_FILL.path, &fill)?,
        })
    }
}

impl PanelLayout {
    /// Reads the panel from the loading screen's layout and the translation of its labels.
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let inset = |index: usize| layout.lit_number(&BORDER_INSET[index], 0).map(|n| n as f32);
        Ok(Self {
            tip: text_style(layout, &TIP)?,
            level: text_style(layout, &LEVEL)?,
            catalogue: text_style(layout, &CATALOGUE)?,
            level_prefix: label(layout, &LEVEL_PREFIX),
            catalogue_prefix: label(layout, &CATALOGUE_PREFIX),
            meter: Meter::resolve(layout)?,
            origin: plain_origin(layout, &LEFT_TEXT)?,
            border_width: layout.lit_number(&BORDER_WIDTH, 0)? as f32,
            border_colour: rgb_of(&BORDER_COLOUR, layout.lit_number(&BORDER_COLOUR, 0)?)?,
            border_padding: Padding {
                left: inset(0)?,
                top: inset(1)?,
                right: inset(2)?,
                bottom: inset(3)?,
            },
            tip_bottom_anchor: layout.get(&SLIDE_TEXT_Y)? as f32,
        })
    }

    /// The stage offset the loading screen's script moves the panel by: sideways, it puts the
    /// clip's registration point on the left edge of the safe zone; down, the bottom of the tip's
    /// box goes to the script's anchor.
    fn safe_rect_shift(&self, fit: &Layout, safe_zone: SafeZone) -> (f32, f32) {
        let (sideways, _) = Anchor::Left.shift(fit, safe_zone, self.origin);
        let lower_bound = self.tip.geometry.bottom + self.border_padding.bottom;
        (
            sideways,
            tip_box_shift(self.tip_bottom_anchor, lower_bound, self.border_width),
        )
    }

    /// The text of `content` on a framebuffer fitted by `fit`, in the colour of `tint`, with the
    /// panel locked to the left of `safe_zone`.
    pub fn text(
        &self,
        fit: &Layout,
        safe_zone: SafeZone,
        tint: &UiTint,
        content: &PanelContent<'_>,
    ) -> PanelText {
        let shift = self.safe_rect_shift(fit, safe_zone);
        PanelText {
            tip: content
                .tip
                .filter(|tip| !tip.trim().is_empty())
                .map(|tip| wrapped_item(&self.tip, fit, shift, tint, tip)),
            level: self.level_prefix.as_ref().map(|prefix| {
                let level = format!("{prefix} {}", content.level);
                item(&self.level, fit, shift, tint, level)
            }),
            catalogue: self.catalogue_prefix.as_ref().map(|prefix| {
                let number = format!("{prefix} {}", content.catalogue_number);
                item(&self.catalogue, fit, shift, tint, number)
            }),
        }
    }

    /// The rectangles of the panel: the box around the tip's lines when there are some, and the
    /// experience meter filled to `experience`.
    pub fn rects(
        &self,
        fit: &Layout,
        safe_zone: SafeZone,
        tint: &UiTint,
        tip: Option<TextExtent>,
        experience: f32,
    ) -> Vec<ColouredRect> {
        let shift = self.safe_rect_shift(fit, safe_zone);
        let colour = |rgb: [u8; 3]| tinted(tint, rgb);
        let mut rects = Vec::new();
        if let Some(extent) = tip {
            let field = self.tip.geometry;
            let (field_left, _) = fit.stage_point(field.left + shift.0, 0.0);
            let extent = across_field(extent, field_left, fit.s(field.right - field.left));
            let path = padded(extent, self.border_padding, fit.scale());
            let width = fit.s(self.border_width);
            rects.extend(
                outline_rects(path, (width, width))
                    .into_iter()
                    .map(|rect| (rect, colour(self.border_colour))),
            );
        }
        let meter = &self.meter;
        let frame = screen_rect(fit, shifted(meter.frame.rect, shift));
        let thickness = (
            fit.s(meter.frame.thickness.0),
            fit.s(meter.frame.thickness.1),
        );
        rects.extend(
            outline_rects(frame, thickness)
                .into_iter()
                .map(|rect| (rect, colour(meter.frame.colour))),
        );
        let fill = filled(screen_rect(fit, shifted(meter.fill, shift)), experience);
        rects.push((fill, colour(meter.fill_colour)));
        rects
    }
}

fn tinted(tint: &UiTint, rgb: [u8; 3]) -> [f32; 4] {
    let [red, green, blue] = tint.apply_to(rgb).map(|channel| f32::from(channel) / 255.0);
    [red, green, blue, 1.0]
}

fn item(
    style: &TextStyle,
    fit: &Layout,
    shift: (f32, f32),
    tint: &UiTint,
    text: String,
) -> TextItem {
    let (origin_x, origin_y) = fit.stage_point(shift.0, shift.1);
    let [red, green, blue] = tint.apply_to(style.colour);
    TextItem {
        text,
        family: style.family.clone(),
        geometry: style.geometry,
        map: ScreenMap {
            origin: (origin_x, origin_y),
            scale: fit.scale(),
        },
        shrink: false,
        colour: [red, green, blue, OPAQUE],
        shadow: None,
        wrap_width: None,
        max_height: None,
    }
}

/// `text` wrapped to the field's width and cut off after the last whole line that fits its height.
fn wrapped_item(
    style: &TextStyle,
    fit: &Layout,
    shift: (f32, f32),
    tint: &UiTint,
    text: &str,
) -> TextItem {
    let geometry = style.geometry.wrapping();
    let pitch = geometry.line_pitch();
    let whole_lines = (geometry.content_height() / pitch).floor().max(1.0);
    TextItem {
        wrap_width: Some(fit.s(geometry.content_width())),
        // Half a line short of the last whole line, so no rounding shows a line past the field.
        max_height: Some(fit.s((whole_lines - 0.5) * pitch)),
        ..item(
            &TextStyle {
                geometry,
                ..style.clone()
            },
            fit,
            shift,
            tint,
            text.to_owned(),
        )
    }
}

fn shifted(rect: StageRect, shift: (f32, f32)) -> StageRect {
    StageRect {
        x: rect.x + shift.0,
        y: rect.y + shift.1,
        ..rect
    }
}

/// The vertical shift that puts the outer bottom of the tip's box on `anchor`, when the box's
/// path would end at `path_bottom` as authored and the outline is `line_width` thick.
fn tip_box_shift(anchor: f32, path_bottom: f32, line_width: f32) -> f32 {
    anchor - (path_bottom + line_width / 2.0)
}

/// `extent` with the horizontal span of the tip's field: the script draws the box around the
/// whole field, so its width does not depend on how far the lines reach.
fn across_field(extent: TextExtent, field_left: f32, field_width: f32) -> TextExtent {
    TextExtent {
        left: field_left,
        width: field_width,
        ..extent
    }
}

fn screen_rect(fit: &Layout, rect: StageRect) -> PxRect {
    let (x, y) = fit.stage_point(rect.x, rect.y);
    PxRect {
        x,
        y,
        w: fit.s(rect.w),
        h: fit.s(rect.h),
    }
}

/// The path of the tip's box: the lines in `extent` and `padding` stage pixels more on each
/// side, `scale` framebuffer pixels per stage pixel.
fn padded(extent: TextExtent, padding: Padding, scale: f32) -> PxRect {
    PxRect {
        x: extent.left - padding.left * scale,
        y: extent.top - padding.top * scale,
        w: extent.width + (padding.left + padding.right) * scale,
        h: extent.height + (padding.top + padding.bottom) * scale,
    }
}

/// The left `fraction` of `rect`; 0 for a fraction that is not a number.
fn filled(rect: PxRect, fraction: f32) -> PxRect {
    let fraction = if fraction.is_nan() {
        0.0
    } else {
        fraction.clamp(0.0, 1.0)
    };
    PxRect {
        w: rect.w * fraction,
        ..rect
    }
}

/// The four bars of a line of `thickness` (across, down) drawn along the edges of `path`, centred
/// on them like a stroke: top, bottom, left and right, the left and right ones between the
/// other two.
fn outline_rects(path: PxRect, thickness: (f32, f32)) -> [PxRect; 4] {
    let (across, down) = thickness;
    let (outer_x, outer_y) = (path.x - across / 2.0, path.y - down / 2.0);
    let (outer_w, outer_h) = (path.w + across, path.h + down);
    let inner_h = (outer_h - 2.0 * down).max(0.0);
    [
        PxRect {
            x: outer_x,
            y: outer_y,
            w: outer_w,
            h: down,
        },
        PxRect {
            x: outer_x,
            y: outer_y + outer_h - down,
            w: outer_w,
            h: down,
        },
        PxRect {
            x: outer_x,
            y: outer_y + down,
            w: across,
            h: inner_h,
        },
        PxRect {
            x: outer_x + outer_w - across,
            y: outer_y + down,
            w: across,
            h: inner_h,
        },
    ]
}

/// A synthetic panel in the layout format, with the numbers of the real one, for the tests of
/// this and sibling modules.
#[cfg(test)]
pub(super) mod test_support {
    use serde_json::{Value, json};

    use crate::rendering::menu::layout::test_support::FIXTURE_CLASS;
    use crate::ui_layout::test_support::instance;

    const LEFT_TEXT: &str = "FilterHolder_mc/Menu_mc/LeftText_mc";

    fn text_field(bounds: [f64; 4], size: u32, colour: [u8; 4], wrap: bool) -> Value {
        json!({
            "kind": "text", "bounds": bounds, "font": FIXTURE_CLASS, "size": size,
            "color": colour, "align": "left", "leftMargin": 0, "rightMargin": 0,
            "indent": 0, "leading": 0,
            "flags": {"wordWrap": wrap, "multiline": wrap, "autoSize": false, "html": false},
        })
    }

    fn placed(char: u32, kind: &str, matrix: [f64; 6]) -> Value {
        let mut value = instance(char, kind, 0.0, 0.0);
        value["world"]["matrix"] = json!(matrix);
        value["local"]["matrix"] = json!(matrix);
        value
    }

    /// Where the fixture puts the tip's field: its origin, in stage pixels.
    pub const TIP_ORIGIN: (f64, f64) = (152.0, 588.0);
    /// The tip field's authored width, wider than the content by the gutters.
    pub const TIP_WIDTH: f64 = 504.95;

    /// The fixture's value of the script's constant for the bottom of the tip.
    pub const SLIDE_TEXT_Y_POSITION: f64 = 651.0;

    /// Where the fixture puts the panel's clip: on the left edge of the tests' safe zone.
    pub const LEFT_TEXT_ORIGIN: (f64, f64) = (64.0, 506.65);

    /// The characters, instances and literals of the panel, to merge into the loading screen's
    /// movie parts.
    pub fn parts() -> Value {
        let literal = |value: Value| json!({ "values": [value] });
        json!({
            "constants": {"LoadingMenu.SLIDE_TEXT_Y_POSITION": SLIDE_TEXT_Y_POSITION},
            "characters": {
                "16": text_field([-2.0, -2.0, 502.95, 115.6], 19, [221, 221, 221, 255], true),
                "17": text_field([-2.0, -2.0, 398.0, 24.8], 17, [255, 255, 255, 255], false),
                "18": text_field([-2.0, -2.0, 102.0, 26.0], 19, [255, 255, 255, 255], false),
                "1": {"kind": "shape", "bounds": [-1.3, -1.3, 201.3, 26.3],
                      "edgeBounds": [0, 0, 200, 25], "fills": [],
                      "lines": [{"width": 2, "color": [255, 255, 255, 255]}], "paths": []},
                "2": {"kind": "shape", "bounds": [0, 0, 30, 30],
                      "fills": [{"type": "solid", "color": [255, 255, 255, 255]}],
                      "lines": [], "paths": []},
            },
            "instances": {
                LEFT_TEXT.to_owned():
                    placed(3, "sprite", [1.0, 0.0, 0.0, 1.0, LEFT_TEXT_ORIGIN.0, LEFT_TEXT_ORIGIN.1]),
                format!("{LEFT_TEXT}/LoadScreenText_tf"):
                    placed(16, "text", [1.0, 0.0, 0.0, 1.0, TIP_ORIGIN.0, TIP_ORIGIN.1]),
                format!("{LEFT_TEXT}/VDSGText_tf"):
                    placed(17, "text", [1.0, 0.0, 0.0, 1.0, 141.0, 559.25]),
                format!("{LEFT_TEXT}/LevelText_tf"):
                    placed(18, "text", [1.0, 0.0, 0.0, 1.0, 141.0, 488.65]),
                format!("{LEFT_TEXT}/LevelMeter_mc/@1"):
                    placed(1, "shape", [1.09491, 0.0, 0.0, 0.440_002, 142.55, 513.05]),
                format!("{LEFT_TEXT}/LevelMeter_mc/Fill_mc/@1"):
                    placed(2, "shape", [7.299_387, 0.0, 0.0, 0.366_666, 142.55, 513.05]),
            },
            "literals": {
                "LoadingMenu.SetLoadingText#BorderRectangle.graphics.lineStyle(0)": literal(json!(2)),
                "LoadingMenu.SetLoadingText#BorderRectangle.graphics.lineStyle(1)":
                    literal(json!(16_777_215)),
                "LoadingMenu.<init>#new Rectangle(0)": literal(json!(8)),
                "LoadingMenu.<init>#new Rectangle(1)": literal(json!(3)),
                "LoadingMenu.<init>#new Rectangle(2)": literal(json!(5)),
                "LoadingMenu.<init>#new Rectangle(3)": literal(json!(10)),
            },
        })
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the layout arithmetic below stays on exactly representable values"
)]
mod tests {
    use super::*;
    use crate::rendering::menu::layout::test_support::standard;

    const GREEN: UiTint = UiTint {
        color: [0.0, 1.0, 0.0],
        opacity: 1.0,
    };

    /// A safe zone whose left edge is where the fixture places the panel's clip, so the lock moves
    /// nothing.
    const AUTHORED: SafeZone = SafeZone { x: 64.0, y: 36.0 };

    /// How far the fixture's panel moves down for its bottom anchor of 651: its tip field ends at
    /// 703.6, with 10 of padding and half of the 2 pixel outline below.
    const AUTHORED_LIFT: f32 = 651.0 - (703.6 + 10.0 + 1.0);

    fn panel() -> PanelLayout {
        standard().loading_panel
    }

    /// The fixture's stage on a 1280 x 720 target, one framebuffer pixel per stage pixel.
    fn fit() -> Layout {
        Layout::for_stage(1280.0, 720.0, 1280.0, 720.0)
    }

    fn close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-2,
            "expected about {expected}, got {actual}"
        );
    }

    #[test]
    fn the_fields_are_placed_by_the_document() {
        let panel = panel();
        close(panel.tip.geometry.content_left(), 152.0);
        close(panel.tip.geometry.line_top(), 588.0);
        close(panel.level.geometry.content_left(), 141.0);
        close(panel.level.geometry.line_top(), 488.65);
        close(panel.catalogue.geometry.line_top(), 559.25);
        assert_eq!(
            (panel.tip.geometry.size, panel.catalogue.geometry.size),
            (19.0, 17.0)
        );
        assert_eq!(panel.tip.colour, [221, 221, 221]);
        assert_eq!(panel.level.colour, [255, 255, 255]);
        assert_eq!(panel.tip.family, "Test Family");
    }

    #[test]
    fn the_whole_panel_is_locked_to_the_safe_zones_left_edge() {
        let (fit, safe) = (fit(), SafeZone { x: 40.0, y: 36.0 });
        let shift = 40.0 - 64.0;
        let content = PanelContent::placeholder(Some("tip"));
        let authored = panel().text(&fit, AUTHORED, &GREEN, &content);
        let locked = panel().text(&fit, safe, &GREEN, &content);
        for (before, after) in authored.into_items().iter().zip(locked.into_items().iter()) {
            close(after.map.origin.0 - before.map.origin.0, shift);
            assert_eq!(after.map.origin.1, before.map.origin.1);
        }
        let before = panel().rects(&fit, AUTHORED, &GREEN, None, 0.5);
        let after = panel().rects(&fit, safe, &GREEN, None, 0.5);
        assert_eq!(before.len(), after.len());
        for ((moved, _), (authored, _)) in after.iter().zip(&before) {
            close(moved.x - authored.x, shift);
            assert_eq!(
                (moved.y, moved.w, moved.h),
                (authored.y, authored.w, authored.h)
            );
        }
    }

    #[test]
    fn the_box_style_and_padding_come_from_the_document() {
        let panel = panel();
        assert_eq!(panel.border_width, 2.0);
        assert_eq!(panel.border_colour, [255, 255, 255]);
        assert_eq!(
            panel.border_padding,
            Padding {
                left: 8.0,
                top: 3.0,
                right: 5.0,
                bottom: 10.0
            }
        );
    }

    #[test]
    fn the_labels_are_translated_and_the_placeholders_follow_them() {
        let text = panel().text(
            &fit(),
            AUTHORED,
            &GREEN,
            &PanelContent::placeholder(Some("tip")),
        );
        assert_eq!(text.level.unwrap().text, "LEVEL 1");
        assert_eq!(text.catalogue.unwrap().text, "VDSG No. 1");
        assert_eq!(text.tip.expect("a tip").text, "tip");
    }

    #[test]
    fn a_blank_tip_is_no_tip() {
        let text = panel().text(
            &fit(),
            AUTHORED,
            &GREEN,
            &PanelContent::placeholder(Some(
                "  
 ",
            )),
        );
        assert!(text.tip.is_none());
    }

    #[test]
    fn a_screen_without_a_tip_still_has_its_level_and_catalogue_number() {
        let text = panel().text(&fit(), AUTHORED, &GREEN, &PanelContent::placeholder(None));
        assert!(text.tip.is_none());
        assert_eq!(text.into_items().len(), 2);
    }

    #[test]
    fn text_is_the_hud_colour_times_the_field_colour() {
        let text = panel().text(
            &fit(),
            AUTHORED,
            &GREEN,
            &PanelContent::placeholder(Some("tip")),
        );
        assert_eq!(text.level.unwrap().colour, [0, 255, 0, 255]);
        let tip = text.tip.expect("a tip").colour;
        assert_eq!((tip[0], tip[2], tip[3]), (0, 0, 255));
        assert!((218..=224).contains(&tip[1]), "{tip:?}");
    }

    #[test]
    fn the_tip_wraps_at_the_field_width_and_stops_after_its_last_whole_line() {
        let fit = Layout::for_stage(1280.0, 720.0, 640.0, 360.0);
        let text = panel().text(
            &fit,
            AUTHORED,
            &GREEN,
            &PanelContent::placeholder(Some("tip")),
        );
        let tip = text.tip.expect("a tip");
        close(
            tip.wrap_width.expect("a tip wraps"),
            (super::test_support::TIP_WIDTH as f32 - 4.0) * 0.5,
        );
        let height = tip.max_height.expect("a tip is bounded") / fit.scale();
        let pitch = tip.geometry.line_pitch();
        let lines = ((height + 0.5 * pitch) / pitch).round();
        close(height, (lines - 0.5) * pitch);
        assert!(
            lines * pitch <= tip.geometry.content_height() + 0.01,
            "{lines} lines in {}",
            tip.geometry.content_height()
        );
    }

    #[test]
    fn the_single_line_fields_neither_wrap_nor_stop() {
        let text = panel().text(&fit(), AUTHORED, &GREEN, &PanelContent::placeholder(None));
        let (level, catalogue) = (text.level.unwrap(), text.catalogue.unwrap());
        assert!(level.wrap_width.is_none() && level.max_height.is_none());
        assert!(catalogue.wrap_width.is_none());
    }

    #[test]
    fn the_box_path_surrounds_the_lines_by_the_padding_at_the_screen_scale() {
        let extent = TextExtent {
            left: 100.0,
            top: 200.0,
            width: 300.0,
            height: 60.0,
        };
        let path = padded(extent, panel().border_padding, 0.5);
        assert_eq!((path.x, path.y), (96.0, 198.5));
        assert_eq!((path.w, path.h), (306.5, 66.5));
    }

    #[test]
    fn an_outline_is_four_bars_centred_on_the_path_that_leave_the_inside_empty() {
        let path = PxRect {
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 50.0,
        };
        let [top, bottom, left, right] = outline_rects(path, (4.0, 2.0));
        assert_eq!((top.x, top.y, top.w, top.h), (8.0, 19.0, 104.0, 2.0));
        assert_eq!(
            (bottom.x, bottom.y, bottom.w, bottom.h),
            (8.0, 69.0, 104.0, 2.0)
        );
        assert_eq!((left.x, left.y, left.w, left.h), (8.0, 21.0, 4.0, 48.0));
        assert_eq!(
            (right.x, right.y, right.w, right.h),
            (108.0, 21.0, 4.0, 48.0)
        );
        let covered: f32 = [top, bottom, left, right].iter().map(|r| r.w * r.h).sum();
        assert_eq!(
            covered,
            104.0 * 52.0 - 96.0 * 48.0,
            "the bars tile the ring"
        );
    }

    #[test]
    fn a_line_thicker_than_the_path_is_tall_leaves_no_negative_bars() {
        let path = PxRect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 1.0,
        };
        let [_, _, left, right] = outline_rects(path, (2.0, 4.0));
        assert_eq!((left.h, right.h), (0.0, 0.0));
    }

    #[test]
    fn the_fill_is_the_given_fraction_of_the_meter_and_never_leaves_it() {
        let meter = PxRect {
            x: 5.0,
            y: 6.0,
            w: 200.0,
            h: 10.0,
        };
        assert_eq!(filled(meter, 0.25).w, 50.0);
        assert_eq!(filled(meter, 0.0).w, 0.0);
        assert_eq!(filled(meter, 3.0).w, 200.0);
        assert_eq!(filled(meter, -1.0).w, 0.0);
        assert_eq!(filled(meter, f32::NAN).w, 0.0);
        let half = filled(meter, 0.5);
        assert_eq!((half.x, half.y, half.h), (5.0, 6.0, 10.0));
    }

    #[test]
    fn the_meter_sits_where_the_document_puts_it_and_is_drawn_in_the_hud_colour() {
        let rects = panel().rects(&fit(), AUTHORED, &GREEN, None, 0.5);
        // Four frame bars and the fill, and no box without a tip.
        assert_eq!(rects.len(), 5);
        let (fill, colour) = rects[4];
        close(fill.x, 142.55);
        close(fill.y, 513.05 + AUTHORED_LIFT);
        close(fill.w, 219.0 / 2.0);
        close(fill.h, 11.0);
        assert_eq!(colour, [0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn the_frame_of_the_meter_has_the_stroke_scaled_by_the_documents_matrix() {
        let rects = panel().rects(&fit(), AUTHORED, &GREEN, None, 0.0);
        let (top, _) = rects[0];
        let (left, _) = rects[2];
        close(top.h, 2.0 * 0.440_002);
        close(left.w, 2.0 * 1.09491);
    }

    #[test]
    fn the_tip_box_is_four_more_bars_around_the_extent() {
        let extent = TextExtent {
            left: 152.0,
            top: 588.0,
            width: 400.0,
            height: 80.0,
        };
        let with_box = panel().rects(&fit(), AUTHORED, &GREEN, Some(extent), 0.0);
        let without = panel().rects(&fit(), AUTHORED, &GREEN, None, 0.0);
        assert_eq!(with_box.len(), without.len() + 4);
        let (top, _) = with_box[0];
        close(top.y, 588.0 - 3.0 - 1.0);
        close(top.x, 150.0 - 8.0 - 1.0);
        close(top.w, 504.95 + 13.0 + 2.0);
    }

    #[test]
    fn the_box_spans_the_whole_field_however_short_the_lines_are() {
        let rects_for = |width: f32| {
            let extent = TextExtent {
                left: 152.0,
                top: 588.0,
                width,
                height: 80.0,
            };
            panel().rects(&fit(), AUTHORED, &GREEN, Some(extent), 0.0)[0].0
        };
        assert_eq!(rects_for(40.0), rects_for(480.0));
    }

    #[test]
    fn the_whole_panel_is_lowered_so_the_boxs_bottom_meets_the_anchor() {
        let text = panel().text(
            &fit(),
            AUTHORED,
            &GREEN,
            &PanelContent::placeholder(Some("tip")),
        );
        close(text.tip.expect("a tip").map.origin.1, AUTHORED_LIFT);
        close(text.level.expect("a level").map.origin.1, AUTHORED_LIFT);
    }

    #[test]
    fn the_shift_puts_the_outer_bottom_of_the_outline_on_the_anchor() {
        assert_eq!(tip_box_shift(651.0, 700.0, 2.0), -50.0);
        assert_eq!(tip_box_shift(651.0, 649.0, 4.0), 0.0);
    }

    #[test]
    fn across_field_replaces_only_the_horizontal_span() {
        let extent = TextExtent {
            left: 10.0,
            top: 20.0,
            width: 30.0,
            height: 40.0,
        };
        let spanned = across_field(extent, 5.0, 100.0);
        assert_eq!(
            (spanned.left, spanned.width, spanned.top, spanned.height),
            (5.0, 100.0, 20.0, 40.0)
        );
    }
}
