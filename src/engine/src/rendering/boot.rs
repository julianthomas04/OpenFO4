//! What the main thread owns while the renderer state is built on a worker: the window, the GPU
//! device and the surface, enough to keep presenting black frames.

use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::window::Window;

use super::device_loss::DeviceLossMonitor;
use super::game_settings::GameSettings;
use super::mouse_capture::MouseCapture;
use super::tonemap;
use super::window_surface::WindowSurface;
use super::{adapter, device, instance};
use crate::assets::AssetServer;
use crate::game_flow::StartMode;
use crate::spawn_cell::SpawnCellTarget;
use crate::startup::{CancelToken, StartupError};

/// The device, queue and window surface, shared between the main thread and the worker.
#[derive(Clone)]
pub struct GpuContext {
    pub surface: Arc<WindowSurface>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub device_loss: DeviceLossMonitor,
    /// The configuration the running game uses (uncapped presentation).
    pub config: wgpu::SurfaceConfiguration,
    /// The configuration while the engine starts up (vsynced, so black frames do not spin a core).
    pub boot_config: wgpu::SurfaceConfiguration,
}

struct SurfaceBase {
    format: wgpu::TextureFormat,
    alpha_mode: wgpu::CompositeAlphaMode,
}

/// The surface configuration both phases share, apart from how frames are paced.
fn surface_config(
    base: &SurfaceBase,
    present_mode: wgpu::PresentMode,
    size: PhysicalSize<u32>,
) -> wgpu::SurfaceConfiguration {
    wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: base.format,
        width: size.width,
        height: size.height,
        present_mode,
        alpha_mode: base.alpha_mode,
        view_formats: tonemap::surface_view_formats(base.format),
        desired_maximum_frame_latency: 2,
        color_space: wgpu::SurfaceColorSpace::Auto,
    }
}

impl GpuContext {
    pub fn create(window: &Arc<Window>) -> Result<Self, StartupError> {
        let size = window.inner_size();
        let instance = instance::new();
        let surface = instance.create_surface(window.clone())?;
        let adapter = pollster::block_on(adapter::new(instance, &surface))?;
        let (device, queue) =
            pollster::block_on(device::request_device(&adapter)).map_err(StartupError::Device)?;
        let device_loss = DeviceLossMonitor::watch(&device);

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        // Prefer uncapped modes but never panic on adapters without them.
        let present_mode = [wgpu::PresentMode::Immediate, wgpu::PresentMode::Mailbox]
            .into_iter()
            .find(|m| surface_caps.present_modes.contains(m))
            .unwrap_or(wgpu::PresentMode::Fifo);

        let base = SurfaceBase {
            format: surface_format,
            alpha_mode: surface_caps.alpha_modes[0],
        };
        let config = surface_config(&base, present_mode, size);
        let boot_config = surface_config(&base, wgpu::PresentMode::Fifo, size);

        Ok(Self {
            surface: Arc::new(WindowSurface::new(surface, device_loss.clone())),
            device,
            queue,
            device_loss,
            config,
            boot_config,
        })
    }

    /// Resizes both configurations and puts the surface on the boot one. A zero size (a minimised
    /// window) is ignored.
    pub fn resize_for_boot(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        for config in [&mut self.config, &mut self.boot_config] {
            config.width = size.width;
            config.height = size.height;
        }
        self.surface.configure(&self.device, &self.boot_config);
    }

    pub fn present_black(&self) {
        let Some(frame) = self.surface.acquire_frame(&self.device, &self.boot_config) else {
            return;
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        submit_black_pass(&self.device, &self.queue, &view);
        self.queue.present(frame);
    }
}

fn submit_black_pass(device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::TextureView) {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("boot_black_frame"),
    });
    drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("boot_black_pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    }));
    queue.submit([encoder.finish()]);
}

/// What the startup worker needs besides the boot parts; the renderer state is built from it.
pub struct StartupInputs {
    pub asset_server: AssetServer,
    pub perf_log: bool,
    pub start_mode: StartMode,
    pub cancel: CancelToken,
}

/// Everything the renderer state needs besides the boot parts.
pub struct StateInputs {
    pub startup: StartupInputs,
    pub settings: GameSettings,
    pub spawn_cell: Option<SpawnCellTarget>,
}

/// What the worker needs from the main thread to assemble the renderer state.
#[derive(Clone)]
pub struct BootParts {
    pub window: Arc<Window>,
    pub gpu: GpuContext,
    pub mouse_capture: MouseCapture,
    pub freeze_camera: bool,
}

pub struct BootContext {
    parts: BootParts,
}

impl BootContext {
    pub fn create(window: Arc<Window>, start_mode: StartMode) -> Result<Self, StartupError> {
        // Applied before any slow startup work so the cursor is already in its start state
        // (released under freeze, and at the menu) while the engine loads.
        let freeze_camera =
            super::camera_frozen(crate::utils::env_flag) || super::shot::shot_requested();
        let mut mouse_capture = MouseCapture::new(freeze_camera);
        if start_mode == StartMode::Menu {
            mouse_capture.set_locked(false);
        }
        mouse_capture.apply(&window);

        let gpu = GpuContext::create(&window)?;
        let size = window.inner_size();
        let mut boot = Self {
            parts: BootParts {
                window,
                gpu,
                mouse_capture,
                freeze_camera,
            },
        };
        boot.resize(size);
        Ok(boot)
    }

    pub fn window(&self) -> &Arc<Window> {
        &self.parts.window
    }

    /// The parts for the worker, sized to the window as it is now.
    pub fn parts(&self) -> BootParts {
        self.parts.clone()
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        self.parts.gpu.resize_for_boot(size);
    }

    pub fn present_black(&self) {
        self.parts.gpu.present_black();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rendering::gpu_test::test_device;

    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
    const BYTES_PER_PIXEL: u32 = 4;

    fn read_after_black_pass(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: PhysicalSize<u32>,
    ) -> Vec<u8> {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("black_pass_target"),
            size: wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let row_bytes = size.width * BYTES_PER_PIXEL;
        let rows = vec![0xAB; (row_bytes * size.height) as usize];
        let layout = wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(row_bytes),
            rows_per_image: None,
        };
        queue.write_texture(texture.as_image_copy(), &rows, layout, texture.size());
        submit_black_pass(
            device,
            queue,
            &texture.create_view(&wgpu::TextureViewDescriptor::default()),
        );
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("black_pass_readback"),
            size: u64::from(row_bytes * size.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout,
            },
            texture.size(),
        );
        queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                sender
                    .send(result)
                    .expect("receiver outlives the map callback");
            });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("readback poll");
        receiver.recv().unwrap().expect("staging buffer maps");
        let mapped = staging
            .slice(..)
            .get_mapped_range()
            .expect("staging range is mapped");
        mapped.to_vec()
    }

    #[test]
    fn the_black_pass_clears_a_target_of_any_size_to_opaque_black() {
        let Some((device, queue)) = test_device("the_black_pass_clears_a_target") else {
            return;
        };
        for size in [PhysicalSize::new(64, 32), PhysicalSize::new(128, 16)] {
            let pixels = read_after_black_pass(&device, &queue, size);
            assert_eq!(pixels.len(), (size.width * size.height * 4) as usize);
            assert!(
                pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|texel| *texel == [0, 0, 0, 0xFF]),
                "{size:?}"
            );
        }
    }
}
