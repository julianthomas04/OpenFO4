//! Quest log entries shown on DATA/QUESTS.

use glam::Vec3;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct QuestId(pub String);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestStatus {
    Active,
    Completed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuestObjective {
    pub text: String,
    pub completed: bool,
    /// Engine-space point the objective shows on the map.
    pub target: Option<Vec3>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Quest {
    pub id: QuestId,
    pub name: String,
    pub description: String,
    pub status: QuestStatus,
    pub tracked: bool,
    pub objectives: Vec<QuestObjective>,
    /// Raw `Interface`-relative path of the quest's Vault Boy movie.
    pub vault_boy_swf: Option<String>,
}
