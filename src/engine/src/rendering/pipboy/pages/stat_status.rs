//! STAT / STATUS (`StatusTab`): the condition boy with its limb meters, the player name, the
//! damage and resistance widgets centred as a pair, and the active-effects column with its
//! detail panel. This Rust implementation re-creates the observed layout behavior using
//! runtime-extracted constants from the installed game's movie; it does not copy its scripts.

use egui::{Pos2, Rect, Vec2};

use crate::assets::pipboy_art::{CONDITION_BODY_VARIANTS, FixedArt, Icon};
use crate::rendering::pipboy::scaleform_text::{FieldSpec, TextAutoSize};
use crate::rendering::pipboy::shapes::{Meter, MeterKeys, OutlinedShape, Shade};
use crate::rendering::pipboy::summary::{
    CURRENT_HP_GAIN, TypeTotal, equipped_damage, floored, numbered_type, type_icon_frame,
    type_number, worn_resistance,
};
use crate::rendering::pipboy::tab::{
    BottomBar, ButtonHint, HintKey, InputResult, NavInput, Tab, TabContext,
};
use crate::sim::pipboy::actions::{RADAWAY_FORM_KEY, STIMPAK_FORM_KEY};
use crate::sim::pipboy::{Limb, PipBoyAction, PipBoySnapshot};
use crate::ui_layout::set_text::{SetTextKeys, SetTextRules};
use crate::ui_layout::values::Matrix;
use crate::ui_layout::{
    BoundsKey, LiteralKey, ModuleKeys, NumberKey, PlacementKey, ShapeKey, TextKey, UiLayout,
    UiLayoutError, key_check_fields,
};

const MOVIE: &str = "Pipboy_StatsPage";

macro_rules! status {
    ($path:literal) => {
        concat!("StatusTab_mc/", $path)
    };
}

macro_rules! limb_meter {
    ($clip:literal) => {
        MeterKeys {
            clip: PlacementKey::new(MOVIE, status!($clip)),
            bounds: BoundsKey::new(MOVIE, status!($clip)),
            outline: ShapeKey::new(MOVIE, concat!(status!($clip), "/@1")),
            fill: PlacementKey::new(MOVIE, concat!(status!($clip), "/Fill_mc")),
            fill_shape: ShapeKey::new(MOVIE, concat!(status!($clip), "/Fill_mc/@2")),
            ghost: PlacementKey::new(MOVIE, concat!(status!($clip), "/GhostFill_mc")),
            ghost_shape: ShapeKey::new(MOVIE, concat!(status!($clip), "/GhostFill_mc/@1")),
        }
    };
}

macro_rules! change_event {
    ($path:literal) => {
        concat!("StatusTab.onPipboyChangeEvent#", $path)
    };
}

macro_rules! body_head {
    ($($variant:literal),+) => {
        [$(PlacementKey::new(
            concat!("Components/ConditionClips/Condition_Body_", $variant),
            "Head_mc",
        )),+]
    };
}

/// The limbs whose meters show their condition, with their meter's `SetMeter` maximum key.
const CONDITION_LIMBS: [(Limb, usize); 5] = [
    (Limb::Head, 0),
    (Limb::LeftArm, 1),
    (Limb::RightArm, 2),
    (Limb::LeftLeg, 3),
    (Limb::RightLeg, 4),
];

/// Stand-in: the `HeadFlags` the game sends for a crippled head (the bandaged, frowning face);
/// the flags come from native code.
const CRIPPLED_HEAD_FLAGS: usize = 16;

struct StatusKeys {
    name: TextKey,
    meters: [MeterKeys; 5],
    torso: MeterKeys,
    meter_ghosts: [LiteralKey; 5],
    meter_maxima: [LiteralKey; 5],
    gain_above: LiteralKey,
    boy: PlacementKey,
    body_scale: LiteralKey,
    head_frame_step: LiteralKey,
    body_heads: [PlacementKey; CONDITION_BODY_VARIANTS as usize],
    widgets: [PlacementKey; 2],
    widget_icon: BoundsKey,
    widget_icon_clip: PlacementKey,
    widget_panel: ShapeKey,
    widget_glyphs: [PlacementKey; 2],
    entry: BoundsKey,
    entry_panel: ShapeKey,
    entry_icon: PlacementKey,
    entry_value: TextKey,
    entry_spacing: NumberKey,
    damage_types: NumberKey,
    shown_above: LiteralKey,
    placeholder_type: LiteralKey,
    placeholder_value: LiteralKey,
    pair_gap: LiteralKey,
    pair_width: LiteralKey,
    pair_divisor: LiteralKey,
    count_open: [LiteralKey; 2],
    count_close: [LiteralKey; 2],
    count_above: [LiteralKey; 2],
    effects_labels: LiteralKey,
    effects_hide: ShapeKey,
    effects_mask: ShapeKey,
    effects_holder: PlacementKey,
    effects_gap: LiteralKey,
    effects_step: LiteralKey,
    widget: BoundsKey,
    icon_background_shape: ShapeKey,
    effects_background_shape: ShapeKey,
    entry_holder: PlacementKey,
    timer: PlacementKey,
    background_pad: LiteralKey,
    effect_entry: BoundsKey,
    effect_source: TextKey,
    effect_text: TextKey,
    source_suffix: LiteralKey,
    set_text: SetTextKeys,
}

static KEYS: StatusKeys = StatusKeys {
    name: TextKey::new(MOVIE, status!("Name_tf")),
    meters: [
        limb_meter!("Head_Meter"),
        limb_meter!("LArm_Meter"),
        limb_meter!("RArm_Meter"),
        limb_meter!("LLeg_Meter"),
        limb_meter!("RLeg_Meter"),
    ],
    torso: limb_meter!("Torso_Meter"),
    meter_ghosts: [
        LiteralKey::new(MOVIE, change_event!("Head_Meter.SetMeter(1)"), 1),
        LiteralKey::new(MOVIE, change_event!("LArm_Meter.SetMeter(1)"), 1),
        LiteralKey::new(MOVIE, change_event!("RArm_Meter.SetMeter(1)"), 1),
        LiteralKey::new(MOVIE, change_event!("LLeg_Meter.SetMeter(1)"), 1),
        LiteralKey::new(MOVIE, change_event!("RLeg_Meter.SetMeter(1)"), 1),
    ],
    meter_maxima: [
        LiteralKey::new(MOVIE, change_event!("Head_Meter.SetMeter(2)"), 1),
        LiteralKey::new(MOVIE, change_event!("LArm_Meter.SetMeter(2)"), 1),
        LiteralKey::new(MOVIE, change_event!("RArm_Meter.SetMeter(2)"), 1),
        LiteralKey::new(MOVIE, change_event!("LLeg_Meter.SetMeter(2)"), 1),
        LiteralKey::new(MOVIE, change_event!("RLeg_Meter.SetMeter(2)"), 1),
    ],
    gain_above: LiteralKey::new(MOVIE, change_event!("$1.DataObj.CurrentHPGain>"), 1),
    boy: PlacementKey::new(MOVIE, status!("ConditionBoy_mc")),
    body_scale: LiteralKey::new(
        MOVIE,
        "Shared.AS3.ConditionBoy.redrawUIComponent#BodyClip_mc.scaleX=",
        1,
    ),
    head_frame_step: LiteralKey::new(
        MOVIE,
        "Shared.AS3.ConditionBoy.redrawUIComponent#_HeadFlags+",
        1,
    ),
    body_heads: body_head!(
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16"
    ),
    widgets: [
        PlacementKey::new(MOVIE, status!("DMGWidget_mc")),
        PlacementKey::new(MOVIE, status!("DRWidget_mc")),
    ],
    widget_icon: BoundsKey::new(MOVIE, "[Stats_DMGDRWidget]/Icon_mc"),
    widget_icon_clip: PlacementKey::new(MOVIE, "[Stats_DMGDRWidget]/Icon_mc"),
    widget_panel: ShapeKey::new(MOVIE, "[Stats_DMGDRWidget]/Icon_mc/@1/@1"),
    widget_glyphs: [
        PlacementKey::new(MOVIE, "[Stats_DMGDRWidget]/Icon_mc:Weapon/@3"),
        PlacementKey::new(MOVIE, "[Stats_DMGDRWidget]/Icon_mc:Armor/@3"),
    ],
    entry: BoundsKey::new(MOVIE, "[Stats_DMGDRWidget_Entry]"),
    entry_panel: ShapeKey::new(MOVIE, "[Stats_DMGDRWidget_Entry]/@1/@1"),
    entry_icon: PlacementKey::new(MOVIE, "[Stats_DMGDRWidget_Entry]/Icon_mc"),
    entry_value: TextKey::new(MOVIE, "[Stats_DMGDRWidget_Entry]/Value_tf"),
    entry_spacing: NumberKey::new(MOVIE, "Stats_DMGDRWidget.ENTRY_SPACING"),
    damage_types: NumberKey::new(MOVIE, "Shared.GlobalFunc.NUM_DAMAGE_TYPES"),
    shown_above: LiteralKey::new(MOVIE, "Stats_DMGDRWidget.redraw#$2[$4].value>", 1),
    placeholder_type: LiteralKey::new(MOVIE, "Stats_DMGDRWidget.redraw#AddEntry(1).type", 1),
    placeholder_value: LiteralKey::new(MOVIE, "Stats_DMGDRWidget.redraw#AddEntry(1).value", 1),
    pair_gap: LiteralKey::new(MOVIE, "StatusTab.redrawUIComponent#$1=", 1),
    pair_width: LiteralKey::new(MOVIE, "StatusTab.redrawUIComponent#$2=", 1),
    pair_divisor: LiteralKey::new(
        MOVIE,
        "StatusTab.redrawUIComponent#($2-((DMGWidget_mc.width+DRWidget_mc.width)+$1))/",
        1,
    ),
    count_open: [
        LiteralKey::texts(
            MOVIE,
            "StatusTab.UpdateStimpakRadawayEnabled#+_DataObj.StimpakCount.toString()",
            1,
        ),
        LiteralKey::texts(
            MOVIE,
            "StatusTab.UpdateStimpakRadawayEnabled#+_DataObj.RadawayCount.toString()",
            1,
        ),
    ],
    count_close: [
        LiteralKey::texts(
            MOVIE,
            "StatusTab.UpdateStimpakRadawayEnabled#(\"$$Stimpak (\"+_DataObj.StimpakCount.toString())+",
            1,
        ),
        LiteralKey::texts(
            MOVIE,
            "StatusTab.UpdateStimpakRadawayEnabled#(\"$$Radaway (\"+_DataObj.RadawayCount.toString())+",
            1,
        ),
    ],
    count_above: [
        LiteralKey::new(
            MOVIE,
            "StatusTab.UpdateStimpakRadawayEnabled#_DataObj.StimpakCount>",
            1,
        ),
        LiteralKey::new(
            MOVIE,
            "StatusTab.UpdateStimpakRadawayEnabled#_DataObj.RadawayCount>",
            1,
        ),
    ],
    effects_labels: LiteralKey::texts(
        MOVIE,
        "StatusTab.ToggleShowingEffects#ActiveEffectsButton.ButtonText=",
        2,
    ),
    effects_hide: ShapeKey::new(MOVIE, status!("ActiveEffects_HideRect/@2")),
    effects_mask: ShapeKey::new(MOVIE, status!("ActiveEffects_MaskRect/@2")),
    effects_holder: PlacementKey::new(MOVIE, status!("ActiveEffects_mc")),
    effects_gap: LiteralKey::new(MOVIE, "StatusTab.redrawUIComponent#$5=", 1),
    effects_step: LiteralKey::new(MOVIE, "StatusTab.ScrollActiveEffects#$2=", 1),
    widget: BoundsKey::new(MOVIE, "[ActiveEffectsWidget]"),
    icon_background_shape: ShapeKey::new(MOVIE, "[ActiveEffectsWidget]/IconBackground_mc/@1"),
    effects_background_shape: ShapeKey::new(MOVIE, "[ActiveEffectsWidget]/EffectsBackground_mc/@1"),
    entry_holder: PlacementKey::new(MOVIE, "[ActiveEffectsWidget]/EntryHolder_mc"),
    timer: PlacementKey::new(MOVIE, "[ActiveEffectsWidget]/TimerIcon_mc"),
    background_pad: LiteralKey::new(MOVIE, "ActiveEffectsWidget.redrawUIComponent#$3=", 1),
    effect_entry: BoundsKey::new(MOVIE, "[ActiveEffects_Entry]"),
    effect_source: TextKey::new(MOVIE, "[ActiveEffects_Entry]/Source_tf"),
    effect_text: TextKey::new(MOVIE, "[ActiveEffects_Entry]/Effects_tf"),
    source_suffix: LiteralKey::texts(MOVIE, "ActiveEffects_Entry.set:sourceText#$1+", 1),
    set_text: SetTextKeys::new(MOVIE),
};

key_check_fields!(StatusKeys {
    name,
    meters,
    torso,
    meter_ghosts,
    meter_maxima,
    gain_above,
    boy,
    body_scale,
    head_frame_step,
    body_heads,
    widgets,
    widget_icon,
    widget_icon_clip,
    widget_panel,
    widget_glyphs,
    entry,
    entry_panel,
    entry_icon,
    entry_value,
    entry_spacing,
    damage_types,
    shown_above,
    placeholder_type,
    placeholder_value,
    pair_gap,
    pair_width,
    pair_divisor,
    count_open,
    count_close,
    count_above,
    effects_labels,
    effects_hide,
    effects_mask,
    effects_holder,
    effects_gap,
    effects_step,
    widget,
    icon_background_shape,
    effects_background_shape,
    entry_holder,
    timer,
    background_pad,
    effect_entry,
    effect_source,
    effect_text,
    source_suffix,
    set_text,
});

fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    StatusLayout::resolve(layout).map(drop)
}

pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[&KEYS],
    resolve: validate,
};

fn unresolved(key: &str, reason: impl Into<String>) -> UiLayoutError {
    UiLayoutError::unresolved(MOVIE, key, reason.into())
}

fn number(layout: &UiLayout, key: &LiteralKey) -> Result<f32, UiLayoutError> {
    Ok(layout.lit_number(key, 0)? as f32)
}

/// The body variant for the crippled arms and legs, one bit per limb. Stand-in for `BodyFlags`,
/// which native code sends.
pub fn body_variant(conditions: impl Fn(Limb) -> f32) -> u8 {
    let crippled = |limb: Limb| u8::from(conditions(limb) <= 0.0);
    let variant = crippled(Limb::LeftArm)
        | crippled(Limb::RightArm) << 1
        | crippled(Limb::LeftLeg) << 2
        | crippled(Limb::RightLeg) << 3;
    variant.min(CONDITION_BODY_VARIANTS - 1)
}

/// A placement's matrix on each frame of its parent's timeline, from its first frame.
fn frame_matrices(placement: &crate::ui_layout::values::Placement) -> Vec<Matrix> {
    let track = placement.track();
    let [_, last] = placement.frames;
    (1..=last)
        .map(|frame| {
            track
                .iter()
                .take_while(|key| key.frame <= frame)
                .last()
                .map_or(placement.local.matrix, |key| key.placement.matrix)
        })
        .collect()
}

/// The condition boy: the body clip scaled inside its holder, the head clip inside the body's
/// `Head_mc`, which moves with the body's animation.
struct BoyLayout {
    body: Matrix,
    shade: Shade,
    head_frame_step: usize,
    head_matrices: Vec<Vec<Matrix>>,
}

/// One damage or resistance widget (`Stats_DMGDRWidget`).
struct WidgetLayout {
    y: f32,
    /// The icon clip's `x + width`, and its bounds' left and right edge in the widget.
    icon_start: f32,
    icon_bounds: (f32, f32),
    panel: OutlinedShape,
    glyphs: [Pos2; 2],
    glyph_scale: f32,
    entry_width: f32,
    entry_right: f32,
    entry_panel: OutlinedShape,
    entry_icon: (Pos2, f32),
    entry_value: FieldSpec,
    spacing: f32,
    damage_types: u32,
    shown_above: f32,
    placeholder: TypeTotal,
    pair_gap: f32,
    pair_width: f32,
    pair_divisor: f32,
}

/// The active-effects column (`ActiveEffectsWidget` per effect type, `ActiveEffects_Entry` per
/// source).
struct EffectsLayout {
    hide: OutlinedShape,
    mask: Rect,
    origin: Pos2,
    gap: f32,
    step: f32,
    widget_height: f32,
    icon_background: Background,
    effects_background: Background,
    entry_holder: Vec2,
    timer: (Pos2, f32),
    background_pad: f32,
    entry_height: f32,
    source: FieldSpec,
    text: FieldSpec,
    source_suffix: String,
}

pub struct StatusLayout {
    name: FieldSpec,
    meters: [Meter; 5],
    torso: Meter,
    meter_ghosts: [f32; 5],
    meter_maxima: [f32; 5],
    gain_above: f32,
    boy: BoyLayout,
    widgets: WidgetLayout,
    effects: EffectsLayout,
    count_texts: [(String, String); 2],
    count_above: [u32; 2],
    effects_labels: [String; 2],
    rules: SetTextRules,
}

/// A background whose clip height the code sets: its shape and the clip height it is authored at.
struct Background {
    shape: OutlinedShape,
    clip_height: f32,
}

impl Background {
    fn resolve(layout: &UiLayout, key: &ShapeKey) -> Result<Self, UiLayoutError> {
        let shape = layout.get(key)?;
        let [_, y0, _, y1] = shape.bounds;
        Ok(Self {
            shape: OutlinedShape::resolve(layout, key)?,
            clip_height: y1 - y0,
        })
    }

    fn at_height(&self, origin: Vec2, height: f32) -> OutlinedShape {
        self.shape
            .scaled(Vec2::new(1.0, height / self.clip_height))
            .translated(origin)
    }
}

impl BoyLayout {
    fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let boy = layout.get(&KEYS.boy)?;
        let scale = number(layout, &KEYS.body_scale)?;
        let body = boy.world.matrix.then(&Matrix {
            a: scale,
            d: scale,
            ..Matrix::IDENTITY
        });
        let head_matrices = KEYS
            .body_heads
            .iter()
            .map(|key| layout.get(key).map(|head| frame_matrices(&head)))
            .collect::<Result<Vec<_>, _>>()?;
        if head_matrices.iter().any(Vec::is_empty) {
            return Err(unresolved(
                KEYS.body_heads[0].path,
                "a body clip has no head frames",
            ));
        }
        Ok(Self {
            body,
            shade: Shade::of([255, 255, 255, 255], &boy.world.color),
            head_frame_step: layout.lit_whole(&KEYS.head_frame_step, 0)? as usize,
            head_matrices,
        })
    }
}

impl WidgetLayout {
    fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let [icon_left, _, icon_right, _] = layout.get(&KEYS.widget_icon)?;
        let icon_x = layout.get(&KEYS.widget_icon_clip)?.local.matrix.tx;
        let [entry_left, _, entry_right, _] = layout.get(&KEYS.entry)?;
        let glyph = |key: &PlacementKey| -> Result<(Pos2, f32), UiLayoutError> {
            let matrix = layout.get(key)?.local.matrix;
            Ok((Pos2::new(matrix.tx, matrix.ty), matrix.a))
        };
        let (weapon, glyph_scale) = glyph(&KEYS.widget_glyphs[0])?;
        let (armour, _) = glyph(&KEYS.widget_glyphs[1])?;
        let icon = layout.get(&KEYS.entry_icon)?.local.matrix;
        let placeholder_kind = numbered_type(layout.lit_whole(&KEYS.placeholder_type, 0)?)
            .ok_or_else(|| unresolved(KEYS.placeholder_type.path, "not a damage type"))?;
        Ok(Self {
            y: layout.get(&KEYS.widgets[0])?.world.matrix.ty,
            icon_start: icon_x + icon_right - icon_left,
            icon_bounds: (icon_left, icon_right),
            panel: OutlinedShape::resolve(layout, &KEYS.widget_panel)?,
            glyphs: [weapon, armour],
            glyph_scale,
            entry_width: entry_right - entry_left,
            entry_right,
            entry_panel: OutlinedShape::resolve(layout, &KEYS.entry_panel)?,
            entry_icon: (Pos2::new(icon.tx, icon.ty), icon.a),
            entry_value: FieldSpec::resolve(layout, &KEYS.entry_value, TextAutoSize::Shrink)?,
            spacing: layout.get(&KEYS.entry_spacing)? as f32,
            damage_types: layout.whole_constant(&KEYS.damage_types)?,
            shown_above: number(layout, &KEYS.shown_above)?,
            placeholder: TypeTotal {
                kind: placeholder_kind,
                amount: number(layout, &KEYS.placeholder_value)?,
            },
            pair_gap: number(layout, &KEYS.pair_gap)?,
            pair_width: number(layout, &KEYS.pair_width)?,
            pair_divisor: number(layout, &KEYS.pair_divisor)?,
        })
    }

    /// The entries a widget shows: every total above the threshold in the given order, or the
    /// placeholder when there are no totals at all.
    fn entries(&self, totals: &[TypeTotal]) -> Vec<TypeTotal> {
        if totals.is_empty() {
            return vec![self.placeholder];
        }
        totals
            .iter()
            .filter(|total| total.amount > self.shown_above)
            .copied()
            .collect()
    }

    /// Each entry's x in the widget and the widget's width: entries follow the icon's
    /// `x + width` and each other's width, each plus the spacing.
    fn entry_positions(&self, count: usize) -> (Vec<f32>, f32) {
        let mut cursor = self.icon_start + self.spacing;
        let mut xs = Vec::with_capacity(count);
        for _ in 0..count {
            xs.push(cursor);
            cursor += self.entry_width + self.spacing;
        }
        let (icon_left, icon_right) = self.icon_bounds;
        let right = xs
            .last()
            .map_or(icon_right, |x| icon_right.max(x + self.entry_right));
        (xs, right - icon_left)
    }

    /// Where the damage and the resistance widget start so the pair is centred.
    fn pair_x(&self, widths: [f32; 2]) -> [f32; 2] {
        let damage =
            (self.pair_width - (widths[0] + widths[1] + self.pair_gap)) / self.pair_divisor;
        [damage, damage + widths[0] + self.pair_gap]
    }
}

impl EffectsLayout {
    fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let holder = layout.get(&KEYS.effects_holder)?.world.matrix;
        let [_, widget_top, _, widget_bottom] = layout.get(&KEYS.widget)?;
        let [_, entry_top, _, entry_bottom] = layout.get(&KEYS.effect_entry)?;
        let entries = layout.get(&KEYS.entry_holder)?.local.matrix;
        let timer = layout.get(&KEYS.timer)?.local.matrix;
        let hide = OutlinedShape::resolve(layout, &KEYS.effects_hide)?;
        Ok(Self {
            mask: OutlinedShape::resolve(layout, &KEYS.effects_mask)?.bounds(),
            hide,
            origin: Pos2::new(holder.tx, holder.ty),
            gap: number(layout, &KEYS.effects_gap)?,
            step: number(layout, &KEYS.effects_step)?,
            widget_height: widget_bottom - widget_top,
            icon_background: Background::resolve(layout, &KEYS.icon_background_shape)?,
            effects_background: Background::resolve(layout, &KEYS.effects_background_shape)?,
            entry_holder: Vec2::new(entries.tx, entries.ty),
            timer: (Pos2::new(timer.tx, timer.ty), timer.a),
            background_pad: number(layout, &KEYS.background_pad)?,
            entry_height: entry_bottom - entry_top,
            source: FieldSpec::resolve(layout, &KEYS.effect_source, TextAutoSize::Shrink)?,
            text: FieldSpec::resolve(layout, &KEYS.effect_text, TextAutoSize::Shrink)?,
            source_suffix: layout.lit_string(&KEYS.source_suffix, 0)?,
        })
    }

    /// The height of a widget with `entries` sources: its icon tile alone when the panel is
    /// hidden, else the entries plus the background's padding.
    fn widget_height(&self, entries: usize, showing: bool) -> f32 {
        if showing {
            entries as f32 * self.entry_height + self.background_pad
        } else {
            self.widget_height
        }
    }

    /// How far the column can scroll: how far its widgets run past the bottom of the hide rect.
    fn scroll_distance(&self, total_height: f32) -> f32 {
        (self.origin.y + total_height - self.hide.bounds().max.y).max(0.0)
    }
}

impl StatusLayout {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let meters = [0, 1, 2, 3, 4].map(|index| Meter::resolve(layout, &KEYS.meters[index]));
        let [m0, m1, m2, m3, m4] = meters;
        let numbers = |keys: &[LiteralKey; 5]| -> Result<[f32; 5], UiLayoutError> {
            Ok([
                number(layout, &keys[0])?,
                number(layout, &keys[1])?,
                number(layout, &keys[2])?,
                number(layout, &keys[3])?,
                number(layout, &keys[4])?,
            ])
        };
        let count_above = |index: usize| -> Result<u32, UiLayoutError> {
            let key = &KEYS.count_above[index];
            layout.lit_whole(key, 0)
        };
        Ok(Self {
            name: FieldSpec::resolve(layout, &KEYS.name, TextAutoSize::Shrink)?,
            meters: [m0?, m1?, m2?, m3?, m4?],
            torso: Meter::resolve(layout, &KEYS.torso)?,
            meter_ghosts: numbers(&KEYS.meter_ghosts)?,
            meter_maxima: numbers(&KEYS.meter_maxima)?,
            gain_above: number(layout, &KEYS.gain_above)?,
            boy: BoyLayout::resolve(layout)?,
            widgets: WidgetLayout::resolve(layout)?,
            effects: EffectsLayout::resolve(layout)?,
            count_texts: [
                (
                    layout.lit_string(&KEYS.count_open[0], 0)?,
                    layout.lit_string(&KEYS.count_close[0], 0)?,
                ),
                (
                    layout.lit_string(&KEYS.count_open[1], 0)?,
                    layout.lit_string(&KEYS.count_close[1], 0)?,
                ),
            ],
            count_above: [count_above(0)?, count_above(1)?],
            effects_labels: [
                layout.lit_string(&KEYS.effects_labels, 0)?,
                layout.lit_string(&KEYS.effects_labels, 1)?,
            ],
            rules: KEYS.set_text.resolve(layout)?,
        })
    }

    /// The torso meter's ghost: the health after the active healing, none without healing.
    fn torso_ghost(&self, hp: f32, hp_max: f32, gain: f32) -> f32 {
        if gain > self.gain_above {
            (hp + hp_max * gain).min(hp_max)
        } else {
            0.0
        }
    }

    /// The stimpak or radaway hint text: its marked label with the count.
    fn count_label(&self, index: usize, count: u32) -> String {
        let (open, close) = &self.count_texts[index];
        format!("{open}{count}{close}")
    }
}

pub struct StatusPage {
    layout: StatusLayout,
    show_effects: bool,
    /// How far the effects column is scrolled up, in stage pixels.
    effects_scroll: f32,
}

impl StatusPage {
    pub fn new(layout: StatusLayout) -> Self {
        Self {
            layout,
            show_effects: false,
            effects_scroll: 0.0,
        }
    }

    fn item_count(snapshot: &PipBoySnapshot<'_>, form_key: &str) -> u32 {
        snapshot
            .model
            .inventory
            .items
            .iter()
            .filter(|item| item.form_key.as_deref() == Some(form_key))
            .map(|item| item.count)
            .sum()
    }

    /// The stimpak and radaway counts, each with whether its hint is enabled.
    fn counts(&self, snapshot: &PipBoySnapshot<'_>) -> [(u32, bool); 2] {
        let [stimpak_above, radaway_above] = self.layout.count_above;
        let stimpaks = Self::item_count(snapshot, STIMPAK_FORM_KEY);
        let radaways = Self::item_count(snapshot, RADAWAY_FORM_KEY);
        [
            (stimpaks, stimpaks > stimpak_above),
            (radaways, radaways > radaway_above),
        ]
    }

    fn effects_height(&self, snapshot: &PipBoySnapshot<'_>) -> f32 {
        let effects = &self.layout.effects;
        let entries = snapshot.model.character.effects.len();
        if entries == 0 {
            return 0.0;
        }
        effects.widget_height(entries, self.show_effects) + effects.gap
    }

    fn scroll_effects(&mut self, up: bool, snapshot: &PipBoySnapshot<'_>) {
        let effects = &self.layout.effects;
        let distance = effects.scroll_distance(self.effects_height(snapshot));
        let step = if up { -effects.step } else { effects.step };
        self.effects_scroll = (self.effects_scroll + step).clamp(0.0, distance);
    }

    fn draw_boy(&self, cx: &mut TabContext<'_>) {
        let boy = &self.layout.boy;
        let conditions = cx.snapshot.model.character.limbs;
        let variant = body_variant(|limb| conditions.get(limb));
        let colour = boy.shade.colour(cx.tint);
        let body_art = FixedArt::ConditionBody(variant);
        let frame = cx
            .art_library
            .fixed(body_art)
            .frame_at(cx.seconds_open, None);
        cx.draw_fixed_affine(body_art, frame, &boy.body, colour);
        let heads = &boy.head_matrices[usize::from(variant)];
        let head = heads[frame.min(heads.len() - 1)];
        let head_flags = if conditions.get(Limb::Head) <= 0.0 {
            CRIPPLED_HEAD_FLAGS
        } else {
            0
        };
        let head_frame = head_flags + boy.head_frame_step - 1;
        cx.draw_fixed_affine(
            FixedArt::ConditionHead,
            head_frame,
            &boy.body.then(&head),
            colour,
        );
    }

    fn draw_meters(&self, cx: &TabContext<'_>) {
        let layout = &self.layout;
        let conditions = cx.snapshot.model.character.limbs;
        for (limb, index) in CONDITION_LIMBS {
            let max = layout.meter_maxima[index];
            layout.meters[index].paint(
                cx.painter,
                cx.tint,
                conditions.get(limb) * max,
                layout.meter_ghosts[index],
                max,
            );
        }
        let vitals = cx.snapshot.vitals;
        let ghost = layout.torso_ghost(vitals.hp, vitals.hp_max, CURRENT_HP_GAIN);
        layout
            .torso
            .paint(cx.painter, cx.tint, vitals.hp, ghost, vitals.hp_max);
    }

    fn draw_widgets(&self, cx: &mut TabContext<'_>) {
        let widgets = &self.layout.widgets;
        let model = cx.snapshot.model;
        let totals = [
            widgets.entries(&equipped_damage(&model.inventory)),
            widgets.entries(&worn_resistance(&model.inventory)),
        ];
        let layouts = [0, 1].map(|side| widgets.entry_positions(totals[side].len()));
        let lefts = widgets.pair_x([layouts[0].1, layouts[1].1]);
        let white = cx.shade(1.0);
        for side in 0..2 {
            let origin = Vec2::new(lefts[side], widgets.y);
            widgets.panel.translated(origin).paint(cx.painter, cx.tint);
            let glyph = [Icon::Weapon, Icon::Armor][side];
            cx.draw_icon(
                glyph,
                widgets.glyphs[side] + origin,
                widgets.glyph_scale,
                white,
            );
            for (total, x) in totals[side].iter().zip(&layouts[side].0) {
                let entry = origin + Vec2::new(*x, 0.0);
                widgets
                    .entry_panel
                    .translated(entry)
                    .paint(cx.painter, cx.tint);
                let set = if side == 0 { widgets.damage_types } else { 0 };
                let (icon_at, icon_scale) = widgets.entry_icon;
                if let Some(icon) =
                    type_icon_frame(type_number(total.kind) + set, widgets.damage_types)
                {
                    cx.draw_icon(icon, icon_at + entry, icon_scale, white);
                }
                widgets.entry_value.translated(entry).paint(
                    cx.painter,
                    &floored(total.amount),
                    cx.tint,
                );
            }
        }
    }

    fn draw_effects(&self, cx: &mut TabContext<'_>) {
        let effects = &self.layout.effects;
        let list = &cx.snapshot.model.character.effects;
        if self.show_effects {
            effects.hide.paint(cx.painter, cx.tint);
        }
        if list.is_empty() {
            return;
        }
        let painter = cx
            .painter
            .with_clip_rect(effects.mask.intersect(cx.painter.clip_rect()));
        let origin = effects.origin.to_vec2() - Vec2::new(0.0, self.effects_scroll);
        let height = effects.widget_height(list.len(), self.show_effects);
        effects
            .icon_background
            .at_height(origin, height)
            .paint(&painter, cx.tint);
        if self.show_effects {
            effects
                .effects_background
                .at_height(origin, height)
                .paint(&painter, cx.tint);
            for (index, effect) in list.iter().enumerate() {
                let entry = origin
                    + effects.entry_holder
                    + Vec2::new(0.0, index as f32 * effects.entry_height);
                let source = format!("{}{}", effect.source, effects.source_suffix);
                effects
                    .source
                    .translated(entry)
                    .paint(&painter, &source, cx.tint);
                effects
                    .text
                    .translated(entry)
                    .paint(&painter, &effect.description, cx.tint);
            }
        }
        if list
            .iter()
            .any(|effect| effect.remaining_seconds.is_some_and(|left| left > 0.0))
        {
            let (at, scale) = effects.timer;
            let white = cx.shade(1.0);
            cx.draw_icon(Icon::Timer, at + origin, scale, white);
        }
    }
}

impl Tab for StatusPage {
    fn handle_input(&mut self, input: NavInput, cx: &mut TabContext<'_>) -> InputResult {
        let snapshot = cx.snapshot;
        let [(_, stimpak), (_, radaway)] = self.counts(snapshot);
        let has_effects = !snapshot.model.character.effects.is_empty();
        match input {
            NavInput::Hint(HintKey::E) if stimpak => {
                cx.actions.push(PipBoyAction::UseFirstStimpak);
            }
            NavInput::Hint(HintKey::R) if radaway => {
                cx.actions.push(PipBoyAction::UseFirstRadAway);
            }
            NavInput::Hint(HintKey::Q) if has_effects => {
                self.show_effects = !self.show_effects;
                self.effects_scroll = 0.0;
            }
            NavInput::Up | NavInput::Down if self.show_effects => {
                self.scroll_effects(input == NavInput::Up, snapshot);
            }
            NavInput::Scrolled(delta) if self.show_effects && delta != 0.0 => {
                self.scroll_effects(delta > 0.0, snapshot);
            }
            _ => return InputResult::Ignored,
        }
        InputResult::Consumed
    }

    fn draw(&mut self, cx: &mut TabContext<'_>) {
        if cx.snapshot.model.character.effects.is_empty() {
            self.show_effects = false;
        }
        let name =
            self.layout
                .rules
                .apply(cx.layout, &cx.snapshot.model.character.name, false, false);
        self.layout.name.paint(cx.painter, &name, cx.tint);
        self.draw_boy(cx);
        self.draw_meters(cx);
        self.draw_widgets(cx);
        self.draw_effects(cx);
    }

    fn hints(&self, snapshot: &PipBoySnapshot<'_>) -> Vec<ButtonHint> {
        let layout = &self.layout;
        let [(stimpaks, stimpak), (radaways, radaway)] = self.counts(snapshot);
        let mut hints = vec![
            ButtonHint::new(HintKey::E, layout.count_label(0, stimpaks)).enabled(stimpak),
            ButtonHint::new(HintKey::R, layout.count_label(1, radaways)).enabled(radaway),
        ];
        if !snapshot.model.character.effects.is_empty() {
            let label = &layout.effects_labels[usize::from(!self.show_effects)];
            hints.push(ButtonHint::new(HintKey::Q, label.clone()));
        }
        hints
    }

    fn bottom_bar(&self) -> BottomBar {
        BottomBar::Stat
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact literals")]
mod tests {
    use super::*;
    use crate::rendering::pipboy::fonts::Weight;
    use crate::sim::pipboy::DamageType;
    use crate::ui_layout::scaleform_text::FieldGeometry;
    use crate::ui_layout::test_support::{FIXTURE_METRICS, text_field};

    #[test]
    fn the_body_variant_sets_one_bit_per_crippled_limb() {
        let healthy = |_: Limb| 1.0;
        assert_eq!(body_variant(healthy), 0);
        assert_eq!(
            body_variant(|limb| if limb == Limb::LeftArm { 0.0 } else { 1.0 }),
            1
        );
        assert_eq!(
            body_variant(|limb| if limb == Limb::RightLeg { 0.0 } else { 1.0 }),
            8
        );
        assert_eq!(body_variant(|_| 0.0), 15);
    }

    fn widgets() -> WidgetLayout {
        let shape = OutlinedShape::from_rect(
            Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0)),
            Shade::WHITE,
        );
        WidgetLayout {
            y: 0.0,
            icon_start: 50.0,
            icon_bounds: (-1.0, 49.0),
            panel: shape.clone(),
            glyphs: [Pos2::ZERO; 2],
            glyph_scale: 1.0,
            entry_width: 31.0,
            entry_right: 30.0,
            entry_panel: shape.clone(),
            entry_icon: (Pos2::ZERO, 1.0),
            entry_value: FieldSpec {
                geometry: FieldGeometry::of(
                    &text_field([0.0, 0.0, 30.0, 30.0], 20.0),
                    FIXTURE_METRICS,
                ),
                weight: Weight::Regular,
                shade: Shade::WHITE,
                auto_size: TextAutoSize::Shrink,
                field_auto_size: None,
            },
            spacing: 4.0,
            damage_types: 6,
            shown_above: 0.0,
            placeholder: TypeTotal {
                kind: DamageType::Physical,
                amount: 0.0,
            },
            pair_gap: 10.0,
            pair_width: 400.0,
            pair_divisor: 2.0,
        }
    }

    #[test]
    fn a_widget_runs_its_entries_after_the_icon_and_measures_to_the_last() {
        let (xs, width) = widgets().entry_positions(2);
        assert_eq!(xs, [54.0, 89.0]);
        assert_eq!(width, 120.0);
    }

    #[test]
    fn the_pair_is_centred_with_its_gap() {
        assert_eq!(widgets().pair_x([100.0, 70.0]), [110.0, 220.0]);
    }

    #[test]
    fn a_widget_without_totals_shows_the_placeholder_and_hides_zeroes() {
        let widgets = widgets();
        assert_eq!(widgets.entries(&[]).len(), 1);
        let totals = [
            TypeTotal {
                kind: DamageType::Fire,
                amount: 0.0,
            },
            TypeTotal {
                kind: DamageType::Energy,
                amount: 3.0,
            },
        ];
        assert_eq!(widgets.entries(&totals), [totals[1]]);
    }

    fn page() -> StatusPage {
        let layout = crate::rendering::pipboy::test_layout::pipboy_layout();
        StatusPage::new(StatusLayout::resolve(&layout).expect("the test layout resolves"))
    }

    fn snapshot(model: &crate::sim::pipboy::PipBoyModel) -> PipBoySnapshot<'_> {
        PipBoySnapshot {
            model,
            clock: crate::sim::WorldClock::default().date(),
            vitals: crate::sim::pipboy::PlayerVitals {
                hp: 50.0,
                hp_max: 100.0,
                ap: 0.0,
                ap_max: 0.0,
                position: None,
                heading_yaw: 0.0,
            },
        }
    }

    #[test]
    fn the_effects_hint_shows_only_while_there_are_effects() {
        let mut model = crate::sim::pipboy::PipBoyModel::new(std::sync::Arc::default());
        let page = page();
        let keys = |page: &StatusPage, model: &crate::sim::pipboy::PipBoyModel| {
            page.hints(&snapshot(model))
                .iter()
                .map(|hint| hint.key)
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(&page, &model), [HintKey::E, HintKey::R]);
        model
            .character
            .effects
            .push(crate::sim::pipboy::character::ActiveEffect {
                source: "a".to_owned(),
                description: "b".to_owned(),
                remaining_seconds: None,
            });
        assert_eq!(keys(&page, &model), [HintKey::E, HintKey::R, HintKey::Q]);
    }

    #[test]
    fn the_stimpak_hint_counts_and_enables_with_stimpaks() {
        let model = crate::sim::pipboy::PipBoyModel::new(std::sync::Arc::default());
        let page = page();
        let hints = page.hints(&snapshot(&model));
        assert!(!hints[0].enabled);
        assert_eq!(hints[0].label, page.layout.count_label(0, 0));
    }

    #[test]
    fn the_torso_ghost_adds_active_healing_up_to_the_maximum() {
        let mut layout = page().layout;
        layout.gain_above = 0.0;
        assert_eq!(layout.torso_ghost(50.0, 100.0, 0.0), 0.0);
        assert_eq!(layout.torso_ghost(50.0, 100.0, 0.2), 70.0);
        assert_eq!(layout.torso_ghost(90.0, 100.0, 0.5), 100.0);
    }

    #[test]
    #[ignore = "needs OPENFO4_INSTALL_DIR with the converted interface"]
    fn the_installed_status_page_lays_out_its_widgets_and_boy_as_the_game_does() {
        use crate::ui_layout::test_support::assert_all_close;
        let layout = StatusLayout::resolve(&crate::ui_layout::test_support::installed_layout())
            .expect("the installed STATUS page resolves");
        let widgets = &layout.widgets;
        assert_all_close(
            &[widgets.icon_start, widgets.entry_width, widgets.spacing],
            &[73.5277, 42.6726, 3.0],
        );
        let boy = &layout.boy;
        assert_eq!(
            boy.head_matrices.len(),
            usize::from(CONDITION_BODY_VARIANTS)
        );
        let head = boy.head_matrices[0][0];
        assert_all_close(
            &[boy.body.tx, boy.body.a, head.tx, head.ty],
            &[424.95, 1.2, -19.75, -27.75],
        );
    }
}
