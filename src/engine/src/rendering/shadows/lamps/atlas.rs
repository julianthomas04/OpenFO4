use std::collections::BTreeSet;

/// The smallest tile is the atlas side shifted right by this: 1024 minimum
/// tiles fit, whatever `iShadowMapResolution` says.
const MIN_TILE_SHIFT: u32 = 5;
pub const MIN_TILES: u32 = 1 << (2 * MIN_TILE_SHIFT);

/// A square region of the atlas, in texels.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Tile {
    pub x: u32,
    pub y: u32,
    pub size: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum AtlasError {
    #[error("no free {size}² tile")]
    Full { size: u32 },
}

/// Buddy allocator of power-of-two square tiles over a square atlas: level 0
/// is the whole atlas, each level halves the side.
pub struct TileAtlas {
    resolution: u32,
    free: Vec<BTreeSet<(u32, u32)>>,
    used_texels: u64,
}

impl TileAtlas {
    pub fn new(resolution: u32) -> Self {
        assert!(resolution.is_power_of_two() && resolution >> MIN_TILE_SHIFT > 0);
        let mut atlas = Self {
            resolution,
            free: vec![BTreeSet::new(); MIN_TILE_SHIFT as usize + 1],
            used_texels: 0,
        };
        atlas.reset();
        atlas
    }

    pub fn reset(&mut self) {
        for level in &mut self.free {
            level.clear();
        }
        self.free[0].insert((0, 0));
        self.used_texels = 0;
    }

    pub fn resolution(&self) -> u32 {
        self.resolution
    }

    pub fn min_tile(&self) -> u32 {
        self.resolution >> MIN_TILE_SHIFT
    }

    pub fn total_texels(&self) -> u64 {
        u64::from(self.resolution) * u64::from(self.resolution)
    }

    pub fn used_texels(&self) -> u64 {
        self.used_texels
    }

    /// `size` must be a power of two between `min_tile` and the resolution.
    fn level(&self, size: u32) -> usize {
        assert!(size.is_power_of_two() && (self.min_tile()..=self.resolution).contains(&size));
        (self.resolution / size).trailing_zeros() as usize
    }

    pub fn allocate(&mut self, size: u32) -> Result<Tile, AtlasError> {
        let level = self.level(size);
        let source = (0..=level)
            .rev()
            .find(|&l| !self.free[l].is_empty())
            .ok_or(AtlasError::Full { size })?;
        let (x, y) = self.free[source]
            .pop_first()
            .expect("the source level has a free block");
        let mut block = self.resolution >> source;
        for split_level in source + 1..=level {
            block /= 2;
            self.free[split_level].extend([(x + block, y), (x, y + block), (x + block, y + block)]);
        }
        self.used_texels += u64::from(size) * u64::from(size);
        Ok(Tile { x, y, size })
    }

    /// Merges the tile with its three buddies wherever they are all free.
    pub fn free(&mut self, tile: Tile) {
        self.used_texels -= u64::from(tile.size) * u64::from(tile.size);
        let (mut x, mut y, mut size) = (tile.x, tile.y, tile.size);
        let mut level = self.level(size);
        while level > 0 {
            let parent = size * 2;
            let (px, py) = (x - x % parent, y - y % parent);
            let buddies: Vec<(u32, u32)> = [
                (px, py),
                (px + size, py),
                (px, py + size),
                (px + size, py + size),
            ]
            .into_iter()
            .filter(|&block| block != (x, y))
            .collect();
            if !buddies.iter().all(|block| self.free[level].contains(block)) {
                break;
            }
            for block in &buddies {
                self.free[level].remove(block);
            }
            (x, y, size) = (px, py, parent);
            level -= 1;
        }
        self.free[level].insert((x, y));
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const RESOLUTION: u32 = 1024;
    const MIN_TILE: u32 = RESOLUTION >> MIN_TILE_SHIFT;

    fn overlaps(a: &Tile, b: &Tile) -> bool {
        a.x < b.x + b.size && b.x < a.x + a.size && a.y < b.y + b.size && b.y < a.y + a.size
    }

    #[test]
    fn the_whole_atlas_is_one_allocation() {
        let mut atlas = TileAtlas::new(RESOLUTION);
        let tile = atlas.allocate(RESOLUTION).unwrap();
        assert_eq!(
            tile,
            Tile {
                x: 0,
                y: 0,
                size: RESOLUTION
            }
        );
        assert!(matches!(
            atlas.allocate(MIN_TILE),
            Err(AtlasError::Full { size: MIN_TILE })
        ));
    }

    #[test]
    fn four_quarter_tiles_fill_the_atlas_without_overlap() {
        let mut atlas = TileAtlas::new(RESOLUTION);
        let tiles: Vec<Tile> = (0..4)
            .map(|_| atlas.allocate(RESOLUTION / 2).unwrap())
            .collect();
        for (i, a) in tiles.iter().enumerate() {
            for b in &tiles[i + 1..] {
                assert!(!overlaps(a, b), "{a:?} overlaps {b:?}");
            }
        }
        assert!(atlas.allocate(RESOLUTION / 2).is_err());
        assert_eq!(atlas.used_texels(), atlas.total_texels());
    }

    #[test]
    fn an_exhausted_atlas_reports_the_requested_size() {
        let mut atlas = TileAtlas::new(RESOLUTION);
        atlas.allocate(RESOLUTION).unwrap();
        let error = atlas.allocate(256).unwrap_err();
        assert_eq!(error.to_string(), "no free 256² tile");
    }

    #[test]
    fn freeing_every_quarter_merges_back_into_the_whole_atlas() {
        let mut atlas = TileAtlas::new(RESOLUTION);
        let tiles: Vec<Tile> = (0..4)
            .map(|_| atlas.allocate(RESOLUTION / 2).unwrap())
            .collect();
        for tile in tiles {
            atlas.free(tile);
        }
        assert_eq!(atlas.used_texels(), 0);
        assert!(atlas.allocate(RESOLUTION).is_ok());
    }

    #[test]
    fn a_tile_does_not_merge_while_a_buddy_is_still_in_use() {
        let mut atlas = TileAtlas::new(RESOLUTION);
        let kept = atlas.allocate(MIN_TILE).unwrap();
        let freed = atlas.allocate(MIN_TILE).unwrap();
        atlas.free(freed);
        assert!(atlas.allocate(RESOLUTION).is_err());
        atlas.free(kept);
        assert!(atlas.allocate(RESOLUTION).is_ok());
    }

    #[test]
    fn a_small_allocation_splits_a_larger_free_block_and_keeps_the_rest_free() {
        let mut atlas = TileAtlas::new(RESOLUTION);
        atlas.allocate(MIN_TILE).unwrap();
        assert_eq!(atlas.used_texels(), u64::from(MIN_TILE * MIN_TILE));
        for size in [
            MIN_TILE,
            2 * MIN_TILE,
            4 * MIN_TILE,
            8 * MIN_TILE,
            16 * MIN_TILE,
        ] {
            assert!(atlas.allocate(size).is_ok(), "{size}");
        }
    }

    #[test]
    fn the_smallest_tile_is_the_atlas_side_over_thirty_two() {
        assert_eq!(TileAtlas::new(RESOLUTION).min_tile(), 32);
        assert_eq!(TileAtlas::new(4096).min_tile(), 128);
    }

    #[test]
    fn reset_frees_everything() {
        let mut atlas = TileAtlas::new(RESOLUTION);
        atlas.allocate(RESOLUTION / 2).unwrap();
        atlas.reset();
        assert_eq!(atlas.used_texels(), 0);
        assert!(atlas.allocate(RESOLUTION).is_ok());
    }

    #[derive(Debug, Clone)]
    enum Op {
        Allocate(u32),
        Free(usize),
    }

    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            (0..=5u32).prop_map(|shift| Op::Allocate(RESOLUTION >> shift)),
            (0..64usize).prop_map(Op::Free),
        ]
    }

    proptest! {
        #[test]
        fn live_tiles_stay_aligned_in_bounds_disjoint_and_fully_reclaimable(
            ops in proptest::collection::vec(op(), 1..120)
        ) {
            let mut atlas = TileAtlas::new(RESOLUTION);
            let mut live: Vec<Tile> = Vec::new();
            for op in ops {
                match op {
                    Op::Allocate(size) => {
                        if let Ok(tile) = atlas.allocate(size) {
                            prop_assert_eq!(tile.x % size, 0);
                            prop_assert_eq!(tile.y % size, 0);
                            prop_assert!(tile.x + size <= RESOLUTION && tile.y + size <= RESOLUTION);
                            prop_assert!(live.iter().all(|other| !overlaps(&tile, other)));
                            live.push(tile);
                        }
                    }
                    Op::Free(index) if !live.is_empty() => {
                        atlas.free(live.swap_remove(index % live.len()));
                    }
                    Op::Free(_) => {}
                }
                let used: u64 = live.iter().map(|t| u64::from(t.size) * u64::from(t.size)).sum();
                prop_assert_eq!(atlas.used_texels(), used);
            }
            for tile in live {
                atlas.free(tile);
            }
            prop_assert!(atlas.allocate(RESOLUTION).is_ok());
        }
    }
}
