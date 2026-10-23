use std::collections::{HashMap, HashSet};

use bytemuck::Zeroable;
use glam::{UVec2, Vec3, Vec4};

use super::atlas::{AtlasError, MIN_TILES, Tile, TileAtlas};
use super::geometry::{CUBE_FACES, LampFrustums, LampProjection};
use super::invalidation::ShadowInvalidations;
use super::selection::{SelectionView, importance};
use super::settings::{LampShadowConfig, LampShadowSettings};
use super::source::{LampCandidate, LampKey};
use super::table::{GpuShadowLamp, GpuShadowLampTable, MAX_SHADOW_SLOTS};
use crate::rendering::shadows::views::{MAX_LAMP_VIEWS_PER_FRAME, ShadowView};

const _: () = assert!((MAX_SHADOW_SLOTS * CUBE_FACES) as u32 <= MIN_TILES);

/// Incumbent lamps rank as if this much more important.
const INCUMBENT_BONUS: f32 = 1.25;
/// A newly selected lamp keeps its shadow at least this long while eligible.
const SELECTION_HOLD_SECONDS: f32 = 0.5;
/// Shadow strength ramps between 0 and 1 over this time.
const STRENGTH_FADE_SECONDS: f32 = 0.3;
/// A lamp's tile size follows its distance only after the new size has been
/// wanted this long.
const SIZE_CHANGE_HOLD_SECONDS: f32 = 0.5;
/// Nominal tile sides as fractions of the atlas side.
const SPOT_TILE_SHIFT: u32 = 2;
const CUBE_FACE_TILE_SHIFT: u32 = 3;

/// A lamp view to render this frame into its atlas tile.
pub struct LampView {
    pub view: ShadowView,
    pub tile: Tile,
}

/// What the lamp shadows hand the renderers each frame.
pub struct LampShadowFrame {
    pub views: Vec<LampView>,
    pub table: GpuShadowLampTable,
    /// (placed-light slab index, table slot) of every sampled lamp.
    pub cull_slots: Vec<UVec2>,
}

#[derive(Debug, Default, Copy, Clone)]
pub struct LampShadowStats {
    pub resident: u32,
    pub eligible: u32,
    pub selected: u32,
    pub sampled: u32,
    pub views_rendered: u32,
    pub faces_dirty: u32,
    pub atlas_used_percent: f32,
    pub fit_halvings: u32,
    pub atlas_repacks: u32,
}

struct LampSlot {
    candidate: LampCandidate,
    importance: f32,
    selected: bool,
    selected_at: f32,
    strength: f32,
    projection: LampProjection,
    tile_size: u32,
    tiles: [Option<Tile>; CUBE_FACES],
    frustums: Option<LampFrustums>,
    dirty_faces: u8,
    ready: bool,
    /// A different tile size and since when it has been wanted.
    pending_size: Option<(u32, f32)>,
}

impl LampSlot {
    fn new(candidate: LampCandidate, now: f32) -> Self {
        Self {
            candidate,
            importance: 0.0,
            selected: true,
            selected_at: now,
            strength: 0.0,
            projection: LampProjection::of(&candidate),
            tile_size: 0,
            tiles: [None; CUBE_FACES],
            frustums: None,
            dirty_faces: 0,
            ready: false,
            pending_size: None,
        }
    }

    fn has_tiles(&self) -> bool {
        self.tiles.iter().any(Option::is_some)
    }

    fn area(&self, size: u32) -> u64 {
        u64::from(self.projection.face_count()) * u64::from(size) * u64::from(size)
    }

    fn position(&self) -> Vec3 {
        Vec3::from(self.candidate.light.position)
    }

    /// The tile side to use: the target once it has been wanted long
    /// enough, else the current one.
    fn held_size(&mut self, target: u32, now: f32) -> u32 {
        if self.tile_size == 0 || target == self.tile_size {
            self.pending_size = None;
            return target;
        }
        match self.pending_size {
            Some((pending, since)) if pending == target => {
                if now - since < SIZE_CHANGE_HOLD_SECONDS {
                    return self.tile_size;
                }
                self.pending_size = None;
                target
            }
            Some(_) | None => {
                self.pending_size = Some((target, now));
                self.tile_size
            }
        }
    }

    fn sampled(&self) -> bool {
        self.ready && self.strength > 0.0 && self.frustums.is_some()
    }
}

/// The lamp-shadow cache: which lamps are shadowed, where their tiles are in
/// the atlas, which faces must re-render, and the receivers' table.
pub struct LampShadowCache {
    atlas: TileAtlas,
    slots: [Option<LampSlot>; MAX_SHADOW_SLOTS],
    pub stats: LampShadowStats,
}

impl LampShadowCache {
    pub fn new(atlas_resolution: u32) -> Self {
        Self {
            atlas: TileAtlas::new(atlas_resolution),
            slots: std::array::from_fn(|_| None),
            stats: LampShadowStats::default(),
        }
    }

    pub fn update(
        &mut self,
        candidates: &[LampCandidate],
        view: &SelectionView,
        invalidations: &ShadowInvalidations,
        config: &LampShadowConfig,
        settings: &mut LampShadowSettings,
        now: f32,
        dt: f32,
    ) -> LampShadowFrame {
        let mut stats = LampShadowStats {
            resident: candidates.len() as u32,
            atlas_repacks: self.stats.atlas_repacks,
            ..Default::default()
        };
        let scores: HashMap<LampKey, (LampCandidate, Option<f32>)> = candidates
            .iter()
            .map(|candidate| (candidate.key, (*candidate, importance(candidate, view))))
            .collect();
        stats.eligible = scores.values().filter(|(_, score)| score.is_some()).count() as u32;

        self.drop_non_resident(&scores);
        let selected = self.select(&scores, settings, now);
        stats.selected = selected.len() as u32;
        self.apply_selection(&scores, &selected, now);
        self.fade_strengths(dt);
        let resized = self.resize_tiles(view.eye, config, now, &mut stats);
        self.allocate(&resized, &mut stats);
        self.invalidate(
            invalidations,
            std::mem::take(&mut settings.rerender_requested),
        );
        let views = self.schedule_views(&mut stats);
        stats.faces_dirty = self
            .slots
            .iter()
            .flatten()
            .map(|slot| slot.dirty_faces.count_ones())
            .sum();
        stats.atlas_used_percent =
            100.0 * self.atlas.used_texels() as f32 / self.atlas.total_texels() as f32;

        let mut table = GpuShadowLampTable::zeroed();
        table.header(config, settings);
        let mut cull_slots = Vec::new();
        for (index, slot) in self.slots.iter().enumerate() {
            let Some(slot) = slot.as_ref().filter(|slot| slot.sampled()) else {
                continue;
            };
            table.lamps[index] = self.record(slot);
            cull_slots.push(UVec2::new(slot.candidate.slab_index, index as u32));
        }
        stats.sampled = cull_slots.len() as u32;
        self.stats = stats;
        LampShadowFrame {
            views,
            table,
            cull_slots,
        }
    }

    fn drop_non_resident(&mut self, scores: &HashMap<LampKey, (LampCandidate, Option<f32>)>) {
        for index in 0..MAX_SHADOW_SLOTS {
            let resident = self.slots[index]
                .as_ref()
                .and_then(|slot| scores.get(&slot.candidate.key))
                .map(|(candidate, score)| (*candidate, *score));
            match (&mut self.slots[index], resident) {
                (Some(slot), Some((candidate, score))) => {
                    slot.candidate = candidate;
                    slot.importance = score.unwrap_or(0.0);
                }
                (Some(_), None) => self.release(index),
                (None, _) => {}
            }
        }
    }

    /// The lamps to shadow: eligible lamps ranked by importance, incumbents
    /// boosted and held for a while; frozen, the current choice stays.
    fn select(
        &self,
        scores: &HashMap<LampKey, (LampCandidate, Option<f32>)>,
        settings: &LampShadowSettings,
        now: f32,
    ) -> HashSet<LampKey> {
        let incumbents: HashMap<LampKey, f32> = self
            .slots
            .iter()
            .flatten()
            .filter(|slot| slot.selected)
            .map(|slot| (slot.candidate.key, slot.selected_at))
            .collect();
        if settings.freeze_selection {
            return incumbents.into_keys().collect();
        }
        if !settings.enabled {
            return HashSet::new();
        }
        let mut ranked: Vec<(bool, f32, LampKey)> = scores
            .iter()
            .filter_map(|(key, (_, score))| {
                let score = (*score)?;
                Some(match incumbents.get(key) {
                    Some(&since) => (
                        now - since < SELECTION_HOLD_SECONDS,
                        score * INCUMBENT_BONUS,
                        *key,
                    ),
                    None => (false, score, *key),
                })
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.total_cmp(&a.1)).then(a.2.cmp(&b.2)));
        ranked
            .into_iter()
            .take(settings.max_lamps as usize)
            .map(|(_, _, key)| key)
            .collect()
    }

    fn apply_selection(
        &mut self,
        scores: &HashMap<LampKey, (LampCandidate, Option<f32>)>,
        selected: &HashSet<LampKey>,
        now: f32,
    ) {
        let mut unslotted: Vec<LampKey> = selected.iter().copied().collect();
        for slot in self.slots.iter_mut().flatten() {
            let key = slot.candidate.key;
            let now_selected = selected.contains(&key);
            if now_selected && !slot.selected {
                slot.selected_at = now;
            }
            slot.selected = now_selected;
            unslotted.retain(|k| *k != key);
        }
        unslotted.sort();
        for key in unslotted {
            let (candidate, score) = scores[&key];
            let index = self.free_slot();
            let mut slot = LampSlot::new(candidate, now);
            slot.importance = score.unwrap_or(0.0);
            self.slots[index] = Some(slot);
        }
    }

    /// An empty slot, else the fading-out slot closest to gone. The table
    /// holds twice the maximum selection, so one of the two always exists.
    fn free_slot(&mut self) -> usize {
        if let Some(index) = self.slots.iter().position(Option::is_none) {
            return index;
        }
        let index = (0..MAX_SHADOW_SLOTS)
            .filter(|&i| self.slots[i].as_ref().is_some_and(|slot| !slot.selected))
            .min_by(|&a, &b| {
                let strength = |i: usize| self.slots[i].as_ref().map_or(0.0, |slot| slot.strength);
                strength(a).total_cmp(&strength(b))
            })
            .expect("at most half the slots are selected");
        self.release(index);
        index
    }

    fn release(&mut self, index: usize) {
        if let Some(slot) = self.slots[index].take() {
            for tile in slot.tiles.into_iter().flatten() {
                self.atlas.free(tile);
            }
        }
    }

    /// Selected lamps fade in once rendered; deselected ones fade out and
    /// leave the table at zero strength.
    fn fade_strengths(&mut self, dt: f32) {
        let step = dt / STRENGTH_FADE_SECONDS;
        for index in 0..MAX_SHADOW_SLOTS {
            let Some(slot) = self.slots[index].as_mut() else {
                continue;
            };
            if slot.selected {
                if slot.ready {
                    slot.strength = (slot.strength + step).min(1.0);
                }
            } else {
                slot.strength -= step;
                if slot.strength <= 0.0 {
                    self.release(index);
                }
            }
        }
    }

    /// Tile sides halve every `halve_distance` between the camera and the
    /// lamp's range sphere and the least important lamps shrink until all fit
    /// the atlas; a lamp follows a new side once it has been wanted for a
    /// while, unless the held sides would overfill the atlas. Returns the
    /// slots that need new tiles.
    fn resize_tiles(
        &mut self,
        eye: Vec3,
        config: &LampShadowConfig,
        now: f32,
        stats: &mut LampShadowStats,
    ) -> Vec<usize> {
        let occupied: Vec<usize> = (0..MAX_SHADOW_SLOTS)
            .filter(|&i| self.slots[i].is_some())
            .collect();
        let mut targets: Vec<u32> = occupied
            .iter()
            .map(|&i| self.distance_tile(self.slot(i), eye, config))
            .collect();
        stats.fit_halvings = self.fit_atlas(&occupied, &mut targets);
        let mut sizes: Vec<u32> = occupied
            .iter()
            .zip(&targets)
            .map(|(&i, &target)| self.slot_mut(i).held_size(target, now))
            .collect();
        if self.area(&occupied, &sizes) > self.atlas.total_texels() {
            sizes = targets;
        }
        occupied
            .into_iter()
            .zip(sizes)
            .filter(|&(i, size)| {
                let slot = self.slot_mut(i);
                let changed = size != slot.tile_size || !slot.has_tiles();
                slot.tile_size = size;
                changed
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn distance_tile(&self, slot: &LampSlot, eye: Vec3, config: &LampShadowConfig) -> u32 {
        let nominal = match slot.projection {
            LampProjection::Spot { .. } => self.atlas.resolution() >> SPOT_TILE_SHIFT,
            LampProjection::Cube { .. } => self.atlas.resolution() >> CUBE_FACE_TILE_SHIFT,
        };
        let surface_distance =
            (eye.distance(slot.position()) - slot.candidate.light.radius).max(0.0);
        let halvings = (surface_distance / config.halve_distance) as u32;
        nominal
            .checked_shr(halvings)
            .unwrap_or(0)
            .max(self.atlas.min_tile())
    }

    /// Halves the least important lamps' sides until all fit; returns the
    /// halvings.
    fn fit_atlas(&self, occupied: &[usize], sizes: &mut [u32]) -> u32 {
        let mut halvings = 0;
        while self.area(occupied, sizes) > self.atlas.total_texels() {
            let shrink = (0..occupied.len())
                .filter(|&k| sizes[k] > self.atlas.min_tile())
                .min_by(|&a, &b| {
                    let importance = |k: usize| self.slot(occupied[k]).importance;
                    importance(a).total_cmp(&importance(b))
                })
                .expect("every lamp at the minimum tile fits the atlas");
            sizes[shrink] /= 2;
            halvings += 1;
        }
        halvings
    }

    fn area(&self, occupied: &[usize], sizes: &[u32]) -> u64 {
        occupied
            .iter()
            .zip(sizes)
            .map(|(&i, &size)| self.slot(i).area(size))
            .sum()
    }

    fn slot(&self, index: usize) -> &LampSlot {
        self.slots[index].as_ref().expect("an occupied slot")
    }

    fn slot_mut(&mut self, index: usize) -> &mut LampSlot {
        self.slots[index].as_mut().expect("an occupied slot")
    }

    /// Frees the resized slots' tiles, then allocates largest first. A failed
    /// allocation repacks the whole atlas largest first, which cannot fail
    /// once the sizes fit.
    fn allocate(&mut self, resized: &[usize], stats: &mut LampShadowStats) {
        for &index in resized {
            if let Some(slot) = self.slots[index].as_mut() {
                for tile in std::mem::take(&mut slot.tiles).into_iter().flatten() {
                    self.atlas.free(tile);
                }
            }
        }
        if let Err(e) = self.allocate_slots(resized) {
            tracing::info!("lamp shadow atlas repacked: {e}");
            stats.atlas_repacks += 1;
            self.atlas.reset();
            for slot in self.slots.iter_mut().flatten() {
                slot.tiles = [None; CUBE_FACES];
            }
            let all: Vec<usize> = (0..MAX_SHADOW_SLOTS)
                .filter(|&i| self.slots[i].is_some())
                .collect();
            if let Err(e) = self.allocate_slots(&all) {
                unreachable!("a largest-first buddy packing of fitting tiles failed: {e}");
            }
        }
    }

    fn allocate_slots(&mut self, indices: &[usize]) -> Result<(), AtlasError> {
        let mut order = indices.to_vec();
        order
            .sort_by_key(|&i| std::cmp::Reverse(self.slots[i].as_ref().map_or(0, |s| s.tile_size)));
        for index in order {
            let Some(slot) = self.slots[index].as_mut() else {
                continue;
            };
            let mask = slot.projection.face_mask();
            for face in 0..CUBE_FACES {
                if mask & (1 << face) != 0 {
                    slot.tiles[face] = Some(self.atlas.allocate(slot.tile_size)?);
                }
            }
            slot.frustums = Some(LampFrustums::new(
                &slot.candidate,
                slot.projection,
                slot.tile_size,
            ));
            slot.dirty_faces = mask;
            slot.ready = false;
        }
        Ok(())
    }

    fn invalidate(&mut self, invalidations: &ShadowInvalidations, everything: bool) {
        for slot in self.slots.iter_mut().flatten() {
            if !slot.has_tiles() {
                continue;
            }
            let touched = everything
                || invalidations.touches_sphere(slot.position(), slot.candidate.light.radius);
            if touched {
                slot.dirty_faces = slot.projection.face_mask();
            }
        }
    }

    /// Faces rendered with an incomplete caster list render again.
    pub fn redirty(&mut self, tiles: &[Tile]) {
        for slot in self.slots.iter_mut().flatten() {
            for (face, tile) in slot.tiles.iter().enumerate() {
                if tile.is_some_and(|tile| tiles.contains(&tile)) {
                    slot.dirty_faces |= 1 << face;
                }
            }
        }
    }

    /// Selected lamps only, unrendered ones first, then by importance, up to
    /// the per-frame view budget. A lamp is ready once none of its faces is
    /// dirty.
    fn schedule_views(&mut self, stats: &mut LampShadowStats) -> Vec<LampView> {
        let mut order: Vec<usize> = (0..MAX_SHADOW_SLOTS)
            .filter(|&i| {
                self.slots[i]
                    .as_ref()
                    .is_some_and(|s| s.selected && s.dirty_faces != 0)
            })
            .collect();
        order.sort_by(|&a, &b| {
            let key = |i: usize| {
                let slot = self.slots[i].as_ref().expect("filtered to occupied slots");
                (slot.ready, -slot.importance)
            };
            let (ready_a, rank_a) = key(a);
            let (ready_b, rank_b) = key(b);
            ready_a
                .cmp(&ready_b)
                .then(rank_a.total_cmp(&rank_b))
                .then(a.cmp(&b))
        });

        let mut views = Vec::new();
        for index in order {
            let Some(slot) = self.slots[index].as_mut() else {
                continue;
            };
            let Some(frustums) = slot.frustums else {
                continue;
            };
            for face in 0..CUBE_FACES {
                if views.len() == MAX_LAMP_VIEWS_PER_FRAME {
                    break;
                }
                let (true, Some(tile)) = (slot.dirty_faces & (1 << face) != 0, slot.tiles[face])
                else {
                    continue;
                };
                views.push(LampView {
                    view: frustums.view(&slot.candidate, face),
                    tile,
                });
                slot.dirty_faces &= !(1 << face);
            }
            if slot.dirty_faces == 0 {
                slot.ready = true;
            }
        }
        stats.views_rendered = views.len() as u32;
        views
    }

    fn record(&self, slot: &LampSlot) -> GpuShadowLamp {
        let resolution = self.atlas.resolution() as f32;
        let tiles = slot.tiles.map(|tile| {
            tile.map_or(Vec4::ZERO, |tile| {
                Vec4::new(tile.x as f32, tile.y as f32, tile.size as f32, 0.0) / resolution
            })
        });
        let frustums = slot.frustums.as_ref().expect("sampled slots have frustums");
        GpuShadowLamp::new(
            frustums,
            tiles,
            slot.position(),
            slot.candidate.spec.near_clip,
            slot.strength,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::placed_light::GpuPlacedLight;
    use crate::rendering::shadows::lamps::geometry::ALL_FACES;
    use crate::rendering::shadows::lamps::source::{LampShadowSpec, ShadowType};
    use crate::rendering::shadows::settings::ShadowFilter;

    fn key(light_index: u32) -> LampKey {
        LampKey {
            residency: 1,
            light_index,
        }
    }

    fn candidate(light_index: u32) -> LampCandidate {
        let mut light = GpuPlacedLight::zeroed();
        light.position = [light_index as f32 * 10.0, 0.0, 0.0];
        light.radius = 5.0;
        light.direction = [0.0, 0.0, 1.0];
        LampCandidate {
            key: key(light_index),
            light,
            spec: LampShadowSpec {
                shadow_type: ShadowType::Omni,
                near_clip: 1.0,
                depth_bias: None,
            },
            slab_index: light_index,
        }
    }

    fn settings(max_lamps: u32) -> LampShadowSettings {
        let config = LampShadowConfig {
            max_lamps,
            filter: ShadowFilter::Grid3x3,
            halve_distance: 75.0,
            fade_end: 150.0,
            atlas_resolution: 1024,
            bias_scale: 1.0,
        };
        LampShadowSettings::new(&config)
    }

    fn scores(entries: &[(u32, Option<f32>)]) -> HashMap<LampKey, (LampCandidate, Option<f32>)> {
        entries
            .iter()
            .map(|&(index, score)| (key(index), (candidate(index), score)))
            .collect()
    }

    fn cache_with_incumbents(incumbents: &[(u32, f32)]) -> LampShadowCache {
        let mut cache = LampShadowCache::new(1024);
        for (slot, &(index, selected_at)) in incumbents.iter().enumerate() {
            cache.slots[slot] = Some(LampSlot::new(candidate(index), selected_at));
        }
        cache
    }

    fn selected(
        cache: &LampShadowCache,
        entries: &[(u32, Option<f32>)],
        settings: &LampShadowSettings,
        now: f32,
    ) -> Vec<u32> {
        let mut keys: Vec<u32> = cache
            .select(&scores(entries), settings, now)
            .into_iter()
            .map(|k| k.light_index)
            .collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn the_most_important_lamps_are_selected_up_to_the_cap() {
        let cache = LampShadowCache::new(1024);
        let picked = selected(
            &cache,
            &[
                (1, Some(5.0)),
                (2, Some(9.0)),
                (3, Some(1.0)),
                (4, Some(7.0)),
            ],
            &settings(2),
            0.0,
        );
        assert_eq!(picked, vec![2, 4]);
    }

    #[test]
    fn ineligible_lamps_are_never_selected() {
        let cache = LampShadowCache::new(1024);
        let picked = selected(&cache, &[(1, None), (2, Some(0.1))], &settings(4), 0.0);
        assert_eq!(picked, vec![2]);
    }

    #[test]
    fn equal_scores_are_broken_by_lamp_key_so_the_choice_is_stable() {
        let cache = LampShadowCache::new(1024);
        let picked = selected(
            &cache,
            &[(7, Some(1.0)), (3, Some(1.0)), (5, Some(1.0))],
            &settings(2),
            0.0,
        );
        assert_eq!(picked, vec![3, 5]);
    }

    #[test]
    fn a_cap_of_zero_selects_nothing() {
        let cache = LampShadowCache::new(1024);
        assert_eq!(
            selected(&cache, &[(1, Some(9.0))], &settings(0), 0.0),
            Vec::<u32>::new()
        );
    }

    #[test]
    fn disabled_lamp_shadows_select_nothing() {
        let cache = LampShadowCache::new(1024);
        let mut off = settings(4);
        off.enabled = false;
        assert_eq!(
            selected(&cache, &[(1, Some(9.0))], &off, 0.0),
            Vec::<u32>::new()
        );
    }

    #[test]
    fn a_frozen_selection_keeps_the_incumbents_whatever_the_scores() {
        let cache = cache_with_incumbents(&[(1, 0.0)]);
        let mut frozen = settings(4);
        frozen.freeze_selection = true;
        assert_eq!(
            selected(&cache, &[(1, None), (2, Some(99.0))], &frozen, 100.0),
            vec![1]
        );
    }

    #[test]
    fn an_incumbent_outranks_a_slightly_better_challenger() {
        // Past the hold, the incumbent still ranks 25% higher: 10 * 1.25 = 12.5 > 12.
        let cache = cache_with_incumbents(&[(1, 0.0)]);
        let picked = selected(
            &cache,
            &[(1, Some(10.0)), (2, Some(12.0))],
            &settings(1),
            10.0,
        );
        assert_eq!(picked, vec![1]);
    }

    #[test]
    fn a_clearly_better_challenger_replaces_an_incumbent_after_the_hold() {
        let cache = cache_with_incumbents(&[(1, 0.0)]);
        let picked = selected(
            &cache,
            &[(1, Some(10.0)), (2, Some(13.0))],
            &settings(1),
            10.0,
        );
        assert_eq!(picked, vec![2]);
    }

    #[test]
    fn a_new_incumbent_keeps_its_place_during_the_hold_whatever_the_challenger() {
        let cache = cache_with_incumbents(&[(1, 9.9)]);
        let picked = selected(
            &cache,
            &[(1, Some(0.1)), (2, Some(1000.0))],
            &settings(1),
            10.0,
        );
        assert_eq!(picked, vec![1]);
    }

    #[test]
    fn an_incumbent_that_became_ineligible_is_dropped_even_during_the_hold() {
        let cache = cache_with_incumbents(&[(1, 9.9)]);
        let picked = selected(&cache, &[(1, None), (2, Some(1.0))], &settings(1), 10.0);
        assert_eq!(picked, vec![2]);
    }

    fn dirty_slot(light_index: u32, importance: f32, ready: bool, dirty_faces: u8) -> LampSlot {
        let candidate = candidate(light_index);
        let mut slot = LampSlot::new(candidate, 0.0);
        slot.importance = importance;
        slot.ready = ready;
        slot.dirty_faces = dirty_faces;
        slot.frustums = Some(LampFrustums::new(&candidate, slot.projection, 256));
        for (face, tile) in slot.tiles.iter_mut().enumerate() {
            *tile = Some(Tile {
                x: face as u32 * 256,
                y: light_index * 256,
                size: 256,
            });
        }
        slot
    }

    fn scheduled_tiles(cache: &mut LampShadowCache) -> Vec<(u32, u32)> {
        let mut stats = LampShadowStats::default();
        cache
            .schedule_views(&mut stats)
            .into_iter()
            .map(|view| (view.tile.y / 256, view.tile.x / 256))
            .collect()
    }

    #[test]
    fn lamp_views_per_frame_are_capped() {
        let mut cache = LampShadowCache::new(1024);
        for (slot, index) in (0..3).enumerate() {
            cache.slots[slot] = Some(dirty_slot(index, 1.0, false, ALL_FACES));
        }
        let mut stats = LampShadowStats::default();
        let views = cache.schedule_views(&mut stats);
        assert_eq!(views.len(), MAX_LAMP_VIEWS_PER_FRAME);
        assert_eq!(stats.views_rendered, MAX_LAMP_VIEWS_PER_FRAME as u32);
    }

    #[test]
    fn faces_left_over_by_the_cap_render_on_the_following_frames() {
        let mut cache = LampShadowCache::new(1024);
        for (slot, index) in (0..3).enumerate() {
            cache.slots[slot] = Some(dirty_slot(index, 3.0 - index as f32, false, ALL_FACES));
        }
        assert_eq!(scheduled_tiles(&mut cache).len(), 12);
        let second = scheduled_tiles(&mut cache);
        assert_eq!(second.len(), 6);
        assert_eq!(scheduled_tiles(&mut cache), []);
    }

    #[test]
    fn a_lamp_becomes_ready_once_all_its_faces_rendered() {
        let mut cache = LampShadowCache::new(1024);
        cache.slots[0] = Some(dirty_slot(0, 1.0, false, ALL_FACES));
        cache.slots[1] = Some(dirty_slot(1, 0.5, false, ALL_FACES));
        scheduled_tiles(&mut cache);
        assert!(cache.slot(0).ready);
        assert!(cache.slot(1).ready);
        assert_eq!(cache.slot(0).dirty_faces, 0);
    }

    #[test]
    fn a_lamp_split_by_the_cap_is_not_ready_yet() {
        let mut cache = LampShadowCache::new(1024);
        for (slot, index) in (0..3).enumerate() {
            cache.slots[slot] = Some(dirty_slot(index, 3.0 - index as f32, false, ALL_FACES));
        }
        scheduled_tiles(&mut cache);
        assert!(cache.slot(0).ready && cache.slot(1).ready);
        assert!(!cache.slot(2).ready);
        assert_eq!(cache.slot(2).dirty_faces.count_ones(), 6);
    }

    #[test]
    fn lamps_that_never_rendered_go_before_more_important_cached_ones() {
        let mut cache = LampShadowCache::new(1024);
        cache.slots[0] = Some(dirty_slot(0, 100.0, true, 0b1));
        cache.slots[1] = Some(dirty_slot(1, 1.0, false, 0b1));
        let order = scheduled_tiles(&mut cache);
        assert_eq!(order, vec![(1, 0), (0, 0)]);
    }

    #[test]
    fn among_equally_ready_lamps_the_more_important_goes_first() {
        let mut cache = LampShadowCache::new(1024);
        cache.slots[0] = Some(dirty_slot(0, 1.0, false, 0b1));
        cache.slots[1] = Some(dirty_slot(1, 5.0, false, 0b1));
        assert_eq!(scheduled_tiles(&mut cache), vec![(1, 0), (0, 0)]);
    }

    #[test]
    fn deselected_clean_or_unallocated_lamps_schedule_nothing() {
        let mut cache = LampShadowCache::new(1024);
        let mut deselected = dirty_slot(0, 1.0, false, ALL_FACES);
        deselected.selected = false;
        let clean = dirty_slot(1, 1.0, true, 0);
        let mut no_frustums = dirty_slot(2, 1.0, false, ALL_FACES);
        no_frustums.frustums = None;
        cache.slots[0] = Some(deselected);
        cache.slots[1] = Some(clean);
        cache.slots[2] = Some(no_frustums);
        assert_eq!(scheduled_tiles(&mut cache), []);
    }

    #[test]
    fn only_dirty_faces_are_scheduled() {
        let mut cache = LampShadowCache::new(1024);
        cache.slots[0] = Some(dirty_slot(0, 1.0, true, 0b10_0100));
        let faces: Vec<u32> = scheduled_tiles(&mut cache)
            .into_iter()
            .map(|(_, face)| face)
            .collect();
        assert_eq!(faces, vec![2, 5]);
    }

    #[test]
    fn a_new_tile_size_is_adopted_only_after_it_was_wanted_for_the_hold_time() {
        let mut slot = LampSlot::new(candidate(0), 0.0);
        slot.tile_size = 256;
        assert_eq!(slot.held_size(128, 0.0), 256);
        assert_eq!(slot.held_size(128, SIZE_CHANGE_HOLD_SECONDS - 0.01), 256);
        assert_eq!(slot.held_size(128, SIZE_CHANGE_HOLD_SECONDS), 128);
    }

    #[test]
    fn a_wavering_tile_size_target_restarts_the_hold() {
        let mut slot = LampSlot::new(candidate(0), 0.0);
        slot.tile_size = 256;
        slot.held_size(128, 0.0);
        slot.held_size(64, 0.25);
        assert_eq!(slot.held_size(64, 0.5), 256);
        assert_eq!(slot.held_size(64, 0.75), 64);
    }

    #[test]
    fn a_lamp_without_a_tile_takes_its_first_size_at_once() {
        let mut slot = LampSlot::new(candidate(0), 0.0);
        assert_eq!(slot.held_size(128, 0.0), 128);
    }

    #[test]
    fn returning_to_the_current_tile_size_cancels_the_pending_change() {
        let mut slot = LampSlot::new(candidate(0), 0.0);
        slot.tile_size = 256;
        slot.held_size(128, 0.0);
        assert_eq!(slot.held_size(256, 0.125), 256);
        assert_eq!(slot.held_size(128, 0.25), 256);
        assert_eq!(slot.held_size(128, 0.5), 256);
    }
}
