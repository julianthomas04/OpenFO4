use glam::Vec3;

/// The geometry of the one directional light of the exterior: the body that currently lights it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightSource {
    /// Unit vector towards the light in engine space.
    pub direction: Vec3,
    /// 0 when the light is the sun, 1 when it is the moon, 0.5 on a tie.
    pub from_moon_weight: f32,
    /// 0 while neither body is up, 1 under a full sun or moon.
    pub strength: f32,
}
