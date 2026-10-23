//! The chart's grid: the vault boy cards in their authored places, the rank stars on them and the
//! SPECIAL headers with their star counters.

use egui::{Color32, Pos2, Rect, Vec2};

use super::layout::{CardLayout, CellLayout, ChartLayout, HeaderLayout, Text};
use super::stars::{draw_rank_star, draw_special_star};
use super::state::{ChartState, Selection};
use crate::assets::pipboy_art::ArtClip;
use crate::assets::pipboy_playback::PlaybackStart;
use crate::rendering::pipboy::pages::perk_chart_layout::{
    COLUMNS, Cell, GridView, grid_stars, special_star_count,
};
use crate::rendering::pipboy::paint::draw_sprite;
use crate::rendering::pipboy::scaleform_text::paint_field;
use crate::rendering::pipboy::tab::TabContext;
use crate::sim::pipboy::{PipBoyModel, Special};
use crate::ui_layout::scaleform_text::FieldGeometry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardLook {
    /// Held: the animation plays.
    Owned,
    /// Not held but takeable: the second frame under the colour filter.
    Available,
    /// Neither: the first frame, the ghost outline.
    Unavailable,
}

pub fn card_look(owned: u8, available: bool) -> CardLook {
    if owned > 0 {
        CardLook::Owned
    } else if available {
        CardLook::Available
    } else {
        CardLook::Unavailable
    }
}

/// How the card's clip starts playing: a held perk plays on from the menu's frame, a takeable
/// one holds the menu's frame, the rest rest on the clip's own first frame.
pub fn card_start(look: CardLook, frames: &CardLayout) -> Option<PlaybackStart> {
    match look {
        CardLook::Owned => Some(PlaybackStart::PlayFrom(frames.owned_from)),
        CardLook::Available => Some(PlaybackStart::HoldAt(frames.available_at)),
        CardLook::Unavailable => None,
    }
}

/// The 0-based frame of a card `seconds` after the chart opened.
pub fn card_frame(look: CardLook, clip: &ArtClip, frames: &CardLayout, seconds: f64) -> usize {
    clip.frame_at(seconds, card_start(look, frames))
}

fn stage_rect(view: &GridView, rect: Rect) -> Rect {
    Rect::from_min_max(view.to_stage(rect.min), view.to_stage(rect.max))
}

fn on_stage(view: &GridView, cell: &CellLayout, stage: Rect) -> bool {
    stage_rect(view, cell.border).intersects(stage)
}

/// The look of the SPECIAL row's cards: the menu takes it from the native entry data, which is
/// not available, so the cards play as held perks do.
const SPECIAL_CARD_LOOK: CardLook = CardLook::Owned;

/// The clip of the sprite the grid holds below its cards.
pub const STRAY_CLIP: &str = "perkchart/perks_holder_depth_1";

fn draw_special_card(
    cx: &mut TabContext<'_>,
    layout: &ChartLayout,
    view: &GridView,
    cell: Cell,
    cell_layout: &CellLayout,
) {
    let Some(clip) = layout
        .cards
        .special_classes
        .get(cell.column)
        .and_then(|class| cx.art_library.perk_card_for_class(class))
    else {
        crate::utils::warn_once(&format!("pipboy-special-card:{}", cell.column), || {
            format!("no chart card art for SPECIAL column {}", cell.column)
        });
        return;
    };
    let frame = card_frame(SPECIAL_CARD_LOOK, clip, &layout.cards, cx.seconds_open);
    let at = view.to_stage(cell_layout.origin);
    cx.draw_clip(
        clip,
        frame,
        at,
        cell_layout.scale.x * view.scale,
        Color32::WHITE,
    );
}

fn draw_stray(cx: &mut TabContext<'_>, layout: &ChartLayout, view: &GridView) {
    let Some(clip) = cx.art_library.clip(STRAY_CLIP) else {
        crate::utils::warn_once("pipboy-perk-chart-stray", || {
            format!("the art manifest has no clip '{STRAY_CLIP}'")
        });
        return;
    };
    let stray = layout.cards.stray;
    cx.draw_clip(
        clip,
        0,
        view.to_stage(stray.origin),
        stray.scale.x * view.scale,
        Color32::WHITE,
    );
}

fn draw_perk_card(
    cx: &mut TabContext<'_>,
    layout: &ChartLayout,
    view: &GridView,
    cell: &CellLayout,
    selection: &Selection<'_>,
) {
    let Some(clip) = selection
        .perk
        .vault_boy_swf
        .as_deref()
        .and_then(|path| cx.art_library.perk_card_for_swf(path))
    else {
        crate::utils::warn_once(&format!("pipboy-perk-card:{}", selection.perk.id.0), || {
            format!("no perk chart card art for perk '{}'", selection.perk.name)
        });
        return;
    };
    let look = card_look(selection.owned, selection.available);
    let frame = card_frame(look, clip, &layout.cards, cx.seconds_open);
    let at = view.to_stage(cell.origin);
    let scale = cell.scale.x * view.scale;
    if look == CardLook::Available {
        let sprite = cx.art.clip_scaled_filtered(
            cx.painter.ctx(),
            cx.art_library,
            clip,
            frame,
            scale,
            layout.unavailable_filter,
        );
        if let Some(sprite) = sprite {
            draw_sprite(cx.painter, sprite, at, Color32::WHITE);
        }
    } else {
        cx.draw_clip(clip, frame, at, scale, Color32::WHITE);
    }
}

fn draw_grid_stars(
    cx: &mut TabContext<'_>,
    layout: &ChartLayout,
    view: &GridView,
    cell: Cell,
    selection: &Selection<'_>,
) {
    let Some(holder) = layout.star_holder(cell) else {
        return;
    };
    let available_slot = selection.available && selection.unspent_points > 0;
    let stars = grid_stars(selection.max_rank, selection.owned, available_slot);
    let size = layout.grid_stars.size * layout.grid_stars.scale * holder.scale * view.scale;
    for (index, star) in stars.into_iter().enumerate() {
        let grid =
            holder.origin + Vec2::new(layout.grid_stars.pitch * index as f32 * holder.scale.x, 0.0);
        draw_rank_star(cx, layout, view.to_stage(grid), size, star);
    }
}

/// Every card of the chart that shows on the stage, with its rank stars.
pub fn draw_cards(
    cx: &mut TabContext<'_>,
    layout: &ChartLayout,
    state: &ChartState,
    model: &PipBoyModel,
    stage: Rect,
) {
    let view = state.view;
    draw_stray(cx, layout, &view);
    for cell in Cell::ALL {
        let cell_layout = layout.cell(cell);
        if !on_stage(&view, cell_layout, stage) {
            continue;
        }
        if cell.is_special_row() {
            draw_special_card(cx, layout, &view, cell, cell_layout);
        } else if let Some(selection) = Selection::of(model, cell) {
            draw_perk_card(cx, layout, &view, cell_layout, &selection);
            draw_grid_stars(cx, layout, &view, cell, &selection);
        }
    }
}

/// `geometry`, which is in grid units, on the stage.
pub fn stage_geometry(geometry: FieldGeometry, view: &GridView) -> FieldGeometry {
    let scale = view.scale;
    FieldGeometry {
        left: view.x + geometry.left * scale,
        right: view.x + geometry.right * scale,
        top: view.y + geometry.top * scale,
        bottom: view.y + geometry.bottom * scale,
        left_inset: geometry.left_inset * scale,
        right_inset: geometry.right_inset * scale,
        vertical_inset: geometry.vertical_inset * scale,
        size: geometry.size * scale,
        leading: geometry.leading * scale,
        letter_spacing: geometry.letter_spacing * scale,
        indent: geometry.indent * scale,
        ..geometry
    }
}

fn draw_header_text(cx: &TabContext<'_>, view: &GridView, field: &Text, label: &str) {
    let geometry = stage_geometry(field.geometry(), view);
    paint_field(
        cx.painter,
        &geometry,
        label,
        field.style(cx.tint).shrinking(),
    );
}

fn draw_special_counter(
    cx: &mut TabContext<'_>,
    layout: &ChartLayout,
    view: &GridView,
    header: &HeaderLayout,
    value: u8,
) {
    let stars = &layout.special_stars;
    let count = usize::from(special_star_count(value, stars.cap));
    if count == 0 {
        return;
    }
    let width = stars.pitch * (count - 1) as f32 + stars.size.x;
    let left = header.stars_origin.x - width / stars.halves;
    for index in 0..count {
        let grid = Pos2::new(left + stars.pitch * index as f32, header.stars_origin.y);
        draw_special_star(cx, view.to_stage(grid), stars.size * view.scale);
    }
}

/// The SPECIAL names above the columns and the stars that count each attribute.
pub fn draw_headers(
    cx: &mut TabContext<'_>,
    layout: &ChartLayout,
    state: &ChartState,
    model: &PipBoyModel,
) {
    let view = state.view;
    for column in 0..COLUMNS {
        let header = &layout.headers[column];
        draw_header_text(cx, &view, &header.shadow, &header.label);
        draw_header_text(cx, &view, &header.text, &header.label);
        let value = model.character.special.get(Special::ALL[column]).base;
        draw_special_counter(cx, layout, &view, header, value);
    }
}

#[cfg(test)]
mod tests {
    use super::super::layout::cards::Anchor;
    use super::*;
    use crate::assets::pipboy_art::ArtColour;
    use crate::assets::pipboy_playback::{ScriptAction, ScriptStep};

    fn clip(frames: usize, script: Vec<ScriptStep>) -> ArtClip {
        ArtClip {
            id: "perkcard/0".to_owned(),
            source: "LevelUpMenu.swf".to_owned(),
            illustrates: None,
            colour: ArtColour::Full,
            fps: 30.0,
            script,
            bounds: [0.0, 0.0, 10.0, 10.0],
            frames: (0..frames).map(|n| format!("{n}.svg")).collect(),
            offset: None,
            tint: None,
        }
    }

    fn stop_then_loop(last: u32) -> Vec<ScriptStep> {
        vec![
            ScriptStep {
                frame: 1,
                action: ScriptAction::Stop,
                target: None,
            },
            ScriptStep {
                frame: last,
                action: ScriptAction::GotoAndPlay,
                target: Some(2),
            },
        ]
    }

    fn frames() -> CardLayout {
        CardLayout {
            owned_from: 2,
            available_at: 2,
            special_classes: Vec::new(),
            stray: Anchor {
                origin: Pos2::ZERO,
                scale: Vec2::splat(1.0),
            },
        }
    }

    #[test]
    fn a_held_perk_plays_a_takeable_one_waits_on_its_second_frame_and_the_rest_show_the_ghost() {
        assert_eq!(card_look(1, false), CardLook::Owned);
        assert_eq!(card_look(0, true), CardLook::Available);
        assert_eq!(card_look(0, false), CardLook::Unavailable);
        assert_eq!(card_look(2, true), CardLook::Owned);
    }

    #[test]
    fn the_start_follows_the_look_and_the_menus_frames() {
        let frames = CardLayout {
            owned_from: 3,
            available_at: 4,
            ..frames()
        };
        assert_eq!(
            card_start(CardLook::Owned, &frames),
            Some(PlaybackStart::PlayFrom(3))
        );
        assert_eq!(
            card_start(CardLook::Available, &frames),
            Some(PlaybackStart::HoldAt(4))
        );
        assert_eq!(card_start(CardLook::Unavailable, &frames), None);
    }

    #[test]
    fn the_frames_follow_the_look() {
        let clip = clip(10, stop_then_loop(10));
        let frames = frames();
        assert_eq!(card_frame(CardLook::Unavailable, &clip, &frames, 5.0), 0);
        assert_eq!(card_frame(CardLook::Available, &clip, &frames, 5.0), 1);
        assert_eq!(card_frame(CardLook::Owned, &clip, &frames, 0.0), 1);
    }

    #[test]
    fn an_owned_clip_returns_to_the_frame_its_script_names_after_the_last_frame() {
        let clip = clip(10, stop_then_loop(10));
        let frames = frames();
        assert_eq!(card_frame(CardLook::Owned, &clip, &frames, 7.5 / 30.0), 8);
        assert_eq!(card_frame(CardLook::Owned, &clip, &frames, 8.5 / 30.0), 1);
    }

    #[test]
    fn a_single_frame_card_has_only_its_first_frame() {
        let clip = clip(1, Vec::new());
        let frames = frames();
        assert_eq!(card_frame(CardLook::Available, &clip, &frames, 1.0), 0);
        assert_eq!(card_frame(CardLook::Owned, &clip, &frames, 1.0), 0);
    }

    #[test]
    fn header_geometry_scales_with_the_grid() {
        let view = GridView {
            scale: 0.5,
            x: 10.0,
            y: 20.0,
        };
        let grid = FieldGeometry {
            left: 100.0,
            right: 300.0,
            top: 40.0,
            bottom: 80.0,
            left_inset: 2.0,
            right_inset: 2.0,
            vertical_inset: 2.0,
            size: 26.0,
            leading: 0.0,
            letter_spacing: 0.0,
            indent: 0.0,
            align: crate::ui_layout::values::Align::Center,
            wraps: false,
            metrics: crate::ui_layout::test_support::FIXTURE_METRICS,
        };
        let stage = stage_geometry(grid, &view);
        assert_eq!((stage.left, stage.right, stage.top), (60.0, 160.0, 40.0));
        assert_eq!((stage.size, stage.left_inset), (13.0, 1.0));
    }
}
