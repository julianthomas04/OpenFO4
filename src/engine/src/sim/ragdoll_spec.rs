//! The v2 `.col.json` ragdoll contract: one rigid body per bone plus the joint
//! constraints between them. Pure data (glam + serde) so the renderer can read
//! the same bone names the sim builds bodies from. Parsing validates the whole
//! description; a bad file never yields a partial ragdoll.

use glam::{Quat, Vec3};

use super::collision::{ColShape, RawShape, convert_shape};
use super::water_materials::WaterBodyProps;

/// Node-local or model-space rigid transform of the sidecar contract.
#[derive(Debug, Clone, Copy)]
pub struct RigidFrame {
    pub translation: Vec3,
    pub rotation: Quat,
}

#[derive(Debug, Clone)]
pub struct RagdollBody {
    /// Bone node name; equals the skinned model's joint name.
    pub node: String,
    pub mass: f32,
    pub anchor: bool,
    pub linear_damping: f32,
    pub angular_damping: f32,
    /// Node world pose in model space at rest.
    pub rest: RigidFrame,
    /// Shapes in the node-local frame.
    pub shapes: Vec<ColShape>,
    /// Collision layer, part id and materials for the water pass; `None` for a
    /// sidecar written before the field existed (the bone then stays out of it).
    pub water: Option<WaterBodyProps>,
}

#[derive(Debug, Clone, Copy)]
pub enum JointKind {
    /// Radians. Twist is a rotation about frame X. The plane range bounds how far
    /// the child's twist axis tilts toward frame Y (a rotation about Z, positive
    /// toward +Y); the symmetric cone bounds its total deviation from the parent's
    /// twist axis, so it also limits the tilt toward Z (a rotation about Y).
    Ragdoll {
        twist: [f32; 2],
        plane: [f32; 2],
        cone: Option<f32>,
    },
    /// Hinge about frame X; radians.
    LimitedHinge { range: [f32; 2] },
}

#[derive(Debug, Clone)]
pub struct RagdollConstraint {
    pub child: usize,
    pub parent: usize,
    pub frame_child: RigidFrame,
    pub frame_parent: RigidFrame,
    pub kind: JointKind,
    pub friction: f32,
}

#[derive(Debug, Clone)]
pub struct RagdollSpec {
    pub bodies: Vec<RagdollBody>,
    pub constraints: Vec<RagdollConstraint>,
    /// Per body, the body owning its nearest `NiNode` ancestor that owns a body (`None`
    /// for a root; a model with several physics systems has one root per system): the hierarchy authored XRGD poses are relative to, which can
    /// differ from the constraint hierarchy. `None` for sidecars written before the
    /// field existed.
    pub node_parents: Option<Vec<Option<usize>>>,
}

impl RagdollSpec {
    /// The body poked by the scripted hook: the first one that simulates.
    pub fn root_body(&self) -> Option<usize> {
        self.bodies.iter().position(|body| !body.anchor)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RagdollSpecError {
    #[error("cannot read: {0}")]
    Unreadable(#[from] std::io::Error),
    #[error("unparseable: {0}")]
    Malformed(#[from] serde_json::Error),
    #[error("the sidecar has no ragdoll bodies")]
    NoBodies,
    #[error("body {body} ({node:?}): {reason}")]
    Body {
        body: usize,
        node: String,
        reason: &'static str,
    },
    #[error("constraint {constraint}: {reason}")]
    Constraint {
        constraint: usize,
        reason: &'static str,
    },
    #[error("constraints form a cycle through body {0}")]
    Cycle(usize),
    #[error("node_parent is present on some bodies only")]
    PartialNodeParents,
    #[error("body {body} has node_parent {parent}, which is not another body")]
    NodeParentOutOfRange { body: usize, parent: usize },
    #[error("node_parent forms a cycle through body {0}")]
    NodeParentCycle(usize),
}

#[derive(serde::Deserialize)]
struct RawSidecar {
    bodies: Option<Vec<RawBody>>,
    #[serde(default)]
    constraints: Vec<RawConstraint>,
}

#[derive(serde::Deserialize)]
struct RawFrame {
    t: [f32; 3],
    r: [f32; 4],
}

#[derive(serde::Deserialize)]
struct RawBody {
    node: String,
    mass: f32,
    anchor: bool,
    damping: [f32; 2],
    rest: RawFrame,
    shapes: Vec<RawShape>,
    #[serde(default)]
    water: Option<WaterBodyProps>,
    /// Absent is `None`; an explicit `null` (the root) is `Some(None)`.
    #[serde(default, deserialize_with = "present_or_null")]
    #[allow(
        clippy::option_option,
        reason = "distinguishes an absent key from an explicit null root"
    )]
    node_parent: Option<Option<usize>>,
}

#[allow(
    clippy::option_option,
    reason = "distinguishes an absent key from an explicit null root"
)]
fn present_or_null<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<usize>>, D::Error> {
    serde::Deserialize::deserialize(deserializer).map(Some)
}

#[derive(serde::Deserialize)]
struct RawConstraint {
    #[serde(rename = "type")]
    kind: String,
    child: usize,
    parent: usize,
    frame_child: RawFrame,
    frame_parent: RawFrame,
    twist: Option<[f32; 2]>,
    cone: Option<f32>,
    plane: Option<[f32; 2]>,
    hinge: Option<[f32; 2]>,
    friction: f32,
}

/// Reads and validates the ragdoll description of a loose `.col.json`.
pub fn load_ragdoll_spec(path: &str) -> Result<RagdollSpec, RagdollSpecError> {
    parse_ragdoll_spec(&std::fs::read(path)?)
}

pub fn parse_ragdoll_spec(bytes: &[u8]) -> Result<RagdollSpec, RagdollSpecError> {
    let raw: RawSidecar = serde_json::from_slice(bytes)?;
    let raw_bodies = raw
        .bodies
        .filter(|bodies| !bodies.is_empty())
        .ok_or(RagdollSpecError::NoBodies)?;
    let node_parents = convert_node_parents(&raw_bodies)?;
    let bodies = raw_bodies
        .into_iter()
        .enumerate()
        .map(|(index, body)| convert_body(index, body))
        .collect::<Result<Vec<_>, _>>()?;
    let constraints = raw
        .constraints
        .into_iter()
        .enumerate()
        .map(|(index, constraint)| convert_constraint(index, &constraint, bodies.len()))
        .collect::<Result<Vec<_>, _>>()?;
    reject_cycles(bodies.len(), &constraints)?;
    Ok(RagdollSpec {
        bodies,
        constraints,
        node_parents,
    })
}

fn convert_node_parents(
    raw_bodies: &[RawBody],
) -> Result<Option<Vec<Option<usize>>>, RagdollSpecError> {
    let declared = raw_bodies
        .iter()
        .filter(|body| body.node_parent.is_some())
        .count();
    if declared == 0 {
        return Ok(None);
    }
    if declared < raw_bodies.len() {
        return Err(RagdollSpecError::PartialNodeParents);
    }
    let parents: Vec<Option<usize>> = raw_bodies
        .iter()
        .filter_map(|body| body.node_parent)
        .collect();
    for (body, parent) in parents.iter().enumerate() {
        if let Some(parent) = *parent
            && (parent >= parents.len() || parent == body)
        {
            return Err(RagdollSpecError::NodeParentOutOfRange { body, parent });
        }
    }
    for start in 0..parents.len() {
        let mut current = start;
        for _ in 0..parents.len() {
            match parents[current] {
                Some(parent) => current = parent,
                None => break,
            }
        }
        if parents[current].is_some() {
            return Err(RagdollSpecError::NodeParentCycle(start));
        }
    }
    Ok(Some(parents))
}

fn convert_frame(raw: &RawFrame) -> Option<RigidFrame> {
    let translation = Vec3::from(raw.t);
    let rotation = Quat::from_xyzw(raw.r[0], raw.r[1], raw.r[2], raw.r[3]);
    let usable =
        translation.is_finite() && rotation.is_finite() && rotation.length_squared() > 1e-12;
    usable.then(|| RigidFrame {
        translation,
        rotation: rotation.normalize(),
    })
}

fn convert_body(index: usize, raw: RawBody) -> Result<RagdollBody, RagdollSpecError> {
    let fail = |reason| RagdollSpecError::Body {
        body: index,
        node: raw.node.clone(),
        reason,
    };
    if raw.node.is_empty() {
        return Err(fail("empty node name"));
    }
    if !raw.mass.is_finite() || raw.mass < 0.0 {
        return Err(fail("mass is not a finite non-negative number"));
    }
    if !raw.damping.iter().all(|d| d.is_finite() && *d >= 0.0) {
        return Err(fail("damping is not finite and non-negative"));
    }
    let rest = convert_frame(&raw.rest).ok_or_else(|| fail("rest pose is not finite"))?;
    let declared = raw.shapes.len();
    let shapes: Vec<ColShape> = raw.shapes.into_iter().filter_map(convert_shape).collect();
    if shapes.len() < declared {
        return Err(fail("a shape is malformed"));
    }
    if !raw.anchor && (raw.mass <= 0.0 || shapes.is_empty()) {
        return Err(fail(
            "a simulated body needs a positive mass and at least one shape",
        ));
    }
    Ok(RagdollBody {
        node: raw.node.clone(),
        mass: raw.mass,
        anchor: raw.anchor,
        linear_damping: raw.damping[0],
        angular_damping: raw.damping[1],
        rest,
        shapes,
        water: raw.water,
    })
}

fn finite_range(range: [f32; 2]) -> bool {
    range.iter().all(|v| v.is_finite()) && range[0] <= range[1]
}

fn convert_constraint(
    index: usize,
    raw: &RawConstraint,
    body_count: usize,
) -> Result<RagdollConstraint, RagdollSpecError> {
    let fail = |reason| RagdollSpecError::Constraint {
        constraint: index,
        reason,
    };
    if raw.child >= body_count || raw.parent >= body_count {
        return Err(fail("body index out of range"));
    }
    if raw.child == raw.parent {
        return Err(fail("child and parent are the same body"));
    }
    if !raw.friction.is_finite() || raw.friction < 0.0 {
        return Err(fail("friction is not finite and non-negative"));
    }
    let frame_child =
        convert_frame(&raw.frame_child).ok_or_else(|| fail("child frame is not finite"))?;
    let frame_parent =
        convert_frame(&raw.frame_parent).ok_or_else(|| fail("parent frame is not finite"))?;
    let kind = match raw.kind.as_str() {
        "ragdoll" => {
            let twist = raw
                .twist
                .filter(|r| finite_range(*r))
                .ok_or_else(|| fail("twist range missing or invalid"))?;
            let plane = raw
                .plane
                .filter(|r| finite_range(*r))
                .ok_or_else(|| fail("plane range missing or invalid"))?;
            if raw.cone.is_some_and(|cone| !cone.is_finite() || cone < 0.0) {
                return Err(fail("cone is not finite and non-negative"));
            }
            JointKind::Ragdoll {
                twist,
                plane,
                cone: raw.cone,
            }
        }
        "limited_hinge" => {
            let range = raw
                .hinge
                .filter(|r| finite_range(*r))
                .ok_or_else(|| fail("hinge range missing or invalid"))?;
            JointKind::LimitedHinge { range }
        }
        _ => return Err(fail("unknown constraint type")),
    };
    Ok(RagdollConstraint {
        child: raw.child,
        parent: raw.parent,
        frame_child,
        frame_parent,
        kind,
        friction: raw.friction,
    })
}

/// Follows the child -> parent edges of every body; a walk longer than the body
/// count can only be a cycle.
fn reject_cycles(
    body_count: usize,
    constraints: &[RagdollConstraint],
) -> Result<(), RagdollSpecError> {
    let mut parents: Vec<Vec<usize>> = vec![Vec::new(); body_count];
    for constraint in constraints {
        parents[constraint.child].push(constraint.parent);
    }
    // 0 = unvisited, 1 = on the current path, 2 = proven acyclic.
    let mut state = vec![0u8; body_count];
    for start in 0..body_count {
        let mut stack = vec![(start, 0usize)];
        while let Some(&mut (body, ref mut next)) = stack.last_mut() {
            if *next == 0 {
                match state[body] {
                    2 => {
                        stack.pop();
                        continue;
                    }
                    1 => return Err(RagdollSpecError::Cycle(body)),
                    _ => state[body] = 1,
                }
            }
            if let Some(&parent) = parents[body].get(*next) {
                *next += 1;
                if state[parent] == 1 {
                    return Err(RagdollSpecError::Cycle(parent));
                }
                if state[parent] == 0 {
                    stack.push((parent, 0));
                }
            } else {
                state[body] = 2;
                stack.pop();
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    //! Fixtures follow the v2 sidecar contract.

    use proptest::prelude::*;
    use serde_json::{Value, json};

    use super::*;

    fn sphere() -> Value {
        json!({ "type": "sphere", "center": [0.0, 0.0, 0.0], "radius": 0.5 })
    }

    fn frame(x: f32, y: f32, z: f32) -> Value {
        json!({ "t": [x, y, z], "r": [0.0, 0.0, 0.0, 1.0] })
    }

    fn body(node: &str) -> Value {
        json!({
            "node": node,
            "mass": 2.0,
            "anchor": false,
            "damping": [0.1, 0.05],
            "rest": frame(0.0, 1.0, 0.0),
            "shapes": [sphere()],
        })
    }

    fn with(mut base: Value, key: &str, value: Value) -> Value {
        base[key] = value;
        base
    }

    fn hinge(child: usize, parent: usize) -> Value {
        json!({
            "type": "limited_hinge",
            "child": child,
            "parent": parent,
            "frame_child": frame(0.0, 0.0, 0.0),
            "frame_parent": frame(0.0, 1.0, 0.0),
            "hinge": [-1.0, 0.5],
            "friction": 0.01,
        })
    }

    fn ragdoll_joint(child: usize, parent: usize) -> Value {
        json!({
            "type": "ragdoll",
            "child": child,
            "parent": parent,
            "frame_child": frame(0.0, 0.0, 0.0),
            "frame_parent": frame(0.0, 1.0, 0.0),
            "twist": [-0.5, 0.5],
            "plane": [-0.3, 0.2],
            "cone": 0.7,
            "friction": 10.0,
        })
    }

    fn parse(sidecar: &Value) -> Result<RagdollSpec, RagdollSpecError> {
        parse_ragdoll_spec(&serde_json::to_vec(sidecar).unwrap())
    }

    fn two_body_sidecar() -> Value {
        json!({ "version": 2, "bodies": [body("Root"), body("Child")], "constraints": [hinge(1, 0)] })
    }

    #[test]
    fn a_body_carries_its_water_object_when_the_sidecar_has_one() {
        let water = json!({ "layer": 10, "part": 3, "materials": [[2086, 2]] });
        let spec = parse(&json!({
            "bodies": [with(body("Root"), "water", water), body("Child")],
            "constraints": [hinge(1, 0)],
        }))
        .unwrap();
        let props = spec.bodies[0].water.as_ref().unwrap();
        assert_eq!((props.layer, props.part), (10, 3));
        assert_eq!(props.materials, [(2086, 2)]);
        assert!(spec.bodies[1].water.is_none());
    }

    #[test]
    fn two_body_ragdoll_parses_with_bodies_and_constraints() {
        let spec = parse(&two_body_sidecar()).unwrap();
        assert_eq!(spec.bodies.len(), 2);
        assert_eq!(spec.bodies[1].node, "Child");
        assert_eq!(spec.bodies[0].mass, 2.0);
        assert_eq!(spec.bodies[0].linear_damping, 0.1);
        assert_eq!(spec.bodies[0].angular_damping, 0.05);
        assert_eq!(spec.bodies[0].rest.translation, Vec3::new(0.0, 1.0, 0.0));
        assert_eq!(spec.bodies[0].shapes.len(), 1);
        assert_eq!(spec.constraints.len(), 1);
        assert_eq!(
            (spec.constraints[0].child, spec.constraints[0].parent),
            (1, 0)
        );
        assert!(matches!(
            spec.constraints[0].kind,
            JointKind::LimitedHinge { range } if range == [-1.0, 0.5]
        ));
    }

    #[test]
    fn ragdoll_constraint_carries_twist_plane_cone_and_friction() {
        let sidecar =
            json!({ "bodies": [body("A"), body("B")], "constraints": [ragdoll_joint(1, 0)] });
        let spec = parse(&sidecar).unwrap();
        assert!(matches!(
            spec.constraints[0].kind,
            JointKind::Ragdoll { twist, plane, cone: Some(cone) }
                if twist == [-0.5, 0.5] && plane == [-0.3, 0.2] && cone == 0.7
        ));
        assert_eq!(spec.constraints[0].friction, 10.0);
    }

    #[test]
    fn a_null_cone_means_unbounded() {
        let joint = with(ragdoll_joint(1, 0), "cone", Value::Null);
        let spec =
            parse(&json!({ "bodies": [body("A"), body("B")], "constraints": [joint] })).unwrap();
        assert!(matches!(
            spec.constraints[0].kind,
            JointKind::Ragdoll { cone: None, .. }
        ));
    }

    #[test]
    fn frame_quaternions_are_normalised() {
        let doubled = with(
            body("A"),
            "rest",
            json!({ "t": [0.0, 0.0, 0.0], "r": [0.0, 0.0, 0.0, 2.0] }),
        );
        let spec = parse(&json!({ "bodies": [doubled], "constraints": [] }));
        assert_eq!(spec.unwrap().bodies[0].rest.rotation, Quat::IDENTITY);
    }

    #[test]
    fn root_body_is_the_first_body_that_simulates() {
        let anchor = with(
            with(body("Anchor"), "anchor", json!(true)),
            "mass",
            json!(0.0),
        );
        let spec = parse(&json!({ "bodies": [anchor, body("Free")], "constraints": [] })).unwrap();
        assert_eq!(spec.root_body(), Some(1));
    }

    #[test]
    fn an_anchor_needs_neither_mass_nor_shapes() {
        let anchor = json!({
            "node": "Hook", "mass": 0.0, "anchor": true, "damping": [0.0, 0.0],
            "rest": frame(0.0, 0.0, 0.0), "shapes": [],
        });
        let spec = parse(&json!({ "bodies": [anchor], "constraints": [] })).unwrap();
        assert!(spec.bodies[0].anchor);
        assert_eq!(spec.root_body(), None);
    }

    #[test]
    fn absent_node_parent_yields_no_hierarchy() {
        assert!(parse(&two_body_sidecar()).unwrap().node_parents.is_none());
    }

    #[test]
    fn explicit_null_node_parent_marks_a_root_instead_of_being_absent() {
        let sidecar = json!({
            "bodies": [with(body("Root"), "node_parent", Value::Null), with(body("Child"), "node_parent", json!(0))],
            "constraints": [],
        });
        let spec = parse(&sidecar).unwrap();
        assert_eq!(spec.node_parents, Some(vec![None, Some(0)]));
    }

    #[test]
    fn several_null_roots_are_allowed() {
        let bodies: Vec<Value> = ["A", "B"]
            .iter()
            .map(|n| with(body(n), "node_parent", Value::Null))
            .collect();
        let spec = parse(&json!({ "bodies": bodies, "constraints": [] })).unwrap();
        assert_eq!(spec.node_parents, Some(vec![None, None]));
    }

    #[test]
    fn node_parent_on_only_some_bodies_is_rejected() {
        let sidecar = json!({
            "bodies": [with(body("Root"), "node_parent", Value::Null), body("Child")],
            "constraints": [],
        });
        assert!(matches!(
            parse(&sidecar),
            Err(RagdollSpecError::PartialNodeParents)
        ));
    }

    #[test]
    fn node_parent_out_of_range_or_self_is_rejected() {
        for parent in [2, 0] {
            let sidecar = json!({
                "bodies": [with(body("A"), "node_parent", json!(parent)), with(body("B"), "node_parent", Value::Null)],
                "constraints": [],
            });
            assert!(
                matches!(
                    parse(&sidecar),
                    Err(RagdollSpecError::NodeParentOutOfRange { body: 0, .. })
                ),
                "parent {parent}"
            );
        }
    }

    #[test]
    fn node_parent_cycle_is_rejected() {
        let sidecar = json!({
            "bodies": [with(body("A"), "node_parent", json!(1)), with(body("B"), "node_parent", json!(0))],
            "constraints": [],
        });
        assert!(matches!(
            parse(&sidecar),
            Err(RagdollSpecError::NodeParentCycle(_))
        ));
    }

    #[test]
    fn a_legacy_shapes_only_sidecar_has_no_ragdoll_bodies() {
        let legacy = json!({ "shapes": [sphere()], "dynamic": true, "mass": 3.0 });
        assert!(matches!(parse(&legacy), Err(RagdollSpecError::NoBodies)));
    }

    #[test]
    fn empty_or_null_bodies_are_rejected() {
        for bodies in [json!([]), Value::Null] {
            let sidecar = json!({ "bodies": bodies });
            assert!(matches!(parse(&sidecar), Err(RagdollSpecError::NoBodies)));
        }
    }

    #[test]
    fn malformed_json_is_a_malformed_error() {
        assert!(matches!(
            parse_ragdoll_spec(b"{ not json"),
            Err(RagdollSpecError::Malformed(_))
        ));
    }

    #[test]
    fn invalid_bodies_are_rejected_with_their_index_and_node() {
        let cases = [
            ("empty node name", with(body("x"), "node", json!(""))),
            ("negative mass", with(body("x"), "mass", json!(-1.0))),
            (
                "negative damping",
                with(body("x"), "damping", json!([-0.1, 0.0])),
            ),
            (
                "zero rest rotation",
                with(
                    body("x"),
                    "rest",
                    json!({ "t": [0, 0, 0], "r": [0, 0, 0, 0] }),
                ),
            ),
            (
                "simulated without shapes",
                with(body("x"), "shapes", json!([])),
            ),
            (
                "simulated without mass",
                with(body("x"), "mass", json!(0.0)),
            ),
            (
                "malformed shape",
                with(body("x"), "shapes", json!([{ "type": "sphere" }])),
            ),
        ];
        for (name, bad) in cases {
            let sidecar = json!({ "bodies": [body("Ok"), bad], "constraints": [] });
            assert!(
                matches!(parse(&sidecar), Err(RagdollSpecError::Body { body: 1, .. })),
                "{name}"
            );
        }
    }

    #[test]
    fn invalid_constraints_are_rejected_with_their_index() {
        let cases = [
            ("child out of range", with(hinge(1, 0), "child", json!(2))),
            ("parent out of range", with(hinge(1, 0), "parent", json!(9))),
            ("self constraint", with(hinge(1, 0), "parent", json!(1))),
            (
                "negative friction",
                with(hinge(1, 0), "friction", json!(-1.0)),
            ),
            (
                "unknown type",
                with(hinge(1, 0), "type", json!("prismatic")),
            ),
            (
                "missing hinge range",
                with(hinge(1, 0), "hinge", Value::Null),
            ),
            (
                "reversed hinge range",
                with(hinge(1, 0), "hinge", json!([1.0, -1.0])),
            ),
            (
                "missing twist",
                with(ragdoll_joint(1, 0), "twist", Value::Null),
            ),
            (
                "missing plane",
                with(ragdoll_joint(1, 0), "plane", Value::Null),
            ),
            (
                "negative cone",
                with(ragdoll_joint(1, 0), "cone", json!(-0.1)),
            ),
            (
                "zero child frame",
                with(
                    hinge(1, 0),
                    "frame_child",
                    json!({ "t": [0, 0, 0], "r": [0, 0, 0, 0] }),
                ),
            ),
        ];
        for (name, bad) in cases {
            let sidecar =
                json!({ "bodies": [body("A"), body("B")], "constraints": [hinge(1, 0), bad] });
            assert!(
                matches!(
                    parse(&sidecar),
                    Err(RagdollSpecError::Constraint { constraint: 1, .. })
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn constraint_cycles_are_rejected() {
        let two =
            json!({ "bodies": [body("A"), body("B")], "constraints": [hinge(1, 0), hinge(0, 1)] });
        assert!(matches!(parse(&two), Err(RagdollSpecError::Cycle(_))));
        let three = json!({
            "bodies": [body("A"), body("B"), body("C")],
            "constraints": [hinge(1, 0), hinge(2, 1), hinge(0, 2)],
        });
        assert!(matches!(parse(&three), Err(RagdollSpecError::Cycle(_))));
    }

    #[test]
    fn a_body_with_two_parents_forming_a_diamond_is_not_a_cycle() {
        let diamond = json!({
            "bodies": [body("A"), body("B"), body("C"), body("D")],
            "constraints": [hinge(1, 0), hinge(2, 0), hinge(3, 1), hinge(3, 2)],
        });
        assert_eq!(parse(&diamond).unwrap().constraints.len(), 4);
    }

    #[test]
    fn load_reads_a_sidecar_from_disk_and_reports_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("skeleton.col.json");
        std::fs::write(&path, serde_json::to_vec(&two_body_sidecar()).unwrap()).unwrap();
        assert_eq!(
            load_ragdoll_spec(path.to_str().unwrap())
                .unwrap()
                .bodies
                .len(),
            2
        );
        let missing = dir.path().join("absent.col.json");
        assert!(matches!(
            load_ragdoll_spec(missing.to_str().unwrap()),
            Err(RagdollSpecError::Unreadable(_))
        ));
    }

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..256)) {
            let _ = parse_ragdoll_spec(&bytes);
        }

        #[test]
        fn arbitrary_json_values_in_a_body_never_panic(mass in any::<f32>(), d0 in any::<f32>(), t in any::<f32>()) {
            let sidecar = json!({
                "bodies": [{
                    "node": "A", "mass": mass, "anchor": false, "damping": [d0, 0.0],
                    "rest": { "t": [t, 0.0, 0.0], "r": [0.0, 0.0, 0.0, 1.0] },
                    "shapes": [sphere()],
                }],
                "constraints": [],
            });
            let _ = parse(&sidecar);
        }
    }
}
