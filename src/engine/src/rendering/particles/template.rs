//! A particle system prepared for simulation: the parsed extras plus the node
//! frames, emitter geometry and tracks resolved against one model file.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Mat4, Vec3};

use crate::anim::scalar_track::ScalarTrack;
use crate::assets::types::effect_material::EffectMaterial;
use crate::assets::types::mesh::{CpuMesh, LoadedModel, Topology, node_world_matrices};
use crate::assets::types::model_extras::NodeExtras;
use crate::assets::types::particle_system::{
    EmissionType, Emitter, EmitterKind, Modifier, ModifierKind, ParticleSystem, TrackField,
    aspect_flags,
};

/// Model-space transform of every named glTF node of a model file.
pub type NodeTransforms = HashMap<String, Mat4>;

/// Transforms by node name. Nodes sharing a name cannot be told apart by name: the last one
/// wins and the clash is reported once per `source`.
pub fn node_transforms(document: &gltf::Document, source: &str) -> NodeTransforms {
    let worlds = node_world_matrices(document);
    let mut transforms = NodeTransforms::new();
    let mut duplicates = 0;
    for node in document.nodes() {
        let Some(name) = node.name() else { continue };
        if transforms
            .insert(name.to_owned(), worlds[node.index()])
            .is_some()
        {
            duplicates += 1;
        }
    }
    if duplicates > 0 {
        crate::utils::warn_once(&format!("node-names:{source}"), || {
            format!(
                "{source}: {duplicates} glTF node names occur more than once; particle systems resolve the last one"
            )
        });
    }
    transforms
}

#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    #[error("particle system `{system}`: {role} node `{node}` is not in the model")]
    MissingNode {
        system: String,
        role: &'static str,
        node: String,
    },
    #[error("particle system `{system}`: mesh emitter `{emitter}` has no usable geometry")]
    MeshEmitterWithoutGeometry { system: String, emitter: String },
    #[error("particle system `{system}`: track target `{target}` names no emitter or modifier")]
    UnknownTrackTarget { system: String, target: String },
}

/// One triangle of a mesh emitter, in the system node's local space.
#[derive(Debug, Clone, Copy)]
pub(super) struct Triangle {
    pub corners: [Vec3; 3],
    pub normal: Vec3,
}

/// One source vertex of a vertex-emitting mesh emitter, in the system node's local space.
#[derive(Debug, Clone, Copy)]
pub(super) struct VertexPoint {
    pub position: Vec3,
    /// Zero when the source has no normal: the emission direction then falls back to the
    /// emitter's own axis.
    pub normal: Vec3,
}

/// Where a mesh emitter's particles are born, in the system node's local space.
#[derive(Debug)]
pub(super) enum MeshSurface {
    Faces {
        triangles: Vec<Triangle>,
        /// Running total of triangle areas, for area-weighted picking.
        cumulative_area: Vec<f32>,
    },
    Vertices(Vec<VertexPoint>),
}

impl MeshSurface {
    /// Face emission over triangle sources; vertex emission, or face emission whose sources
    /// are point sets only, samples vertices (the second value is true for that fallback).
    fn build<'a>(
        meshes: impl Iterator<Item = &'a CpuMesh> + Clone,
        to_system: Mat4,
        emission: Option<EmissionType>,
    ) -> Option<(Self, bool)> {
        if emission == Some(EmissionType::Vertices) {
            return Self::build_vertices(meshes, to_system).map(|surface| (surface, false));
        }
        let mut triangles = Vec::new();
        let mut cumulative_area = Vec::new();
        let mut total = 0.0;
        for mesh in meshes
            .clone()
            .filter(|mesh| mesh.topology == Topology::Triangles)
        {
            for corner_indices in mesh.indices.chunks_exact(3) {
                let corners: [Vec3; 3] = std::array::from_fn(|corner| {
                    let index = corner_indices[corner];
                    to_system.transform_point3(Vec3::from(mesh.vertices[index as usize].position))
                });
                let cross = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
                let area = cross.length() * 0.5;
                if area <= f32::EPSILON {
                    continue;
                }
                total += area;
                cumulative_area.push(total);
                triangles.push(Triangle {
                    corners,
                    normal: cross / (area * 2.0),
                });
            }
        }
        if triangles.is_empty() {
            let points_only = meshes.clone().all(|mesh| mesh.topology == Topology::Points);
            return points_only
                .then(|| Self::build_vertices(meshes, to_system))
                .flatten()
                .map(|surface| (surface, true));
        }
        Some((
            Self::Faces {
                triangles,
                cumulative_area,
            },
            false,
        ))
    }

    fn build_vertices<'a>(
        meshes: impl Iterator<Item = &'a CpuMesh>,
        to_system: Mat4,
    ) -> Option<Self> {
        let normal_matrix = glam::Mat3::from_mat4(to_system);
        let points: Vec<VertexPoint> = meshes
            .flat_map(|mesh| &mesh.vertices)
            .map(|vertex| VertexPoint {
                position: to_system.transform_point3(Vec3::from(vertex.position)),
                normal: (normal_matrix * Vec3::from(vertex.normal)).normalize_or_zero(),
            })
            .collect();
        (!points.is_empty()).then_some(Self::Vertices(points))
    }

    /// The triangle (area-weighted) or vertex (uniform) at `pick` in `[0, 1)`.
    pub fn pick(&self, pick: f32) -> SurfacePick<'_> {
        match self {
            Self::Faces {
                triangles,
                cumulative_area,
            } => {
                let total = cumulative_area.last().copied().unwrap_or(0.0);
                let index = cumulative_area
                    .partition_point(|&area| area <= pick * total)
                    .min(triangles.len() - 1);
                SurfacePick::Triangle(&triangles[index])
            }
            Self::Vertices(points) => {
                let index = ((pick * points.len() as f32) as usize).min(points.len() - 1);
                SurfacePick::Vertex(&points[index])
            }
        }
    }
}

pub(super) enum SurfacePick<'a> {
    Triangle(&'a Triangle),
    Vertex(&'a VertexPoint),
}

#[derive(Debug)]
pub(super) struct EmitterSetup {
    pub spec: Emitter,
    /// Emitter node frame relative to the system node.
    pub to_system: Mat4,
    pub surface: Option<MeshSurface>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TrackTarget {
    Emitter(usize),
    Modifier(usize),
}

#[derive(Debug)]
pub(super) struct ResolvedTrack {
    pub target: TrackTarget,
    pub field: TrackField,
    pub track: ScalarTrack,
}

/// Indices of the first modifier of each single-instance kind.
#[derive(Debug, Default)]
pub(super) struct ModifierIndex {
    pub lod: Option<usize>,
    pub simple_color: Option<usize>,
    pub scale: Option<usize>,
    pub sub_tex: Option<usize>,
    pub rotation: Option<usize>,
    pub inherit_velocity: Option<usize>,
}

#[derive(Debug)]
pub struct ParticleSystemTemplate {
    pub name: String,
    pub(super) system: ParticleSystem,
    pub(super) material: Arc<EffectMaterial>,
    /// The system node's model-space transform.
    pub(super) system_model: Mat4,
    pub(super) emitters: Vec<EmitterSetup>,
    /// Sorted by `order`.
    pub(super) modifiers: Vec<Modifier>,
    /// Object node frame relative to the system node, per modifier that names one.
    pub(super) object_frames: Vec<Option<Mat4>>,
    pub(super) index: ModifierIndex,
    pub(super) tracks: Vec<ResolvedTrack>,
    /// Authored fields this simulation does not model, so a differing look is explainable.
    pub unsupported_fields: Vec<&'static str>,
}

impl ParticleSystemTemplate {
    pub fn new(
        node: &NodeExtras<ParticleSystem>,
        nodes: &NodeTransforms,
        emitter_meshes: &HashMap<String, Vec<CpuMesh>>,
        material: Arc<EffectMaterial>,
    ) -> Result<Self, TemplateError> {
        let system = &node.value;
        let name = &node.node_name;
        let system_model = node.transform;
        let from_model = system_model.inverse();
        let frame_of = |role: &'static str, node_name: &str| {
            if node_name == name {
                return Ok(Mat4::IDENTITY);
            }
            nodes
                .get(node_name)
                .map(|model| from_model * *model)
                .ok_or_else(|| TemplateError::MissingNode {
                    system: name.clone(),
                    role,
                    node: node_name.to_owned(),
                })
        };

        let vertex_fallback = std::cell::Cell::new(false);
        let emitters = system
            .emitters
            .iter()
            .map(|spec| {
                let surface = (spec.kind == EmitterKind::Mesh)
                    .then(|| {
                        let meshes = spec
                            .mesh_nodes
                            .iter()
                            .filter_map(|mesh_node| emitter_meshes.get(mesh_node))
                            .flatten();
                        let (surface, fell_back) =
                            MeshSurface::build(meshes, from_model, spec.emission_type).ok_or_else(
                                || TemplateError::MeshEmitterWithoutGeometry {
                                    system: name.clone(),
                                    emitter: spec.name.clone(),
                                },
                            )?;
                        vertex_fallback.set(vertex_fallback.get() || fell_back);
                        Ok(surface)
                    })
                    .transpose()?;
                Ok(EmitterSetup {
                    spec: spec.clone(),
                    to_system: frame_of("emitter", &spec.node)?,
                    surface,
                })
            })
            .collect::<Result<Vec<_>, TemplateError>>()?;

        let mut modifiers = system.modifiers.clone();
        modifiers.sort_by_key(|modifier| modifier.order);
        let object_frames = modifiers
            .iter()
            .map(|modifier| match &modifier.kind {
                ModifierKind::Gravity { object, .. }
                | ModifierKind::Drag { object, .. }
                | ModifierKind::InheritVelocity { object, .. } => {
                    frame_of("modifier object", object).map(Some)
                }
                _ => Ok(None),
            })
            .collect::<Result<Vec<_>, _>>()?;

        let tracks = system
            .tracks
            .iter()
            .map(|field_track| {
                let target = emitters
                    .iter()
                    .position(|emitter| emitter.spec.name == field_track.target)
                    .map(TrackTarget::Emitter)
                    .or_else(|| {
                        modifiers
                            .iter()
                            .position(|modifier| modifier.name == field_track.target)
                            .map(TrackTarget::Modifier)
                    })
                    .ok_or_else(|| TemplateError::UnknownTrackTarget {
                        system: name.clone(),
                        target: field_track.target.clone(),
                    })?;
                Ok(ResolvedTrack {
                    target,
                    field: field_track.field,
                    track: field_track.track.clone(),
                })
            })
            .collect::<Result<Vec<_>, TemplateError>>()?;

        let index = ModifierIndex {
            lod: first(&modifiers, |kind| matches!(kind, ModifierKind::Lod { .. })),
            simple_color: first(&modifiers, |kind| {
                matches!(kind, ModifierKind::SimpleColor { .. })
            }),
            scale: first(&modifiers, |kind| {
                matches!(kind, ModifierKind::Scale { .. })
            }),
            sub_tex: first(&modifiers, |kind| {
                matches!(kind, ModifierKind::SubTex { .. })
            }),
            rotation: first(&modifiers, |kind| {
                matches!(kind, ModifierKind::Rotation { .. })
            }),
            inherit_velocity: first(&modifiers, |kind| {
                matches!(kind, ModifierKind::InheritVelocity { .. })
            }),
        };

        Ok(Self {
            name: name.clone(),
            system: system.clone(),
            material,
            system_model,
            emitters,
            modifiers,
            object_frames,
            index,
            tracks,
            unsupported_fields: {
                let mut fields = unsupported_fields(system);
                if vertex_fallback.get() {
                    fields.push(POINT_SOURCE_FALLBACK);
                }
                fields
            },
        })
    }

    pub fn max_particles(&self) -> usize {
        self.system.max_particles as usize
    }
}

/// Not an ignored field but a deviation worth a log line: a face-emission mesh emitter whose
/// source is a point set samples its vertices.
const POINT_SOURCE_FALLBACK: &str =
    "mesh emitter face emission over a point set: sampling vertices";

fn nonzero(value: f32) -> bool {
    value.abs() > f32::EPSILON
}

/// The authored fields of `system` the simulation ignores, each named once.
fn unsupported_fields(system: &ParticleSystem) -> Vec<&'static str> {
    let mut fields = Vec::new();
    let mut note = |field: &'static str| {
        if !fields.contains(&field) {
            fields.push(field);
        }
    };
    if nonzero(system.update.frequency - 1.0) || nonzero(system.update.phase) {
        note("update.frequency/phase");
    }
    if system
        .data
        .has_flag(aspect_flags::INITIAL_ROTATION_FROM_VELOCITY)
    {
        note("initial_rotation_from_velocity");
    }
    for modifier in &system.modifiers {
        match &modifier.kind {
            ModifierKind::Gravity { turbulence, .. } if nonzero(*turbulence) => {
                note("gravity.turbulence");
                note("gravity.turbulence_scale");
            }
            ModifierKind::Drag { range_falloff, .. } if nonzero(*range_falloff) => {
                note("drag.range_falloff");
            }
            _ => {}
        }
    }
    fields
}

fn first(modifiers: &[Modifier], matches: impl Fn(&ModifierKind) -> bool) -> Option<usize> {
    modifiers
        .iter()
        .position(|modifier| matches(&modifier.kind))
}

/// Templates and per-system failures of one model.
#[derive(Debug, Default)]
pub struct TemplateSet {
    pub templates: Vec<Arc<ParticleSystemTemplate>>,
    /// Systems that could not be prepared; the caller reports them.
    pub failed: Vec<TemplateError>,
}

impl TemplateSet {
    /// `(system name, ignored fields)` of every prepared system that ignores something.
    pub fn unsupported_fields(&self) -> impl Iterator<Item = (&str, &[&'static str])> {
        self.templates
            .iter()
            .filter(|template| !template.unsupported_fields.is_empty())
            .map(|template| {
                (
                    template.name.as_str(),
                    template.unsupported_fields.as_slice(),
                )
            })
    }

    /// Prepares every particle system of `model`. `material_for` resolves the
    /// glTF material index a system names; systems it cannot resolve are
    /// returned by node name as the second value.
    pub fn build(
        model: &LoadedModel,
        nodes: &NodeTransforms,
        material_for: impl Fn(u32) -> Option<Arc<EffectMaterial>>,
    ) -> (Self, Vec<String>) {
        let mut set = Self::default();
        let mut without_material = Vec::new();
        for node in &model.extras.particle_systems {
            let Some(material) = material_for(node.value.material) else {
                without_material.push(node.node_name.clone());
                continue;
            };
            match ParticleSystemTemplate::new(node, nodes, &model.emitter_meshes, material) {
                Ok(template) => set.templates.push(Arc::new(template)),
                Err(error) => set.failed.push(error),
            }
        }
        (set, without_material)
    }
}
