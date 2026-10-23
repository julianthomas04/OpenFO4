use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::assets::{
    layout::texture_key,
    types::texture::{create_cube_texture, load_cube},
};

pub const CUBE_SLOTS: u32 = 256;

/// Environment variable naming a cube (path below `textures`) that every environment material reflects.
pub const FORCED_CUBE_ENV: &str = "OPENFO4_ENV_CUBE";

const CUBE_FACES: u32 = 6;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct CubeHandle(u32);

impl CubeHandle {
    /// Slot 0: the 1x1 black cube, also used when no cube is wanted or one failed to load.
    pub const NONE: Self = Self(0);

    pub fn slot(self) -> u32 {
        self.0
    }
}

/// Bindless array of environment cube maps. Slot 0 is a 1x1 black cube; every
/// other slot is a fully resident cube loaded on first `acquire` and kept for
/// the session (cubes are small and never streamed).
pub struct CubeArray {
    device: wgpu::Device,
    queue: wgpu::Queue,
    asset_root: PathBuf,
    /// Occupied slots in slot order; index 0 is the black cube.
    views: Vec<wgpu::TextureView>,
    handles: HashMap<String, CubeHandle>,
    changed: bool,
    /// Debug override: every environment material reflects this cube.
    forced_cube: Option<String>,
}

fn path_key(path: &str) -> String {
    path.replace('\\', "/").to_lowercase()
}

impl CubeArray {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, asset_root: &Path) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cube_array_black"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: CUBE_FACES,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[0, 0, 0, 255].repeat(CUBE_FACES as usize),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: CUBE_FACES,
            },
        );
        let black_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("cube_array_black_view"),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        Self {
            device: device.clone(),
            queue: queue.clone(),
            asset_root: asset_root.to_path_buf(),
            views: vec![black_view],
            handles: HashMap::new(),
            changed: false,
            forced_cube: None,
        }
    }

    pub fn set_forced_cube(&mut self, path: Option<String>) {
        self.forced_cube = path;
    }

    pub fn forced_cube(&self) -> Option<&str> {
        self.forced_cube.as_deref()
    }

    /// Resolves a cube texture path (below `textures`) to a slot, loading the
    /// cube on first use. A missing, malformed or surplus cube resolves to the
    /// black slot with a one-time warning, so `NONE` for a named cube means it failed.
    pub fn acquire(&mut self, path: &str) -> CubeHandle {
        let key = path_key(path);
        if let Some(&handle) = self.handles.get(&key) {
            return handle;
        }
        let handle = self.load(path, &key).unwrap_or(CubeHandle::NONE);
        self.handles.insert(key, handle);
        handle
    }

    fn load(&mut self, path: &str, key: &str) -> Option<CubeHandle> {
        if self.views.len() >= CUBE_SLOTS as usize {
            crate::utils::warn_once("cube-array-exhausted", || {
                format!("cube array holds all {CUBE_SLOTS} slots; further cubes render black")
            });
            return None;
        }
        let full_path = self.asset_root.join(texture_key(path));
        let image = match load_cube(&full_path.to_string_lossy()) {
            Ok(image) => image,
            Err(error) => {
                crate::utils::warn_once(&format!("cube-load:{key}"), || {
                    format!("environment cube '{path}' not loaded: {error}")
                });
                return None;
            }
        };
        let texture = create_cube_texture(&self.device, &self.queue, &image, path);
        self.views
            .push(texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some(path),
                dimension: Some(wgpu::TextureViewDimension::Cube),
                ..Default::default()
            }));
        self.changed = true;
        Some(CubeHandle(self.views.len() as u32 - 1))
    }

    /// The cube view of `handle`.
    pub fn view(&self, handle: CubeHandle) -> &wgpu::TextureView {
        &self.views[handle.slot() as usize]
    }

    /// One view per slot: the loaded cubes in slot order, every free slot padded with the
    /// black cube so the bound array is always fully populated and a cube added later only
    /// changes which view a slot shows.
    pub fn views(&self) -> Vec<&wgpu::TextureView> {
        let black = &self.views[0];
        self.views
            .iter()
            .chain(std::iter::repeat(black))
            .take(CUBE_SLOTS as usize)
            .collect()
    }

    /// True once after a cube was added, so the world bind group is rebuilt then.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }
}

#[cfg(test)]
mod tests;
