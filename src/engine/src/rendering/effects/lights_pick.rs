//! Picks the placed lights that light an effect draw.

use glam::Vec3;

use super::constants::{EffectLight, LIGHT_COUNT};
use crate::rendering::placed_light::{GpuPlacedLight, SKIPPED_FLAGS};

/// A placed light and the factor its animation currently scales its colour by (1.0 when the
/// light is not animated).
#[derive(Clone, Copy)]
pub struct PickableLight {
    pub light: GpuPlacedLight,
    pub dimmer: f32,
}

impl PickableLight {
    pub fn steady(light: GpuPlacedLight) -> Self {
        Self { light, dimmer: 1.0 }
    }

    fn effect_light(&self) -> EffectLight {
        let mut light = EffectLight::from_placed(&self.light);
        light.color *= self.dimmer;
        light
    }
}

/// Rec. 709 luma weights: how strongly a colour lights a texture on screen.
const LUMA: Vec3 = Vec3::new(0.2126, 0.7152, 0.0722);

/// The lights that light one draw: at most [`LIGHT_COUNT`], strongest first.
#[derive(Debug, Clone, Copy)]
pub struct PickedLights {
    lights: [EffectLight; LIGHT_COUNT],
    len: usize,
}

impl PickedLights {
    pub const NONE: Self = Self {
        lights: [EffectLight {
            position: Vec3::ZERO,
            radius: 0.0,
            color: Vec3::ZERO,
            spot: None,
        }; LIGHT_COUNT],
        len: 0,
    };

    pub fn as_slice(&self) -> &[EffectLight] {
        &self.lights[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, index: usize) -> Option<&EffectLight> {
        self.as_slice().get(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &EffectLight> {
        self.as_slice().iter()
    }
}

/// Picks the lights that light a point the most. Owns the scratch buffer so a frame's picks
/// allocate nothing after the first.
#[derive(Default)]
pub struct LightPicker {
    ranked: Vec<(f32, usize)>,
}

impl LightPicker {
    /// The (at most [`LIGHT_COUNT`]) lights that light `centre` the most, strongest first: the
    /// luma of the light colour times its attenuation at `centre`. Lights the cluster cull
    /// skips ([`SKIPPED_FLAGS`]), that do not reach `centre` or that darken (negative colour)
    /// are not picked.
    pub fn pick(&mut self, candidates: &[PickableLight], centre: Vec3) -> PickedLights {
        self.ranked.clear();
        for (index, candidate) in candidates.iter().enumerate() {
            let placed = &candidate.light;
            if placed.flags & SKIPPED_FLAGS != 0 || placed.radius <= 0.0 {
                continue;
            }
            let offset = Vec3::from(placed.position) - centre;
            if offset.length_squared() > placed.radius * placed.radius {
                continue;
            }
            let light = candidate.effect_light();
            let strength = light.color.dot(LUMA) * light.attenuation_at(centre);
            if strength > 0.0 {
                self.ranked.push((strength, index));
            }
        }
        let by_strength = |a: &(f32, usize), b: &(f32, usize)| b.0.total_cmp(&a.0);
        if self.ranked.len() > LIGHT_COUNT {
            self.ranked
                .select_nth_unstable_by(LIGHT_COUNT - 1, by_strength);
            self.ranked.truncate(LIGHT_COUNT);
        }
        self.ranked.sort_by(by_strength);
        let mut picked = PickedLights::NONE;
        for &(_, index) in &self.ranked {
            picked.lights[picked.len] = candidates[index].effect_light();
            picked.len += 1;
        }
        picked
    }
}

/// One-shot form of [`LightPicker::pick`].
pub fn pick_lights(candidates: &[PickableLight], centre: Vec3) -> PickedLights {
    LightPicker::default().pick(candidates, centre)
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals copied, not computed"
)]
mod tests {
    use bytemuck::Zeroable;

    use super::*;
    use crate::rendering::placed_light::{LIGHT_FLAG_AMBIENT_ONLY, LIGHT_FLAG_OFF_BY_DEFAULT};

    fn light(position: [f32; 3], radius: f32, colour: [f32; 3]) -> PickableLight {
        PickableLight::steady(GpuPlacedLight {
            position,
            radius,
            color: colour,
            ..GpuPlacedLight::zeroed()
        })
    }

    #[test]
    fn the_strongest_four_are_picked_strongest_first() {
        let lights: Vec<_> = (1..=6)
            .map(|k| light([k as f32, 0.0, 0.0], 100.0, [1.0; 3]))
            .collect();

        let picked = pick_lights(&lights, Vec3::ZERO);

        let xs: Vec<f32> = picked.iter().map(|l| l.position.x).collect();
        assert_eq!(xs, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn picking_many_lights_keeps_exactly_the_strongest_four() {
        let lights: Vec<_> = (1..=40)
            .map(|k| light([(k * 7 % 41) as f32, 0.0, 0.0], 100.0, [1.0; 3]))
            .collect();
        let mut picker = LightPicker::default();

        let picked = picker.pick(&lights, Vec3::ZERO);
        let again = picker.pick(&lights, Vec3::ZERO);

        let xs: Vec<f32> = picked.iter().map(|l| l.position.x).collect();
        assert_eq!(xs, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(
            again.as_slice(),
            picked.as_slice(),
            "reusing the scratch is stable"
        );
    }

    #[test]
    fn lights_the_cluster_cull_skips_are_not_picked() {
        for flags in [LIGHT_FLAG_OFF_BY_DEFAULT, LIGHT_FLAG_AMBIENT_ONLY] {
            let mut skipped = light([1.0, 0.0, 0.0], 50.0, [1.0; 3]);
            skipped.light.flags = flags;
            let kept = light([20.0, 0.0, 0.0], 50.0, [0.5; 3]);

            let picked = pick_lights(&[skipped, kept], Vec3::ZERO);

            assert_eq!(picked.len(), 1, "flags {flags:#x}");
            assert_eq!(picked.get(0).unwrap().position.x, 20.0);
        }
    }

    #[test]
    fn a_dim_near_light_can_lose_to_a_bright_farther_one() {
        let lights = [
            light([1.0, 0.0, 0.0], 50.0, [0.05; 3]),
            light([10.0, 0.0, 0.0], 50.0, [5.0; 3]),
        ];

        let picked = pick_lights(&lights, Vec3::ZERO);

        assert_eq!(picked.get(0).unwrap().position.x, 10.0);
    }

    #[test]
    fn lights_out_of_range_empty_slots_and_negative_lights_are_not_picked() {
        let lights = [
            light([100.0, 0.0, 0.0], 5.0, [1.0; 3]),
            light([0.0; 3], 0.0, [1.0; 3]),
            light([1.0, 0.0, 0.0], 50.0, [-1.0; 3]),
            light([1.0, 0.0, 0.0], 50.0, [0.0; 3]),
        ];

        assert!(pick_lights(&lights, Vec3::ZERO).is_empty());
    }

    #[test]
    fn no_candidates_pick_nothing() {
        assert!(pick_lights(&[], Vec3::ZERO).is_empty());
    }

    #[test]
    fn the_dimmer_scales_the_picked_colour_and_the_ranking() {
        let mut dark = light([1.0, 0.0, 0.0], 50.0, [1.0; 3]);
        dark.dimmer = 0.0;
        let lit = light([20.0, 0.0, 0.0], 50.0, [0.5; 3]);

        let picked = pick_lights(&[dark, lit], Vec3::ZERO);

        assert_eq!(picked.len(), 1);
        assert_eq!(picked.get(0).unwrap().position.x, 20.0);

        let mut half = light([1.0, 0.0, 0.0], 50.0, [1.0; 3]);
        half.dimmer = 0.5;
        assert!((pick_lights(&[half], Vec3::ZERO).get(0).unwrap().color.x - 0.5).abs() < 1e-6);
    }
}
