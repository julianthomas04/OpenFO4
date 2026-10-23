use std::collections::{HashMap, HashSet};

use glam::{Mat4, Quat, Vec3};
use gltf::animation::util::ReadOutputs;
use serde::Deserialize;

use super::clip::{Clip, ClipError, Cycle, Interpolation, Keys, Marker, NodeTrack};
use super::skeleton::{JointIndex, Skeleton, SkeletonError, Trs};

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error(transparent)]
    Gltf(#[from] gltf::Error),
    #[error("node {0} has no name")]
    UnnamedNode(usize),
    #[error("node {0} ({1:?}) has more than one parent")]
    MultipleParents(usize, String),
    #[error("node hierarchy contains a cycle")]
    HierarchyCycle,
    #[error(transparent)]
    Skeleton(#[from] SkeletonError),
    #[error("malformed extras on {owner}: {source}")]
    BadExtras {
        owner: String,
        source: serde_json::Error,
    },
    #[error(
        "animation {animation:?} channel targets {property} on node {node:?}, which is unsupported"
    )]
    UnsupportedTarget {
        animation: String,
        node: String,
        property: &'static str,
    },
    #[error("animation {animation:?} uses CUBICSPLINE interpolation, which is unsupported")]
    CubicSpline { animation: String },
    #[error("animation {animation:?} has a channel without input or output data")]
    MissingChannelData { animation: String },
    #[error("animation {animation:?} animates {property} of node {node:?} more than once")]
    DuplicateChannel {
        animation: String,
        node: String,
        property: &'static str,
    },
    #[error("animation {animation:?} has a rotation key that is not a finite non-zero quaternion")]
    DegenerateRotation { animation: String },
    #[error("more than one animation is named {0:?}")]
    DuplicateClipName(String),
    #[error("animation {animation:?}: {source}")]
    Clip {
        animation: String,
        source: ClipError,
    },
    #[error("skin {skin} joint node {node} has no name")]
    UnnamedSkinJoint { skin: usize, node: usize },
    #[error("skin {skin} has {joints} joints but {matrices} inverse bind matrices")]
    InverseBindCount {
        skin: usize,
        joints: usize,
        matrices: usize,
    },
}

#[derive(Clone, Debug)]
pub struct SkinData {
    pub joint_names: Vec<String>,
    pub inverse_bind: Vec<Mat4>,
}

#[derive(Debug)]
pub struct AnimationAsset {
    pub skeleton: Skeleton,
    /// Joints whose node carries `extras.anim == true`.
    pub anim_joints: Vec<JointIndex>,
    pub clips: Vec<Clip>,
    pub skins: Vec<SkinData>,
    /// Recoverable oddities met while importing; the importer has no model
    /// name, so the caller reports them.
    pub warnings: Vec<ImportWarning>,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportWarning {
    #[error("clip {clip:?} track for node {node:?} was skipped: {reason}")]
    TrackSkipped {
        clip: String,
        node: String,
        reason: ClipError,
    },
    #[error(
        "clip {clip:?} node {node:?}: {count} keyframes shared a time with a later one and were collapsed"
    )]
    DuplicateKeysCollapsed {
        clip: String,
        node: String,
        count: usize,
    },
}

#[derive(Deserialize, Default)]
struct NodeExtras {
    #[serde(default)]
    anim: bool,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum CycleName {
    Loop,
    Clamp,
}

#[derive(Deserialize)]
struct TextKey {
    t: f32,
    key: String,
}

#[derive(Deserialize)]
struct Annotation {
    t: f32,
    text: String,
}

#[derive(Deserialize, Default)]
struct ClipExtras {
    start: Option<f32>,
    stop: Option<f32>,
    cycle: Option<CycleName>,
    #[serde(default)]
    text_keys: Vec<TextKey>,
    #[serde(default)]
    annotations: Vec<Annotation>,
}

fn parse_extras<T: Default + for<'de> Deserialize<'de>>(
    extras: &gltf::json::Extras,
    owner: impl FnOnce() -> String,
) -> Result<T, ImportError> {
    match extras {
        None => Ok(T::default()),
        Some(raw) => serde_json::from_str(raw.get()).map_err(|source| ImportError::BadExtras {
            owner: owner(),
            source,
        }),
    }
}

impl AnimationAsset {
    pub fn load(path: &str) -> Result<Self, ImportError> {
        let gltf = gltf::Gltf::open(path)?;
        let base = std::path::Path::new(path).parent();
        let buffers = gltf::import_buffers(&gltf.document, base, gltf.blob)?;
        Self::from_document(&gltf.document, &buffers)
    }

    pub fn from_document(
        document: &gltf::Document,
        buffers: &[gltf::buffer::Data],
    ) -> Result<Self, ImportError> {
        let nodes: Vec<gltf::Node> = document.nodes().collect();
        let (order, parent_of_node) = joint_order(document, &nodes)?;
        let joint_of_node: HashMap<usize, JointIndex> = order
            .iter()
            .enumerate()
            .map(|(joint, &node)| (node, joint as JointIndex))
            .collect();

        let mut names = Vec::with_capacity(order.len());
        let mut parents = Vec::with_capacity(order.len());
        let mut rest = Vec::with_capacity(order.len());
        let mut anim_joints = Vec::new();
        for (joint, &node_index) in order.iter().enumerate() {
            let node = &nodes[node_index];
            names.push(node_name(node)?);
            parents.push(parent_of_node.get(&node_index).map(|p| joint_of_node[p]));
            rest.push(rest_transform(node));
            let extras: NodeExtras = parse_extras(node.extras(), || format!("node {node_index}"))?;
            if extras.anim {
                anim_joints.push(joint as JointIndex);
            }
        }
        let skeleton = Skeleton::new(names, parents, rest)?;

        let mut warnings = Vec::new();
        let clips = import_clips(document, buffers, &joint_of_node, &skeleton, &mut warnings)?;
        let skins = import_skins(document, buffers)?;
        Ok(Self {
            skeleton,
            anim_joints,
            clips,
            skins,
            warnings,
        })
    }
}

fn node_name(node: &gltf::Node) -> Result<String, ImportError> {
    node.name()
        .map(str::to_owned)
        .ok_or(ImportError::UnnamedNode(node.index()))
}

fn rest_transform(node: &gltf::Node) -> Trs {
    let (translation, rotation, scale) = node.transform().decomposed();
    Trs {
        translation: Vec3::from_array(translation),
        rotation: Quat::from_array(rotation).normalize(),
        scale: Vec3::from_array(scale),
    }
}

/// Node indices in parent-before-child order (scene roots first, then roots no
/// scene references, each subtree depth first) plus each node's parent.
fn joint_order(
    document: &gltf::Document,
    nodes: &[gltf::Node],
) -> Result<(Vec<usize>, HashMap<usize, usize>), ImportError> {
    let mut parent_of_node = HashMap::new();
    for node in nodes {
        for child in node.children() {
            if parent_of_node.insert(child.index(), node.index()).is_some() {
                return Err(ImportError::MultipleParents(
                    child.index(),
                    node_name(&child)?,
                ));
            }
        }
    }

    let scene_roots = document
        .scenes()
        .flat_map(|scene| scene.nodes().map(|n| n.index()).collect::<Vec<_>>());
    let orphan_roots = nodes.iter().map(gltf::Node::index);
    let mut visited = HashSet::new();
    let mut order = Vec::with_capacity(nodes.len());
    let mut stack = Vec::new();
    for root in scene_roots.chain(orphan_roots) {
        if parent_of_node.contains_key(&root) || !visited.insert(root) {
            continue;
        }
        stack.push(root);
        while let Some(index) = stack.pop() {
            order.push(index);
            let children: Vec<usize> = nodes[index].children().map(|c| c.index()).collect();
            for child in children.into_iter().rev() {
                if !visited.insert(child) {
                    return Err(ImportError::HierarchyCycle);
                }
                stack.push(child);
            }
        }
    }
    if order.len() != nodes.len() {
        return Err(ImportError::HierarchyCycle);
    }
    Ok((order, parent_of_node))
}

fn import_clips(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    joint_of_node: &HashMap<usize, JointIndex>,
    skeleton: &Skeleton,
    warnings: &mut Vec<ImportWarning>,
) -> Result<Vec<Clip>, ImportError> {
    let mut clips: Vec<Clip> = Vec::with_capacity(document.animations().len());
    for animation in document.animations() {
        let name = animation
            .name()
            .map_or_else(|| format!("animation_{}", animation.index()), str::to_owned);
        if clips.iter().any(|c| c.name() == name) {
            return Err(ImportError::DuplicateClipName(name));
        }
        clips.push(import_clip(
            &animation,
            name,
            buffers,
            joint_of_node,
            skeleton,
            warnings,
        )?);
    }
    Ok(clips)
}

#[derive(Default)]
struct TrackBuilder {
    translation: Option<Keys<Vec3>>,
    rotation: Option<Keys<Quat>>,
    scale: Option<Keys<Vec3>>,
}

fn import_clip(
    animation: &gltf::Animation,
    name: String,
    buffers: &[gltf::buffer::Data],
    joint_of_node: &HashMap<usize, JointIndex>,
    skeleton: &Skeleton,
    warnings: &mut Vec<ImportWarning>,
) -> Result<Clip, ImportError> {
    let extras: ClipExtras = parse_extras(animation.extras(), || format!("animation {name:?}"))?;
    let start = extras.start.unwrap_or(0.0);
    let wrap = |source| ImportError::Clip {
        animation: name.clone(),
        source,
    };
    let mut failed_nodes: HashSet<String> = HashSet::new();
    let mut collapsed: HashMap<String, usize> = HashMap::new();

    let mut builders: Vec<(String, TrackBuilder)> = Vec::new();
    let mut builder_of_joint: HashMap<JointIndex, usize> = HashMap::new();
    for channel in animation.channels() {
        let joint = joint_of_node[&channel.target().node().index()];
        let node = skeleton.names()[joint as usize].clone();
        let sampler = channel.sampler();
        let interpolation = match sampler.interpolation() {
            gltf::animation::Interpolation::Step => Interpolation::Step,
            gltf::animation::Interpolation::Linear => Interpolation::Linear,
            gltf::animation::Interpolation::CubicSpline => {
                return Err(ImportError::CubicSpline { animation: name });
            }
        };
        let reader = channel.reader(|buffer| Some(&buffers[buffer.index()].0[..]));
        let missing = || ImportError::MissingChannelData {
            animation: name.clone(),
        };
        let times: Vec<f32> = reader
            .read_inputs()
            .ok_or_else(missing)?
            .map(|t| t - start)
            .collect();
        let outputs = reader.read_outputs().ok_or_else(missing)?;

        let slot = *builder_of_joint.entry(joint).or_insert_with(|| {
            builders.push((node.clone(), TrackBuilder::default()));
            builders.len() - 1
        });
        let builder = &mut builders[slot].1;
        let duplicate = |property| ImportError::DuplicateChannel {
            animation: name.clone(),
            node: node.clone(),
            property,
        };
        match outputs {
            ReadOutputs::Translations(values) => {
                let values = values.map(Vec3::from_array).collect();
                let keys = match Keys::new(times, values, interpolation) {
                    Ok(keys) => keys,
                    Err(reason) => {
                        skip_track(&mut failed_nodes, warnings, &name, &node, reason);
                        continue;
                    }
                };
                *collapsed.entry(node.clone()).or_default() += keys.collapsed_duplicates();
                if builder.translation.replace(keys).is_some() {
                    return Err(duplicate("translation"));
                }
            }
            ReadOutputs::Scales(values) => {
                let values = values.map(Vec3::from_array).collect();
                let keys = match Keys::new(times, values, interpolation) {
                    Ok(keys) => keys,
                    Err(reason) => {
                        skip_track(&mut failed_nodes, warnings, &name, &node, reason);
                        continue;
                    }
                };
                *collapsed.entry(node.clone()).or_default() += keys.collapsed_duplicates();
                if builder.scale.replace(keys).is_some() {
                    return Err(duplicate("scale"));
                }
            }
            ReadOutputs::Rotations(values) => {
                let values = values
                    .into_f32()
                    .map(|q| {
                        normalized(Quat::from_array(q)).ok_or_else(|| {
                            ImportError::DegenerateRotation {
                                animation: name.clone(),
                            }
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let keys = match Keys::new(times, values, interpolation) {
                    Ok(keys) => keys,
                    Err(reason) => {
                        skip_track(&mut failed_nodes, warnings, &name, &node, reason);
                        continue;
                    }
                };
                *collapsed.entry(node.clone()).or_default() += keys.collapsed_duplicates();
                if builder.rotation.replace(keys).is_some() {
                    return Err(duplicate("rotation"));
                }
            }
            ReadOutputs::MorphTargetWeights(_) => {
                return Err(ImportError::UnsupportedTarget {
                    animation: name,
                    node,
                    property: "morph target weights",
                });
            }
        }
    }

    warnings.extend(
        collapsed
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .map(|(node, count)| ImportWarning::DuplicateKeysCollapsed {
                clip: name.clone(),
                node,
                count,
            }),
    );
    let tracks: Vec<NodeTrack> = builders
        .into_iter()
        .filter(|(node_name, _)| !failed_nodes.contains(node_name))
        .map(|(node_name, b)| NodeTrack {
            node_name,
            translation: b.translation,
            rotation: b.rotation,
            scale: b.scale,
        })
        .collect();

    let latest_key = tracks
        .iter()
        .flat_map(|t| {
            [
                t.translation.as_ref().map(Keys::last_time),
                t.rotation.as_ref().map(Keys::last_time),
                t.scale.as_ref().map(Keys::last_time),
            ]
        })
        .flatten()
        .fold(0.0_f32, f32::max);
    let duration = extras.stop.map_or(latest_key, |stop| stop - start);

    let markers = extras
        .text_keys
        .into_iter()
        .map(|k| Marker {
            time: k.t - start,
            text: k.key,
        })
        .chain(extras.annotations.into_iter().map(|a| Marker {
            time: a.t - start,
            text: a.text,
        }))
        .collect();
    let cycle = match extras.cycle {
        Some(CycleName::Loop) => Cycle::Loop,
        Some(CycleName::Clamp) | None => Cycle::Clamp,
    };

    Clip::new(name.clone(), duration, cycle, tracks, markers).map_err(wrap)
}

fn skip_track(
    failed_nodes: &mut HashSet<String>,
    warnings: &mut Vec<ImportWarning>,
    clip: &str,
    node: &str,
    reason: ClipError,
) {
    if failed_nodes.insert(node.to_owned()) {
        warnings.push(ImportWarning::TrackSkipped {
            clip: clip.to_owned(),
            node: node.to_owned(),
            reason,
        });
    }
}

fn normalized(q: Quat) -> Option<Quat> {
    let length_squared = q.length_squared();
    (length_squared.is_finite() && length_squared > f32::MIN_POSITIVE)
        .then(|| q / length_squared.sqrt())
}

/// One inverse bind matrix per skin joint; identity when the file stores none.
pub(crate) fn read_inverse_binds(
    skin: &gltf::Skin,
    buffers: &[gltf::buffer::Data],
) -> Result<Vec<Mat4>, ImportError> {
    let joint_count = skin.joints().count();
    let reader = skin.reader(|buffer| Some(&buffers[buffer.index()].0[..]));
    let inverse_binds: Vec<Mat4> = match reader.read_inverse_bind_matrices() {
        Some(matrices) => matrices.map(|m| Mat4::from_cols_array_2d(&m)).collect(),
        None => vec![Mat4::IDENTITY; joint_count],
    };
    if inverse_binds.len() != joint_count {
        return Err(ImportError::InverseBindCount {
            skin: skin.index(),
            joints: joint_count,
            matrices: inverse_binds.len(),
        });
    }
    Ok(inverse_binds)
}

fn import_skins(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> Result<Vec<SkinData>, ImportError> {
    document
        .skins()
        .map(|skin| {
            let joint_names = skin
                .joints()
                .map(|joint| {
                    joint
                        .name()
                        .map(str::to_owned)
                        .ok_or(ImportError::UnnamedSkinJoint {
                            skin: skin.index(),
                            node: joint.index(),
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let inverse_bind = read_inverse_binds(&skin, buffers)?;
            Ok(SkinData {
                joint_names,
                inverse_bind,
            })
        })
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    //! Fixtures are tiny GLB files built in memory per the glTF 2.0 spec
    //! (12-byte header, JSON chunk, BIN chunk, 4-byte aligned chunks).

    use super::*;

    /// Accessors of the shared binary buffer: (element type, floats per element, data).
    const ACCESSORS: [(&str, usize, &[f32]); 8] = [
        ("SCALAR", 1, &[1.0, 2.0]),
        ("SCALAR", 1, &[1.0, 1.0, 2.0]),
        ("VEC3", 3, &[0.0, 0.0, 0.0, 2.0, 4.0, 6.0]),
        ("VEC3", 3, &[0.0, 0.0, 0.0, 9.0, 9.0, 9.0, 2.0, 4.0, 6.0]),
        ("VEC4", 4, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0]),
        ("VEC4", 4, &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0]),
        (
            "MAT4",
            16,
            &[
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, -2.0, -3.0, 1.0,
            ],
        ),
        ("SCALAR", 1, &[2.0, 1.0]),
    ];
    const TIMES: usize = 0;
    const TIMES_WITH_DUPLICATE: usize = 1;
    const TRANSLATIONS: usize = 2;
    const TRANSLATIONS_WITH_DUPLICATE: usize = 3;
    const ROTATIONS: usize = 4;
    const ROTATIONS_WITH_ZERO: usize = 5;
    const INVERSE_BINDS: usize = 6;
    const TIMES_DECREASING: usize = 7;

    fn glb(nodes: &str, rest: &str) -> Vec<u8> {
        let mut bin = Vec::new();
        let mut views = Vec::new();
        let mut accessors = Vec::new();
        for (index, (kind, width, data)) in ACCESSORS.iter().enumerate() {
            views.push(format!(
                r#"{{"buffer":0,"byteOffset":{},"byteLength":{}}}"#,
                bin.len(),
                data.len() * 4
            ));
            accessors.push(format!(
                r#"{{"bufferView":{index},"componentType":5126,"count":{},"type":"{kind}"}}"#,
                data.len() / width
            ));
            bin.extend(data.iter().flat_map(|f| f.to_le_bytes()));
        }
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"buffers":[{{"byteLength":{}}}],"bufferViews":[{}],"accessors":[{}],"nodes":[{nodes}]{rest}}}"#,
            bin.len(),
            views.join(","),
            accessors.join(",")
        );
        let mut json = json.into_bytes();
        json.resize(json.len().next_multiple_of(4), b' ');
        let total = 12 + 8 + json.len() + 8 + bin.len();
        let mut out = Vec::with_capacity(total);
        out.extend(b"glTF");
        out.extend(2_u32.to_le_bytes());
        out.extend((total as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(b"JSON");
        out.extend(&json);
        out.extend((bin.len() as u32).to_le_bytes());
        out.extend(b"BIN\0");
        out.extend(&bin);
        out
    }

    fn import(nodes: &str, rest: &str) -> Result<AnimationAsset, ImportError> {
        let gltf = gltf::Gltf::from_slice(&glb(nodes, rest)).expect("fixture is valid glTF");
        let buffers = gltf::import_buffers(&gltf.document, None, gltf.blob).unwrap();
        AnimationAsset::from_document(&gltf.document, &buffers)
    }

    fn channel(sampler: usize, node: usize, path: &str) -> String {
        format!(r#"{{"sampler":{sampler},"target":{{"node":{node},"path":"{path}"}}}}"#)
    }

    fn sampler(input: usize, output: usize, interpolation: &str) -> String {
        format!(r#"{{"input":{input},"output":{output},"interpolation":"{interpolation}"}}"#)
    }

    fn animation(name: &str, extras: &str, channels: &[String], samplers: &[String]) -> String {
        format!(
            r#"{{"name":"{name}","extras":{extras},"channels":[{}],"samplers":[{}]}}"#,
            channels.join(","),
            samplers.join(",")
        )
    }

    fn animations(list: &[String]) -> String {
        format!(r#","animations":[{}]"#, list.join(","))
    }

    const TWO_NODES: &str = r#"{"name":"root","children":[1]},{"name":"leaf"}"#;

    fn linear_translation_clip(name: &str, extras: &str) -> String {
        animation(
            name,
            extras,
            &[channel(0, 1, "translation")],
            &[sampler(TIMES, TRANSLATIONS, "LINEAR")],
        )
    }

    #[test]
    fn joints_are_ordered_parent_before_child_even_when_the_file_lists_the_child_first() {
        let asset = import(
            r#"{"name":"leaf"},{"name":"root","children":[0]}"#,
            r#","scenes":[{"nodes":[1]}]"#,
        )
        .unwrap();
        assert_eq!(asset.skeleton.names(), ["root", "leaf"]);
        assert_eq!(asset.skeleton.parents(), &[None, Some(0)]);
    }

    #[test]
    fn rest_pose_comes_from_the_node_transform() {
        let asset = import(
            r#"{"name":"n","translation":[1,2,3],"rotation":[0,0,0,2],"scale":[2,2,2]}"#,
            "",
        )
        .unwrap();
        let rest = asset.skeleton.rest_pose()[0];
        assert_eq!(rest.translation, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(rest.scale, Vec3::splat(2.0));
        assert!(
            (rest.rotation.length() - 1.0).abs() < 1e-6,
            "rest rotation is normalised"
        );
    }

    #[test]
    fn an_unnamed_node_is_rejected() {
        let err = import(r#"{"name":"a"},{}"#, "").unwrap_err();
        assert!(matches!(err, ImportError::UnnamedNode(1)));
    }

    #[test]
    fn a_node_with_two_parents_is_rejected() {
        let err = import(
            r#"{"name":"a","children":[2]},{"name":"b","children":[2]},{"name":"c"}"#,
            "",
        )
        .unwrap_err();
        assert!(matches!(err, ImportError::MultipleParents(2, name) if name == "c"));
    }

    #[test]
    fn a_hierarchy_cycle_is_rejected() {
        let err = import(
            r#"{"name":"a","children":[1]},{"name":"b","children":[0]}"#,
            "",
        )
        .unwrap_err();
        assert!(matches!(err, ImportError::HierarchyCycle));
    }

    #[test]
    fn duplicate_node_names_are_rejected_by_the_skeleton() {
        let err = import(r#"{"name":"a"},{"name":"a"}"#, "").unwrap_err();
        assert!(matches!(
            err,
            ImportError::Skeleton(SkeletonError::DuplicateName(_))
        ));
    }

    #[test]
    fn only_nodes_with_the_anim_extra_become_anim_joints() {
        let asset = import(
            r#"{"name":"a","extras":{"anim":true}},{"name":"b","extras":{"anim":false}},{"name":"c"}"#,
            "",
        )
        .unwrap();
        assert_eq!(asset.anim_joints, [0]);
    }

    #[test]
    fn malformed_node_extras_are_reported_with_the_node() {
        let err = import(r#"{"name":"a","extras":{"anim":"yes"}}"#, "").unwrap_err();
        assert!(matches!(err, ImportError::BadExtras { owner, .. } if owner == "node 0"));
    }

    #[test]
    fn clip_without_extras_spans_its_latest_key_and_clamps() {
        let list = [linear_translation_clip("open", "null")];
        let asset = import(TWO_NODES, &animations(&list)).unwrap();
        let clip = &asset.clips[0];
        assert_eq!(clip.name(), "open");
        assert_eq!(clip.duration(), 2.0);
        assert_eq!(clip.cycle(), Cycle::Clamp);
        assert_eq!(clip.tracks()[0].node_name, "leaf");
        assert_eq!(
            clip.tracks()[0].translation.as_ref().unwrap().values()[1],
            Vec3::new(2.0, 4.0, 6.0)
        );
    }

    #[test]
    fn start_extra_rebases_key_times_and_stop_sets_the_duration() {
        let list = [linear_translation_clip("c", r#"{"start":1.0,"stop":1.5}"#)];
        let asset = import(TWO_NODES, &animations(&list)).unwrap();
        let clip = &asset.clips[0];
        let times = clip.tracks()[0].translation.as_ref().unwrap().times();
        assert_eq!(times, &[0.0, 1.0]);
        assert_eq!(clip.duration(), 0.5);
    }

    #[test]
    fn cycle_extra_selects_looping() {
        let list = [linear_translation_clip("c", r#"{"cycle":"loop"}"#)];
        let asset = import(TWO_NODES, &animations(&list)).unwrap();
        assert_eq!(asset.clips[0].cycle(), Cycle::Loop);
    }

    #[test]
    fn text_keys_and_annotations_become_rebased_sorted_markers() {
        let extras = r#"{"start":1.0,"text_keys":[{"t":2.0,"key":"end"}],"annotations":[{"t":1.5,"text":"mid"}]}"#;
        let list = [linear_translation_clip("c", extras)];
        let asset = import(TWO_NODES, &animations(&list)).unwrap();
        let markers: Vec<(f32, &str)> = asset.clips[0]
            .markers()
            .iter()
            .map(|m| (m.time, m.text.as_str()))
            .collect();
        assert_eq!(markers, [(0.5, "mid"), (1.0, "end")]);
    }

    #[test]
    fn malformed_clip_extras_are_reported_with_the_clip_name() {
        let list = [linear_translation_clip("c", r#"{"cycle":"bounce"}"#)];
        let err = import(TWO_NODES, &animations(&list)).unwrap_err();
        assert!(matches!(err, ImportError::BadExtras { owner, .. } if owner.contains("\"c\"")));
    }

    #[test]
    fn translation_and_rotation_channels_of_one_node_share_a_track() {
        let list = [animation(
            "c",
            "null",
            &[channel(0, 1, "translation"), channel(1, 1, "rotation")],
            &[
                sampler(TIMES, TRANSLATIONS, "LINEAR"),
                sampler(TIMES, ROTATIONS, "STEP"),
            ],
        )];
        let asset = import(TWO_NODES, &animations(&list)).unwrap();
        let tracks = asset.clips[0].tracks();
        assert_eq!(tracks.len(), 1);
        assert!(tracks[0].translation.is_some() && tracks[0].rotation.is_some());
        assert_eq!(
            tracks[0].rotation.as_ref().unwrap().interpolation(),
            Interpolation::Step
        );
    }

    #[test]
    fn cubic_spline_interpolation_is_rejected() {
        let list = [animation(
            "c",
            "null",
            &[channel(0, 1, "translation")],
            &[sampler(TIMES, TRANSLATIONS, "CUBICSPLINE")],
        )];
        let err = import(TWO_NODES, &animations(&list)).unwrap_err();
        assert!(matches!(err, ImportError::CubicSpline { animation } if animation == "c"));
    }

    #[test]
    fn animating_the_same_property_twice_is_rejected() {
        let list = [animation(
            "c",
            "null",
            &[channel(0, 1, "translation"), channel(0, 1, "translation")],
            &[sampler(TIMES, TRANSLATIONS, "LINEAR")],
        )];
        let err = import(TWO_NODES, &animations(&list)).unwrap_err();
        assert!(matches!(
            err,
            ImportError::DuplicateChannel {
                property: "translation",
                ..
            }
        ));
    }

    #[test]
    fn a_zero_rotation_key_is_rejected() {
        let list = [animation(
            "c",
            "null",
            &[channel(0, 1, "rotation")],
            &[sampler(TIMES, ROTATIONS_WITH_ZERO, "LINEAR")],
        )];
        let err = import(TWO_NODES, &animations(&list)).unwrap_err();
        assert!(matches!(err, ImportError::DegenerateRotation { .. }));
    }

    #[test]
    fn rotation_keys_are_normalised_on_import() {
        let list = [animation(
            "c",
            "null",
            &[channel(0, 1, "rotation")],
            &[sampler(TIMES, ROTATIONS, "LINEAR")],
        )];
        let asset = import(TWO_NODES, &animations(&list)).unwrap();
        let keys = asset.clips[0].tracks()[0].rotation.as_ref().unwrap();
        assert!(
            keys.values()
                .iter()
                .all(|q| (q.length() - 1.0).abs() < 1e-6)
        );
    }

    #[test]
    fn duplicate_key_times_collapse_and_raise_a_warning() {
        let list = [animation(
            "c",
            "null",
            &[channel(0, 1, "translation")],
            &[sampler(
                TIMES_WITH_DUPLICATE,
                TRANSLATIONS_WITH_DUPLICATE,
                "LINEAR",
            )],
        )];
        let asset = import(TWO_NODES, &animations(&list)).unwrap();
        let keys = asset.clips[0].tracks()[0].translation.as_ref().unwrap();
        assert_eq!(
            keys.values()[0],
            Vec3::new(9.0, 9.0, 9.0),
            "last duplicate wins"
        );
        assert!(matches!(
            asset.warnings.as_slice(),
            [ImportWarning::DuplicateKeysCollapsed { count: 1, .. }]
        ));
    }

    #[test]
    fn a_track_with_decreasing_times_is_skipped_with_a_warning_not_a_failure() {
        let list = [animation(
            "c",
            "null",
            &[channel(0, 1, "translation")],
            &[sampler(TIMES_DECREASING, TRANSLATIONS, "LINEAR")],
        )];
        let asset = import(TWO_NODES, &animations(&list)).unwrap();
        assert!(asset.clips[0].tracks().is_empty());
        assert!(matches!(
            asset.warnings.as_slice(),
            [ImportWarning::TrackSkipped {
                reason: ClipError::KeyTimesDecreasing { .. },
                ..
            }]
        ));
    }

    #[test]
    fn two_animations_with_the_same_name_are_rejected() {
        let list = [
            linear_translation_clip("same", "null"),
            linear_translation_clip("same", "null"),
        ];
        let err = import(TWO_NODES, &animations(&list)).unwrap_err();
        assert!(matches!(err, ImportError::DuplicateClipName(name) if name == "same"));
    }

    #[test]
    fn an_unnamed_animation_gets_an_index_based_name() {
        let list = [r#"{"channels":[],"samplers":[]}"#.to_owned()];
        let asset = import(TWO_NODES, &animations(&list)).unwrap();
        assert_eq!(asset.clips[0].name(), "animation_0");
        assert_eq!(asset.clips[0].duration(), 0.0);
    }

    #[test]
    fn skin_joint_names_and_inverse_binds_are_read_in_skin_order() {
        let skins =
            format!(r#","skins":[{{"joints":[1,0],"inverseBindMatrices":{INVERSE_BINDS}}}]"#);
        let asset = import(TWO_NODES, &skins).unwrap();
        let skin = &asset.skins[0];
        assert_eq!(skin.joint_names, ["leaf", "root"]);
        assert_eq!(skin.inverse_bind[0], Mat4::IDENTITY);
        assert_eq!(
            skin.inverse_bind[1].w_axis.truncate(),
            Vec3::new(-1.0, -2.0, -3.0)
        );
    }

    #[test]
    fn a_skin_without_inverse_binds_gets_identities() {
        let skins = r#","skins":[{"joints":[0,1]}]"#;
        let asset = import(TWO_NODES, skins).unwrap();
        assert_eq!(asset.skins[0].inverse_bind, [Mat4::IDENTITY; 2]);
    }

    #[test]
    fn a_skin_with_the_wrong_number_of_inverse_binds_is_rejected() {
        let skins =
            format!(r#","skins":[{{"joints":[0,1,0],"inverseBindMatrices":{INVERSE_BINDS}}}]"#);
        let err = import(TWO_NODES, &skins).unwrap_err();
        assert!(matches!(
            err,
            ImportError::InverseBindCount {
                joints: 3,
                matrices: 2,
                ..
            }
        ));
    }

    #[test]
    fn load_reads_a_glb_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiny.glb");
        std::fs::write(&path, glb(TWO_NODES, "")).unwrap();
        let asset = AnimationAsset::load(path.to_str().unwrap()).unwrap();
        assert_eq!(asset.skeleton.len(), 2);
    }

    #[test]
    fn load_reports_a_missing_file_as_a_gltf_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("absent.glb");
        let err = AnimationAsset::load(path.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, ImportError::Gltf(_)));
    }

    #[test]
    fn load_reports_garbage_bytes_as_a_gltf_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("junk.glb");
        std::fs::write(&path, b"this is not a glb at all").unwrap();
        let err = AnimationAsset::load(path.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, ImportError::Gltf(_)));
    }
}
