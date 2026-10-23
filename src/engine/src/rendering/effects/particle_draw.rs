//! One frame's worth of simulated particle quads, handed from the particle system to the effect stage.

use std::sync::Arc;

use glam::{Mat4, Vec3};

use super::shader::ParticleVertex;
use crate::assets::types::effect_material::EffectMaterial;

pub struct ParticleDraw {
    pub vertices: Vec<ParticleVertex>,
    pub indices: Vec<u32>,
    pub material: Arc<EffectMaterial>,
    pub elapsed_seconds: f32,
    pub model: Mat4,
    pub subtexture_rects: Vec<[f32; 4]>,
    /// Centroid of the live particles in engine world space (already through `model`).
    pub bounds_centre: Vec3,
}

/// Allocations of a retired draw, kept without its material so a cleared pool
/// does not delay material and texture eviction.
#[derive(Default)]
struct SpareBuffers {
    vertices: Vec<ParticleVertex>,
    indices: Vec<u32>,
    subtexture_rects: Vec<[f32; 4]>,
}

/// The frame's particle draws; the buffers of earlier frames' draws are reused.
#[derive(Default)]
pub struct ParticleDrawPool {
    live: Vec<ParticleDraw>,
    spare: Vec<SpareBuffers>,
}

impl ParticleDrawPool {
    /// Forgets this frame's draws, releasing their materials and keeping their allocations.
    pub fn clear(&mut self) {
        self.spare
            .extend(self.live.drain(..).map(|draw| SpareBuffers {
                vertices: draw.vertices,
                indices: draw.indices,
                subtexture_rects: draw.subtexture_rects,
            }));
    }

    #[cfg(test)]
    pub fn take_first(self) -> Option<ParticleDraw> {
        self.live.into_iter().next()
    }

    pub fn draws(&self) -> &[ParticleDraw] {
        &self.live
    }

    /// Appends a draw for `material` whose buffers start empty and keep their capacity.
    pub fn push_with(
        &mut self,
        material: &Arc<EffectMaterial>,
        fill: impl FnOnce(&mut ParticleDraw),
    ) {
        let mut spare = self.spare.pop().unwrap_or_default();
        spare.vertices.clear();
        spare.indices.clear();
        spare.subtexture_rects.clear();
        let mut draw = ParticleDraw {
            vertices: spare.vertices,
            indices: spare.indices,
            material: Arc::clone(material),
            elapsed_seconds: 0.0,
            model: Mat4::IDENTITY,
            subtexture_rects: spare.subtexture_rects,
            bounds_centre: Vec3::ZERO,
        };
        fill(&mut draw);
        self.live.push(draw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::effects::test_fixtures::stub_material;

    fn pool_with_one_draw(vertices: usize) -> ParticleDrawPool {
        let mut pool = ParticleDrawPool::default();
        pool.push_with(&Arc::new(stub_material()), |draw| {
            draw.vertices.resize(vertices, bytemuck::Zeroable::zeroed());
        });
        pool
    }

    #[test]
    fn a_cleared_pool_exposes_no_draws() {
        let mut pool = pool_with_one_draw(4);
        pool.clear();
        assert!(pool.draws().is_empty());
    }

    #[test]
    fn a_reused_slot_starts_empty_and_keeps_its_capacity() {
        let mut pool = pool_with_one_draw(64);
        let capacity = pool.draws()[0].vertices.capacity();
        pool.clear();
        pool.push_with(&Arc::new(stub_material()), |draw| {
            assert_eq!(draw.vertices, []);
            assert_eq!(draw.vertices.capacity(), capacity);
        });
        assert_eq!(pool.draws().len(), 1);
    }

    #[test]
    fn a_cleared_pool_holds_no_material_references() {
        let material = Arc::new(stub_material());
        let mut pool = ParticleDrawPool::default();
        pool.push_with(&material, |_| {});
        pool.push_with(&material, |_| {});
        assert_eq!(Arc::strong_count(&material), 3);
        pool.clear();
        assert_eq!(Arc::strong_count(&material), 1);
    }
}
