//! Seeded simulations of the emitters, modifiers, caps and the quad builder.
#![allow(
    clippy::float_cmp,
    reason = "expected values are exact literals or exactly representable sums"
)]

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Mat4, Vec3, Vec4};
use serde_json::{Value, json};

use super::instance::SystemInstance;
use super::quads::CameraBasis;
use super::template::{NodeTransforms, ParticleSystemTemplate, TemplateError};
use super::world::{ParticleWorld, ParticleWorldConfig};
use crate::assets::types::effect_material::EffectMaterial;
use crate::assets::types::mesh::{CpuMesh, Topology};
use crate::assets::types::model_extras::{NodeExtras, parse_versioned};
use crate::assets::types::particle_system::ParticleSystem;
use crate::rendering::effects::particle_draw::{ParticleDraw, ParticleDrawPool};
use crate::rendering::frustum::clip_planes;
use crate::rendering::vertex::Vertex;

const SEED: u64 = 42;

fn material() -> Arc<EffectMaterial> {
    let value = json!({
        "v": 1, "vs": "effect_vs_00a00015", "ps": "effect_ps_00a06015",
        "flags1": 0, "flags2": 0,
        "blend": { "mode": "add", "src": "src_alpha", "dst": "one",
                   "alpha_test": false, "test_func": "greater", "test_ref": 0.5 },
        "z_test": true, "z_write": false, "double_sided": true, "clamp": "wrap_s_wrap_t",
        "textures": { "source": "", "greyscale": "", "normal": "", "env": "",
                      "env_mask": "", "reflectance": "", "lighting": "", "emit_gradient": "" },
        "base_color": [1, 1, 1, 1], "base_color_scale": 1.0,
        "falloff": { "start_angle": 1.0, "stop_angle": 1.0,
                     "start_opacity": 0.0, "stop_opacity": 0.0 },
        "soft_falloff_depth": 10.0, "uv_offset": [0, 0], "uv_scale": [1, 1],
        "lighting_influence": 0.0, "env_map_scale": 1.0, "emittance_color": [0, 0, 0],
        "refraction_power": 0.0, "alpha": 1.0
    });
    Arc::new(parse_versioned("effect", &value).unwrap())
}

/// A world-space system with one stationary point-like box emitter at rate 30
/// and no modifiers; tests override pieces of it.
fn base_json() -> Value {
    json!({
        "v": 1, "world_space": true, "max_particles": 1000, "material": 0,
        "fade": { "near_begin": 0, "near_end": 0, "far_begin": 0, "far_end": 0 },
        "data": { "aspect_ratio": 1.0, "aspect_flags": 0, "speed_to_aspect": [1, 0, 1],
                  "subtexture_rects": [] },
        "emitters": [{
            "type": "box", "name": "Emitter", "node": "EmitterNode",
            "width": 0, "height": 0, "depth": 0,
            "speed": 0, "speed_variation": 0, "declination": 0, "declination_variation": 0,
            "planar_angle": 0, "planar_angle_variation": 0,
            "initial_color": [1, 1, 1, 1], "initial_radius": 1.0, "radius_variation": 0,
            "life_span": 100.0, "life_span_variation": 0,
            "emitter_ctlr": { "birth_rate": { "constant": 30 }, "active": { "constant": true },
                              "cycle": "loop", "start": 0.0, "stop": 1.0 }
        }],
        "modifiers": [],
        "update": { "frequency": 1.0, "phase": 0.0 }
    })
}

fn nodes() -> NodeTransforms {
    HashMap::from([
        ("EmitterNode".to_owned(), Mat4::IDENTITY),
        ("GravityNode".to_owned(), Mat4::IDENTITY),
    ])
}

fn parse(value: &Value) -> NodeExtras<ParticleSystem> {
    NodeExtras {
        node_name: "System".to_owned(),
        transform: Mat4::IDENTITY,
        value: parse_versioned("particle_system", value).unwrap(),
    }
}

fn try_template(
    value: &Value,
    meshes: &HashMap<String, Vec<CpuMesh>>,
) -> Result<ParticleSystemTemplate, TemplateError> {
    ParticleSystemTemplate::new(&parse(value), &nodes(), meshes, material())
}

fn template(value: &Value) -> Arc<ParticleSystemTemplate> {
    Arc::new(try_template(value, &HashMap::new()).unwrap())
}

fn instance(value: &Value) -> SystemInstance {
    SystemInstance::new(1, template(value), Mat4::IDENTITY, SEED)
}

fn build_within(
    system: &SystemInstance,
    camera: &CameraBasis,
    planes: &[Vec4],
) -> Option<ParticleDraw> {
    let mut pool = ParticleDrawPool::default();
    system
        .build_draw(camera, planes, &mut pool)
        .then(|| pool.take_first())
        .flatten()
}

fn build(system: &SystemInstance, camera: &CameraBasis) -> Option<ParticleDraw> {
    build_within(system, camera, &[])
}

fn run(system: &mut SystemInstance, dt: f32, steps: usize) {
    for _ in 0..steps {
        system.update(dt, Vec3::new(0.0, 0.0, 1.0e6));
    }
}

fn with(mut value: Value, edit: impl FnOnce(&mut Value)) -> Value {
    edit(&mut value);
    value
}

fn modifier(kind: &Value) -> Value {
    let mut base = json!({ "name": "M", "order": 0, "active": true });
    base.as_object_mut()
        .unwrap()
        .extend(kind.as_object().unwrap().clone());
    base
}

/// One particle at the origin, then the emitter switches off: rate 4 at dt 0.25
/// emits exactly one on the first step and nothing afterwards.
fn single_particle(edit: impl FnOnce(&mut Value)) -> Value {
    with(base_json(), |value| {
        value["emitters"][0]["emitter_ctlr"] = json!({
            "birth_rate": { "constant": 4 },
            "active": { "times": [0.0, 0.3], "values": [1, 0] },
            "cycle": "clamp", "start": 0.0, "stop": 0.3
        });
        edit(value);
    })
}

#[test]
fn birth_rate_30_spawns_30_particles_in_one_second() {
    let mut system = instance(&base_json());
    run(&mut system, 0.125, 8);
    assert_eq!(system.particle_count(), 30);
}

#[test]
fn simulation_is_deterministic_per_seed() {
    let value = with(base_json(), |v| {
        v["emitters"][0]["speed"] = json!(5.0);
        v["emitters"][0]["speed_variation"] = json!(2.0);
        v["emitters"][0]["declination_variation"] = json!(1.0);
        v["emitters"][0]["planar_angle_variation"] = json!(6.0);
    });
    let positions = |seed| {
        let mut system = SystemInstance::new(1, template(&value), Mat4::IDENTITY, seed);
        run(&mut system, 0.125, 8);
        system
            .particles
            .iter()
            .map(|p| p.position)
            .collect::<Vec<_>>()
    };
    assert_eq!(positions(SEED), positions(SEED));
    assert_ne!(positions(SEED), positions(SEED + 1));
}

#[test]
fn planar_gravity_integrates_velocity_and_position() {
    let value = single_particle(|v| {
        v["modifiers"] = json!([modifier(&json!({
            "type": "gravity", "object": "GravityNode", "axis": [0, -1, 0], "decay": 0,
            "strength": 10.0, "force": "planar", "turbulence": 0,
            "turbulence_scale": 1, "world_aligned": true
        }))]);
    });
    let mut system = instance(&value);
    run(&mut system, 0.25, 4);
    let [particle] = system.particles.as_slice() else {
        panic!("expected exactly one particle");
    };
    assert_eq!(particle.velocity, Vec3::new(0.0, -7.5, 0.0));
    assert_eq!(particle.position, Vec3::new(0.0, -3.75, 0.0));
}

#[test]
fn gravity_axis_follows_a_non_aligned_object() {
    let mut transforms = nodes();
    transforms.insert(
        "GravityNode".to_owned(),
        Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2),
    );
    let value = single_particle(|v| {
        v["modifiers"] = json!([modifier(&json!({
            "type": "gravity", "object": "GravityNode", "axis": [0, 1, 0], "decay": 0,
            "strength": 10.0, "force": "planar", "turbulence": 0,
            "turbulence_scale": 1, "world_aligned": false
        }))]);
    });
    let template =
        ParticleSystemTemplate::new(&parse(&value), &transforms, &HashMap::new(), material())
            .unwrap();
    let mut system = SystemInstance::new(1, Arc::new(template), Mat4::IDENTITY, SEED);
    run(&mut system, 0.25, 4);
    // Local +Y rotated a quarter turn about Z points along world -X.
    let velocity = system.particles[0].velocity;
    assert!((velocity - Vec3::new(-7.5, 0.0, 0.0)).length() < 1e-4);
}

#[test]
fn drag_decays_velocity_along_its_axis_only() {
    let value = single_particle(|v| {
        v["emitters"][0]["speed"] = json!(8.0);
        v["emitters"][0]["declination"] = json!(std::f32::consts::FRAC_PI_4);
        v["modifiers"] = json!([modifier(&json!({
            "type": "drag", "object": "GravityNode", "axis": [0, 1, 0],
            "percentage": 0.5, "range": null, "range_falloff": 0
        }))]);
    });
    let mut system = instance(&value);
    run(&mut system, 0.25, 4);
    let velocity = system.particles[0].velocity;
    let initial = 8.0 * std::f32::consts::FRAC_1_SQRT_2;
    assert!(
        (velocity.x - initial).abs() < 1e-4,
        "x untouched: {velocity}"
    );
    let expected_y = initial * (-0.5f32 * 0.25 * 3.0).exp();
    assert!(
        (velocity.y - expected_y).abs() < 1e-4,
        "y decays: {velocity}"
    );
}

#[test]
fn colour_follows_the_keys_over_the_life() {
    let value = single_particle(|v| {
        v["emitters"][0]["life_span"] = json!(4.0);
        v["modifiers"] = json!([modifier(&json!({
            "type": "simple_color", "fade_in_percent": 0, "fade_out_percent": 1,
            "color1_end_percent": 0, "color2_start_percent": 1, "color2_end_percent": 1,
            "color3_start_percent": 1,
            "colors": [[1, 0, 0, 1], [0, 1, 0, 1], [0, 0, 1, 1]]
        }))]);
    });
    let mut system = instance(&value);
    run(&mut system, 0.25, 4);
    let particle = &system.particles[0];
    let life = particle.age / particle.life;
    assert!((life - 0.21875).abs() < 1e-6);
    assert!((particle.colour[0] - (1.0 - life)).abs() < 1e-5);
    assert!((particle.colour[1] - life).abs() < 1e-5);
}

#[test]
fn flipbook_picks_the_frame_for_the_particle_age() {
    let rects: Vec<Value> = (0..8)
        .map(|k| json!([k as f32 / 8.0, 0.125, 0, 1]))
        .collect();
    let value = single_particle(|v| {
        v["data"]["subtexture_rects"] = json!(rects);
        v["modifiers"] = json!([modifier(&json!({
            "type": "sub_tex", "start_frame": 0, "start_frame_fudge": 0, "end_frame": 7,
            "loop_start_frame": 0, "loop_start_frame_fudge": 0,
            "frame_count": 4, "frame_count_fudge": 0
        }))]);
    });
    let mut system = instance(&value);
    let mut cells = Vec::new();
    for _ in 0..4 {
        system.update(0.25, Vec3::new(0.0, 0.0, 1.0e6));
        cells.push(system.particles.first().map(|p| p.cell));
    }
    // Age 0.125 / 0.375 / 0.625 / 0.875 s at 4 frames per second.
    assert_eq!(cells, [Some(0), Some(1), Some(2), Some(3)]);
}

#[test]
fn scale_modifier_scales_the_quad_over_the_life() {
    let value = single_particle(|v| {
        v["emitters"][0]["life_span"] = json!(1.0);
        v["modifiers"] = json!([modifier(&json!({ "type": "scale", "scales": [1.0, 3.0] }))]);
    });
    let mut system = instance(&value);
    run(&mut system, 0.25, 3);
    let particle = &system.particles[0];
    assert!((particle.scale - (1.0 + 2.0 * 0.625)).abs() < 1e-5);
}

fn lod_value() -> Value {
    with(base_json(), |v| {
        v["modifiers"] = json!([modifier(&json!({
            "type": "lod", "begin": 0.1, "end": 0.7, "end_emit_scale": 0.5, "end_size": 2.0
        }))]);
    })
}

#[test]
fn lod_scales_emission_with_camera_distance() {
    let count_at = |distance: f32| {
        let mut system = instance(&lod_value());
        for _ in 0..8 {
            system.update(0.125, Vec3::new(0.0, 0.0, distance));
        }
        system.particle_count()
    };
    assert_eq!(
        count_at(50.0),
        30,
        "inside the begin fraction nothing is scaled"
    );
    assert_eq!(
        count_at(1000.0),
        15,
        "beyond the end fraction emission is halved"
    );
    let halfway = count_at(400.0);
    assert!((16..30).contains(&halfway), "halfway blends: {halfway}");
}

#[test]
fn lod_scales_particle_size_with_camera_distance() {
    let mut system = instance(&lod_value());
    system.update(0.25, Vec3::new(0.0, 0.0, 1000.0));
    let camera = CameraBasis::from_camera_to_world(Mat4::from_translation(Vec3::Z * 1000.0));
    let draw = build(&system, &camera).unwrap();
    let width = draw.vertices[2].position[0] - draw.vertices[0].position[0];
    assert!(
        (width - 4.0).abs() < 1e-4,
        "radius 1 x end size 2 -> width 4, got {width}"
    );
}

fn fade_value() -> Value {
    with(base_json(), |v| {
        v["fade"]["near_begin"] = json!(10.0);
        v["fade"]["near_end"] = json!(5.0);
        v["fade"]["far_begin"] = json!(200.0);
        v["fade"]["far_end"] = json!(100.0);
    })
}

fn alpha_at(distance: f32) -> Option<u8> {
    let mut system = instance(&fade_value());
    system.update(0.25, Vec3::new(0.0, 0.0, distance));
    let camera = CameraBasis::from_camera_to_world(Mat4::from_translation(Vec3::Z * distance));
    build(&system, &camera).map(|draw| draw.vertices[0].colour[3])
}

#[test]
fn near_and_far_fades_ramp_the_alpha() {
    assert_eq!(alpha_at(3.0), None, "invisible inside near_end");
    assert_eq!(alpha_at(7.5), Some(128), "half way up the near fade");
    assert_eq!(alpha_at(50.0), Some(255), "opaque between the fades");
    assert_eq!(alpha_at(150.0), Some(128), "half way down the far fade");
    assert_eq!(alpha_at(250.0), None, "invisible beyond the far fade");
}

#[test]
fn particle_cap_limits_the_live_count() {
    let value = with(base_json(), |v| {
        v["max_particles"] = json!(5);
        v["emitters"][0]["emitter_ctlr"]["birth_rate"] = json!({ "constant": 100 });
    });
    let mut system = instance(&value);
    run(&mut system, 0.125, 8);
    assert_eq!(system.particle_count(), 5);
}

#[test]
fn particles_die_after_their_life_span() {
    let value = with(base_json(), |v| {
        v["emitters"][0]["life_span"] = json!(0.5);
        v["emitters"][0]["emitter_ctlr"]["active"] = json!({ "constant": false });
    });
    let mut system = instance(&value);
    run(&mut system, 0.125, 8);
    assert_eq!(system.particle_count(), 0);
}

#[test]
fn world_caps_systems_and_clamps_the_step() {
    let config = ParticleWorldConfig {
        max_systems: 2,
        ..ParticleWorldConfig::default()
    };
    let mut world = ParticleWorld::new(config);
    let template = template(&base_json());
    assert_eq!(
        world.spawn_all(
            1,
            &[template.clone(), template.clone(), template],
            Mat4::IDENTITY
        ),
        2
    );
    assert_eq!(world.refused_spawns(), 1);

    world.update(10.0, Vec3::ZERO);
    let elapsed = world.systems().next().unwrap().elapsed_seconds();
    assert!((elapsed - config.max_step_seconds).abs() < 1e-6);
}

#[test]
fn despawn_removes_only_that_owners_systems() {
    let mut world = ParticleWorld::new(ParticleWorldConfig::default());
    let template = template(&base_json());
    world.spawn(1, &template, Mat4::IDENTITY);
    world.spawn(2, &template, Mat4::IDENTITY);
    assert_eq!(world.despawn(1), 1);
    assert_eq!(world.system_count(), 1);
    assert_eq!(world.systems().next().unwrap().owner(), 2);
}

#[test]
fn world_space_particles_stay_put_when_the_owner_moves() {
    let mut system = instance(&single_particle(|_| {}));
    run(&mut system, 0.25, 1);
    let before = system.particles[0].position;
    system.set_transform(Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0)));
    run(&mut system, 0.25, 1);
    assert_eq!(system.particles[0].position, before);
}

#[test]
fn local_space_particles_ride_the_owner_transform() {
    let value = single_particle(|v| v["world_space"] = json!(false));
    let transform = Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0));
    let mut system = SystemInstance::new(1, template(&value), transform, SEED);
    run(&mut system, 0.25, 1);
    let camera = CameraBasis::from_camera_to_world(Mat4::from_translation(Vec3::Z * 10.0));
    let draw = build(&system, &camera).unwrap();
    assert_eq!(draw.model, transform);
    assert!((draw.bounds_centre - Vec3::new(5.0, 0.0, 0.0)).length() < 1e-5);
    assert_eq!(system.particles[0].position, Vec3::ZERO);
}

#[test]
fn box_emitter_fills_its_extents_and_emits_along_local_y() {
    let value = with(base_json(), |v| {
        v["emitters"][0]["width"] = json!(4.0);
        v["emitters"][0]["height"] = json!(2.0);
        v["emitters"][0]["depth"] = json!(6.0);
        v["emitters"][0]["speed"] = json!(3.0);
    });
    let mut system = instance(&value);
    run(&mut system, 0.125, 8);
    for particle in &system.particles {
        // Position includes the fraction of a step the particle has already flown.
        let start = particle.position - particle.velocity * particle.age;
        assert!(start.x.abs() <= 2.0 && start.y.abs() <= 1.0 && start.z.abs() <= 3.0);
        assert!((particle.velocity - Vec3::new(0.0, 3.0, 0.0)).length() < 1e-5);
    }
}

#[test]
fn cylinder_and_sphere_emitters_stay_inside_their_volume() {
    for (kind, inside) in [
        (
            "cylinder",
            (|p: Vec3| p.x.hypot(p.z) <= 2.0 + 1e-4 && p.y.abs() <= 1.0 + 1e-4) as fn(Vec3) -> bool,
        ),
        ("sphere", |p: Vec3| p.length() <= 2.0 + 1e-4),
    ] {
        let value = with(base_json(), |v| {
            v["emitters"][0]["type"] = json!(kind);
            v["emitters"][0]["radius"] = json!(2.0);
            v["emitters"][0]["height"] = json!(2.0);
        });
        let mut system = instance(&value);
        run(&mut system, 0.125, 8);
        assert!(
            system.particles.iter().all(|p| inside(p.position)),
            "{kind}"
        );
    }
}

#[test]
fn declination_and_planar_angle_aim_the_emission() {
    let value = with(base_json(), |v| {
        v["emitters"][0]["speed"] = json!(2.0);
        v["emitters"][0]["declination"] = json!(std::f32::consts::FRAC_PI_2);
        v["emitters"][0]["planar_angle"] = json!(std::f32::consts::FRAC_PI_2);
    });
    let mut system = instance(&value);
    run(&mut system, 0.125, 1);
    // Azimuth runs from +X towards -Z.
    assert!((system.particles[0].velocity - Vec3::new(0.0, 0.0, -2.0)).length() < 1e-5);
}

fn triangle_mesh() -> HashMap<String, Vec<CpuMesh>> {
    let vertex = |position: [f32; 3]| Vertex {
        position,
        tex_coords: [0.0; 2],
        normal: [0.0, 0.0, 1.0],
        tangent: [1.0, 0.0, 0.0, 1.0],
    };
    HashMap::from([(
        "Source".to_owned(),
        vec![CpuMesh {
            topology: Topology::Triangles,
            vertices: vec![
                vertex([0.0, 0.0, 3.0]),
                vertex([2.0, 0.0, 3.0]),
                vertex([0.0, 2.0, 3.0]),
            ],
            indices: vec![0, 1, 2],
            texture_path: None,
            vertex_colors: None,
            effect: None,
            refraction: None,
        }],
    )])
}

fn mesh_value() -> Value {
    with(base_json(), |v| {
        v["emitters"][0]["type"] = json!("mesh");
        v["emitters"][0]["mesh_nodes"] = json!(["Source"]);
        v["emitters"][0]["emission_type"] = json!("face_surface");
        v["emitters"][0]["speed"] = json!(4.0);
    })
}

#[test]
fn mesh_emitter_samples_the_surface_and_flies_along_the_normal() {
    let template = Arc::new(try_template(&mesh_value(), &triangle_mesh()).unwrap());
    let mut system = SystemInstance::new(1, template, Mat4::IDENTITY, SEED);
    run(&mut system, 0.125, 8);
    assert_eq!(system.particle_count(), 30);
    for particle in &system.particles {
        let start = particle.position - particle.velocity * particle.age;
        assert!((start.z - 3.0).abs() < 1e-4);
        assert!(start.x >= -1e-4 && start.y >= -1e-4 && start.x + start.y <= 2.0 + 1e-4);
        assert!((particle.velocity - Vec3::new(0.0, 0.0, 4.0)).length() < 1e-4);
    }
}

fn points_mesh(normal: [f32; 3]) -> HashMap<String, Vec<CpuMesh>> {
    let vertex = |position: [f32; 3]| Vertex {
        position,
        tex_coords: [0.0; 2],
        normal,
        tangent: [1.0, 0.0, 0.0, 1.0],
    };
    HashMap::from([(
        "Source".to_owned(),
        vec![CpuMesh {
            topology: Topology::Points,
            vertices: vec![vertex([1.0, 0.0, 0.0]), vertex([0.0, 5.0, 0.0])],
            indices: vec![0, 1],
            texture_path: None,
            vertex_colors: None,
            effect: None,
            refraction: None,
        }],
    )])
}

fn born_at(particle: &super::instance::Particle) -> Vec3 {
    particle.position - particle.velocity * particle.age
}

#[test]
fn mesh_emitter_may_name_the_system_node_as_its_frame() {
    let value = with(mesh_value(), |v| v["emitters"][0]["node"] = json!("System"));
    let template = try_template(&value, &triangle_mesh());
    assert!(template.is_ok(), "{:?}", template.err());
}

#[test]
fn mesh_emitter_of_a_moved_system_samples_in_the_system_frame() {
    let mut system_json = mesh_value();
    system_json["emitters"][0]["node"] = json!("System");
    let node = NodeExtras {
        node_name: "System".to_owned(),
        transform: Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0)),
        value: parse_versioned("particle_system", &system_json).unwrap(),
    };
    let mut meshes = triangle_mesh();
    // Emitter geometry arrives in model space, as `LoadedModel` bakes node transforms.
    for vertex in &mut meshes.get_mut("Source").unwrap()[0].vertices {
        vertex.position[0] += 10.0;
    }
    let template = ParticleSystemTemplate::new(&node, &nodes(), &meshes, material()).unwrap();
    let transform = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0));
    let mut system = SystemInstance::new(1, Arc::new(template), transform, SEED);
    run(&mut system, 0.125, 8);
    for particle in &system.particles {
        let start = born_at(particle);
        assert!(start.x >= 20.0 - 1e-3 && start.x <= 22.0 + 1e-3, "{start}");
    }
}

#[test]
fn vertex_emission_samples_the_vertices_and_flies_along_their_normals() {
    let value = with(mesh_value(), |v| {
        v["emitters"][0]["emission_type"] = json!("vertices");
    });
    let template = Arc::new(try_template(&value, &points_mesh([0.0, 0.0, 1.0])).unwrap());
    let mut system = SystemInstance::new(1, template, Mat4::IDENTITY, SEED);
    run(&mut system, 0.125, 8);
    assert_eq!(system.particle_count(), 30);
    let (mut first, mut second) = (0, 0);
    for particle in &system.particles {
        let start = born_at(particle);
        if (start - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-4 {
            first += 1;
        } else if (start - Vec3::new(0.0, 5.0, 0.0)).length() < 1e-4 {
            second += 1;
        } else {
            panic!("born away from every vertex: {start}");
        }
        assert!((particle.velocity - Vec3::new(0.0, 0.0, 4.0)).length() < 1e-4);
    }
    assert!(first > 0 && second > 0, "both vertices are used");
}

#[test]
fn vertices_without_normals_fly_along_the_emitter_axis() {
    let value = with(mesh_value(), |v| {
        v["emitters"][0]["emission_type"] = json!("vertices");
    });
    let template = Arc::new(try_template(&value, &points_mesh([0.0; 3])).unwrap());
    let mut system = SystemInstance::new(1, template, Mat4::IDENTITY, SEED);
    run(&mut system, 0.125, 8);
    for particle in &system.particles {
        assert!((particle.velocity - Vec3::new(0.0, 4.0, 0.0)).length() < 1e-4);
    }
}

#[test]
fn face_emission_over_a_point_set_falls_back_to_its_vertices_and_says_so() {
    let template = try_template(&mesh_value(), &points_mesh([0.0, 0.0, 1.0])).unwrap();
    assert!(
        template
            .unsupported_fields
            .iter()
            .any(|field| field.contains("point set")),
        "{:?}",
        template.unsupported_fields
    );
    let faces = try_template(&mesh_value(), &triangle_mesh()).unwrap();
    assert!(faces.unsupported_fields.is_empty());
}

#[test]
fn face_emission_ignores_point_primitives_next_to_triangles() {
    let mut meshes = triangle_mesh();
    meshes
        .get_mut("Source")
        .unwrap()
        .extend(points_mesh([0.0; 3]).remove("Source").unwrap());
    let template = Arc::new(try_template(&mesh_value(), &meshes).unwrap());
    let mut system = SystemInstance::new(1, template, Mat4::IDENTITY, SEED);
    run(&mut system, 0.125, 8);
    for particle in &system.particles {
        assert!(
            (born_at(particle).z - 3.0).abs() < 1e-4,
            "only the triangle emits"
        );
    }
}

#[test]
fn mesh_emitter_without_geometry_is_an_error() {
    let error = try_template(&mesh_value(), &HashMap::new()).unwrap_err();
    assert!(matches!(
        error,
        TemplateError::MeshEmitterWithoutGeometry { .. }
    ));
}

#[test]
fn missing_emitter_node_is_an_error() {
    let value = with(base_json(), |v| v["emitters"][0]["node"] = json!("Nowhere"));
    let error = try_template(&value, &HashMap::new()).unwrap_err();
    assert!(error.to_string().contains("Nowhere"));
}

#[test]
fn emitter_controller_birth_rate_track_drives_the_spawn_rate() {
    let value = with(base_json(), |v| {
        v["emitters"][0]["emitter_ctlr"]["birth_rate"] =
            json!({ "times": [0.0, 1.0], "values": [0.0, 32.0] });
        v["emitters"][0]["emitter_ctlr"]["cycle"] = json!("clamp");
    });
    let mut system = instance(&value);
    run(&mut system, 0.125, 4);
    let first_half = system.particle_count();
    run(&mut system, 0.125, 4);
    let second_half = system.particle_count() - first_half;
    assert!(
        second_half > first_half * 2,
        "{first_half} then {second_half}"
    );
}

#[test]
fn field_track_overrides_the_emitter_speed() {
    let value = with(base_json(), |v| {
        v["tracks"] = json!([{ "target": "Emitter", "field": "speed",
            "track": { "cycle": "clamp", "start": 0.0, "stop": 1.0,
                       "times": [0.0, 1.0], "values": [10.0, 10.0] } }]);
    });
    let mut system = instance(&value);
    run(&mut system, 0.125, 1);
    assert!((system.particles[0].velocity.length() - 10.0).abs() < 1e-5);
}

fn camera_looking_down_minus_z() -> CameraBasis {
    CameraBasis::from_camera_to_world(Mat4::from_translation(Vec3::new(0.0, 0.0, 20.0)))
}

#[test]
fn quads_face_the_camera_with_the_configured_aspect() {
    let value = single_particle(|v| {
        v["data"]["aspect_ratio"] = json!(0.5);
        v["emitters"][0]["initial_radius"] = json!(2.0);
    });
    let mut system = instance(&value);
    run(&mut system, 0.25, 1);
    let draw = build(&system, &camera_looking_down_minus_z()).unwrap();
    assert_eq!(draw.indices, [0, 1, 2, 0, 2, 3]);
    let position = |i: usize| Vec3::from(draw.vertices[i].position);
    // Top-left, bottom-left, bottom-right, top-right; half height 2, half width 1.
    assert!((position(0) - Vec3::new(-1.0, 2.0, 0.0)).length() < 1e-5);
    assert!((position(1) - Vec3::new(-1.0, -2.0, 0.0)).length() < 1e-5);
    assert!((position(2) - Vec3::new(1.0, -2.0, 0.0)).length() < 1e-5);
    assert!((position(3) - Vec3::new(1.0, 2.0, 0.0)).length() < 1e-5);
    assert_eq!(draw.vertices[0].uv, [0.0, 0.0]);
    assert_eq!(draw.vertices[2].uv, [1.0, 1.0]);
    assert_eq!(draw.vertices[0].distortion, 1.0);
    assert_eq!(draw.model, Mat4::IDENTITY);
}

#[test]
fn quads_of_a_rotated_camera_lie_in_its_view_plane() {
    let rotation = Mat4::from_rotation_y(0.7) * Mat4::from_rotation_x(-0.4);
    let camera = CameraBasis::from_camera_to_world(
        Mat4::from_translation(Vec3::new(3.0, 2.0, 9.0)) * rotation,
    );
    let mut system = instance(&single_particle(|_| {}));
    run(&mut system, 0.25, 1);
    let draw = build(&system, &camera).unwrap();
    let centre =
        Vec3::from(draw.vertices[0].position) / 2.0 + Vec3::from(draw.vertices[2].position) / 2.0;
    for vertex in &draw.vertices {
        let offset = Vec3::from(vertex.position) - centre;
        assert!(offset.dot(camera.forward).abs() < 1e-5);
        assert!((offset.length() - 2f32.sqrt()).abs() < 1e-5);
    }
}

#[test]
fn velocity_orientation_stretches_the_quad_along_the_motion() {
    let value = single_particle(|v| {
        v["data"]["aspect_flags"] = json!(1);
        v["data"]["aspect_ratio"] = json!(0.25);
        v["emitters"][0]["speed"] = json!(2.0);
        v["emitters"][0]["declination"] = json!(std::f32::consts::FRAC_PI_2);
    });
    let mut system = instance(&value);
    run(&mut system, 0.25, 1);
    let draw = build(&system, &camera_looking_down_minus_z()).unwrap();
    let position = |i: usize| Vec3::from(draw.vertices[i].position);
    // Velocity along +X: the long side (half height 1) runs along X.
    let along = (position(3) - position(2)).length();
    let across = (position(1) - position(2)).length();
    assert!((position(3) - position(2)).dot(Vec3::X).abs() > 1.9);
    assert!((along - 2.0).abs() < 1e-4 && (across - 0.5).abs() < 1e-4);
}

#[test]
fn speed_to_aspect_blends_with_particle_speed() {
    let at_speed = |speed: f32| {
        let value = single_particle(|v| {
            v["data"]["aspect_flags"] = json!(256);
            v["data"]["aspect_ratio"] = json!(1.0);
            v["data"]["speed_to_aspect"] = json!([3.0, 0.0, 10.0]);
            v["emitters"][0]["speed"] = json!(speed);
            v["emitters"][0]["declination"] = json!(std::f32::consts::FRAC_PI_2);
        });
        let mut system = instance(&value);
        run(&mut system, 0.25, 1);
        let draw = build(&system, &camera_looking_down_minus_z()).unwrap();
        (Vec3::from(draw.vertices[3].position) - Vec3::from(draw.vertices[0].position)).length()
            / 2.0
    };
    assert!((at_speed(0.0) - 1.0).abs() < 1e-4);
    assert!((at_speed(5.0) - 2.0).abs() < 1e-4);
    assert!((at_speed(10.0) - 3.0).abs() < 1e-4);
}

#[test]
fn draw_reports_elapsed_time_atlas_and_world_centre() {
    let rects = json!([[0.0, 0.5, 0.0, 1.0], [0.5, 0.5, 0.0, 1.0]]);
    let value = single_particle(|v| v["data"]["subtexture_rects"] = rects.clone());
    let mut system = instance(&value);
    run(&mut system, 0.25, 2);
    let draw = build(&system, &camera_looking_down_minus_z()).unwrap();
    assert_eq!(draw.elapsed_seconds, 0.5);
    assert_eq!(draw.subtexture_rects.len(), 2);
    assert_eq!(draw.bounds_centre, Vec3::ZERO);
}

#[test]
fn an_empty_system_draws_nothing() {
    let mut system = instance(&with(base_json(), |v| {
        v["emitters"][0]["emitter_ctlr"]["active"] = json!({ "constant": false });
    }));
    run(&mut system, 0.25, 4);
    assert!(build(&system, &camera_looking_down_minus_z()).is_none());
}

#[test]
fn ignored_authored_fields_are_listed_per_template() {
    let value = with(base_json(), |v| {
        v["update"] = json!({ "frequency": 2.0, "phase": 0.5 });
        v["data"]["aspect_flags"] = json!(2);
        v["modifiers"] = json!([
            modifier(&json!({
                "type": "gravity", "object": "GravityNode", "axis": [0, -1, 0], "decay": 0,
                "strength": 1.0, "force": "planar", "turbulence": 3.0,
                "turbulence_scale": 2, "world_aligned": true
            })),
            modifier(&json!({
                "type": "drag", "object": "GravityNode", "axis": [1, 0, 0],
                "percentage": 0.1, "range": null, "range_falloff": 4.0
            })),
        ]);
    });
    let template = template(&value);
    assert_eq!(
        template.unsupported_fields,
        vec![
            "update.frequency/phase",
            "initial_rotation_from_velocity",
            "gravity.turbulence",
            "gravity.turbulence_scale",
            "drag.range_falloff"
        ]
    );
}

#[test]
fn a_plain_system_ignores_nothing() {
    assert!(template(&base_json()).unsupported_fields.is_empty());
}

#[test]
fn a_scaled_placement_scales_speeds_accelerations_and_positions_alike() {
    let value = single_particle(|v| {
        v["emitters"][0]["speed"] = json!(2.0);
        v["modifiers"] = json!([modifier(&json!({
            "type": "gravity", "object": "GravityNode", "axis": [0, -1, 0], "decay": 0,
            "strength": 8.0, "force": "planar", "turbulence": 0,
            "turbulence_scale": 1, "world_aligned": true
        }))]);
    });
    let simulate = |scale: f32| {
        let mut system = SystemInstance::new(
            1,
            template(&value),
            Mat4::from_scale(Vec3::splat(scale)),
            SEED,
        );
        run(&mut system, 0.25, 4);
        let [particle] = system.particles.as_slice() else {
            panic!("expected exactly one particle");
        };
        (particle.position, particle.velocity, particle.radius)
    };
    let (position, velocity, radius) = simulate(1.0);
    let (scaled_position, scaled_velocity, scaled_radius) = simulate(2.0);
    assert!((scaled_position - position * 2.0).length() < 1e-3);
    assert!((scaled_velocity - velocity * 2.0).length() < 1e-3);
    assert!((scaled_radius - radius * 2.0).abs() < 1e-6);
}

const VIEW_HALF_WIDTH: f32 = 10.0;

fn view_planes() -> [Vec4; 6] {
    clip_planes(&Mat4::orthographic_rh(
        -VIEW_HALF_WIDTH,
        VIEW_HALF_WIDTH,
        -VIEW_HALF_WIDTH,
        VIEW_HALF_WIDTH,
        -50.0,
        50.0,
    ))
}

fn resting_particle_at(x: f32) -> SystemInstance {
    let value = single_particle(|v| v["emitters"][0]["speed"] = json!(0.0));
    let mut system = SystemInstance::new(
        1,
        template(&value),
        Mat4::from_translation(Vec3::X * x),
        SEED,
    );
    run(&mut system, 0.25, 1);
    system
}

#[test]
fn a_system_inside_the_view_is_built() {
    let system = resting_particle_at(0.0);
    assert!(build_within(&system, &camera_looking_down_minus_z(), &view_planes()).is_some());
}

#[test]
fn a_system_far_outside_the_view_builds_no_quads() {
    let system = resting_particle_at(100.0);
    assert!(!system.is_in_view(&view_planes()));
    assert!(build_within(&system, &camera_looking_down_minus_z(), &view_planes()).is_none());
}

#[test]
fn a_particle_whose_quad_reaches_into_the_view_is_not_culled() {
    let quad_half_diagonal = 2f32.sqrt();
    let reaching = resting_particle_at(VIEW_HALF_WIDTH + quad_half_diagonal - 0.1);
    let missing = resting_particle_at(VIEW_HALF_WIDTH + quad_half_diagonal + 0.1);
    assert!(reaching.is_in_view(&view_planes()));
    assert!(!missing.is_in_view(&view_planes()));
}
