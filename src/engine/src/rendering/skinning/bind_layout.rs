use std::marker::PhantomData;

use bytemuck::Pod;
use glam::Mat4;

use crate::rendering::gpu_driven::GpuInstance;

/// Group the skinned pipelines bind their per-frame character data to; same
/// slot as the static instance group so shared WGSL (`scene_bindings.wgsl`)
/// finds `instances` where it expects it.
pub const SKINNED_GROUP: u32 = 2;

const INSTANCES_BINDING: u32 = 0;
const PALETTE_BINDING: u32 = 2;
const PALETTE_BASE_BINDING: u32 = 3;

const INITIAL_INSTANCES: usize = 16;
const INITIAL_PALETTE_MATRICES: usize = 512;

pub struct SkinnedBindLayout {
    layout: wgpu::BindGroupLayout,
}

impl SkinnedBindLayout {
    pub fn new(device: &wgpu::Device) -> Self {
        let storage = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("skinned_bgl"),
            entries: &[
                storage(INSTANCES_BINDING, wgpu::ShaderStages::VERTEX_FRAGMENT),
                storage(PALETTE_BINDING, wgpu::ShaderStages::VERTEX),
                storage(PALETTE_BASE_BINDING, wgpu::ShaderStages::VERTEX),
            ],
        });
        Self { layout }
    }

    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }
}

/// A storage buffer that doubles when a write outgrows it.
struct GrowingStorage<T: Pod> {
    label: &'static str,
    buffer: wgpu::Buffer,
    capacity: usize,
    element: PhantomData<T>,
}

impl<T: Pod> GrowingStorage<T> {
    fn new(device: &wgpu::Device, label: &'static str, capacity: usize) -> Self {
        Self {
            label,
            buffer: Self::allocate(device, label, capacity),
            capacity,
            element: PhantomData,
        }
    }

    fn allocate(device: &wgpu::Device, label: &'static str, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: (capacity * std::mem::size_of::<T>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Returns whether the buffer was replaced, which invalidates bind groups on it.
    fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[T]) -> bool {
        let grown = data.len() > self.capacity;
        if grown {
            self.capacity = data.len().next_power_of_two();
            self.buffer = Self::allocate(device, self.label, self.capacity);
        }
        if !data.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(data));
        }
        grown
    }
}

/// Everything the skinned passes read, rewritten once per frame.
pub struct SkinnedFrameBuffers {
    instances: GrowingStorage<GpuInstance>,
    palette: GrowingStorage<Mat4>,
    palette_base: GrowingStorage<u32>,
    bind_group: wgpu::BindGroup,
}

impl SkinnedFrameBuffers {
    pub fn new(device: &wgpu::Device, layout: &SkinnedBindLayout) -> Self {
        let instances = GrowingStorage::new(device, "skinned_instances", INITIAL_INSTANCES);
        let palette = GrowingStorage::new(device, "skin_palette", INITIAL_PALETTE_MATRICES);
        let palette_base = GrowingStorage::new(device, "skin_palette_base", INITIAL_INSTANCES);
        let bind_group = Self::bind_group_for(device, layout, &instances, &palette, &palette_base);
        Self {
            instances,
            palette,
            palette_base,
            bind_group,
        }
    }

    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &SkinnedBindLayout,
        instances: &[GpuInstance],
        palette: &[Mat4],
        palette_base: &[u32],
    ) {
        let grown = [
            self.instances.write(device, queue, instances),
            self.palette.write(device, queue, palette),
            self.palette_base.write(device, queue, palette_base),
        ];
        if grown.contains(&true) {
            self.bind_group = Self::bind_group_for(
                device,
                layout,
                &self.instances,
                &self.palette,
                &self.palette_base,
            );
        }
    }

    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }

    fn bind_group_for(
        device: &wgpu::Device,
        layout: &SkinnedBindLayout,
        instances: &GrowingStorage<GpuInstance>,
        palette: &GrowingStorage<Mat4>,
        palette_base: &GrowingStorage<u32>,
    ) -> wgpu::BindGroup {
        fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
            wgpu::BindGroupEntry {
                binding,
                resource: buffer.as_entire_binding(),
            }
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skinned_bind_group"),
            layout: layout.layout(),
            entries: &[
                entry(INSTANCES_BINDING, &instances.buffer),
                entry(PALETTE_BINDING, &palette.buffer),
                entry(PALETTE_BASE_BINDING, &palette_base.buffer),
            ],
        })
    }
}
