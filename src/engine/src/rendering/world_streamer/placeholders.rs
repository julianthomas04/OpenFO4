//! Leveled-item placeholders: placed references with a leveled link, which stand for a roll of
//! their list, and references on a marker item base.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use tracing::{debug, warn};

use crate::assets::db::DbClient;
use crate::assets::db::leveled_items::LeveledItemRow;
use crate::assets::db::placed_objects::PlacedObject;
use crate::leveled_items::{ItemStack, LeveledLists, resolve};
use crate::random::SplitMix64;
use crate::sim::pipboy::character::STARTING_LEVEL;
use crate::sim::{EntityId, refr_entity_id};
use crate::utils::warn_once;

/// The `leveled_items` table is missing or malformed: a database converted before leveled item
/// lists existed lacks it and must be reconverted.
#[derive(Debug, thiserror::Error)]
#[error("the database's leveled_items table cannot be read: {0:#}")]
pub(super) struct LeveledItemsUnreadable(anyhow::Error);

/// Every leveled item list of the database; an empty table is not an error here.
pub(super) fn read_leveled_lists(
    db: &DbClient,
) -> Result<Vec<LeveledItemRow>, LeveledItemsUnreadable> {
    db.get_leveled_items().map_err(LeveledItemsUnreadable)
}

/// What a placed reference stands for in the cell being assembled.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Effective {
    /// No leveled link on a real base: assembled with its own base and count.
    Own,
    /// A linked reference whose list rolled this item, assembled at the reference's transform.
    Resolved(ItemStack),
    /// Nothing spawns: a marker base without a link, or a link without a ref key, without lists,
    /// with an empty roll or with a list that does not resolve.
    Vacant,
}

impl Effective {
    /// The base and stack count the reference assembles with; `None` when nothing spawns. A
    /// resolved stack is multiplied by the reference's own count when it has one.
    pub(super) fn item_entry<'a>(
        &'a self,
        obj: &PlacedObject,
        own_base: &'a str,
    ) -> Option<(&'a str, Option<i64>)> {
        match self {
            Self::Own => Some((own_base, obj.item_count)),
            Self::Resolved(stack) => Some((
                stack.base.as_str(),
                Some(i64::from(stack.count).saturating_mul(obj.item_count.unwrap_or(1))),
            )),
            Self::Vacant => None,
        }
    }
}

/// The marker item bases, the leveled lists linked references roll, and the items those rolls
/// produced this session.
pub(super) struct Placeholders {
    marker_bases: HashSet<String>,
    /// `None` when the database holds no leveled lists: every linked reference spawns nothing.
    lists: Option<LeveledLists>,
    /// Ref key -> the item its roll produced. A roll that produced an item sticks to the
    /// reference for the session, across cell loads and worldspace switches; one that produced
    /// nothing is not kept, so the reference rolls again at its next cell load. Bounded by the
    /// number of linked references (1,930 in the vanilla data), in memory only.
    kept: Mutex<HashMap<String, ItemStack>>,
}

impl Placeholders {
    /// `marker_bases` are the `items` rows flagged `is_marker`. Without any leveled list row a linked
    /// reference can resolve to nothing, which is said once here.
    pub(super) fn new(marker_bases: HashSet<String>, rows: Vec<LeveledItemRow>) -> Self {
        let lists = if rows.is_empty() {
            warn!(
                "the database holds no leveled item lists: references with a leveled link \
                 spawn nothing; reconvert the database"
            );
            None
        } else {
            Some(LeveledLists::from_rows(rows))
        };
        Self {
            marker_bases,
            lists,
            kept: Mutex::new(HashMap::new()),
        }
    }

    /// Every reference with a leveled link stands for its list's roll, whatever its base (the
    /// original shows a linked real item as the roll's item): the item it rolled earlier this
    /// session, else a roll of this cell load, kept when it yields an item. Without a link a
    /// reference on a marker item base spawns nothing and any other keeps its own base.
    /// `is_item` tells a resolvable item base.
    ///
    /// The lock is held only to look up and to store: resolving and logging happen outside it,
    /// and when two workers resolve the same reference at once the first stored result wins for
    /// both.
    pub(super) fn effective(
        &self,
        obj: &PlacedObject,
        roll: &CellRoll,
        is_item: &dyn Fn(&str) -> bool,
    ) -> Effective {
        let Some(list) = obj.leveled_item.as_deref() else {
            return self.unlinked(obj);
        };
        let (Some(ref_key), Some(lists)) = (obj.form_key.as_deref(), &self.lists) else {
            return Effective::Vacant;
        };
        if let Some(stack) = self.kept_stack(ref_key) {
            debug!("{}", leveled_log_line(ref_key, list, Leveled::Kept(&stack)));
            return Effective::Resolved(stack);
        }
        let mut rng = SplitMix64::new(roll_seed(roll.load_nonce, refr_entity_id(ref_key)));
        match resolve(lists, is_item, list, roll.player_level, &mut rng) {
            Ok(Some(stack)) => {
                let stack = self.keep(ref_key, stack);
                debug!(
                    "{}",
                    leveled_log_line(ref_key, list, Leveled::Rolled(&stack))
                );
                Effective::Resolved(stack)
            }
            Ok(None) => {
                debug!("{}", leveled_log_line(ref_key, list, Leveled::Nothing));
                Effective::Vacant
            }
            Err(error) => {
                warn_once(&format!("leveled-list {list}"), || {
                    format!("references linked to leveled list {list} spawn nothing: {error}")
                });
                Effective::Vacant
            }
        }
    }

    /// What a reference stands for at reduced detail, which rolls nothing: a linked reference
    /// draws the item it kept at full detail, else nothing (never its own base), so both detail
    /// levels agree.
    pub(super) fn at_reduced_detail(&self, obj: &PlacedObject) -> Effective {
        if obj.leveled_item.is_none() {
            return self.unlinked(obj);
        }
        obj.form_key
            .as_deref()
            .and_then(|ref_key| self.kept_stack(ref_key))
            .map_or(Effective::Vacant, Effective::Resolved)
    }

    fn unlinked(&self, obj: &PlacedObject) -> Effective {
        let on_marker = obj
            .base_object
            .as_deref()
            .is_some_and(|base| self.marker_bases.contains(base));
        if on_marker {
            Effective::Vacant
        } else {
            Effective::Own
        }
    }

    fn kept_stack(&self, ref_key: &str) -> Option<ItemStack> {
        self.kept().get(ref_key).cloned()
    }

    /// Stores `stack` unless another worker stored one first; returns the stored one.
    fn keep(&self, ref_key: &str, stack: ItemStack) -> ItemStack {
        self.kept()
            .entry(ref_key.to_owned())
            .or_insert(stack)
            .clone()
    }

    fn kept(&self) -> MutexGuard<'_, HashMap<String, ItemStack>> {
        self.kept.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// How a linked reference's list resolved in one cell load.
#[derive(Clone, Copy)]
enum Leveled<'a> {
    Nothing,
    Kept(&'a ItemStack),
    Rolled(&'a ItemStack),
}

fn leveled_log_line(ref_key: &str, list: &str, outcome: Leveled<'_>) -> String {
    let (stack, how) = match outcome {
        Leveled::Nothing => return format!("leveled: ref={ref_key} list={list} -> nothing"),
        Leveled::Kept(stack) => (stack, "kept"),
        Leveled::Rolled(stack) => (stack, "rolled"),
    };
    format!(
        "leveled: ref={ref_key} list={list} -> {} x {} ({how})",
        stack.base, stack.count
    )
}

/// The inputs every linked reference of one cell load rolls with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CellRoll {
    pub player_level: u32,
    /// Distinct for every load of every cell, in every session.
    pub load_nonce: u64,
}

/// The state the workers draw a cell's roll from: the player level the main thread publishes and
/// a count of cell loads.
pub(super) struct RollState {
    player_level: AtomicU32,
    session_seed: u64,
    loads: AtomicU64,
}

impl RollState {
    /// `session_seed` differs between sessions; the level starts at the game's starting level.
    pub(super) fn new(session_seed: u64) -> Self {
        Self {
            player_level: AtomicU32::new(STARTING_LEVEL),
            session_seed,
            loads: AtomicU64::new(0),
        }
    }

    /// Takes effect for cells loaded afterwards.
    pub(super) fn set_player_level(&self, level: u32) {
        self.player_level.store(level, Ordering::Relaxed);
    }

    /// The roll of one cell load; called once per full-detail assembly.
    pub(super) fn next_cell(&self) -> CellRoll {
        let load = self.loads.fetch_add(1, Ordering::Relaxed);
        CellRoll {
            player_level: self.player_level.load(Ordering::Relaxed),
            load_nonce: mixed(&(self.session_seed, load)),
        }
    }
}

/// The seed of one reference's roll in one cell load. Hashed, so neighbouring loads and
/// neighbouring entities share no structure.
pub(super) fn roll_seed(load_nonce: u64, entity: EntityId) -> u64 {
    mixed(&(load_nonce, entity.0))
}

fn mixed(value: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;
    use std::thread;

    use glam::Vec3;

    use crate::assets::db::leveled_items::LeveledEntryRow;
    use crate::assets::db::test_support::database_with;

    use super::super::world_edits::{CellEdits, placed_item, take_item};
    use super::*;

    const CRATE_MARKER: &str = "PH_CRATE";
    const PLAIN_ITEM: &str = "PLAIN_CAN";
    const ROLLED_ITEM: &str = "ROLLED_CAN";
    const LIST: &str = "LL_FIXTURE";
    const ROLL: CellRoll = CellRoll {
        player_level: 1,
        load_nonce: 7,
    };

    fn list(key: &str, chance_none: u8, entries: &[(&str, u32, u32)]) -> LeveledItemRow {
        LeveledItemRow {
            form_key: key.to_owned(),
            editor_id: None,
            chance_none,
            flags: 0,
            entries: entries
                .iter()
                .map(|&(target, level, count)| LeveledEntryRow {
                    target: target.to_owned(),
                    level,
                    count,
                })
                .collect(),
        }
    }

    fn placeholders(rows: Vec<LeveledItemRow>) -> Placeholders {
        Placeholders::new(HashSet::from([CRATE_MARKER.to_owned()]), rows)
    }

    fn one_item_list() -> Vec<LeveledItemRow> {
        vec![list(LIST, 0, &[(ROLLED_ITEM, 1, 3)])]
    }

    fn reference(base: &str, link: Option<&str>, count: Option<i64>) -> PlacedObject {
        let mut obj = PlacedObject::runtime(
            "00ABCD:Fixture.esm".to_owned(),
            base.to_owned(),
            [0.0; 3],
            0.0,
            1,
        );
        obj.leveled_item = link.map(str::to_owned);
        obj.item_count = count;
        obj
    }

    fn is_item(key: &str) -> bool {
        matches!(key, PLAIN_ITEM | ROLLED_ITEM)
    }

    fn effective_of(placeholders: &Placeholders, obj: &PlacedObject) -> Effective {
        placeholders.effective(obj, &ROLL, &is_item)
    }

    fn rolled(count: u32) -> Effective {
        Effective::Resolved(ItemStack {
            base: ROLLED_ITEM.to_owned(),
            count,
        })
    }

    #[test]
    fn each_resolution_of_a_linked_reference_logs_one_plain_line() {
        let stack = ItemStack {
            base: ROLLED_ITEM.to_owned(),
            count: 3,
        };
        let line = |outcome| leveled_log_line("00ABCD:Fixture.esm", LIST, outcome);
        assert_eq!(
            line(Leveled::Nothing),
            "leveled: ref=00ABCD:Fixture.esm list=LL_FIXTURE -> nothing"
        );
        assert_eq!(
            line(Leveled::Rolled(&stack)),
            "leveled: ref=00ABCD:Fixture.esm list=LL_FIXTURE -> ROLLED_CAN x 3 (rolled)"
        );
        assert_eq!(
            line(Leveled::Kept(&stack)),
            "leveled: ref=00ABCD:Fixture.esm list=LL_FIXTURE -> ROLLED_CAN x 3 (kept)"
        );
    }

    #[test]
    fn a_linked_reference_on_a_marker_base_rolls_its_list() {
        let obj = reference(CRATE_MARKER, Some(LIST), None);

        assert_eq!(
            effective_of(&placeholders(one_item_list()), &obj),
            rolled(3)
        );
    }

    #[test]
    fn a_linked_reference_on_a_real_item_rolls_its_list_instead_of_its_base() {
        let obj = reference(PLAIN_ITEM, Some(LIST), None);

        assert_eq!(
            effective_of(&placeholders(one_item_list()), &obj),
            rolled(3)
        );
    }

    #[test]
    fn a_reference_on_a_real_item_without_a_link_keeps_its_own_base() {
        let obj = reference(PLAIN_ITEM, None, Some(2));

        assert_eq!(
            effective_of(&placeholders(one_item_list()), &obj),
            Effective::Own
        );
    }

    #[test]
    fn a_linked_reference_on_a_real_item_whose_roll_is_empty_spawns_nothing() {
        let rows = vec![list(LIST, 100, &[(ROLLED_ITEM, 1, 1)])];
        let obj = reference(PLAIN_ITEM, Some(LIST), None);

        assert_eq!(effective_of(&placeholders(rows), &obj), Effective::Vacant);
    }

    #[test]
    fn a_marker_reference_without_a_link_spawns_nothing() {
        let obj = reference(CRATE_MARKER, None, None);

        assert_eq!(
            effective_of(&placeholders(one_item_list()), &obj),
            Effective::Vacant
        );
    }

    #[test]
    fn a_marker_reference_without_a_ref_key_spawns_nothing() {
        let mut obj = reference(CRATE_MARKER, Some(LIST), None);
        obj.form_key = None;

        assert_eq!(
            effective_of(&placeholders(one_item_list()), &obj),
            Effective::Vacant
        );
    }

    #[test]
    fn without_leveled_lists_every_placeholder_spawns_nothing() {
        let obj = reference(CRATE_MARKER, Some(LIST), None);

        assert_eq!(
            effective_of(&placeholders(Vec::new()), &obj),
            Effective::Vacant
        );
    }

    #[test]
    fn a_list_with_no_entry_at_the_player_level_spawns_nothing() {
        let rows = vec![list(LIST, 0, &[(ROLLED_ITEM, 9, 1)])];
        let obj = reference(CRATE_MARKER, Some(LIST), None);

        assert_eq!(effective_of(&placeholders(rows), &obj), Effective::Vacant);
    }

    #[test]
    fn a_list_that_always_rolls_none_spawns_nothing() {
        let rows = vec![list(LIST, 100, &[(ROLLED_ITEM, 1, 1)])];
        let obj = reference(CRATE_MARKER, Some(LIST), None);

        assert_eq!(effective_of(&placeholders(rows), &obj), Effective::Vacant);
    }

    #[test]
    fn a_list_naming_a_base_that_is_no_item_spawns_nothing() {
        let rows = vec![list(LIST, 0, &[(CRATE_MARKER, 1, 1)])];
        let obj = reference(CRATE_MARKER, Some(LIST), None);

        assert_eq!(effective_of(&placeholders(rows), &obj), Effective::Vacant);
    }

    #[test]
    fn a_resolved_entry_takes_the_rolled_base_and_multiplies_the_placeholder_count() {
        let obj = reference(CRATE_MARKER, Some(LIST), Some(4));

        assert_eq!(
            rolled(3).item_entry(&obj, CRATE_MARKER),
            Some((ROLLED_ITEM, Some(12)))
        );
    }

    #[test]
    fn a_resolved_entry_of_a_placeholder_without_a_count_takes_the_stack_count() {
        let obj = reference(CRATE_MARKER, Some(LIST), None);

        assert_eq!(
            rolled(3).item_entry(&obj, CRATE_MARKER),
            Some((ROLLED_ITEM, Some(3)))
        );
    }

    #[test]
    fn an_own_entry_keeps_the_reference_base_and_count_and_a_vacant_one_has_none() {
        let obj = reference(PLAIN_ITEM, None, Some(2));

        assert_eq!(
            Effective::Own.item_entry(&obj, PLAIN_ITEM),
            Some((PLAIN_ITEM, Some(2)))
        );
        assert_eq!(Effective::Vacant.item_entry(&obj, PLAIN_ITEM), None);
    }

    fn load(load_nonce: u64, player_level: u32) -> CellRoll {
        CellRoll {
            player_level,
            load_nonce,
        }
    }

    #[test]
    fn an_item_a_reference_rolled_stays_with_it_on_a_reload_with_another_nonce() {
        let placeholders = placeholders(vec![list(
            LIST,
            0,
            &[(ROLLED_ITEM, 1, 1), (PLAIN_ITEM, 1, 1)],
        )]);
        let obj = reference(CRATE_MARKER, Some(LIST), None);
        let first = placeholders.effective(&obj, &load(1, 1), &is_item);

        let reloads: Vec<Effective> = (2..40)
            .map(|nonce| placeholders.effective(&obj, &load(nonce, 1), &is_item))
            .collect();

        assert!(matches!(first, Effective::Resolved(_)));
        assert!(reloads.iter().all(|reload| *reload == first));
    }

    #[test]
    fn a_reference_that_rolled_nothing_rolls_again_and_can_come_up_with_an_item() {
        let placeholders = placeholders(vec![list(LIST, 50, &[(ROLLED_ITEM, 1, 2)])]);
        let empty_first = (0..32)
            .map(|id| {
                let mut obj = reference(CRATE_MARKER, Some(LIST), None);
                obj.form_key = Some(format!("{id:06X}:Fixture.esm"));
                obj
            })
            .find(|obj| placeholders.effective(obj, &load(1, 1), &is_item) == Effective::Vacant)
            .expect("a list with half a chance of nothing rolls nothing for one of 32 references");

        let results: Vec<Effective> = (2..64)
            .map(|nonce| placeholders.effective(&empty_first, &load(nonce, 1), &is_item))
            .collect();

        let first_item = results
            .iter()
            .position(|result| *result != Effective::Vacant)
            .expect("a list with half a chance of an item yields one within 62 loads");
        assert!(
            results[first_item..]
                .iter()
                .all(|result| *result == rolled(2))
        );
    }

    #[test]
    fn a_kept_item_stands_even_after_the_player_level_changed() {
        let placeholders = placeholders(vec![list(
            LIST,
            0,
            &[(ROLLED_ITEM, 1, 1), (PLAIN_ITEM, 5, 1)],
        )]);
        let obj = reference(CRATE_MARKER, Some(LIST), None);
        let at_level_one = placeholders.effective(&obj, &load(1, 1), &is_item);

        let at_level_five = placeholders.effective(&obj, &load(2, 5), &is_item);

        assert_eq!(at_level_one, rolled(1));
        assert_eq!(at_level_five, rolled(1));
    }

    #[test]
    fn a_resolved_item_that_was_taken_returns_with_its_cell_after_an_unload_and_reload() {
        let placeholders = placeholders(vec![list(
            LIST,
            0,
            &[(ROLLED_ITEM, 1, 3), (PLAIN_ITEM, 1, 2)],
        )]);
        let obj = reference(CRATE_MARKER, Some(LIST), None);
        let ref_key = obj.form_key.as_deref().unwrap();
        let entity = refr_entity_id(ref_key);
        let Effective::Resolved(stack) = placeholders.effective(&obj, &load(1, 1), &is_item) else {
            panic!("the list always rolls an item");
        };
        let mut items = HashMap::from([(
            entity,
            placed_item(
                entity,
                &stack.base,
                Some(i64::from(stack.count)),
                None,
                Vec3::ZERO,
            ),
        )]);
        let mut edits = CellEdits::default();

        take_item(&mut items, &mut edits, entity).expect("the rolled item stands in its cell");

        assert!(items.is_empty() && edits.removed.contains(&entity));
        let reloaded_edits = CellEdits::default();
        let after_reload = placeholders.effective(&obj, &load(2, 1), &is_item);
        assert!(reloaded_edits.removed.is_empty());
        assert_eq!(after_reload, Effective::Resolved(stack));
    }

    #[test]
    fn a_linked_reference_draws_its_kept_item_at_reduced_detail() {
        let placeholders = placeholders(one_item_list());
        let obj = reference(PLAIN_ITEM, Some(LIST), None);
        placeholders.effective(&obj, &ROLL, &is_item);

        assert_eq!(placeholders.at_reduced_detail(&obj), rolled(3));
    }

    #[test]
    fn a_linked_reference_that_kept_nothing_draws_nothing_at_reduced_detail_not_its_base() {
        let placeholders = placeholders(one_item_list());
        let obj = reference(PLAIN_ITEM, Some(LIST), None);

        assert_eq!(placeholders.at_reduced_detail(&obj), Effective::Vacant);
    }

    #[test]
    fn reduced_detail_rolls_nothing_so_a_later_full_detail_load_still_rolls() {
        let placeholders = placeholders(one_item_list());
        let obj = reference(PLAIN_ITEM, Some(LIST), None);
        placeholders.at_reduced_detail(&obj);

        assert_eq!(effective_of(&placeholders, &obj), rolled(3));
    }

    #[test]
    fn an_unlinked_reference_is_the_same_at_reduced_detail_as_at_full_detail() {
        let placeholders = placeholders(one_item_list());
        let plain = reference(PLAIN_ITEM, None, Some(2));
        let marker = reference(CRATE_MARKER, None, None);

        assert_eq!(placeholders.at_reduced_detail(&plain), Effective::Own);
        assert_eq!(placeholders.at_reduced_detail(&marker), Effective::Vacant);
    }

    #[test]
    fn a_roll_resolves_without_holding_the_lock_of_the_kept_items() {
        let placeholders = placeholders(one_item_list());
        let obj = reference(CRATE_MARKER, Some(LIST), None);
        let lock_is_free = |key: &str| {
            assert!(
                placeholders.kept.try_lock().is_ok(),
                "the kept items stay locked while a list resolves"
            );
            is_item(key)
        };

        assert_eq!(
            placeholders.effective(&obj, &ROLL, &lock_is_free),
            rolled(3)
        );
    }

    #[test]
    fn workers_rolling_one_reference_at_once_end_with_one_kept_item() {
        let rows = vec![list(
            LIST,
            0,
            &(0..16).map(|n| (ROLLED_ITEM, 1, n + 1)).collect::<Vec<_>>(),
        )];
        let obj = reference(CRATE_MARKER, Some(LIST), None);
        for _ in 0..50 {
            let placeholders = placeholders(rows.clone());
            let start = Barrier::new(2);

            let results: Vec<Effective> = thread::scope(|scope| {
                let workers: Vec<_> = (1..=2)
                    .map(|nonce| {
                        let (placeholders, obj, start) = (&placeholders, &obj, &start);
                        scope.spawn(move || {
                            start.wait();
                            placeholders.effective(obj, &load(nonce, 1), &is_item)
                        })
                    })
                    .collect();
                workers.into_iter().map(|w| w.join().unwrap()).collect()
            });

            assert_eq!(results[0], results[1]);
            assert_eq!(effective_of(&placeholders, &obj), results[0]);
        }
    }

    #[test]
    fn a_database_without_the_leveled_items_table_fails_with_the_table_named() {
        let db = database_with("DROP TABLE leveled_items");

        let error = read_leveled_lists(&db).unwrap_err();

        assert!(error.to_string().contains("leveled_items"), "{error}");
    }

    #[test]
    fn an_empty_leveled_items_table_reads_as_no_lists() {
        let db = database_with("");

        assert_eq!(read_leveled_lists(&db).unwrap(), Vec::new());
    }

    #[test]
    fn a_roll_seed_is_fixed_by_its_load_and_entity() {
        assert_eq!(roll_seed(7, EntityId(40)), roll_seed(7, EntityId(40)));
    }

    #[test]
    fn neighbouring_loads_and_entities_get_unrelated_seeds() {
        let seeds = [
            roll_seed(7, EntityId(40)),
            roll_seed(8, EntityId(40)),
            roll_seed(7, EntityId(41)),
            roll_seed(8, EntityId(41)),
            roll_seed(40, EntityId(7)),
        ];

        assert_eq!(seeds.iter().collect::<HashSet<_>>().len(), seeds.len());
        assert_ne!(
            seeds[1].wrapping_sub(seeds[0]),
            seeds[3].wrapping_sub(seeds[2]),
            "a step in the load must not shift every entity's seed alike"
        );
    }

    #[test]
    fn every_cell_load_gets_its_own_nonce_and_every_session_its_own_sequence() {
        let session = RollState::new(5);
        let other_session = RollState::new(6);

        let first = session.next_cell().load_nonce;
        let second = session.next_cell().load_nonce;
        let other_first = other_session.next_cell().load_nonce;

        assert_ne!(first, second);
        assert_ne!(first, other_first);
        assert_eq!(RollState::new(5).next_cell().load_nonce, first);
    }

    #[test]
    fn a_cell_rolls_at_the_starting_level_until_the_level_is_published() {
        let state = RollState::new(5);
        assert_eq!(state.next_cell().player_level, STARTING_LEVEL);

        state.set_player_level(12);

        assert_eq!(state.next_cell().player_level, 12);
    }
}
