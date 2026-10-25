//! The bottom info bar (`Pipboy_BottomBar`): one frame of its info clip per page, each with its
//! panels, texts, meters, icons and the damage or resistance widget, all read from the layout.

use egui::{Pos2, Vec2};

use super::SET_TEXT;
use super::scaleform_text::{FieldSpec, TextAutoSize};
use super::shapes::{Meter, MeterKeys, OutlinedShape, Shade};
use super::summary::{
    CURRENT_HP_GAIN, TypeTotal, equipped_damage, floored, type_icon_frame, type_number,
    worn_resistance,
};
use super::tab::{BottomBar, TabContext};
use crate::assets::pipboy_art::Icon;
use crate::sim::pipboy::InventoryCategory;
use crate::sim::world_clock::{GAME_YEAR_BASE, GameClock};
use crate::ui_layout::set_text::SetTextRules;
use crate::ui_layout::values::Matrix;
use crate::ui_layout::{
    BoundsKey, LiteralKey, ModuleKeys, NumberKey, PlacementKey, ShapeKey, StringKey, TextKey,
    UiLayout, UiLayoutError, key_check_fields,
};

const MOVIE: &str = "PipboyMenu";

macro_rules! info {
    ($frame:literal, $($path:literal),+) => {
        concat!("Menu_mc/BottomBar_mc/Info_mc:", $frame, "/", $($path),+)
    };
}

macro_rules! redraw {
    ($what:literal) => {
        concat!("Pipboy_BottomBar.redrawUIComponent#", $what)
    };
}

macro_rules! panels {
    ($frame:literal) => {
        [
            ShapeKey::new(MOVIE, info!($frame, "@1/@1")),
            ShapeKey::new(MOVIE, info!($frame, "@3/@1")),
            ShapeKey::new(MOVIE, info!($frame, "@5/@1")),
        ]
    };
}

macro_rules! meter {
    ($frame:literal, $clip:literal) => {
        MeterKeys {
            clip: PlacementKey::new(MOVIE, info!($frame, $clip)),
            bounds: BoundsKey::new(MOVIE, info!($frame, $clip)),
            outline: ShapeKey::new(MOVIE, info!($frame, $clip, "/@5")),
            fill: PlacementKey::new(MOVIE, info!($frame, $clip, "/Fill_mc")),
            fill_shape: ShapeKey::new(MOVIE, info!($frame, $clip, "/Fill_mc/@1")),
            ghost: PlacementKey::new(MOVIE, info!($frame, $clip, "/GhostFill_mc")),
            ghost_shape: ShapeKey::new(MOVIE, info!($frame, $clip, "/GhostFill_mc/@1")),
        }
    };
}

macro_rules! inventory_frame {
    ($frame:literal) => {
        InventoryKeys {
            panels: panels!($frame),
            weight_icon: PlacementKey::new(MOVIE, info!($frame, "@7")),
            caps_icon: PlacementKey::new(MOVIE, info!($frame, "@9")),
            weight: TextKey::new(MOVIE, info!($frame, "Weight_tf")),
            caps: TextKey::new(MOVIE, info!($frame, "Caps_tf")),
        }
    };
}

macro_rules! widget {
    ($frame:literal) => {
        WidgetKeys {
            widget: PlacementKey::new(MOVIE, info!($frame, "DMGDRWidget_mc")),
            glyph: PlacementKey::new(MOVIE, info!($frame, "DMGDRWidget_mc/Icon_mc")),
        }
    };
}

const fn text(path: &'static str, count: usize) -> LiteralKey {
    LiteralKey::texts(MOVIE, path, count)
}

const fn number(path: &'static str) -> LiteralKey {
    LiteralKey::new(MOVIE, path, 1)
}

struct StatsKeys {
    panels: [ShapeKey; 3],
    hp: TextKey,
    level: TextKey,
    ap: TextKey,
    xp: MeterKeys,
}

static STATS: StatsKeys = StatsKeys {
    panels: panels!("StatsPage"),
    hp: TextKey::new(MOVIE, info!("StatsPage", "HP_tf")),
    level: TextKey::new(MOVIE, info!("StatsPage", "LVL_tf")),
    ap: TextKey::new(MOVIE, info!("StatsPage", "AP_tf")),
    xp: meter!("StatsPage", "XPMeter_mc"),
};

struct InventoryKeys {
    panels: [ShapeKey; 3],
    weight_icon: PlacementKey,
    caps_icon: PlacementKey,
    weight: TextKey,
    caps: TextKey,
}

static WEAPONS: InventoryKeys = inventory_frame!("InvPage_Weapons");
static APPAREL: InventoryKeys = inventory_frame!("InvPage_Apparel");
static AID: InventoryKeys = inventory_frame!("InvPage_Aid");
static MISC: InventoryKeys = inventory_frame!("InvPage_Misc");

struct WidgetKeys {
    widget: PlacementKey,
    glyph: PlacementKey,
}

static WEAPONS_WIDGET: WidgetKeys = widget!("InvPage_Weapons");
static APPAREL_WIDGET: WidgetKeys = widget!("InvPage_Apparel");

static AID_HP_LABEL: TextKey = TextKey::new(MOVIE, info!("InvPage_Aid", "@13"));
static AID_HP_METER: MeterKeys = meter!("InvPage_Aid", "HPMeter");

struct DataKeys {
    panels: [ShapeKey; 3],
    date: TextKey,
    time: TextKey,
    location: TextKey,
}

static DATA: DataKeys = DataKeys {
    panels: panels!("DataPage"),
    date: TextKey::new(MOVIE, info!("DataPage", "Date_tf")),
    time: TextKey::new(MOVIE, info!("DataPage", "Time_tf")),
    location: TextKey::new(MOVIE, info!("DataPage", "Location_tf")),
};

static NONE_PANEL: ShapeKey = ShapeKey::new(MOVIE, info!("None", "@1/@1"));

/// The pieces the bar's code joins its texts from.
struct TextKeys {
    labels: LiteralKey,
    hp_gap: LiteralKey,
    hp_slash: LiteralKey,
    hp_floor: LiteralKey,
    level_gap: LiteralKey,
    ap_tails: LiteralKey,
    ap_slash: LiteralKey,
    no_ap_below: LiteralKey,
    xp_scale: LiteralKey,
    xp_ghost: LiteralKey,
    xp_max: LiteralKey,
    weight_slash: LiteralKey,
    max_caps: NumberKey,
    max_caps_text: StringKey,
    date_dot: LiteralKey,
    year_prefix: LiteralKey,
    minutes_per_hour: LiteralKey,
    half_day: LiteralKey,
    midnight: LiteralKey,
    hour_names: LiteralKey,
    colon: LiteralKey,
    pad_below: LiteralKey,
    pad: LiteralKey,
    time_gap: LiteralKey,
    day_halves: LiteralKey,
    morning_before: LiteralKey,
    current_gain_above: LiteralKey,
    selected_gain_above: LiteralKey,
}

static TEXTS: TextKeys = TextKeys {
    labels: text(redraw!("GlobalFunc.SetText(1)"), 4),
    hp_gap: text(redraw!("Info_mc.HP_tf.text+"), 1),
    hp_slash: text(redraw!("((Info_mc.HP_tf.text+\"  \")+Math.max())+"), 1),
    hp_floor: number(redraw!("Math.max(0)")),
    level_gap: text(redraw!("Info_mc.LVL_tf.text+"), 1),
    ap_tails: text(redraw!("Info_mc.AP_tf.text+"), 2),
    ap_slash: text(redraw!("((Info_mc.AP_tf.text+\"  \")+Math.floor())+"), 1),
    no_ap_below: number(redraw!("_DataObj.MaxAP<=")),
    xp_scale: number(redraw!("_DataObj.XPProgressPct*")),
    xp_ghost: number(redraw!("Info_mc.XPMeter_mc.SetMeter(1)")),
    xp_max: number(redraw!("Info_mc.XPMeter_mc.SetMeter(2)")),
    weight_slash: text(redraw!("Math.floor()+"), 1),
    max_caps: NumberKey::new(MOVIE, "Pipboy_BottomBar.MAX_DISPLAY_CAPS"),
    max_caps_text: StringKey::new(MOVIE, "Pipboy_BottomBar.MAX_DISPLAY_CAPS_TEXT"),
    date_dot: text(redraw!("_DataObj.DateMonth+"), 1),
    year_prefix: text(redraw!("((_DataObj.DateMonth+\".\")+_DataObj.DateDay)+"), 1),
    minutes_per_hour: number(redraw!("(_DataObj.TimeHour-$1)*")),
    half_day: number(redraw!("$1%")),
    midnight: number(redraw!("$3==")),
    hour_names: LiteralKey::new(MOVIE, redraw!("$3="), 2),
    colon: text(redraw!("$3+"), 1),
    pad_below: number(redraw!("$2<")),
    pad: text(redraw!("+$2"), 1),
    time_gap: text(
        redraw!("(($3+\":\")+(($2<10)?(\"0\"+$2):$2.toString()))+"),
        1,
    ),
    day_halves: text(
        redraw!("((($3+\":\")+(($2<10)?(\"0\"+$2):$2.toString()))+\" \")+"),
        2,
    ),
    morning_before: number(redraw!("$1<")),
    current_gain_above: number(redraw!("_DataObj.CurrentHPGain>")),
    selected_gain_above: number(redraw!("_DataObj.SelectedItemHPGain>")),
};

/// The damage or resistance widget (`BB_DMGDRWidget`) and its entry symbol.
struct EntryKeys {
    value: TextKey,
    icon: PlacementKey,
    icon_frames: [BoundsKey; 12],
    icon_gap: LiteralKey,
    width_divisor: LiteralKey,
    spacing: NumberKey,
    types_per_set: NumberKey,
    start: LiteralKey,
    glyph_gap: LiteralKey,
    shown_above: LiteralKey,
    placeholder_type: LiteralKey,
    placeholder_value: LiteralKey,
}

macro_rules! entry_icon {
    ($frame:literal) => {
        BoundsKey::new(MOVIE, concat!("[BB_DMGDRWidget_Entry]/Icon_mc:", $frame))
    };
}

static ENTRY: EntryKeys = EntryKeys {
    value: TextKey::new(MOVIE, "[BB_DMGDRWidget_Entry]/Value_tf"),
    icon: PlacementKey::new(MOVIE, "[BB_DMGDRWidget_Entry]/Icon_mc"),
    icon_frames: [
        BoundsKey::new(MOVIE, "[BB_DMGDRWidget_Entry]/Icon_mc"),
        entry_icon!("2"),
        entry_icon!("3"),
        entry_icon!("4"),
        entry_icon!("5"),
        entry_icon!("6"),
        entry_icon!("7"),
        entry_icon!("8"),
        entry_icon!("9"),
        entry_icon!("10"),
        entry_icon!("11"),
        entry_icon!("12"),
    ],
    icon_gap: number("BB_DMGDRWidget_Entry.redraw#(Value_tf.x+$4.x)-"),
    width_divisor: number("BB_DMGDRWidget_Entry.get:leftX#Icon_mc.width/"),
    spacing: NumberKey::new(MOVIE, "BB_DMGDRWidget.ENTRY_SPACING"),
    types_per_set: NumberKey::new(MOVIE, "Shared.GlobalFunc.NUM_DAMAGE_TYPES"),
    start: number("BB_DMGDRWidget.redraw#$3="),
    glyph_gap: number("BB_DMGDRWidget.redraw#$3-"),
    shown_above: number("BB_DMGDRWidget.redraw#$2[$4].value>"),
    placeholder_type: number("BB_DMGDRWidget.redraw#AddEntry(1).type"),
    placeholder_value: number("BB_DMGDRWidget.redraw#AddEntry(1).value"),
};

fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    BottomBarLayout::resolve(layout).map(drop)
}

/// Every layout key this module reads, resolved at boot.
pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[
        &STATS,
        &WEAPONS,
        &APPAREL,
        &AID,
        &MISC,
        &WEAPONS_WIDGET,
        &APPAREL_WIDGET,
        &AID_HP_LABEL,
        &AID_HP_METER,
        &DATA,
        &NONE_PANEL,
        &TEXTS,
        &ENTRY,
        &SET_TEXT,
    ],
    resolve: validate,
};

key_check_fields!(StatsKeys {
    panels,
    hp,
    level,
    ap,
    xp
});
key_check_fields!(InventoryKeys {
    panels,
    weight_icon,
    caps_icon,
    weight,
    caps
});
key_check_fields!(WidgetKeys { widget, glyph });
key_check_fields!(DataKeys {
    panels,
    date,
    time,
    location
});
key_check_fields!(TextKeys {
    labels,
    hp_gap,
    hp_slash,
    hp_floor,
    level_gap,
    ap_tails,
    ap_slash,
    no_ap_below,
    xp_scale,
    xp_ghost,
    xp_max,
    weight_slash,
    max_caps,
    max_caps_text,
    date_dot,
    year_prefix,
    minutes_per_hour,
    half_day,
    midnight,
    hour_names,
    colon,
    pad_below,
    pad,
    time_gap,
    day_halves,
    morning_before,
    current_gain_above,
    selected_gain_above,
});
key_check_fields!(EntryKeys {
    value,
    icon,
    icon_frames,
    icon_gap,
    width_divisor,
    spacing,
    types_per_set,
    start,
    glyph_gap,
    shown_above,
    placeholder_type,
    placeholder_value,
});

fn number_of(layout: &UiLayout, key: &LiteralKey, index: usize) -> Result<f32, UiLayoutError> {
    Ok(layout.lit_number(key, index)? as f32)
}

fn panels_of(layout: &UiLayout, keys: &[ShapeKey]) -> Result<Vec<OutlinedShape>, UiLayoutError> {
    keys.iter()
        .map(|key| OutlinedShape::resolve(layout, key))
        .collect()
}

fn field(layout: &UiLayout, key: &TextKey) -> Result<FieldSpec, UiLayoutError> {
    FieldSpec::resolve(layout, key, TextAutoSize::None)
}

/// A field the bar's code sets to shrink its text to fit.
fn shrinking_field(layout: &UiLayout, key: &TextKey) -> Result<FieldSpec, UiLayoutError> {
    FieldSpec::resolve(layout, key, TextAutoSize::Shrink)
}

/// A glyph drawn at a placement: its registration point, scale and colour multiplier.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PlacedIcon {
    at: Pos2,
    scale: f32,
    shade: Shade,
}

impl PlacedIcon {
    fn resolve(layout: &UiLayout, key: &PlacementKey) -> Result<Self, UiLayoutError> {
        let placement = layout.get(key)?;
        let matrix = placement.world.matrix;
        uniform_scale(key, &matrix)?;
        Ok(Self {
            at: Pos2::new(matrix.tx, matrix.ty),
            scale: matrix.a,
            shade: Shade::of([255; 4], &placement.world.color),
        })
    }

    fn draw(&self, cx: &mut TabContext<'_>, icon: Icon) {
        let colour = self.shade.colour(cx.tint);
        cx.draw_icon(icon, self.at, self.scale, colour);
    }
}

fn uniform_scale(key: &PlacementKey, matrix: &Matrix) -> Result<(), UiLayoutError> {
    let uniform = matrix.is_axis_aligned() && (matrix.a - matrix.d).abs() < 0.01;
    if uniform {
        Ok(())
    } else {
        Err(UiLayoutError::unresolved(
            key.movie,
            key.path,
            "the clip is not uniformly scaled",
        ))
    }
}

/// Only translated; the widget's code works in its parent's units.
fn translation_of(key: &PlacementKey, matrix: &Matrix) -> Result<Vec2, UiLayoutError> {
    let identity = Matrix {
        tx: matrix.tx,
        ty: matrix.ty,
        ..Matrix::IDENTITY
    };
    if *matrix == identity {
        Ok(Vec2::new(matrix.tx, matrix.ty))
    } else {
        Err(UiLayoutError::unresolved(
            key.movie,
            key.path,
            "the clip is scaled or rotated",
        ))
    }
}

struct StatsFrame {
    panels: Vec<OutlinedShape>,
    hp: FieldSpec,
    level: FieldSpec,
    ap: FieldSpec,
    xp: Meter,
}

struct InventoryFrame {
    panels: Vec<OutlinedShape>,
    weight_icon: PlacedIcon,
    caps_icon: PlacedIcon,
    weight: FieldSpec,
    caps: FieldSpec,
}

impl InventoryFrame {
    fn resolve(layout: &UiLayout, keys: &InventoryKeys) -> Result<Self, UiLayoutError> {
        Ok(Self {
            panels: panels_of(layout, &keys.panels)?,
            weight_icon: PlacedIcon::resolve(layout, &keys.weight_icon)?,
            caps_icon: PlacedIcon::resolve(layout, &keys.caps_icon)?,
            weight: shrinking_field(layout, &keys.weight)?,
            caps: shrinking_field(layout, &keys.caps)?,
        })
    }
}

struct DataFrame {
    panels: Vec<OutlinedShape>,
    date: FieldSpec,
    time: FieldSpec,
    location: FieldSpec,
}

/// The texts and numbers the bar's code joins and compares.
#[derive(Debug, Clone, PartialEq)]
pub struct BarTexts {
    pub hp_label: String,
    pub level_label: String,
    pub ap_label: String,
    pub blank: String,
    pub hp_gap: String,
    pub slash: String,
    pub hp_floor: f32,
    pub level_gap: String,
    pub no_ap: String,
    pub no_ap_below: f32,
    pub xp_scale: f32,
    pub xp_ghost: f32,
    pub xp_max: f32,
    pub max_caps: f64,
    pub max_caps_text: String,
    pub date_dot: String,
    pub year_prefix: String,
    pub clock: ClockTexts,
    pub gain_above: [f32; 2],
}

/// The pieces of the twelve-hour clock text.
#[derive(Debug, Clone, PartialEq)]
pub struct ClockTexts {
    pub minutes_per_hour: u32,
    pub half_day: u32,
    pub midnight: u32,
    pub midnight_name: u32,
    pub colon: String,
    pub pad_below: u32,
    pub pad: String,
    pub gap: String,
    pub morning: String,
    pub afternoon: String,
    pub morning_before: u32,
}

impl BarTexts {
    fn resolve(layout: &UiLayout, rules: &SetTextRules) -> Result<Self, UiLayoutError> {
        let t = &TEXTS;
        let label = |index| -> Result<String, UiLayoutError> {
            Ok(rules.apply(layout, &layout.lit_string(&t.labels, index)?, false, false))
        };
        let slashes = [&t.hp_slash, &t.ap_slash, &t.weight_slash]
            .map(|key| layout.lit_string(key, 0))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        if slashes.iter().any(|piece| *piece != slashes[0]) {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                t.hp_slash.path,
                "the bar joins its value pairs with different separators",
            ));
        }
        let hp_gap = layout.lit_string(&t.hp_gap, 0)?;
        if layout.lit_string(&t.ap_tails, 1)? != hp_gap {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                t.ap_tails.path,
                "the bar puts different gaps before HP and AP values",
            ));
        }
        Ok(Self {
            hp_label: label(0)?,
            level_label: label(1)?,
            ap_label: label(2)?,
            blank: label(3)?,
            hp_gap,
            slash: slashes[0].clone(),
            hp_floor: number_of(layout, &t.hp_floor, 0)?,
            level_gap: layout.lit_string(&t.level_gap, 0)?,
            no_ap: layout.lit_string(&t.ap_tails, 0)?,
            no_ap_below: number_of(layout, &t.no_ap_below, 0)?,
            xp_scale: number_of(layout, &t.xp_scale, 0)?,
            xp_ghost: number_of(layout, &t.xp_ghost, 0)?,
            xp_max: number_of(layout, &t.xp_max, 0)?,
            max_caps: layout.get(&t.max_caps)?,
            max_caps_text: layout.get(&t.max_caps_text)?,
            date_dot: layout.lit_string(&t.date_dot, 0)?,
            year_prefix: layout.lit_string(&t.year_prefix, 0)?,
            clock: ClockTexts {
                minutes_per_hour: layout.lit_whole(&t.minutes_per_hour, 0)?,
                half_day: layout.lit_whole(&t.half_day, 0)?,
                midnight: layout.lit_whole(&t.midnight, 0)?,
                midnight_name: layout.lit_whole(&t.hour_names, 1)?,
                colon: layout.lit_string(&t.colon, 0)?,
                pad_below: layout.lit_whole(&t.pad_below, 0)?,
                pad: layout.lit_string(&t.pad, 0)?,
                gap: layout.lit_string(&t.time_gap, 0)?,
                morning: layout.lit_string(&t.day_halves, 0)?,
                afternoon: layout.lit_string(&t.day_halves, 1)?,
                morning_before: layout.lit_whole(&t.morning_before, 0)?,
            },
            gain_above: [
                number_of(layout, &t.current_gain_above, 0)?,
                number_of(layout, &t.selected_gain_above, 0)?,
            ],
        })
    }

    /// `"$HP" + "  " + max(0, floor(hp)) + "/" + floor(max)`.
    pub fn hp(&self, hp: f32, hp_max: f32) -> String {
        let shown = hp.floor().max(self.hp_floor);
        format!(
            "{}{}{}{}{}",
            self.hp_label,
            self.hp_gap,
            floored(shown),
            self.slash,
            floored(hp_max)
        )
    }

    pub fn level(&self, level: u32) -> String {
        format!("{}{}{level}", self.level_label, self.level_gap)
    }

    pub fn ap(&self, ap: f32, ap_max: f32) -> String {
        if ap_max <= self.no_ap_below {
            format!("{}{}", self.ap_label, self.no_ap)
        } else {
            format!(
                "{}{}{}{}{}",
                self.ap_label,
                self.hp_gap,
                floored(ap),
                self.slash,
                floored(ap_max)
            )
        }
    }

    pub fn weight(&self, carried: f32, capacity: f32) -> String {
        format!("{}{}{}", floored(carried), self.slash, floored(capacity))
    }

    pub fn caps(&self, caps: u32) -> String {
        if f64::from(caps) >= self.max_caps {
            self.max_caps_text.clone()
        } else {
            caps.to_string()
        }
    }

    /// `DateMonth + "." + DateDay + ".2" + DateYear`. The game's year value counts from 2000;
    /// the clock's year is the full year.
    pub fn date(&self, clock: &GameClock) -> String {
        let year_value = i32::from(clock.year) - i32::from(GAME_YEAR_BASE);
        format!(
            "{}{}{}{}{year_value}",
            clock.month, self.date_dot, clock.day, self.year_prefix
        )
    }

    /// The `HP` meter's ghost on the AID frame: the health after the active and the selected
    /// healing, each a fraction of the maximum, capped at the maximum; none without healing.
    #[allow(clippy::float_cmp, reason = "the game compares the gains exactly")]
    pub fn healed_ghost(&self, hp: f32, hp_max: f32, current_gain: f32, selected_gain: f32) -> f32 {
        let [current_above, selected_above] = self.gain_above;
        let mut ghost = hp;
        if current_gain > current_above {
            ghost += hp_max * current_gain;
        }
        if selected_gain > selected_above {
            ghost += hp_max * selected_gain;
        }
        if current_gain == current_above && selected_gain == selected_above {
            ghost = 0.0;
        }
        ghost.min(hp_max)
    }
}

impl ClockTexts {
    /// `h:mm AM` on a twelve-hour clock.
    pub fn time(&self, minute_of_day: u16) -> String {
        let minute_of_day = u32::from(minute_of_day);
        let hour = minute_of_day / self.minutes_per_hour;
        let minute = minute_of_day % self.minutes_per_hour;
        let mut shown_hour = hour % self.half_day;
        if shown_hour == self.midnight {
            shown_hour = self.midnight_name;
        }
        let minute_text = if minute < self.pad_below {
            format!("{}{minute}", self.pad)
        } else {
            minute.to_string()
        };
        let half = if hour < self.morning_before {
            &self.morning
        } else {
            &self.afternoon
        };
        format!("{shown_hour}{}{minute_text}{}{half}", self.colon, self.gap)
    }
}

/// The damage or resistance widget's entry symbol and right-to-left layout rules.
pub struct EntryLayout {
    value: FieldSpec,
    icon_y: f32,
    icon_scale: f32,
    icon_shade: Shade,
    icon_widths: Vec<f32>,
    pub rules: EntryRules,
}

/// The numbers of `BB_DMGDRWidget.redraw`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntryRules {
    pub icon_gap: f32,
    pub width_divisor: f32,
    pub spacing: f32,
    pub types_per_set: u32,
    pub start: f32,
    pub glyph_gap: f32,
    pub shown_above: f32,
    pub placeholder: (u32, f32),
}

impl EntryRules {
    /// The entries the widget lays out, first laid out first (rightmost): the totals from the
    /// last to the first that are above the threshold, or the placeholder when there are none.
    pub fn entries(&self, totals: &[TypeTotal]) -> Vec<(u32, f32)> {
        if totals.is_empty() {
            return vec![self.placeholder];
        }
        totals
            .iter()
            .rev()
            .filter(|total| total.amount > self.shown_above)
            .map(|total| (type_number(total.kind), total.amount))
            .collect()
    }

    /// The icon frame of type `number`: shifted by one set for damage dealt.
    pub fn icon_frame(&self, number: u32, dealt: bool) -> u32 {
        if dealt {
            number + self.types_per_set
        } else {
            number
        }
    }

    /// Where an entry at `cursor` puts its icon (entry space) and where the next entry starts,
    /// given the left of its value's glyph run (entry space) and its icon's width.
    pub fn advance(&self, cursor: f32, run_left: f32, icon_width: f32) -> (f32, f32) {
        let icon_x = run_left - self.icon_gap;
        let left_x = icon_x - icon_width / self.width_divisor;
        (icon_x, cursor + left_x - self.spacing)
    }
}

impl EntryLayout {
    fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let icon = layout.get(&ENTRY.icon)?;
        uniform_scale(&ENTRY.icon, &icon.world.matrix)?;
        let icon_widths = ENTRY
            .icon_frames
            .iter()
            .map(|key| layout.get(key).map(|[x0, _, x1, _]| x1 - x0))
            .collect::<Result<Vec<_>, _>>()?;
        let types_per_set = layout.get(&ENTRY.types_per_set)?;
        if types_per_set <= 0.0 || types_per_set.fract() != 0.0 {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                ENTRY.types_per_set.path,
                "not a positive whole number",
            ));
        }
        let rules = EntryRules {
            icon_gap: number_of(layout, &ENTRY.icon_gap, 0)?,
            width_divisor: number_of(layout, &ENTRY.width_divisor, 0)?,
            spacing: layout.get(&ENTRY.spacing)? as f32,
            types_per_set: types_per_set as u32,
            start: number_of(layout, &ENTRY.start, 0)?,
            glyph_gap: number_of(layout, &ENTRY.glyph_gap, 0)?,
            shown_above: number_of(layout, &ENTRY.shown_above, 0)?,
            placeholder: (
                layout.lit_whole(&ENTRY.placeholder_type, 0)?,
                number_of(layout, &ENTRY.placeholder_value, 0)?,
            ),
        };
        if icon_widths.len() != 2 * rules.types_per_set as usize {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                ENTRY.icon.path,
                "the icon clip does not hold one resistance and one damage frame per type",
            ));
        }
        Ok(Self {
            value: shrinking_field(layout, &ENTRY.value)?,
            icon_y: icon.world.matrix.ty,
            icon_scale: icon.world.matrix.a,
            icon_shade: Shade::of([255; 4], &icon.world.color),
            icon_widths,
            rules,
        })
    }
}

/// The widget clip on one frame: where it sits and its weapon or armour glyph.
struct Widget {
    origin: Vec2,
    glyph: PlacedIcon,
    glyph_icon: Icon,
    dealt: bool,
}

impl Widget {
    fn resolve(
        layout: &UiLayout,
        keys: &WidgetKeys,
        glyph_icon: Icon,
        dealt: bool,
    ) -> Result<Self, UiLayoutError> {
        let widget = layout.get(&keys.widget)?;
        Ok(Self {
            origin: translation_of(&keys.widget, &widget.world.matrix)?,
            glyph: PlacedIcon::resolve(layout, &keys.glyph)?,
            glyph_icon,
            dealt,
        })
    }

    fn draw(&self, cx: &mut TabContext<'_>, entry: &EntryLayout, totals: &[TypeTotal]) {
        let rules = &entry.rules;
        let mut cursor = rules.start;
        for (number, value) in rules.entries(totals) {
            let entry_origin = self.origin + Vec2::new(cursor, 0.0);
            let painted =
                entry
                    .value
                    .translated(entry_origin)
                    .paint(cx.painter, &floored(value), cx.tint);
            let frame = rules.icon_frame(number, self.dealt);
            let width = frame
                .checked_sub(1)
                .and_then(|index| entry.icon_widths.get(index as usize));
            let (Some(width), Some(icon)) = (width, type_icon_frame(frame, rules.types_per_set))
            else {
                tracing::warn!("damage type {number} has no icon frame; its entry is left out");
                continue;
            };
            let (icon_x, next) = rules.advance(cursor, painted.run.0 - entry_origin.x, *width);
            let at = entry_origin + Vec2::new(icon_x, entry.icon_y);
            let colour = entry.icon_shade.colour(cx.tint);
            cx.draw_icon(icon, at.to_pos2(), entry.icon_scale, colour);
            cursor = next;
        }
        let glyph = PlacedIcon {
            at: Pos2::new(self.origin.x + cursor - rules.glyph_gap, self.glyph.at.y),
            ..self.glyph
        };
        glyph.draw(cx, self.glyph_icon);
    }
}

/// The bar resolved once from the layout.
pub struct BottomBarLayout {
    stats: StatsFrame,
    weapons: InventoryFrame,
    apparel: InventoryFrame,
    aid: InventoryFrame,
    misc: InventoryFrame,
    weapons_widget: Widget,
    apparel_widget: Widget,
    aid_hp_label: (FieldSpec, String),
    aid_hp: Meter,
    data: DataFrame,
    none: OutlinedShape,
    entry: EntryLayout,
    pub texts: BarTexts,
}

/// What the bar needs beyond the snapshot: the healing of the item the page has selected, as a
/// fraction of the maximum health.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BarInputs {
    pub selected_hp_gain: f32,
}

impl BottomBarLayout {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let rules = SET_TEXT.resolve(layout)?;
        let label = layout.get(&AID_HP_LABEL)?;
        let label_text = label.initial_text.ok_or_else(|| {
            UiLayoutError::unresolved(MOVIE, AID_HP_LABEL.path, "the static text has no text")
        })?;
        let label_text = rules.apply(layout, label_text.trim_end_matches('\r'), false, false);
        Ok(Self {
            stats: StatsFrame {
                panels: panels_of(layout, &STATS.panels)?,
                hp: shrinking_field(layout, &STATS.hp)?,
                level: shrinking_field(layout, &STATS.level)?,
                ap: shrinking_field(layout, &STATS.ap)?,
                xp: Meter::resolve(layout, &STATS.xp)?,
            },
            weapons: InventoryFrame::resolve(layout, &WEAPONS)?,
            apparel: InventoryFrame::resolve(layout, &APPAREL)?,
            aid: InventoryFrame::resolve(layout, &AID)?,
            misc: InventoryFrame::resolve(layout, &MISC)?,
            weapons_widget: Widget::resolve(layout, &WEAPONS_WIDGET, Icon::Weapon, true)?,
            apparel_widget: Widget::resolve(layout, &APPAREL_WIDGET, Icon::Armor, false)?,
            aid_hp_label: (field(layout, &AID_HP_LABEL)?, label_text),
            aid_hp: Meter::resolve(layout, &AID_HP_METER)?,
            data: DataFrame {
                panels: panels_of(layout, &DATA.panels)?,
                date: field(layout, &DATA.date)?,
                time: field(layout, &DATA.time)?,
                location: field(layout, &DATA.location)?,
            },
            none: OutlinedShape::resolve(layout, &NONE_PANEL)?,
            entry: EntryLayout::resolve(layout)?,
            texts: BarTexts::resolve(layout, &rules)?,
        })
    }

    pub fn draw(&self, cx: &mut TabContext<'_>, bar: BottomBar, inputs: BarInputs) {
        match bar {
            BottomBar::Stat => self.draw_stats(cx),
            BottomBar::Inventory(category) => self.draw_inventory(cx, category, inputs),
            BottomBar::DateTime { with_location } => self.draw_data(cx, with_location),
            BottomBar::Radio => self.none.paint(cx.painter, cx.tint),
        }
    }

    fn draw_stats(&self, cx: &TabContext<'_>) {
        let frame = &self.stats;
        paint_panels(cx, &frame.panels);
        let vitals = cx.snapshot.vitals;
        let progression = cx.snapshot.model.character.progression;
        let texts = &self.texts;
        frame
            .hp
            .paint(cx.painter, &texts.hp(vitals.hp, vitals.hp_max), cx.tint);
        frame
            .level
            .paint(cx.painter, &texts.level(progression.level), cx.tint);
        let progress = xp_progress(progression.xp_into_level, progression.xp_for_next_level);
        frame.xp.paint(
            cx.painter,
            cx.tint,
            progress * texts.xp_scale,
            texts.xp_ghost,
            texts.xp_max,
        );
        frame
            .ap
            .paint(cx.painter, &texts.ap(vitals.ap, vitals.ap_max), cx.tint);
    }

    fn draw_inventory(
        &self,
        cx: &mut TabContext<'_>,
        category: InventoryCategory,
        inputs: BarInputs,
    ) {
        let frame = match category {
            InventoryCategory::Weapons => &self.weapons,
            InventoryCategory::Apparel => &self.apparel,
            InventoryCategory::Aid => &self.aid,
            InventoryCategory::Misc
            | InventoryCategory::Junk
            | InventoryCategory::Mods
            | InventoryCategory::Ammo => &self.misc,
        };
        paint_panels(cx, &frame.panels);
        let inventory = &cx.snapshot.model.inventory;
        let weight = self
            .texts
            .weight(inventory.carried_weight(), inventory.carry_capacity);
        let caps = self.texts.caps(inventory.caps);
        match category {
            InventoryCategory::Weapons => {
                let totals = equipped_damage(inventory);
                self.weapons_widget.draw(cx, &self.entry, &totals);
            }
            InventoryCategory::Apparel => {
                let totals = worn_resistance(inventory);
                self.apparel_widget.draw(cx, &self.entry, &totals);
            }
            InventoryCategory::Aid => self.draw_aid(cx, inputs),
            InventoryCategory::Misc
            | InventoryCategory::Junk
            | InventoryCategory::Mods
            | InventoryCategory::Ammo => {}
        }
        frame.weight_icon.draw(cx, Icon::Weight);
        frame.caps_icon.draw(cx, Icon::Caps);
        frame.weight.paint(cx.painter, &weight, cx.tint);
        frame.caps.paint(cx.painter, &caps, cx.tint);
    }

    fn draw_aid(&self, cx: &TabContext<'_>, inputs: BarInputs) {
        let (label, text) = &self.aid_hp_label;
        label.paint(cx.painter, text, cx.tint);
        let vitals = cx.snapshot.vitals;
        let ghost = self.texts.healed_ghost(
            vitals.hp,
            vitals.hp_max,
            CURRENT_HP_GAIN,
            inputs.selected_hp_gain,
        );
        self.aid_hp
            .paint(cx.painter, cx.tint, vitals.hp, ghost, vitals.hp_max);
    }

    fn draw_data(&self, cx: &TabContext<'_>, with_location: bool) {
        let frame = &self.data;
        paint_panels(cx, &frame.panels);
        let model = cx.snapshot.model;
        frame
            .date
            .paint(cx.painter, &self.texts.date(&cx.snapshot.clock), cx.tint);
        let time = self.texts.clock.time(cx.snapshot.clock.minute_of_day);
        frame.time.paint(cx.painter, &time, cx.tint);
        let location = match (&model.current_location, with_location) {
            (Some(name), true) => name.as_str(),
            _ => self.texts.blank.as_str(),
        };
        frame.location.paint(cx.painter, location, cx.tint);
    }
}

fn paint_panels(cx: &TabContext<'_>, panels: &[OutlinedShape]) {
    for panel in panels {
        panel.paint(cx.painter, cx.tint);
    }
}

/// Stand-in for the game's experience progress value: the share of this level's experience
/// gained, `0` before the next level has a target.
pub fn xp_progress(into_level: u32, for_next_level: u32) -> f32 {
    if for_next_level == 0 {
        0.0
    } else {
        into_level as f32 / for_next_level as f32
    }
}

/// The experience share the perk chart's level bar shows, clamped to a full bar.
pub fn xp_fraction(into_level: u32, for_next_level: u32) -> f32 {
    xp_progress(into_level, for_next_level).clamp(0.0, 1.0)
}

#[cfg(test)]
#[allow(clippy::float_cmp, reason = "exact literals")]
mod tests {
    use super::*;
    use crate::sim::pipboy::DamageType;

    fn clock_texts() -> ClockTexts {
        ClockTexts {
            minutes_per_hour: 60,
            half_day: 12,
            midnight: 0,
            midnight_name: 12,
            colon: "h".to_owned(),
            pad_below: 10,
            pad: "o".to_owned(),
            gap: "_".to_owned(),
            morning: "MO".to_owned(),
            afternoon: "AF".to_owned(),
            morning_before: 12,
        }
    }

    fn texts() -> BarTexts {
        BarTexts {
            hp_label: "VIT".to_owned(),
            level_label: "RANK".to_owned(),
            ap_label: "STA".to_owned(),
            blank: "_".to_owned(),
            hp_gap: "::".to_owned(),
            slash: "|".to_owned(),
            hp_floor: 0.0,
            level_gap: "~".to_owned(),
            no_ap: "::?|?".to_owned(),
            no_ap_below: 0.0,
            xp_scale: 100.0,
            xp_ghost: 0.0,
            xp_max: 100.0,
            max_caps: 5000.0,
            max_caps_text: "5000^".to_owned(),
            date_dot: "-".to_owned(),
            year_prefix: "-y".to_owned(),
            clock: clock_texts(),
            gain_above: [0.0, 0.0],
        }
    }

    fn rules() -> EntryRules {
        EntryRules {
            icon_gap: 7.0,
            width_divisor: 2.0,
            spacing: 11.0,
            types_per_set: 8,
            start: 0.0,
            glyph_gap: 19.0,
            shown_above: 0.0,
            placeholder: (1, 0.0),
        }
    }

    fn clock(minute_of_day: u16) -> GameClock {
        GameClock {
            year: 2123,
            month: 4,
            day: 9,
            minute_of_day,
        }
    }

    #[test]
    fn the_date_appends_the_year_value_after_its_prefix() {
        assert_eq!(texts().date(&clock(0)), "4-9-y123");
    }

    #[test]
    fn the_time_uses_a_twelve_hour_clock_with_midnight_and_noon() {
        let clock = clock_texts();
        assert_eq!(clock.time(0), "12ho0_MO");
        assert_eq!(clock.time(9 * 60 + 7), "9ho7_MO");
        assert_eq!(clock.time(12 * 60), "12ho0_AF");
        assert_eq!(clock.time(15 * 60 + 45), "3h45_AF");
        assert_eq!(clock.time(23 * 60 + 59), "11h59_AF");
    }

    #[test]
    fn health_is_floored_and_never_below_zero() {
        assert_eq!(texts().hp(79.9, 120.7), "VIT::79|120");
        assert_eq!(texts().hp(-3.0, 120.0), "VIT::0|120");
    }

    #[test]
    fn action_points_show_dashes_without_a_maximum() {
        assert_eq!(texts().ap(0.0, 0.0), "STA::?|?");
        assert_eq!(texts().ap(60.9, 100.0), "STA::60|100");
    }

    #[test]
    fn the_level_follows_its_label() {
        assert_eq!(texts().level(12), "RANK~12");
    }

    #[test]
    fn weight_is_floored_and_caps_cap_at_the_display_maximum() {
        let texts = texts();
        assert_eq!(texts.weight(112.9, 200.9), "112|200");
        assert_eq!(texts.caps(4999), "4999");
        assert_eq!(texts.caps(5000), "5000^");
    }

    #[test]
    fn the_aid_ghost_adds_both_gains_and_hides_without_healing() {
        let texts = texts();
        assert_eq!(texts.healed_ghost(50.0, 100.0, 0.0, 0.0), 0.0);
        assert_eq!(texts.healed_ghost(50.0, 100.0, 0.1, 0.2), 80.0);
        assert_eq!(texts.healed_ghost(90.0, 100.0, 0.0, 0.5), 100.0);
    }

    #[test]
    fn widget_entries_run_from_the_last_total_and_skip_zeroes() {
        let totals = [
            TypeTotal {
                kind: DamageType::Physical,
                amount: 30.0,
            },
            TypeTotal {
                kind: DamageType::Energy,
                amount: 0.0,
            },
            TypeTotal {
                kind: DamageType::Radiation,
                amount: 4.0,
            },
        ];
        assert_eq!(rules().entries(&totals), [(6, 4.0), (1, 30.0)]);
        assert_eq!(rules().entries(&[]), [(1, 0.0)]);
    }

    #[test]
    fn damage_entries_use_the_frames_one_set_further() {
        assert_eq!(rules().icon_frame(1, true), 9);
        assert_eq!(rules().icon_frame(1, false), 1);
    }

    #[test]
    fn each_entry_ends_its_icon_spacing_left_of_its_icon() {
        let (icon_x, next) = rules().advance(-20.0, -40.0, 30.0);
        assert_eq!(icon_x, -47.0);
        assert_eq!(next, -20.0 + (-47.0 - 15.0) - 11.0);
    }

    #[test]
    fn experience_progress_is_a_share_and_survives_a_zero_target() {
        assert_eq!(xp_progress(50, 200), 0.25);
        assert_eq!(xp_progress(10, 0), 0.0);
    }
}
