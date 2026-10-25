use glam::Vec3;

/// The geometry of the one directional light of the exterior.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightSource {
    /// Unit vector towards the light in engine space.
    pub direction: Vec3,
    /// How far the direction has turned from the sun to the moon: 0 when the light is the sun,
    /// 1 when it is the moon. 0.5 while neither gives light.
    pub from_moon_weight: f32,
    /// 0 while neither body is up, 1 under a full sun or moon.
    pub strength: f32,
}
