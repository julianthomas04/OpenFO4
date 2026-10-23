//! A uniform buffer of per-draw blocks read through one dynamic-offset bind group.

use std::num::NonZeroU64;

/// Where a buffer of per-draw blocks lives in a pipeline layout.
#[derive(Clone, Copy)]
pub struct DrawBufferSpec {
    pub buffer_label: &'static str,
    pub group_label: &'static str,
    pub binding: u32,
    /// Bytes one draw's block binds.
    pub block_size: u64,
    /// Bytes between two draws' blocks.
    pub stride: u64,
}

pub struct DrawBuffer {
    spec: DrawBufferSpec,
    pub buffer: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
    pub capacity: usize,
}

impl DrawBuffer {
    pub fn new(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        spec: DrawBufferSpec,
        capacity: usize,
    ) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(spec.buffer_label),
            size: capacity as u64 * spec.stride,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(spec.group_label),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: spec.binding,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: NonZeroU64::new(spec.block_size),
                }),
            }],
        });
        Self {
            spec,
            buffer,
            bind_group,
            capacity,
        }
    }

    /// Reallocates to the next power of two above `needed` when the buffer is too small; the
    /// old contents are not kept.
    pub fn reserve(
        &mut self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        needed: usize,
    ) {
        if needed > self.capacity {
            *self = Self::new(device, layout, self.spec, needed.next_power_of_two());
        }
    }
}
