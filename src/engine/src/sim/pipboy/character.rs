//! Player character state shown on STAT: SPECIAL, limb condition, progression and active effects.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Sex {
    #[default]
    Male,
    Female,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Special {
    Strength,
    Perception,
    Endurance,
    Charisma,
    Intelligence,
    Agility,
    Luck,
}

impl Special {
    pub const ALL: [Special; 7] = [
        Special::Strength,
        Special::Perception,
        Special::Endurance,
        Special::Charisma,
        Special::Intelligence,
        Special::Agility,
        Special::Luck,
    ];
}

/// The authored name and description of one SPECIAL attribute; empty when the data lacks it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpecialText {
    pub name: String,
    pub description: String,
}

/// The texts of all seven SPECIAL attributes, indexed by [`Special`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpecialTexts(pub [SpecialText; 7]);

impl SpecialTexts {
    pub fn get(&self, special: Special) -> &SpecialText {
        &self.0[special as usize]
    }
}

/// One SPECIAL attribute: the permanent base rank and the temporary bonus from gear and effects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpecialValue {
    pub base: u8,
    pub bonus: i8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpecialStats(pub [SpecialValue; 7]);

impl SpecialStats {
    pub fn get(&self, special: Special) -> SpecialValue {
        self.0[special as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Limb {
    Head,
    Torso,
    LeftArm,
    RightArm,
    LeftLeg,
    RightLeg,
}

impl Limb {
    pub const ALL: [Limb; 6] = [
        Limb::Head,
        Limb::Torso,
        Limb::LeftArm,
        Limb::RightArm,
        Limb::LeftLeg,
        Limb::RightLeg,
    ];
}

/// Condition per limb as a fraction in `0..=1`; `0` is crippled.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LimbConditions(pub [f32; 6]);

impl Default for LimbConditions {
    fn default() -> Self {
        Self([1.0; 6])
    }
}

impl LimbConditions {
    pub fn get(&self, limb: Limb) -> f32 {
        self.0[limb as usize]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progression {
    pub level: u32,
    /// Experience earned inside the current level.
    pub xp_into_level: u32,
    /// Experience the current level needs in total before the next level.
    pub xp_for_next_level: u32,
    pub unspent_perk_points: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActiveEffect {
    /// What applied the effect (an item, perk or condition name).
    pub source: String,
    pub description: String,
    pub remaining_seconds: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Character {
    pub name: String,
    pub sex: Sex,
    pub special: SpecialStats,
    pub limbs: LimbConditions,
    pub progression: Progression,
    pub effects: Vec<ActiveEffect>,
    pub rads: f32,
}
