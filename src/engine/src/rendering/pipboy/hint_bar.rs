//! The button hint bar (`ButtonBarMenu`): drawn in screen space, centred at the bottom of the
//! screen's safe rectangle, over whatever menu is open. Each hint is its key as plain text
//! ("E)") followed by its upper-cased label; hints are spaced by a fixed gap, the row is centred
//! on the bar and bracketed. A flashing hint animates only its key, a disabled one is greyed out.
//!
//! Every menu that shows hints (the Pip-Boy shell, the perk chart, later the HUD) resolves one
//! [`HintBarLayout`]. A menu whose bar sits at the bottom of the screen paints it with
//! [`HintBarLayout::paint`] on a painter set up by [`ScreenFit`]; a menu that places the bar in its
//! own movie and shades it (the perk chart) paints it with [`HintBarLayout::paint_in_movie`].

use std::ops::Range;

use egui::emath::TSTransform;
use egui::{Color32, Painter, Pos2, Rect, Vec2};

use super::paint::{faded, fill};
use super::scaleform_text::{FieldSpec, FieldStyle, TextAutoSize, natural_width, paint_field};
use super::shapes::{Bracket, Shade, alpha_at_frame, rect_of};
use super::tab::ButtonHint;
use crate::rendering::game_settings::SafeZone;
use crate::ui_layout::scaleform_text::AutoSize;
use crate::ui_layout::set_text::{SetTextKeys, SetTextRules};
use crate::ui_layout::values::{ColorTransform, TrackKey};
use crate::ui_layout::{
    LiteralKey, ModuleKeys, NumberKey, PlacementKey, ShapeKey, StageKey, SymbolTimelineKey,
    TextKey, UiLayout, UiLayoutError, key_check_fields,
};

const MOVIE: &str = "ButtonBarMenu";

struct HintBarKeys {
    stage: StageKey,
    bar: PlacementKey,
    left_bracket: ShapeKey,
    right_bracket: ShapeKey,
    key_field: TextKey,
    key_y: LiteralKey,
    key_font: LiteralKey,
    key_suffix: LiteralKey,
    label: TextKey,
    icon_holder: PlacementKey,
    icon_holder_x: LiteralKey,
    flash: PlacementKey,
    holder_symbol: SymbolTimelineKey,
    flash_loop: LiteralKey,
    gap: LiteralKey,
    centre_divisor: LiteralKey,
    disabled_alpha: NumberKey,
    set_text: SetTextKeys,
}

static KEYS: HintBarKeys = HintBarKeys {
    stage: StageKey::of(MOVIE),
    bar: PlacementKey::new(
        MOVIE,
        "[ButtonBarMenu_fla.ButtonBarHolder_1]/ButtonHintBar_mc",
    ),
    left_bracket: ShapeKey::new(
        MOVIE,
        "[Shared.AS3.BSButtonHintBar]/ButtonBracket_Left_mc/@1",
    ),
    right_bracket: ShapeKey::new(
        MOVIE,
        "[Shared.AS3.BSButtonHintBar]/ButtonBracket_Right_mc/@1",
    ),
    key_field: TextKey::new(
        MOVIE,
        "[Shared.AS3.BSButtonHint]/IconHolderInstance/IconAnimInstance/Icon_tf",
    ),
    key_y: LiteralKey::new(MOVIE, "Shared.AS3.BSButtonHint.UpdateIconTextField#$5=", 2),
    key_font: LiteralKey::texts(MOVIE, "Shared.AS3.BSButtonHint.GetExpectedFont#$1=", 3),
    key_suffix: LiteralKey::texts(
        MOVIE,
        "Shared.AS3.BSButtonHint.get:PCKey#_buttonHintData.PCKey+",
        1,
    ),
    label: TextKey::new(MOVIE, "[Shared.AS3.BSButtonHint]/textField_tf"),
    icon_holder: PlacementKey::new(MOVIE, "[Shared.AS3.BSButtonHint]/IconHolderInstance"),
    icon_holder_x: LiteralKey::new(
        MOVIE,
        "Shared.AS3.BSButtonHint.redrawPrimaryButton#IconHolderInstance.x=",
        1,
    ),
    flash: PlacementKey::new(
        MOVIE,
        "[Shared.AS3.BSButtonHint]/IconHolderInstance:Flashing/IconAnimInstance",
    ),
    holder_symbol: SymbolTimelineKey::new(MOVIE, "ButtonBarMenu_fla.BSButtonHint_IconHolder_8"),
    flash_loop: LiteralKey::texts(
        MOVIE,
        "ButtonBarMenu_fla.BSButtonHint_IconHolder_8.frame60#gotoAndPlay(0)",
        1,
    ),
    gap: LiteralKey::new(
        MOVIE,
        "Shared.AS3.BSButtonHintBar.redrawUIComponent#$6.width+",
        1,
    ),
    centre_divisor: LiteralKey::new(
        MOVIE,
        "Shared.AS3.BSButtonHintBar.redrawUIComponent#-(ButtonHintBarInternal_mc.width)/",
        1,
    ),
    disabled_alpha: NumberKey::new(MOVIE, "Shared.AS3.BSButtonHint.DISABLED_GREY_OUT_ALPHA"),
    set_text: SetTextKeys::new(MOVIE),
};

key_check_fields!(HintBarKeys {
    stage,
    bar,
    left_bracket,
    right_bracket,
    key_field,
    key_y,
    key_font,
    key_suffix,
    label,
    icon_holder,
    icon_holder_x,
    flash,
    holder_symbol,
    flash_loop,
    gap,
    centre_divisor,
    disabled_alpha,
    set_text,
});

fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    HintBarLayout::resolve(layout).map(drop)
}

/// Every layout key the hint bar reads, resolved at boot.
pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[&KEYS],
    resolve: validate,
};

/// How the hint bar's 1280 x 720 movie maps onto the screen: scaled uniformly to fit, centred,
/// with the visible rectangle being the whole screen in movie units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenFit {
    /// From movie units to the painter's points.
    pub transform: TSTransform,
    /// The screen in movie units.
    pub visible: Rect,
}

impl ScreenFit {
    /// The fit of a movie with stage `stage` onto a screen `screen_points` large.
    pub fn new(stage: Rect, screen_points: Vec2) -> Self {
        let scale = (screen_points.x / stage.width()).min(screen_points.y / stage.height());
        let visible_size = screen_points / scale;
        let visible = Rect::from_center_size(stage.center(), visible_size);
        let transform = TSTransform::new(-visible.min.to_vec2() * scale, scale);
        Self { transform, visible }
    }
}

/// How a menu that places the bar inside its own movie and shades it wants it drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoviePlacement {
    /// The bar clip's registration point in the movie.
    pub origin: Pos2,
    /// The shaded background behind the brackets and the hints.
    pub backdrop: Shade,
}

/// The hint bar resolved from the layout.
pub struct HintBarLayout {
    stage: Rect,
    bar_offset: Vec2,
    left_bracket: Bracket,
    right_bracket: Bracket,
    key_field: FieldSpec,
    key_suffix: String,
    /// The key holder's x as the code sets it, where the key field's box starts inside the
    /// holder, and the label field's authored x, which the code replaces by the holder's width.
    icon_holder_x: f32,
    key_x_in_holder: f32,
    label_x: f32,
    label: FieldSpec,
    /// The top and bottom of a hint's fields relative to the bar's registration point.
    row_extent: [f32; 2],
    /// The key's opacity on each frame of the flashing loop, from the loop's first frame.
    flash_alphas: Vec<f32>,
    fps: f32,
    gap: f32,
    centre_divisor: f32,
    disabled_alpha: f32,
    rules: SetTextRules,
}

/// The frames of the holder's timeline the flashing animation loops over: from the frame the
/// loop's label names up to, but not including, the frame whose script jumps back to it (that
/// frame is left as soon as it is entered).
fn flash_loop_span(layout: &UiLayout) -> Result<Range<u32>, UiLayoutError> {
    let holder = layout.get(&KEYS.holder_symbol)?;
    let label = layout.lit_string(&KEYS.flash_loop, 0)?;
    let start = *holder.labels.get(&label).ok_or_else(|| {
        UiLayoutError::unresolved(
            MOVIE,
            KEYS.flash_loop.path,
            format!("no frame label '{label}'"),
        )
    })?;
    let script_frame = holder.frame_count;
    if !KEYS
        .flash_loop
        .path
        .contains(&format!(".frame{script_frame}#"))
    {
        return Err(UiLayoutError::unresolved(
            MOVIE,
            KEYS.flash_loop.path,
            format!("the jump back is not on the last frame, {script_frame}"),
        ));
    }
    if start >= script_frame {
        return Err(UiLayoutError::unresolved(
            MOVIE,
            KEYS.flash_loop.path,
            format!("the loop starts at frame {start}, after the jump back"),
        ));
    }
    Ok(start..script_frame)
}

/// The opacity of the animated clip on each frame of `span`.
fn loop_alphas(track: &[TrackKey], base: &ColorTransform, span: Range<u32>) -> Vec<f32> {
    span.map(|frame| alpha_at_frame(track, base, frame))
        .collect()
}

fn flash_alphas(layout: &UiLayout) -> Result<Vec<f32>, UiLayoutError> {
    let flash = layout.get(&KEYS.flash)?;
    if flash.track().is_empty() {
        return Err(UiLayoutError::unresolved(
            MOVIE,
            KEYS.flash.path,
            "the flashing key has no animation",
        ));
    }
    Ok(loop_alphas(
        flash.track(),
        &flash.local.color,
        flash_loop_span(layout)?,
    ))
}

impl HintBarLayout {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let stage = layout.get(&KEYS.stage)?;
        let bar = layout.get(&KEYS.bar)?.local.matrix;
        let key_placement = layout.get(&KEYS.key_field)?.placement;
        let key_clip = key_placement.local.matrix;
        let holder = layout.get(&KEYS.icon_holder)?;
        let key_y = layout.lit_number(&KEYS.key_y, 0)? as f32;
        let key_font = layout.lit_string(&KEYS.key_font, 0)?;
        let key_field =
            FieldSpec::resolve_in_font(layout, &KEYS.key_field, TextAutoSize::None, &key_font)?
                .translated(Vec2::new(0.0, key_y - key_clip.ty));
        let label = FieldSpec::resolve(layout, &KEYS.label, TextAutoSize::None)?;
        Ok(Self {
            stage: rect_of(stage.rect),
            bar_offset: Vec2::new(bar.tx, bar.ty),
            left_bracket: Bracket::resolve(layout, &KEYS.left_bracket, false)?,
            right_bracket: Bracket::resolve(layout, &KEYS.right_bracket, false)?,
            key_field,
            key_suffix: layout.lit_string(&KEYS.key_suffix, 0)?,
            icon_holder_x: layout.lit_number(&KEYS.icon_holder_x, 0)? as f32,
            key_x_in_holder: key_placement.world.matrix.tx - holder.world.matrix.tx,
            label_x: layout.get(&KEYS.label)?.placement.local.matrix.tx,
            row_extent: [
                key_field.geometry.top.min(label.geometry.top),
                key_field.geometry.bottom.max(label.geometry.bottom),
            ],
            label,
            flash_alphas: flash_alphas(layout)?,
            fps: stage.fps,
            gap: layout.lit_number(&KEYS.gap, 0)? as f32,
            centre_divisor: layout.lit_number(&KEYS.centre_divisor, 0)? as f32,
            disabled_alpha: layout.get(&KEYS.disabled_alpha)? as f32,
            rules: KEYS.set_text.resolve(layout)?,
        })
    }

    /// The fit of the bar's movie onto a screen `screen_points` large.
    pub fn fit(&self, screen_points: Vec2) -> ScreenFit {
        ScreenFit::new(self.stage, screen_points)
    }

    /// The key's opacity `seconds` into its flashing loop.
    pub fn flash_alpha(&self, seconds: f64) -> f32 {
        let frames = self.flash_alphas.len();
        let frame = (seconds.max(0.0) * f64::from(self.fps)) as usize % frames;
        self.flash_alphas[frame]
    }

    fn style(field: &FieldSpec, colour: Color32) -> FieldStyle {
        field.style(colour).auto_sized(AutoSize::Left)
    }

    /// The width of `field` after it sizes itself to `text`.
    fn sized_width(painter: &Painter, field: &FieldSpec, text: &str) -> f32 {
        let natural = natural_width(
            painter,
            text,
            &field.geometry,
            Self::style(field, Color32::WHITE),
        );
        let sized = field.geometry.auto_sized(AutoSize::Left, natural);
        sized.right - sized.left
    }

    fn measure(&self, painter: &Painter, layout: &UiLayout, hint: &ButtonHint) -> HintRow {
        let key = format!("{}{}", hint.key.label(), self.key_suffix);
        let label = self.rules.apply(layout, &hint.label, true, false);
        let key_box = Self::sized_width(painter, &self.key_field, &key);
        let label_box = Self::sized_width(painter, &self.label, &label);
        let label_shift = label_shift(
            self.icon_holder_x,
            self.key_x_in_holder + key_box,
            self.label_x,
        );
        let left = self
            .key_field
            .geometry
            .left
            .min(self.label.geometry.left + label_shift);
        let right = (self.key_field.geometry.left + key_box)
            .max(self.label.geometry.left + label_shift + label_box);
        HintRow {
            key,
            label,
            label_shift,
            left,
            right,
        }
    }

    /// Paints `hints` as the button bar menu shows them, centred at the bottom of the screen's
    /// safe rectangle, in a painter whose coordinates are the fit's movie units. `seconds`
    /// drives the flashing.
    #[allow(
        clippy::too_many_arguments,
        reason = "one call per frame from each menu"
    )]
    pub fn paint(
        &self,
        painter: &Painter,
        layout: &UiLayout,
        fit: &ScreenFit,
        safe_zone: SafeZone,
        tint: Color32,
        hints: &[ButtonHint],
        seconds: f64,
    ) {
        let holder = Pos2::new(fit.visible.center().x, fit.visible.max.y - safe_zone.y);
        self.paint_at(
            painter,
            layout,
            holder + self.bar_offset,
            None,
            tint,
            hints,
            seconds,
        );
    }

    /// Paints `hints` in the menu's own movie, on a painter whose coordinates are the movie's:
    /// the bar sits at `placement.origin` over a shaded background.
    pub fn paint_in_movie(
        &self,
        painter: &Painter,
        layout: &UiLayout,
        placement: &MoviePlacement,
        tint: Color32,
        hints: &[ButtonHint],
        seconds: f64,
    ) {
        self.paint_at(
            painter,
            layout,
            placement.origin,
            Some(placement.backdrop),
            tint,
            hints,
            seconds,
        );
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the shared body of the two ways to place the bar"
    )]
    fn paint_at(
        &self,
        painter: &Painter,
        layout: &UiLayout,
        bar: Pos2,
        backdrop: Option<Shade>,
        tint: Color32,
        hints: &[ButtonHint],
        seconds: f64,
    ) {
        let rows: Vec<HintRow> = hints
            .iter()
            .map(|hint| self.measure(painter, layout, hint))
            .collect();
        let positions = row_positions(&rows, self.gap);
        let (Some(first), Some(last)) = (rows.first(), rows.last()) else {
            return;
        };
        let last_x = positions[positions.len() - 1];
        let width = last_x + last.right - first.left;
        let internal_x = bar.x - width / self.centre_divisor;
        let left = internal_x + first.left;
        let right = internal_x + last_x + last.right;
        let left_origin = Pos2::new(left - self.left_bracket.bounds().width(), bar.y);
        let right_origin = Pos2::new(right, bar.y);
        if let Some(backdrop) = backdrop {
            let hints_box = Rect::from_min_max(
                Pos2::new(left, bar.y + self.row_extent[0]),
                Pos2::new(right, bar.y + self.row_extent[1]),
            );
            let shaded = hints_box
                .union(self.left_bracket.bounds().translate(left_origin.to_vec2()))
                .union(
                    self.right_bracket
                        .bounds()
                        .translate(right_origin.to_vec2()),
                );
            fill(painter, shaded, backdrop.colour(tint));
        }
        self.left_bracket.paint(painter, tint, left_origin);
        self.right_bracket.paint(painter, tint, right_origin);
        for ((hint, row), x) in hints.iter().zip(&rows).zip(positions) {
            self.paint_hint(
                painter,
                tint,
                hint,
                row,
                Vec2::new(internal_x + x, bar.y),
                seconds,
            );
        }
    }

    fn paint_hint(
        &self,
        painter: &Painter,
        tint: Color32,
        hint: &ButtonHint,
        row: &HintRow,
        origin: Vec2,
        seconds: f64,
    ) {
        let enabled_alpha = if hint.enabled {
            1.0
        } else {
            self.disabled_alpha
        };
        let key_alpha = if hint.flashing {
            enabled_alpha * self.flash_alpha(seconds)
        } else {
            enabled_alpha
        };
        let key_colour = faded(self.key_field.shade.colour(tint), key_alpha);
        paint_field(
            painter,
            &self.key_field.translated(origin).geometry,
            &row.key,
            Self::style(&self.key_field, key_colour),
        );
        let label_colour = faded(self.label.shade.colour(tint), enabled_alpha);
        let label_origin = origin + Vec2::new(row.label_shift, 0.0);
        paint_field(
            painter,
            &self.label.translated(label_origin).geometry,
            &row.label,
            Self::style(&self.label, label_colour),
        );
    }
}

/// How far the code moves a hint's label: it sets the label's x to the key holder's width, which
/// reaches from the holder's origin to the right edge of the key field, and the holder sits at
/// `holder_x`.
fn label_shift(holder_x: f32, holder_width: f32, authored_label_x: f32) -> f32 {
    holder_x + holder_width - authored_label_x
}

/// One hint measured: its texts, how far the code moves its label, and its bounds relative to
/// its registration point.
struct HintRow {
    key: String,
    label: String,
    label_shift: f32,
    left: f32,
    right: f32,
}

/// Each hint's x: the next starts its predecessor's width plus `gap` further.
fn row_positions(rows: &[HintRow], gap: f32) -> Vec<f32> {
    let mut x = 0.0;
    rows.iter()
        .map(|row| {
            let at = x;
            x += row.right - row.left + gap;
            at
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_layout::test_support::assert_close;

    fn row(left: f32, right: f32) -> HintRow {
        HintRow {
            key: String::new(),
            label: String::new(),
            label_shift: 0.0,
            left,
            right,
        }
    }

    #[test]
    fn a_wider_screen_shows_more_of_the_stage_around_its_centre() {
        let stage = Rect::from_min_size(Pos2::ZERO, Vec2::new(100.0, 50.0));
        let fit = ScreenFit::new(stage, Vec2::new(400.0, 100.0));
        assert_close(fit.transform.scaling, 2.0);
        assert_eq!(
            fit.visible,
            Rect::from_min_max(Pos2::new(-50.0, 0.0), Pos2::new(150.0, 50.0))
        );
        assert_eq!(fit.transform * fit.visible.min, Pos2::ZERO);
        assert_eq!(fit.transform * fit.visible.max, Pos2::new(400.0, 100.0));
    }

    #[test]
    fn the_label_follows_the_right_edge_of_the_key_field_inside_its_holder() {
        let key_x_in_holder = 2.0;
        let key_box = 30.0;
        assert_close(label_shift(0.0, key_x_in_holder + key_box, 2.0), 30.0);
        assert_close(label_shift(5.0, key_x_in_holder + key_box, 2.0), 35.0);
    }

    #[test]
    fn each_hint_starts_its_predecessors_width_plus_the_gap_further() {
        let rows = [row(-1.0, 9.0), row(0.0, 5.0), row(2.0, 4.0)];
        assert_eq!(row_positions(&rows, 3.0), [0.0, 13.0, 21.0]);
    }

    fn key_at(frame: u32, mult: f32, add: f32) -> TrackKey {
        let mut placement = crate::ui_layout::test_support::placement(0.0, 0.0).local;
        placement.color = ColorTransform {
            mult: [1.0, 1.0, 1.0, mult],
            add: [0.0, 0.0, 0.0, add],
        };
        TrackKey { frame, placement }
    }

    const OPAQUE: ColorTransform = ColorTransform::IDENTITY;

    #[test]
    fn the_flash_loop_covers_its_first_frame_up_to_the_jump_back_and_holds_keys_between_frames() {
        let track = [
            key_at(1, 1.0, 0.0),
            key_at(3, 0.5, 0.0),
            key_at(5, 0.25, 0.0),
        ];
        let alphas = loop_alphas(&track, &OPAQUE, 2..6);
        assert_eq!(alphas, [1.0, 0.5, 0.5, 0.25]);
    }

    #[test]
    fn the_flash_opacity_counts_the_added_alpha_and_starts_from_the_base() {
        let track = [key_at(4, 0.5, 51.0)];
        let base = ColorTransform {
            mult: [1.0, 1.0, 1.0, 0.75],
            add: [0.0; 4],
        };
        let alphas = loop_alphas(&track, &base, 3..5);
        assert_close(alphas[0], 0.75);
        assert_close(alphas[1], 0.7);
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the converted interface"]
    fn the_installed_flash_loops_between_its_label_and_the_jump_back() {
        let bar = HintBarLayout::resolve(&crate::ui_layout::test_support::installed_layout())
            .expect("the installed hint bar resolves");
        let alphas = &bar.flash_alphas;
        assert!(alphas.len() > 2);
        assert_close(alphas[0], 1.0);
        assert!(alphas.iter().all(|alpha| (0.0..=1.0).contains(alpha)));
        assert!(alphas.iter().any(|alpha| *alpha < 0.5));
    }
}
