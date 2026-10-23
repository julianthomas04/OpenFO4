//! All running particle systems of the scene, keyed by the model instance
//! that owns them.

use std::sync::Arc;

use glam::{Mat4, Vec3, Vec4};

use super::constants::{MAX_ACTIVE_SYSTEMS, MAX_STEP_SECONDS};
use super::instance::SystemInstance;
use super::quads::CameraBasis;
use super::template::ParticleSystemTemplate;
use crate::rendering::effects::particle_draw::ParticleDrawPool;

/// Owner of a group of systems (a spawned model instance or addon node).
pub type InstanceId = u64;

#[derive(Debug, Clone, Copy)]
pub struct ParticleWorldConfig {
    pub seed: u64,
    pub max_systems: usize,
    pub max_step_seconds: f32,
}

impl Default for ParticleWorldConfig {
    fn default() -> Self {
        Self {
            seed: 0x0FF0_4F04,
            max_systems: MAX_ACTIVE_SYSTEMS,
            max_step_seconds: MAX_STEP_SECONDS,
        }
    }
}

#[derive(Debug)]
pub struct ParticleWorld {
    config: ParticleWorldConfig,
    systems: Vec<SystemInstance>,
    spawned: u64,
    refused: u64,
}

impl ParticleWorld {
    pub fn new(config: ParticleWorldConfig) -> Self {
        Self {
            config,
            systems: Vec::new(),
            spawned: 0,
            refused: 0,
        }
    }

    /// Starts one system for `owner` at `transform` (the owner's model-to-world
    /// matrix). Returns `false` when the system cap is reached.
    pub fn spawn(
        &mut self,
        owner: InstanceId,
        template: &Arc<ParticleSystemTemplate>,
        transform: Mat4,
    ) -> bool {
        if self.systems.len() >= self.config.max_systems {
            self.refused += 1;
            return false;
        }
        let seed = self
            .config
            .seed
            .wrapping_add(self.spawned.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        self.spawned += 1;
        self.systems.push(SystemInstance::new(
            owner,
            Arc::clone(template),
            transform,
            seed,
        ));
        true
    }

    /// Starts the templates for `owner` in order until one is refused; returns how many were
    /// accepted, so the accepted ones are always a prefix of `templates`.
    pub fn spawn_all(
        &mut self,
        owner: InstanceId,
        templates: &[Arc<ParticleSystemTemplate>],
        transform: Mat4,
    ) -> usize {
        templates
            .iter()
            .take_while(|template| self.spawn(owner, template, transform))
            .count()
    }

    /// Moves every system of `owner` (world-space particles stay where they are).
    pub fn set_transform(&mut self, owner: InstanceId, transform: Mat4) {
        for system in self.systems.iter_mut().filter(|s| s.owner() == owner) {
            system.set_transform(transform);
        }
    }

    /// Removes every system of `owner`; returns how many were removed.
    pub fn despawn(&mut self, owner: InstanceId) -> usize {
        let before = self.systems.len();
        self.systems.retain(|system| system.owner() != owner);
        before - self.systems.len()
    }

    /// Advances all systems by `dt` seconds, clamped to the configured step.
    pub fn update(&mut self, dt: f32, camera_position: Vec3) {
        let dt = dt.clamp(0.0, self.config.max_step_seconds);
        if dt == 0.0 {
            return;
        }
        for system in &mut self.systems {
            system.update(dt, camera_position);
        }
    }

    /// Appends one draw per visible system inside the view bounded by `planes` to `pool`.
    pub fn collect_draws(
        &self,
        camera: &CameraBasis,
        planes: &[Vec4],
        pool: &mut ParticleDrawPool,
    ) {
        for system in &self.systems {
            system.build_draw(camera, planes, pool);
        }
    }

    pub fn system_count(&self) -> usize {
        self.systems.len()
    }

    pub fn particle_count(&self) -> usize {
        self.systems
            .iter()
            .map(SystemInstance::particle_count)
            .sum()
    }

    /// Spawns refused because the system cap was reached.
    pub fn refused_spawns(&self) -> u64 {
        self.refused
    }

    pub fn systems(&self) -> impl Iterator<Item = &SystemInstance> {
        self.systems.iter()
    }
}
