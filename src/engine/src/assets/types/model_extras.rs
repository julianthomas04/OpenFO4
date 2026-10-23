//! Typed access to the converter's glTF extras: per-node particle systems,
//! master particle systems, addon nodes and lights, and per-material effect /
//! refraction descriptions. Every extras object carries `"v"`; an unknown
//! version is an error the caller reports before treating the shape as legacy.

use std::collections::HashSet;

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};

use super::effect_material::{EffectMaterial, RefractionMaterial};
use super::mesh::node_world_matrices;
use super::particle_system::{MasterParticleSystem, ParticleSystem};
use crate::anim::scalar_track::Track;

/// Major version of every extras object this engine reads.
pub const EXTRAS_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum ExtrasError {
    #[error("{kind} extras are not valid JSON: {source}")]
    Json {
        kind: &'static str,
        source: serde_json::Error,
    },
    #[error("{kind} extras carry no `v` version")]
    MissingVersion { kind: &'static str },
    #[error("{kind} extras `v` is not a non-negative integer")]
    NonIntegerVersion { kind: &'static str },
    #[error("{kind} extras version {found} is not supported (engine reads version {supported})")]
    UnsupportedVersion {
        kind: &'static str,
        found: u64,
        supported: u32,
    },
    #[error("{kind} extras are invalid: {source}")]
    Invalid {
        kind: &'static str,
        source: serde_json::Error,
    },
}

/// Deserializes an extras object after checking its `v` against
/// [`EXTRAS_VERSION`].
pub fn parse_versioned<T: DeserializeOwned>(
    kind: &'static str,
    value: &Value,
) -> Result<T, ExtrasError> {
    let version = value
        .get("v")
        .ok_or(ExtrasError::MissingVersion { kind })?
        .as_u64()
        .ok_or(ExtrasError::NonIntegerVersion { kind })?;
    if version != u64::from(EXTRAS_VERSION) {
        return Err(ExtrasError::UnsupportedVersion {
            kind,
            found: version,
            supported: EXTRAS_VERSION,
        });
    }
    T::deserialize(value).map_err(|source| ExtrasError::Invalid { kind, source })
}

/// The extras object of a glTF element; `None` when it has none or it is not
/// a JSON object (not ours).
fn extras_object(
    kind: &'static str,
    extras: &gltf::json::Extras,
) -> Result<Option<Map<String, Value>>, ExtrasError> {
    let Some(raw) = extras else { return Ok(None) };
    let value: Value =
        serde_json::from_str(raw.get()).map_err(|source| ExtrasError::Json { kind, source })?;
    Ok(match value {
        Value::Object(map) => Some(map),
        _ => None,
    })
}

fn parse_entry<T: DeserializeOwned>(
    kind: &'static str,
    extras: &gltf::json::Extras,
) -> Result<Option<T>, ExtrasError> {
    extras_object(kind, extras)?
        .and_then(|mut map| map.remove(kind))
        .map(|value| parse_versioned(kind, &value))
        .transpose()
}

/// `material.extras.effect`; `Ok(None)` for a material without it.
pub fn effect_material(material: &gltf::Material) -> Result<Option<EffectMaterial>, ExtrasError> {
    parse_entry("effect", material.extras())
}

/// `material.extras.refraction`; `Ok(None)` for a material without it.
pub fn refraction_material(
    material: &gltf::Material,
) -> Result<Option<RefractionMaterial>, ExtrasError> {
    parse_entry("refraction", material.extras())
}

/// `node.extras.addon_node`: the `BSValueNode` value of a marker model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddonNodeExtras {
    pub v: u32,
    pub index: i32,
    pub flags: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightKind {
    Point,
}

/// `node.extras.light`: animation tracks of a light model's point light.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LightExtras {
    pub v: u32,
    pub kind: LightKind,
    pub dimmer_rest: f32,
    /// `dimmer` (scalar) and `offset` (eu, model space) tracks.
    pub tracks: Vec<Track>,
}

/// `node.extras.damage_stage`: the damage range a `BSBlastNode` / `BSDamageStage`
/// subtree is active in. Kept as data; the undamaged stage is 0, so until
/// destruction is simulated every subtree follows its `hidden` flag alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DamageStage {
    pub min: i32,
    pub max: i32,
}

/// A transform in model space (engine axes and units). Parsing rejects a
/// rotation that is not finite or has zero length and a scale that is not
/// finite or is zero, so [`Self::matrix`] never yields NaN.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawConnectTransform")]
pub struct ConnectTransform {
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: f32,
}

#[derive(Deserialize)]
struct RawConnectTransform {
    translation: [f32; 3],
    rotation: [f32; 4],
    scale: f32,
}

impl TryFrom<RawConnectTransform> for ConnectTransform {
    type Error = String;

    fn try_from(raw: RawConnectTransform) -> Result<Self, Self::Error> {
        if raw.translation.iter().any(|value| !value.is_finite()) {
            return Err(format!("translation {:?} is not finite", raw.translation));
        }
        let rotation_length = Quat::from_array(raw.rotation).length();
        if !rotation_length.is_finite() || rotation_length <= f32::EPSILON {
            return Err(format!("rotation {:?} has no usable length", raw.rotation));
        }
        if !raw.scale.is_finite() || raw.scale == 0.0 {
            return Err(format!(
                "scale {} is not a finite non-zero number",
                raw.scale
            ));
        }
        Ok(Self {
            translation: raw.translation,
            rotation: raw.rotation,
            scale: raw.scale,
        })
    }
}

impl ConnectTransform {
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(
            Vec3::splat(self.scale),
            Quat::from_array(self.rotation).normalize(),
            Vec3::from_array(self.translation),
        )
    }
}

/// One named attachment point, local to the node that carries it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectPoint {
    pub name: String,
    #[serde(flatten)]
    pub transform: ConnectTransform,
}

/// `node.extras.connect_parents`: the points parts attach to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectParents {
    pub v: u32,
    pub points: Vec<ConnectPoint>,
}

/// `node.extras.connect_children` on a part's root node. Only the first node
/// carrying it counts (assembly reads the model's root); the converter writes
/// it on the root alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectChildren {
    pub v: u32,
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default)]
    pub skinned: bool,
    /// The source root transform, which the converter resets to identity.
    pub root: Option<ConnectTransform>,
}

/// A parsed extras value with the node it hangs on.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeExtras<T> {
    pub node_name: String,
    /// Model-space transform of the node (engine axes and units).
    pub transform: Mat4,
    pub value: T,
}

/// Extras of one node that could not be used.
#[derive(Debug)]
pub struct RejectedExtras {
    pub node_name: String,
    pub error: ExtrasError,
}

/// Every node-level extras object of a model file.
#[derive(Debug, Default)]
pub struct ModelExtras {
    pub particle_systems: Vec<NodeExtras<ParticleSystem>>,
    pub master_particle_systems: Vec<NodeExtras<MasterParticleSystem>>,
    pub addon_nodes: Vec<NodeExtras<AddonNodeExtras>>,
    pub lights: Vec<NodeExtras<LightExtras>>,
    pub damage_stages: Vec<NodeExtras<DamageStage>>,
    /// Attachment points; read on hidden nodes too and not part of
    /// [`Self::is_empty`], so no existing load path changes.
    pub connect_parents: Vec<NodeExtras<ConnectParents>>,
    pub connect_children: Vec<NodeExtras<ConnectChildren>>,
    /// Per glTF node index: the node or an ancestor is culled in the source
    /// model. Addon nodes, particle systems and lights on such nodes are
    /// dropped from this struct; the loaders skip their meshes.
    pub hidden_nodes: Vec<bool>,
    /// Extras of unknown version or invalid content; the caller reports them.
    pub rejected: Vec<RejectedExtras>,
}

impl ModelExtras {
    pub fn from_document(document: &gltf::Document) -> Self {
        let mut extras = Self {
            hidden_nodes: hidden_nodes(document),
            ..Self::default()
        };
        let mut worlds: Option<Vec<Mat4>> = None;
        for node in document.nodes() {
            let name = node.name().unwrap_or_default().to_owned();
            let map = match extras_object("node", node.extras()) {
                Ok(Some(map)) => map,
                Ok(None) => continue,
                Err(error) => {
                    extras.rejected.push(RejectedExtras {
                        node_name: name,
                        error,
                    });
                    continue;
                }
            };
            let mut context = NodeContext {
                name: &name,
                transform: None,
                worlds: &mut worlds,
                document,
                node_index: node.index(),
            };
            let hidden = extras.hidden_nodes[node.index()];
            extras.read_node(&map, &mut context, hidden);
        }
        extras
    }

    fn read_node(&mut self, map: &Map<String, Value>, context: &mut NodeContext, hidden: bool) {
        read_damage_stage(map, context, &mut self.damage_stages, &mut self.rejected);
        read_kind(
            map,
            "connect_parents",
            context,
            &mut self.connect_parents,
            &mut self.rejected,
        );
        read_kind(
            map,
            "connect_children",
            context,
            &mut self.connect_children,
            &mut self.rejected,
        );
        if hidden {
            return;
        }
        read_kind(
            map,
            "particle_system",
            context,
            &mut self.particle_systems,
            &mut self.rejected,
        );
        read_kind(
            map,
            "master_particle_system",
            context,
            &mut self.master_particle_systems,
            &mut self.rejected,
        );
        read_kind(
            map,
            "addon_node",
            context,
            &mut self.addon_nodes,
            &mut self.rejected,
        );
        read_kind(map, "light", context, &mut self.lights, &mut self.rejected);
    }

    pub fn is_empty(&self) -> bool {
        self.particle_systems.is_empty()
            && self.master_particle_systems.is_empty()
            && self.addon_nodes.is_empty()
            && self.lights.is_empty()
    }

    /// glTF node names of the geometry mesh emitters sample from.
    pub fn emitter_mesh_nodes(&self) -> HashSet<String> {
        self.particle_systems
            .iter()
            .flat_map(|system| &system.value.emitters)
            .flat_map(|emitter| emitter.mesh_nodes.iter().cloned())
            .collect()
    }
}

const HIDDEN_KEY: &str = "hidden";

fn is_flagged_hidden(node: &gltf::Node) -> bool {
    extras_object("node", node.extras())
        .ok()
        .flatten()
        .and_then(|map| map.get(HIDDEN_KEY).and_then(Value::as_bool))
        .unwrap_or(false)
}

/// Per node index: the node itself or any ancestor carries `"hidden": true`.
/// The converter marks only the culled object, never its descendants.
pub fn hidden_nodes(document: &gltf::Document) -> Vec<bool> {
    fn fill(node: &gltf::Node, inherited: bool, hidden: &mut [bool]) {
        let this = inherited || is_flagged_hidden(node);
        hidden[node.index()] = this;
        for child in node.children() {
            fill(&child, this, hidden);
        }
    }

    let mut hidden = vec![false; document.nodes().len()];
    let children: HashSet<usize> = document
        .nodes()
        .flat_map(|node| node.children().map(|child| child.index()))
        .collect();
    for root in document
        .nodes()
        .filter(|node| !children.contains(&node.index()))
    {
        fill(&root, false, &mut hidden);
    }
    hidden
}

/// Whether some mesh node is culled: a model whose meshes are all hidden is
/// valid and empty, unlike a file that never had any mesh.
pub fn has_hidden_mesh(document: &gltf::Document, hidden_nodes: &[bool]) -> bool {
    document
        .nodes()
        .any(|node| node.mesh().is_some() && hidden_nodes[node.index()])
}

struct NodeContext<'a> {
    name: &'a str,
    transform: Option<Mat4>,
    worlds: &'a mut Option<Vec<Mat4>>,
    document: &'a gltf::Document,
    node_index: usize,
}

impl NodeContext<'_> {
    fn transform(&mut self) -> Mat4 {
        *self.transform.get_or_insert_with(|| {
            let worlds = self
                .worlds
                .get_or_insert_with(|| node_world_matrices(self.document));
            worlds[self.node_index]
        })
    }
}

fn read_damage_stage(
    map: &Map<String, Value>,
    context: &mut NodeContext,
    found: &mut Vec<NodeExtras<DamageStage>>,
    rejected: &mut Vec<RejectedExtras>,
) {
    let Some(value) = map.get("damage_stage") else {
        return;
    };
    match DamageStage::deserialize(value) {
        Ok(value) => found.push(NodeExtras {
            node_name: context.name.to_owned(),
            transform: context.transform(),
            value,
        }),
        Err(source) => rejected.push(RejectedExtras {
            node_name: context.name.to_owned(),
            error: ExtrasError::Invalid {
                kind: "damage_stage",
                source,
            },
        }),
    }
}

fn read_kind<T: DeserializeOwned>(
    map: &Map<String, Value>,
    kind: &'static str,
    context: &mut NodeContext,
    found: &mut Vec<NodeExtras<T>>,
    rejected: &mut Vec<RejectedExtras>,
) {
    let Some(value) = map.get(kind) else { return };
    match parse_versioned::<T>(kind, value) {
        Ok(value) => found.push(NodeExtras {
            node_name: context.name.to_owned(),
            transform: context.transform(),
            value,
        }),
        Err(error) => rejected.push(RejectedExtras {
            node_name: context.name.to_owned(),
            error,
        }),
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals parsed or copied, not computed"
)]
#[allow(
    clippy::unreadable_literal,
    reason = "fixture numbers are copied verbatim from the converter output"
)]
mod tests {
    use glam::Vec3;
    use serde_json::json;

    use super::*;
    use crate::anim::scalar_track::TrackTarget;
    use crate::installer_source;

    #[test]
    fn every_extras_version_the_installer_writes_is_the_one_the_engine_reads() {
        let writers = [
            include_str!("../../../../installer/file_converters/nif_node_extras.cs"),
            include_str!("../../../../installer/file_converters/nif_particle_converter.cs"),
            include_str!("../../../../installer/file_converters/effect_material.cs"),
        ];
        for installer in writers {
            assert_eq!(
                installer_source::constant(installer, "Version"),
                EXTRAS_VERSION.to_string()
            );
        }
    }

    fn document(value: &Value) -> gltf::Document {
        let bytes = value.to_string().into_bytes();
        gltf::Gltf::from_slice(&bytes).unwrap().document
    }

    fn light_extras() -> Value {
        json!({
            "v": 1, "kind": "point", "dimmer_rest": 2.5,
            "tracks": [
                { "target": "dimmer", "cycle": "loop", "start": 0.0, "stop": 3.3,
                  "times": [0.0, 3.3], "values": [2.5, 2.5] },
                { "target": "offset", "cycle": "loop", "start": 0.0, "stop": 3.3,
                  "times": [0.0, 3.3], "values": [[0, 0, 0], [0.1, 0, 0]] }
            ]
        })
    }

    #[test]
    fn light_extras_round_trip() {
        let light: LightExtras = parse_versioned("light", &light_extras()).unwrap();
        assert_eq!(light.dimmer_rest, 2.5);
        assert_eq!(light.tracks[1].target, TrackTarget::Offset);
        let again: LightExtras =
            parse_versioned("light", &serde_json::to_value(&light).unwrap()).unwrap();
        assert_eq!(light, again);
    }

    #[test]
    fn addon_node_extras_round_trip() {
        let value = json!({ "v": 1, "index": 5, "flags": 0 });
        let node: AddonNodeExtras = parse_versioned("addon_node", &value).unwrap();
        assert_eq!(node.index, 5);
        let again: AddonNodeExtras =
            parse_versioned("addon_node", &serde_json::to_value(node).unwrap()).unwrap();
        assert_eq!(node, again);
    }

    #[test]
    fn version_errors_are_distinct_variants() {
        let missing = parse_versioned::<AddonNodeExtras>("addon_node", &json!({"index": 5}));
        assert!(matches!(missing, Err(ExtrasError::MissingVersion { .. })));
        let text = parse_versioned::<AddonNodeExtras>("addon_node", &json!({"v": "1"}));
        assert!(matches!(text, Err(ExtrasError::NonIntegerVersion { .. })));
        let future = parse_versioned::<AddonNodeExtras>(
            "addon_node",
            &json!({"v": 2, "index": 5, "flags": 0}),
        );
        assert!(matches!(
            future,
            Err(ExtrasError::UnsupportedVersion {
                found: 2,
                supported: 1,
                ..
            })
        ));
        let invalid = parse_versioned::<AddonNodeExtras>("addon_node", &json!({"v": 1}));
        assert!(matches!(invalid, Err(ExtrasError::Invalid { .. })));
    }

    fn nodes_document() -> Value {
        json!({
            "asset": {"version": "2.0"},
            "scenes": [{"nodes": [0]}],
            "nodes": [
                {"name": "Root", "children": [1, 2, 3, 4], "translation": [10, 0, 0]},
                {"name": "FXMPS", "translation": [0, 1, 0], "extras": {
                    "master_particle_system":
                        {"v": 1, "max_emitter_objects": 20, "systems": ["Sparks"]}}},
                {"name": "Marker", "translation": [0, 0, 2],
                 "extras": {"addon_node": {"v": 1, "index": 5, "flags": 0}}},
                {"name": "Flicker", "extras": {"light": light_extras()}},
                {"name": "FromTheFuture", "extras": {"addon_node": {"v": 9}}}
            ]
        })
    }

    #[test]
    fn node_extras_carry_the_composed_model_transform() {
        let extras = ModelExtras::from_document(&document(&nodes_document()));
        assert_eq!(extras.addon_nodes.len(), 1);
        let marker = &extras.addon_nodes[0];
        assert_eq!(marker.node_name, "Marker");
        assert_eq!(marker.value.index, 5);
        assert_eq!(
            marker.transform.transform_point3(Vec3::ZERO),
            Vec3::new(10.0, 0.0, 2.0)
        );
        assert_eq!(extras.master_particle_systems[0].value.systems, ["Sparks"]);
        assert_eq!(extras.lights.len(), 1);
        assert!(!extras.is_empty());
    }

    #[test]
    fn unsupported_node_extras_are_reported_not_dropped() {
        let extras = ModelExtras::from_document(&document(&nodes_document()));
        assert_eq!(extras.rejected.len(), 1);
        assert_eq!(extras.rejected[0].node_name, "FromTheFuture");
        assert!(matches!(
            extras.rejected[0].error,
            ExtrasError::UnsupportedVersion { found: 9, .. }
        ));
    }

    #[test]
    fn document_without_extras_is_empty() {
        let doc = json!({"asset": {"version": "2.0"}, "nodes": [{"name": "A"}]});
        let extras = ModelExtras::from_document(&document(&doc));
        assert!(extras.is_empty());
        assert!(extras.rejected.is_empty());
    }

    #[test]
    fn material_extras_parse_effect_and_refraction() {
        let doc = json!({
            "asset": {"version": "2.0"},
            "materials": [
                {"name": "|", "extras": {"refraction": {
                    "v": 1, "strength": 0.04, "normal_a": "a_n.dds", "normal_b": "b_n.dds",
                    "uv_offset": [0, 0], "uv_scale": [1, 1],
                    "vertex_colors": true, "double_sided": false}}},
                {"name": "plain"},
                {"name": "future", "extras": {"effect": {"v": 3}}}
            ]
        });
        let document = document(&doc);
        let materials: Vec<_> = document.materials().collect();
        let refraction = refraction_material(&materials[0]).unwrap().unwrap();
        assert_eq!(refraction.strength, 0.04);
        assert!(effect_material(&materials[0]).unwrap().is_none());
        assert!(refraction_material(&materials[1]).unwrap().is_none());
        assert!(matches!(
            effect_material(&materials[2]),
            Err(ExtrasError::UnsupportedVersion { found: 3, .. })
        ));
    }

    fn hidden_document() -> Value {
        json!({
            "asset": {"version": "2.0"},
            "scenes": [{"nodes": [0]}],
            "nodes": [
                {"name": "Root", "children": [1, 4]},
                {"name": "Stage", "children": [2, 3], "extras": {
                    "hidden": true, "damage_stage": {"min": 1, "max": 3}}},
                {"name": "Marker", "extras": {"addon_node": {"v": 1, "index": 5, "flags": 0}}},
                {"name": "Glow", "extras": {"light": light_extras()}},
                {"name": "Visible", "extras": {
                    "addon_node": {"v": 1, "index": 6, "flags": 0},
                    "damage_stage": {"min": 0, "max": 0}}}
            ]
        })
    }

    #[test]
    fn hidden_is_inherited_through_the_parent_chain() {
        let hidden = hidden_nodes(&document(&hidden_document()));
        assert_eq!(hidden, [false, true, true, true, false]);
    }

    #[test]
    fn attachments_under_a_hidden_ancestor_are_not_read() {
        let extras = ModelExtras::from_document(&document(&hidden_document()));
        assert_eq!(extras.addon_nodes.len(), 1);
        assert_eq!(extras.addon_nodes[0].node_name, "Visible");
        assert_eq!(extras.lights, []);
    }

    #[test]
    fn damage_stage_is_parsed_on_hidden_and_visible_nodes() {
        let extras = ModelExtras::from_document(&document(&hidden_document()));
        let stages: Vec<_> = extras
            .damage_stages
            .iter()
            .map(|node| (node.node_name.as_str(), node.value))
            .collect();
        assert_eq!(
            stages,
            [
                ("Stage", DamageStage { min: 1, max: 3 }),
                ("Visible", DamageStage { min: 0, max: 0 })
            ]
        );
    }

    #[test]
    fn malformed_damage_stage_is_reported() {
        let doc = json!({"asset": {"version": "2.0"}, "nodes": [
            {"name": "Bad", "extras": {"damage_stage": {"min": "x"}}}]});
        let extras = ModelExtras::from_document(&document(&doc));
        assert_eq!(extras.damage_stages, []);
        assert!(matches!(
            extras.rejected[0].error,
            ExtrasError::Invalid {
                kind: "damage_stage",
                ..
            }
        ));
    }

    fn connect_document() -> Value {
        json!({
            "asset": {"version": "2.0"},
            "scenes": [{"nodes": [0]}],
            "nodes": [
                {"name": "Root", "children": [1], "extras": {
                    "connect_parents": {"v": 1, "points": [
                        {"name": "P-Receiver", "translation": [0, 0, 0],
                         "rotation": [0, 0, 0, 1], "scale": 1.0}]},
                    "connect_children": {"v": 1, "names": ["C-Grip"], "skinned": false,
                        "root": {"translation": [0, 0, 0],
                                 "rotation": [0, 0, 0.0871557, 0.9961947], "scale": 1.0}}}},
                {"name": "Receiver", "translation": [0, 2, 0], "extras": {
                    "hidden": true,
                    "connect_parents": {"v": 1, "points": [
                        {"name": "P-Mag", "translation": [1, 0, 0],
                         "rotation": [0, 0, 0, 1], "scale": 1.0}]}}}
            ]
        })
    }

    #[test]
    fn connect_points_are_read_with_node_transform_and_on_hidden_nodes() {
        let extras = ModelExtras::from_document(&document(&connect_document()));
        assert_eq!(extras.connect_parents.len(), 2);
        let hidden = &extras.connect_parents[1];
        assert_eq!(hidden.node_name, "Receiver");
        assert_eq!(hidden.value.points[0].name, "P-Mag");
        assert_eq!(
            hidden.transform.transform_point3(Vec3::ZERO),
            Vec3::new(0.0, 2.0, 0.0)
        );
        assert_eq!(
            hidden.value.points[0].transform.matrix().w_axis.truncate(),
            Vec3::X
        );
        assert!(extras.rejected.is_empty());
    }

    #[test]
    fn connect_children_root_is_optional() {
        let extras = ModelExtras::from_document(&document(&connect_document()));
        let children = &extras.connect_children[0].value;
        assert_eq!(children.names, ["C-Grip"]);
        assert_eq!(children.root.unwrap().rotation[2], 0.0871557);

        let doc = json!({"asset": {"version": "2.0"}, "nodes": [
            {"name": "Part", "extras": {"connect_children": {"v": 1}}}]});
        let extras = ModelExtras::from_document(&document(&doc));
        assert!(extras.connect_children[0].value.root.is_none());
    }

    #[test]
    fn connect_points_do_not_make_extras_non_empty() {
        let extras = ModelExtras::from_document(&document(&connect_document()));
        assert!(extras.is_empty());
    }

    #[test]
    fn malformed_connect_points_are_reported() {
        let doc = json!({"asset": {"version": "2.0"}, "nodes": [
            {"name": "Bad", "extras": {"connect_parents": {"v": 1, "points": [{"name": 3}]}}}]});
        let extras = ModelExtras::from_document(&document(&doc));
        assert_eq!(extras.connect_parents, []);
        assert!(matches!(
            extras.rejected[0].error,
            ExtrasError::Invalid {
                kind: "connect_parents",
                ..
            }
        ));
    }

    fn point_document(translation: &Value, rotation: &Value, scale: &Value) -> Value {
        json!({"asset": {"version": "2.0"}, "nodes": [
            {"name": "Bad", "extras": {"connect_parents": {"v": 1, "points": [
                {"name": "P", "translation": translation, "rotation": rotation,
                 "scale": scale}]}}}]})
    }

    #[test]
    fn unusable_connect_transforms_are_rejected_at_parse_time() {
        let cases = [
            (
                "zero rotation",
                json!([0, 0, 0]),
                json!([0, 0, 0, 0]),
                json!(1.0),
            ),
            (
                "zero scale",
                json!([0, 0, 0]),
                json!([0, 0, 0, 1]),
                json!(0.0),
            ),
            (
                "huge rotation",
                json!([0, 0, 0]),
                json!([3.0e38, 3.0e38, 0, 0]),
                json!(1.0),
            ),
        ];
        for (label, translation, rotation, scale) in cases {
            let doc = point_document(&translation, &rotation, &scale);
            let extras = ModelExtras::from_document(&document(&doc));
            assert!(extras.connect_parents.is_empty(), "{label} was accepted");
            assert!(
                matches!(
                    extras.rejected[0].error,
                    ExtrasError::Invalid {
                        kind: "connect_parents",
                        ..
                    }
                ),
                "{label}"
            );
        }
    }

    #[test]
    fn a_connect_transform_that_is_not_unit_length_is_normalized() {
        let doc = point_document(&json!([0, 0, 0]), &json!([0, 0, 2, 0]), &json!(1.0));
        let extras = ModelExtras::from_document(&document(&doc));
        let matrix = extras.connect_parents[0].value.points[0].transform.matrix();
        assert!(matrix.to_cols_array().iter().all(|value| value.is_finite()));
        assert!((matrix.x_axis.x - -1.0).abs() < 1e-6);
    }
}
