//! What the streamer hands the fire effects: the lights of addon nodes, which light models can
//! animate a placed light, and the placed models that carry particle systems of their own.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Mat3, Mat4};

use super::{AttachmentKey, place_light};
use crate::assets::db::addon_nodes::AddonNodes;
use crate::assets::types::model_extras::ModelExtras;
use crate::rendering::placed_light::{GpuPlacedLight, LightBase};
use crate::rendering::shadows::lamps::source::ShadowLampSource;

/// Names a placed light for as long as its cell stays resident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LightKey {
    pub residency: u64,
    pub light_index: u32,
}

/// A light of a cell whose record names a light model (a candidate for model animation).
#[derive(Debug, Clone)]
pub(super) struct LightModelRef {
    pub light_index: u32,
    /// Model path relative to the asset root.
    pub model: Arc<str>,
    /// Engine space <- light model: the placement's rotation and scale.
    pub frame: Mat3,
}

/// A placed light with the model that may animate it.
#[derive(Debug, Clone, Copy)]
pub struct ModelLight<'a> {
    pub key: LightKey,
    /// Index in the GPU slab; `None` while the cell's lights have no slab range.
    pub slab_index: Option<u32>,
    /// The light as placed, before any animation.
    pub base: &'a GpuPlacedLight,
    /// Model path relative to the asset root.
    pub model: &'a str,
    /// Engine space <- light model: the placement's rotation and scale.
    pub frame: Mat3,
}

/// A placed model with particle systems of its own: where, in engine space, it stands.
#[derive(Debug, Clone, Copy)]
pub struct ParticleHost<'a> {
    pub key: AttachmentKey,
    /// Model path relative to the asset root.
    pub model_path: &'a Arc<str>,
    pub transform: Mat4,
}

pub(super) struct StoredParticleHost {
    pub model_path: Arc<str>,
    pub transform: Mat4,
}

/// One host per placement of a model whose file carries particle systems.
pub(super) fn particle_hosts_of(
    model_path: &Arc<str>,
    extras: &ModelExtras,
    placements: &[Mat4],
) -> Vec<StoredParticleHost> {
    if extras.particle_systems.is_empty() {
        return Vec::new();
    }
    placements
        .iter()
        .map(|placement| StoredParticleHost {
            model_path: Arc::clone(model_path),
            transform: *placement,
        })
        .collect()
}

/// The lookups addon lights are placed from.
pub(super) struct AddonLightSources<'a> {
    pub addon_nodes: &'a AddonNodes,
    pub light_lookup: &'a HashMap<String, LightBase>,
    pub light_models: &'a HashMap<String, Arc<str>>,
}

/// Where placed addon lights go.
pub(super) struct LightSink<'a> {
    pub lights: &'a mut Vec<GpuPlacedLight>,
    pub shadow_lamps: &'a mut Vec<ShadowLampSource>,
    pub light_models: &'a mut Vec<LightModelRef>,
}

/// Places the `LNAM` light of every addon node of `extras` for each placement. Returns how many
/// lights were placed.
pub(super) fn place_addon_lights(
    sources: &AddonLightSources,
    extras: &ModelExtras,
    placements: &[Mat4],
    sink: &mut LightSink,
) -> u32 {
    let mut placed = 0;
    for placement in placements {
        for node in &extras.addon_nodes {
            let Some(row) = sources.addon_nodes.get(node.value.index) else {
                continue;
            };
            let Some(light_key) = &row.light else {
                continue;
            };
            let Some(base) = sources.light_lookup.get(light_key) else {
                crate::utils::warn_once(&format!("addon_light_unusable:{light_key}"), || {
                    format!(
                        "addon node {} names light {light_key}, which is not a usable light record; no light is placed",
                        row.form_key
                    )
                });
                continue;
            };
            let attachment = *placement * node.transform;
            let phase_key = format!("{}:{}", row.form_key, node.node_name);
            let index = sink.lights.len() as u32;
            if place_light(
                base.place_attached(&attachment, &phase_key),
                light_key,
                sink.lights,
                sink.shadow_lamps,
            ) {
                placed += 1;
                if let Some(model) = sources.light_models.get(light_key) {
                    sink.light_models.push(LightModelRef {
                        light_index: index,
                        model: Arc::clone(model),
                        frame: Mat3::from_mat4(attachment),
                    });
                }
            }
        }
    }
    placed
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals or exact compositions of them"
)]
mod tests {
    use glam::Vec3;
    use serde_json::json;

    use super::*;
    use crate::assets::db::lights::Light;
    use crate::assets::types::model_extras::{AddonNodeExtras, NodeExtras};

    fn light_row(model_path: Option<&str>) -> Light {
        Light {
            form_key: Some("0A:fire.esm".to_owned()),
            editor_id: None,
            model_path: model_path.map(str::to_owned),
            radius: Some(100.0),
            color: Some(0x00FF_8000),
            flags: Some(0),
            fade_value: Some(1.0),
            falloff_exponent: None,
            is_marker: false,
            constant: None,
            scalar: None,
            exponent: None,
            fov: None,
            near_clip: None,
            flicker_period: None,
            flicker_intensity_amplitude: None,
            flicker_movement_amplitude: None,
            gobo: None,
            lens: None,
            god_rays: None,
            god_rays_near_clip: None,
        }
    }

    fn addons(light: Option<&str>) -> AddonNodes {
        AddonNodes::from_rows(vec![crate::assets::db::addon_nodes::AddonNode {
            form_key: "0B:fire.esm".to_owned(),
            model_path: None,
            node_index: 5,
            light: light.map(str::to_owned),
            cap: 0,
            flags: 0,
        }])
    }

    fn extras_with_addon(index: i32) -> ModelExtras {
        ModelExtras {
            addon_nodes: vec![NodeExtras {
                node_name: "ValueNode".to_owned(),
                transform: Mat4::from_translation(Vec3::new(0.0, 0.0, 4.0)),
                value: AddonNodeExtras {
                    v: 1,
                    index,
                    flags: 0,
                },
            }],
            ..ModelExtras::default()
        }
    }

    struct Placed {
        lights: Vec<GpuPlacedLight>,
        shadow_lamps: Vec<ShadowLampSource>,
        light_models: Vec<LightModelRef>,
        count: u32,
    }

    fn place(addon_nodes: &AddonNodes, index: i32, placements: &[Mat4]) -> Placed {
        let rows = [light_row(Some(r"Lights\FireFlickerLight01.nif"))];
        let light_lookup = HashMap::from([(
            "0A:fire.esm".to_owned(),
            LightBase::from_row(&rows[0]).unwrap(),
        )]);
        let light_models = HashMap::from([(
            "0A:fire.esm".to_owned(),
            Arc::<str>::from(r"meshes\fire.glb"),
        )]);
        let mut placed = Placed {
            lights: Vec::new(),
            shadow_lamps: Vec::new(),
            light_models: Vec::new(),
            count: 0,
        };
        placed.count = place_addon_lights(
            &AddonLightSources {
                addon_nodes,
                light_lookup: &light_lookup,
                light_models: &light_models,
            },
            &extras_with_addon(index),
            placements,
            &mut LightSink {
                lights: &mut placed.lights,
                shadow_lamps: &mut placed.shadow_lamps,
                light_models: &mut placed.light_models,
            },
        );
        placed
    }

    #[test]
    fn the_light_of_an_addon_node_is_placed_at_the_composed_transform() {
        let placement = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0));

        let placed = place(&addons(Some("0A:fire.esm")), 5, &[placement]);

        assert_eq!(placed.count, 1);
        assert_eq!(placed.lights[0].position, [10.0, 0.0, 4.0]);
        assert_eq!(
            placed.lights[0].radius, 10.0,
            "100 Bethesda units in engine units"
        );
        assert_eq!(placed.light_models[0].light_index, 0);
        assert_eq!(&*placed.light_models[0].model, r"meshes\fire.glb");
    }

    #[test]
    fn each_placement_gets_its_own_light() {
        let placements = [
            Mat4::from_translation(Vec3::X),
            Mat4::from_translation(Vec3::Y),
        ];

        let placed = place(&addons(Some("0A:fire.esm")), 5, &placements);

        assert_eq!(placed.count, 2);
        assert_eq!(placed.lights[1].position, [0.0, 1.0, 4.0]);
        assert_eq!(placed.light_models[1].light_index, 1);
    }

    #[test]
    fn unresolved_indices_records_without_a_light_and_unusable_lights_place_nothing() {
        let placement = [Mat4::IDENTITY];
        assert_eq!(place(&addons(Some("0A:fire.esm")), 99, &placement).count, 0);
        assert_eq!(place(&addons(None), 5, &placement).count, 0);
        assert_eq!(place(&addons(Some("0C:gone.esm")), 5, &placement).count, 0);
    }

    #[test]
    fn a_model_with_particle_systems_hosts_one_per_placement() {
        let mut extras = ModelExtras::default();
        let path: Arc<str> = Arc::from(r"meshes\barrel.glb");
        let placements = [Mat4::IDENTITY, Mat4::from_translation(Vec3::Z)];
        assert!(particle_hosts_of(&path, &extras, &placements).is_empty());

        extras.particle_systems.push(NodeExtras {
            node_name: "Flames".to_owned(),
            transform: Mat4::IDENTITY,
            value: serde_json::from_value(particle_system_json()).unwrap(),
        });
        let hosts = particle_hosts_of(&path, &extras, &placements);

        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[1].transform, placements[1]);
        assert_eq!(&*hosts[0].model_path, r"meshes\barrel.glb");
    }

    fn particle_system_json() -> serde_json::Value {
        json!({
            "v": 1, "world_space": true, "max_particles": 1, "material": 0,
            "fade": { "near_begin": 0, "near_end": 0, "far_begin": 0, "far_end": 0 },
            "data": { "aspect_ratio": 1.0, "aspect_flags": 0, "speed_to_aspect": [1, 0, 1],
                      "subtexture_rects": [] },
            "emitters": [], "modifiers": [],
            "update": { "frequency": 1.0, "phase": 0.0 }
        })
    }
}
