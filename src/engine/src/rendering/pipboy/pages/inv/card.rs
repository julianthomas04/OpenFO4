//! The item card: entries stacked upward from the card's anchor, each with the height its own
//! clip has, in the order the game's menu places them.

use egui::{Color32, Painter, Pos2, Vec2};

use super::card_layout::{CardLayout, Row};
use super::card_model::{CardEntry, Chip, Comparison, MultiKind, StatLine, value_text};
use crate::assets::pipboy_art::Icon;
use crate::rendering::pipboy::scaleform_text::{PaintedField, text_height};
use crate::rendering::pipboy::summary::{damage_dealt_icon, damage_icon};
use crate::rendering::pipboy::tab::TabContext;
use crate::ui_layout::UiLayout;
use crate::ui_layout::scaleform_text::AutoSize;
use crate::ui_layout::set_text::SetTextRules;

/// Where each entry sits: its index in the entries, its `y` below the anchor and its height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub index: usize,
    pub y: f32,
    pub height: f32,
}

/// The entries in the order the card stacks them, bottom first: the plain ones from the last,
/// then the damage entry, then the resistance entry, then the first description.
pub fn stacking_order(entries: &[CardEntry]) -> Vec<usize> {
    let plain = entries
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, entry)| !matches!(entry, CardEntry::Multi { .. } | CardEntry::Description(_)))
        .map(|(index, _)| index);
    let multi = |kind: MultiKind| {
        entries.iter().position(
            |entry| matches!(entry, CardEntry::Multi { kind: found, .. } if *found == kind),
        )
    };
    let description = entries
        .iter()
        .position(|entry| matches!(entry, CardEntry::Description(_)));
    plain
        .chain(multi(MultiKind::Damage))
        .chain(multi(MultiKind::Resist))
        .chain(description)
        .collect()
}

/// Stacks `heights` (bottom first) upward: each entry's `y` is the running offset minus its
/// height, and the offset then moves by the height plus the spacing.
pub fn stack(heights: &[f32], spacing: f32) -> Vec<f32> {
    let mut current = 0.0;
    heights
        .iter()
        .map(|height| {
            let y = current - height;
            current -= height + spacing;
            y
        })
        .collect()
}

struct Context<'a> {
    card: &'a CardLayout,
    rules: &'a SetTextRules,
    layout: &'a UiLayout,
}

impl Context<'_> {
    fn text(&self, text: &str) -> String {
        self.rules.apply(self.layout, text, false, false)
    }

    fn description_height(&self, painter: &Painter, text: &str) -> (f32, f32) {
        let field = self.card.description.label.growing(AutoSize::Left);
        let shown = self.text(text);
        let text_height = text_height(
            painter,
            &shown,
            &field.geometry,
            field.style(Color32::WHITE),
        );
        let background_height = text_height + self.card.description.padding;
        let (bg_top, bg_bottom) = self.card.description.background.bounds(background_height);
        let geometry = field.geometry.auto_height(text_height);
        let top = bg_top.min(geometry.top);
        let bottom = bg_bottom.max(geometry.bottom);
        (bottom - top, background_height)
    }

    fn height(&self, painter: &Painter, entry: &CardEntry) -> f32 {
        let card = self.card;
        match entry {
            CardEntry::Standard(_) => card.standard.row.height(),
            CardEntry::Timed(_) => card.timed.row.height(),
            CardEntry::Ammo { .. } => card.ammo.row.height(),
            CardEntry::Multi { chips, .. } => {
                let ((top, bottom), _) = card.multi.bounds(chips.len(), card.multi_spacing);
                bottom - top
            }
            CardEntry::Components(lines) => {
                let count = lines.len() as f32;
                (count - 1.0) * (card.components.pitch + card.components.spacing)
                    + card.components.pitch
            }
            CardEntry::Description(text) => self.description_height(painter, text).0,
        }
    }
}

/// The placement of every entry that shows, bottom first.
pub fn plan(
    painter: &Painter,
    layout: &UiLayout,
    card: &CardLayout,
    rules: &SetTextRules,
    entries: &[CardEntry],
) -> Vec<Placed> {
    let context = Context {
        card,
        rules,
        layout,
    };
    let order = stacking_order(entries);
    let heights: Vec<f32> = order
        .iter()
        .map(|index| context.height(painter, &entries[*index]))
        .collect();
    stack(&heights, card.spacing)
        .into_iter()
        .zip(order)
        .zip(heights)
        .map(|((y, index), height)| Placed { index, y, height })
        .collect()
}

fn draw_comparison(
    cx: &mut TabContext<'_>,
    card: &CardLayout,
    at: Pos2,
    comparison: Option<Comparison>,
) {
    let Some(comparison) = comparison else {
        return;
    };
    let white = cx.shade(1.0);
    for offset in card.comparison.glyph_offsets(comparison.rating) {
        cx.draw_icon(
            super::card_layout::Comparison::icon(comparison.rating),
            at + Vec2::new(*offset, 0.0),
            1.0,
            white,
        );
    }
}

fn paint_row_text(
    cx: &TabContext<'_>,
    row: &Row,
    origin: Vec2,
    label: &str,
    value: &str,
) -> PaintedField {
    row.background.translated(origin).paint(cx.painter, cx.tint);
    row.label
        .translated(origin)
        .paint(cx.painter, label, cx.tint);
    row.value
        .translated(origin)
        .paint(cx.painter, value, cx.tint)
}

fn stat_value(line: &StatLine) -> String {
    value_text(line.value, line.precision, line.percent)
}

fn draw_chip(
    cx: &mut TabContext<'_>,
    card: &CardLayout,
    kind: MultiKind,
    chip: &Chip,
    origin: Vec2,
) {
    let layout = &card.multi.chip;
    layout
        .background
        .translated(origin)
        .paint(cx.painter, cx.tint);
    layout.value.translated(origin).paint(
        cx.painter,
        &value_text(chip.value, super::native::CHIP_PRECISION, false),
        cx.tint,
    );
    let icon = match kind {
        MultiKind::Damage => damage_dealt_icon(chip.kind),
        MultiKind::Resist => damage_icon(chip.kind),
    };
    let white = cx.shade(1.0);
    cx.draw_icon(icon, origin.to_pos2() + layout.icon, 1.0, white);
    draw_comparison(
        cx,
        card,
        origin.to_pos2() + layout.comparison,
        chip.comparison,
    );
}

/// Draws the card; `placed` comes from [`plan`] for the same `entries`.
pub fn draw(
    cx: &mut TabContext<'_>,
    card: &CardLayout,
    rules: &SetTextRules,
    entries: &[CardEntry],
    placed: &[Placed],
) {
    let context = Context {
        card,
        rules,
        layout: cx.layout,
    };
    for place in placed {
        let origin = card.anchor + Vec2::new(0.0, place.y);
        match &entries[place.index] {
            CardEntry::Standard(line) => {
                paint_row_text(
                    cx,
                    &card.standard.row,
                    origin,
                    &context.text(&line.label),
                    &stat_value(line),
                );
                draw_comparison(
                    cx,
                    card,
                    origin.to_pos2() + card.standard.comparison,
                    line.comparison,
                );
            }
            CardEntry::Timed(line) => {
                let timed = &card.timed;
                let painted = paint_row_text(
                    cx,
                    &timed.row,
                    origin,
                    &context.text(&line.label),
                    &stat_value(line),
                );
                let line_left = painted.run.0 - painted.geometry.left;
                let icon_x = timed.icon.x(timed.row.value_x + line_left, timed.gap);
                let white = cx.shade(1.0);
                cx.draw_icon(
                    Icon::Timer,
                    origin.to_pos2() + Vec2::new(icon_x, timed.icon.y()),
                    1.0,
                    white,
                );
                draw_comparison(
                    cx,
                    card,
                    origin.to_pos2() + timed.comparison,
                    line.comparison,
                );
            }
            CardEntry::Ammo { name, carried } => {
                paint_row_text(
                    cx,
                    &card.ammo.row,
                    origin,
                    &context.text(name),
                    &carried.to_string(),
                );
                let white = cx.shade(1.0);
                cx.draw_icon(Icon::Ammo, origin.to_pos2() + card.ammo.glyph, 1.0, white);
            }
            CardEntry::Multi { kind, chips } => {
                let multi = &card.multi;
                let (_, background_height) = multi.bounds(chips.len(), card.multi_spacing);
                crate::rendering::pipboy::paint::fill(
                    cx.painter,
                    multi.background.rect(background_height).translate(origin),
                    multi.background.shade.colour(cx.tint),
                );
                let label_key = match kind {
                    MultiKind::Damage => &card.damage_label,
                    MultiKind::Resist => &card.resist_label,
                };
                multi
                    .label
                    .translated(origin)
                    .paint(cx.painter, &context.text(label_key), cx.tint);
                for (index, chip) in chips.iter().enumerate() {
                    let at = origin
                        + multi.holder
                        + Vec2::new(0.0, multi.chip_y(index, card.multi_spacing));
                    draw_chip(cx, card, *kind, chip, at);
                }
            }
            CardEntry::Components(lines) => {
                let components = &card.components;
                for (index, line) in lines.iter().enumerate() {
                    let at = origin
                        + components.holder
                        + Vec2::new(0.0, index as f32 * (components.pitch + components.spacing));
                    let text = components.suffix.apply(&line.name, line.count);
                    let painted = components
                        .text
                        .translated(at)
                        .paint(cx.painter, &text, cx.tint);
                    if line.tagged {
                        let width = painted.run.1 - painted.run.0;
                        let x = components.text_x
                            + components.text_width / 2.0
                            + width / 2.0
                            + components.lead;
                        let white = cx.shade(1.0);
                        cx.draw_icon(
                            Icon::SearchTag,
                            at.to_pos2() + Vec2::new(x, components.icon_y),
                            1.0,
                            white,
                        );
                    }
                }
            }
            CardEntry::Description(text) => {
                let description = &card.description;
                let (_, background_height) = context.description_height(cx.painter, text);
                crate::rendering::pipboy::paint::fill(
                    cx.painter,
                    description
                        .background
                        .rect(background_height)
                        .translate(origin),
                    description.background.shade.colour(cx.tint),
                );
                description
                    .label
                    .growing(AutoSize::Left)
                    .translated(origin)
                    .paint(cx.painter, &context.text(text), cx.tint);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::pipboy::pages::inv::card_model::StatLine;

    fn line() -> CardEntry {
        CardEntry::Standard(StatLine {
            label: "a".into(),
            value: 1.0,
            precision: 0,
            percent: false,
            comparison: None,
        })
    }

    fn multi(kind: MultiKind) -> CardEntry {
        CardEntry::Multi {
            kind,
            chips: Vec::new(),
        }
    }

    #[test]
    fn plain_entries_stack_from_the_last_then_damage_resistance_and_description() {
        let entries = vec![
            CardEntry::Description("d".into()),
            multi(MultiKind::Resist),
            line(),
            multi(MultiKind::Damage),
            line(),
        ];
        assert_eq!(stacking_order(&entries), vec![4, 2, 3, 1, 0]);
    }

    #[test]
    fn each_entry_sits_its_own_height_above_the_one_below_less_the_spacing() {
        let ys = stack(&[40.0, 30.0], -3.5);
        assert!((ys[0] + 40.0).abs() < 1e-4);
        assert!((ys[1] - (-40.0 + 3.5 - 30.0)).abs() < 1e-4);
    }
}
