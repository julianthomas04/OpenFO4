//! The page header (`Pipboy_Header`): the five page titles, the rule broken by brackets around
//! the selected title, and the sub-tab strip centred under it that slides when the sub-tab
//! changes. This Rust implementation re-creates the observed layout behavior using
//! runtime-extracted constants from the installed game's movie; it does not copy its scripts.

use egui::{Painter, Pos2, Rect, Vec2};

use super::nav::{Nav, Page, Step};
use super::scaleform_text::{FieldSpec, PaintedField, TextAutoSize, natural_width, paint_field};
use super::shapes::{Bracket, OutlinedShape, Shade, code_colour};
use super::tab::TabContext;
use crate::ui_layout::scaleform_text::{AutoSize, FIELD_GUTTER_PX};
use crate::ui_layout::set_text::{SetTextKeys, SetTextRules};
use crate::ui_layout::{
    LiteralKey, ModuleKeys, NumberKey, PlacementKey, ShapeKey, StageKey, TextKey, TimelineKey,
    UiLayout, UiLayoutError, key_check_fields,
};

const MOVIE: &str = "PipboyMenu";

macro_rules! page_header {
    ($path:literal) => {
        concat!("Menu_mc/Header_mc/PageHeader_mc/", $path)
    };
}

macro_rules! alpha_holder {
    ($path:literal) => {
        concat!("Menu_mc/Header_mc/TabHeader_mc/AlphaHolder/", $path)
    };
}

macro_rules! code {
    ($path:literal) => {
        concat!("Pipboy_Header.", $path)
    };
}

/// The sub-tab slots from two left of the selected one to two right of it.
const SLOTS: usize = 5;

struct HeaderKeys {
    stage: StageKey,
    titles: [TextKey; Page::ALL.len()],
    selectors: [PlacementKey; 2],
    selector_shapes: [ShapeKey; 2],
    borders: [PlacementKey; 2],
    border_caps: [ShapeKey; 2],
    selector_gaps: [LiteralKey; 2],
    line_width: LiteralKey,
    line_colour: LiteralKey,
    line_alpha: LiteralKey,
    tab_header: PlacementKey,
    tab_timeline: TimelineKey,
    slide: PlacementKey,
    slide_labels: LiteralKey,
    first_script_frame: LiteralKey,
    stop_script_frame: LiteralKey,
    stop_target: LiteralKey,
    strip_divisor: LiteralKey,
    slots: [TextKey; SLOTS],
    slot_clips: [PlacementKey; SLOTS],
    selected_slot: NumberKey,
    tab_spacing: NumberKey,
    half_width: LiteralKey,
    set_text: SetTextKeys,
}

static KEYS: HeaderKeys = HeaderKeys {
    stage: StageKey::of(MOVIE),
    titles: [
        TextKey::new(MOVIE, page_header!("STAT_tf")),
        TextKey::new(MOVIE, page_header!("INV_tf")),
        TextKey::new(MOVIE, page_header!("DATA_tf")),
        TextKey::new(MOVIE, page_header!("MAP_tf")),
        TextKey::new(MOVIE, page_header!("RADIO_tf")),
    ],
    selectors: [
        PlacementKey::new(MOVIE, page_header!("Selector_Left")),
        PlacementKey::new(MOVIE, page_header!("Selector_Right")),
    ],
    selector_shapes: [
        ShapeKey::new(MOVIE, page_header!("Selector_Left/@1")),
        ShapeKey::new(MOVIE, page_header!("Selector_Right/@1")),
    ],
    borders: [
        PlacementKey::new(MOVIE, page_header!("LeftBorder")),
        PlacementKey::new(MOVIE, page_header!("RightBorder")),
    ],
    border_caps: [
        ShapeKey::new(MOVIE, page_header!("LeftBorder/@1")),
        ShapeKey::new(MOVIE, page_header!("RightBorder/@1")),
    ],
    selector_gaps: [
        LiteralKey::new(
            MOVIE,
            code!("redrawPages#pageTextXBounds[_currPageIndex].x-"),
            1,
        ),
        LiteralKey::new(
            MOVIE,
            code!("redrawPages#pageTextXBounds[_currPageIndex].y+"),
            1,
        ),
    ],
    line_width: LiteralKey::new(MOVIE, code!("redrawPages#$1.graphics.lineStyle(0)"), 1),
    line_colour: LiteralKey::new(MOVIE, code!("redrawPages#$1.graphics.lineStyle(1)"), 1),
    line_alpha: LiteralKey::new(MOVIE, code!("redrawPages#$1.graphics.lineStyle(2)"), 1),
    tab_header: PlacementKey::new(MOVIE, "Menu_mc/Header_mc/TabHeader_mc"),
    tab_timeline: TimelineKey::new(MOVIE, "Menu_mc/Header_mc/TabHeader_mc"),
    slide: PlacementKey::new(MOVIE, "Menu_mc/Header_mc/TabHeader_mc/AlphaHolder"),
    slide_labels: LiteralKey::texts(MOVIE, code!("redrawTabs#TabHeader_mc.gotoAndPlay(0)"), 2),
    first_script_frame: LiteralKey::new(
        MOVIE,
        "PipboyMenu_fla.TabHeader_14.<init>#addFrameScript(0)",
        1,
    ),
    stop_script_frame: LiteralKey::new(
        MOVIE,
        "PipboyMenu_fla.TabHeader_14.<init>#addFrameScript(2)",
        1,
    ),
    stop_target: LiteralKey::new(
        MOVIE,
        "PipboyMenu_fla.TabHeader_14.frame5#gotoAndStop(0)",
        1,
    ),
    strip_divisor: LiteralKey::new(
        MOVIE,
        code!("redrawTabs#(pageTextXBounds[_currPageIndex].x+pageTextXBounds[_currPageIndex].y)/"),
        1,
    ),
    slots: [
        TextKey::new(MOVIE, alpha_holder!("LeftTwo/textField_tf")),
        TextKey::new(MOVIE, alpha_holder!("LeftOne/textField_tf")),
        TextKey::new(MOVIE, alpha_holder!("Selected/textField_tf")),
        TextKey::new(MOVIE, alpha_holder!("RightOne/textField_tf")),
        TextKey::new(MOVIE, alpha_holder!("RightTwo/textField_tf")),
    ],
    slot_clips: [
        PlacementKey::new(MOVIE, alpha_holder!("LeftTwo")),
        PlacementKey::new(MOVIE, alpha_holder!("LeftOne")),
        PlacementKey::new(MOVIE, alpha_holder!("Selected")),
        PlacementKey::new(MOVIE, alpha_holder!("RightOne")),
        PlacementKey::new(MOVIE, alpha_holder!("RightTwo")),
    ],
    selected_slot: NumberKey::new(MOVIE, code!("SELECTED_INDEX")),
    tab_spacing: NumberKey::new(MOVIE, code!("TAB_SPACING")),
    half_width: LiteralKey::new(
        MOVIE,
        code!("redrawTabs#TabHeader_mc.AlphaHolder.Selected.width/"),
        2,
    ),
    set_text: SetTextKeys::new(MOVIE),
};

key_check_fields!(HeaderKeys {
    stage,
    titles,
    selectors,
    selector_shapes,
    borders,
    border_caps,
    selector_gaps,
    line_width,
    line_colour,
    line_alpha,
    tab_header,
    tab_timeline,
    slide,
    slide_labels,
    first_script_frame,
    stop_script_frame,
    stop_target,
    strip_divisor,
    slots,
    slot_clips,
    selected_slot,
    tab_spacing,
    half_width,
    set_text,
});

fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    HeaderLayout::resolve(layout).map(drop)
}

/// Every layout key the header reads, resolved at boot.
pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[&KEYS],
    resolve: validate,
};

/// A page title field: its spec, its translated text and its authored x in the page header.
struct Title {
    field: FieldSpec,
    text: String,
    local_x: f32,
}

/// The sub-tab strip's slide: the strip's offset on each frame of its timeline, where each
/// direction's animation starts and where it returns to rest.
#[derive(Debug, Clone, PartialEq)]
struct SlideTimeline {
    /// The offset on frames 1..=n.
    offsets: Vec<f32>,
    /// The label frame played for a move to a higher and to a lower sub-tab.
    starts: [u32; 2],
    /// The frame whose script jumps back to `rest`.
    stop_frame: u32,
    rest: u32,
    fps: f32,
}

impl SlideTimeline {
    /// The strip's offset `seconds` after an animation began at frame `start`: the timeline
    /// plays a frame per tick until a frame script stops it or it loops to its first frame.
    fn offset(&self, start: u32, seconds: f64) -> f32 {
        let ticks = (seconds.max(0.0) * f64::from(self.fps)) as u32;
        let mut frame = start;
        for _ in 0..ticks {
            frame += 1;
            if frame == self.stop_frame {
                frame = self.rest;
                break;
            }
            if frame as usize > self.offsets.len() {
                frame = 1;
                break;
            }
        }
        self.offsets[frame as usize - 1]
    }
}

fn slide_timeline(layout: &UiLayout, fps: f32) -> Result<SlideTimeline, UiLayoutError> {
    let timeline = layout.get(&KEYS.tab_timeline)?;
    let slide = layout.get(&KEYS.slide)?;
    let track = slide.track();
    let offsets: Vec<f32> = (1..=timeline.frame_count)
        .map(|frame| {
            track
                .iter()
                .take_while(|key| key.frame <= frame)
                .last()
                .map_or(slide.local.matrix.tx, |key| key.placement.matrix.tx)
        })
        .collect();
    let label = |index: usize| -> Result<u32, UiLayoutError> {
        let name = layout.lit_string(&KEYS.slide_labels, index)?;
        timeline.labels.get(&name).copied().ok_or_else(|| {
            UiLayoutError::unresolved(MOVIE, KEYS.slide_labels.path, format!("no label '{name}'"))
        })
    };
    let first_script_frame = layout.lit_whole(&KEYS.first_script_frame, 0)? + 1;
    if first_script_frame != 1 {
        return Err(UiLayoutError::unresolved(
            MOVIE,
            KEYS.first_script_frame.path,
            format!(
                "the first frame script is on frame {first_script_frame}, so frame 1 does not stop a slide that wraps to it"
            ),
        ));
    }
    let stop_frame = layout.lit_whole(&KEYS.stop_script_frame, 0)? + 1;
    if !KEYS
        .stop_target
        .path
        .contains(&format!(".frame{stop_frame}#"))
    {
        return Err(UiLayoutError::unresolved(
            MOVIE,
            KEYS.stop_script_frame.path,
            format!("the script is on frame {stop_frame}"),
        ));
    }
    let rest = layout.lit_whole(&KEYS.stop_target, 0)?;
    if offsets.is_empty() || rest == 0 || rest as usize > offsets.len() {
        return Err(UiLayoutError::unresolved(
            MOVIE,
            KEYS.stop_target.path,
            "the strip's timeline is too short",
        ));
    }
    Ok(SlideTimeline {
        offsets,
        starts: [label(0)?, label(1)?],
        stop_frame,
        rest,
        fps,
    })
}

/// The header resolved from the layout.
pub struct HeaderLayout {
    titles: Vec<Title>,
    /// The page header's and the header's own origin on the stage.
    page_header_x: f32,
    header_x: f32,
    selectors: [(Bracket, f32); 2],
    selector_gaps: [f32; 2],
    borders: [Pos2; 2],
    border_caps: [OutlinedShape; 2],
    line_width: f32,
    line_shade: Shade,
    /// The strip's authored x in the header, the slots' fields and authored x in the strip.
    strip_x: f32,
    strip_divisor: f32,
    slots: [(FieldSpec, f32); SLOTS],
    selected_slot: usize,
    tab_spacing: f32,
    half_width: f32,
    slide: SlideTimeline,
    rules: SetTextRules,
}

impl HeaderLayout {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let rules = KEYS.set_text.resolve(layout)?;
        let titles = KEYS
            .titles
            .iter()
            .map(|key| {
                let field = layout.get(key)?;
                let text = field.initial_text.ok_or_else(|| {
                    UiLayoutError::unresolved(MOVIE, key.path, "the title has no text")
                })?;
                Ok(Title {
                    field: FieldSpec::resolve(layout, key, TextAutoSize::Shrink)?,
                    text: rules.apply(layout, &text, false, false),
                    local_x: field.placement.local.matrix.tx,
                })
            })
            .collect::<Result<Vec<_>, UiLayoutError>>()?;
        let stat = layout.get(&KEYS.titles[0])?.placement;
        let tab_header = layout.get(&KEYS.tab_header)?;
        let selectors = [0, 1].map(|side| -> Result<(Bracket, f32), UiLayoutError> {
            let clip = layout.get(&KEYS.selectors[side])?;
            let mirrored = clip.world.matrix.a < 0.0;
            let bracket = Bracket::resolve(layout, &KEYS.selector_shapes[side], mirrored)?;
            Ok((bracket, clip.world.matrix.ty))
        });
        let [left, right] = selectors;
        let borders = [0, 1].map(|side| {
            layout
                .get(&KEYS.borders[side])
                .map(|clip| Pos2::new(clip.world.matrix.tx, clip.world.matrix.ty))
        });
        let [left_border, right_border] = borders;
        let slots = [0, 1, 2, 3, 4].map(|slot| -> Result<(FieldSpec, f32), UiLayoutError> {
            Ok((
                FieldSpec::resolve(layout, &KEYS.slots[slot], TextAutoSize::None)?,
                layout.get(&KEYS.slot_clips[slot])?.local.matrix.tx,
            ))
        });
        let [s0, s1, s2, s3, s4] = slots;
        let selected_slot = layout.whole_constant(&KEYS.selected_slot)? as usize;
        if selected_slot >= SLOTS {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                KEYS.selected_slot.path,
                "not one of the five slots",
            ));
        }
        let mut line_shade = code_colour(layout, &KEYS.line_colour, 0)?;
        line_shade.alpha = layout.lit_number(&KEYS.line_alpha, 0)? as f32;
        let stage = layout.get(&KEYS.stage)?;
        Ok(Self {
            titles,
            page_header_x: stat.world.matrix.tx - stat.local.matrix.tx,
            header_x: tab_header.world.matrix.tx - tab_header.local.matrix.tx,
            selectors: [left?, right?],
            selector_gaps: [
                layout.lit_number(&KEYS.selector_gaps[0], 0)? as f32,
                layout.lit_number(&KEYS.selector_gaps[1], 0)? as f32,
            ],
            borders: [left_border?, right_border?],
            border_caps: [
                OutlinedShape::resolve(layout, &KEYS.border_caps[0])?,
                OutlinedShape::resolve(layout, &KEYS.border_caps[1])?,
            ],
            line_width: layout.lit_number(&KEYS.line_width, 0)? as f32,
            line_shade,
            strip_x: tab_header.local.matrix.tx,
            strip_divisor: layout.lit_number(&KEYS.strip_divisor, 0)? as f32,
            slots: [s0?, s1?, s2?, s3?, s4?],
            selected_slot,
            tab_spacing: layout.get(&KEYS.tab_spacing)? as f32,
            half_width: layout.lit_number(&KEYS.half_width, 0)? as f32,
            slide: slide_timeline(layout, stage.fps)?,
            rules,
        })
    }
}

/// What a click on the header selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChromeHit {
    Page(Page),
    SubTab(usize),
}

#[derive(Clone, Copy, Debug)]
struct Slide {
    start: u32,
    started: f64,
}

/// The header with its slide state and where its titles and sub-tabs were drawn.
pub struct Chrome {
    layout: HeaderLayout,
    title_hits: Vec<(Page, Rect)>,
    sub_hits: Vec<(usize, Rect)>,
    slide: Option<Slide>,
}

/// `PageTextBounds` of one title (`CalculatePageTextBounds`): the code keeps the field's x plus the
/// first line's x offset in an `int`, so each is truncated to whole pixels (the sum after the
/// field's x was), then the first glyph's and the last glyph's advance end within the line, which
/// stay fractional, are added.
fn text_bounds(local_x: f32, field_x: f32, painted: &PaintedField) -> (f32, f32) {
    let (run_left, run_right) = painted.run;
    let whole = (local_x.trunc() + (painted.line_x - field_x)).trunc();
    (
        whole + (run_left - painted.line_x),
        whole + (run_right - painted.line_x),
    )
}

/// The x of each sub-tab clip from the selected one outward: each neighbour sits half of both
/// widths plus the spacing further out.
fn slot_positions(
    widths: &[f32; SLOTS],
    selected: usize,
    selected_x: f32,
    half: f32,
    spacing: f32,
) -> [f32; SLOTS] {
    let mut xs = [selected_x; SLOTS];
    for slot in (0..selected).rev() {
        xs[slot] = xs[slot + 1] - widths[slot + 1] / half - widths[slot] / half - spacing;
    }
    for slot in selected + 1..SLOTS {
        xs[slot] = xs[slot - 1] + widths[slot - 1] / half + widths[slot] / half + spacing;
    }
    xs
}

impl Chrome {
    pub fn new(layout: HeaderLayout) -> Self {
        Self {
            layout,
            title_hits: Vec::new(),
            sub_hits: Vec::new(),
            slide: None,
        }
    }

    /// Forgets the drawn positions and any running slide, as when the Pip-Boy opens.
    pub fn reset(&mut self) {
        self.title_hits.clear();
        self.sub_hits.clear();
        self.slide = None;
    }

    /// Starts the strip's slide for a move to the next (higher) or previous sub-tab.
    pub fn start_slide(&mut self, step: Step, seconds: f64) {
        let label = match step {
            Step::Next => 0,
            Step::Previous => 1,
        };
        self.slide = Some(Slide {
            start: self.layout.slide.starts[label],
            started: seconds,
        });
    }

    pub fn hit_test(&self, pos: Pos2) -> Option<ChromeHit> {
        let page = self
            .title_hits
            .iter()
            .find_map(|(page, rect)| rect.contains(pos).then_some(ChromeHit::Page(*page)));
        page.or_else(|| {
            self.sub_hits
                .iter()
                .find_map(|(index, rect)| rect.contains(pos).then_some(ChromeHit::SubTab(*index)))
        })
    }

    /// Draws the titles, the rule with its brackets and the sub-tab strip.
    pub fn draw(&mut self, cx: &TabContext<'_>, nav: &Nav) {
        let Some(bounds) = self.draw_titles(cx.painter, cx.tint, nav.page()) else {
            return;
        };
        self.draw_rule(cx.painter, cx.tint, bounds);
        self.draw_strip(cx, nav, bounds);
    }

    fn draw_titles(
        &mut self,
        painter: &Painter,
        tint: egui::Color32,
        selected: Page,
    ) -> Option<(f32, f32)> {
        self.title_hits.clear();
        let mut bounds = None;
        for (page, title) in Page::ALL.into_iter().zip(&self.layout.titles) {
            let painted = title.field.paint(painter, &title.text, tint);
            let geometry = painted.geometry;
            self.title_hits.push((
                page,
                Rect::from_min_max(
                    Pos2::new(geometry.left, geometry.top),
                    Pos2::new(geometry.right, geometry.bottom),
                ),
            ));
            if page == selected {
                let field_x = self.layout.page_header_x + title.local_x;
                bounds = Some(text_bounds(title.local_x, field_x, &painted));
            }
        }
        bounds
    }

    fn draw_rule(&self, painter: &Painter, tint: egui::Color32, (left, right): (f32, f32)) {
        let layout = &self.layout;
        let origin = layout.page_header_x;
        let [gap_left, gap_right] = layout.selector_gaps;
        let selector_x = [origin + left - gap_left, origin + right + gap_right];
        let colour = layout.line_shade.colour(tint);
        let half = layout.line_width / 2.0;
        let [left_border, right_border] = layout.borders;
        for (from, to, y) in [
            (left_border.x, selector_x[0], left_border.y),
            (selector_x[1], right_border.x, right_border.y),
        ] {
            super::paint::fill(
                painter,
                Rect::from_min_max(Pos2::new(from, y - half), Pos2::new(to, y + half)),
                colour,
            );
        }
        for cap in &layout.border_caps {
            cap.paint(painter, tint);
        }
        for ((bracket, y), x) in layout.selectors.iter().zip(selector_x) {
            bracket.paint(painter, tint, Pos2::new(x, *y));
        }
    }

    fn slide_offset(&self, seconds: f64) -> f32 {
        let timeline = &self.layout.slide;
        self.slide
            .map_or(timeline.offsets[timeline.rest as usize - 1], |slide| {
                timeline.offset(slide.start, seconds - slide.started)
            })
    }

    fn draw_strip(&mut self, cx: &TabContext<'_>, nav: &Nav, (left, right): (f32, f32)) {
        self.sub_hits.clear();
        let names: Vec<&str> = nav
            .page()
            .sub_tabs()
            .iter()
            .map(|sub| sub.label())
            .collect();
        if names.is_empty() {
            return;
        }
        let layout = &self.layout;
        let current = nav.sub_index();
        let selected = layout.selected_slot;
        let texts: [String; SLOTS] = std::array::from_fn(|slot| {
            (current + slot)
                .checked_sub(selected)
                .and_then(|index| names.get(index))
                .map_or_else(String::new, |name| {
                    layout.rules.apply(cx.layout, name, false, false)
                })
        });
        let style = |field: &FieldSpec| {
            field
                .style(field.shade.colour(cx.tint))
                .auto_sized(AutoSize::Center)
        };
        let widths: [f32; SLOTS] = std::array::from_fn(|slot| {
            let field = &layout.slots[slot].0;
            natural_width(cx.painter, &texts[slot], &field.geometry, style(field))
                + 2.0 * FIELD_GUTTER_PX
        });
        let authored: [f32; SLOTS] = std::array::from_fn(|slot| layout.slots[slot].1);
        let xs = slot_positions(
            &widths,
            selected,
            authored[selected],
            layout.half_width,
            layout.tab_spacing,
        );
        let strip_x = layout.header_x + (left + right) / layout.strip_divisor;
        let shift = strip_x - (layout.header_x + layout.strip_x);
        let offset = self.slide_offset(cx.seconds_open);
        let layout = &self.layout;
        for slot in 0..SLOTS {
            if texts[slot].is_empty() {
                continue;
            }
            let field = layout.slots[slot]
                .0
                .translated(Vec2::new(shift + offset + xs[slot] - authored[slot], 0.0));
            let painted = paint_field(cx.painter, &field.geometry, &texts[slot], style(&field));
            let geometry = painted.geometry;
            self.sub_hits.push((
                current + slot - selected,
                Rect::from_min_max(
                    Pos2::new(geometry.left, geometry.top),
                    Pos2::new(geometry.right, geometry.bottom),
                ),
            ));
        }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact literals")]
mod tests {
    use super::*;

    fn timeline() -> SlideTimeline {
        SlideTimeline {
            offsets: vec![0.0, -9.0, -6.0, -3.0, 0.0, 8.0, 6.0, 4.0, 2.0, 0.0],
            starts: [6, 2],
            stop_frame: 5,
            rest: 1,
            fps: 10.0,
        }
    }

    fn painted_line(line_x: f32, run: (f32, f32)) -> PaintedField {
        use crate::ui_layout::test_support::{FIXTURE_METRICS, text_field};
        PaintedField {
            geometry: crate::ui_layout::scaleform_text::FieldGeometry::of(
                &text_field([0.0, 0.0, 100.0, 24.0], 20.0),
                FIXTURE_METRICS,
            ),
            size: 20.0,
            baseline: 0.0,
            rect: Rect::NOTHING,
            line_x,
            run,
        }
    }

    #[test]
    fn the_bounds_keep_the_glyph_fractions_and_truncate_only_the_integer_part() {
        let painted = painted_line(117.6, (117.85, 207.6));
        let (start, end) = text_bounds(105.6, 115.6, &painted);
        assert_close_f32(start, 105.0 + 2.0 + 0.25);
        assert_close_f32(end, 105.0 + 2.0 + 90.0);
    }

    #[test]
    fn a_fractional_line_offset_is_truncated_with_the_field_x() {
        let painted = painted_line(115.6 + 7.4, (123.0, 190.0));
        let (start, end) = text_bounds(105.6, 115.6, &painted);
        assert_close_f32(start, 105.0 + 7.0);
        assert_close_f32(end, 112.0 + 67.0);
    }

    fn assert_close_f32(actual: f32, expected: f32) {
        crate::ui_layout::test_support::assert_close(actual, expected);
    }

    #[test]
    fn a_slide_steps_frame_by_frame_until_its_script_stops_it() {
        let slide = timeline();
        let lower: Vec<f32> = (0..5)
            .map(|tick| slide.offset(2, f64::from(tick) / 10.0))
            .collect();
        assert_eq!(lower, [-9.0, -6.0, -3.0, 0.0, 0.0]);
        assert_eq!(slide.offset(2, 0.15), -6.0);
    }

    #[test]
    fn a_slide_that_runs_off_the_timeline_loops_to_its_first_frame() {
        let slide = timeline();
        let higher: Vec<f32> = (0..7)
            .map(|tick| slide.offset(6, f64::from(tick) / 10.0))
            .collect();
        assert_eq!(higher, [8.0, 6.0, 4.0, 2.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn neighbours_sit_half_of_both_widths_plus_the_spacing_apart() {
        let xs = slot_positions(&[10.0, 20.0, 40.0, 30.0, 6.0], 2, 5.0, 2.0, 3.0);
        assert_eq!(xs, [-46.0, -28.0, 5.0, 43.0, 64.0]);
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the converted interface"]
    fn the_installed_header_slides_and_frames_the_rule_as_the_game_does() {
        let layout = HeaderLayout::resolve(&crate::ui_layout::test_support::installed_layout())
            .expect("the installed header resolves");
        let slide = &layout.slide;
        let higher: Vec<f32> = (0..6)
            .map(|tick| slide.offset(slide.starts[0], f64::from(tick) / f64::from(slide.fps)))
            .collect();
        let lower: Vec<f32> = (0..4)
            .map(|tick| slide.offset(slide.starts[1], f64::from(tick) / f64::from(slide.fps)))
            .collect();
        crate::ui_layout::test_support::assert_all_close(
            &higher,
            &[25.0, 18.75, 12.5, 6.25, 0.0, 0.0],
        );
        crate::ui_layout::test_support::assert_all_close(&lower, &[-25.0, -16.65, -8.3, 0.0]);
        let [left, right] = layout.borders;
        crate::ui_layout::test_support::assert_all_close(
            &[left.x, right.x, left.y],
            &[38.15, 832.8, 63.15],
        );
        let (bracket, y) = &layout.selectors[0];
        let bounds = bracket.bounds();
        crate::ui_layout::test_support::assert_all_close(
            &[y + bounds.min.y, y + bounds.max.y],
            &[35.15, 65.2],
        );
    }
}
