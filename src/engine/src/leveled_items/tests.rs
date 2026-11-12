use super::*;
use crate::random::SplitMix64;
use crate::random::test_support::ScriptedRandom;

const ALL_LEVELS: u8 = 1;
const FOR_EACH_IN_COUNT: u8 = 2;
const USE_ALL: u8 = 4;

fn entry(target: &str, level: u32, count: u32) -> LeveledEntryRow {
    LeveledEntryRow {
        target: target.to_owned(),
        level,
        count,
    }
}

fn row(key: &str, chance_none: u8, flags: u8, entries: Vec<LeveledEntryRow>) -> LeveledItemRow {
    LeveledItemRow {
        form_key: key.to_owned(),
        editor_id: None,
        chance_none,
        flags,
        entries,
    }
}

fn lists(rows: Vec<LeveledItemRow>) -> LeveledLists {
    LeveledLists::from_rows(rows)
}

fn is_item(key: &str) -> bool {
    matches!(key, "a" | "b" | "c" | "d")
}

fn stack(base: &str, count: u32) -> Option<ItemStack> {
    Some(ItemStack {
        base: base.to_owned(),
        count,
    })
}

fn run(
    lists: &LeveledLists,
    root: &str,
    level: u32,
    rng: &mut ScriptedRandom,
) -> Option<ItemStack> {
    resolve(lists, &is_item, root, level, rng).unwrap()
}

fn level_ladder(flags: u8) -> LeveledLists {
    lists(vec![row(
        "ladder",
        0,
        flags,
        vec![entry("a", 1, 1), entry("b", 11, 1), entry("c", 17, 1)],
    )])
}

#[test]
fn a_single_entry_resolves_to_its_item_and_count() {
    let lists = lists(vec![row("list", 0, 0, vec![entry("a", 1, 3)])]);
    let mut rng = ScriptedRandom::new(&[99, 0], &[]);
    assert_eq!(run(&lists, "list", 1, &mut rng), stack("a", 3));
    assert_eq!(rng.asked_below(), &[100, 1]);
    assert!(rng.is_spent());
}

#[test]
fn a_nested_list_resolves_through_its_pick_and_multiplies_the_counts() {
    let lists = lists(vec![
        row("outer", 0, 0, vec![entry("inner", 1, 3)]),
        row("inner", 0, 0, vec![entry("a", 1, 2), entry("b", 1, 2)]),
    ]);
    let mut rng = ScriptedRandom::new(&[50, 0, 50, 1], &[]);
    assert_eq!(run(&lists, "outer", 1, &mut rng), stack("b", 6));
    assert_eq!(rng.asked_below(), &[100, 1, 100, 2]);
}

#[test]
fn a_nested_list_that_rolls_chance_none_ends_the_whole_resolve() {
    let lists = lists(vec![
        row("outer", 0, 0, vec![entry("inner", 1, 1)]),
        row("inner", 50, 0, vec![entry("a", 1, 1)]),
    ]);
    let mut rng = ScriptedRandom::new(&[0, 0, 49], &[]);
    assert_eq!(run(&lists, "outer", 1, &mut rng), None);
    assert!(rng.is_spent());
}

#[test]
fn chance_none_is_a_percentage_with_exact_boundaries() {
    let cases = [
        (0, 0, true),
        (0, 99, true),
        (25, 24, false),
        (25, 25, true),
        (80, 79, false),
        (80, 80, true),
        (100, 0, false),
        (100, 99, false),
    ];
    for (chance_none, draw, spawns) in cases {
        let lists = lists(vec![row("list", chance_none, 0, vec![entry("a", 1, 1)])]);
        let script = if spawns { vec![draw, 0] } else { vec![draw] };
        let mut rng = ScriptedRandom::new(&script, &[]);
        let expected = spawns.then(|| ItemStack {
            base: "a".to_owned(),
            count: 1,
        });
        assert_eq!(
            run(&lists, "list", 1, &mut rng),
            expected,
            "chance none {chance_none}, draw {draw}"
        );
        assert!(rng.is_spent(), "chance none {chance_none}, draw {draw}");
    }
}

#[test]
fn without_the_all_levels_flag_only_the_highest_eligible_level_qualifies() {
    let ladder = level_ladder(0);
    for (level, expected) in [
        (1, "a"),
        (10, "a"),
        (11, "b"),
        (16, "b"),
        (17, "c"),
        (60, "c"),
    ] {
        let mut rng = ScriptedRandom::new(&[99, 0], &[]);
        assert_eq!(
            run(&ladder, "ladder", level, &mut rng),
            stack(expected, 1),
            "player level {level}"
        );
        assert_eq!(rng.asked_below(), &[100, 1], "player level {level}");
    }
}

#[test]
fn with_the_all_levels_flag_every_entry_at_or_below_the_player_qualifies() {
    let ladder = level_ladder(ALL_LEVELS);
    let mut rng = ScriptedRandom::new(&[99, 1], &[]);
    assert_eq!(run(&ladder, "ladder", 11, &mut rng), stack("b", 1));
    assert_eq!(rng.asked_below(), &[100, 2]);

    let mut rng = ScriptedRandom::new(&[99, 0], &[]);
    assert_eq!(run(&ladder, "ladder", 1, &mut rng), stack("a", 1));
    assert_eq!(rng.asked_below(), &[100, 1]);

    let mut rng = ScriptedRandom::new(&[99, 2], &[]);
    assert_eq!(run(&ladder, "ladder", 17, &mut rng), stack("c", 1));
    assert_eq!(rng.asked_below(), &[100, 3]);
}

#[test]
fn with_the_all_levels_flag_a_level_between_ladder_levels_keeps_the_lower_entries_only() {
    let ladder = level_ladder(ALL_LEVELS);
    for (level, candidates) in [(10, 1), (12, 2), (16, 2)] {
        let mut rng = ScriptedRandom::new(&[99, 0], &[]);
        assert_eq!(run(&ladder, "ladder", level, &mut rng), stack("a", 1));
        assert_eq!(rng.asked_below(), &[100, candidates], "player level {level}");
    }
    let mut rng = ScriptedRandom::new(&[99, 1], &[]);
    assert_eq!(run(&ladder, "ladder", 12, &mut rng), stack("b", 1));
}

#[test]
fn a_list_without_an_eligible_entry_resolves_to_nothing_after_the_chance_roll() {
    let lists = lists(vec![row("list", 0, ALL_LEVELS, vec![entry("a", 5, 1)])]);
    let mut rng = ScriptedRandom::new(&[99], &[]);
    assert_eq!(run(&lists, "list", 1, &mut rng), None);
    assert_eq!(rng.asked_below(), &[100]);
}

#[test]
fn an_empty_list_resolves_to_nothing() {
    let lists = lists(vec![row("list", 0, 0, vec![])]);
    let mut rng = ScriptedRandom::new(&[99], &[]);
    assert_eq!(run(&lists, "list", 1, &mut rng), None);
}

#[test]
fn use_all_takes_the_first_candidate_without_a_pick() {
    let lists = lists(vec![row(
        "list",
        0,
        USE_ALL | ALL_LEVELS,
        vec![entry("c", 9, 1), entry("b", 1, 4), entry("a", 1, 1)],
    )]);
    let mut rng = ScriptedRandom::new(&[99], &[]);
    assert_eq!(run(&lists, "list", 1, &mut rng), stack("b", 4));
    assert_eq!(rng.asked_below(), &[100]);
}

#[test]
fn an_entry_listed_twice_weighs_twice() {
    let lists = lists(vec![row(
        "list",
        0,
        0,
        vec![entry("a", 1, 1), entry("a", 1, 1), entry("b", 1, 1)],
    )]);
    let picks = [0, 1, 2].map(|pick| {
        let mut rng = ScriptedRandom::new(&[99, pick], &[]);
        let result = run(&lists, "list", 1, &mut rng);
        assert_eq!(rng.asked_below(), &[100, 3]);
        result.unwrap().base
    });
    assert_eq!(picks, ["a", "a", "b"]);
}

#[test]
fn the_for_each_in_count_flag_is_accepted_and_leaves_the_count_as_it_is() {
    let lists = lists(vec![row(
        "list",
        0,
        FOR_EACH_IN_COUNT,
        vec![entry("a", 1, 5)],
    )]);
    let mut rng = ScriptedRandom::new(&[99, 0], &[]);
    assert_eq!(run(&lists, "list", 1, &mut rng), stack("a", 5));
}

#[test]
fn from_rows_reads_the_two_applied_bits_and_ignores_load_order() {
    let rows = vec![
        row("both", 0, ALL_LEVELS | FOR_EACH_IN_COUNT | USE_ALL, vec![]),
        row("neither", 0, FOR_EACH_IN_COUNT, vec![]),
    ];
    let forward = lists(rows.clone());
    let backward = lists(rows.into_iter().rev().collect());
    assert_eq!(forward, backward);
    assert_eq!(
        forward.get("both").unwrap().flags,
        LeveledFlags {
            all_levels_at_or_below_player: true,
            use_all: true
        }
    );
    assert_eq!(
        forward.get("neither").unwrap().flags,
        LeveledFlags {
            all_levels_at_or_below_player: false,
            use_all: false
        }
    );
    assert_eq!(forward.get("missing"), None);
}

#[test]
fn a_cycle_is_an_error_before_any_draw_even_behind_an_ineligible_entry() {
    let lists = lists(vec![
        row("one", 0, 0, vec![entry("a", 1, 1), entry("two", 50, 1)]),
        row("two", 0, 0, vec![entry("one", 1, 1)]),
    ]);
    let result = resolve(&lists, &is_item, "one", 1, &mut ScriptedRandom::silent());
    assert_eq!(result, Err(ResolveError::Cycle("one".to_owned())));
}

#[test]
fn a_list_that_names_itself_is_a_cycle() {
    let lists = lists(vec![row("list", 0, 0, vec![entry("list", 1, 1)])]);
    let result = resolve(&lists, &is_item, "list", 1, &mut ScriptedRandom::silent());
    assert_eq!(result, Err(ResolveError::Cycle("list".to_owned())));
}

#[test]
fn a_list_reached_twice_without_a_cycle_is_not_a_cycle() {
    let lists = lists(vec![
        row(
            "top",
            0,
            ALL_LEVELS,
            vec![entry("left", 1, 1), entry("right", 1, 1)],
        ),
        row("left", 0, 0, vec![entry("shared", 1, 1)]),
        row("right", 0, 0, vec![entry("shared", 1, 1)]),
        row("shared", 0, 0, vec![entry("a", 1, 1)]),
    ]);
    let mut rng = ScriptedRandom::new(&[99, 1, 99, 0, 99, 0], &[]);
    assert_eq!(run(&lists, "top", 1, &mut rng), stack("a", 1));
}

#[test]
fn a_missing_list_is_an_error_at_the_root_and_in_the_tree() {
    let lists = lists(vec![row(
        "list",
        0,
        0,
        vec![entry("a", 1, 1), entry("gone", 9, 1)],
    )]);
    let mut silent = ScriptedRandom::silent();
    assert_eq!(
        resolve(&lists, &is_item, "absent", 1, &mut silent),
        Err(ResolveError::UnknownList("absent".to_owned()))
    );
    assert_eq!(
        resolve(&lists, &is_item, "list", 1, &mut silent),
        Err(ResolveError::UnknownEntry {
            list: "list".to_owned(),
            target: "gone".to_owned()
        }),
        "an entry that is neither list nor item fails even when the level excludes it"
    );
}

#[test]
fn player_level_zero_is_an_error_before_any_draw() {
    let lists = lists(vec![row("list", 0, 0, vec![entry("a", 1, 1)])]);
    let result = resolve(&lists, &is_item, "list", 0, &mut ScriptedRandom::silent());
    assert_eq!(result, Err(ResolveError::InvalidLevel(0)));
}

fn nested_fixture() -> LeveledLists {
    lists(vec![
        row(
            "top",
            20,
            ALL_LEVELS,
            vec![entry("mid", 1, 2), entry("d", 1, 1)],
        ),
        row(
            "mid",
            0,
            0,
            vec![entry("a", 1, 1), entry("b", 1, 1), entry("c", 1, 1)],
        ),
    ])
}

#[test]
fn a_resolve_depends_only_on_its_list_level_and_seed() {
    let lists = nested_fixture();
    let resolve_top = |seed: u64| {
        resolve(&lists, &is_item, "top", 1, &mut SplitMix64::new(seed)).unwrap()
    };
    let alone: Vec<_> = (0..200).map(resolve_top).collect();
    let interleaved: Vec<_> = (0..200)
        .map(|seed| {
            let mut other = SplitMix64::new(seed ^ 0xABCD);
            resolve(&lists, &is_item, "mid", 5, &mut other).unwrap();
            let result = resolve_top(seed);
            resolve(&lists, &is_item, "mid", 1, &mut other).unwrap();
            result
        })
        .collect();
    assert_eq!(alone, interleaved);
    assert!(alone.iter().any(Option::is_none) && alone.iter().any(Option::is_some));
}

#[test]
fn nesting_four_lists_deep_resolves_to_the_leaf_with_multiplied_counts() {
    let lists = lists(vec![
        row("l1", 0, 0, vec![entry("l2", 1, 2)]),
        row("l2", 0, 0, vec![entry("l3", 1, 3)]),
        row("l3", 0, 0, vec![entry("l4", 1, 5)]),
        row("l4", 0, 0, vec![entry("a", 1, 7)]),
    ]);
    let mut rng = ScriptedRandom::new(&[99, 0, 99, 0, 99, 0, 99, 0], &[]);
    assert_eq!(run(&lists, "l1", 1, &mut rng), stack("a", 210));
    assert_eq!(rng.asked_below(), &[100, 1, 100, 1, 100, 1, 100, 1]);
    assert!(rng.is_spent());
}

fn pick_for(lists: &LeveledLists, seed: u64) -> Option<String> {
    resolve(lists, &is_item, "even", 1, &mut SplitMix64::new(seed))
        .unwrap()
        .map(|stack| stack.base)
}

fn even_list(chance_none: u8) -> LeveledLists {
    lists(vec![row(
        "even",
        chance_none,
        0,
        vec![
            entry("a", 1, 1),
            entry("b", 1, 1),
            entry("c", 1, 1),
            entry("d", 1, 1),
        ],
    )])
}

const SEEDS: u64 = 4000;
const TOLERANCE: usize = 150;

fn assert_near(actual: usize, expected: usize, what: &str) {
    assert!(
        actual.abs_diff(expected) <= TOLERANCE,
        "{what}: {actual}, expected {expected} +- {TOLERANCE}"
    );
}

#[test]
fn different_seeds_spread_evenly_over_an_even_list() {
    let lists = even_list(0);
    let picks: Vec<_> = (0..SEEDS).map(|seed| pick_for(&lists, seed)).collect();
    for item in ["a", "b", "c", "d"] {
        let hits = picks
            .iter()
            .filter(|pick| pick.as_deref() == Some(item))
            .count();
        assert_near(hits, SEEDS as usize / 4, item);
    }
}

#[test]
fn neighbouring_seeds_do_not_pick_alike() {
    let lists = even_list(0);
    let picks: Vec<_> = (0..SEEDS).map(|seed| pick_for(&lists, seed)).collect();
    let alike = picks.windows(2).filter(|pair| pair[0] == pair[1]).count();
    assert_near(
        alike,
        SEEDS as usize / 4,
        "neighbouring seeds picking the same entry",
    );
}

#[test]
fn chance_none_hits_its_share_of_seeds() {
    let lists = even_list(25);
    let empty = (0..SEEDS)
        .filter(|&seed| pick_for(&lists, seed).is_none())
        .count();
    assert_near(empty, SEEDS as usize / 4, "empty rolls at 25%");
}
