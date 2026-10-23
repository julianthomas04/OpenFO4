//! Pure per-particle evaluation of the colour, scale and flipbook modifiers
//! as a function of life fraction.

use crate::assets::types::particle_system::ModifierKind;

use super::constants::SUBTEXTURE_FRAMES_PER_COUNT_SECOND;

/// Fraction of the way `value` is from `from` to `to`, clamped to `0..=1`;
/// a degenerate range is a step at `to`.
pub fn ramp(value: f32, from: f32, to: f32) -> f32 {
    if to <= from {
        return f32::from(value >= to);
    }
    ((value - from) / (to - from)).clamp(0.0, 1.0)
}

pub fn mix(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    std::array::from_fn(|i| (b[i] - a[i]).mul_add(t, a[i]))
}

/// Colour at life fraction `life` for a [`ModifierKind::SimpleColor`]; `None`
/// when `kind` is another modifier.
pub fn simple_colour(kind: &ModifierKind, life: f32) -> Option<[f32; 4]> {
    let ModifierKind::SimpleColor {
        fade_in_percent,
        fade_out_percent,
        color1_end_percent,
        color2_start_percent,
        color2_end_percent,
        color3_start_percent,
        colors,
    } = kind
    else {
        return None;
    };
    let mut colour = if life <= *color1_end_percent {
        colors[0]
    } else if life < *color2_start_percent {
        mix(
            colors[0],
            colors[1],
            ramp(life, *color1_end_percent, *color2_start_percent),
        )
    } else if life <= *color2_end_percent {
        colors[1]
    } else if life < *color3_start_percent {
        mix(
            colors[1],
            colors[2],
            ramp(life, *color2_end_percent, *color3_start_percent),
        )
    } else {
        colors[2]
    };
    if *fade_in_percent > 0.0 && life < *fade_in_percent {
        colour[3] *= life / fade_in_percent;
    }
    if *fade_out_percent < 1.0 && life > *fade_out_percent {
        colour[3] *= (1.0 - life) / (1.0 - fade_out_percent);
    }
    colour[3] = colour[3].clamp(0.0, 1.0);
    Some(colour)
}

/// Size factor at life fraction `life`: `scales` spread evenly over the life.
pub fn scale_at(scales: &[f32], life: f32) -> f32 {
    match scales {
        [] => 1.0,
        [only] => *only,
        _ => {
            let position = life.clamp(0.0, 1.0) * (scales.len() - 1) as f32;
            let lower = (position as usize).min(scales.len() - 2);
            let t = position - lower as f32;
            (scales[lower + 1] - scales[lower]).mul_add(t, scales[lower])
        }
    }
}

/// Per-particle random flipbook parameters, fixed at birth.
#[derive(Debug, Clone, Copy, Default)]
pub struct Flipbook {
    pub start: f32,
    pub frames_per_second: f32,
    pub loop_start: f32,
}

impl Flipbook {
    pub fn from_kind(kind: &ModifierKind, mut unit: impl FnMut() -> f32) -> Option<Self> {
        let ModifierKind::SubTex {
            start_frame,
            start_frame_fudge,
            loop_start_frame,
            loop_start_frame_fudge,
            frame_count,
            frame_count_fudge,
            ..
        } = kind
        else {
            return None;
        };
        Some(Self {
            start: unit().mul_add(*start_frame_fudge, *start_frame),
            frames_per_second: (unit().mul_add(2.0, -1.0))
                .mul_add(*frame_count_fudge, *frame_count)
                * SUBTEXTURE_FRAMES_PER_COUNT_SECOND,
            loop_start: unit().mul_add(*loop_start_frame_fudge, *loop_start_frame),
        })
    }

    /// Atlas cell after `age` seconds. Past `end_frame` the sequence jumps to
    /// the loop start and repeats; a loop start at the end holds the last frame.
    pub fn cell(&self, end_frame: f32, age: f32, cells: usize) -> usize {
        if cells == 0 {
            return 0;
        }
        let end = end_frame.max(0.0).floor() + 1.0;
        let loop_start = self.loop_start.clamp(0.0, end - 1.0);
        let mut frame = self.frames_per_second.mul_add(age, self.start);
        if frame >= end {
            frame = loop_start + (frame - end).rem_euclid(end - loop_start);
        }
        (frame.max(0.0) as usize).min(cells - 1)
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals or exactly representable sums"
)]
mod tests {
    use super::*;

    fn colour_kind(
        fade_in: f32,
        fade_out: f32,
        c1_end: f32,
        c2: (f32, f32),
        c3: f32,
    ) -> ModifierKind {
        ModifierKind::SimpleColor {
            fade_in_percent: fade_in,
            fade_out_percent: fade_out,
            color1_end_percent: c1_end,
            color2_start_percent: c2.0,
            color2_end_percent: c2.1,
            color3_start_percent: c3,
            colors: [
                [1.0, 0.0, 0.0, 1.0],
                [0.0, 1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0, 1.0],
            ],
        }
    }

    #[test]
    fn colour_holds_blends_and_holds_at_the_keys() {
        let kind = colour_kind(0.0, 1.0, 0.2, (0.4, 0.6), 0.8);
        let at = |life| simple_colour(&kind, life).unwrap();
        let close = |actual: [f32; 4], expected: [f32; 4]| {
            actual
                .iter()
                .zip(expected)
                .all(|(a, e)| (a - e).abs() < 1e-5)
        };
        assert!(close(at(0.1), [1.0, 0.0, 0.0, 1.0]));
        assert!(close(at(0.3), [0.5, 0.5, 0.0, 1.0]));
        assert!(close(at(0.5), [0.0, 1.0, 0.0, 1.0]));
        assert!(close(at(0.7), [0.0, 0.5, 0.5, 1.0]));
        assert!(close(at(0.9), [0.0, 0.0, 1.0, 1.0]));
    }

    #[test]
    fn alpha_fades_in_and_out() {
        let kind = colour_kind(0.2, 0.5, 0.0, (1.0, 1.0), 1.0);
        let alpha = |life| simple_colour(&kind, life).unwrap()[3];
        assert!((alpha(0.1) - 0.5).abs() < 1e-6);
        assert!((alpha(0.3) - 1.0).abs() < 1e-6);
        assert!((alpha(0.75) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn scale_interpolates_across_the_life() {
        let scales = [1.0, 3.0, 2.0];
        assert_eq!(scale_at(&scales, 0.0), 1.0);
        assert_eq!(scale_at(&scales, 0.25), 2.0);
        assert_eq!(scale_at(&scales, 1.0), 2.0);
        assert_eq!(scale_at(&[], 0.5), 1.0);
        assert_eq!(scale_at(&[4.0], 0.9), 4.0);
    }

    #[test]
    fn flipbook_advances_wraps_to_loop_start_and_holds() {
        let book = Flipbook {
            start: 0.0,
            frames_per_second: 4.0,
            loop_start: 0.0,
        };
        let cells: Vec<usize> = (0..10)
            .map(|i| book.cell(3.0, i as f32 * 0.25, 8))
            .collect();
        assert_eq!(cells, [0, 1, 2, 3, 0, 1, 2, 3, 0, 1]);

        let hold = Flipbook {
            loop_start: 3.0,
            ..book
        };
        assert_eq!(hold.cell(3.0, 5.0, 8), 3);

        let clamped = Flipbook {
            frames_per_second: 100.0,
            ..book
        };
        assert!(clamped.cell(63.0, 0.5, 4) < 4);
    }
}
