//! `node.extras.particle_system` and `node.extras.master_particle_system`: the
//! converter's description of a NIF particle system (units already engine
//! units, axes engine axes, angles radians). Every direction, extent and angle
//! is expressed in the glTF node-local frame of the node it belongs to.

use serde::{Deserialize, Serialize};

use crate::anim::scalar_track::{Keys, ScalarTrack, StepTrack, Timing};

/// `data.aspect_flags` bits.
pub mod aspect_flags {
    /// Quad long axis follows the particle velocity.
    pub const VELOCITY_ORIENTATION: u32 = 1;
    pub const INITIAL_ROTATION_FROM_VELOCITY: u32 = 2;
    /// Aspect ratio blends towards `speed_to_aspect[0]` with speed.
    pub const SPEED_TO_ASPECT: u32 = 256;
}

/// Camera-distance alpha fade in engine units; `0` disables a side.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Fade {
    pub near_begin: f32,
    pub near_end: f32,
    pub far_begin: f32,
    pub far_end: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleData {
    pub aspect_ratio: f32,
    pub aspect_flags: u32,
    /// `[aspect2, speed1, speed2]`, speeds in eu/s.
    pub speed_to_aspect: [f32; 3],
    /// `[u_offset, u_width, v_offset, v_height]` per atlas cell, row-major.
    /// Empty when the system has no atlas.
    #[serde(default)]
    pub subtexture_rects: Vec<[f32; 4]>,
}

impl ParticleData {
    pub fn has_flag(&self, flag: u32) -> bool {
        self.aspect_flags & flag != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmitterKind {
    Cylinder,
    Box,
    Sphere,
    Mesh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmissionType {
    FaceSurface,
    FaceCentre,
    /// Uniformly over the source vertices (point sets and triangle meshes alike).
    Vertices,
}

/// Birth-rate value: constant or sampled keys (timed by the emitter controller).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RateParam {
    Constant { constant: f32 },
    Keyed(Keys<f32>),
}

/// Emitter on/off value: constant or 0/1 step keys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ActiveParam {
    Constant { constant: bool },
    Keyed(Keys<f32>),
}

/// Birth rate and activity of one emitter, driven by one controller clock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmitterCtlr {
    pub birth_rate: RateParam,
    pub active: ActiveParam,
    #[serde(flatten)]
    pub timing: Timing,
}

impl EmitterCtlr {
    /// Particles per second, `elapsed` seconds after the system started.
    pub fn birth_rate_at(&self, elapsed: f32) -> f32 {
        match &self.birth_rate {
            RateParam::Constant { constant } => *constant,
            RateParam::Keyed(keys) => keys.sample_linear(self.timing.key_time(elapsed)),
        }
    }

    pub fn active_at(&self, elapsed: f32) -> bool {
        match &self.active {
            ActiveParam::Constant { constant } => *constant,
            ActiveParam::Keyed(keys) => keys.sample_step(self.timing.key_time(elapsed)) != 0.0,
        }
    }
}

fn white() -> [f32; 4] {
    [1.0; 4]
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Emitter {
    #[serde(rename = "type")]
    pub kind: EmitterKind,
    pub name: String,
    /// glTF node whose transform places the volume. All emitter directions
    /// and extents are in this node's local frame: local +Y is the declination
    /// axis, the planar angle runs around +Y from +X towards -Z.
    pub node: String,
    #[serde(default)]
    pub radius: f32,
    /// Cylinder extent along local Y; for a box, the extent along local Y.
    #[serde(default)]
    pub height: f32,
    /// Box extent along local X.
    #[serde(default)]
    pub width: f32,
    /// Box extent along local Z.
    #[serde(default)]
    pub depth: f32,
    /// glTF node names of the mesh emitter's source geometry.
    #[serde(default)]
    pub mesh_nodes: Vec<String>,
    #[serde(default)]
    pub emission_type: Option<EmissionType>,
    pub speed: f32,
    pub speed_variation: f32,
    /// Polar angle (radians) from the emitter node's local +Y.
    pub declination: f32,
    pub declination_variation: f32,
    /// Azimuth (radians) around local +Y, from +X towards -Z.
    pub planar_angle: f32,
    pub planar_angle_variation: f32,
    #[serde(default = "white")]
    pub initial_color: [f32; 4],
    pub initial_radius: f32,
    pub radius_variation: f32,
    pub life_span: f32,
    pub life_span_variation: f32,
    pub emitter_ctlr: EmitterCtlr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GravityForce {
    Planar,
    Spherical,
}

/// Per-type modifier fields; the type is the JSON `type` tag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ModifierKind {
    Lod {
        /// Dimensionless fractions of the LOD reference range.
        begin: f32,
        end: f32,
        end_emit_scale: f32,
        end_size: f32,
    },
    SimpleColor {
        fade_in_percent: f32,
        fade_out_percent: f32,
        color1_end_percent: f32,
        color2_start_percent: f32,
        color2_end_percent: f32,
        color3_start_percent: f32,
        colors: [[f32; 4]; 3],
    },
    Scale {
        scales: Vec<f32>,
    },
    Gravity {
        object: String,
        axis: [f32; 3],
        decay: f32,
        /// eu/s^2.
        strength: f32,
        force: GravityForce,
        turbulence: f32,
        turbulence_scale: f32,
        world_aligned: bool,
    },
    Drag {
        object: String,
        axis: [f32; 3],
        percentage: f32,
        /// `None` is unbounded (the NIF's `FLT_MAX`).
        #[serde(default)]
        range: Option<f32>,
        range_falloff: f32,
    },
    Wind {
        strength: f32,
    },
    SubTex {
        start_frame: f32,
        start_frame_fudge: f32,
        end_frame: f32,
        loop_start_frame: f32,
        loop_start_frame_fudge: f32,
        frame_count: f32,
        frame_count_fudge: f32,
    },
    Rotation {
        speed: f32,
        speed_variation: f32,
        angle: f32,
        angle_variation: f32,
        random_speed_sign: bool,
        random_axis: bool,
        axis: [f32; 3],
    },
    InheritVelocity {
        object: String,
        chance_percent: f32,
        multiplier: f32,
        variation: f32,
    },
}

fn active_by_default() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Modifier {
    pub name: String,
    #[serde(default)]
    pub order: i32,
    #[serde(default = "active_by_default")]
    pub active: bool,
    #[serde(default)]
    pub active_track: Option<StepTrack>,
    #[serde(flatten)]
    pub kind: ModifierKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackField {
    Speed,
    LifeSpan,
    InitialRadius,
    Declination,
    DeclinationVariation,
    PlanarAngle,
    GravityStrength,
    RotationSpeed,
    RotationSpeedVariation,
}

/// Animates one field of the named emitter or modifier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldTrack {
    /// Name of the emitter or modifier.
    pub target: String,
    pub field: TrackField,
    pub track: ScalarTrack,
}

/// Timing of the system's update controller.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UpdateTiming {
    pub frequency: f32,
    pub phase: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleSystem {
    pub v: u32,
    pub world_space: bool,
    /// Particle cap (the NIF's `BSMaxVertices`).
    pub max_particles: u32,
    /// glTF material index whose `extras.effect` holds the particle permutation.
    pub material: u32,
    pub fade: Fade,
    pub data: ParticleData,
    pub emitters: Vec<Emitter>,
    pub modifiers: Vec<Modifier>,
    #[serde(default)]
    pub tracks: Vec<FieldTrack>,
    pub update: UpdateTiming,
}

/// `BSMasterParticleSystem`: names the particle-system nodes it plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MasterParticleSystem {
    pub v: u32,
    pub max_emitter_objects: u32,
    pub systems: Vec<String>,
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "expected values are exact literals parsed or copied, not computed"
)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::assets::types::model_extras::{ExtrasError, parse_versioned};

    fn timing() -> Value {
        json!({ "cycle": "loop", "start": 0.0, "stop": 3.33, "phase": 0.0, "frequency": 1.0 })
    }

    fn emitter() -> Value {
        let mut value = json!({
            "type": "cylinder", "name": "NiPSysCylinderEmitter:0", "node": "Emitter01",
            "radius": 3.2, "height": 0.5, "width": 0, "depth": 0, "mesh_nodes": [],
            "speed": 6.0, "speed_variation": 3.96, "declination": 0,
            "declination_variation": 0.565, "planar_angle": 0, "planar_angle_variation": 1.131,
            "initial_color": [1, 1, 1, 1], "initial_radius": 2.8, "radius_variation": 0.56,
            "life_span": 1.1, "life_span_variation": 0.333,
            "emitter_ctlr": { "birth_rate": { "constant": 30 }, "active": { "constant": true } }
        });
        for (key, field) in timing().as_object().unwrap() {
            value["emitter_ctlr"][key] = field.clone();
        }
        value
    }

    fn modifiers() -> Value {
        json!([
            { "type": "lod", "name": "LOD", "order": 1, "active": true,
              "begin": 0.1, "end": 0.7, "end_emit_scale": 0.2, "end_size": 1.0 },
            { "type": "simple_color", "name": "Color", "order": 2, "active": true,
              "fade_in_percent": 0.05, "fade_out_percent": 0.5, "color1_end_percent": 0.0,
              "color2_start_percent": 1.0, "color2_end_percent": 1.0, "color3_start_percent": 1.0,
              "colors": [[1, 1, 1, 1], [1, 0.5, 0, 1], [0, 0, 0, 0]] },
            { "type": "scale", "name": "Scale", "order": 3, "active": true, "scales": [1.0, 2.0] },
            { "type": "gravity", "name": "Gravity", "order": 4, "active": true,
              "object": "GravityObj", "axis": [0, 0, 1], "decay": 0, "strength": 13.5,
              "force": "planar", "turbulence": 0, "turbulence_scale": 1, "world_aligned": true },
            { "type": "drag", "name": "DragX", "order": 5, "active": true,
              "object": "DragObj", "axis": [1, 0, 0], "percentage": 0.05,
              "range": null, "range_falloff": 0 },
            { "type": "wind", "name": "Wind", "order": 6, "active": true, "strength": 1.5 },
            { "type": "sub_tex", "name": "SubTex", "order": 7, "active": true,
              "start_frame": 0, "start_frame_fudge": 64, "end_frame": 63,
              "loop_start_frame": 0, "loop_start_frame_fudge": 0,
              "frame_count": 60, "frame_count_fudge": 0 },
            { "type": "rotation", "name": "Rot", "order": 8, "active": true,
              "speed": 0.26, "speed_variation": 0, "angle": 0, "angle_variation": 3.0,
              "random_speed_sign": true, "random_axis": true, "axis": [0, 0, 1],
              "active_track": { "cycle": "clamp", "start": 0.0, "stop": 3.33,
                                "times": [0.0, 3.33], "values": [1, 0] } },
            { "type": "inherit_velocity", "name": "Inherit", "order": 9, "active": true,
              "object": "InheritObj", "chance_percent": 100, "multiplier": 0.5,
              "variation": 0.1 }
        ])
    }

    fn fixture() -> Value {
        json!({
            "v": 1, "world_space": true, "max_particles": 60, "material": 3,
            "fade": { "near_begin": 10.0, "near_end": 5.0, "far_begin": 0, "far_end": 0 },
            "data": { "aspect_ratio": 1.2, "aspect_flags": 257,
                      "speed_to_aspect": [0.5, 0.3, 9.75],
                      "subtexture_rects": [[0, 0.5, 0, 0.5], [0.5, 0.5, 0, 0.5]] },
            "emitters": [emitter()],
            "modifiers": modifiers(),
            "tracks": [ { "target": "NiPSysCylinderEmitter:0", "field": "speed",
                          "track": { "cycle": "loop", "start": 0.0, "stop": 2.0,
                                     "times": [0.0, 2.0], "values": [6.0, 8.0] } } ],
            "update": { "frequency": 1.0, "phase": 0.0 }
        })
    }

    fn parse(value: &Value) -> Result<ParticleSystem, ExtrasError> {
        parse_versioned("particle_system", value)
    }

    #[test]
    fn full_fixture_parses_every_modifier_kind() {
        let system = parse(&fixture()).unwrap();
        assert_eq!(system.max_particles, 60);
        assert_eq!(system.fade.near_begin, 10.0);
        assert!(system.data.has_flag(aspect_flags::VELOCITY_ORIENTATION));
        assert!(system.data.has_flag(aspect_flags::SPEED_TO_ASPECT));
        assert!(
            !system
                .data
                .has_flag(aspect_flags::INITIAL_ROTATION_FROM_VELOCITY)
        );
        assert_eq!(system.data.subtexture_rects[1], [0.5, 0.5, 0.0, 0.5]);
        assert_eq!(system.emitters[0].kind, EmitterKind::Cylinder);
        assert_eq!(system.modifiers.len(), 9);
        assert!(matches!(
            system.modifiers[3].kind,
            ModifierKind::Gravity {
                force: GravityForce::Planar,
                world_aligned: true,
                ..
            }
        ));
        assert!(matches!(
            system.modifiers[4].kind,
            ModifierKind::Drag { range: None, .. }
        ));
        assert!(system.modifiers[7].active_track.is_some());
        assert_eq!(system.tracks[0].field, TrackField::Speed);
    }

    #[test]
    fn full_fixture_round_trips() {
        let system = parse(&fixture()).unwrap();
        let again = parse(&serde_json::to_value(&system).unwrap()).unwrap();
        assert_eq!(system, again);
    }

    #[test]
    fn unknown_version_is_rejected() {
        let mut value = fixture();
        value["v"] = json!(7);
        assert!(matches!(
            parse(&value),
            Err(ExtrasError::UnsupportedVersion { found: 7, .. })
        ));
    }

    #[test]
    fn unknown_modifier_type_is_rejected() {
        let mut value = fixture();
        value["modifiers"] = json!([{ "type": "bomb", "name": "Bomb" }]);
        let error = parse(&value).unwrap_err();
        assert!(matches!(error, ExtrasError::Invalid { .. }));
        assert!(error.to_string().contains("bomb"));
    }

    #[test]
    fn mesh_emitter_carries_node_names_and_emission_type() {
        let mut value = fixture();
        value["emitters"][0]["type"] = json!("mesh");
        value["emitters"][0]["mesh_nodes"] = json!(["Flames01", "Flames02"]);
        value["emitters"][0]["emission_type"] = json!("face_surface");
        let system = parse(&value).unwrap();
        let emitter = &system.emitters[0];
        assert_eq!(emitter.kind, EmitterKind::Mesh);
        assert_eq!(emitter.mesh_nodes, ["Flames01", "Flames02"]);
        assert_eq!(emitter.emission_type, Some(EmissionType::FaceSurface));
    }

    #[test]
    fn empty_atlas_is_allowed() {
        let mut value = fixture();
        value["data"]["subtexture_rects"] = json!([]);
        assert!(parse(&value).unwrap().data.subtexture_rects.is_empty());
    }

    #[test]
    fn constant_emitter_controller_ignores_time() {
        let ctlr = parse(&fixture()).unwrap().emitters.remove(0).emitter_ctlr;
        assert_eq!(ctlr.birth_rate_at(0.0), 30.0);
        assert_eq!(ctlr.birth_rate_at(99.0), 30.0);
        assert!(ctlr.active_at(5.0));
    }

    #[test]
    fn keyed_emitter_controller_follows_its_own_clock() {
        let mut value = fixture();
        value["emitters"][0]["emitter_ctlr"]["birth_rate"] =
            json!({ "times": [0.0, 2.0], "values": [0.0, 20.0] });
        value["emitters"][0]["emitter_ctlr"]["active"] =
            json!({ "times": [0.0, 3.33], "values": [1, 0] });
        value["emitters"][0]["emitter_ctlr"]["stop"] = json!(3.33);
        let ctlr = parse(&value).unwrap().emitters.remove(0).emitter_ctlr;
        assert_eq!(ctlr.birth_rate_at(1.0), 10.0);
        assert!(ctlr.active_at(3.0));
        assert!(ctlr.active_at(3.3));
        // Looping: 3.33 + 1.0 wraps back to 1.0 s, where the rate is 10 again.
        assert!((ctlr.birth_rate_at(4.33) - 10.0).abs() < 1e-3);

        value["emitters"][0]["emitter_ctlr"]["cycle"] = json!("clamp");
        let clamped = parse(&value).unwrap().emitters.remove(0).emitter_ctlr;
        assert!(!clamped.active_at(3.33));
        assert!(!clamped.active_at(50.0));
    }

    #[test]
    fn emitter_without_controller_is_rejected() {
        let mut value = fixture();
        value["emitters"][0]
            .as_object_mut()
            .unwrap()
            .remove("emitter_ctlr");
        assert!(matches!(parse(&value), Err(ExtrasError::Invalid { .. })));
    }

    #[test]
    fn master_particle_system_round_trips() {
        let value = json!({ "v": 1, "max_emitter_objects": 20, "systems": ["A", "B"] });
        let master: MasterParticleSystem =
            parse_versioned("master_particle_system", &value).unwrap();
        assert_eq!(master.max_emitter_objects, 20);
        let again: MasterParticleSystem = parse_versioned(
            "master_particle_system",
            &serde_json::to_value(&master).unwrap(),
        )
        .unwrap();
        assert_eq!(master, again);
    }
}
