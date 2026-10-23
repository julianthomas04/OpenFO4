use std::sync::{Arc, OnceLock};

#[derive(Debug, Clone, thiserror::Error)]
#[error("GPU device lost ({reason:?}): {message}")]
pub struct DeviceLost {
    pub reason: wgpu::DeviceLostReason,
    pub message: String,
}

/// Records wgpu's device-lost notification. A lost device destroys every
/// buffer and texture, so GPU work and mapped reads must stop once it fires.
#[derive(Clone, Default)]
pub struct DeviceLossMonitor(Arc<OnceLock<DeviceLost>>);

impl DeviceLossMonitor {
    pub fn watch(device: &wgpu::Device) -> Self {
        let monitor = Self::default();
        let loss = monitor.0.clone();
        device.set_device_lost_callback(move |reason, message| {
            if let Err(repeated) = loss.set(DeviceLost { reason, message }) {
                tracing::error!("device lost again after the first loss: {repeated}");
            }
        });
        monitor
    }

    pub fn check(&self) -> Result<(), DeviceLost> {
        match self.0.get() {
            Some(lost) => Err(lost.clone()),
            None => Ok(()),
        }
    }
}
