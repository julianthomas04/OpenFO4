//! Animation of placed lights by their light model: a `dimmer`, `light_color` and `offset`
//! track per model, evaluated on the CPU. The animated light is written over the cell's slab
//! entry every frame; the cell keeps the placed light as it was, so a slab repack stays correct.

use std::collections::HashMap;
use std::path::PathBuf;

use glam::{Mat3, Vec3};

use super::effects::lights_pick::PickableLight;
use super::gpu_slab::GpuSlab;
use super::particles::{BackgroundCache, Lookup};
use super::placed_light::GpuPlacedLight;
use super::world_streamer::{LightKey, ModelLight, WorldStreamer};
use crate::anim::scalar_track::{Track, TrackKeys, TrackTarget, TrackValue};
use crate::assets::types::model_extras::{LightExtras, ModelExtras};

/// Rest values at or below this cannot normalise anything.
const MIN_REST: f32 = 1e-6;

/// A light animates while the camera is within this distance (eu) of its reach; beyond it
/// the slab keeps its last state, which nothing near enough to notice depends on.
pub const ANIMATION_RANGE_EU: f32 = 4096.0;

#[derive(Debug, thiserror::Error)]
pub enum LightAnimationError {
    #[error("model file unreadable: {0}")]
    Unreadable(#[from] gltf::Error),
}

/// How one light model animates a light, relative to the light's rest.
#[derive(Debug)]
pub struct LightAnimation {
    /// The dimmer the light model rests at; sampled dimmers are divided by it.
    dimmer_rest: f32,
    dimmer: Option<Track>,
    colour: Option<Track>,
    /// The colour the light model rests at (its colour track's first key).
    colour_rest: Vec3,
    offset: Option<Track>,
    /// Length of the dimmer track in seconds, which a light's flicker phase is a fraction of.
    period: f32,
}

/// The animated state of a light at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightSample {
    /// Factor on the placed colour from the dimmer track (1 at rest).
    pub dimmer: f32,
    /// Per-channel factor on the placed colour from the colour track (1 at rest).
    pub tint: Vec3,
    /// Model-space displacement of the light from its rest position, engine units.
    pub offset: Vec3,
}

impl LightSample {
    pub const REST: Self = Self {
        dimmer: 1.0,
        tint: Vec3::ONE,
        offset: Vec3::ZERO,
    };
}

impl LightAnimation {
    /// `None` when the model's light has no track that moves it.
    pub fn from_extras(extras: &LightExtras, model: &str) -> Option<Self> {
        let find = |target: TrackTarget| {
            extras
                .tracks
                .iter()
                .find(|track| track.target == target)
                .cloned()
        };
        let dimmer = find(TrackTarget::Dimmer);
        let colour = find(TrackTarget::LightColor);
        let offset = find(TrackTarget::Offset);
        if dimmer.is_none() && colour.is_none() && offset.is_none() {
            return None;
        }
        let dimmer_rest = if extras.dimmer_rest.is_finite() && extras.dimmer_rest > MIN_REST {
            extras.dimmer_rest
        } else {
            crate::utils::warn_once(&format!("light_dimmer_rest:{model}"), || {
                format!(
                    "light model {model}: dimmer rest {} cannot normalise a dimmer; treating it as 1",
                    extras.dimmer_rest
                )
            });
            1.0
        };
        let colour_rest = colour.as_ref().map_or(Vec3::ONE, first_colour);
        let period = dimmer.as_ref().map_or(0.0, |track| {
            (track.timing.stop - track.timing.start).max(0.0)
        });
        Some(Self {
            dimmer_rest,
            dimmer,
            colour,
            colour_rest,
            offset,
            period,
        })
    }

    pub fn period(&self) -> f32 {
        self.period
    }

    pub fn sample(&self, elapsed: f32) -> LightSample {
        let dimmer = self
            .dimmer
            .as_ref()
            .and_then(|track| scalar(track.evaluate(elapsed)))
            .map_or(1.0, |value| value / self.dimmer_rest);
        let tint = self
            .colour
            .as_ref()
            .and_then(|track| vector(track.evaluate(elapsed)))
            .map_or(Vec3::ONE, |value| normalised(value, self.colour_rest));
        let offset = self
            .offset
            .as_ref()
            .and_then(|track| vector(track.evaluate(elapsed)))
            .unwrap_or(Vec3::ZERO);
        LightSample {
            dimmer,
            tint,
            offset,
        }
    }
}

fn scalar(value: TrackValue) -> Option<f32> {
    match value {
        TrackValue::Scalar(value) => Some(value),
        TrackValue::Vec3(_) => None,
    }
}

fn vector(value: TrackValue) -> Option<Vec3> {
    match value {
        TrackValue::Vec3(value) => Some(value),
        TrackValue::Scalar(_) => None,
    }
}

fn first_colour(track: &Track) -> Vec3 {
    match &track.keys {
        TrackKeys::Vec3(keys) => Vec3::from(keys.values[0]),
        TrackKeys::Scalar(_) => Vec3::ONE,
    }
}

/// `value / rest` per channel; a channel with no rest value stays as it is.
fn normalised(value: Vec3, rest: Vec3) -> Vec3 {
    Vec3::new(
        channel(value.x, rest.x),
        channel(value.y, rest.y),
        channel(value.z, rest.z),
    )
}

fn channel(value: f32, rest: f32) -> f32 {
    if rest > MIN_REST { value / rest } else { 1.0 }
}

/// Whether the camera is close enough to the light's reach for its animation to matter.
fn in_animation_range(light: &GpuPlacedLight, camera_position: Vec3) -> bool {
    let reach = ANIMATION_RANGE_EU + light.radius;
    Vec3::from(light.position).distance_squared(camera_position) <= reach * reach
}

/// A light with `sample` applied: colour scaled by dimmer and tint, position moved by the
/// offset through the placement frame.
pub fn animated_light(base: &GpuPlacedLight, frame: &Mat3, sample: &LightSample) -> GpuPlacedLight {
    let mut light = *base;
    let colour = Vec3::from(base.color) * sample.tint * sample.dimmer;
    light.color = colour.to_array();
    light.position = (Vec3::from(base.position) + *frame * sample.offset).to_array();
    light
}

/// Loads and caches the animation of each light model file on a worker thread; a light whose
/// model is still loading stays static meanwhile.
struct LightModelCache {
    animations: BackgroundCache<LightAnimation>,
}

impl LightModelCache {
    fn new(asset_root: &str) -> Self {
        let asset_root = PathBuf::from(asset_root);
        Self {
            animations: BackgroundCache::new("light-models", move |model| {
                match load_animation(&asset_root.join(model), model) {
                    Ok(animation) => animation,
                    Err(error) => {
                        crate::utils::warn_once(&format!("light_model:{model}"), || {
                            format!("light model {model}: {error}; its lights stay static")
                        });
                        None
                    }
                }
            }),
        }
    }

    fn get(&mut self, model: &str) -> Option<std::sync::Arc<LightAnimation>> {
        match self.animations.get(model) {
            Lookup::Ready(animation) => Some(animation),
            Lookup::Pending | Lookup::Unavailable => None,
        }
    }
}

fn load_animation(
    path: &std::path::Path,
    model: &str,
) -> Result<Option<LightAnimation>, LightAnimationError> {
    let document = gltf::Gltf::open(path)?.document;
    let extras = ModelExtras::from_document(&document);
    for rejected in &extras.rejected {
        crate::utils::warn_once(
            &format!("light-extras:{model}:{}", rejected.node_name),
            || {
                format!(
                    "light model {model}: node {:?}: {}; ignoring it",
                    rejected.node_name, rejected.error
                )
            },
        );
    }
    Ok(extras
        .lights
        .first()
        .and_then(|node| LightAnimation::from_extras(&node.value, model)))
}

/// One animated light for this frame.
#[derive(Debug, Clone, Copy)]
pub struct LightWrite {
    pub slab_index: Option<u32>,
    /// Final colour (dimmer applied), position moved.
    pub light: GpuPlacedLight,
}

/// What effect lighting needs of an animated light.
#[derive(Debug, Clone, Copy)]
struct PickState {
    /// Colour before the dimmer, position moved.
    light: GpuPlacedLight,
    dimmer: f32,
}

/// Animates every placed light whose light model has tracks.
pub struct LightAnimator {
    cache: LightModelCache,
    writes: Vec<LightWrite>,
    /// Consecutive slab entries written in one call.
    run: Vec<GpuPlacedLight>,
    picks: HashMap<LightKey, PickState>,
    animated_total: usize,
}

impl LightAnimator {
    pub fn new(asset_root: &str) -> Self {
        Self {
            cache: LightModelCache::new(asset_root),
            writes: Vec::new(),
            run: Vec::new(),
            picks: HashMap::new(),
            animated_total: 0,
        }
    }

    /// Animates every model light of `streamer` within reach of the camera at `elapsed`
    /// seconds and writes the result over the slab entries.
    pub fn update(
        &mut self,
        streamer: &WorldStreamer,
        elapsed: f32,
        camera_position: Vec3,
        slab: &GpuSlab<GpuPlacedLight>,
        queue: &wgpu::Queue,
    ) {
        self.begin_frame();
        streamer.for_each_model_light(|light| {
            if in_animation_range(light.base, camera_position) {
                self.animate(light, elapsed);
            }
        });
        self.flush(|offset, lights| slab.write(offset, lights, queue));
        self.log_growth();
    }

    /// Hands the pending writes to `write` as runs of consecutive slab entries.
    fn flush(&mut self, mut write: impl FnMut(u32, &[GpuPlacedLight])) {
        self.writes.sort_unstable_by_key(|write| write.slab_index);
        let mut start = 0;
        self.run.clear();
        for entry in &self.writes {
            let Some(index) = entry.slab_index else {
                continue;
            };
            if !self.run.is_empty() && index != start + self.run.len() as u32 {
                write(start, &self.run);
                self.run.clear();
            }
            if self.run.is_empty() {
                start = index;
            }
            self.run.push(entry.light);
        }
        if !self.run.is_empty() {
            write(start, &self.run);
        }
    }

    fn begin_frame(&mut self) {
        self.writes.clear();
        self.picks.clear();
    }

    pub fn animate(&mut self, light: ModelLight<'_>, elapsed: f32) {
        let Some(animation) = self.cache.get(light.model) else {
            return;
        };
        let phase_seconds = light.base.flicker[3] * animation.period();
        let sample = animation.sample(elapsed + phase_seconds);
        let moved = animated_light(
            light.base,
            &light.frame,
            &LightSample {
                dimmer: 1.0,
                ..sample
            },
        );
        self.writes.push(LightWrite {
            slab_index: light.slab_index,
            light: animated_light(light.base, &light.frame, &sample),
        });
        self.picks.insert(
            light.key,
            PickState {
                light: moved,
                dimmer: sample.dimmer,
            },
        );
    }

    pub fn writes(&self) -> &[LightWrite] {
        &self.writes
    }

    /// The light as effect lighting sees it: animated when its model animates it.
    pub fn pickable(&self, key: LightKey, placed: &GpuPlacedLight) -> PickableLight {
        match self.picks.get(&key) {
            Some(state) => PickableLight {
                light: state.light,
                dimmer: state.dimmer,
            },
            None => PickableLight::steady(*placed),
        }
    }

    fn log_growth(&mut self) {
        if self.writes.len() != self.animated_total {
            self.animated_total = self.writes.len();
            tracing::info!("animated lights: {}", self.animated_total);
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals or exact interpolations of them"
)]
mod tests {
    use bytemuck::Zeroable;
    use serde_json::json;

    use super::*;

    fn track(target: &str, values: &serde_json::Value) -> Track {
        serde_json::from_value(json!({
            "target": target, "cycle": "loop", "start": 0.0, "stop": 2.0,
            "times": [0.0, 2.0], "values": values
        }))
        .unwrap()
    }

    fn extras(dimmer_rest: f32, tracks: Vec<Track>) -> LightExtras {
        LightExtras {
            v: 1,
            kind: crate::assets::types::model_extras::LightKind::Point,
            dimmer_rest,
            tracks,
        }
    }

    fn base() -> GpuPlacedLight {
        GpuPlacedLight {
            position: [1.0, 2.0, 3.0],
            radius: 10.0,
            color: [0.5, 0.25, 0.125],
            flicker: [0.0, 0.0, 0.0, 0.5],
            ..GpuPlacedLight::zeroed()
        }
    }

    fn model_light<'a>(base: &'a GpuPlacedLight, model: &'a str) -> ModelLight<'a> {
        ModelLight {
            key: LightKey {
                residency: 1,
                light_index: 0,
            },
            slab_index: Some(7),
            base,
            model,
            frame: Mat3::IDENTITY,
        }
    }

    #[test]
    fn dimmer_is_normalised_by_its_rest_value() {
        let animation = LightAnimation::from_extras(
            &extras(2.0, vec![track("dimmer", &json!([1.0, 5.0]))]),
            "m",
        )
        .unwrap();
        assert_eq!(animation.sample(0.0).dimmer, 0.5);
        assert_eq!(animation.sample(1.0).dimmer, 1.5);
    }

    #[test]
    fn zero_dimmer_rest_is_treated_as_one() {
        for rest in [0.0, -1.0, f32::NAN] {
            let animation = LightAnimation::from_extras(
                &extras(rest, vec![track("dimmer", &json!([1.0, 5.0]))]),
                "zero-rest",
            )
            .unwrap();
            assert_eq!(animation.sample(0.0).dimmer, 1.0, "rest {rest}");
            assert!(animation.sample(1.0).dimmer.is_finite());
        }
    }

    #[test]
    fn colour_track_is_relative_to_its_first_key() {
        let animation = LightAnimation::from_extras(
            &extras(
                1.0,
                vec![track(
                    "light_color",
                    &json!([[1.0, 0.5, 0.0], [0.5, 0.5, 1.0]]),
                )],
            ),
            "m",
        )
        .unwrap();
        assert_eq!(animation.sample(0.0).tint, Vec3::ONE);
        assert_eq!(animation.sample(2.0 - 1e-3).tint.y, 1.0);
        let end = animation.sample(1.0);
        assert_eq!(end.tint.x, 0.75);
        assert_eq!(end.tint.z, 1.0, "a channel resting at zero stays unscaled");
    }

    #[test]
    fn a_model_without_tracks_does_not_animate() {
        assert!(LightAnimation::from_extras(&extras(1.0, Vec::new()), "m").is_none());
    }

    #[test]
    fn animated_light_scales_colour_and_moves_through_the_frame() {
        let sample = LightSample {
            dimmer: 2.0,
            tint: Vec3::new(1.0, 0.5, 1.0),
            offset: Vec3::new(1.0, 0.0, 0.0),
        };
        let frame = Mat3::from_diagonal(Vec3::splat(2.0));

        let light = animated_light(&base(), &frame, &sample);

        assert_eq!(light.color, [1.0, 0.25, 0.25]);
        assert_eq!(light.position, [3.0, 2.0, 3.0]);
        assert_eq!(light.radius, 10.0);
    }

    #[test]
    fn resting_sample_leaves_the_light_unchanged() {
        let light = animated_light(&base(), &Mat3::IDENTITY, &LightSample::REST);
        assert_eq!(light.color, base().color);
        assert_eq!(light.position, base().position);
    }

    #[test]
    fn animator_without_a_model_file_leaves_the_light_static() {
        let mut animator = LightAnimator::new("definitely/not/an/asset/root");
        animator.begin_frame();
        let placed = base();

        animator.animate(model_light(&placed, "meshes/missing_light.glb"), 1.0);
        animator.cache.animations.wait_idle();
        animator.animate(model_light(&placed, "meshes/missing_light.glb"), 1.0);

        assert!(animator.writes().is_empty());
        let key = LightKey {
            residency: 1,
            light_index: 0,
        };
        assert_eq!(animator.pickable(key, &placed).dimmer, 1.0);
    }

    #[test]
    fn animator_feeds_the_dimmer_to_effect_lighting_and_the_slab() {
        let mut animator = LightAnimator::new("unused");
        let animation = LightAnimation::from_extras(
            &extras(2.0, vec![track("dimmer", &json!([1.0, 5.0]))]),
            "m",
        )
        .unwrap();
        animator
            .cache
            .animations
            .insert_ready("meshes/fire.glb", Some(animation));
        animator.begin_frame();
        let placed = base();

        // Phase 0.5 of a 2 s period shifts the sample by one second.
        animator.animate(model_light(&placed, "meshes/fire.glb"), 0.0);

        let write = animator.writes()[0];
        assert_eq!(write.slab_index, Some(7));
        assert_eq!(write.light.color, [0.75, 0.375, 0.1875]);
        let key = LightKey {
            residency: 1,
            light_index: 0,
        };
        let pick = animator.pickable(key, &placed);
        assert_eq!(pick.dimmer, 1.5);
        assert_eq!(
            pick.light.color, placed.color,
            "the dimmer is not applied twice"
        );
    }

    fn write_at(slab_index: Option<u32>, red: f32) -> LightWrite {
        LightWrite {
            slab_index,
            light: GpuPlacedLight {
                color: [red, 0.0, 0.0],
                ..GpuPlacedLight::zeroed()
            },
        }
    }

    #[test]
    fn writes_are_batched_into_runs_of_consecutive_slab_entries() {
        let mut animator = LightAnimator::new("unused");
        animator.writes = vec![
            write_at(Some(11), 4.0),
            write_at(Some(5), 1.0),
            write_at(None, 9.0),
            write_at(Some(6), 2.0),
            write_at(Some(10), 3.0),
        ];
        let mut calls = Vec::new();

        animator.flush(|offset, lights| {
            calls.push((
                offset,
                lights.iter().map(|l| l.color[0]).collect::<Vec<_>>(),
            ));
        });

        assert_eq!(
            calls,
            vec![(5, vec![1.0, 2.0]), (10, vec![3.0, 4.0])],
            "an unplaced light is not written and a gap splits the run"
        );
    }

    #[test]
    fn only_lights_within_reach_of_the_camera_animate() {
        let light = GpuPlacedLight {
            position: [ANIMATION_RANGE_EU + 5.0, 0.0, 0.0],
            radius: 10.0,
            ..GpuPlacedLight::zeroed()
        };
        assert!(
            in_animation_range(&light, Vec3::ZERO),
            "its reach touches the range"
        );
        assert!(!in_animation_range(&light, Vec3::new(-10.0, 0.0, 0.0)));
    }
}
