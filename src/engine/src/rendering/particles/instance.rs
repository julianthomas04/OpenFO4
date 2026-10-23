//! One running particle system: emission, forces and per-particle state.

use std::sync::Arc;

use glam::{Mat4, Vec3};

use crate::assets::types::particle_system::{
    EmissionType, Emitter, EmitterKind, GravityForce, ModifierKind, TrackField,
};

use super::constants::{
    DRAG_RATE_PER_SECOND, LOD_REFERENCE_DISTANCE, RADIUS_TO_HALF_EXTENT,
    SPHERICAL_GRAVITY_ATTRACTS, WIND_ACCELERATION_PER_STRENGTH,
};
use super::curves::{Flipbook, ramp, scale_at, simple_colour};
use super::rng::ParticleRng;
use super::template::{EmitterSetup, ParticleSystemTemplate, SurfacePick, TrackTarget};

/// Shortest particle life, so a zero life span never divides by zero.
const MIN_LIFE_SECONDS: f32 = 1e-3;

#[derive(Debug, Clone)]
pub(super) struct Particle {
    pub position: Vec3,
    pub velocity: Vec3,
    pub age: f32,
    pub life: f32,
    /// Quad half-extent before the scale and LOD factors.
    pub radius: f32,
    pub initial_colour: [f32; 4],
    pub colour: [f32; 4],
    pub scale: f32,
    pub cell: usize,
    pub roll_angle: f32,
    pub roll_speed: f32,
    pub wind_direction: Vec3,
    pub flipbook: Flipbook,
}

/// The transforms a system needs between its spaces, rebuilt every update.
#[derive(Debug, Clone, Copy)]
pub(super) struct Frames {
    pub system_to_world: Mat4,
    pub sim_to_world: Mat4,
    pub sim_from_world: Mat4,
    pub sim_from_system: Mat4,
    /// Uniform scale of the owner's placement as world-space particles see it. Radii, speeds,
    /// accelerations and distances authored in model units are multiplied by it, so a scaled
    /// placement behaves like a scaled copy of the unscaled one. 1 for local-space systems:
    /// their model matrix scales everything on the way to the screen.
    pub radius_scale: f32,
}

impl Frames {
    fn new(world_space: bool, instance: Mat4, system_model: Mat4) -> Self {
        let system_to_world = instance * system_model;
        let sim_to_world = if world_space {
            Mat4::IDENTITY
        } else {
            system_to_world
        };
        let sim_from_world = sim_to_world.inverse();
        Self {
            system_to_world,
            sim_to_world,
            sim_from_world,
            sim_from_system: sim_from_world * system_to_world,
            radius_scale: if world_space {
                system_to_world.x_axis.truncate().length()
            } else {
                1.0
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Force {
    Accelerate(Vec3),
    Attract {
        position: Vec3,
        strength: f32,
        decay: f32,
    },
    Drag {
        axis: Vec3,
        rate: f32,
        centre: Vec3,
        range: Option<f32>,
    },
    Wind(f32),
}

#[derive(Debug, Default, Clone, Copy)]
struct Gates {
    colour: bool,
    scale: bool,
    sub_tex: bool,
}

#[derive(Debug, Default)]
struct EmitterState {
    accumulator: f32,
    previous_origin: Option<Vec3>,
    velocity_world: Vec3,
}

struct EmitterParams {
    speed: f32,
    life_span: f32,
    initial_radius: f32,
    declination: f32,
    declination_variation: f32,
    planar_angle: f32,
}

#[derive(Debug)]
pub struct SystemInstance {
    owner: u64,
    template: Arc<ParticleSystemTemplate>,
    transform: Mat4,
    rng: ParticleRng,
    elapsed: f32,
    pub(super) particles: Vec<Particle>,
    emitters: Vec<EmitterState>,
    forces: Vec<Force>,
    gates: Gates,
    lod_emit_scale: f32,
    lod_size_scale: f32,
    camera_distance: f32,
    pub(super) frames: Frames,
}

impl SystemInstance {
    pub(super) fn new(
        owner: u64,
        template: Arc<ParticleSystemTemplate>,
        transform: Mat4,
        seed: u64,
    ) -> Self {
        let frames = Frames::new(
            template.system.world_space,
            transform,
            template.system_model,
        );
        let emitters = template
            .emitters
            .iter()
            .map(|_| EmitterState::default())
            .collect();
        let capacity = template.max_particles().min(64);
        Self {
            owner,
            transform,
            rng: ParticleRng::new(seed),
            elapsed: 0.0,
            particles: Vec::with_capacity(capacity),
            emitters,
            forces: Vec::new(),
            gates: Gates::default(),
            lod_emit_scale: 1.0,
            lod_size_scale: 1.0,
            camera_distance: 0.0,
            frames,
            template,
        }
    }

    pub fn owner(&self) -> u64 {
        self.owner
    }

    pub fn name(&self) -> &str {
        &self.template.name
    }

    pub fn particle_count(&self) -> usize {
        self.particles.len()
    }

    pub fn elapsed_seconds(&self) -> f32 {
        self.elapsed
    }

    pub(super) fn template(&self) -> &ParticleSystemTemplate {
        &self.template
    }

    pub(super) fn lod_size_scale(&self) -> f32 {
        self.lod_size_scale
    }

    pub fn set_transform(&mut self, transform: Mat4) {
        self.transform = transform;
        self.frames = Frames::new(
            self.template.system.world_space,
            transform,
            self.template.system_model,
        );
    }

    /// Alpha factor from the system's near and far camera-distance fades.
    pub(super) fn fade_alpha(&self) -> f32 {
        let fade = &self.template.system.fade;
        let distance = self.camera_distance;
        let mut alpha = 1.0;
        if let Some((low, high)) = fade_range(fade.near_begin, fade.near_end) {
            alpha *= ramp(distance, low, high);
        }
        if let Some((low, high)) = fade_range(fade.far_begin, fade.far_end) {
            alpha *= 1.0 - ramp(distance, low, high);
        }
        alpha
    }

    pub fn update(&mut self, dt: f32, camera_position: Vec3) {
        self.elapsed += dt;
        self.frames = Frames::new(
            self.template.system.world_space,
            self.transform,
            self.template.system_model,
        );
        self.camera_distance =
            (camera_position - self.frames.system_to_world.w_axis.truncate()).length();
        self.refresh_gates();
        self.refresh_lod();
        self.refresh_forces();
        self.track_emitter_velocity(dt);
        self.integrate(dt);
        self.emit(dt);
        self.refresh_derived();
    }

    fn modifier_active(&self, index: usize) -> bool {
        let modifier = &self.template.modifiers[index];
        modifier.active
            && modifier
                .active_track
                .as_ref()
                .is_none_or(|track| track.evaluate(self.elapsed))
    }

    fn refresh_gates(&mut self) {
        let index = &self.template.index;
        let active = |slot: Option<usize>| slot.is_some_and(|i| self.modifier_active(i));
        self.gates = Gates {
            colour: active(index.simple_color),
            scale: active(index.scale),
            sub_tex: active(index.sub_tex),
        };
    }

    fn refresh_lod(&mut self) {
        self.lod_emit_scale = 1.0;
        self.lod_size_scale = 1.0;
        let Some(slot) = self.template.index.lod.filter(|&i| self.modifier_active(i)) else {
            return;
        };
        let ModifierKind::Lod {
            begin,
            end,
            end_emit_scale,
            end_size,
        } = self.template.modifiers[slot].kind
        else {
            return;
        };
        let t = ramp(self.camera_distance / LOD_REFERENCE_DISTANCE, begin, end);
        self.lod_emit_scale = (end_emit_scale - 1.0).mul_add(t, 1.0);
        self.lod_size_scale = (end_size - 1.0).mul_add(t, 1.0);
    }

    /// Value of `field` for the emitter or modifier at `target`, after tracks.
    fn tracked(&self, target: TrackTarget, field: TrackField, base: f32) -> f32 {
        self.template
            .tracks
            .iter()
            .rev()
            .find(|track| track.target == target && track.field == field)
            .map_or(base, |track| track.track.evaluate(self.elapsed))
    }

    fn refresh_forces(&mut self) {
        self.forces.clear();
        let frames = self.frames;
        let scale = frames.radius_scale;
        for index in 0..self.template.modifiers.len() {
            if !self.modifier_active(index) {
                continue;
            }
            let template = Arc::clone(&self.template);
            let object = template.object_frames[index];
            let direction = |axis: [f32; 3], world_aligned: bool| {
                let axis = Vec3::from(axis);
                let sim = if world_aligned {
                    frames.sim_from_world.transform_vector3(axis)
                } else {
                    let local = object.map_or(axis, |frame| frame.transform_vector3(axis));
                    frames.sim_from_system.transform_vector3(local)
                };
                sim.normalize_or_zero()
            };
            let position = object.map(|frame| {
                frames
                    .sim_from_system
                    .transform_point3(frame.w_axis.truncate())
            });
            match &template.modifiers[index].kind {
                ModifierKind::Gravity {
                    axis,
                    decay,
                    strength,
                    force,
                    world_aligned,
                    ..
                } => {
                    let strength = self.tracked(
                        TrackTarget::Modifier(index),
                        TrackField::GravityStrength,
                        *strength,
                    );
                    self.forces.push(match force {
                        GravityForce::Planar => {
                            Force::Accelerate(direction(*axis, *world_aligned) * strength * scale)
                        }
                        GravityForce::Spherical => Force::Attract {
                            position: position.unwrap_or(Vec3::ZERO),
                            strength: scale
                                * if SPHERICAL_GRAVITY_ATTRACTS {
                                    strength
                                } else {
                                    -strength
                                },
                            decay: *decay / scale,
                        },
                    });
                }
                ModifierKind::Drag {
                    axis,
                    percentage,
                    range,
                    ..
                } => self.forces.push(Force::Drag {
                    axis: direction(*axis, false),
                    rate: percentage * DRAG_RATE_PER_SECOND,
                    centre: position.unwrap_or(Vec3::ZERO),
                    range: range.map(|range| range * scale),
                }),
                ModifierKind::Wind { strength } => self.forces.push(Force::Wind(
                    strength * WIND_ACCELERATION_PER_STRENGTH * scale,
                )),
                _ => {}
            }
        }
    }

    fn track_emitter_velocity(&mut self, dt: f32) {
        for (state, setup) in self.emitters.iter_mut().zip(&self.template.emitters) {
            let origin = self
                .frames
                .system_to_world
                .transform_point3(setup.to_system.w_axis.truncate());
            state.velocity_world = match state.previous_origin {
                Some(previous) if dt > 0.0 => (origin - previous) / dt,
                _ => Vec3::ZERO,
            };
            state.previous_origin = Some(origin);
        }
    }

    fn integrate(&mut self, dt: f32) {
        let forces = &self.forces;
        let mut index = 0;
        while index < self.particles.len() {
            let particle = &mut self.particles[index];
            particle.age += dt;
            if particle.age >= particle.life {
                self.particles.swap_remove(index);
                continue;
            }
            for force in forces {
                apply_force(particle, *force, dt);
            }
            particle.position += particle.velocity * dt;
            index += 1;
        }
    }

    fn emitter_params(&self, index: usize, spec: &EmitterSetup) -> EmitterParams {
        let target = TrackTarget::Emitter(index);
        let spec = &spec.spec;
        EmitterParams {
            speed: self.tracked(target, TrackField::Speed, spec.speed),
            life_span: self.tracked(target, TrackField::LifeSpan, spec.life_span),
            initial_radius: self.tracked(target, TrackField::InitialRadius, spec.initial_radius),
            declination: self.tracked(target, TrackField::Declination, spec.declination),
            declination_variation: self.tracked(
                target,
                TrackField::DeclinationVariation,
                spec.declination_variation,
            ),
            planar_angle: self.tracked(target, TrackField::PlanarAngle, spec.planar_angle),
        }
    }

    fn emit(&mut self, dt: f32) {
        let template = Arc::clone(&self.template);
        let capacity = template.max_particles();
        for (index, setup) in template.emitters.iter().enumerate() {
            let ctlr = &setup.spec.emitter_ctlr;
            if !ctlr.active_at(self.elapsed) {
                continue;
            }
            let rate = ctlr.birth_rate_at(self.elapsed).max(0.0) * self.lod_emit_scale;
            let state = &mut self.emitters[index];
            state.accumulator += rate * dt;
            let due = state.accumulator.floor();
            state.accumulator -= due;
            let room = capacity.saturating_sub(self.particles.len());
            let count = (due as usize).min(room);
            if count == 0 {
                continue;
            }
            let params = self.emitter_params(index, setup);
            for sequence in 0..count {
                let age = dt * (1.0 - (sequence as f32 + 0.5) / count as f32);
                self.spawn(index, setup, &params, age);
            }
        }
    }

    fn spawn(&mut self, emitter: usize, setup: &EmitterSetup, params: &EmitterParams, age: f32) {
        let spec = &setup.spec;
        let (point, direction) = self.emission_point_and_direction(setup, params);
        let to_sim = if setup.surface.is_some() {
            self.frames.sim_from_system
        } else {
            self.frames.sim_from_system * setup.to_system
        };
        let position = to_sim.transform_point3(point);
        let direction = to_sim.transform_vector3(direction).normalize_or_zero();
        let speed = self.rng.spread(params.speed, spec.speed_variation) * self.frames.radius_scale;
        let mut velocity = direction * speed;
        velocity += self.inherited_velocity(emitter);

        let life = self
            .rng
            .spread(params.life_span, spec.life_span_variation)
            .max(MIN_LIFE_SECONDS);
        let radius = self
            .rng
            .spread(params.initial_radius, spec.radius_variation)
            .max(0.0)
            * RADIUS_TO_HALF_EXTENT
            * self.frames.radius_scale;
        let (roll_angle, roll_speed) = self.initial_roll();
        let flipbook = self.initial_flipbook();
        let wind_angle = self.rng.unit() * std::f32::consts::TAU;
        self.particles.push(Particle {
            position: position + velocity * age,
            velocity,
            age,
            life,
            radius,
            initial_colour: spec.initial_color,
            colour: spec.initial_color,
            scale: 1.0,
            cell: 0,
            roll_angle,
            roll_speed,
            wind_direction: Vec3::new(wind_angle.cos(), 0.0, wind_angle.sin()),
            flipbook,
        });
    }

    /// Spawn point and direction in the emitter node's frame (mesh emitters:
    /// the system node's frame).
    fn emission_point_and_direction(
        &mut self,
        setup: &EmitterSetup,
        params: &EmitterParams,
    ) -> (Vec3, Vec3) {
        let spec = &setup.spec;
        let rng = &mut self.rng;
        if let (EmitterKind::Mesh, Some(surface)) = (spec.kind, &setup.surface) {
            match surface.pick(rng.unit()) {
                SurfacePick::Vertex(vertex) => {
                    let direction = if vertex.normal == Vec3::ZERO {
                        let axis = emitter_direction(rng, spec, params);
                        setup.to_system.transform_vector3(axis)
                    } else {
                        vertex.normal
                    };
                    (vertex.position, direction)
                }
                SurfacePick::Triangle(triangle) => {
                    let [a, b, c] = triangle.corners;
                    let point = if spec.emission_type == Some(EmissionType::FaceCentre) {
                        (a + b + c) / 3.0
                    } else {
                        let (mut u, mut v) = (rng.unit(), rng.unit());
                        if u + v > 1.0 {
                            (u, v) = (1.0 - u, 1.0 - v);
                        }
                        a + (b - a) * u + (c - a) * v
                    };
                    (point, triangle.normal)
                }
            }
        } else {
            let point = match spec.kind {
                EmitterKind::Box => Vec3::new(
                    (rng.unit() - 0.5) * spec.width,
                    (rng.unit() - 0.5) * spec.height,
                    (rng.unit() - 0.5) * spec.depth,
                ),
                EmitterKind::Sphere => random_direction(rng) * spec.radius * rng.unit().cbrt(),
                EmitterKind::Cylinder => {
                    let angle = rng.unit() * std::f32::consts::TAU;
                    let radius = spec.radius * rng.unit().sqrt();
                    Vec3::new(
                        radius * angle.cos(),
                        (rng.unit() - 0.5) * spec.height,
                        -radius * angle.sin(),
                    )
                }
                EmitterKind::Mesh => Vec3::ZERO,
            };
            (point, emitter_direction(rng, spec, params))
        }
    }

    fn inherited_velocity(&mut self, emitter: usize) -> Vec3 {
        let Some(slot) = self
            .template
            .index
            .inherit_velocity
            .filter(|&i| self.modifier_active(i))
        else {
            return Vec3::ZERO;
        };
        let ModifierKind::InheritVelocity {
            chance_percent,
            multiplier,
            variation,
            ..
        } = self.template.modifiers[slot].kind
        else {
            return Vec3::ZERO;
        };
        if !self.rng.chance(chance_percent / 100.0) {
            return Vec3::ZERO;
        }
        let factor = self.rng.spread(multiplier, variation);
        self.frames
            .sim_from_world
            .transform_vector3(self.emitters[emitter].velocity_world)
            * factor
    }

    fn initial_roll(&mut self) -> (f32, f32) {
        let Some(slot) = self
            .template
            .index
            .rotation
            .filter(|&i| self.modifier_active(i))
        else {
            return (0.0, 0.0);
        };
        let ModifierKind::Rotation {
            speed,
            speed_variation,
            angle,
            angle_variation,
            random_speed_sign,
            ..
        } = self.template.modifiers[slot].kind
        else {
            return (0.0, 0.0);
        };
        let target = TrackTarget::Modifier(slot);
        let speed = self.tracked(target, TrackField::RotationSpeed, speed);
        let speed_variation =
            self.tracked(target, TrackField::RotationSpeedVariation, speed_variation);
        let mut roll_speed = self.rng.spread(speed, speed_variation);
        if random_speed_sign && self.rng.chance(0.5) {
            roll_speed = -roll_speed;
        }
        (self.rng.spread(angle, angle_variation), roll_speed)
    }

    fn initial_flipbook(&mut self) -> Flipbook {
        let rng = &mut self.rng;
        self.template
            .index
            .sub_tex
            .and_then(|slot| {
                Flipbook::from_kind(&self.template.modifiers[slot].kind, || rng.unit())
            })
            .unwrap_or_default()
    }

    fn refresh_derived(&mut self) {
        let template = Arc::clone(&self.template);
        let colour_kind = template
            .index
            .simple_color
            .map(|i| &template.modifiers[i].kind);
        let scales = template
            .index
            .scale
            .and_then(|i| match &template.modifiers[i].kind {
                ModifierKind::Scale { scales } => Some(scales.as_slice()),
                _ => None,
            });
        let end_frame = template
            .index
            .sub_tex
            .map(|i| match template.modifiers[i].kind {
                ModifierKind::SubTex { end_frame, .. } => end_frame,
                _ => 0.0,
            });
        let cells = template.system.data.subtexture_rects.len();
        let gates = self.gates;
        for particle in &mut self.particles {
            let life = particle.age / particle.life;
            particle.colour = match colour_kind.filter(|_| gates.colour) {
                Some(kind) => {
                    let keyed = simple_colour(kind, life).unwrap_or([1.0; 4]);
                    std::array::from_fn(|i| keyed[i] * particle.initial_colour[i])
                }
                None => particle.initial_colour,
            };
            particle.scale = scales
                .filter(|_| gates.scale)
                .map_or(1.0, |scales| scale_at(scales, life));
            particle.cell = end_frame
                .filter(|_| gates.sub_tex)
                .map_or(0, |end| particle.flipbook.cell(end, particle.age, cells));
        }
    }
}

/// A random direction in the emitter node's frame from the declination and planar angle.
fn emitter_direction(rng: &mut ParticleRng, spec: &Emitter, params: &EmitterParams) -> Vec3 {
    let declination = rng.spread(params.declination, params.declination_variation);
    let planar = rng.spread(params.planar_angle, spec.planar_angle_variation);
    Vec3::new(
        declination.sin() * planar.cos(),
        declination.cos(),
        -declination.sin() * planar.sin(),
    )
}

fn fade_range(begin: f32, end: f32) -> Option<(f32, f32)> {
    (begin.max(end) > 0.0).then(|| (begin.min(end), begin.max(end)))
}

fn random_direction(rng: &mut ParticleRng) -> Vec3 {
    let y = rng.signed();
    let angle = rng.unit() * std::f32::consts::TAU;
    let ring = (1.0 - y * y).sqrt();
    Vec3::new(ring * angle.cos(), y, ring * angle.sin())
}

fn apply_force(particle: &mut Particle, force: Force, dt: f32) {
    match force {
        Force::Accelerate(acceleration) => particle.velocity += acceleration * dt,
        Force::Attract {
            position,
            strength,
            decay,
        } => {
            let offset = position - particle.position;
            let falloff = (-decay * offset.length()).exp();
            particle.velocity += offset.normalize_or_zero() * strength * falloff * dt;
        }
        Force::Drag {
            axis,
            rate,
            centre,
            range,
        } => {
            if range.is_some_and(|range| (particle.position - centre).length() > range) {
                return;
            }
            let along = particle.velocity.dot(axis);
            particle.velocity -= axis * along * (1.0 - (-rate * dt).exp());
        }
        Force::Wind(strength) => particle.velocity += particle.wind_direction * strength * dt,
    }
}
