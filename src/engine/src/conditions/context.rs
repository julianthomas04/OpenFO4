//! The state a condition reads: the context trait, the new-game context and the world clock.

use std::collections::HashMap;
use std::hash::BuildHasher;

pub const GAME_HOUR_EDITOR_ID: &str = "GameHour";
pub const GAME_DAYS_PASSED_EDITOR_ID: &str = "GameDaysPassed";

/// The world clock as the condition layer reads it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GameTime {
    /// The `GameHour` global: the hour of the day with the minutes as a fraction.
    pub hour: f32,
    /// The `GameDaysPassed` global: whole days passed plus the elapsed fraction of today.
    pub days_passed: f32,
}

impl GameTime {
    pub fn new(hour: f32, whole_days_passed: u32) -> Self {
        Self {
            hour,
            days_passed: whole_days_passed as f32 + hour / HOURS_PER_DAY,
        }
    }

    pub fn at_hour(hour: f32) -> Self {
        Self::new(hour, 0)
    }
}

const HOURS_PER_DAY: f32 = 24.0;

#[derive(Debug, Clone, Copy)]
enum LiveGlobal {
    Hour,
    DaysPassed,
}

pub trait ConditionContext {
    fn game_hour(&self) -> f32;
    fn global(&self, key: &str) -> Option<f32>;
    fn quest_running(&self, quest: &str) -> bool;
    fn quest_stage(&self, quest: &str) -> i32;
    fn stage_done(&self, quest: &str, stage: i32) -> bool;
    fn vm_quest_variable(&self, quest: &str, variable: &str) -> f32;
    fn is_first_person(&self) -> bool;
    fn has_silent_running_perk(&self) -> bool;
    /// Whether the player is in an interior cell; `None` where the context does not know, which
    /// leaves the function unsupported rather than guessed.
    fn is_in_interior(&self) -> Option<bool>;
    /// A fresh whole-number roll from 0 to 99 per call, as the GECK documents `GetRandomPercent`;
    /// `None` where the context has no random source.
    fn random_percent(&self) -> Option<f32> {
        None
    }
    /// Whether the player is in the worldspace with this form key; `None` where unknown.
    fn in_worldspace(&self, _worldspace: &str) -> Option<bool> {
        None
    }
    /// Whether the player is in the cell with this form key; `None` where unknown.
    fn in_cell(&self, _cell: &str) -> Option<bool> {
        None
    }
    /// Whether the player's current location is this location or one of its descendants; `None`
    /// where unknown.
    fn in_current_location(&self, _location: &str) -> Option<bool> {
        None
    }
    /// Whether the quest is completed; `None` where the context has no quest state.
    fn quest_completed(&self, _quest: &str) -> Option<bool> {
        None
    }
}

#[derive(Debug, Clone)]
pub struct GlobalEntry {
    pub key: String,
    pub editor_id: Option<String>,
    pub value: f32,
}

/// Every global with its editor id where it has one; `form_keys` maps editor ids to form keys.
pub fn global_entries<V: BuildHasher, K: BuildHasher>(
    values: &HashMap<String, f32, V>,
    form_keys: &HashMap<String, String, K>,
) -> Vec<GlobalEntry> {
    let editor_ids: HashMap<&str, &str> = form_keys
        .iter()
        .map(|(editor_id, key)| (key.as_str(), editor_id.as_str()))
        .collect();
    values
        .iter()
        .map(|(key, &value)| GlobalEntry {
            key: key.clone(),
            editor_id: editor_ids.get(key.as_str()).map(ToString::to_string),
            value,
        })
        .collect()
}

/// A new game before any quest started; quest state is empty and `GameHour` follows the clock.
/// Where the player is stays unknown until [`set_interior`](Self::set_interior) tells it, and the
/// interior function is unsupported until then.
#[derive(Debug, Clone)]
pub struct FreshWorldContext {
    time: GameTime,
    interior: Option<bool>,
    globals: HashMap<String, f32>,
    live_globals: HashMap<String, LiveGlobal>,
}

impl FreshWorldContext {
    /// Globals are reachable by form key and by editor id (both case-insensitive). Every global whose
    /// editor id is `GameHour` or `GameDaysPassed` is answered from the clock instead of its ESM
    /// value. The clock starts at `hour` with no days passed; [`set_time`](Self::set_time) follows it.
    pub fn new(hour: f32, globals: impl IntoIterator<Item = GlobalEntry>) -> Self {
        let mut map = HashMap::new();
        let mut live_globals = HashMap::new();
        for entry in globals {
            let key = entry.key.to_ascii_lowercase();
            map.insert(key.clone(), entry.value);
            if let Some(editor_id) = entry.editor_id {
                let editor_id = editor_id.to_ascii_lowercase();
                let live = [
                    (GAME_HOUR_EDITOR_ID, LiveGlobal::Hour),
                    (GAME_DAYS_PASSED_EDITOR_ID, LiveGlobal::DaysPassed),
                ]
                .into_iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(&editor_id));
                if let Some((_, live)) = live {
                    live_globals.insert(key, live);
                    live_globals.insert(editor_id.clone(), live);
                }
                map.insert(editor_id, entry.value);
            }
        }
        Self {
            time: GameTime::at_hour(hour),
            interior: None,
            globals: map,
            live_globals,
        }
    }
}

impl FreshWorldContext {
    pub fn set_time(&mut self, time: GameTime) {
        self.time = time;
    }

    /// Whether the player is in an interior cell, which `IS_IN_INTERIOR` reports.
    pub fn set_interior(&mut self, interior: bool) {
        self.interior = Some(interior);
    }
}

impl ConditionContext for FreshWorldContext {
    fn game_hour(&self) -> f32 {
        self.time.hour
    }

    fn global(&self, key: &str) -> Option<f32> {
        let key = key.to_ascii_lowercase();
        match self.live_globals.get(&key) {
            Some(LiveGlobal::Hour) => Some(self.time.hour),
            Some(LiveGlobal::DaysPassed) => Some(self.time.days_passed),
            None => self.globals.get(&key).copied(),
        }
    }

    fn quest_running(&self, _quest: &str) -> bool {
        false
    }

    fn quest_stage(&self, _quest: &str) -> i32 {
        0
    }

    fn stage_done(&self, _quest: &str, _stage: i32) -> bool {
        false
    }

    fn vm_quest_variable(&self, _quest: &str, _variable: &str) -> f32 {
        0.0
    }

    fn is_first_person(&self) -> bool {
        true
    }

    fn has_silent_running_perk(&self) -> bool {
        false
    }

    fn is_in_interior(&self) -> Option<bool> {
        self.interior
    }

    fn quest_completed(&self, _quest: &str) -> Option<bool> {
        Some(false)
    }
}
