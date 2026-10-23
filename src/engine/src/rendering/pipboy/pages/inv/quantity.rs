//! The quantity menu: how many of a stack to drop, set with a slider.

use egui::{Pos2, Rect, Vec2};

use super::modal::{ArtPlacement, FrameKeys, ModalFrame, StrokedShape, frame_keys};
use crate::assets::pipboy_art::FixedArt;
use crate::rendering::pipboy::scaleform_text::{FieldSpec, TextAutoSize};
use crate::rendering::pipboy::shapes::{OutlinedShape, edge_rect, rect_of};
use crate::rendering::pipboy::tab::{HintKey, NavInput, TabContext};
use crate::sim::pipboy::ItemId;
use crate::ui_layout::set_text::SetTextRules;
use crate::ui_layout::{
    LiteralKey, PlacementKey, ShapeKey, TextKey, UiLayout, UiLayoutError, key_check_fields,
};

const MOVIE: &str = "Pipboy_InvPage";

macro_rules! slider_path {
    ($tail:literal) => {
        concat!("[Shared.QuantityMenuNEW]/Slider_mc/", $tail)
    };
}

pub struct QuantityKeys {
    frame: FrameKeys,
    count: TextKey,
    outline: ShapeKey,
    fill: ShapeKey,
    fill_clip: PlacementKey,
    marker_clip: PlacementKey,
    marker: ShapeKey,
    left_arrow: ShapeKey,
    right_arrow: ShapeKey,
    left_arrow_clip: PlacementKey,
    right_arrow_clip: PlacementKey,
    track: ShapeKey,
    track_clip: PlacementKey,
    left_inset: LiteralKey,
    bumper_divisor: LiteralKey,
    smallest_jump: LiteralKey,
    smallest_value: LiteralKey,
}

key_check_fields!(QuantityKeys {
    frame,
    count,
    outline,
    fill,
    fill_clip,
    marker_clip,
    marker,
    left_arrow,
    right_arrow,
    left_arrow_clip,
    right_arrow_clip,
    track,
    track_clip,
    left_inset,
    bumper_divisor,
    smallest_jump,
    smallest_value,
});

pub static QUANTITY_KEYS: QuantityKeys = QuantityKeys {
    frame: frame_keys!(
        "Pipboy_InvPage",
        "Shared.QuantityMenuNEW",
        "Shared.QuantityMenuNEW.redrawUIComponent"
    ),
    count: TextKey::new(MOVIE, "[Shared.QuantityMenuNEW]/Count_tf"),
    outline: ShapeKey::new(MOVIE, slider_path!("@3")),
    fill: ShapeKey::new(MOVIE, slider_path!("Fill_mc/@1")),
    fill_clip: PlacementKey::new(MOVIE, slider_path!("Fill_mc")),
    marker_clip: PlacementKey::new(MOVIE, slider_path!("Marker_mc")),
    marker: ShapeKey::new(MOVIE, slider_path!("Marker_mc/@3")),
    left_arrow: ShapeKey::new(MOVIE, slider_path!("LeftArrow_mc/@2")),
    right_arrow: ShapeKey::new(MOVIE, slider_path!("RightArrow_mc/@2")),
    left_arrow_clip: PlacementKey::new(MOVIE, slider_path!("LeftArrow_mc")),
    right_arrow_clip: PlacementKey::new(MOVIE, slider_path!("RightArrow_mc")),
    track: ShapeKey::new(MOVIE, slider_path!("SliderBackground_mc/@1")),
    track_clip: PlacementKey::new(MOVIE, slider_path!("SliderBackground_mc")),
    left_inset: LiteralKey::new(MOVIE, "Shared.AS3.BSSlider.<init>#Fill_mc.x+", 1),
    bumper_divisor: LiteralKey::new(MOVIE, "Shared.QuantityMenuNEW.<init>#$1/", 2),
    smallest_jump: LiteralKey::new(MOVIE, "Shared.QuantityMenuNEW.<init>#Math.max(1)", 2),
    smallest_value: LiteralKey::new(
        MOVIE,
        "Shared.QuantityMenuNEW.<init>#Slider_mc.minValue=",
        1,
    ),
};

/// A slider arrow: where it takes clicks and where its art sits.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SliderArrow {
    hit: Rect,
    art: ArtPlacement,
}

impl SliderArrow {
    fn resolve(
        layout: &UiLayout,
        shape: &ShapeKey,
        clip: &PlacementKey,
    ) -> Result<Self, UiLayoutError> {
        Ok(Self {
            hit: edge_rect(layout, shape)?,
            art: ArtPlacement::resolve(layout, clip)?,
        })
    }
}

/// The slider: its parts at the maximum value and how the value moves the marker and the fill.
#[derive(Debug, Clone)]
pub struct QuantityLayout {
    frame: ModalFrame,
    count: FieldSpec,
    outline: StrokedShape,
    fill: OutlinedShape,
    marker: StrokedShape,
    left_arrow: SliderArrow,
    right_arrow: SliderArrow,
    track: Rect,
    track_origin_x: f32,
    /// The fill's left edge and the gap its end keeps from it.
    fill_inset: f32,
    /// The distance the marker travels from the value `0` to its resting place at the maximum.
    fill_length: f32,
    bumper_divisor: f32,
    smallest_jump: f32,
    smallest_value: u32,
}

impl QuantityLayout {
    pub fn resolve(layout: &UiLayout, rules: &SetTextRules) -> Result<Self, UiLayoutError> {
        let keys = &QUANTITY_KEYS;
        let fill_x = layout.get(&keys.fill_clip)?.local.matrix.tx;
        let marker_x = layout.get(&keys.marker_clip)?.local.matrix.tx;
        let fill_inset = layout.lit_number(&keys.left_inset, 0)? as f32;
        let track = layout.get(&keys.track)?;
        let smallest_value = layout.lit_number(&keys.smallest_value, 0)?;
        Ok(Self {
            frame: ModalFrame::resolve(layout, &keys.frame, rules)?,
            count: FieldSpec::resolve(layout, &keys.count, TextAutoSize::Shrink)?,
            outline: StrokedShape::resolve(layout, &keys.outline)?,
            fill: OutlinedShape::resolve(layout, &keys.fill)?,
            marker: StrokedShape::resolve(layout, &keys.marker)?,
            left_arrow: SliderArrow::resolve(layout, &keys.left_arrow, &keys.left_arrow_clip)?,
            right_arrow: SliderArrow::resolve(layout, &keys.right_arrow, &keys.right_arrow_clip)?,
            track: rect_of(track.placement.world.matrix.bounds(track.bounds)),
            track_origin_x: layout.get(&keys.track_clip)?.world.matrix.tx,
            fill_inset,
            fill_length: marker_x - (fill_x + fill_inset),
            bumper_divisor: layout.lit_number(&keys.bumper_divisor, 0)? as f32,
            smallest_jump: layout.lit_number(&keys.smallest_jump, 0)? as f32,
            smallest_value: smallest_value as u32,
        })
    }

    /// The marker's distance from where it rests at the maximum.
    fn marker_shift(&self, value: u32, max: u32) -> f32 {
        self.fill_length * (value as f32 / max as f32 - 1.0)
    }

    /// The smallest step a click on an arrow moves the value by.
    fn arrow_jump(&self, max: u32) -> i64 {
        (max as f32 / self.bumper_divisor)
            .max(self.smallest_jump)
            .floor() as i64
    }

    pub fn draw(&self, cx: &mut TabContext<'_>, quantity: &Quantity) {
        let (painter, tint) = (cx.painter, cx.tint);
        self.frame.draw(painter, tint);
        self.count.paint(painter, &quantity.value.to_string(), tint);
        self.outline.paint_closed(painter, tint);
        self.fill
            .with_width(
                self.fill_inset + self.fill_length * (quantity.value as f32 / quantity.max as f32),
            )
            .paint(painter, tint);
        self.marker
            .translated(Vec2::new(
                self.marker_shift(quantity.value, quantity.max),
                0.0,
            ))
            .paint_solid(painter, tint);
        let white = cx.shade(1.0);
        for arrow in [&self.left_arrow, &self.right_arrow] {
            arrow.art.draw(cx, FixedArt::SliderArrow, Vec2::ZERO, white);
        }
    }

    /// What the input does to `quantity`.
    pub fn handle(&self, quantity: &mut Quantity, input: NavInput) -> QuantityOutcome {
        match input {
            NavInput::PrevSubTab => quantity.jump(-1),
            NavInput::NextSubTab => quantity.jump(1),
            NavInput::Scrolled(delta) if delta < 0.0 => quantity.jump(-1),
            NavInput::Scrolled(delta) if delta > 0.0 => quantity.jump(1),
            NavInput::Clicked(at) => self.click(quantity, at),
            NavInput::Hint(HintKey::Enter | HintKey::E) => {
                return QuantityOutcome::Drop {
                    item: quantity.item,
                    count: quantity.value,
                };
            }
            NavInput::Hint(HintKey::Tab) => return QuantityOutcome::Cancel,
            _ => {}
        }
        QuantityOutcome::Open
    }

    fn click(&self, quantity: &mut Quantity, at: Pos2) {
        let jump = self.arrow_jump(quantity.max);
        if self.left_arrow.hit.contains(at) {
            quantity.jump(-jump);
        } else if self.right_arrow.hit.contains(at) {
            quantity.jump(jump);
        } else if self.track.contains(at) {
            let along = (at.x - self.track_origin_x) / self.fill_length;
            let span = quantity.max.saturating_sub(self.smallest_value);
            quantity.set((along * span as f32).max(0.0) as u32);
        }
    }

    pub fn new_quantity(&self, item: ItemId, max: u32) -> Quantity {
        Quantity::new(item, max, self.smallest_value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantityOutcome {
    Open,
    Drop { item: ItemId, count: u32 },
    Cancel,
}

/// How many of a stack the menu would drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quantity {
    pub item: ItemId,
    max: u32,
    min: u32,
    value: u32,
}

impl Quantity {
    /// The menu opens at the whole stack.
    fn new(item: ItemId, max: u32, min: u32) -> Self {
        let max = max.max(1);
        Self {
            item,
            max,
            min: min.min(max),
            value: max,
        }
    }

    pub fn value(&self) -> u32 {
        self.value
    }

    fn set(&mut self, value: u32) {
        self.value = value.clamp(self.min, self.max);
    }

    /// Moves the value by `delta`; a step down past zero lands on one.
    fn jump(&mut self, delta: i64) {
        if delta < 0 && delta.unsigned_abs() > u64::from(self.value) {
            self.value = 1.clamp(self.min, self.max);
        } else {
            let moved =
                (i64::from(self.value) + delta).clamp(i64::from(self.min), i64::from(self.max));
            self.value = moved as u32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quantity(max: u32) -> Quantity {
        Quantity::new(ItemId(1), max, 1)
    }

    #[test]
    fn the_menu_opens_at_the_whole_stack() {
        assert_eq!(quantity(9).value(), 9);
    }

    #[test]
    fn jumps_stay_between_one_and_the_stack_size() {
        let mut q = quantity(9);
        q.jump(1);
        assert_eq!(q.value(), 9);
        q.jump(-4);
        assert_eq!(q.value(), 5);
        q.jump(-100);
        assert_eq!(q.value(), 1);
        q.jump(100);
        assert_eq!(q.value(), 9);
    }

    #[test]
    fn a_stack_of_nothing_still_offers_one() {
        assert_eq!(quantity(0).value(), 1);
    }
}
