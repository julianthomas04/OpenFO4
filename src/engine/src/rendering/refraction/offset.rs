//! The distortion math of the refraction stage. `shaders/refraction.wgsl` evaluates the same
//! formulas per pixel; the GPU test in `stage` holds the two in agreement. Every tuning value
//! is a named constant here and reaches the shader through the frame uniform.

use glam::Vec2;

use crate::anim::scalar_track::{TrackTarget, TrackValue};
use crate::assets::types::effect_material::RefractionMaterial;
use crate::utils::warn_once;

/// View depth (eu) at which a material's `strength` is the offset in screen UV.
pub const REFERENCE_DEPTH_EU: f32 = 20.0;
/// Upper bound of the inverse-depth gain: haze right in front of the camera does not explode.
pub const MAX_DEPTH_GAIN: f32 = 2.0;
/// View depths below this are treated as this when computing the gain.
pub const MIN_VIEW_DEPTH_EU: f32 = 0.5;
/// Largest distortion offset in screen UV (fraction of the viewport).
pub const MAX_OFFSET_UV: f32 = 0.08;
/// Distance (eu) behind the card over which the distortion fades in from nothing, so geometry
/// that touches or pierces the card is not smeared along the intersection.
pub const SOFT_FADE_EU: f32 = 5.0;
/// A displaced sample this much (eu) or more in front of the card is rejected.
pub const REJECT_BIAS_EU: f32 = 0.5;
/// Offset (screen UV) at which the distorted colour fully replaces the pixel; below it the
/// pixel fades towards what is already drawn, so the card edge and untouched surfaces
/// (water, translucent buckets drawn earlier) keep their look.
pub const REPLACE_OFFSET_UV: f32 = 0.002;
/// Layer B samples its normal map at this multiple of layer A's UV scale.
pub const LAYER_B_SCALE_RATIO: f32 = 1.7;
/// Layer B scrolls at this multiple of layer A's offset (negative: against layer A).
pub const LAYER_B_SCROLL_RATIO: f32 = -0.6;
/// Depth reported for a sky pixel (depth 0 in reverse-Z): effectively infinitely far.
pub const FAR_VIEW_DEPTH_EU: f32 = 1.0e30;

/// `uv * scale + offset`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvTransform {
    pub offset: Vec2,
    pub scale: Vec2,
}

/// The transforms of the two normal-map layers at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerTransforms {
    pub a: UvTransform,
    pub b: UvTransform,
}

/// A material's animated values at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimatedHaze {
    pub uv: UvTransform,
    pub strength: f32,
}

/// The material's UV offset, scale and strength with its controller tracks applied at
/// `elapsed` seconds (a `refraction_strength` track replaces the base strength). When several
/// tracks drive one target, the first in the list wins. A track that drives nothing a
/// refraction card has (colour, alpha, light targets) is skipped with a warning.
pub fn animated_haze(material: &RefractionMaterial, elapsed: f32) -> AnimatedHaze {
    let mut uv = UvTransform {
        offset: Vec2::from(material.uv_offset),
        scale: Vec2::from(material.uv_scale),
    };
    let mut strength = material.strength;
    for track in material.controllers.iter().rev() {
        let TrackValue::Scalar(value) = track.evaluate(elapsed) else {
            warn_unsupported(track.target);
            continue;
        };
        match track.target {
            TrackTarget::UOffset => uv.offset.x = value,
            TrackTarget::VOffset => uv.offset.y = value,
            TrackTarget::UScale => uv.scale.x = value,
            TrackTarget::VScale => uv.scale.y = value,
            TrackTarget::RefractionStrength => strength = value,
            other => warn_unsupported(other),
        }
    }
    AnimatedHaze { uv, strength }
}

fn warn_unsupported(target: TrackTarget) {
    warn_once(&format!("refraction-track:{}", target.name()), || {
        format!(
            "refraction material: controller target {} has no effect on heat haze; ignored",
            target.name()
        )
    });
}

/// Layer A follows the animated transform; layer B is finer and scrolls against it, so the
/// two normal maps never line up and the distortion does not read as a sliding picture.
pub fn layer_transforms(uv: UvTransform) -> LayerTransforms {
    LayerTransforms {
        a: uv,
        b: UvTransform {
            offset: uv.offset * LAYER_B_SCROLL_RATIO,
            scale: uv.scale * LAYER_B_SCALE_RATIO,
        },
    }
}

/// Inverse-depth gain: distant haze distorts less, bounded above.
pub fn depth_gain(card_view_depth: f32) -> f32 {
    (REFERENCE_DEPTH_EU / card_view_depth.max(MIN_VIEW_DEPTH_EU)).min(MAX_DEPTH_GAIN)
}

/// Fade factor `0..=1` from how far the scene behind the card is: 0 where the scene is at or
/// in front of the card, 1 from [`SOFT_FADE_EU`] behind it.
pub fn soft_factor(scene_view_depth: f32, card_view_depth: f32) -> f32 {
    ((scene_view_depth - card_view_depth) / SOFT_FADE_EU).clamp(0.0, 1.0)
}

/// Whether a scene sample lies in front of the card and must not be pulled into it.
pub fn is_in_front(sample_view_depth: f32, card_view_depth: f32) -> bool {
    sample_view_depth < card_view_depth - REJECT_BIAS_EU
}

/// Tangent-space xy of a normal-map texel (`rg` in `0..=1`).
pub fn unpack_normal_xy(rg: Vec2) -> Vec2 {
    rg * 2.0 - Vec2::ONE
}

/// Everything the offset of one pixel depends on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OffsetInputs {
    pub normal_a: Vec2,
    pub normal_b: Vec2,
    pub strength: f32,
    pub vertex_alpha: f32,
    /// `abs(dot(surface normal, direction to the camera))`.
    pub facing: f32,
    pub card_view_depth: f32,
    pub scene_view_depth: f32,
}

/// The screen-UV offset of the sample: the mean of the two layers' tangent-space xy, scaled
/// by strength, vertex alpha, view-angle facing, depth gain and soft fade, limited to
/// [`MAX_OFFSET_UV`]. Tangent +y moves the sample up the screen (screen v grows downwards).
pub fn screen_offset(inputs: &OffsetInputs) -> Vec2 {
    let combined = (inputs.normal_a + inputs.normal_b) * 0.5;
    let magnitude = inputs.strength
        * inputs.vertex_alpha
        * inputs.facing
        * depth_gain(inputs.card_view_depth)
        * soft_factor(inputs.scene_view_depth, inputs.card_view_depth);
    let offset = Vec2::new(combined.x, -combined.y) * magnitude;
    offset.clamp_length_max(MAX_OFFSET_UV)
}

/// The weight of the distorted colour over what is already in the target.
pub fn blend_weight(offset: Vec2) -> f32 {
    (offset.length() / REPLACE_OFFSET_UV).clamp(0.0, 1.0)
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals or exact products of them"
)]
mod tests {
    use serde_json::json;

    use super::*;

    fn inputs() -> OffsetInputs {
        OffsetInputs {
            normal_a: Vec2::new(1.0, 0.0),
            normal_b: Vec2::new(1.0, 0.0),
            strength: 0.01,
            vertex_alpha: 1.0,
            facing: 1.0,
            card_view_depth: REFERENCE_DEPTH_EU,
            scene_view_depth: FAR_VIEW_DEPTH_EU,
        }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn strength_is_the_offset_at_the_reference_depth() {
        let offset = screen_offset(&inputs());
        assert!(near(offset.x, 0.01) && near(offset.y, 0.0), "{offset}");
    }

    #[test]
    fn the_offset_scales_linearly_with_strength_alpha_and_facing() {
        let base = screen_offset(&inputs()).x;
        for scaled in [
            OffsetInputs {
                strength: 0.02,
                ..inputs()
            },
            OffsetInputs {
                vertex_alpha: 2.0,
                ..inputs()
            },
            OffsetInputs {
                facing: 2.0,
                ..inputs()
            },
        ] {
            assert!(near(screen_offset(&scaled).x, 2.0 * base));
        }
        for zeroed in [
            OffsetInputs {
                facing: 0.0,
                ..inputs()
            },
            OffsetInputs {
                vertex_alpha: 0.0,
                ..inputs()
            },
        ] {
            assert_eq!(screen_offset(&zeroed), Vec2::ZERO);
        }
    }

    #[test]
    fn distant_haze_distorts_less_and_near_haze_is_capped() {
        let at = |depth| {
            screen_offset(&OffsetInputs {
                card_view_depth: depth,
                ..inputs()
            })
            .x
        };
        assert!(near(at(2.0 * REFERENCE_DEPTH_EU), 0.005));
        assert!(near(at(0.01), 0.01 * MAX_DEPTH_GAIN));
        assert!(near(at(MIN_VIEW_DEPTH_EU), at(MIN_VIEW_DEPTH_EU * 0.1)));
    }

    #[test]
    fn the_two_layers_are_averaged_and_tangent_up_is_screen_up() {
        let offset = screen_offset(&OffsetInputs {
            normal_a: Vec2::new(1.0, 1.0),
            normal_b: Vec2::new(-1.0, 1.0),
            ..inputs()
        });
        assert!(near(offset.x, 0.0));
        assert!(offset.y < 0.0, "screen v decreases upwards: {offset}");
    }

    #[test]
    fn the_offset_length_is_limited() {
        let offset = screen_offset(&OffsetInputs {
            strength: 10.0,
            ..inputs()
        });
        assert!(near(offset.length(), MAX_OFFSET_UV));
    }

    #[test]
    fn geometry_close_behind_the_card_fades_the_distortion_out() {
        let at = |behind| {
            screen_offset(&OffsetInputs {
                scene_view_depth: REFERENCE_DEPTH_EU + behind,
                ..inputs()
            })
            .x
        };
        assert_eq!(at(0.0), 0.0);
        assert_eq!(at(-3.0), 0.0, "scene in front of the card");
        assert!(near(at(SOFT_FADE_EU / 2.0), 0.005));
        assert!(near(at(SOFT_FADE_EU), 0.01));
    }

    #[test]
    fn samples_in_front_of_the_card_are_rejected_beyond_the_bias_only() {
        let card = 10.0;
        assert!(is_in_front(card - REJECT_BIAS_EU - 0.01, card));
        assert!(!is_in_front(card - REJECT_BIAS_EU, card));
        assert!(!is_in_front(card + 1.0, card));
    }

    #[test]
    fn the_blend_weight_ramps_to_full_replacement() {
        assert_eq!(blend_weight(Vec2::ZERO), 0.0);
        assert!(near(
            blend_weight(Vec2::new(0.0, REPLACE_OFFSET_UV / 2.0)),
            0.5
        ));
        assert_eq!(blend_weight(Vec2::splat(1.0)), 1.0);
    }

    #[test]
    fn layer_b_is_finer_and_scrolls_against_layer_a() {
        let uv = UvTransform {
            offset: Vec2::new(0.5, -0.25),
            scale: Vec2::new(2.0, 3.0),
        };
        let layers = layer_transforms(uv);
        assert_eq!(layers.a, uv);
        assert_eq!(layers.b.scale, uv.scale * LAYER_B_SCALE_RATIO);
        assert!(layers.b.offset.x < 0.0 && layers.b.offset.y > 0.0);
    }

    #[allow(
        clippy::needless_pass_by_value,
        reason = "callers pass json! literals, which are temporaries"
    )]
    fn material(controllers: serde_json::Value) -> RefractionMaterial {
        serde_json::from_value(json!({
            "v": 1, "strength": 0.04, "normal_a": "a.dds", "normal_b": "b.dds",
            "uv_offset": [0.25, 0.5], "uv_scale": [1.0, 2.0],
            "vertex_colors": true, "double_sided": false, "controllers": controllers
        }))
        .unwrap()
    }

    fn scroll_track(target: &str, from: f32, to: f32) -> serde_json::Value {
        json!({
            "target": target, "cycle": "loop", "start": 0.0, "stop": 2.0,
            "frequency": 1.0, "phase": 0.0, "times": [0.0, 2.0], "values": [from, to]
        })
    }

    #[test]
    fn without_tracks_the_materials_own_transform_applies() {
        let uv = animated_haze(&material(json!([])), 7.0).uv;
        assert_eq!(uv.offset, Vec2::new(0.25, 0.5));
        assert_eq!(uv.scale, Vec2::new(1.0, 2.0));
    }

    #[test]
    fn offset_tracks_scroll_the_uv_over_time_and_loop() {
        let material = material(json!([
            scroll_track("u_offset", 0.0, 1.0),
            scroll_track("v_scale", 1.0, 3.0),
        ]));
        let at = |seconds| animated_haze(&material, seconds).uv;
        assert!(near(at(0.0).offset.x, 0.0));
        assert!(near(at(1.0).offset.x, 0.5));
        assert!(near(at(0.5).offset.x, at(2.5).offset.x), "a loop repeats");
        assert_eq!(
            at(1.0).offset.y,
            0.5,
            "an undriven component keeps the material value"
        );
        assert!(near(at(1.0).scale.y, 2.0));
        assert_eq!(at(1.0).scale.x, 1.0);
    }

    #[test]
    fn the_first_track_of_a_target_wins() {
        let material = material(json!([
            scroll_track("u_offset", 0.1, 0.1),
            scroll_track("u_offset", 0.9, 0.9),
        ]));
        assert!(near(animated_haze(&material, 0.0).uv.offset.x, 0.1));
    }

    #[test]
    fn a_track_a_card_cannot_use_leaves_the_transform_unchanged() {
        let material = material(json!([scroll_track("alpha", 0.0, 1.0)]));
        let uv = animated_haze(&material, 1.0).uv;
        assert_eq!(uv.offset, Vec2::new(0.25, 0.5));
    }

    #[test]
    fn a_strength_track_replaces_the_base_strength() {
        let animated = material(json!([scroll_track("refraction_strength", 0.0, 0.2)]));
        assert!(near(animated_haze(&animated, 0.0).strength, 0.0));
        assert!(near(animated_haze(&animated, 1.0).strength, 0.1));
        assert!(near(
            animated_haze(&material(json!([])), 1.0).strength,
            0.04
        ));
    }
}
