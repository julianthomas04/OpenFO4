use glam::Vec3;

/// The exterior's one directional light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightSource {
    /// Unit vector towards the light in engine space.
    pub direction: Vec3,
    /// Multiplies the weather's sunlight colour: 1 at solar noon and solar midnight.
    pub strength: f32,
}
