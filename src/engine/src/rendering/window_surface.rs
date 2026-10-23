use std::mem::ManuallyDrop;
use std::ops::Deref;

use super::device_loss::DeviceLossMonitor;

/// The window's surface. A texture acquired before a device loss can be neither
/// presented nor discarded afterwards, and wgpu panics when it unconfigures a
/// swapchain such a texture still holds, so a surface outliving its device is
/// leaked instead of dropped.
pub struct WindowSurface {
    surface: ManuallyDrop<wgpu::Surface<'static>>,
    device_loss: DeviceLossMonitor,
}

impl WindowSurface {
    pub fn new(surface: wgpu::Surface<'static>, device_loss: DeviceLossMonitor) -> Self {
        Self {
            surface: ManuallyDrop::new(surface),
            device_loss,
        }
    }

    /// The next frame to draw into, or `None` when this frame has to be skipped. A stale or lost
    /// swapchain is reconfigured with `config` and recovers on a following frame.
    pub fn acquire_frame(
        &self,
        device: &wgpu::Device,
        config: &wgpu::SurfaceConfiguration,
    ) -> Option<wgpu::SurfaceTexture> {
        match self.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => Some(frame),
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                self.configure(device, config);
                Some(frame)
            }
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => None,
            // Lost is recoverable the same way as Outdated: reconfigure and
            // try again next frame instead of killing the app.
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.configure(device, config);
                None
            }
        }
    }
}

impl Deref for WindowSurface {
    type Target = wgpu::Surface<'static>;

    fn deref(&self) -> &Self::Target {
        &self.surface
    }
}

impl Drop for WindowSurface {
    fn drop(&mut self) {
        match self.device_loss.check() {
            // SAFETY: `surface` is never accessed again after this drop.
            Ok(()) => unsafe { ManuallyDrop::drop(&mut self.surface) },
            Err(lost) => {
                tracing::warn!(
                    "leaking the window surface, its swapchain cannot be released after {lost}"
                );
            }
        }
    }
}
