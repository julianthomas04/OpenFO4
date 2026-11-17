//! The message list of the HUD movie (`HUDNotificationsGroup_mc/Messages_mc`): the lines the game
//! stacks under the top-left of the screen, each fading in and out on the timeline of its item
//! clip, inside one frame (bracket and shaded background) around the whole stack.
//!
//! From the movie: the list's registration point, the stack limit, the spacing, the clip height
//! limit, the item clip's frame labels and alpha track, the text field, and the bracket and
//! background settings. From measurement in the original: [`OBSERVED`]. The only line the original
//! was seen to show is the "too heavy" setting text; whoever pushes a line decides what it says.

use std::sync::Arc;

use super::backdrop::{BackdropKeys, BackdropLook, BackdropSpec, backdrop_keys};
use super::bracket::{BracketKeys, BracketSpec, bracket_keys};
use super::field_text::{ScreenMap, TextExtent, TextItem};
use super::layout::Layout;
use super::message_list::{ListSpec, MessageList};
use super::meters;
use super::quad::{PxRect, QuadBuilder};
use super::text_face::TextFace;
use crate::rendering::pipboy::shapes::alpha_at_frame;
use crate::rendering::ui_colour::UiTint;
use crate::ui_layout::values::{ColorTransform, TrackKey};
use crate::ui_layout::{
    LabelKey, LiteralKey, NumberKey, PlacementKey, SymbolTimelineKey, TextKey, UiLayout,
    UiLayoutError, key_check_fields,
};

const MOVIE: &str = meters::MOVIE;
const ITEM_SYMBOL: &str = "HUDMessageItem";

/// The longest step the list plays on in one update, like the crosshair's: a hitch does not retire
/// a line unseen.
const MAX_STEP_SECONDS: f32 = 0.25;

/// What the original shows that the movie does not say. Everything measured is here.
///
/// Measured in the original, 2026-11-12 and 2026-11-13, on one line (game setting
/// `sItemTooHeavy`; also `sNoKeyDropWarning`),
/// 1600x900 captures read in stage pixels of the 1280x720 stage (1 capture pixel = 0.8 stage
/// pixel), at the capture's HUD opacity (1: the brackets read opaque); n counts independent events
/// (frames in brackets). Times are from the start of the line (its first ink), 50 ms sampling.
/// - Backdrop: over the mid-grey floor the interior read (43-45, 72-73, 38-39) where the scene
///   read (101-107, 96-101, 86-91); n = 2 scenes, spread +-1.5 levels. The fit used is a blend in
///   linear light, which is what the engine's sRGB surface does: alpha 0.815, colour (0, 69, 0),
///   mean error 2.3-4.0 levels. Alternatives that fit the same pairs: a blend in sRGB space
///   (alpha 0.68, colour (12, 56, 11); alpha 0.535, colour (0, 53, 0)), which the engine cannot
///   draw. The data is underdetermined; not measured: the backdrop over a dark scene.
/// - Box: x 64.0-407.2, y 77.6-109.6 for every text (n = 5 events, about 200 frames, spread
///   <= 1 px): fixed, not fitted to the text.
/// - Fade-in (n = 3, +-25 ms): the frame appears before the text. At the first ink only the
///   brackets and the backdrop show (bracket green 66-144 of 212 at full, so 0.31-0.68); one
///   sample (about 50 ms) later the brackets are at 194 (0.92) and the text at 116-156 of 255;
///   the text is at 195 after that and full about 180-200 ms after the line starts. The lead is
///   drawn as a step (backdrop full, brackets 0.7, inside the measured 0.31-0.92) from the start
///   of the line until the item clip's `FadeOut` label; the ramp itself is not modelled.
/// - Fade-out (n = 3, +-25 ms): the text first, about 200 ms (255, 233, 194, 156/116, 78, 0 in
///   steps of about 47 ms); brackets and backdrop stay about 50-100 ms longer; the whole box is
///   gone about 2.25 s after the line starts (2.23-2.26).
/// - The movie's track gives: text full 7 frames (0.233 s) after the line starts against 0.18-0.20
///   measured, a fade-out of 6 frames (0.2 s, as measured), and the text gone 72 frames (2.4 s)
///   after the start against about 2.25 s measured for the box. The code that times the hold is
///   not in the data; the residuals are left as they are, not fitted.
/// - The same text again while a line with it is on show (n = 4 repeats, 2 sources, high
///   confidence): not added, and the shown line is not refreshed (it faded on schedule). Whether
///   the same text can come back right after the line ended is not known; here it can.
///
/// Unobserved (the original cannot raise more than one line): the order of
/// several lines (chosen: the oldest on top, new lines below), whether the others slide when a
/// line comes or goes (the movie has `UpdatePositions` and `bAnimating` but no duration or
/// distance; chosen: lines jump to their slots), what a line over the limit does (chosen: it
/// waits for a free slot, and only as many lines wait as the list shows), the pitch (chosen: box
/// height plus the movie's spacing), whether the clip plays straight through its frames (chosen;
/// consistent with the measured life of one line) and the stack height beyond one line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct MessageListLook {
    pub backdrop: BackdropLook,
    pub frame_lead: FrameLead,
}

/// How opaque the frame is while a line fades in, as a fraction of full.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct FrameLead {
    pub backdrop: f32,
    pub brackets: f32,
}

pub(super) const OBSERVED: MessageListLook = MessageListLook {
    backdrop: BackdropLook {
        colour: [0, 69, 0],
        alpha: 0.815,
    },
    frame_lead: FrameLead {
        backdrop: 1.0,
        brackets: 0.7,
    },
};

/// The measured numbers [`OBSERVED`] and the layout are checked against (not read at run time),
/// from the same session and under the same conditions.
#[cfg(test)]
pub(super) struct Measured {
    /// Box of one line: x 64.0-407.2, y 77.6-109.6 (n = 2 pictures, spread 0), the allowed
    /// deviation per edge (left, top, right, bottom): the left edge is 1.0 off the movie's
    /// numbers for a reason the data does not give.
    pub frame: [f32; 4],
    pub frame_tolerance: [f32; 4],
    /// Bar 1.9 thick (n = 1, 3 frames); hooks 8.0 long, about 2.0 thick, inward, top and bottom,
    /// no line along the top and bottom edges.
    pub bar: f32,
    pub hook_length: f32,
    pub hook_thickness: f32,
    /// Bracket shadow: near black, about +1.6 right and down (n = 1, 3 frames).
    pub shadow_offset: f32,
    /// Text ink: x 76.0-286.4, y 85.6-103.2, baseline 100.0 (n = 1, 3 frames); insets from the
    /// box left 12.0 and top 8.0.
    pub ink_left: f32,
    pub ink_top: f32,
    pub baseline: f32,
    pub inset_left: f32,
    /// The HUD colour of the brackets, (19-20, 255, 22-23).
    pub hud_colour: [u8; 3],
    /// Backdrop scenes, per channel the lowest and highest value.
    pub scene: [[f32; 2]; 3],
    pub interior: [[f32; 2]; 3],
    /// Milliseconds after the line starts: the text full (180-200), the fade-out duration of the
    /// text (200) and the whole box gone (2250), and how far the movie's track may be off them.
    pub text_full_ms: u32,
    pub text_fade_out_ms: u32,
    pub box_gone_ms: u32,
    pub fade_in_tolerance_ms: u32,
    pub box_gone_tolerance_ms: u32,
}

#[cfg(test)]
pub(super) const MEASURED: Measured = Measured {
    frame: [64.0, 77.6, 407.2, 109.6],
    frame_tolerance: [1.0, 0.5, 0.5, 1.0],
    bar: 1.9,
    hook_length: 8.0,
    hook_thickness: 2.0,
    shadow_offset: 1.6,
    ink_left: 76.0,
    ink_top: 85.6,
    baseline: 100.0,
    inset_left: 12.0,
    hud_colour: [19, 255, 22],
    scene: [[101.0, 107.0], [96.0, 101.0], [86.0, 91.0]],
    interior: [[43.0, 45.0], [72.0, 73.0], [38.0, 39.0]],
    text_full_ms: 190,
    text_fade_out_ms: 200,
    box_gone_ms: 2250,
    fade_in_tolerance_ms: 50,
    box_gone_tolerance_ms: 200,
};

pub(super) struct MessagesKeys {
    origin: PlacementKey,
    max_shown: NumberKey,
    spacing: NumberKey,
    fade_in: LabelKey,
    fade_out: LabelKey,
    frames: SymbolTimelineKey,
    item_clip: PlacementKey,
    text: TextKey,
    max_clip_height: LiteralKey,
    brackets: BracketKeys,
    backdrop: BackdropKeys,
}

key_check_fields!(MessagesKeys {
    origin,
    max_shown,
    spacing,
    fade_in,
    fade_out,
    frames,
    item_clip,
    text,
    max_clip_height,
    brackets,
    backdrop,
});

/// The setting literals of the list component, which carry its bracket, backdrop and clip height.
macro_rules! component_prefix {
    () => {
        "HUDMenu_fla.HUDNotificationsGroup_116.__setProp_Messages_mc_HUDNotificationsGroup_Messages_mc_0#Messages_mc"
    };
}

pub(super) static KEYS: MessagesKeys = MessagesKeys {
    origin: PlacementKey::new(MOVIE, "HUDNotificationsGroup_mc/Messages_mc"),
    max_shown: NumberKey::new(MOVIE, "Messages.MAX_SHOWN"),
    spacing: NumberKey::new(MOVIE, "Messages.MessageSpacing"),
    fade_in: LabelKey::symbol(MOVIE, ITEM_SYMBOL, "FadeIn"),
    fade_out: LabelKey::symbol(MOVIE, ITEM_SYMBOL, "FadeOut"),
    frames: SymbolTimelineKey::new(MOVIE, ITEM_SYMBOL),
    item_clip: PlacementKey::new(MOVIE, "[HUDMessageItem]/HUDMessageItemInternal_mc"),
    text: TextKey::new(
        MOVIE,
        "[HUDMessageItem]/HUDMessageItemInternal_mc/MessageText_tf",
    ),
    max_clip_height: LiteralKey::new(MOVIE, concat!(component_prefix!(), ".maxClipHeight="), 1),
    brackets: bracket_keys!(MOVIE, component_prefix!()),
    backdrop: backdrop_keys!(MOVIE, component_prefix!()),
};

/// The list's values from the layout, resolved once. Positions are stage pixels of the HUD movie.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct MessagesSpec {
    list: ListSpec,
    /// The registration point of the list.
    origin: (f32, f32),
    text: TextFace,
    /// The tallest a line's text gets before its lines are cut off.
    max_text_height: f32,
    /// The alpha animation of a line's item clip.
    track: Vec<TrackKey>,
    base: ColorTransform,
    brackets: Option<BracketSpec>,
    backdrop: Option<BackdropSpec>,
}

impl MessagesSpec {
    pub(super) fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let origin = layout.get(&KEYS.origin)?.world.matrix;
        let item = layout.get(&KEYS.item_clip)?;
        if item.track().is_empty() {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                KEYS.item_clip.path,
                "the message item clip has no alpha track",
            ));
        }
        let text = TextFace::resolve(layout, &KEYS.text, None)?;
        Ok(Self {
            list: list_spec(layout)?,
            origin: (origin.tx, origin.ty),
            max_text_height: max_text_height(layout, &text)?,
            text,
            track: item.track().to_vec(),
            base: item.local.color,
            brackets: BracketSpec::resolve(layout, &KEYS.brackets)?,
            backdrop: BackdropSpec::resolve(layout, &KEYS.backdrop)?,
        })
    }

    /// The family of the face the lines are drawn in.
    pub(super) fn family(&self) -> &str {
        &self.text.family
    }
}

fn list_spec(layout: &UiLayout) -> Result<ListSpec, UiLayoutError> {
    let spec = ListSpec {
        max_shown: layout.whole_constant(&KEYS.max_shown)? as usize,
        spacing: layout.get(&KEYS.spacing)? as f32,
        fps: layout.movie(MOVIE)?.stage().fps,
        fade_in_frame: KEYS.fade_in.resolve(layout)?,
        fade_out_frame: KEYS.fade_out.resolve(layout)?,
        last_frame: layout.get(&KEYS.frames)?.frame_count,
    };
    let invalid = |key: &str, reason: String| UiLayoutError::unresolved(MOVIE, key, reason);
    if spec.max_shown == 0 {
        return Err(invalid(
            KEYS.max_shown.path,
            "the list would never show a line".to_owned(),
        ));
    }
    if !(spec.spacing.is_finite() && spec.spacing >= 0.0) {
        return Err(invalid(
            KEYS.spacing.path,
            format!("the spacing {} is not a length", spec.spacing),
        ));
    }
    if !(spec.fps.is_finite() && spec.fps > 0.0) {
        return Err(invalid(
            "stage",
            format!("the frame rate {} is not positive", spec.fps),
        ));
    }
    let ordered = 1 <= spec.fade_in_frame
        && spec.fade_in_frame <= spec.fade_out_frame
        && spec.fade_out_frame <= spec.last_frame;
    if !ordered {
        return Err(invalid(
            KEYS.fade_out.path,
            format!(
                "the frames FadeIn {}, FadeOut {} and last {} are not in order",
                spec.fade_in_frame, spec.fade_out_frame, spec.last_frame
            ),
        ));
    }
    Ok(spec)
}

/// The clip height limit the movie sets on the component, less the field's own top and bottom
/// gutter.
fn max_text_height(layout: &UiLayout, text: &TextFace) -> Result<f32, UiLayoutError> {
    let clip = layout.lit_number(&KEYS.max_clip_height, 0)? as f32;
    let height = clip - 2.0 * text.geometry.vertical_inset;
    if height.is_finite() && height > 0.0 {
        Ok(height)
    } else {
        Err(UiLayoutError::unresolved(
            MOVIE,
            KEYS.max_clip_height.path,
            format!("the clip height {clip} leaves no room for a line of text"),
        ))
    }
}

/// A line as laid out: its text item, the stage box around it, and how opaque the frame is on its
/// account.
struct Line {
    item: TextItem,
    boxed: [f32; 4],
    backdrop: f32,
    brackets: f32,
}

/// The frame around the lines on show, and how opaque its parts are.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ShownFrame {
    rect: PxRect,
    backdrop: f32,
    brackets: f32,
}

/// The stack of lines: their text items and the frame around them.
pub(super) struct MessagesWidget {
    spec: MessagesSpec,
    look: MessageListLook,
    list: MessageList,
    /// The text height of a line as measured, for the framebuffer size it was measured at.
    heights: Vec<(Arc<str>, f32)>,
    heights_for: (u32, u32),
    /// What `items` and `frame` were laid out for: the lines and their clip frames, and the size.
    drawn: Vec<(Arc<str>, u32)>,
    drawn_for: (u32, u32),
    items: Vec<TextItem>,
    frame: Option<ShownFrame>,
}

impl MessagesWidget {
    pub(super) fn new(spec: MessagesSpec, look: MessageListLook) -> Self {
        Self {
            list: MessageList::new(spec.list.clone()),
            spec,
            look,
            heights: Vec::new(),
            heights_for: (0, 0),
            drawn: Vec::new(),
            drawn_for: (0, 0),
            items: Vec::new(),
            frame: None,
        }
    }

    /// Queues a line. A blank one is refused with a warning: it would show an empty frame.
    pub(super) fn push(&mut self, text: Arc<str>) {
        if text.trim().is_empty() {
            tracing::warn!("a blank message line was not shown");
            return;
        }
        self.list.push(text);
    }

    pub(super) fn items(&self) -> &[TextItem] {
        &self.items
    }

    /// Plays the list on by `dt` seconds (at most a quarter of a second) and lays the lines out on
    /// a framebuffer of `fb` pixels laid out by `layout`; true when the text items changed. A line's
    /// text and its shadow fade with the alpha of its item clip's track; the frame leads the text
    /// while a line fades in and follows it after.
    pub(super) fn update(
        &mut self,
        dt: f32,
        fb: (u32, u32),
        layout: &Layout,
        tint: &UiTint,
        mut measure: impl FnMut(&TextItem) -> TextExtent,
    ) -> bool {
        self.list.advance(dt.clamp(0.0, MAX_STEP_SECONDS));
        if fb.0 == 0 || fb.1 == 0 {
            return self.clear();
        }
        if self.is_drawn(fb) {
            return false;
        }
        if self.heights_for != fb {
            self.heights.clear();
            self.heights_for = fb;
        }
        let lines = self.lay_out(layout, tint, &mut measure);
        self.frame = self.frame_around(&lines, layout);
        let items: Vec<TextItem> = lines.into_iter().map(|line| line.item).collect();
        self.remember(fb);
        let changed = items != self.items;
        self.items = items;
        changed
    }

    /// The brackets, then the backdrop (a flat rectangle is pushed after every shadowed one).
    pub(super) fn emit(&self, qb: &mut QuadBuilder, layout: &Layout) {
        let Some(frame) = &self.frame else {
            return;
        };
        if let Some(brackets) = &self.spec.brackets {
            brackets.emit(qb, layout, frame.rect, frame.brackets);
        }
        if self.spec.backdrop.is_some() {
            BackdropSpec::emit(qb, frame.rect, self.look.backdrop, frame.backdrop);
        }
    }

    fn clear(&mut self) -> bool {
        let had_items = !self.items.is_empty();
        self.items.clear();
        self.frame = None;
        self.drawn.clear();
        self.drawn_for = (0, 0);
        had_items
    }

    fn is_drawn(&self, fb: (u32, u32)) -> bool {
        let shown = self.list.shown();
        self.drawn_for == fb
            && shown.len() == self.drawn.len()
            && shown
                .iter()
                .zip(&self.drawn)
                .all(|(message, (text, frame))| {
                    Arc::ptr_eq(&message.text, text) && message.frame as u32 == *frame
                })
    }

    fn remember(&mut self, fb: (u32, u32)) {
        let shown = self.list.shown();
        self.drawn.clear();
        self.drawn.extend(
            shown
                .iter()
                .map(|message| (Arc::clone(&message.text), message.frame as u32)),
        );
        self.drawn_for = fb;
        self.heights
            .retain(|(text, _)| shown.iter().any(|message| message.text == *text));
    }

    fn lay_out(
        &mut self,
        layout: &Layout,
        tint: &UiTint,
        measure: &mut impl FnMut(&TextItem) -> TextExtent,
    ) -> Vec<Line> {
        let scale = layout.scale();
        let map = ScreenMap {
            origin: (0.0, 0.0),
            scale,
        };
        let (origin_x, origin_y) = self.spec.origin;
        let fade_out = self.spec.list.fade_out_frame as f32;
        let mut offset = 0.0;
        let mut lines = Vec::new();
        for message in self.list.shown() {
            let geometry = self
                .spec
                .text
                .geometry
                .translated(origin_x, origin_y + offset);
            let mut item = self
                .spec
                .text
                .item(geometry, message.text.to_string(), map, tint);
            item.shrink = true;
            item.wrap_width = geometry.wraps.then(|| layout.s(geometry.content_width()));
            item.max_height = Some(layout.s(self.spec.max_text_height));
            let measured = if let Some((_, height)) =
                self.heights.iter().find(|(text, _)| *text == message.text)
            {
                *height
            } else {
                // The text height of the field leaves out the leading below its last line.
                let height = measure(&item).height / scale - geometry.leading;
                self.heights.push((Arc::clone(&message.text), height));
                height
            };
            let boxed = geometry.auto_height(measured.min(self.spec.max_text_height));
            item.geometry = boxed;
            let alpha = alpha_at_frame(&self.spec.track, &self.spec.base, message.frame as u32);
            fade(&mut item, alpha);
            let leading = message.frame < fade_out;
            lines.push(Line {
                item,
                boxed: [boxed.left, boxed.top, boxed.right, boxed.bottom],
                backdrop: if leading {
                    self.look.frame_lead.backdrop
                } else {
                    alpha
                },
                brackets: if leading {
                    self.look.frame_lead.brackets
                } else {
                    alpha
                },
            });
            offset += boxed.bottom - boxed.top + self.spec.list.spacing;
        }
        lines
    }

    /// The frame around the boxes of `lines`, pinned to the framebuffer's top-left corner like
    /// the meters are pinned to their bottom corners.
    fn frame_around(&self, lines: &[Line], layout: &Layout) -> Option<ShownFrame> {
        let mut boxes = lines.iter().map(|line| line.boxed);
        let first = boxes.next()?;
        let [left, top, right, bottom] = boxes.fold(first, |[l, t, r, b], [bl, bt, br, bb]| {
            [l.min(bl), t.min(bt), r.max(br), b.max(bb)]
        });
        let content = PxRect {
            x: layout.s(left),
            y: layout.s(top),
            w: layout.s(right - left),
            h: layout.s(bottom - top),
        };
        let strongest = |part: fn(&Line) -> f32| lines.iter().map(part).fold(0.0, f32::max);
        Some(ShownFrame {
            rect: self
                .spec
                .brackets
                .map_or(content, |brackets| brackets.frame(content, layout)),
            backdrop: strongest(|line| line.backdrop),
            brackets: strongest(|line| line.brackets),
        })
    }
}

/// `item`'s text and shadow at `alpha` of the alpha they have.
fn fade(item: &mut TextItem, alpha: f32) {
    let scaled = |byte: u8| (f32::from(byte) * alpha).round().clamp(0.0, 255.0) as u8;
    item.colour[3] = scaled(item.colour[3]);
    if let Some(shadow) = &mut item.shadow {
        shadow.colour[3] = scaled(shadow.colour[3]);
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::rendering::hud::quad::recorded_rects;
    use crate::rendering::hud::shadow;
    use crate::rendering::lighting::cell_lighting::srgb_to_linear;
    use crate::rendering::ui_colour::linear_to_srgb_u8;
    use crate::ui_layout::test_support::{
        FIXTURE_REGULAR_CLASS, instance, literal, synthesized_layout,
    };
    use serde_json::{Value, json};

    const ORIGIN_PATH: &str = "HUDNotificationsGroup_mc/Messages_mc";
    const CLIP_PATH: &str = "[HUDMessageItem]/HUDMessageItemInternal_mc";
    const TEXT_PATH: &str = "[HUDMessageItem]/HUDMessageItemInternal_mc/MessageText_tf";
    const CHARS_PER_LINE: usize = 10;
    /// 31 characters, as long as the measured line, so the fake measure gives it the same height.
    const SYNTHETIC_LINE: &str = "Too heavy for the test to lift.";
    const LINE_HEIGHT: f32 = 20.0;
    const MAX_BEARING: f32 = 2.0;
    const FPS: f32 = 30.0;

    const TINT: UiTint = UiTint {
        color: [1.0; 3],
        opacity: 1.0,
    };

    /// The movie numbers a fixture is written with.
    #[derive(Clone, Copy)]
    struct Numbers {
        origin: (f64, f64),
        max_shown: u32,
        spacing: f64,
        field: [f64; 4],
        margin: f64,
        pad: (f64, f64),
        line: f64,
        corner: f64,
        leading: f64,
        fade: (u32, u32),
        item_frames: u32,
        max_clip: f64,
        movie_track: bool,
        brackets: bool,
        backdrop: bool,
    }

    /// Numbers that are not the game's.
    const OTHER: Numbers = Numbers {
        origin: (30.0, 50.0),
        max_shown: 3,
        spacing: 5.0,
        field: [-1.0, -1.0, 99.0, 21.0],
        margin: 1.0,
        pad: (4.0, 3.0),
        line: 2.0,
        corner: 5.0,
        leading: 0.0,
        fade: (3, 5),
        item_frames: 9,
        max_clip: 60.0,
        movie_track: false,
        brackets: true,
        backdrop: true,
    };

    /// The numbers of the converted `HUDMenu`, for the checks against the measured original.
    const MOVIE_NUMBERS: Numbers = Numbers {
        origin: (71.0, 80.0),
        max_shown: 4,
        spacing: 4.0,
        field: [-2.0, -2.0, 328.0, 27.05],
        margin: 2.0,
        pad: (6.0, 2.0),
        line: 2.0,
        corner: 6.0,
        leading: -2.0,
        fade: (6, 14),
        item_frames: 85,
        max_clip: 160.0,
        movie_track: true,
        brackets: true,
        backdrop: true,
    };

    fn key(frame: u32, alpha: f64) -> Value {
        json!({
            "frame": frame, "matrix": [1, 0, 0, 1, 0, 0], "visible": true,
            "color": {"mult": [1, 1, 1, alpha], "add": [0, 0, 0, 0]},
        })
    }

    /// The alpha track of the converted `HUDMessageItem` clip.
    fn movie_track() -> Vec<Value> {
        let fade_in = [
            0.140_625,
            0.281_25,
            0.421_875,
            0.570_312_5,
            0.710_937_5,
            0.851_562_5,
        ];
        let mut keys = vec![key(1, 0.0)];
        keys.extend(
            (7..=12)
                .zip(fade_in)
                .map(|(frame, alpha)| key(frame, alpha)),
        );
        keys.push(key(13, 1.0));
        keys.extend(
            (72..=77)
                .zip(fade_in.into_iter().rev())
                .map(|(frame, alpha)| key(frame, alpha)),
        );
        keys.push(key(78, 0.0));
        keys
    }

    fn clip(numbers: Numbers, with_track: bool) -> Value {
        let mut clip = instance(1, "sprite", 0.0, 0.0);
        if with_track {
            clip["track"] = if numbers.movie_track {
                json!(movie_track())
            } else {
                json!([
                    key(1, 0.0),
                    key(4, 0.5),
                    key(5, 1.0),
                    key(8, 0.5),
                    key(9, 0.0)
                ])
            };
        }
        clip
    }

    fn overlay(numbers: Numbers, style: &str, method: &str, with_track: bool) -> Value {
        let setting = |name: &str| format!("{}.{name}", component_prefix!());
        json!({
            "characters": {
                "900": {"kind": "sprite", "frameCount": numbers.item_frames,
                        "labels": {"FadeIn": numbers.fade.0, "FadeOut": numbers.fade.1}},
                "910": {
                    "kind": "text", "bounds": numbers.field, "font": FIXTURE_REGULAR_CLASS,
                    "size": 20, "color": [255, 255, 255, 255], "align": "left",
                    "leftMargin": numbers.margin, "rightMargin": numbers.margin,
                    "indent": 0, "leading": numbers.leading,
                    "flags": {"wordWrap": true, "multiline": true, "autoSize": false, "html": false},
                },
            },
            "symbols": {"HUDMessageItem": 900},
            "instances": {
                ORIGIN_PATH: instance(1, "sprite", numbers.origin.0, numbers.origin.1),
                CLIP_PATH: clip(numbers, with_track),
                TEXT_PATH: instance(910, "text", 2.0, 2.0),
            },
            "constants": {
                "Messages.MAX_SHOWN": numbers.max_shown,
                "Messages.MessageSpacing": numbers.spacing,
            },
            "literals": {
                setting("maxClipHeight="): literal(&json!(numbers.max_clip)),
                setting("bShowBrackets="): literal(&json!(numbers.brackets)),
                setting("BracketStyle="): literal(&json!(style)),
                setting("bracketCornerLength="): literal(&json!(numbers.corner)),
                setting("bracketLineWidth="): literal(&json!(numbers.line)),
                setting("bracketPaddingX="): literal(&json!(numbers.pad.0)),
                setting("bracketPaddingY="): literal(&json!(numbers.pad.1)),
                setting("bUseShadedBackground="): literal(&json!(numbers.backdrop)),
                setting("ShadedBackgroundMethod="): literal(&json!(method)),
                setting("ShadedBackgroundType="): literal(&json!("normal")),
            },
        })
    }

    fn layout_with(numbers: Numbers, style: &str, method: &str, with_track: bool) -> UiLayout {
        synthesized_layout(
            &[&KEYS],
            &[(MOVIE, overlay(numbers, style, method, with_track))],
        )
    }

    fn layout_of(numbers: Numbers) -> UiLayout {
        layout_with(numbers, "horizontal", "Shader", true)
    }

    fn spec_of(numbers: Numbers) -> MessagesSpec {
        MessagesSpec::resolve(&layout_of(numbers)).expect("the fixture resolves")
    }

    /// The synthetic message list spec the scene tests of the HUD are built with.
    pub(in crate::rendering::hud) fn synthetic_spec() -> MessagesSpec {
        spec_of(OTHER)
    }

    const LOOK: MessageListLook = MessageListLook {
        backdrop: BackdropLook {
            colour: [0, 255, 0],
            alpha: 0.5,
        },
        frame_lead: FrameLead {
            backdrop: 0.9,
            brackets: 0.6,
        },
    };

    fn widget_of(numbers: Numbers) -> MessagesWidget {
        MessagesWidget::new(spec_of(numbers), LOOK)
    }

    fn widget() -> MessagesWidget {
        widget_of(OTHER)
    }

    fn lines_of(item: &TextItem) -> usize {
        item.text.len().div_ceil(CHARS_PER_LINE).max(1)
    }

    fn fake_measure(item: &TextItem) -> TextExtent {
        TextExtent {
            left: 0.0,
            top: 0.0,
            width: 0.0,
            height: lines_of(item) as f32 * LINE_HEIGHT * item.map.scale,
        }
    }

    fn step(widget: &mut MessagesWidget, dt: f32, fb: (u32, u32)) -> bool {
        widget.update(dt, fb, &Layout::hud_stage(fb), &TINT, fake_measure)
    }

    fn frames(count: f32) -> f32 {
        count / FPS
    }

    fn push(widget: &mut MessagesWidget, text: &str) {
        widget.push(text.into());
    }

    fn frame_of(widget: &MessagesWidget) -> ShownFrame {
        widget.frame.expect("a line is shown")
    }

    fn alphas(widget: &MessagesWidget) -> Vec<u8> {
        widget.items().iter().map(|item| item.colour[3]).collect()
    }

    fn emitted_with(
        widget: &MessagesWidget,
        fb: (u32, u32),
        shadow: f32,
    ) -> Vec<(PxRect, [f32; 4])> {
        let mut verts = Vec::new();
        let mut qb = QuadBuilder::new(
            &mut verts,
            fb.0 as f32,
            fb.1 as f32,
            shadow,
            [0.0, 0.0, 0.0, 0.6],
        );
        widget.emit(&mut qb, &Layout::hud_stage(fb));
        qb.finish();
        recorded_rects(&verts, (fb.0 as f32, fb.1 as f32))
    }

    fn emitted(widget: &MessagesWidget, fb: (u32, u32)) -> Vec<(PxRect, [f32; 4])> {
        emitted_with(widget, fb, 0.0)
    }

    const HD: (u32, u32) = (1280, 720);

    #[test]
    fn the_spec_reads_the_movie_the_list_is_authored_in() {
        let spec = spec_of(OTHER);
        assert_eq!(
            spec.list,
            ListSpec {
                max_shown: 3,
                spacing: 5.0,
                fps: FPS,
                fade_in_frame: 3,
                fade_out_frame: 5,
                last_frame: OTHER.item_frames,
            }
        );
        assert_eq!(spec.origin, (30.0, 50.0));
        assert_eq!(spec.track.len(), 5);
        assert!((spec.max_text_height - (60.0 - 2.0 * 2.0)).abs() < 1e-6);
        assert!(spec.brackets.is_some() && spec.backdrop.is_some());
    }

    #[test]
    fn a_layout_without_brackets_or_backdrop_gets_neither() {
        let numbers = Numbers {
            brackets: false,
            backdrop: false,
            ..OTHER
        };
        let spec = spec_of(numbers);
        assert!(spec.brackets.is_none() && spec.backdrop.is_none());
    }

    #[test]
    fn a_style_a_method_a_stack_a_clip_or_a_timeline_the_hud_cannot_play_is_refused() {
        let error = |layout: UiLayout| MessagesSpec::resolve(&layout).unwrap_err().to_string();
        assert!(error(layout_with(OTHER, "vertical", "Shader", true)).contains("BracketStyle"));
        assert!(
            error(layout_with(OTHER, "horizontal", "Blur", true))
                .contains("ShadedBackgroundMethod")
        );
        let empty_stack = Numbers {
            max_shown: 0,
            ..OTHER
        };
        assert!(error(layout_of(empty_stack)).contains("MAX_SHOWN"));
        let reversed = Numbers {
            fade: (5, 3),
            ..OTHER
        };
        assert!(error(layout_of(reversed)).contains("FadeOut"));
        let flat_clip = Numbers {
            max_clip: 4.0,
            ..OTHER
        };
        assert!(error(layout_of(flat_clip)).contains("maxClipHeight"));
        assert!(error(layout_with(OTHER, "horizontal", "Shader", false)).contains("alpha track"));
    }

    #[test]
    fn the_frame_around_one_line_is_the_measured_box() {
        let mut widget = widget_of(MOVIE_NUMBERS);
        push(&mut widget, SYNTHETIC_LINE);
        step(&mut widget, 0.0, HD);
        let rect = frame_of(&widget).rect;
        let [x0, y0, x1, _] = MEASURED.frame;
        let [left, top, right, _] = MEASURED.frame_tolerance;
        for (actual, measured, tolerance) in [
            (rect.x, x0, left),
            (rect.y, y0, top),
            (rect.x + rect.w, x1, right),
        ] {
            assert!(
                (actual - measured).abs() <= tolerance,
                "{actual} against measured {measured}"
            );
        }
    }

    #[test]
    fn the_text_sits_where_the_measured_ink_sits() {
        let mut widget = widget_of(MOVIE_NUMBERS);
        push(&mut widget, SYNTHETIC_LINE);
        step(&mut widget, 0.0, HD);
        let rect = frame_of(&widget).rect;
        let geometry = widget.items()[0].geometry;
        assert!(geometry.content_left() <= MEASURED.ink_left);
        assert!(MEASURED.ink_left - geometry.content_left() <= MAX_BEARING);
        assert!(geometry.line_top() <= MEASURED.ink_top && MEASURED.ink_top < MEASURED.baseline);
        let inset = geometry.content_left() - rect.x;
        assert!(inset <= MEASURED.inset_left && MEASURED.inset_left <= inset + MAX_BEARING);
        let [_, y0, _, _] = MEASURED.frame;
        assert!(geometry.line_top() - rect.y <= MEASURED.ink_top - y0);
    }

    #[test]
    fn the_bracket_of_one_line_has_the_measured_bar_and_hooks() {
        let mut widget = widget_of(MOVIE_NUMBERS);
        push(&mut widget, "line");
        step(&mut widget, 0.0, HD);
        let brackets: Vec<PxRect> = emitted(&widget, HD)
            .into_iter()
            .skip(1)
            .map(|(rect, _)| rect)
            .collect();
        let (bar, hook) = (brackets[0], brackets[1]);
        assert!((bar.w - MEASURED.bar).abs() <= 0.2, "{bar:?}");
        assert!((hook.h - MEASURED.hook_thickness).abs() <= 0.2, "{hook:?}");
        assert!(
            ((hook.x + hook.w - bar.x) - MEASURED.hook_length).abs() <= 0.2,
            "{hook:?}"
        );
    }

    #[test]
    fn the_engine_shadow_is_the_measured_bracket_shadow() {
        assert!((shadow::QUAD_OFFSET_STAGE_PX - MEASURED.shadow_offset).abs() <= 0.2);
    }

    #[test]
    fn a_line_is_as_wide_as_the_field_and_wraps_at_its_content_width() {
        let mut widget = widget();
        push(&mut widget, "a line");
        step(&mut widget, 0.0, (2560, 1440));
        let item = &widget.items()[0];
        let content = item.geometry.content_width();
        assert!((item.wrap_width.unwrap() - content * 2.0).abs() < 1e-3);
        let rect = frame_of(&widget).rect;
        let field_width = item.geometry.right - item.geometry.left;
        assert!((rect.w - (field_width + 2.0 * OTHER.pad.0 as f32) * 2.0).abs() < 1e-3);
    }

    #[test]
    fn a_word_wider_than_the_line_shrinks_instead_of_overrunning_the_bracket() {
        let mut widget = widget();
        push(&mut widget, "a line");
        step(&mut widget, 0.0, HD);
        assert!(widget.items()[0].shrink);
    }

    #[test]
    fn the_frame_grows_with_the_wrapped_height_of_a_line() {
        let tall = Numbers {
            max_clip: 200.0,
            ..OTHER
        };
        let mut short = widget_of(tall);
        push(&mut short, "one");
        step(&mut short, 0.0, HD);
        let mut long = widget_of(tall);
        push(&mut long, "a text of three lines");
        step(&mut long, 0.0, HD);
        let growth = frame_of(&long).rect.h - frame_of(&short).rect.h;
        assert!((growth - 2.0 * LINE_HEIGHT).abs() < 1e-3, "{growth}");
    }

    #[test]
    fn the_text_height_leaves_out_the_leading_below_the_last_line() {
        let mut widget = widget_of(MOVIE_NUMBERS);
        push(&mut widget, "line");
        step(&mut widget, 0.0, HD);
        let gutters_and_padding = 2.0 * 2.0 + 2.0 * MOVIE_NUMBERS.pad.1 as f32;
        let height = frame_of(&widget).rect.h;
        assert!(
            (height - (LINE_HEIGHT + 2.0 + gutters_and_padding)).abs() < 1e-3,
            "{height}"
        );
    }

    #[test]
    fn a_line_never_grows_past_the_clip_height_of_the_movie() {
        let mut widget = widget();
        push(&mut widget, &"x".repeat(200));
        step(&mut widget, 0.0, (2560, 1440));
        let item = &widget.items()[0];
        let cap = OTHER.max_clip as f32 - 2.0 * 2.0;
        assert!((item.max_height.unwrap() - cap * 2.0).abs() < 1e-3);
        let box_height = item.geometry.bottom - item.geometry.top;
        assert!(
            (box_height - OTHER.max_clip as f32).abs() < 1e-3,
            "{box_height}"
        );
        let rect = frame_of(&widget).rect;
        assert!((rect.h - (OTHER.max_clip as f32 + 2.0 * OTHER.pad.1 as f32) * 2.0).abs() < 1e-3);
    }

    #[test]
    fn a_blank_line_shows_nothing() {
        let mut widget = widget();
        push(&mut widget, "");
        push(&mut widget, "  ");
        assert!(!step(&mut widget, 0.0, HD));
        assert!(widget.items().is_empty() && widget.frame.is_none());
    }

    #[test]
    fn lines_stack_downwards_in_order_of_arrival_by_their_box_plus_the_spacing() {
        let mut widget = widget();
        push(&mut widget, "first");
        push(&mut widget, "second");
        step(&mut widget, 0.0, HD);
        let items = widget.items();
        assert_eq!(items[0].text, "first");
        assert_eq!(items[1].text, "second");
        let box_height = items[0].geometry.bottom - items[0].geometry.top;
        let pitch = items[1].geometry.top - items[0].geometry.top;
        assert!(
            (pitch - (box_height + OTHER.spacing as f32)).abs() < 1e-3,
            "{pitch}"
        );
    }

    #[test]
    fn one_frame_surrounds_every_line() {
        let mut one = widget();
        push(&mut one, "first");
        step(&mut one, 0.0, HD);
        let mut two = widget();
        push(&mut two, "first");
        push(&mut two, "second");
        step(&mut two, 0.0, HD);
        let (one, two) = (frame_of(&one).rect, frame_of(&two).rect);
        assert_eq!((one.x, one.y, one.w), (two.x, two.y, two.w));
        let box_height = LINE_HEIGHT + 2.0 * 2.0;
        assert!((two.h - one.h - (box_height + OTHER.spacing as f32)).abs() < 1e-3);
    }

    #[test]
    fn the_layout_scales_with_the_framebuffer() {
        let mut small = widget();
        push(&mut small, "a line");
        step(&mut small, 0.0, HD);
        let mut large = widget();
        push(&mut large, "a line");
        step(&mut large, 0.0, (2560, 1440));
        let (a, b) = (frame_of(&small).rect, frame_of(&large).rect);
        for (small, large) in [(a.x, b.x), (a.y, b.y), (a.w, b.w), (a.h, b.h)] {
            assert!((large - 2.0 * small).abs() < 1e-3);
        }
    }

    #[test]
    fn the_list_hugs_the_top_left_corner_of_the_screen_whatever_its_shape() {
        for (fb, scale) in [((3440_u32, 1440_u32), 2.0_f32), ((1024, 768), 0.8)] {
            let mut widget = widget();
            push(&mut widget, "a line");
            step(&mut widget, 0.0, fb);
            let rect = frame_of(&widget).rect;
            let left = widget.items()[0].geometry.left - OTHER.pad.0 as f32;
            let top = widget.items()[0].geometry.top - OTHER.pad.1 as f32;
            assert!((rect.x - left * scale).abs() < 1e-3, "{fb:?}: {}", rect.x);
            assert!((rect.y - top * scale).abs() < 1e-3, "{fb:?}: {}", rect.y);
        }
    }

    #[test]
    fn the_text_fades_with_the_alpha_track_of_the_item_clip() {
        let mut widget = widget();
        push(&mut widget, "a line");
        step(&mut widget, 0.0, HD);
        assert_eq!(alphas(&widget), [0]);
        step(&mut widget, frames(1.5), HD);
        assert_eq!(alphas(&widget), [128]);
        step(&mut widget, frames(1.0), HD);
        assert_eq!(alphas(&widget), [255]);
        step(&mut widget, frames(3.0), HD);
        assert_eq!(alphas(&widget), [128]);
    }

    #[test]
    fn the_shadow_of_the_text_fades_with_the_text() {
        let full = TINT.shadow_alpha(shadow::TEXT_ALPHA);
        let shadow_alpha = |widget: &MessagesWidget| widget.items()[0].shadow.unwrap().colour[3];
        let mut widget = widget();
        push(&mut widget, "a line");
        step(&mut widget, 0.0, HD);
        assert_eq!(shadow_alpha(&widget), 0);
        step(&mut widget, frames(1.5), HD);
        assert_eq!(shadow_alpha(&widget), (f32::from(full) * 0.5).round() as u8);
        step(&mut widget, frames(1.0), HD);
        assert_eq!(shadow_alpha(&widget), full);
    }

    #[test]
    fn the_hud_opacity_is_the_ceiling_of_the_text_alpha() {
        let faded = UiTint {
            opacity: 0.5,
            ..TINT
        };
        let mut widget = widget();
        push(&mut widget, "a line");
        widget.update(
            frames(2.5),
            HD,
            &Layout::hud_stage(HD),
            &faded,
            fake_measure,
        );
        assert_eq!(alphas(&widget), [128]);
    }

    #[test]
    fn the_text_takes_the_hud_colour() {
        let green = UiTint {
            color: [0.0, 1.0, 0.0],
            opacity: 1.0,
        };
        let mut widget = widget();
        push(&mut widget, "a line");
        widget.update(
            frames(2.5),
            HD,
            &Layout::hud_stage(HD),
            &green,
            fake_measure,
        );
        assert_eq!(widget.items()[0].colour[..3], [0, 255, 0]);
    }

    #[test]
    fn the_frame_leads_a_line_that_fades_in_and_follows_it_after() {
        let mut widget = widget();
        push(&mut widget, "a line");
        step(&mut widget, 0.0, HD);
        let frame = frame_of(&widget);
        assert_eq!(alphas(&widget), [0]);
        assert_eq!((frame.backdrop, frame.brackets), (0.9, 0.6));
        step(&mut widget, frames(2.5), HD);
        let frame = frame_of(&widget);
        assert_eq!((frame.backdrop, frame.brackets), (1.0, 1.0));
        step(&mut widget, frames(3.0), HD);
        let frame = frame_of(&widget);
        assert_eq!((frame.backdrop, frame.brackets), (0.5, 0.5));
    }

    #[test]
    fn the_frame_has_the_strongest_part_of_any_line_not_the_newest() {
        let mut widget = widget();
        push(&mut widget, "old");
        step(&mut widget, frames(2.5), HD);
        push(&mut widget, "new");
        step(&mut widget, 0.0, HD);
        assert_eq!(alphas(&widget), [255, 0]);
        let frame = frame_of(&widget);
        assert_eq!((frame.backdrop, frame.brackets), (1.0, 1.0));
    }

    #[test]
    fn a_new_line_fading_in_lifts_the_frame_over_an_old_line_fading_out() {
        let mut widget = widget();
        push(&mut widget, "old");
        step(&mut widget, frames(5.5), HD);
        assert_eq!(alphas(&widget), [128]);
        push(&mut widget, "new");
        step(&mut widget, 0.0, HD);
        let frame = frame_of(&widget);
        assert_eq!((frame.backdrop, frame.brackets), (0.9, 0.6));
    }

    #[test]
    fn a_line_and_its_frame_leave_together() {
        let mut widget = widget();
        push(&mut widget, "a line");
        step(&mut widget, 0.0, HD);
        assert!(step(&mut widget, 0.25, HD));
        assert!(!step(&mut widget, 0.25, HD));
        assert_eq!(widget.items(), []);
        assert!(widget.frame.is_none());
        assert_eq!(emitted(&widget, HD), []);
    }

    #[test]
    fn a_hitch_does_not_retire_a_line_unseen() {
        let mut widget = widget_of(MOVIE_NUMBERS);
        push(&mut widget, "a line");
        step(&mut widget, 10.0, HD);
        assert_eq!(widget.items().len(), 1);
        step(&mut widget, f32::INFINITY, HD);
        assert_eq!(widget.items().len(), 1);
        step(&mut widget, -1.0, HD);
        assert_eq!(widget.items().len(), 1);
    }

    #[test]
    fn the_limit_holds_the_newest_line_back_until_a_slot_frees() {
        let mut widget = widget();
        for text in ["a", "b", "c", "d"] {
            push(&mut widget, text);
        }
        step(&mut widget, 0.0, HD);
        let texts: Vec<&str> = widget
            .items()
            .iter()
            .map(|item| item.text.as_str())
            .collect();
        assert_eq!(texts, ["a", "b", "c"]);
    }

    #[test]
    fn items_change_only_when_something_about_them_does() {
        let mut widget = widget();
        assert!(!step(&mut widget, 0.0, HD));
        push(&mut widget, "a line");
        assert!(step(&mut widget, 0.0, HD));
        assert!(!step(&mut widget, 0.0, HD));
        assert!(step(&mut widget, 0.0, (1920, 1080)));
        assert!(step(&mut widget, frames(2.5), (1920, 1080)));
    }

    #[test]
    fn an_unchanged_stack_is_not_laid_out_again() {
        let mut widget = widget();
        let mut measured = 0;
        let mut update = |widget: &mut MessagesWidget, dt: f32, fb: (u32, u32)| {
            widget.update(dt, fb, &Layout::hud_stage(fb), &TINT, |item| {
                measured += 1;
                fake_measure(item)
            });
        };
        push(&mut widget, "first");
        update(&mut widget, 0.0, HD);
        update(&mut widget, 0.0, HD);
        update(&mut widget, frames(0.2), HD);
        push(&mut widget, "second");
        update(&mut widget, 0.0, HD);
        update(&mut widget, 0.0, (1920, 1080));
        assert_eq!(measured, 1 + 1 + 2);
    }

    #[test]
    fn a_framebuffer_without_area_shows_nothing_and_does_not_fault() {
        let mut widget = widget();
        push(&mut widget, "a line");
        assert!(step(&mut widget, 0.0, HD));
        assert!(step(&mut widget, 0.0, (0, 0)));
        assert!(widget.items().is_empty() && widget.frame.is_none());
        assert!(!step(&mut widget, 0.0, (0, 720)));
        assert!(step(&mut widget, 0.0, HD));
        assert_eq!(widget.items().len(), 1);
    }

    #[test]
    fn the_frame_is_brackets_then_a_flat_backdrop_beneath_them() {
        let mut widget = widget();
        push(&mut widget, "a line");
        step(&mut widget, frames(2.5), HD);
        let rects = emitted(&widget, HD);
        assert_eq!(rects.len(), 1 + 6);
        let frame = frame_of(&widget).rect;
        let (backdrop, colour) = rects[0];
        assert!((backdrop.x - frame.x).abs() < 1e-2 && (backdrop.h - frame.h).abs() < 1e-2);
        assert!((colour[3] - LOOK.backdrop.alpha).abs() < 1e-6);
        assert!(rects[1..].iter().all(|(_, colour)| colour[..3] == [1.0; 3]));
    }

    #[test]
    fn the_frame_parts_are_drawn_at_their_own_alpha_and_the_brackets_cast_a_shadow() {
        let mut widget = widget();
        push(&mut widget, "a line");
        step(&mut widget, 0.0, HD);
        let rects = emitted_with(&widget, HD, 1.5);
        assert_eq!(rects.len(), 1 + 6 + 6);
        assert!((rects[0].1[3] - LOOK.backdrop.alpha * 0.9).abs() < 1e-6);
        let shadows = &rects[1..7];
        assert!(
            shadows
                .iter()
                .all(|(_, colour)| (colour[3] - 0.6 * 0.6).abs() < 1e-6)
        );
        let brackets = &rects[7..];
        assert!(
            brackets
                .iter()
                .all(|(_, colour)| (colour[3] - 0.6).abs() < 1e-6)
        );
    }

    #[test]
    fn a_layout_without_brackets_or_backdrop_draws_only_what_it_has() {
        let only_backdrop = Numbers {
            brackets: false,
            ..OTHER
        };
        let mut widget = widget_of(only_backdrop);
        push(&mut widget, "a line");
        step(&mut widget, frames(2.5), HD);
        assert_eq!(emitted(&widget, HD).len(), 1);
        let only_brackets = Numbers {
            backdrop: false,
            ..OTHER
        };
        let mut widget = widget_of(only_brackets);
        push(&mut widget, "a line");
        step(&mut widget, frames(2.5), HD);
        assert_eq!(emitted(&widget, HD).len(), 6);
    }

    /// What the engine puts on screen over `scene` (8-bit sRGB): the quad's linear colour times
    /// the HUD tint, blended in linear light with the scene, encoded by the sRGB surface.
    fn on_screen(look: BackdropLook, tint: [f32; 3], scene: [f32; 3]) -> [f32; 3] {
        let mut out = [0.0; 3];
        for channel in 0..3 {
            let quad = srgb_to_linear(look.colour[channel]) * tint[channel];
            let behind = srgb_to_linear(scene[channel] as u8);
            let blended = look.alpha * quad + (1.0 - look.alpha) * behind;
            out[channel] = f32::from(linear_to_srgb_u8(blended));
        }
        out
    }

    #[test]
    fn the_measured_backdrop_lands_on_the_measured_interior_the_way_the_engine_blends() {
        const TOLERANCE_LEVELS: f32 = 4.0;
        let tint = MEASURED
            .hud_colour
            .map(|channel| f32::from(channel) / 255.0);
        for end in 0..2 {
            let scene = MEASURED.scene.map(|range| range[end]);
            let shown = on_screen(OBSERVED.backdrop, tint, scene);
            for channel in 0..3 {
                let [low, high] = MEASURED.interior[channel];
                assert!(
                    shown[channel] >= low - TOLERANCE_LEVELS
                        && shown[channel] <= high + TOLERANCE_LEVELS,
                    "channel {channel}: {} against {low}..{high} over {}",
                    shown[channel],
                    scene[channel]
                );
            }
        }
    }

    #[test]
    fn the_rejected_srgb_space_fit_would_miss_the_measured_interior_in_the_engine() {
        let tint = MEASURED
            .hud_colour
            .map(|channel| f32::from(channel) / 255.0);
        let rejected = BackdropLook {
            colour: [12, 56, 11],
            alpha: 0.68,
        };
        let shown = on_screen(rejected, tint, MEASURED.scene.map(|range| range[0]));
        assert!(shown[0] > MEASURED.interior[0][1] + 10.0, "{shown:?}");
    }

    /// The life of one line against the measured times, from the line's start: the frame is
    /// there before the text, the text is full, fades out in 200 ms and the box is gone, each
    /// within the stated residual of the movie track.
    #[test]
    fn the_movie_timeline_fits_the_measured_times_of_one_line() {
        let mut widget = widget_of(MOVIE_NUMBERS);
        push(&mut widget, "line");
        let (mut text_full, mut fade_start, mut text_gone, mut box_gone) = (None, None, None, None);
        for ms in 0..=3000_u32 {
            step(&mut widget, 0.001, HD);
            let alpha = widget
                .items()
                .first()
                .map(|item| f32::from(item.colour[3]) / 255.0);
            if ms == 0 {
                assert_eq!(alpha, Some(0.0), "no text at the first ink");
                assert!(widget.frame.is_some_and(|frame| frame.backdrop > 0.0));
            }
            if text_full.is_none() && alpha == Some(1.0) {
                text_full = Some(ms);
            }
            if text_full.is_some() && fade_start.is_none() && alpha.is_some_and(|a| a < 1.0) {
                fade_start = Some(ms);
            }
            if fade_start.is_some() && text_gone.is_none() && alpha == Some(0.0) {
                text_gone = Some(ms);
            }
            let box_alpha = widget
                .frame
                .map_or(0.0, |frame| frame.backdrop.max(frame.brackets));
            if box_gone.is_none() && box_alpha < 0.05 {
                box_gone = Some(ms);
            }
        }
        let near = |actual: Option<u32>, measured: u32, tolerance: u32| {
            let actual = actual.expect("the event happens");
            assert!(
                actual.abs_diff(measured) <= tolerance,
                "{actual} ms against {measured}"
            );
        };
        near(
            text_full,
            MEASURED.text_full_ms,
            MEASURED.fade_in_tolerance_ms,
        );
        let fade = text_gone.unwrap() - fade_start.unwrap();
        near(
            Some(fade),
            MEASURED.text_fade_out_ms,
            MEASURED.fade_in_tolerance_ms,
        );
        near(
            box_gone,
            MEASURED.box_gone_ms,
            MEASURED.box_gone_tolerance_ms,
        );
    }

    /// The measured box of the `sItemTooHeavy` line against the installed movie and font, at two
    /// sizes.
    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the converted interface and fonts"]
    fn the_installed_movie_frames_the_too_heavy_line_as_the_original_does() {
        use crate::assets::layout::{ASSETS_DIR, fonts_dir};
        use crate::rendering::hud::field_text::{baselines_of, extent_of};
        use crate::rendering::ui_fonts::UiFonts;

        let layout = super::super::real_install::installed_layout();
        layout.validate(&[&KEYS]).unwrap();
        let spec = MessagesSpec::resolve(&layout).unwrap();
        let install = std::env::var(crate::assets::INSTALL_DIR_VAR).expect("OPENFO4_INSTALL_DIR");
        let fonts = UiFonts::load(&fonts_dir(&format!("{install}/{ASSETS_DIR}"))).unwrap();
        let mut font_system = fonts.font_system();
        let too_heavy_line = crate::rendering::item_feedback::load_too_heavy_text(
            &crate::assets::db::test_support::installed_database(),
        )
        .unwrap();
        for fb in [HD, (1600, 900)] {
            let hud = Layout::hud_stage(fb);
            let scale = hud.scale();
            let mut widget = MessagesWidget::new(spec.clone(), OBSERVED);
            push(&mut widget, &too_heavy_line);
            widget.update(0.0, fb, &hud, &TINT, |item| {
                extent_of(&mut font_system, item)
            });
            let rect = frame_of(&widget).rect;
            let item = &widget.items()[0];
            let extent = extent_of(&mut font_system, item);
            let baseline = baselines_of(&mut font_system, item)[0];
            println!(
                "{fb:?}: frame {rect:?} ink left {} baseline {baseline}",
                extent.left
            );
            let [x0, y0, x1, y1] = MEASURED.frame;
            let [tl, tt, tr, tb] = MEASURED.frame_tolerance;
            for (actual, measured, tolerance, edge) in [
                (rect.x / scale, x0, tl, "left"),
                (rect.y / scale, y0, tt, "top"),
                ((rect.x + rect.w) / scale, x1, tr, "right"),
                ((rect.y + rect.h) / scale, y1, tb, "bottom"),
                (extent.left / scale, MEASURED.ink_left, 1.0, "ink left"),
                (baseline / scale, MEASURED.baseline, 1.0, "baseline"),
            ] {
                assert!(
                    (actual - measured).abs() <= tolerance,
                    "{fb:?}: {edge} {actual} against measured {measured}"
                );
            }
        }
    }
}
