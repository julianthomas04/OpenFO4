//! Sampled scalar / vector tracks of the effect, refraction, light and particle
//! extras, evaluated on the CPU with loop or clamp timing.

use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TrackError {
    #[error("track has no keys")]
    NoKeys,
    #[error("track has {times} times but {values} values")]
    LengthMismatch { times: usize, values: usize },
    #[error("track key {index} is not a finite number")]
    NonFiniteKey { index: usize },
    #[error("track times decrease at key {index}")]
    UnorderedTimes { index: usize },
    #[error("track range {start}..{stop} is not a finite, ordered interval")]
    InvalidRange { start: f32, stop: f32 },
    #[error("track {field} {value} is not finite")]
    NonFiniteTiming { field: &'static str, value: f32 },
    #[error("track target `{target}` needs {expected} values")]
    WrongValueKind {
        target: &'static str,
        expected: &'static str,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cycle {
    Loop,
    Clamp,
}

fn default_frequency() -> f32 {
    1.0
}

/// Maps elapsed seconds to key time: `t = elapsed * frequency + phase`, then
/// wrapped (`loop`) or clamped (`clamp`) into `[start, stop]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawTiming")]
pub struct Timing {
    pub cycle: Cycle,
    pub start: f32,
    pub stop: f32,
    pub frequency: f32,
    pub phase: f32,
}

#[derive(Deserialize)]
struct RawTiming {
    cycle: Cycle,
    start: f32,
    stop: f32,
    #[serde(default = "default_frequency")]
    frequency: f32,
    #[serde(default)]
    phase: f32,
}

impl TryFrom<RawTiming> for Timing {
    type Error = TrackError;

    fn try_from(raw: RawTiming) -> Result<Self, TrackError> {
        let timing = Self {
            cycle: raw.cycle,
            start: raw.start,
            stop: raw.stop,
            frequency: raw.frequency,
            phase: raw.phase,
        };
        timing.validate()?;
        Ok(timing)
    }
}

impl Timing {
    pub fn validate(&self) -> Result<(), TrackError> {
        if !(self.start.is_finite() && self.stop.is_finite() && self.start <= self.stop) {
            return Err(TrackError::InvalidRange {
                start: self.start,
                stop: self.stop,
            });
        }
        for (field, value) in [("frequency", self.frequency), ("phase", self.phase)] {
            if !value.is_finite() {
                return Err(TrackError::NonFiniteTiming { field, value });
            }
        }
        Ok(())
    }

    /// Key time, in the track's absolute seconds, `elapsed` seconds after the
    /// instance started. A zero-length range always yields `start`.
    pub fn key_time(&self, elapsed: f32) -> f32 {
        let time = elapsed * self.frequency + self.phase;
        let duration = self.stop - self.start;
        if duration <= 0.0 {
            return self.start;
        }
        match self.cycle {
            Cycle::Clamp => time.clamp(self.start, self.stop),
            Cycle::Loop => self.start + (time - self.start).rem_euclid(duration),
        }
    }
}

/// Values a key list can interpolate.
pub trait KeyValue: Copy {
    #[must_use]
    fn lerp(self, other: Self, t: f32) -> Self;
    fn is_finite_value(self) -> bool;
}

impl KeyValue for f32 {
    fn lerp(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }

    fn is_finite_value(self) -> bool {
        self.is_finite()
    }
}

impl KeyValue for [f32; 3] {
    fn lerp(self, other: Self, t: f32) -> Self {
        Vec3::from(self).lerp(Vec3::from(other), t).to_array()
    }

    fn is_finite_value(self) -> bool {
        self.iter().all(|c| c.is_finite())
    }
}

/// Validated keys: non-empty, equal lengths, finite, times non-decreasing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    try_from = "RawKeys<V>",
    bound(deserialize = "V: KeyValue + Deserialize<'de>")
)]
pub struct Keys<V: KeyValue> {
    pub times: Vec<f32>,
    pub values: Vec<V>,
}

#[derive(Deserialize)]
struct RawKeys<V> {
    times: Vec<f32>,
    values: Vec<V>,
}

impl<V: KeyValue> TryFrom<RawKeys<V>> for Keys<V> {
    type Error = TrackError;

    fn try_from(raw: RawKeys<V>) -> Result<Self, TrackError> {
        Self::new(raw.times, raw.values)
    }
}

impl<V: KeyValue> Keys<V> {
    pub fn new(times: Vec<f32>, values: Vec<V>) -> Result<Self, TrackError> {
        if times.is_empty() {
            return Err(TrackError::NoKeys);
        }
        if times.len() != values.len() {
            return Err(TrackError::LengthMismatch {
                times: times.len(),
                values: values.len(),
            });
        }
        if let Some(index) =
            (0..times.len()).find(|&i| !times[i].is_finite() || !values[i].is_finite_value())
        {
            return Err(TrackError::NonFiniteKey { index });
        }
        if let Some(index) = (1..times.len()).find(|&i| times[i] < times[i - 1]) {
            return Err(TrackError::UnorderedTimes { index });
        }
        Ok(Self { times, values })
    }

    /// Index of the first key later than `time` (keys are non-empty).
    fn upper(&self, time: f32) -> usize {
        self.times.partition_point(|&t| t <= time)
    }

    /// Linear interpolation, held at the first and last key outside the range.
    pub fn sample_linear(&self, time: f32) -> V {
        let upper = self.upper(time);
        if upper == 0 {
            return self.values[0];
        }
        if upper == self.times.len() {
            return self.values[upper - 1];
        }
        let (t0, t1) = (self.times[upper - 1], self.times[upper]);
        self.values[upper - 1].lerp(self.values[upper], (time - t0) / (t1 - t0))
    }

    /// Value of the latest key at or before `time`; the first key before all.
    pub fn sample_step(&self, time: f32) -> V {
        self.values[self.upper(time).saturating_sub(1)]
    }
}

/// Scalar keys with their own timing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScalarTrack {
    #[serde(flatten)]
    pub timing: Timing,
    #[serde(flatten)]
    pub keys: Keys<f32>,
}

impl ScalarTrack {
    pub fn evaluate(&self, elapsed: f32) -> f32 {
        self.keys.sample_linear(self.timing.key_time(elapsed))
    }
}

/// Boolean timeline stored as 0/1 keys, step-interpolated, with its own timing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepTrack {
    #[serde(flatten)]
    pub timing: Timing,
    #[serde(flatten)]
    pub keys: Keys<f32>,
}

impl StepTrack {
    pub fn evaluate(&self, elapsed: f32) -> bool {
        self.keys.sample_step(self.timing.key_time(elapsed)) != 0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackTarget {
    EmissiveMult,
    Alpha,
    UOffset,
    VOffset,
    UScale,
    VScale,
    EmissiveColor,
    Dimmer,
    LightColor,
    Offset,
    RefractionStrength,
}

impl TrackTarget {
    pub fn name(self) -> &'static str {
        match self {
            Self::EmissiveMult => "emissive_mult",
            Self::Alpha => "alpha",
            Self::UOffset => "u_offset",
            Self::VOffset => "v_offset",
            Self::UScale => "u_scale",
            Self::VScale => "v_scale",
            Self::EmissiveColor => "emissive_color",
            Self::Dimmer => "dimmer",
            Self::LightColor => "light_color",
            Self::Offset => "offset",
            Self::RefractionStrength => "refraction_strength",
        }
    }

    pub fn is_vector(self) -> bool {
        matches!(self, Self::EmissiveColor | Self::LightColor | Self::Offset)
    }
}

/// Value a [`Track`] produces: scalar targets give `Scalar`, `emissive_color`
/// (rgb), `light_color` (rgb) and `offset` (eu) give `Vec3`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrackValue {
    Scalar(f32),
    Vec3(Vec3),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TrackKeys {
    Scalar(Keys<f32>),
    Vec3(Keys<[f32; 3]>),
}

/// A material / light track (`extras.effect.controllers[]`, refraction and
/// light extras): a named target, timing and sampled keys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawTrack")]
pub struct Track {
    pub target: TrackTarget,
    #[serde(flatten)]
    pub timing: Timing,
    #[serde(flatten)]
    pub keys: TrackKeys,
}

#[derive(Deserialize)]
struct RawTrack {
    target: TrackTarget,
    #[serde(flatten)]
    timing: Timing,
    #[serde(flatten)]
    keys: TrackKeys,
}

impl TryFrom<RawTrack> for Track {
    type Error = TrackError;

    fn try_from(raw: RawTrack) -> Result<Self, TrackError> {
        let vector_keys = matches!(raw.keys, TrackKeys::Vec3(_));
        if vector_keys != raw.target.is_vector() {
            return Err(TrackError::WrongValueKind {
                target: raw.target.name(),
                expected: if raw.target.is_vector() {
                    "[x, y, z]"
                } else {
                    "scalar"
                },
            });
        }
        Ok(Self {
            target: raw.target,
            timing: raw.timing,
            keys: raw.keys,
        })
    }
}

impl Track {
    pub fn evaluate(&self, elapsed: f32) -> TrackValue {
        let time = self.timing.key_time(elapsed);
        match &self.keys {
            TrackKeys::Scalar(keys) => TrackValue::Scalar(keys.sample_linear(time)),
            TrackKeys::Vec3(keys) => TrackValue::Vec3(Vec3::from(keys.sample_linear(time))),
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals or exact interpolations of them"
)]
mod tests {
    use super::*;
    use serde_json::json;

    fn timing(cycle: Cycle, start: f32, stop: f32) -> Timing {
        Timing {
            cycle,
            start,
            stop,
            frequency: 1.0,
            phase: 0.0,
        }
    }

    fn ramp(cycle: Cycle) -> ScalarTrack {
        ScalarTrack {
            timing: timing(cycle, 1.0, 3.0),
            keys: Keys::new(vec![1.0, 2.0, 3.0], vec![0.0, 10.0, 0.0]).unwrap(),
        }
    }

    #[test]
    fn linear_interpolates_between_keys() {
        let track = ramp(Cycle::Clamp);
        assert_eq!(track.evaluate(1.5), 5.0);
        assert_eq!(track.evaluate(2.5), 5.0);
        assert_eq!(track.evaluate(2.0), 10.0);
    }

    #[test]
    fn clamp_holds_both_ends() {
        let track = ramp(Cycle::Clamp);
        assert_eq!(track.evaluate(-5.0), 0.0);
        assert_eq!(track.evaluate(100.0), 0.0);
        let rising = ScalarTrack {
            timing: timing(Cycle::Clamp, 0.0, 2.0),
            keys: Keys::new(vec![0.0, 2.0], vec![1.0, 5.0]).unwrap(),
        };
        assert_eq!(rising.evaluate(100.0), 5.0);
    }

    #[test]
    fn loop_wraps_into_the_range() {
        let track = ScalarTrack {
            timing: timing(Cycle::Loop, 1.0, 3.0),
            keys: Keys::new(vec![1.0, 3.0], vec![0.0, 8.0]).unwrap(),
        };
        assert_eq!(track.evaluate(1.5), 2.0);
        assert_eq!(track.evaluate(3.5), 2.0);
        assert_eq!(track.evaluate(7.5), 2.0);
        assert_eq!(track.evaluate(0.0), 4.0);
    }

    #[test]
    fn frequency_and_phase_scale_elapsed_time() {
        let mut track = ScalarTrack {
            timing: timing(Cycle::Clamp, 0.0, 10.0),
            keys: Keys::new(vec![0.0, 10.0], vec![0.0, 10.0]).unwrap(),
        };
        track.timing.frequency = 2.0;
        track.timing.phase = 1.0;
        assert_eq!(track.evaluate(2.0), 5.0);
    }

    #[test]
    fn zero_length_range_yields_start() {
        let track = ScalarTrack {
            timing: timing(Cycle::Loop, 0.0, 0.0),
            keys: Keys::new(vec![0.0], vec![7.0]).unwrap(),
        };
        assert_eq!(track.evaluate(123.0), 7.0);
    }

    #[test]
    fn step_track_holds_the_latest_key() {
        let track = StepTrack {
            timing: timing(Cycle::Clamp, 0.0, 3.33),
            keys: Keys::new(vec![0.0, 3.33], vec![1.0, 0.0]).unwrap(),
        };
        assert!(track.evaluate(0.0));
        assert!(track.evaluate(3.0));
        assert!(!track.evaluate(3.33));
        assert!(!track.evaluate(50.0));
    }

    #[test]
    fn vec3_track_interpolates_componentwise() {
        let track: Track = serde_json::from_value(json!({
            "target": "offset", "cycle": "loop", "start": 0.0, "stop": 2.0,
            "frequency": 1.0, "phase": 0.0,
            "times": [0.0, 2.0], "values": [[0.0, 0.0, 0.0], [2.0, 4.0, -2.0]]
        }))
        .unwrap();
        assert_eq!(
            track.evaluate(1.0),
            TrackValue::Vec3(Vec3::new(1.0, 2.0, -1.0))
        );
    }

    #[test]
    fn light_color_track_parses_as_a_vector_target() {
        let track: Track = serde_json::from_value(json!({
            "target": "light_color", "cycle": "loop", "start": 0.0, "stop": 1.0,
            "times": [0.0, 1.0], "values": [[1.0, 0.5, 0.0], [0.0, 0.5, 1.0]]
        }))
        .unwrap();
        assert_eq!(track.target, TrackTarget::LightColor);
        assert_eq!(
            track.evaluate(0.5),
            TrackValue::Vec3(Vec3::new(0.5, 0.5, 0.5))
        );
    }

    #[test]
    fn light_color_track_rejects_scalar_values() {
        let result: Result<Track, _> = serde_json::from_value(json!({
            "target": "light_color", "cycle": "clamp", "start": 0.0, "stop": 1.0,
            "times": [0.0], "values": [1.0]
        }));
        assert!(result.unwrap_err().to_string().contains("light_color"));
    }

    #[test]
    fn keys_reject_malformed_input() {
        assert_eq!(Keys::<f32>::new(vec![], vec![]), Err(TrackError::NoKeys));
        assert_eq!(
            Keys::new(vec![0.0, 1.0], vec![1.0]),
            Err(TrackError::LengthMismatch {
                times: 2,
                values: 1
            })
        );
        assert_eq!(
            Keys::new(vec![1.0, 0.5], vec![1.0, 2.0]),
            Err(TrackError::UnorderedTimes { index: 1 })
        );
        assert_eq!(
            Keys::new(vec![0.0, 1.0], vec![1.0, f32::NAN]),
            Err(TrackError::NonFiniteKey { index: 1 })
        );
    }

    #[test]
    fn timing_rejects_reversed_range() {
        let error = timing(Cycle::Loop, 3.0, 1.0).validate().unwrap_err();
        assert_eq!(
            error,
            TrackError::InvalidRange {
                start: 3.0,
                stop: 1.0
            }
        );
    }

    #[test]
    fn track_rejects_scalar_values_for_a_vector_target() {
        let result: Result<Track, _> = serde_json::from_value(json!({
            "target": "emissive_color", "cycle": "clamp", "start": 0.0, "stop": 1.0,
            "times": [0.0], "values": [1.0]
        }));
        assert!(result.unwrap_err().to_string().contains("emissive_color"));
    }

    #[test]
    fn track_round_trips_through_json() {
        let json = json!({
            "target": "emissive_mult", "cycle": "loop", "start": 0.0, "stop": 50.0,
            "frequency": 1.0, "phase": 0.0, "times": [0.0, 0.0333], "values": [2.6, 2.7]
        });
        let track: Track = serde_json::from_value(json).unwrap();
        assert_eq!(track.target, TrackTarget::EmissiveMult);
        let again: Track = serde_json::from_value(serde_json::to_value(&track).unwrap()).unwrap();
        assert_eq!(track, again);
    }
}
