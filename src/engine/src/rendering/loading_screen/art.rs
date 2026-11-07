//! What the loading screen's 3D art is made of, as plain data: the model to draw and how the
//! screen's record places it and limits the viewer's rotation and zoom.

use std::path::PathBuf;

/// A model's placement in the scene, as the screen's transform record stores it: Bethesda units
/// and axes (X east, Y north, Z up), rotations in radians.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelTransform {
    pub position: [f32; 3],
    pub rotation_radians: [f32; 3],
    pub scale: f32,
    /// The viewer's rotation turns the whole placement around the world origin (the camera)
    /// instead of the model around its own position.
    pub around_origin: bool,
}

/// One loading screen's art.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadingArt {
    /// The converted model, as an absolute path to its GLB.
    pub model_path: PathBuf,
    /// The folder the model's textures are found below.
    pub asset_root: PathBuf,
    pub transform: ModelTransform,
    /// The viewer's turn away from the transform's rotation, in degrees, as `(min, max)`.
    pub rotation_limits_deg: (i16, i16),
    /// The viewer's zoom as `(min, max)`; 0 is the placement as stored, 1 halves the distance.
    pub zoom_limits: (f32, f32),
    /// The viewer cannot turn the model.
    pub no_rotation: bool,
}
