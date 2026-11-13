//! Rolls a leveled item list into the one item stack a placeholder reference stands for.

use std::collections::{HashMap, HashSet};

use crate::assets::db::leveled_items::{LeveledEntryRow, LeveledItemRow};
use crate::random::RandomSource;

#[cfg(test)]
mod tests;

const FLAG_ALL_LEVELS_AT_OR_BELOW_PLAYER: u8 = 1;
const FLAG_USE_ALL: u8 = 4;
const PERCENT: usize = 100;

/// Bit 1 and bit 4. Bit 2 (`CalculateForEachItemInCount`) is accepted by the row reader and not
/// applied: a leaf's count is the stack count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LeveledFlags {
    pub all_levels_at_or_below_player: bool,
    pub use_all: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeveledList {
    pub chance_none: u8,
    pub flags: LeveledFlags,
    pub entries: Vec<LeveledEntryRow>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LeveledLists {
    lists: HashMap<String, LeveledList>,
}

impl LeveledLists {
    pub fn from_rows(rows: Vec<LeveledItemRow>) -> Self {
        let lists = rows
            .into_iter()
            .map(|row| {
                let list = LeveledList {
                    chance_none: row.chance_none,
                    flags: LeveledFlags {
                        all_levels_at_or_below_player: row.flags
                            & FLAG_ALL_LEVELS_AT_OR_BELOW_PLAYER
                            != 0,
                        use_all: row.flags & FLAG_USE_ALL != 0,
                    },
                    entries: row.entries,
                };
                (row.form_key, list)
            })
            .collect();
        Self { lists }
    }

    pub fn get(&self, key: &str) -> Option<&LeveledList> {
        self.lists.get(key)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemStack {
    pub base: String,
    pub count: u32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResolveError {
    #[error("leveled list {0} does not exist")]
    UnknownList(String),
    #[error("leveled list {list} names {target}, which is neither a list nor an item")]
    UnknownEntry { list: String, target: String },
    #[error("leveled list {0} contains itself")]
    Cycle(String),
    #[error("a player level of {0} is not a level the game has")]
    InvalidLevel(u32),
}

/// Resolves the list `root` for a player of `player_level` (at least 1: 0 is `InvalidLevel`);
/// `Ok(None)` means nothing spawns.
///
/// The caller owns the randomness and seeds it; the resolver draws from `rng` in a fixed order:
/// per list the chance none first (always one draw), then the pick (one draw over the candidates,
/// none with `use_all`); nested lists draw the same way, in the order they are entered. The whole
/// tree reachable from `root` is validated before the first draw (unknown list or item target,
/// cycle), so a failure is the same for every reference of a list, whatever the roll.
///
/// Rules (public CK wiki: `falloutck.uesp.net` `LeveledCharacter`, `cs.uesp.net` Leveled Item; unverified
/// against the game):
/// 1. `chance_none` percent: `rng.below(100) < chance_none` ends the list with nothing.
/// 2. Candidates: entries with `level <= player_level`; without
///    `flags.all_levels_at_or_below_player` only those of the highest such level. No candidate:
///    `Ok(None)` (the game's behaviour is unverified; the caller logs it at debug level).
/// 3. `flags.use_all`: the first candidate in record order. Otherwise a uniform pick over the
///    candidates; an entry listed twice weighs twice.
/// 4. A picked target that is a list is resolved recursively and its count multiplied by the
///    entry's (saturating); any other target must satisfy `is_item`.
/// 5. Not applied: bit 2, per-entry chance none, max count, the global chance none, per-entry
///    condition data.
pub fn resolve(
    lists: &LeveledLists,
    is_item: &dyn Fn(&str) -> bool,
    root: &str,
    player_level: u32,
    rng: &mut dyn RandomSource,
) -> Result<Option<ItemStack>, ResolveError> {
    if player_level == 0 {
        return Err(ResolveError::InvalidLevel(player_level));
    }
    validate_list(lists, is_item, root, &mut Vec::new(), &mut HashSet::new())?;
    roll_list(lists, root, player_level, rng)
}

fn validate_list<'a>(
    lists: &'a LeveledLists,
    is_item: &dyn Fn(&str) -> bool,
    key: &'a str,
    path: &mut Vec<&'a str>,
    validated: &mut HashSet<&'a str>,
) -> Result<(), ResolveError> {
    let list = lists
        .get(key)
        .ok_or_else(|| ResolveError::UnknownList(key.to_owned()))?;
    if path.contains(&key) {
        return Err(ResolveError::Cycle(key.to_owned()));
    }
    if validated.contains(key) {
        return Ok(());
    }
    path.push(key);
    for entry in &list.entries {
        if lists.get(&entry.target).is_some() {
            validate_list(lists, is_item, &entry.target, path, validated)?;
        } else if !is_item(&entry.target) {
            return Err(ResolveError::UnknownEntry {
                list: key.to_owned(),
                target: entry.target.clone(),
            });
        }
    }
    path.pop();
    validated.insert(key);
    Ok(())
}

fn roll_list(
    lists: &LeveledLists,
    key: &str,
    player_level: u32,
    rng: &mut dyn RandomSource,
) -> Result<Option<ItemStack>, ResolveError> {
    let list = lists
        .get(key)
        .ok_or_else(|| ResolveError::UnknownList(key.to_owned()))?;
    if rng.below(PERCENT) < usize::from(list.chance_none) {
        return Ok(None);
    }
    let candidates = eligible_entries(list, player_level);
    let entry = match candidates.as_slice() {
        [] => return Ok(None),
        [first, ..] if list.flags.use_all => *first,
        _ => candidates[rng.below(candidates.len())],
    };
    if lists.get(&entry.target).is_none() {
        return Ok(Some(ItemStack {
            base: entry.target.clone(),
            count: entry.count,
        }));
    }
    Ok(
        roll_list(lists, &entry.target, player_level, rng)?.map(|nested| ItemStack {
            count: nested.count.saturating_mul(entry.count),
            ..nested
        }),
    )
}

fn eligible_entries(list: &LeveledList, player_level: u32) -> Vec<&LeveledEntryRow> {
    let eligible = list
        .entries
        .iter()
        .filter(|entry| entry.level <= player_level);
    if list.flags.all_levels_at_or_below_player {
        return eligible.collect();
    }
    let eligible: Vec<_> = eligible.collect();
    let highest = eligible.iter().map(|entry| entry.level).max();
    eligible
        .into_iter()
        .filter(|entry| Some(entry.level) == highest)
        .collect()
}
