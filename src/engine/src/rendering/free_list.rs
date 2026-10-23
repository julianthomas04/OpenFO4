//! Free-list range allocation shared by the mega buffers and the instance
//! slab: best-fit reuse over a sorted, coalesced list of free ranges, with
//! bump allocation past the high-water cursor as the fallback.

use std::ops::Range;

/// Best-fit over the free list, then bump allocation. Returns the offset of
/// a `count`-sized range, or None when neither the free list nor the
/// remaining capacity can hold it.
pub fn alloc_range(
    free_list: &mut Vec<Range<u32>>,
    count: u32,
    cursor: &mut u32,
    capacity: u32,
) -> Option<u32> {
    let mut best: Option<usize> = None;
    for (i, range) in free_list.iter().enumerate() {
        let size = range.end - range.start;
        if size < count {
            continue;
        }
        let better = match best {
            Some(b) => size < free_list[b].end - free_list[b].start,
            None => true,
        };
        if better {
            best = Some(i);
            if size == count {
                break;
            }
        }
    }

    if let Some(i) = best {
        let offset = free_list[i].start;
        if free_list[i].end - free_list[i].start == count {
            free_list.remove(i);
        } else {
            free_list[i].start += count;
        }
        return Some(offset);
    }

    let end = cursor.checked_add(count)?;
    if end <= capacity {
        let offset = *cursor;
        *cursor = end;
        Some(offset)
    } else {
        None
    }
}

/// Sorted insert + neighbor coalescing. The list is sorted, disjoint and
/// already coalesced, so only the immediate neighbours can touch the new
/// range: one binary search, one splice, no scan of the rest of the list.
pub fn insert_free_range(free_list: &mut Vec<Range<u32>>, offset: u32, count: u32) {
    let mut merged = offset..offset + count;

    let insert_pos = free_list.partition_point(|r| r.start < merged.start);

    let mut first = insert_pos;
    if insert_pos > 0 {
        let previous = &free_list[insert_pos - 1];
        if previous.end >= merged.start {
            merged.start = previous.start;
            merged.end = merged.end.max(previous.end);
            first = insert_pos - 1;
        }
    }

    let mut last = insert_pos;
    while last < free_list.len() && free_list[last].start <= merged.end {
        merged.end = merged.end.max(free_list[last].end);
        last += 1;
    }

    free_list.splice(first..last, std::iter::once(merged));
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn allocation_bumps_the_cursor_when_nothing_is_free() {
        let mut free = Vec::new();
        let mut cursor = 0;
        assert_eq!(alloc_range(&mut free, 10, &mut cursor, 100), Some(0));
        assert_eq!(alloc_range(&mut free, 5, &mut cursor, 100), Some(10));
        assert_eq!(cursor, 15);
    }

    #[test]
    fn allocation_fails_past_the_capacity_without_moving_the_cursor() {
        let mut free = Vec::new();
        let mut cursor = 95;
        assert_eq!(alloc_range(&mut free, 6, &mut cursor, 100), None);
        assert_eq!(cursor, 95);
        assert_eq!(alloc_range(&mut free, 5, &mut cursor, 100), Some(95));
    }

    #[test]
    fn allocation_does_not_overflow_the_cursor() {
        let mut free = Vec::new();
        let mut cursor = u32::MAX - 1;
        assert_eq!(alloc_range(&mut free, 5, &mut cursor, u32::MAX), None);
    }

    #[test]
    fn the_smallest_sufficient_free_range_is_used() {
        let mut free = vec![0..50, 100..110, 200..230];
        let mut cursor = 300;
        assert_eq!(alloc_range(&mut free, 8, &mut cursor, 1000), Some(100));
        assert_eq!(free, vec![0..50, 108..110, 200..230]);
        assert_eq!(cursor, 300);
    }

    #[test]
    fn an_exact_fit_consumes_the_whole_range() {
        let mut free = vec![0..50, 100..110];
        let mut cursor = 300;
        assert_eq!(alloc_range(&mut free, 10, &mut cursor, 1000), Some(100));
        assert_eq!(free, vec![0..50]);
    }

    #[test]
    fn a_request_larger_than_every_free_range_bumps_instead() {
        let mut free = vec![0..5, 10..15];
        let mut cursor = 20;
        assert_eq!(alloc_range(&mut free, 6, &mut cursor, 100), Some(20));
        assert_eq!(free, vec![0..5, 10..15]);
    }

    #[test]
    fn a_freed_range_between_two_free_ranges_merges_all_three() {
        let mut free = vec![0..10, 20..30];
        insert_free_range(&mut free, 10, 10);
        assert_eq!(free, vec![0..30]);
    }

    #[test]
    fn a_freed_range_merges_with_only_the_neighbour_it_touches() {
        let mut free = vec![0..10, 30..40];
        insert_free_range(&mut free, 10, 5);
        assert_eq!(free, vec![0..15, 30..40]);
        insert_free_range(&mut free, 25, 5);
        assert_eq!(free, vec![0..15, 25..40]);
    }

    #[test]
    fn a_freed_range_apart_from_the_others_stays_separate_and_sorted() {
        let mut free = vec![0..10, 30..40];
        insert_free_range(&mut free, 15, 5);
        assert_eq!(free, vec![0..10, 15..20, 30..40]);
    }

    #[test]
    fn a_freed_range_before_all_others_is_inserted_first() {
        let mut free = Vec::new();
        free.push(10..20);
        insert_free_range(&mut free, 0, 5);
        assert_eq!(free, vec![0..5, 10..20]);
    }

    const CAPACITY: u32 = 256;

    #[derive(Debug, Clone)]
    enum Op {
        Alloc(u32),
        Free(usize),
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            (1..=40u32).prop_map(Op::Alloc),
            (0..32usize).prop_map(Op::Free),
        ]
    }

    proptest! {
        /// Against a bitmap of used cells: allocations never overlap or leave the
        /// capacity, the free list stays sorted, disjoint and coalesced, and a
        /// request that fits a free range or the remaining tail is never refused.
        #[test]
        fn allocator_matches_a_bitmap_model(ops in proptest::collection::vec(op(), 1..150)) {
            let mut free: Vec<Range<u32>> = Vec::new();
            let mut cursor = 0u32;
            let mut used = [false; CAPACITY as usize];
            let mut live: Vec<(u32, u32)> = Vec::new();
            for op in ops {
                match op {
                    Op::Alloc(count) => {
                        let fits_free = free.iter().any(|r| r.end - r.start >= count);
                        let fits_tail = cursor + count <= CAPACITY;
                        match alloc_range(&mut free, count, &mut cursor, CAPACITY) {
                            Some(offset) => {
                                prop_assert!(offset + count <= CAPACITY);
                                for cell in offset..offset + count {
                                    prop_assert!(!used[cell as usize], "cell {} handed out twice", cell);
                                    used[cell as usize] = true;
                                }
                                live.push((offset, count));
                            }
                            None => prop_assert!(!fits_free && !fits_tail),
                        }
                    }
                    Op::Free(index) if !live.is_empty() => {
                        let (offset, count) = live.swap_remove(index % live.len());
                        for cell in offset..offset + count {
                            used[cell as usize] = false;
                        }
                        insert_free_range(&mut free, offset, count);
                    }
                    Op::Free(_) => {}
                }
                for pair in free.windows(2) {
                    prop_assert!(pair[0].end < pair[1].start, "not coalesced: {:?}", free);
                }
                for range in &free {
                    prop_assert!(range.start < range.end);
                    prop_assert!((range.start..range.end).all(|cell| !used[cell as usize]));
                }
            }
        }
    }
}
