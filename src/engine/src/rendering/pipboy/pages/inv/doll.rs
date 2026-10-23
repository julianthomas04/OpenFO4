//! The APPAREL paper doll: one dim base figure, the clips of the slots the selected item and the
//! worn pieces cover (those of the selection pulse), and the resistance badges of six body
//! parts.

use egui::{Pos2, Vec2};

use crate::assets::pipboy_art::{ArtClip, DollPiece, FixedArt};
use crate::rendering::pipboy::paint::faded;
use crate::rendering::pipboy::scaleform_text::{FieldSpec, TextAutoSize};
use crate::rendering::pipboy::summary::{damage_icon, numbered_type, whole};
use crate::rendering::pipboy::tab::TabContext;
use crate::sim::pipboy::inventory::{ApparelStats, DamageValue, InventoryItem, ItemDetails};
use crate::sim::pipboy::{DamageType, Inventory, Limb};
use crate::ui_layout::{
    LabelKey, LiteralKey, PlacementKey, TextKey, UiLayout, UiLayoutError, key_check_fields,
};

use super::native::UnderwearType;

const MOVIE: &str = "Pipboy_InvPage";

/// The body slots the doll tracks, in the order the doll draws them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DollSlot {
    Torso,
    RightArm,
    LeftArm,
    RightLeg,
    LeftLeg,
    Goggles,
    Helmet,
    GasMask,
}

impl DollSlot {
    pub const ALL: [DollSlot; 8] = [
        DollSlot::Torso,
        DollSlot::RightArm,
        DollSlot::LeftArm,
        DollSlot::RightLeg,
        DollSlot::LeftLeg,
        DollSlot::Goggles,
        DollSlot::Helmet,
        DollSlot::GasMask,
    ];

    fn index(self) -> usize {
        self as usize
    }

    fn piece(self) -> DollPiece {
        match self {
            DollSlot::Torso => DollPiece::Torso,
            DollSlot::RightArm => DollPiece::RightArm,
            DollSlot::LeftArm => DollPiece::LeftArm,
            DollSlot::RightLeg => DollPiece::RightLeg,
            DollSlot::LeftLeg => DollPiece::LeftLeg,
            DollSlot::Goggles => DollPiece::Goggles,
            DollSlot::Helmet => DollPiece::Helmet,
            DollSlot::GasMask => DollPiece::GasMask,
        }
    }
}

/// Stand-in for the biped-slot data the game's native code reads from a piece: the slot a limb
/// maps to. The model has no goggles or gas mask slots, so those two never show.
pub fn slot_of(limb: Limb) -> DollSlot {
    match limb {
        Limb::Head => DollSlot::Helmet,
        Limb::Torso => DollSlot::Torso,
        Limb::LeftArm => DollSlot::LeftArm,
        Limb::RightArm => DollSlot::RightArm,
        Limb::LeftLeg => DollSlot::LeftLeg,
        Limb::RightLeg => DollSlot::RightLeg,
    }
}

fn covers(apparel: &ApparelStats, slot: DollSlot) -> bool {
    apparel.covers.iter().any(|limb| slot_of(*limb) == slot)
}

/// The resistances the worn pieces give `slot`, in the order the pieces come.
pub fn slot_resists(inventory: &Inventory, slot: DollSlot) -> Vec<DamageValue> {
    inventory
        .items
        .iter()
        .filter(|item| item.equipped)
        .filter_map(|item| match &item.details {
            ItemDetails::Apparel(apparel) if covers(apparel, slot) => Some(apparel),
            _ => None,
        })
        .flat_map(|apparel| apparel.resistances.iter().copied())
        .collect()
}

/// The resistance of `kind` in `resists`: the last matching entry, not a sum.
pub fn resist_of(resists: &[DamageValue], kind: DamageType) -> f32 {
    resists
        .iter()
        .rfind(|value| value.kind == kind)
        .map_or(0.0, |value| value.amount)
}

/// A slot shows when the selected item covers it or the worn pieces give it a resistance.
pub fn slot_visible(selected: bool, resists: &[DamageValue]) -> bool {
    selected || !resists.is_empty()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Badge {
    Head,
    Torso,
    LeftArm,
    RightArm,
    LeftLeg,
    RightLeg,
}

impl Badge {
    const ALL: [Badge; 6] = [
        Badge::Head,
        Badge::RightArm,
        Badge::LeftArm,
        Badge::RightLeg,
        Badge::LeftLeg,
        Badge::Torso,
    ];

    /// The slots whose resistances the badge adds up; the underwear slot has no item here.
    fn slots(self) -> &'static [DollSlot] {
        match self {
            Badge::Head => &[DollSlot::Helmet, DollSlot::Goggles, DollSlot::GasMask],
            Badge::Torso => &[DollSlot::Torso],
            Badge::LeftArm => &[DollSlot::LeftArm],
            Badge::RightArm => &[DollSlot::RightArm],
            Badge::LeftLeg => &[DollSlot::LeftLeg],
            Badge::RightLeg => &[DollSlot::RightLeg],
        }
    }
}

struct BadgeKeys {
    icon: PlacementKey,
    value: TextKey,
}

key_check_fields!(BadgeKeys { icon, value });

struct SlotKeys {
    track: PlacementKey,
    animate: LabelKey,
}

key_check_fields!(SlotKeys { track, animate });

pub struct DollKeys {
    doll: PlacementKey,
    display_types: [LiteralKey; 3],
    badges: [BadgeKeys; 6],
    slots: [SlotKeys; 8],
}

key_check_fields!(DollKeys {
    doll,
    display_types,
    badges,
    slots
});

macro_rules! badge_keys {
    ($name:literal) => {
        BadgeKeys {
            icon: PlacementKey::new(MOVIE, concat!("PaperDoll_mc/", $name, "/Icon_mc")),
            value: TextKey::new(MOVIE, concat!("PaperDoll_mc/", $name, "/Value_tf")),
        }
    };
}

macro_rules! slot_keys {
    ($class:literal) => {
        SlotKeys {
            track: PlacementKey::new(MOVIE, concat!("[Pipboy_InvPage_fla.", $class, "]/@1")),
            animate: LabelKey::symbol(MOVIE, concat!("Pipboy_InvPage_fla.", $class), "Animate"),
        }
    };
}

pub static DOLL_KEYS: DollKeys = DollKeys {
    doll: PlacementKey::new(MOVIE, "PaperDoll_mc"),
    display_types: [
        LiteralKey::new(MOVIE, "PaperDoll.<init>#new Array(0)", 1),
        LiteralKey::new(MOVIE, "PaperDoll.<init>#new Array(1)", 1),
        LiteralKey::new(MOVIE, "PaperDoll.<init>#new Array(2)", 1),
    ],
    badges: [
        badge_keys!("Head_Resist_mc"),
        badge_keys!("RArm_Resist_mc"),
        badge_keys!("LArm_Resist_mc"),
        badge_keys!("RLeg_Resist_mc"),
        badge_keys!("LLeg_Resist_mc"),
        badge_keys!("Torso_Resist_mc"),
    ],
    slots: [
        slot_keys!("Torso_Anim_7"),
        slot_keys!("Arm_Right_Anim_9"),
        slot_keys!("Arm_Left_Anim_11"),
        slot_keys!("Leg_Right_Anim_13"),
        slot_keys!("Leg_Left_Anim_15"),
        slot_keys!("Goggles_Anim_17"),
        slot_keys!("Helmet_Anim_19"),
        slot_keys!("GasMask_Anim_21"),
    ],
};

/// A slot clip's selection pulse: its alpha on each frame of the loop.
#[derive(Debug, Clone, PartialEq)]
struct Pulse {
    alphas: Vec<f32>,
}

impl Pulse {
    fn resolve(layout: &UiLayout, keys: &SlotKeys) -> Result<Self, UiLayoutError> {
        let first = keys.animate.resolve(layout)?;
        let class = keys.animate.path;
        let frames = layout.movie(MOVIE)?.symbol_timeline(class)?.frame_count;
        let track = layout.get(&keys.track)?;
        let alpha_at = |frame: u32| {
            track
                .track()
                .iter()
                .rfind(|key| key.frame <= frame)
                .map_or(1.0, |key| key.placement.color.mult[3])
        };
        let last = frames.saturating_sub(1).max(first);
        Ok(Self {
            alphas: (first..=last).map(alpha_at).collect(),
        })
    }

    /// The alpha `seconds` after the pulse started, the loop playing at `fps`.
    fn alpha(&self, seconds: f64, fps: f32) -> f32 {
        let frame = (seconds * f64::from(fps)).floor().max(0.0) as usize;
        self.alphas[frame % self.alphas.len()]
    }
}

#[derive(Debug, Clone, Copy)]
struct BadgeLayout {
    icon: Vec2,
    scale: f32,
    value: FieldSpec,
}

/// The doll resolved from the layout.
#[derive(Debug, Clone)]
pub struct DollLayout {
    origin: Vec2,
    scale: f32,
    display_types: [u32; 3],
    badges: [BadgeLayout; 6],
    pulses: [Pulse; 8],
}

impl DollLayout {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let keys = &DOLL_KEYS;
        let doll = layout.get(&keys.doll)?.world.matrix;
        let display = |index: usize| -> Result<u32, UiLayoutError> {
            Ok(layout.lit_number(&keys.display_types[index], 0)? as u32)
        };
        let badge = |index: usize| -> Result<BadgeLayout, UiLayoutError> {
            let icon = layout.get(&keys.badges[index].icon)?.world.matrix;
            Ok(BadgeLayout {
                icon: Vec2::new(icon.tx, icon.ty),
                scale: icon.a,
                value: FieldSpec::resolve(layout, &keys.badges[index].value, TextAutoSize::Shrink)?,
            })
        };
        let pulse = |index: usize| Pulse::resolve(layout, &keys.slots[index]);
        Ok(Self {
            origin: Vec2::new(doll.tx, doll.ty),
            scale: doll.a,
            display_types: [display(0)?, display(1)?, display(2)?],
            badges: [
                badge(0)?,
                badge(1)?,
                badge(2)?,
                badge(3)?,
                badge(4)?,
                badge(5)?,
            ],
            pulses: [
                pulse(0)?,
                pulse(1)?,
                pulse(2)?,
                pulse(3)?,
                pulse(4)?,
                pulse(5)?,
                pulse(6)?,
                pulse(7)?,
            ],
        })
    }

    /// The number of damage types the doll cycles through.
    pub fn cycle_len(&self) -> usize {
        self.display_types.len()
    }

    fn kind(&self, cycle: usize) -> DamageType {
        numbered_type(self.display_types[cycle % self.display_types.len()])
            .unwrap_or(DamageType::Physical)
    }

    fn place(&self, clip: &ArtClip) -> Option<Pos2> {
        let offset = clip.offset.or_else(|| {
            tracing::error!("doll clip {} has no offset in the art manifest", clip.id);
            None
        })?;
        Some((self.origin + Vec2::new(offset[0] * self.scale, offset[1] * self.scale)).to_pos2())
    }
}

fn base_piece(underwear: UnderwearType) -> DollPiece {
    match underwear {
        UnderwearType::Underwear => DollPiece::Body,
        UnderwearType::GenericClothes => DollPiece::Clothes1,
        UnderwearType::VaultSuit => DollPiece::Clothes2,
    }
}

/// What the doll draws for the selected item.
pub struct DollView<'a> {
    pub inventory: &'a Inventory,
    pub selected: &'a InventoryItem,
    pub underwear: UnderwearType,
    /// Which of the cycled damage types the badges show.
    pub cycle: usize,
    /// Seconds since the selection last changed, for the pulse.
    pub pulse_seconds: f64,
}

impl DollLayout {
    pub fn draw(&self, cx: &mut TabContext<'_>, view: &DollView<'_>) {
        let library = cx.art_library;
        let base = library.fixed(FixedArt::Doll(base_piece(view.underwear)));
        if let Some(at) = self.place(base) {
            let tint = base.tint.unwrap_or([1.0; 4]);
            let colour = faded(cx.shade(tint[0]), tint[3]);
            cx.draw_clip(base, 0, at, self.scale, colour);
        }
        let selected_slots: Vec<DollSlot> = match &view.selected.details {
            ItemDetails::Apparel(apparel) => DollSlot::ALL
                .into_iter()
                .filter(|slot| covers(apparel, *slot))
                .collect(),
            _ => Vec::new(),
        };
        for slot in DollSlot::ALL {
            let selected = selected_slots.contains(&slot);
            let resists = slot_resists(view.inventory, slot);
            if !slot_visible(selected, &resists) {
                continue;
            }
            let clip = library.fixed(FixedArt::Doll(slot.piece()));
            let Some(at) = self.place(clip) else {
                continue;
            };
            let alpha = if selected {
                self.pulses[slot.index()].alpha(view.pulse_seconds, clip.fps)
            } else {
                1.0
            };
            let colour = faded(cx.shade(1.0), alpha);
            cx.draw_clip(clip, 0, at, self.scale, colour);
        }
        let kind = self.kind(view.cycle);
        let white = cx.shade(1.0);
        for (badge, layout) in Badge::ALL.into_iter().zip(&self.badges) {
            let total: f32 = badge
                .slots()
                .iter()
                .map(|slot| resist_of(&slot_resists(view.inventory, *slot), kind))
                .sum();
            cx.draw_icon(
                damage_icon(kind),
                layout.icon.to_pos2(),
                layout.scale,
                white,
            );
            layout.value.paint(cx.painter, &whole(total), cx.tint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::pipboy::inventory::{InventoryCategory, ItemId};

    fn worn(
        id: u32,
        covers: Vec<Limb>,
        resistances: Vec<DamageValue>,
        equipped: bool,
    ) -> InventoryItem {
        InventoryItem {
            id: ItemId(id),
            form_key: None,
            name: "x".into(),
            category: InventoryCategory::Apparel,
            count: 1,
            weight: 1.0,
            value: 1,
            equipped,
            favourite: None,
            legendary: false,
            details: ItemDetails::Apparel(ApparelStats {
                resistances,
                covers,
                slot_mask: 1 << id,
            }),
        }
    }

    fn resist(kind: DamageType, amount: f32) -> DamageValue {
        DamageValue { kind, amount }
    }

    #[test]
    fn a_slot_takes_the_last_matching_resistance_not_the_sum() {
        let values = [
            resist(DamageType::Energy, 3.0),
            resist(DamageType::Physical, 2.0),
            resist(DamageType::Energy, 7.0),
        ];
        assert!((resist_of(&values, DamageType::Energy) - 7.0).abs() < 1e-6);
        assert!(resist_of(&values, DamageType::Fire).abs() < 1e-6);
    }

    #[test]
    fn only_worn_pieces_that_cover_a_slot_resist_there() {
        let inventory = Inventory {
            items: vec![
                worn(
                    1,
                    vec![Limb::Torso],
                    vec![resist(DamageType::Physical, 4.0)],
                    true,
                ),
                worn(
                    2,
                    vec![Limb::Torso],
                    vec![resist(DamageType::Physical, 9.0)],
                    false,
                ),
                worn(
                    3,
                    vec![Limb::Head],
                    vec![resist(DamageType::Physical, 1.0)],
                    true,
                ),
            ],
            ..Inventory::default()
        };
        let torso = slot_resists(&inventory, DollSlot::Torso);
        assert_eq!(torso.len(), 1);
        assert!(slot_resists(&inventory, DollSlot::LeftLeg).is_empty());
        assert_eq!(slot_resists(&inventory, DollSlot::Helmet).len(), 1);
    }

    #[test]
    fn a_slot_shows_when_selected_or_resisting() {
        assert!(slot_visible(true, &[]));
        assert!(slot_visible(false, &[resist(DamageType::Fire, 0.0)]));
        assert!(!slot_visible(false, &[]));
    }

    #[test]
    fn the_pulse_loops_over_its_alphas_at_the_frame_rate() {
        let pulse = Pulse {
            alphas: vec![0.5, 0.25, 0.75],
        };
        assert!((pulse.alpha(0.0, 30.0) - 0.5).abs() < 1e-6);
        assert!((pulse.alpha(1.0 / 30.0, 30.0) - 0.25).abs() < 1e-6);
        assert!((pulse.alpha(3.0 / 30.0, 30.0) - 0.5).abs() < 1e-6);
    }
}
