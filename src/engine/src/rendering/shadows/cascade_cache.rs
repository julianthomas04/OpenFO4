use glam::Vec3;

use super::cascades::{Cascade, CascadeSet};
use super::lamps::ShadowInvalidations;
use crate::rendering::lighting::frame_uniform::MAX_CASCADES;
use crate::utils::{env_flag, env_or};

const ENABLE_ENV: &str = "OPENFO4_SHADOW_CACHE";
const INTERVALS_ENV: &str = "OPENFO4_SHADOW_CACHE_INTERVALS";
const DRIFT_ENV: &str = "OPENFO4_SHADOW_DRIFT";
const EYE_CENTERED_ENV: &str = "OPENFO4_SHADOW_CACHE_EYE_FROM";
const STALE_INTERVALS_ENV: &str = "OPENFO4_SHADOW_CACHE_STALE_INTERVALS";

/// Fewest frames between two renders of a cascade; 0 renders whenever the fit
/// differs from the cached one.
const DEFAULT_INTERVALS: [u32; MAX_CASCADES] = [0, 2, 4, 4];
/// Fewest frames between two renders of a cascade whose only reason is a
/// changed caster, not drift. Far cascades cover many casters and cannot
/// resolve small changes, so they re-render these lazily.
const DEFAULT_STALE_INTERVALS: [u32; MAX_CASCADES] = [0, 4, 8, 16];
/// Drift, as a fraction of the cached radius, that allows a refresh once the
/// interval has passed, and the drift that forces one regardless.
const DEFAULT_SOFT_DRIFT: f32 = 0.03;
const DEFAULT_HARD_DRIFT: f32 = 0.08;
/// Change of the sun direction (unit vector distance) that re-renders an
/// every-frame cascade.
const SUN_CHANGE_EPSILON: f32 = 1e-4;
/// Far cascades resolve less, so they tolerate more sun movement and, past the
/// interval, re-render one at a time instead of all on the same frame.
const FAR_SUN_CHANGE_EPSILON: f32 = 1e-3;
/// Sun change after which a far cascade is overdue rather than merely due.
const FAR_SUN_HARD_CHANGE: f32 = 1e-2;
/// A due far cascade that waited this many of its own intervals becomes
/// overdue, so a stream of overdue cascades cannot starve it.
const STARVATION_INTERVALS: u32 = 4;
/// A cascade with interval 0 refreshes when its fit moved by more than this
/// fraction of a texel.
const EXACT_FIT_TEXEL_FRACTION: f32 = 1e-3;

#[derive(Debug, Clone)]
pub struct CacheSettings {
    pub enabled: bool,
    pub intervals: [u32; MAX_CASCADES],
    pub stale_intervals: [u32; MAX_CASCADES],
    pub soft_drift: f32,
    pub hard_drift: f32,
    /// First cascade fitted around the eye instead of along the view axis.
    pub eye_centered_from: usize,
}

impl CacheSettings {
    pub fn from_env(cascade_count: usize) -> Self {
        let enabled = env_flag(ENABLE_ENV, true);
        let (soft_drift, hard_drift) = drift_from_env();
        Self {
            enabled,
            intervals: intervals_from_env(INTERVALS_ENV, DEFAULT_INTERVALS),
            stale_intervals: intervals_from_env(STALE_INTERVALS_ENV, DEFAULT_STALE_INTERVALS),
            soft_drift,
            hard_drift,
            // Only the farthest cascade by default: it is the one whose
            // resolution loss from the larger sphere is least visible.
            eye_centered_from: if enabled {
                env_or(EYE_CENTERED_ENV, cascade_count.saturating_sub(1))
            } else {
                cascade_count
            },
        }
    }
}

fn parse_list<T: std::str::FromStr>(raw: &str) -> Option<Vec<T>> {
    raw.split(',')
        .map(|part| part.trim().parse().ok())
        .collect()
}

/// `raw` is 1 to `MAX_CASCADES` comma separated frame counts overriding the
/// leading cascades' `defaults`; `None` when it is anything else.
fn parse_intervals(raw: &str, defaults: [u32; MAX_CASCADES]) -> Option<[u32; MAX_CASCADES]> {
    let list =
        parse_list::<u32>(raw).filter(|list| !list.is_empty() && list.len() <= MAX_CASCADES)?;
    let mut intervals = defaults;
    intervals[..list.len()].copy_from_slice(&list);
    Some(intervals)
}

/// `raw` is `soft,hard` with `0 <= soft <= hard`, both finite.
fn parse_drift(raw: &str) -> Option<(f32, f32)> {
    match parse_list::<f32>(raw).as_deref() {
        Some(&[soft, hard])
            if soft.is_finite() && soft >= 0.0 && hard.is_finite() && hard >= soft =>
        {
            Some((soft, hard))
        }
        _ => None,
    }
}

fn intervals_from_env(name: &str, defaults: [u32; MAX_CASCADES]) -> [u32; MAX_CASCADES] {
    let Ok(raw) = std::env::var(name) else {
        return defaults;
    };
    parse_intervals(&raw, defaults).unwrap_or_else(|| {
        tracing::warn!(
            "{name}={raw:?} is not 1-{MAX_CASCADES} comma separated frame counts; using the defaults"
        );
        defaults
    })
}

fn drift_from_env() -> (f32, f32) {
    let defaults = (DEFAULT_SOFT_DRIFT, DEFAULT_HARD_DRIFT);
    let Ok(raw) = std::env::var(DRIFT_ENV) else {
        return defaults;
    };
    parse_drift(&raw).unwrap_or_else(|| {
        tracing::warn!(
            "{DRIFT_ENV}={raw:?} is not `soft,hard` with 0 <= soft <= hard; using the defaults"
        );
        defaults
    })
}

/// Why a cascade map was rendered again.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RefreshReason {
    /// Nothing cached: first frame, after a discard or a whole-world change.
    Missing,
    Sun,
    /// A caster changed inside the cascade and no drift forced the render.
    Stale,
    /// The fit left the cached sphere by more than the soft drift.
    Drift,
    /// The fit left the cached sphere by more than the hard drift.
    Hard,
}

impl RefreshReason {
    pub const ALL: [Self; 5] = [
        Self::Missing,
        Self::Sun,
        Self::Stale,
        Self::Drift,
        Self::Hard,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Sun => "sun",
            Self::Stale => "stale",
            Self::Drift => "drift",
            Self::Hard => "hard",
        }
    }
}

/// Committed renders per cascade and reason, indexed by `RefreshReason as usize`.
pub type ReasonCounts = [[u64; RefreshReason::ALL.len()]; MAX_CASCADES];

/// A cascade map as last rendered.
#[derive(Debug, Clone)]
struct Entry {
    cascade: Cascade,
    to_sun: Vec3,
    frames_since_render: u32,
    /// A caster changed inside the cascade's light cylinder since the render.
    stale: bool,
    /// Why this entry was rendered; counted when the render is committed.
    reason: RefreshReason,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Urgency {
    /// Past the interval and drifted or stale.
    Due,
    /// The fit left the cached sphere by more than the hard drift.
    Overdue,
}

enum Decision {
    Keep,
    /// Render this frame.
    Render(RefreshReason),
    /// Render this frame unless another deferred cascade is more urgent; at
    /// most one such far cascade renders per frame.
    Defer(Urgency, u32, RefreshReason),
}

/// Decides which sun cascades re-render and hands every other cascade its
/// cached matrices, so receivers keep sampling the map as it was rendered.
///
/// A render counts only once the renderer reports it encoded (`commit`): a
/// frame that is skipped between update and encode must not look rendered.
#[derive(Debug, Clone)]
pub struct CascadeCache {
    pub settings: CacheSettings,
    entries: [Option<Entry>; MAX_CASCADES],
    pending: [Option<Entry>; MAX_CASCADES],
    refreshes: ReasonCounts,
}

impl CascadeCache {
    pub fn new(cascade_count: usize) -> Self {
        Self::with_settings(CacheSettings::from_env(cascade_count))
    }

    pub fn with_settings(settings: CacheSettings) -> Self {
        Self {
            settings,
            entries: Default::default(),
            pending: Default::default(),
            refreshes: [[0; RefreshReason::ALL.len()]; MAX_CASCADES],
        }
    }

    /// Cumulative committed renders per cascade and reason.
    pub fn refreshes(&self) -> ReasonCounts {
        self.refreshes
    }

    /// Forgets every cached map, e.g. while sun shadows are off and the maps
    /// are not kept up to date.
    pub fn discard(&mut self) {
        self.entries = Default::default();
        self.pending = Default::default();
    }

    /// Adopts the proposals of the previous `resolve` whose cascade was
    /// rendered (bit set in `rendered`); the others are dropped.
    pub fn commit(&mut self, rendered: u32) {
        for (i, pending) in self.pending.iter_mut().enumerate() {
            let Some(entry) = pending.take() else {
                continue;
            };
            if rendered >> i & 1 == 1 {
                self.refreshes[i][entry.reason as usize] += 1;
                self.entries[i] = Some(entry);
            }
        }
    }

    /// Marks cached cascades whose light cylinder a changed region reaches,
    /// ignoring regions too small for the cascade to draw their casters.
    /// Changes of this frame are already in the cascades that render this
    /// frame; their commit replaces the marked entry.
    ///
    /// A whole-world change drops every cached map, and `current`, the set
    /// already chosen for this frame, renders every cascade so that no frame
    /// samples the old world's shadows. The renderer encodes after this call.
    pub fn note_invalidations(
        &mut self,
        invalidations: &ShadowInvalidations,
        current: Option<&mut CascadeSet>,
    ) {
        if invalidations.is_everything() {
            self.entries = Default::default();
            if let (true, Some(current)) = (self.settings.enabled, current) {
                let to_sun = current.to_sun;
                for (i, cascade) in current.cascades.iter_mut().enumerate() {
                    cascade.refresh = true;
                    self.pending[i] = Some(Entry {
                        cascade: *cascade,
                        to_sun,
                        frames_since_render: 0,
                        stale: false,
                        reason: RefreshReason::Missing,
                    });
                }
            }
            return;
        }
        for entry in self.entries.iter_mut().flatten() {
            let sphere = entry.cascade.sphere;
            let smallest_drawn = entry.cascade.min_texels * entry.cascade.texel_world;
            entry.stale |= invalidations.touches_cylinder(
                sphere.truncate(),
                entry.to_sun,
                sphere.w,
                smallest_drawn,
            );
        }
    }

    /// The cascades of a frozen fit: cascades with a committed map keep it,
    /// the others render the frozen fit until a render of it is committed.
    pub fn hold(&mut self, frozen: &CascadeSet) -> CascadeSet {
        let mut held = frozen.clone();
        self.pending = Default::default();
        for (i, cascade) in held.cascades.iter_mut().enumerate() {
            cascade.refresh = self.settings.enabled && self.entries[i].is_none();
            if cascade.refresh {
                self.pending[i] = Some(Entry {
                    cascade: *cascade,
                    to_sun: frozen.to_sun,
                    frames_since_render: 0,
                    stale: false,
                    reason: RefreshReason::Missing,
                });
            }
        }
        held
    }

    /// Chooses the cascades to render and substitutes the cached fit for the
    /// rest. Call `commit` first.
    pub fn resolve(&mut self, mut fitted: CascadeSet) -> CascadeSet {
        if !self.settings.enabled {
            return fitted;
        }
        self.pending = Default::default();
        for entry in self.entries.iter_mut().flatten() {
            entry.frames_since_render = entry.frames_since_render.saturating_add(1);
        }

        let mut refresh: [Option<RefreshReason>; MAX_CASCADES] = [None; MAX_CASCADES];
        let mut deferred: Option<(Urgency, u32, usize, RefreshReason)> = None;
        for (i, fit) in fitted.cascades.iter().enumerate() {
            match self.classify(i, fit, fitted.to_sun) {
                Decision::Keep => {}
                Decision::Render(reason) => refresh[i] = Some(reason),
                Decision::Defer(_, _, reason) if i == 0 => refresh[i] = Some(reason),
                Decision::Defer(urgency, age, reason) => {
                    if deferred.is_none_or(|(best, best_age, ..)| (urgency, age) > (best, best_age))
                    {
                        deferred = Some((urgency, age, i, reason));
                    }
                }
            }
        }
        if let Some((_, _, i, reason)) = deferred {
            refresh[i] = Some(reason);
        }

        for (i, cascade) in fitted.cascades.iter_mut().enumerate() {
            if let Some(reason) = refresh[i] {
                self.pending[i] = Some(Entry {
                    cascade: *cascade,
                    to_sun: fitted.to_sun,
                    frames_since_render: 0,
                    stale: false,
                    reason,
                });
            } else if let Some(entry) = &self.entries[i] {
                *cascade = Cascade {
                    refresh: false,
                    ..entry.cascade
                };
            }
        }
        fitted
    }

    fn classify(&self, index: usize, fit: &Cascade, to_sun: Vec3) -> Decision {
        let Some(entry) = &self.entries[index] else {
            return Decision::Render(RefreshReason::Missing);
        };
        let sun_change = entry.to_sun.distance(to_sun);
        let cached = &entry.cascade;
        let interval = self.settings.intervals[index];
        if interval == 0 {
            if sun_change > SUN_CHANGE_EPSILON {
                return Decision::Render(RefreshReason::Sun);
            }
            let moved =
                fit.sphere.distance(cached.sphere) > cached.texel_world * EXACT_FIT_TEXEL_FRACTION;
            return match (moved, entry.stale) {
                (true, _) => Decision::Render(RefreshReason::Drift),
                (false, true) => Decision::Render(RefreshReason::Stale),
                (false, false) => Decision::Keep,
            };
        }
        let radius = cached.sphere.w;
        let drift = (fit.sphere.truncate() - cached.sphere.truncate()).length()
            + (fit.sphere.w - radius).max(0.0);
        let age = entry.frames_since_render;
        let due = |wait: u32| {
            if age > wait.max(1).saturating_mul(STARVATION_INTERVALS) {
                Urgency::Overdue
            } else {
                Urgency::Due
            }
        };
        let stale_wait = interval.max(self.settings.stale_intervals[index]);
        if drift > self.settings.hard_drift * radius {
            Decision::Defer(Urgency::Overdue, age, RefreshReason::Hard)
        } else if sun_change > FAR_SUN_HARD_CHANGE {
            Decision::Defer(Urgency::Overdue, age, RefreshReason::Sun)
        } else if age >= interval && sun_change > FAR_SUN_CHANGE_EPSILON {
            Decision::Defer(due(interval), age, RefreshReason::Sun)
        } else if age >= interval && drift > self.settings.soft_drift * radius {
            Decision::Defer(due(interval), age, RefreshReason::Drift)
        } else if age >= stale_wait && entry.stale {
            Decision::Defer(due(stale_wait), age, RefreshReason::Stale)
        } else {
            Decision::Keep
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::{Mat4, Vec4};

    use super::*;
    use crate::rendering::frustum::Aabb;

    const SUN: Vec3 = Vec3::Y;

    fn settings() -> CacheSettings {
        CacheSettings {
            enabled: true,
            intervals: [0, 2, 2, 2],
            stale_intervals: [0, 4, 8, 16],
            soft_drift: 0.03,
            hard_drift: 0.08,
            eye_centered_from: 3,
        }
    }

    fn cache() -> CascadeCache {
        CascadeCache::with_settings(settings())
    }

    /// Distinguishable matrices: each cascade's matrix encodes its centre.
    fn cascade(center: Vec3, radius: f32) -> Cascade {
        Cascade {
            view_proj: Mat4::from_translation(center),
            sphere: center.extend(radius),
            texel_world: 0.1,
            min_texels: 1.0,
            skip_alpha: false,
            refresh: true,
        }
    }

    fn set_at(centers: [Vec3; 3]) -> CascadeSet {
        CascadeSet {
            cascades: centers.map(|center| cascade(center, 10.0)).to_vec(),
            to_sun: SUN,
        }
    }

    fn still_set() -> CascadeSet {
        set_at([Vec3::ZERO, Vec3::X * 100.0, Vec3::X * 200.0])
    }

    fn refreshed(set: &CascadeSet) -> Vec<bool> {
        set.cascades.iter().map(|c| c.refresh).collect()
    }

    /// Resolves and commits whatever was chosen, as the renderer does.
    fn render(cache: &mut CascadeCache, fitted: CascadeSet) -> CascadeSet {
        let resolved = cache.resolve(fitted);
        let rendered = resolved
            .cascades
            .iter()
            .enumerate()
            .filter(|(_, c)| c.refresh)
            .fold(0, |mask, (i, _)| mask | 1 << i);
        cache.commit(rendered);
        resolved
    }

    fn primed_cache() -> CascadeCache {
        let mut cache = cache();
        render(&mut cache, still_set());
        cache
    }

    #[test]
    fn first_resolve_renders_every_cascade() {
        let resolved = cache().resolve(still_set());
        assert_eq!(refreshed(&resolved), vec![true, true, true]);
    }

    #[test]
    fn a_render_that_was_never_committed_is_not_remembered() {
        let mut cache = cache();
        cache.resolve(still_set());
        assert_eq!(
            refreshed(&cache.resolve(still_set())),
            vec![true, true, true]
        );
    }

    #[test]
    fn committing_only_some_cascades_caches_only_those() {
        let mut cache = cache();
        cache.resolve(still_set());
        cache.commit(0b001);
        assert_eq!(
            refreshed(&cache.resolve(still_set())),
            vec![false, true, true]
        );
    }

    #[test]
    fn an_unchanged_fit_keeps_every_cached_map() {
        let mut cache = primed_cache();
        let resolved = cache.resolve(still_set());
        assert_eq!(refreshed(&resolved), vec![false, false, false]);
    }

    #[test]
    fn a_kept_cascade_carries_the_matrices_of_its_cached_render() {
        let mut cache = primed_cache();
        let moved_slightly = set_at([Vec3::ZERO, Vec3::X * 100.0 + Vec3::Y * 0.1, Vec3::X * 200.0]);
        let resolved = cache.resolve(moved_slightly);
        assert!(!resolved.cascades[1].refresh);
        assert_eq!(
            resolved.cascades[1].view_proj,
            Mat4::from_translation(Vec3::X * 100.0)
        );
        assert_eq!(
            resolved.cascades[1].sphere,
            Vec4::new(100.0, 0.0, 0.0, 10.0)
        );
    }

    #[test]
    fn an_every_frame_cascade_renders_when_its_fit_moved_a_fraction_of_a_texel() {
        let mut cache = primed_cache();
        let resolved = cache.resolve(set_at([Vec3::X * 0.01, Vec3::X * 100.0, Vec3::X * 200.0]));
        assert_eq!(refreshed(&resolved), vec![true, false, false]);
    }

    #[test]
    fn an_every_frame_cascade_ignores_motion_below_the_exact_fit_tolerance() {
        let mut cache = primed_cache();
        let resolved = cache.resolve(set_at([Vec3::X * 1e-5, Vec3::X * 100.0, Vec3::X * 200.0]));
        assert!(!resolved.cascades[0].refresh);
    }

    #[test]
    fn a_sun_change_renders_the_every_frame_cascade_at_once() {
        let mut cache = primed_cache();
        let mut set = still_set();
        set.to_sun = Vec3::new(0.0, 1.0, 2e-4);
        let resolved = cache.resolve(set);
        assert_eq!(refreshed(&resolved), vec![true, false, false]);
    }

    #[test]
    fn far_cascades_tolerate_small_sun_movement() {
        let mut cache = primed_cache();
        cache.resolve(still_set());
        let mut set = still_set();
        set.to_sun = Vec3::new(0.0, 1.0, 5e-4);
        let resolved = cache.resolve(set);
        assert_eq!(refreshed(&resolved)[1..], [false, false]);
    }

    #[test]
    fn a_large_sun_jump_renders_a_far_cascade_before_its_interval_ends() {
        let mut cache = primed_cache();
        let mut set = still_set();
        set.to_sun = Vec3::new(0.0, 1.0, 0.05);
        let resolved = cache.resolve(set);
        assert_eq!(refreshed(&resolved)[1..].iter().filter(|r| **r).count(), 1);
    }

    #[test]
    fn soft_drift_waits_for_the_interval() {
        let mut cache = primed_cache();
        // 0.5 over a radius of 10 is 5%: past the 3% soft drift, under the 8% hard one.
        let drifted = || set_at([Vec3::ZERO, Vec3::X * 100.5, Vec3::X * 200.0]);
        assert!(!cache.resolve(drifted()).cascades[1].refresh);
        assert!(cache.resolve(drifted()).cascades[1].refresh);
    }

    #[test]
    fn drift_under_the_soft_threshold_never_renders() {
        let mut cache = primed_cache();
        let barely = || set_at([Vec3::ZERO, Vec3::X * 100.2, Vec3::X * 200.0]);
        let resolved: Vec<bool> = (0..6)
            .map(|_| cache.resolve(barely()).cascades[1].refresh)
            .collect();
        assert_eq!(resolved, vec![false; 6]);
    }

    #[test]
    fn hard_drift_renders_without_waiting_for_the_interval() {
        let mut cache = primed_cache();
        let resolved = cache.resolve(set_at([Vec3::ZERO, Vec3::X * 101.0, Vec3::X * 200.0]));
        assert!(resolved.cascades[1].refresh);
    }

    #[test]
    fn a_growing_radius_counts_as_drift_but_a_shrinking_one_does_not() {
        let mut cache = primed_cache();
        let grown = |radius| CascadeSet {
            cascades: vec![
                cascade(Vec3::ZERO, 10.0),
                cascade(Vec3::X * 100.0, radius),
                cascade(Vec3::X * 200.0, 10.0),
            ],
            to_sun: SUN,
        };
        assert!(!cache.resolve(grown(5.0)).cascades[1].refresh);
        assert!(!cache.resolve(grown(5.0)).cascades[1].refresh);
        assert!(cache.resolve(grown(11.0)).cascades[1].refresh);
    }

    #[test]
    fn at_most_one_far_cascade_waits_its_turn_per_frame() {
        let mut cache = primed_cache();
        let drifted = || set_at([Vec3::ZERO, Vec3::X * 100.5, Vec3::X * 200.5]);
        cache.resolve(drifted());
        let second_frame = render(&mut cache, drifted());
        assert_eq!(refreshed(&second_frame), vec![false, true, false]);
        let third_frame = render(&mut cache, drifted());
        assert_eq!(refreshed(&third_frame), vec![false, false, true]);
    }

    #[test]
    fn an_overdue_cascade_takes_the_turn_of_a_due_one() {
        let mut cache = primed_cache();
        let drifted = || set_at([Vec3::ZERO, Vec3::X * 100.5, Vec3::X * 201.0]);
        cache.resolve(drifted());
        let second_frame = cache.resolve(drifted());
        assert_eq!(refreshed(&second_frame), vec![false, false, true]);
    }

    #[test]
    fn an_every_frame_cascade_does_not_use_up_the_far_turn() {
        let mut cache = primed_cache();
        let resolved = cache.resolve(set_at([Vec3::X, Vec3::X * 101.0, Vec3::X * 200.0]));
        assert_eq!(refreshed(&resolved), vec![true, true, false]);
    }

    fn invalidated_around(center: Vec3) -> ShadowInvalidations {
        let mut invalidations = ShadowInvalidations::default();
        invalidations.region(Aabb {
            min: center - Vec3::ONE,
            max: center + Vec3::ONE,
        });
        invalidations
    }

    #[test]
    fn a_changed_caster_renders_an_every_frame_cascade_immediately() {
        let mut cache = primed_cache();
        cache.note_invalidations(&invalidated_around(Vec3::ZERO), None);
        assert_eq!(
            refreshed(&cache.resolve(still_set())),
            vec![true, false, false]
        );
    }

    #[test]
    fn a_changed_caster_waits_for_the_longer_stale_interval_of_a_far_cascade() {
        let mut cache = primed_cache();
        cache.note_invalidations(&invalidated_around(Vec3::X * 200.0), None);
        // stale interval of cascade 2 is 8 frames: ages 1..=7 keep, age 8 renders.
        for _ in 0..7 {
            assert!(!cache.resolve(still_set()).cascades[2].refresh);
        }
        assert!(cache.resolve(still_set()).cascades[2].refresh);
    }

    #[test]
    fn a_changed_caster_outside_a_cascade_leaves_it_alone() {
        let mut cache = primed_cache();
        cache.note_invalidations(&invalidated_around(Vec3::X * 5000.0), None);
        for _ in 0..20 {
            assert_eq!(refreshed(&cache.resolve(still_set())), vec![false; 3]);
        }
    }

    #[test]
    fn a_caster_too_small_for_the_cascade_to_draw_is_ignored() {
        let mut cache = primed_cache();
        let mut tiny = ShadowInvalidations::default();
        // Diagonal 0.017 is below min_texels (1) * texel (0.1).
        tiny.region(Aabb {
            min: Vec3::ZERO,
            max: Vec3::splat(0.01),
        });
        cache.note_invalidations(&tiny, None);
        assert_eq!(refreshed(&cache.resolve(still_set())), vec![false; 3]);
    }

    #[test]
    fn a_whole_world_change_renders_every_cascade_of_the_current_frame() {
        let mut cache = primed_cache();
        let mut everything = ShadowInvalidations::default();
        everything.everything();
        let mut current = cache.resolve(still_set());
        assert_eq!(refreshed(&current), vec![false; 3]);
        cache.note_invalidations(&everything, Some(&mut current));
        assert_eq!(refreshed(&current), vec![true; 3]);
        cache.commit(0b111);
        assert_eq!(refreshed(&cache.resolve(still_set())), vec![false; 3]);
    }

    #[test]
    fn a_whole_world_change_drops_the_cache_when_no_frame_is_pending() {
        let mut cache = primed_cache();
        let mut everything = ShadowInvalidations::default();
        everything.everything();
        cache.note_invalidations(&everything, None);
        assert_eq!(refreshed(&cache.resolve(still_set())), vec![true; 3]);
    }

    #[test]
    fn discard_forgets_every_map() {
        let mut cache = primed_cache();
        cache.discard();
        assert_eq!(refreshed(&cache.resolve(still_set())), vec![true; 3]);
    }

    #[test]
    fn a_disabled_cache_renders_everything_and_remembers_nothing() {
        let mut cache = CascadeCache::with_settings(CacheSettings {
            enabled: false,
            ..settings()
        });
        assert_eq!(refreshed(&render(&mut cache, still_set())), vec![true; 3]);
        assert_eq!(refreshed(&render(&mut cache, still_set())), vec![true; 3]);
    }

    #[test]
    fn committed_renders_are_counted_per_cascade_and_reason() {
        let mut cache = primed_cache();
        let counts = cache.refreshes();
        assert_eq!(counts[0][RefreshReason::Missing as usize], 1);
        assert_eq!(counts[2][RefreshReason::Missing as usize], 1);
        render(
            &mut cache,
            set_at([Vec3::X * 0.5, Vec3::X * 100.0, Vec3::X * 200.0]),
        );
        assert_eq!(cache.refreshes()[0][RefreshReason::Drift as usize], 1);
    }

    #[test]
    fn uncommitted_renders_are_not_counted() {
        let mut cache = cache();
        cache.resolve(still_set());
        cache.commit(0);
        assert!(cache.refreshes().iter().flatten().all(|count| *count == 0));
    }

    #[test]
    fn hold_keeps_committed_maps_and_renders_the_frozen_fit_for_the_rest() {
        let mut cache = cache();
        render(&mut cache, still_set());
        cache.discard();
        cache.resolve(still_set());
        cache.commit(0b001);
        let held = cache.hold(&still_set());
        assert_eq!(refreshed(&held), vec![false, true, true]);
        cache.commit(0b110);
        assert_eq!(refreshed(&cache.hold(&still_set())), vec![false; 3]);
    }

    #[test]
    fn hold_never_renders_when_the_cache_is_disabled() {
        let mut cache = CascadeCache::with_settings(CacheSettings {
            enabled: false,
            ..settings()
        });
        assert_eq!(refreshed(&cache.hold(&still_set())), vec![false; 3]);
    }

    #[test]
    fn intervals_override_the_leading_cascades_only() {
        let parsed = parse_intervals("1, 3", [0, 2, 4, 4]);
        assert_eq!(parsed, Some([1, 3, 4, 4]));
    }

    #[test]
    fn intervals_reject_malformed_lists() {
        let defaults = [0, 2, 4, 4];
        for raw in ["", "a", "1,,2", "1,2,3,4,5", "-1", "1.5"] {
            assert_eq!(parse_intervals(raw, defaults), None, "{raw:?}");
        }
    }

    #[test]
    fn drift_needs_a_soft_and_a_hard_value_in_order() {
        assert_eq!(parse_drift("0.02,0.1"), Some((0.02, 0.1)));
        assert_eq!(parse_drift("0.05, 0.05"), Some((0.05, 0.05)));
    }

    #[test]
    fn drift_rejects_malformed_or_inverted_values() {
        for raw in [
            "",
            "0.1",
            "0.1,0.2,0.3",
            "0.2,0.1",
            "-0.1,0.2",
            "nan,1",
            "0.1,inf",
        ] {
            assert_eq!(parse_drift(raw), None, "{raw:?}");
        }
    }
}
