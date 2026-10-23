//! Which renderer draws a submesh: the object buckets, the effect stage or the refraction
//! stage; and the addon-node attachment points a placed model contributes.

use std::sync::Arc;

use glam::Mat4;

use crate::assets::types::effect_material::{EffectMaterial, RefractionMaterial};
use crate::assets::types::model_extras::{AddonNodeExtras, ModelExtras};
use crate::rendering::effects::geometry::EffectMeshData;

/// The renderer a submesh belongs to.
pub(super) enum ShapeRoute {
    /// The GPU-driven object buckets, whose hidden-effect rule decides what the FX toggle shows.
    Object,
    /// The effect stage: always drawn, never subject to the FX toggle.
    Effect {
        mesh: Arc<EffectMeshData>,
        material: Arc<EffectMaterial>,
    },
    /// The refraction stage.
    Refraction {
        mesh: Arc<EffectMeshData>,
        material: Arc<RefractionMaterial>,
    },
}

/// What the material name and the parsed extras say about a submesh.
pub(super) struct ShapeFacts {
    /// The `refr` token.
    pub is_refraction: bool,
    /// The `fx` token: an effect-shader shape.
    pub is_effect: bool,
    /// The `eff` token: the converter wrote effect extras for the shape.
    pub has_effect_key: bool,
    /// An untextured blend shape (a CK helper volume).
    pub is_helper: bool,
    pub is_water: bool,
    /// A part of a skinned or animated model.
    pub animated: bool,
}

/// How an object-path shape's visibility is decided.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub(super) enum ObjectVisibility {
    /// The hidden-effect rule of its material name (`MaterialName::is_hidden_effect_shape`).
    #[default]
    ByMaterialRule,
    /// Never drawn by the object path (a refraction shape no stage draws).
    Hidden,
    /// Always drawn by the object path: an effect shape on a skinned or animated model, which
    /// the effect stage cannot animate with its model, whatever the hidden-effect rule says.
    Drawn,
}

pub(super) struct ShapeClass {
    pub route: ShapeRoute,
    pub visibility: ObjectVisibility,
}

/// Classifies a submesh. Refraction wins over effect, because refraction materials carry an
/// `eff` token as well. A refraction shape the stage cannot draw stays hidden in the object
/// path; an effect shape on an animated model is drawn there with the object-shader effect
/// path, as no stage animates it with its model.
pub(super) fn classify(
    facts: &ShapeFacts,
    effect: Option<Arc<EffectMaterial>>,
    refraction: Option<Arc<RefractionMaterial>>,
    mesh: impl FnOnce() -> Arc<EffectMeshData>,
) -> ShapeClass {
    let object = |visibility| ShapeClass {
        route: ShapeRoute::Object,
        visibility,
    };
    if facts.is_refraction {
        return match refraction {
            Some(material) if !facts.animated => ShapeClass {
                route: ShapeRoute::Refraction {
                    mesh: mesh(),
                    material,
                },
                visibility: ObjectVisibility::ByMaterialRule,
            },
            _ => object(ObjectVisibility::Hidden),
        };
    }
    let effect_shape = facts.is_effect && !facts.is_helper && !facts.is_water;
    if effect_shape && facts.animated {
        return object(ObjectVisibility::Drawn);
    }
    match effect {
        Some(material) if effect_shape && facts.has_effect_key => ShapeClass {
            route: ShapeRoute::Effect {
                mesh: mesh(),
                material,
            },
            visibility: ObjectVisibility::ByMaterialRule,
        },
        _ => object(ObjectVisibility::ByMaterialRule),
    }
}

/// Identifies one attachment of a resident cell: the cell's residency and the attachment's
/// position in it. Stable for as long as the cell is resident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AttachmentKey {
    pub residency: u64,
    pub ordinal: u32,
}

/// An addon node of a placed model: where, in engine space, the addon attaches.
#[derive(Debug, Clone, Copy)]
pub struct AddonAttachment<'a> {
    pub key: AttachmentKey,
    pub node_name: &'a str,
    pub addon: &'a AddonNodeExtras,
    /// Engine space <- addon node: the placement's transform composed with the node's
    /// model-space transform.
    pub transform: Mat4,
}

pub(super) struct StoredAttachment {
    pub node_name: String,
    pub addon: AddonNodeExtras,
    pub transform: Mat4,
}

/// The attachments of every addon node of `extras` for each placement.
pub(super) fn attachments_of(extras: &ModelExtras, placements: &[Mat4]) -> Vec<StoredAttachment> {
    placements
        .iter()
        .flat_map(|placement| {
            extras.addon_nodes.iter().map(move |node| StoredAttachment {
                node_name: node.node_name.clone(),
                addon: node.value,
                transform: *placement * node.transform,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use serde_json::json;

    use super::*;
    use crate::assets::types::model_extras::NodeExtras;
    use crate::rendering::effects::test_fixtures::stub_material;

    fn facts() -> ShapeFacts {
        ShapeFacts {
            is_refraction: false,
            is_effect: true,
            has_effect_key: true,
            is_helper: false,
            is_water: false,
            animated: false,
        }
    }

    fn mesh() -> Arc<EffectMeshData> {
        Arc::new(EffectMeshData::from_vertices(Vec::new(), Vec::new()))
    }

    fn refraction() -> Arc<RefractionMaterial> {
        Arc::new(
            serde_json::from_value(json!({
                "v": 1, "strength": 1.0, "normal_a": "a.dds", "normal_b": "b.dds",
                "uv_offset": [0.0, 0.0], "uv_scale": [1.0, 1.0],
                "vertex_colors": true, "double_sided": true
            }))
            .unwrap(),
        )
    }

    fn effect() -> Arc<EffectMaterial> {
        Arc::new(stub_material())
    }

    #[test]
    fn an_effect_shape_with_extras_is_drawn_by_the_effect_stage() {
        let class = classify(&facts(), Some(effect()), None, mesh);
        assert!(matches!(class.route, ShapeRoute::Effect { .. }));
        assert_eq!(class.visibility, ObjectVisibility::ByMaterialRule);
    }

    #[test]
    fn an_effect_shape_without_extras_stays_a_toggleable_legacy_shape() {
        let without_key = ShapeFacts {
            has_effect_key: false,
            ..facts()
        };
        assert!(matches!(
            classify(&without_key, None, None, mesh).route,
            ShapeRoute::Object
        ));
        assert!(matches!(
            classify(&facts(), None, None, mesh).route,
            ShapeRoute::Object
        ));
    }

    #[test]
    fn effects_on_animated_models_are_drawn_by_the_object_path() {
        let animated = ShapeFacts {
            animated: true,
            ..facts()
        };
        let class = classify(&animated, Some(effect()), None, mesh);
        assert!(matches!(class.route, ShapeRoute::Object));
        assert_eq!(class.visibility, ObjectVisibility::Drawn);
    }

    #[test]
    fn helper_and_water_shapes_are_never_effects() {
        for facts in [
            ShapeFacts {
                is_helper: true,
                ..facts()
            },
            ShapeFacts {
                is_water: true,
                ..facts()
            },
            ShapeFacts {
                is_effect: false,
                ..facts()
            },
        ] {
            assert!(matches!(
                classify(&facts, Some(effect()), None, mesh).route,
                ShapeRoute::Object
            ));
        }
    }

    #[test]
    fn refraction_wins_over_the_effect_token_it_also_carries() {
        let both = ShapeFacts {
            is_refraction: true,
            ..facts()
        };
        let class = classify(&both, Some(effect()), Some(refraction()), mesh);
        assert!(matches!(class.route, ShapeRoute::Refraction { .. }));
        assert_eq!(class.visibility, ObjectVisibility::ByMaterialRule);
    }

    #[test]
    fn a_refraction_shape_no_stage_draws_is_hidden() {
        let refracting = ShapeFacts {
            is_refraction: true,
            is_effect: false,
            ..facts()
        };
        let without_extras = classify(&refracting, None, None, mesh);
        assert!(matches!(without_extras.route, ShapeRoute::Object));
        assert_eq!(without_extras.visibility, ObjectVisibility::Hidden);

        let animated = ShapeFacts {
            animated: true,
            ..refracting
        };
        let on_animated = classify(&animated, None, Some(refraction()), mesh);
        assert!(matches!(on_animated.route, ShapeRoute::Object));
        assert_eq!(on_animated.visibility, ObjectVisibility::Hidden);
    }

    fn node(name: &str, translation: Vec3) -> NodeExtras<AddonNodeExtras> {
        NodeExtras {
            node_name: name.to_owned(),
            transform: Mat4::from_translation(translation),
            value: AddonNodeExtras {
                v: 1,
                index: 5,
                flags: 0,
            },
        }
    }

    #[test]
    fn attachments_compose_each_placement_with_the_node_transform() {
        let mut extras = ModelExtras::default();
        extras
            .addon_nodes
            .push(node("addon", Vec3::new(0.0, 2.0, 0.0)));
        let placements = [
            Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0)),
            Mat4::from_translation(Vec3::new(0.0, 0.0, 30.0)),
        ];

        let attachments = attachments_of(&extras, &placements);

        assert_eq!(attachments.len(), 2);
        assert_eq!(
            attachments[0].transform.transform_point3(Vec3::ZERO),
            Vec3::new(10.0, 2.0, 0.0)
        );
        assert_eq!(
            attachments[1].transform.transform_point3(Vec3::ZERO),
            Vec3::new(0.0, 2.0, 30.0)
        );
        assert_eq!(attachments[0].addon.index, 5);
        assert_eq!(attachments[0].node_name, "addon");
    }

    #[test]
    fn a_model_without_addon_nodes_has_no_attachments() {
        assert!(attachments_of(&ModelExtras::default(), &[Mat4::IDENTITY]).is_empty());
    }
}
