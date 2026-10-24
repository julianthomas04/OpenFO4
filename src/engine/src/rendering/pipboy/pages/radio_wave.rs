//! The RADIO page's live waveform: a continuous sine stream that scrolls ten samples per timer
//! tick, with the stream's frequency, amplitude and phase evolving as the game's wave clip does.

use std::collections::VecDeque;
use std::f32::consts::PI;

use crate::rendering::pipboy::shapes::{Shade, code_colour};
use crate::ui_layout::{
    LiteralKey, ModuleKeys, NumberKey, UiLayout, UiLayoutError, key_check_fields,
};

const MOVIE: &str = "Pipboy_RadioPage";

pub(super) struct WaveKeys {
    width: NumberKey,
    quiet_amplitude: NumberKey,
    initial_amplitude: NumberKey,
    max_amplitude: LiteralKey,
    tick_milliseconds: LiteralKey,
    samples_per_tick: LiteralKey,
    phase_step: LiteralKey,
    turn_factor: LiteralKey,
    rolls: LiteralKey,
    reset_below: LiteralKey,
    amplitude_floor: LiteralKey,
    start_randoms: LiteralKey,
    frequency_base: LiteralKey,
    line_width: LiteralKey,
    line_colour: LiteralKey,
    bright: LiteralKey,
    alpha_divisor: LiteralKey,
    lerp: [LiteralKey; 4],
}

pub(super) static KEYS: WaveKeys = WaveKeys {
    width: NumberKey::new(MOVIE, "RadioWave.WAVE_WIDTH"),
    quiet_amplitude: NumberKey::new(MOVIE, "RadioWave.NO_AMP"),
    initial_amplitude: NumberKey::new(MOVIE, "RadioWave.amp"),
    max_amplitude: LiteralKey::new(MOVIE, "RadioWave.<cinit>#MAX_AMP=", 1),
    tick_milliseconds: LiteralKey::new(MOVIE, "Pipboy_RadioPage.<init>#new Timer(0)", 1),
    samples_per_tick: LiteralKey::new(MOVIE, "RadioWave.Animate#$2=", 1),
    phase_step: LiteralKey::new(MOVIE, "RadioWave.GetWavePoint#offset+", 1),
    turn_factor: LiteralKey::new(MOVIE, "RadioWave.GetWavePoint#*Math.PI", 1),
    rolls: LiteralKey::new(MOVIE, "RadioWave.GetWavePoint#Math.random()*", 2),
    reset_below: LiteralKey::new(MOVIE, "RadioWave.GetWavePoint#(Math.random()*100)<", 1),
    amplitude_floor: LiteralKey::new(MOVIE, "RadioWave.GetWavePoint#(Math.random()*60)+", 1),
    start_randoms: LiteralKey::new(MOVIE, "RadioWave.<init>#Math.random()*", 4),
    frequency_base: LiteralKey::new(MOVIE, "RadioWave.<init>#-(Math.random()*4)", 1),
    line_width: LiteralKey::new(MOVIE, "RadioWave.Animate#graphics.lineStyle(0)", 1),
    line_colour: LiteralKey::new(MOVIE, "RadioWave.Animate#graphics.lineStyle(1)", 1),
    bright: LiteralKey::new(MOVIE, "RadioWave.<cinit>#COLOR_BRIGHT=", 1),
    alpha_divisor: LiteralKey::new(MOVIE, "RadioWave.Animate#darkness/", 1),
    lerp: [
        LiteralKey::new(
            MOVIE,
            "Pipboy_RadioPage.updateRadioFrequency#GlobalFunc.Lerp(0)",
            1,
        ),
        LiteralKey::new(
            MOVIE,
            "Pipboy_RadioPage.updateRadioFrequency#GlobalFunc.Lerp(1)",
            1,
        ),
        LiteralKey::new(
            MOVIE,
            "Pipboy_RadioPage.updateRadioFrequency#GlobalFunc.Lerp(2)",
            1,
        ),
        LiteralKey::new(
            MOVIE,
            "Pipboy_RadioPage.updateRadioFrequency#GlobalFunc.Lerp(3)",
            1,
        ),
    ],
};

key_check_fields!(WaveKeys {
    width,
    quiet_amplitude,
    initial_amplitude,
    max_amplitude,
    tick_milliseconds,
    samples_per_tick,
    phase_step,
    turn_factor,
    rolls,
    reset_below,
    amplitude_floor,
    start_randoms,
    frequency_base,
    line_width,
    line_colour,
    bright,
    alpha_divisor,
    lerp,
});

fn validate(layout: &UiLayout) -> Result<(), UiLayoutError> {
    WaveParams::resolve(layout).map(drop)
}

pub static LAYOUT_KEYS: ModuleKeys = ModuleKeys {
    keys: &[&KEYS],
    resolve: validate,
};

/// Everything the wave clip's code fixes: its size, timing, amplitude rules and look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaveParams {
    pub width: usize,
    pub max_amplitude: f32,
    pub quiet_amplitude: f32,
    pub initial_amplitude: f32,
    pub tick_seconds: f64,
    pub samples_per_tick: usize,
    pub phase_step: f32,
    pub turn_factor: f32,
    pub roll_range: f32,
    pub reset_below: f32,
    pub amplitude_span: f32,
    pub amplitude_floor: f32,
    pub start_phase_turns: f32,
    pub frequency_span: f32,
    pub frequency_base: f32,
    pub line_width: f32,
    pub line_shade: Shade,
    pub line_alpha: f32,
    /// The station frequency range and the cycle range it maps onto: `(from_cycles, to_cycles,
    /// from_mhz, to_mhz)`.
    pub lerp: [f32; 4],
}

impl WaveParams {
    pub fn resolve(layout: &UiLayout) -> Result<Self, UiLayoutError> {
        let number = |key: &LiteralKey, index: usize| layout.lit_number(key, index);
        let lerp = |index: usize| number(&KEYS.lerp[index], 0).map(|value| value as f32);
        let width = layout.whole_constant(&KEYS.width)?;
        let samples = layout.lit_whole(&KEYS.samples_per_tick, 0)?;
        if width < 1 || samples < 1 {
            return Err(UiLayoutError::unresolved(
                MOVIE,
                KEYS.width.path,
                "the wave has no samples",
            ));
        }
        Ok(Self {
            width: width as usize,
            max_amplitude: number(&KEYS.max_amplitude, 0)? as f32,
            quiet_amplitude: layout.get(&KEYS.quiet_amplitude)? as f32,
            initial_amplitude: layout.get(&KEYS.initial_amplitude)? as f32,
            tick_seconds: number(&KEYS.tick_milliseconds, 0)? / 1000.0,
            samples_per_tick: samples as usize,
            phase_step: number(&KEYS.phase_step, 0)? as f32,
            turn_factor: number(&KEYS.turn_factor, 0)? as f32,
            roll_range: number(&KEYS.rolls, 0)? as f32,
            reset_below: number(&KEYS.reset_below, 0)? as f32,
            amplitude_span: number(&KEYS.rolls, 1)? as f32,
            amplitude_floor: number(&KEYS.amplitude_floor, 0)? as f32,
            start_phase_turns: number(&KEYS.start_randoms, 0)? as f32,
            frequency_span: number(&KEYS.start_randoms, 1)? as f32,
            frequency_base: number(&KEYS.frequency_base, 0)? as f32,
            line_width: number(&KEYS.line_width, 0)? as f32,
            line_shade: code_colour(layout, &KEYS.line_colour, 0)?,
            line_alpha: (number(&KEYS.bright, 0)? / number(&KEYS.alpha_divisor, 0)?) as f32,
            lerp: [lerp(0)?, lerp(1)?, lerp(2)?, lerp(3)?],
        })
    }

    /// The whole number of cycles across the trace for a station on `mhz`.
    pub fn cycles_for_frequency(&self, mhz: f32) -> f32 {
        let [from_cycles, to_cycles, from_mhz, to_mhz] = self.lerp;
        let t = ((mhz - from_mhz) / (to_mhz - from_mhz)).clamp(0.0, 1.0);
        (from_cycles + t * (to_cycles - from_cycles)).trunc()
    }
}

/// `SplitMix64`: a tiny deterministic generator standing in for the engine's random numbers.
#[derive(Clone, Debug)]
struct Noise(u64);

impl Noise {
    fn next_unit(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / (1_u64 << 24) as f32
    }
}

/// One wave clip: the queue of points it has generated, the first `width` of which it draws.
#[derive(Clone, Debug)]
pub struct Waveform {
    params: WaveParams,
    points: VecDeque<f32>,
    noise: Noise,
    phase: f32,
    frequency_offset: f32,
    amplitude: f32,
    previous_sine: f32,
    wave_index: u64,
    ticks_done: u64,
}

impl Waveform {
    pub fn new(params: WaveParams, seed: u64) -> Self {
        let mut noise = Noise(seed);
        let phase = noise.next_unit() * params.start_phase_turns * PI;
        let frequency_offset = params.frequency_base - noise.next_unit() * params.frequency_span;
        Self {
            params,
            points: VecDeque::new(),
            noise,
            phase,
            frequency_offset,
            amplitude: params.initial_amplitude,
            previous_sine: 0.0,
            wave_index: 0,
            ticks_done: 0,
        }
    }

    /// The vertical position of each drawn point inside the clip, left to right.
    pub fn points(&self) -> impl ExactSizeIterator<Item = f32> + '_ {
        self.points.iter().copied().take(self.params.width)
    }

    /// Queues a whole trace's worth of new points at `cycles` across the trace.
    pub fn init(&mut self, cycles: f32) {
        for _ in 0..self.params.width {
            let point = self.next_point(cycles);
            self.points.push_back(point);
        }
    }

    /// Runs every timer tick due `seconds` after the page opened, each dropping the oldest
    /// samples and queuing as many new ones. A long gap runs at most a full trace of ticks.
    pub fn advance_to(&mut self, seconds: f64, cycles: f32) {
        let due = (seconds / self.params.tick_seconds).floor().max(0.0) as u64;
        let ticks = due.saturating_sub(self.ticks_done);
        let most = (self.params.width / self.params.samples_per_tick + 1) as u64;
        for _ in 0..ticks.min(most) {
            self.animate(cycles);
        }
        self.ticks_done = self.ticks_done.max(due);
    }

    fn animate(&mut self, cycles: f32) {
        for _ in 0..self.params.samples_per_tick.min(self.points.len()) {
            self.points.pop_front();
        }
        for _ in 0..self.params.samples_per_tick {
            let point = self.next_point(cycles);
            self.points.push_back(point);
        }
    }

    fn next_point(&mut self, cycles: f32) -> f32 {
        let params = self.params;
        let angle =
            params.turn_factor * PI * (cycles + self.frequency_offset) * self.wave_index as f32
                / params.width as f32
                + self.phase;
        self.phase += params.phase_step;
        let sine = angle.sin();
        let point = params.max_amplitude - self.amplitude * sine;
        if self.previous_sine < 0.0 && sine > 0.0 {
            let roll = self.noise.next_unit() * params.roll_range;
            if roll < params.reset_below {
                self.amplitude = (self.noise.next_unit() * params.amplitude_span
                    + params.amplitude_floor)
                    .trunc();
            }
        }
        self.previous_sine = sine;
        self.wave_index += 1;
        point
    }
}

#[cfg(test)]
pub(super) fn test_params() -> WaveParams {
    WaveParams {
        width: 24,
        max_amplitude: 50.0,
        quiet_amplitude: 3.0,
        initial_amplitude: 2.0,
        tick_seconds: 0.05,
        samples_per_tick: 4,
        phase_step: 0.1,
        turn_factor: 2.0,
        roll_range: 100.0,
        reset_below: 60.0,
        amplitude_span: 30.0,
        amplitude_floor: 10.0,
        start_phase_turns: 2.0,
        frequency_span: 2.0,
        frequency_base: 1.0,
        line_width: 1.5,
        line_shade: Shade::WHITE,
        line_alpha: 1.0,
        lerp: [4.0, 8.0, 50.0, 90.0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave(seed: u64) -> Waveform {
        Waveform::new(test_params(), seed)
    }

    #[test]
    fn the_frequency_maps_onto_whole_cycles_and_clamps() {
        let params = test_params();
        assert!((params.cycles_for_frequency(50.0) - 4.0).abs() < 1e-6);
        assert!((params.cycles_for_frequency(90.0) - 8.0).abs() < 1e-6);
        assert!((params.cycles_for_frequency(70.0) - 6.0).abs() < 1e-6);
        assert!((params.cycles_for_frequency(80.0) - 7.0).abs() < 1e-6);
        assert!((params.cycles_for_frequency(1.0) - 4.0).abs() < 1e-6);
        assert!((params.cycles_for_frequency(999.0) - 8.0).abs() < 1e-6);
    }

    #[test]
    fn a_fractional_cycle_count_is_truncated() {
        assert!((test_params().cycles_for_frequency(75.0) - 6.0).abs() < 1e-6);
    }

    #[test]
    fn a_new_wave_starts_at_its_initial_amplitude() {
        let params = test_params();
        let mut wave = wave(5);
        wave.init(4.0);
        let first = wave.points().next().unwrap();
        assert!((params.max_amplitude - first).abs() <= params.initial_amplitude + 1e-4);
    }

    #[test]
    fn the_phase_advances_once_per_sample_not_per_tick() {
        let params = test_params();
        let mut wave = wave(1);
        let start = wave.phase;
        wave.init(4.0);
        let expected = params.width as f32 * params.phase_step;
        assert!((wave.phase - start - expected).abs() < 1e-3);
    }

    #[test]
    fn the_stream_is_continuous_across_ticks() {
        let params = test_params();
        let mut continuous = wave(3);
        continuous.init(7.0);
        let before: Vec<f32> = continuous.points().collect();
        continuous.advance_to(params.tick_seconds, 7.0);
        let after: Vec<f32> = continuous.points().collect();
        let kept = params.width - params.samples_per_tick;
        assert_eq!(&after[..kept], &before[params.samples_per_tick..]);
    }

    #[test]
    fn a_new_station_queues_behind_the_trace_being_shown() {
        let mut wave = wave(3);
        wave.init(7.0);
        let shown: Vec<f32> = wave.points().collect();
        wave.init(9.0);
        assert_eq!(shown, wave.points().collect::<Vec<_>>());
    }

    #[test]
    fn amplitudes_are_whole_numbers_inside_the_authored_span() {
        let params = test_params();
        let mut wave = wave(9);
        wave.init(6.0);
        for step in 1..200 {
            wave.advance_to(f64::from(step) * params.tick_seconds, 6.0);
        }
        let highest = params.amplitude_floor + params.amplitude_span;
        assert_eq!(wave.amplitude.fract(), 0.0);
        assert!(wave.amplitude >= params.initial_amplitude.min(params.amplitude_floor));
        assert!(wave.amplitude < highest);
        let reach = params.max_amplitude + highest;
        assert!(
            wave.points()
                .all(|y| (params.max_amplitude - highest..=reach).contains(&y))
        );
    }

    #[test]
    fn the_same_seed_gives_the_same_trace_and_other_seeds_differ() {
        let run = |seed| {
            let mut wave = wave(seed);
            wave.init(8.0);
            for step in 1..40 {
                wave.advance_to(f64::from(step) * 0.07, 8.0);
            }
            wave.points().collect::<Vec<_>>()
        };
        assert_eq!(run(42), run(42));
        assert_ne!(run(42), run(43));
    }

    #[test]
    fn a_long_gap_runs_a_bounded_number_of_ticks() {
        let mut wave = wave(3);
        wave.init(7.0);
        wave.advance_to(10_000.0, 7.0);
        assert_eq!(wave.points().len(), test_params().width);
    }

    #[test]
    fn time_within_one_tick_or_backwards_changes_nothing() {
        let params = test_params();
        let mut wave = wave(3);
        wave.init(7.0);
        wave.advance_to(params.tick_seconds * 3.0, 7.0);
        let before: Vec<f32> = wave.points().collect();
        wave.advance_to(params.tick_seconds * 3.5, 7.0);
        wave.advance_to(params.tick_seconds, 7.0);
        assert_eq!(before, wave.points().collect::<Vec<_>>());
    }

    #[test]
    fn each_clip_carries_its_own_frequency_offset_inside_the_authored_span() {
        let params = test_params();
        for seed in 0..20 {
            let offset = wave(seed).frequency_offset;
            let low = params.frequency_base - params.frequency_span;
            assert!((low..=params.frequency_base).contains(&offset), "{offset}");
        }
    }
}
