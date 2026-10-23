//! Pure assembly of a placed weapon: the base model plus the models of its
//! default mods, each positioned by matching the mod's attach point to a
//! connect point of an already placed part. Inputs are the connect-point
//! extras of the GLBs and the mod rows; no file or database access.

use std::collections::HashSet;
use std::sync::Arc;

use glam::Mat4;

use super::model_extras::{ExtrasError, ModelExtras};

#[derive(Debug, Clone, PartialEq)]
pub struct ParentPoint {
    /// As authored; matching ignores case.
    pub name: String,
    /// Model-space matrix of the node carrying the point.
    pub node_world: Mat4,
    /// The point's transform, local to that node.
    pub local: Mat4,
}

/// Connect-point extras of one node that could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedConnect {
    pub node_name: String,
    pub kind: &'static str,
    pub reason: String,
}

const CONNECT_KINDS: [&str; 2] = ["connect_parents", "connect_children"];

/// What assembly needs from one model file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConnectInfo {
    /// Nodes in glTF index order, points in array order.
    pub parents: Vec<ParentPoint>,
    /// `None` when the model carries no `connect_children.root`.
    pub child_root: Option<Mat4>,
    /// Whether the model file has any mesh node; the dummy root of a gun does not.
    pub has_geometry: bool,
    /// Connect-point extras the reader dropped; the caller reports them.
    pub rejected: Vec<RejectedConnect>,
}

impl ConnectInfo {
    /// Reads the glTF document alone, so a model without meshes is accepted.
    pub fn from_document(document: &gltf::Document) -> Self {
        let mut info = Self::from_extras(&ModelExtras::from_document(document));
        info.has_geometry = document.nodes().any(|node| node.mesh().is_some());
        info
    }

    pub fn from_extras(extras: &ModelExtras) -> Self {
        let parents = extras
            .connect_parents
            .iter()
            .flat_map(|node| {
                node.value.points.iter().map(|point| ParentPoint {
                    name: point.name.clone(),
                    node_world: node.transform,
                    local: point.transform.matrix(),
                })
            })
            .collect();
        let child_root = extras
            .connect_children
            .first()
            .and_then(|node| node.value.root)
            .map(|root| root.matrix());
        let rejected = extras
            .rejected
            .iter()
            .filter_map(|rejected| {
                let kind = error_kind(&rejected.error);
                CONNECT_KINDS.contains(&kind).then(|| RejectedConnect {
                    node_name: rejected.node_name.clone(),
                    kind,
                    reason: rejected.error.to_string(),
                })
            })
            .collect();
        Self {
            parents,
            child_root,
            has_geometry: false,
            rejected,
        }
    }
}

fn error_kind(error: &ExtrasError) -> &'static str {
    match error {
        ExtrasError::Json { kind, .. }
        | ExtrasError::MissingVersion { kind }
        | ExtrasError::NonIntegerVersion { kind }
        | ExtrasError::UnsupportedVersion { kind, .. }
        | ExtrasError::Invalid { kind, .. } => kind,
    }
}

/// The columns of an `object_modifications` row assembly reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModRow {
    /// `None`: a null mod without a model.
    pub model_path: Option<String>,
    pub attach_point_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WeaponPart {
    /// Model path as stored in the database (or the weapon's base model).
    pub model_path: String,
    pub part_to_weapon: Mat4,
    /// `None` for the base model.
    pub mod_key: Option<String>,
    pub connect: Arc<ConnectInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Assembly {
    pub base: WeaponPart,
    /// Mod parts in placement order.
    pub mods: Vec<WeaponPart>,
    /// Attach point names (`"(none)"` for a part without one) that found no
    /// connect point.
    pub unmatched: Vec<String>,
    /// Mod form keys without a row or whose model could not be read.
    pub unresolved: Vec<String>,
    /// Placed parts without a `connect_children.root`.
    pub no_root: usize,
    /// Mod form keys that appear more than once in the default combination;
    /// only the first occurrence is placed.
    pub duplicates: Vec<String>,
}

impl Assembly {
    /// Every part in placement order, the base model first.
    pub fn parts(&self) -> impl Iterator<Item = &WeaponPart> {
        std::iter::once(&self.base).chain(&self.mods)
    }
}

const NO_ATTACH_POINT: &str = "(none)";

struct Pending {
    mod_key: String,
    model_path: String,
    attach_point: Option<String>,
    connect: Arc<ConnectInfo>,
}

/// Connect point name an attach point `ap_<kind>_<slot>` connects to: `p-<slot>`.
fn parent_point_name(attach_point: &str) -> String {
    let slot = attach_point
        .rsplit_once('_')
        .map_or(attach_point, |(_, slot)| slot);
    format!("p-{}", slot.to_ascii_lowercase())
}

fn attach_transform(placed: &Assembly, pending: &Pending) -> Option<Mat4> {
    let wanted = parent_point_name(pending.attach_point.as_deref()?);
    placed.parts().find_map(|parent| {
        let point = parent
            .connect
            .parents
            .iter()
            .find(|point| point.name.eq_ignore_ascii_case(&wanted))?;
        let child_root = pending.connect.child_root.unwrap_or(Mat4::IDENTITY);
        Some(parent.part_to_weapon * point.node_world * point.local * child_root)
    })
}

/// Positions the default mods of one weapon. `mod_row` and `connect_info`
/// resolve a mod form key and a model path; `connect_info` returns `None` when
/// the model cannot be read.
pub fn assemble(
    base_model_path: &str,
    base_connect: Arc<ConnectInfo>,
    default_mods: &[String],
    mod_row: impl Fn(&str) -> Option<ModRow>,
    mut connect_info: impl FnMut(&str) -> Option<Arc<ConnectInfo>>,
) -> Assembly {
    let mut assembly = Assembly {
        base: WeaponPart {
            model_path: base_model_path.to_owned(),
            part_to_weapon: Mat4::IDENTITY,
            mod_key: None,
            connect: base_connect,
        },
        mods: Vec::new(),
        unmatched: Vec::new(),
        unresolved: Vec::new(),
        no_root: 0,
        duplicates: Vec::new(),
    };
    let mut pending = Vec::new();
    let mut seen = HashSet::new();
    for mod_key in default_mods {
        if !seen.insert(mod_key.as_str()) {
            if !assembly.duplicates.contains(mod_key) {
                assembly.duplicates.push(mod_key.clone());
            }
            continue;
        }
        let Some(row) = mod_row(mod_key) else {
            assembly.unresolved.push(mod_key.clone());
            continue;
        };
        let Some(model_path) = row.model_path else {
            continue;
        };
        match connect_info(&model_path) {
            Some(connect) => pending.push(Pending {
                mod_key: mod_key.clone(),
                model_path,
                attach_point: row.attach_point_name,
                connect,
            }),
            None => assembly.unresolved.push(mod_key.clone()),
        }
    }

    loop {
        let before = pending.len();
        let mut still_pending = Vec::new();
        for part in pending {
            match attach_transform(&assembly, &part) {
                Some(part_to_weapon) => {
                    assembly.no_root += usize::from(part.connect.child_root.is_none());
                    assembly.mods.push(WeaponPart {
                        model_path: part.model_path,
                        part_to_weapon,
                        mod_key: Some(part.mod_key),
                        connect: part.connect,
                    });
                }
                None => still_pending.push(part),
            }
        }
        pending = still_pending;
        if pending.is_empty() || pending.len() == before {
            break;
        }
    }
    assembly.unmatched = pending
        .into_iter()
        .map(|part| {
            part.attach_point
                .unwrap_or_else(|| NO_ATTACH_POINT.to_owned())
        })
        .collect();
    assembly
}

#[cfg(test)]
#[allow(
    clippy::unreadable_literal,
    clippy::excessive_precision,
    reason = "fixture numbers are copied verbatim from the converter output"
)]
mod tests {
    use std::collections::HashMap;

    use glam::{Quat, Vec3};

    use super::*;

    const TOLERANCE: f32 = 1e-5;
    const IDENTITY_Q: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

    fn pose(t: [f32; 3], q: [f32; 4]) -> Mat4 {
        Mat4::from_rotation_translation(Quat::from_xyzw(q[0], q[1], q[2], q[3]), Vec3::from(t))
    }

    fn assert_pose(actual: Mat4, t: [f32; 3], q: [f32; 4]) {
        let expected = pose(t, q);
        let worst = actual
            .to_cols_array()
            .iter()
            .zip(expected.to_cols_array())
            .map(|(a, e)| (a - e).abs())
            .fold(0.0, f32::max);
        assert!(
            worst < TOLERANCE,
            "{actual:?} != {expected:?} (off by {worst})"
        );
    }

    fn point(name: &str, node_world: Mat4, t: [f32; 3], q: [f32; 4]) -> ParentPoint {
        ParentPoint {
            name: name.to_owned(),
            node_world,
            local: pose(t, q),
        }
    }

    fn origin_point(name: &str) -> ParentPoint {
        point(name, Mat4::IDENTITY, [0.0; 3], IDENTITY_Q)
    }

    fn info(parents: Vec<ParentPoint>, child_root: Option<Mat4>) -> Arc<ConnectInfo> {
        Arc::new(ConnectInfo {
            parents,
            child_root,
            has_geometry: true,
            rejected: vec![],
        })
    }

    fn leaf() -> Arc<ConnectInfo> {
        info(vec![], Some(Mat4::IDENTITY))
    }

    fn receiver_base() -> Arc<ConnectInfo> {
        info(vec![origin_point("P-Receiver")], None)
    }

    #[derive(Default)]
    struct Fixture {
        mods: HashMap<&'static str, ModRow>,
        infos: HashMap<&'static str, Arc<ConnectInfo>>,
    }

    impl Fixture {
        fn part(
            mut self,
            key: &'static str,
            attach_point: Option<&str>,
            model: Option<&'static str>,
            connect: impl Into<Option<Arc<ConnectInfo>>>,
        ) -> Self {
            self.mods.insert(
                key,
                ModRow {
                    model_path: model.map(str::to_owned),
                    attach_point_name: attach_point.map(str::to_owned),
                },
            );
            if let (Some(model), Some(connect)) = (model, connect.into()) {
                self.infos.insert(model, connect);
            }
            self
        }

        fn assemble(&self, base: Arc<ConnectInfo>, default_mods: &[&str]) -> Assembly {
            let mods: Vec<String> = default_mods.iter().map(|key| (*key).to_owned()).collect();
            assemble(
                "base.nif",
                base,
                &mods,
                |key| self.mods.get(key).cloned(),
                |model| self.infos.get(model).cloned(),
            )
        }
    }

    fn part_pose(assembly: &Assembly, index: usize) -> Mat4 {
        assembly.parts().nth(index).unwrap().part_to_weapon
    }

    fn model_paths(assembly: &Assembly) -> Vec<&str> {
        assembly
            .parts()
            .map(|part| part.model_path.as_str())
            .collect()
    }

    fn ten_mm() -> Fixture {
        let receiver = info(
            vec![
                point(
                    "P-Grip",
                    Mat4::IDENTITY,
                    [-0.002276, -0.237420, 0.060138],
                    IDENTITY_Q,
                ),
                point(
                    "P-Barrel",
                    Mat4::IDENTITY,
                    [-0.003435, 0.486862, -1.851451],
                    IDENTITY_Q,
                ),
                point(
                    "P-Mag",
                    pose(
                        [-0.001670, -0.127423, -0.013520],
                        [-0.2293388, 0.0000157, 0.0000632, 0.9733466],
                    ),
                    [-0.000516, -0.074238, 0.020398],
                    [0.2293391, -0.0000153, -0.0000631, 0.9733466],
                ),
            ],
            Some(pose(
                [0.0; 3],
                [0.0016753, 0.0010093, -0.0037573, 0.9999910],
            )),
        );
        let grip_root = pose([0.0; 3], [0.0016754, 0.0010094, -0.0037573, 0.9999910]);
        Fixture::default()
            .part(
                "receiver",
                Some("ap_gun_receiver"),
                Some("receiver.nif"),
                Some(receiver),
            )
            .part(
                "grip",
                Some("ap_gun_Grip"),
                Some("grip.nif"),
                Some(info(vec![], Some(grip_root))),
            )
            .part("barrel", Some("ap_gun_Barrel"), Some("barrel.nif"), leaf())
            .part("mag", Some("ap_gun_Mag"), Some("mag.nif"), leaf())
            .part("null_muzzle", Some("ap_gun_Muzzle"), None, None)
    }

    #[test]
    fn ten_mm_parts_chain_off_the_receiver() {
        let result = ten_mm().assemble(
            receiver_base(),
            &["receiver", "grip", "barrel", "null_muzzle", "mag"],
        );
        assert_eq!(
            model_paths(&result),
            [
                "base.nif",
                "receiver.nif",
                "grip.nif",
                "barrel.nif",
                "mag.nif"
            ]
        );
        assert!(result.unmatched.is_empty() && result.unresolved.is_empty());
        let part = |index: usize| result.parts().collect::<Vec<_>>()[index].part_to_weapon;
        assert_pose(part(0), [0.0; 3], IDENTITY_Q);
        assert_pose(
            part(1),
            [0.0; 3],
            [0.0016746, 0.0010086, -0.0037573, 0.9999910],
        );
        assert_pose(
            part(2),
            [-0.003941, -0.237597, 0.059347],
            [0.0033503, 0.0020183, -0.0075145, 0.9999641],
        );
        assert_pose(
            part(3),
            [-0.003486, 0.493085, -1.849804],
            [0.0016750, 0.0010090, -0.0037573, 0.9999910],
        );
        assert_pose(
            part(4),
            [-0.003489, -0.184850, 0.037262],
            [0.0016749, 0.0010090, -0.0037573, 0.9999910],
        );
        assert_eq!(result.no_root, 0);
    }

    #[test]
    fn parts_listed_before_their_parent_are_placed_in_a_later_pass() {
        let result = ten_mm().assemble(receiver_base(), &["grip", "receiver"]);
        assert_eq!(
            model_paths(&result),
            ["base.nif", "receiver.nif", "grip.nif"]
        );
        assert_pose(
            part_pose(&result, 2),
            [-0.003941, -0.237597, 0.059347],
            [0.0033503, 0.0020183, -0.0075145, 0.9999641],
        );
    }

    #[test]
    fn pipe_gun_parts_attach_at_their_connect_points() {
        let receiver = info(
            vec![
                point(
                    "P-Barrel",
                    Mat4::IDENTITY,
                    [0.001048, 0.101631, -2.638134],
                    IDENTITY_Q,
                ),
                origin_point("P-Grip"),
                point(
                    "P-Mag",
                    pose([-0.249570, -0.065446, -0.962628], IDENTITY_Q),
                    [0.0; 3],
                    IDENTITY_Q,
                ),
                point(
                    "P-Scope",
                    Mat4::IDENTITY,
                    [0.000465, 0.259985, -0.247885],
                    IDENTITY_Q,
                ),
            ],
            Some(Mat4::IDENTITY),
        );
        let fixture = Fixture::default()
            .part(
                "receiver",
                Some("ap_gun_receiver"),
                Some("r.nif"),
                Some(receiver),
            )
            .part("barrel", Some("ap_gun_Barrel"), Some("b.nif"), leaf())
            .part("grip", Some("ap_gun_Grip"), Some("g.nif"), leaf())
            .part("mag", Some("ap_gun_Mag"), Some("m.nif"), leaf())
            .part("scope", Some("ap_gun_Scope"), Some("s.nif"), leaf());
        let result = fixture.assemble(
            receiver_base(),
            &["receiver", "barrel", "grip", "mag", "scope"],
        );
        let expected = [
            [0.0; 3],
            [0.001051, 0.101627, -2.638135],
            [0.0; 3],
            [-0.249569, -0.065448, -0.962629],
            [0.000465, 0.259985, -0.247886],
        ];
        for (part, translation) in result.mods.iter().zip(expected) {
            assert_pose(part.part_to_weapon, translation, IDENTITY_Q);
        }
    }

    #[test]
    fn melee_root_rotation_cancels_the_point_rotation() {
        let base = info(
            vec![point(
                "p-Material",
                Mat4::IDENTITY,
                [0.0; 3],
                [0.0, 0.0, 0.0871557, 0.9961947],
            )],
            None,
        );
        let grip_root = pose([0.0; 3], [0.0, 0.0, -0.0871558, 0.9961947]);
        let fixture = Fixture::default().part(
            "grip",
            Some("ap_melee_Material"),
            Some("grip.nif"),
            Some(info(vec![], Some(grip_root))),
        );
        let result = fixture.assemble(base, &["grip"]);
        assert_pose(part_pose(&result, 1), [0.0; 3], IDENTITY_Q);
    }

    #[test]
    fn missile_launcher_barrel_keeps_its_half_turn_root() {
        let receiver = info(
            vec![point(
                "P-Barrel",
                Mat4::IDENTITY,
                [0.0, 1.098690, 1.027684],
                IDENTITY_Q,
            )],
            Some(Mat4::IDENTITY),
        );
        let barrel_root = pose([0.0; 3], [0.0, -1.0, 0.0, 0.0]);
        let fixture = Fixture::default()
            .part(
                "receiver",
                Some("ap_gun_receiver"),
                Some("r.nif"),
                Some(receiver),
            )
            .part(
                "barrel",
                Some("ap_gun_Barrel"),
                Some("b.nif"),
                Some(info(vec![], Some(barrel_root))),
            );
        let result = fixture.assemble(receiver_base(), &["receiver", "barrel"]);
        assert_pose(
            part_pose(&result, 2),
            [0.0, 1.098690, 1.027684],
            [0.0, -1.0, 0.0, 0.0],
        );
    }

    #[test]
    fn attach_point_resolves_to_its_last_segment_lower_cased() {
        assert_eq!(parent_point_name("ap_Gun_UniversalOffset_Range"), "p-range");
        assert_eq!(parent_point_name("ap_gun_receiver"), "p-receiver");
        assert_eq!(parent_point_name("ap_melee_Material"), "p-material");
    }

    #[test]
    fn duplicate_connect_points_resolve_to_the_first() {
        let base = info(
            vec![
                point("P-Scope", Mat4::IDENTITY, [1.0, 0.0, 0.0], IDENTITY_Q),
                point("P-Scope", Mat4::IDENTITY, [2.0, 0.0, 0.0], IDENTITY_Q),
            ],
            None,
        );
        let fixture = Fixture::default().part("scope", Some("ap_gun_Scope"), Some("s.nif"), leaf());
        let result = fixture.assemble(base, &["scope"]);
        assert_pose(part_pose(&result, 1), [1.0, 0.0, 0.0], IDENTITY_Q);
    }

    #[test]
    fn null_mods_are_skipped_silently() {
        let fixture = Fixture::default().part("muzzle", Some("ap_gun_Muzzle"), None, None);
        let result = fixture.assemble(info(vec![], None), &["muzzle"]);
        assert!(result.mods.is_empty());
        assert!(result.unmatched.is_empty() && result.unresolved.is_empty());
    }

    #[test]
    fn unknown_mods_and_unreadable_models_are_unresolved() {
        let fixture =
            Fixture::default().part("broken", Some("ap_gun_Grip"), Some("broken.nif"), None);
        let result = fixture.assemble(info(vec![], None), &["missing", "broken"]);
        assert!(result.mods.is_empty());
        assert_eq!(result.unresolved, ["missing", "broken"]);
    }

    #[test]
    fn parts_without_a_matching_point_are_unmatched_and_the_rest_still_spawn() {
        let fixture = Fixture::default()
            .part("receiver", Some("ap_gun_receiver"), Some("r.nif"), leaf())
            .part("orphan", Some("ap_gun_Nowhere"), Some("o.nif"), leaf())
            .part("no_ap", None, Some("n.nif"), leaf());
        let result = fixture.assemble(receiver_base(), &["orphan", "no_ap", "receiver"]);
        assert_eq!(model_paths(&result), ["base.nif", "r.nif"]);
        assert_eq!(result.unmatched, ["ap_gun_Nowhere", "(none)"]);
    }

    #[test]
    fn placed_parts_without_child_root_are_counted() {
        let fixture = Fixture::default().part(
            "receiver",
            Some("ap_gun_receiver"),
            Some("r.nif"),
            Some(info(vec![], None)),
        );
        let result = fixture.assemble(receiver_base(), &["receiver"]);
        assert_eq!(result.no_root, 1);
        assert_pose(part_pose(&result, 1), [0.0; 3], IDENTITY_Q);
    }

    #[test]
    fn from_extras_flattens_points_with_their_node_transform() {
        let doc = serde_json::json!({
            "asset": {"version": "2.0"},
            "scenes": [{"nodes": [0]}],
            "nodes": [
                {"name": "Root", "children": [1], "extras": {
                    "connect_children": {"v": 1, "root": {
                        "translation": [0, 0, 0], "rotation": [0, 0, 0, 1], "scale": 1.0}}}},
                {"name": "Receiver", "translation": [0, 2, 0], "extras": {
                    "connect_parents": {"v": 1, "points": [
                        {"name": "P-Grip", "translation": [1, 0, 0],
                         "rotation": [0, 0, 0, 1], "scale": 1.0}]}}}
            ]
        });
        let bytes = doc.to_string().into_bytes();
        let document = gltf::Gltf::from_slice(&bytes).unwrap().document;
        let connect = ConnectInfo::from_extras(&ModelExtras::from_document(&document));
        assert_eq!(connect.parents.len(), 1);
        assert_eq!(connect.parents[0].name, "P-Grip");
        assert_eq!(
            connect.parents[0].node_world.transform_point3(Vec3::ZERO),
            Vec3::new(0.0, 2.0, 0.0)
        );
        assert_eq!(connect.child_root, Some(Mat4::IDENTITY));
        assert!(!connect.has_geometry);
    }

    #[test]
    fn a_document_with_a_mesh_node_has_geometry() {
        let doc = serde_json::json!({
            "asset": {"version": "2.0"},
            "meshes": [{"primitives": []}],
            "nodes": [{"name": "Root", "mesh": 0}]
        });
        let bytes = doc.to_string().into_bytes();
        let document = gltf::Gltf::from_slice(&bytes).unwrap().document;
        assert!(ConnectInfo::from_document(&document).has_geometry);
    }

    #[test]
    fn a_point_on_the_base_wins_over_the_same_name_on_a_placed_part() {
        let receiver = info(
            vec![point("P-Grip", Mat4::IDENTITY, [9.0, 0.0, 0.0], IDENTITY_Q)],
            Some(Mat4::IDENTITY),
        );
        let base = info(
            vec![
                origin_point("P-Receiver"),
                point("P-Grip", Mat4::IDENTITY, [1.0, 0.0, 0.0], IDENTITY_Q),
            ],
            None,
        );
        let fixture = Fixture::default()
            .part(
                "receiver",
                Some("ap_gun_receiver"),
                Some("r.nif"),
                Some(receiver),
            )
            .part("grip", Some("ap_gun_Grip"), Some("g.nif"), leaf());
        let result = fixture.assemble(base, &["receiver", "grip"]);
        assert_pose(part_pose(&result, 2), [1.0, 0.0, 0.0], IDENTITY_Q);
    }

    #[test]
    fn a_part_attaches_to_a_part_placed_earlier_in_the_same_pass() {
        let receiver = info(
            vec![point("P-Grip", Mat4::IDENTITY, [0.0, 2.0, 0.0], IDENTITY_Q)],
            Some(Mat4::IDENTITY),
        );
        let grip = info(
            vec![point(
                "P-Pommel",
                Mat4::IDENTITY,
                [0.0, 0.0, 3.0],
                IDENTITY_Q,
            )],
            Some(Mat4::IDENTITY),
        );
        let fixture = Fixture::default()
            .part(
                "receiver",
                Some("ap_gun_receiver"),
                Some("r.nif"),
                Some(receiver),
            )
            .part("grip", Some("ap_gun_Grip"), Some("g.nif"), Some(grip))
            .part("pommel", Some("ap_gun_Pommel"), Some("p.nif"), leaf());
        let result = fixture.assemble(receiver_base(), &["receiver", "grip", "pommel"]);
        assert_eq!(
            model_paths(&result),
            ["base.nif", "r.nif", "g.nif", "p.nif"]
        );
        assert_pose(part_pose(&result, 3), [0.0, 2.0, 3.0], IDENTITY_Q);
    }

    #[test]
    fn a_repeated_default_mod_is_placed_once_and_reported() {
        let fixture =
            Fixture::default().part("receiver", Some("ap_gun_receiver"), Some("r.nif"), leaf());
        let result = fixture.assemble(receiver_base(), &["receiver", "receiver", "receiver"]);
        assert_eq!(result.mods.len(), 1);
        assert_eq!(result.duplicates, ["receiver"]);
    }

    #[test]
    fn rejected_connect_extras_are_surfaced_with_node_and_kind() {
        let doc = serde_json::json!({
            "asset": {"version": "2.0"},
            "nodes": [
                {"name": "Receiver", "extras": {"connect_parents": {"v": 1, "points": [
                    {"name": "P", "translation": [0, 0, 0], "rotation": [0, 0, 0, 0],
                     "scale": 1.0}]}}},
                {"name": "Glow", "extras": {"light": {"v": 9}}}
            ]
        });
        let bytes = doc.to_string().into_bytes();
        let document = gltf::Gltf::from_slice(&bytes).unwrap().document;
        let connect = ConnectInfo::from_document(&document);
        assert!(connect.parents.is_empty());
        assert_eq!(connect.rejected.len(), 1);
        assert_eq!(connect.rejected[0].node_name, "Receiver");
        assert_eq!(connect.rejected[0].kind, "connect_parents");
    }
}
