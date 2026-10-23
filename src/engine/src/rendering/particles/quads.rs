//! Camera-facing quad expansion of a system's particles.

use glam::{Mat4, Vec2, Vec3, Vec4};

use crate::assets::types::particle_system::aspect_flags;
use crate::rendering::effects::particle_draw::ParticleDrawPool;
use crate::rendering::effects::shader::ParticleVertex;
use crate::rendering::frustum::Aabb;

use super::curves::ramp;
use super::instance::{Particle, SystemInstance};

/// Screen-space velocity shorter than this does not orient a quad.
const MIN_ORIENTING_SPEED: f32 = 1e-4;

/// Corner offsets in units of (right, up) and their quad uv, top-left first.
const CORNERS: [([f32; 2], [f32; 2]); 4] = [
    ([-1.0, 1.0], [0.0, 0.0]),
    ([-1.0, -1.0], [0.0, 1.0]),
    ([1.0, -1.0], [1.0, 1.0]),
    ([1.0, 1.0], [1.0, 0.0]),
];

/// The camera's orientation in engine world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraBasis {
    pub position: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub forward: Vec3,
}

impl CameraBasis {
    /// From a camera-to-world matrix whose camera looks down local -Z.
    pub fn from_camera_to_world(camera_to_world: Mat4) -> Self {
        Self {
            position: camera_to_world.w_axis.truncate(),
            right: camera_to_world.x_axis.truncate().normalize_or_zero(),
            up: camera_to_world.y_axis.truncate().normalize_or_zero(),
            forward: -camera_to_world.z_axis.truncate().normalize_or_zero(),
        }
    }
}

/// The two in-plane axes a quad's half-extents run along.
struct QuadAxes {
    right: Vec3,
    up: Vec3,
}

fn rolled_axes(right: Vec3, up: Vec3, angle: f32) -> QuadAxes {
    let (sin, cos) = angle.sin_cos();
    QuadAxes {
        right: right * cos + up * sin,
        up: up * cos - right * sin,
    }
}

/// Axes with the quad's height along the particle's on-screen velocity.
fn velocity_axes(right: Vec3, up: Vec3, velocity: Vec3) -> Option<QuadAxes> {
    let screen = Vec2::new(velocity.dot(right), velocity.dot(up));
    (screen.length() > MIN_ORIENTING_SPEED).then(|| {
        let direction = screen.normalize();
        QuadAxes {
            right: right * direction.y - up * direction.x,
            up: right * direction.x + up * direction.y,
        }
    })
}

fn colour_byte(channel: f32) -> u8 {
    (channel.clamp(0.0, 1.0) * 255.0).round() as u8
}

impl SystemInstance {
    /// World-space box that holds every quad of the live particles, `None` when there are none.
    /// Each quad reaches at most its half-diagonal beyond its particle position.
    pub fn quad_bounds(&self) -> Option<Aabb> {
        let data = &self.template().system.data;
        let widest_aspect = data
            .has_flag(aspect_flags::SPEED_TO_ASPECT)
            .then(|| data.speed_to_aspect[0])
            .map_or(data.aspect_ratio, |end_aspect| {
                end_aspect.max(data.aspect_ratio)
            })
            .max(0.0);
        let lod = self.lod_size_scale();
        let reach = |particle: &Particle| {
            let half_height = particle.radius * particle.scale * lod;
            half_height * widest_aspect.hypot(1.0)
        };
        let mut particles = self.particles.iter();
        let first = particles.next()?;
        let (mut min, mut max, mut largest_reach) = (first.position, first.position, reach(first));
        for particle in particles {
            min = min.min(particle.position);
            max = max.max(particle.position);
            largest_reach = largest_reach.max(reach(particle));
        }
        let bounds = Aabb {
            min: min - Vec3::splat(largest_reach),
            max: max + Vec3::splat(largest_reach),
        };
        Some(bounds.transformed(&self.frames.sim_to_world))
    }

    /// Whether any quad of the system can reach the view volume bounded by `planes`.
    pub fn is_in_view(&self, planes: &[Vec4]) -> bool {
        self.quad_bounds()
            .is_some_and(|bounds| !bounds.is_outside(planes))
    }

    /// Expands the live particles of a system that is in view into camera-facing quads and
    /// appends the draw to `pool`; returns whether one was added. Call after
    /// [`SystemInstance::update`].
    pub fn build_draw(
        &self,
        camera: &CameraBasis,
        planes: &[Vec4],
        pool: &mut ParticleDrawPool,
    ) -> bool {
        let fade = self.fade_alpha();
        if self.particles.is_empty() || fade <= 0.0 || !self.is_in_view(planes) {
            return false;
        }
        let template = self.template();
        let data = &template.system.data;
        let frames = &self.frames;
        let right = frames
            .sim_from_world
            .transform_vector3(camera.right)
            .normalize_or_zero();
        let up = frames
            .sim_from_world
            .transform_vector3(camera.up)
            .normalize_or_zero();
        let speed_aspect = data
            .has_flag(aspect_flags::SPEED_TO_ASPECT)
            .then_some(data.speed_to_aspect);
        let oriented = data.has_flag(aspect_flags::VELOCITY_ORIENTATION);

        pool.push_with(&template.material, |draw| {
            draw.vertices.reserve(self.particles.len() * 4);
            draw.indices.reserve(self.particles.len() * 6);
            let mut centroid = Vec3::ZERO;
            for particle in &self.particles {
                let aspect = match speed_aspect {
                    Some([aspect2, slow, fast]) => {
                        let t = ramp(particle.velocity.length(), slow, fast);
                        (aspect2 - data.aspect_ratio).mul_add(t, data.aspect_ratio)
                    }
                    None => data.aspect_ratio,
                };
                let axes = oriented
                    .then(|| velocity_axes(right, up, particle.velocity))
                    .flatten()
                    .unwrap_or_else(|| {
                        rolled_axes(
                            right,
                            up,
                            particle.roll_angle + particle.roll_speed * particle.age,
                        )
                    });
                let first = draw.vertices.len() as u32;
                push_quad(
                    &mut draw.vertices,
                    particle,
                    &axes,
                    aspect,
                    self.lod_size_scale(),
                    fade,
                );
                draw.indices
                    .extend([0, 1, 2, 0, 2, 3].map(|corner| first + corner));
                centroid += particle.position;
            }
            centroid /= self.particles.len() as f32;
            draw.elapsed_seconds = self.elapsed_seconds();
            draw.model = frames.sim_to_world;
            draw.subtexture_rects
                .extend_from_slice(&data.subtexture_rects);
            draw.bounds_centre = frames.sim_to_world.transform_point3(centroid);
        });
        true
    }
}

fn push_quad(
    vertices: &mut Vec<ParticleVertex>,
    particle: &Particle,
    axes: &QuadAxes,
    aspect: f32,
    lod_size: f32,
    fade: f32,
) {
    let half_height = particle.radius * particle.scale * lod_size;
    let half_width = half_height * aspect;
    let colour = [
        colour_byte(particle.colour[0]),
        colour_byte(particle.colour[1]),
        colour_byte(particle.colour[2]),
        colour_byte(particle.colour[3] * fade),
    ];
    for ([across, along], uv) in CORNERS {
        let position = particle.position
            + axes.right * (across * half_width)
            + axes.up * (along * half_height);
        vertices.push(ParticleVertex {
            position: position.to_array(),
            sub_texture: particle.cell as f32,
            uv,
            distortion: 1.0,
            colour,
        });
    }
}
