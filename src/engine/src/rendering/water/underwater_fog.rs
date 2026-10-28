//! The underwater fog of a WATR and the image space depth of field that comes with it
//! (see `docs/engine/rendering/surfaces-and-effects.md`): the fog is applied only by the fogged
//! depth-of-field pass, while the camera's near plane is fully under water; the
//! WATR's image space (`INAM`) then replaces the weather's depth of field.

use std::collections::HashMap;

use glam::{Vec3, Vec4};

use super::constants::gamma_rgb;
use crate::assets::db::{DbClient, waters::Water, waters::WaterData};
use crate::rendering::post::depth_of_field::{ImageSpaceDepthOfField, parse_image_space};
use crate::utils::warn_once;
use crate::water_world::Immersion;

/// The `w` of the fog colour register.
const TINT_W: f32 = 1.0;

/// What a WATR contributes to the fog: the underwater
/// colour, `pow(byte / 255, 2.2)`, and the three distances. The amount is
/// uploaded but not read by the fogged shader.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct UnderwaterFog {
    colour: Vec3,
    near: f32,
    far: f32,
    amount: f32,
}

impl UnderwaterFog {
    pub fn of_water(data: &WaterData) -> Self {
        Self {
            colour: gamma_rgb(data.fog_underwater_color),
            near: data.fog_underwater_near,
            far: data.fog_underwater_far,
            amount: data.fog_underwater_amount,
        }
    }

    /// `cb2[4]` = `(colour, 1)` and `cb2[5]` = `(far, near, amount, 0)` of the fogged pass.
    /// The weather's water multiplier is not applied: its reach under water is unmeasured.
    pub fn registers(&self) -> [Vec4; 2] {
        [
            self.colour.extend(TINT_W),
            Vec4::new(self.far, self.near, self.amount, 0.0),
        ]
    }
}

/// The image space the game gives a WATR without `INAM` (its depth of field strength is
/// 0: the effect then skips itself).
const DEFAULT_IMAGE_SPACE: &str = "000167:Fallout4.esm";

/// The depth of field that replaces the weather's: state 3 (the whole near plane
/// under water) selects the water's image space.
/// States 1 and 2 keep the weather's.
pub fn underwater_override_dof(
    immersion: Immersion,
    dof: Option<ImageSpaceDepthOfField>,
) -> Option<ImageSpaceDepthOfField> {
    (immersion == Immersion::Full).then_some(dof).flatten()
}

/// The depth of field of the image space (`INAM`) of each WATR by WATR form key, the
/// default image space for a WATR without one. A WATR without an entry has an image
/// space row without a depth of field block (a database that predates the export,
/// reported once) or one that cannot be read.
pub fn load_image_space_dofs(
    db: &DbClient,
    waters: &[Water],
) -> HashMap<String, ImageSpaceDepthOfField> {
    let mut by_image_space: HashMap<&str, Option<ImageSpaceDepthOfField>> = HashMap::new();
    let mut dofs = HashMap::new();
    for water in waters {
        let Some(data) = water.data.as_ref() else {
            continue;
        };
        let image_space = data.image_space.as_deref().unwrap_or(DEFAULT_IMAGE_SPACE);
        let dof = *by_image_space
            .entry(image_space)
            .or_insert_with(|| read_dof(db, image_space));
        if let Some(dof) = dof {
            dofs.insert(water.form_key.clone(), dof);
        }
    }
    dofs
}

fn read_dof(db: &DbClient, image_space: &str) -> Option<ImageSpaceDepthOfField> {
    let json = match db.get_image_space_data(image_space) {
        Ok(Some(json)) => json,
        Ok(None) => {
            warn_once(&format!("water-imgs-empty:{image_space}"), || {
                format!("image space {image_space} has no data; its water has no underwater fog")
            });
            return None;
        }
        Err(error) => {
            warn_once(&format!("water-imgs-missing:{image_space}"), || {
                format!(
                    "image space {image_space} of a WATR is unreadable ({error}); its water has no underwater fog"
                )
            });
            return None;
        }
    };
    match parse_image_space(&json) {
        Ok(Some(dof)) => Some(dof),
        Ok(None) => {
            warn_once("water-imgs-without-dof", || {
                "image_spaces rows carry no DepthOfField block (database predates the DoF export): there is no underwater fog until the ESM extraction is re-run".to_owned()
            });
            None
        }
        Err(error) => {
            tracing::error!("image space {image_space} has an unusable DoF block: {error}");
            None
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "registers are copies or exact products of fixture values"
)]
mod tests {
    use super::*;
    use crate::rendering::water::test_fixtures::synthetic_water;

    #[test]
    fn the_fog_comes_from_the_watr_underwater_fields() {
        // Synthetic water: colour (100, 110, 120), amount 0.5, near -5000, far 2000.
        let fog = UnderwaterFog::of_water(&synthetic_water());
        let channel = |byte: f32| (byte / 255.0).powf(2.2);
        assert!(fog.colour.abs_diff_eq(
            Vec3::new(channel(100.0), channel(110.0), channel(120.0)),
            1e-6
        ));
        assert_eq!((fog.near, fog.far, fog.amount), (-5000.0, 2000.0, 0.5));
    }

    const ACTIVE: ImageSpaceDepthOfField = ImageSpaceDepthOfField {
        strength: 1.0,
        distance: -2500.0,
        range: 3500.0,
        mode: 26,
        vignette_radius: 0.0,
        vignette_strength: 0.0,
    };

    #[test]
    fn only_a_fully_submerged_camera_replaces_the_weather_dof() {
        assert_eq!(
            underwater_override_dof(Immersion::Full, Some(ACTIVE)),
            Some(ACTIVE)
        );
        for partial in [Immersion::Above, Immersion::Shallow, Immersion::Deep] {
            assert_eq!(
                underwater_override_dof(partial, Some(ACTIVE)),
                None,
                "{partial:?}"
            );
        }
    }

    #[test]
    fn a_water_without_an_image_space_dof_replaces_nothing() {
        assert_eq!(underwater_override_dof(Immersion::Full, None), None);
    }

    #[test]
    fn the_registers_are_the_untinted_colour_with_w_one_then_far_near_amount() {
        let fog = UnderwaterFog::of_water(&synthetic_water());
        let [colour, distances] = fog.registers();
        assert_eq!(colour.truncate(), fog.colour);
        assert_eq!(colour.w, 1.0);
        assert_eq!(distances, Vec4::new(2000.0, -5000.0, 0.5, 0.0));
    }
}
