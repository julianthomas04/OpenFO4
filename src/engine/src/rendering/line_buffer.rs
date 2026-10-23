use super::render_pipelines::debug_lines::LineVertex;

/// A vertex buffer for debug line lists that grows to the next power of two and is rewritten in
/// place, so rebuilding an overlay does not reallocate every time.
pub struct LineBuffer {
    label: &'static str,
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
    vertex_count: u32,
}

impl LineBuffer {
    pub const fn new(label: &'static str) -> Self {
        Self {
            label,
            buffer: None,
            capacity: 0,
            vertex_count: 0,
        }
    }

    pub fn is_allocated(&self) -> bool {
        self.buffer.is_some()
    }

    /// Replaces the contents. An empty list keeps the allocation and draws nothing.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, vertices: &[LineVertex]) {
        self.vertex_count = vertices.len() as u32;
        if vertices.is_empty() {
            return;
        }
        let bytes: &[u8] = bytemuck::cast_slice(vertices);
        let needed = bytes.len() as u64;
        let buffer = match &self.buffer {
            Some(buffer) if needed <= self.capacity => buffer,
            _ => {
                self.capacity = needed.next_power_of_two();
                self.buffer
                    .insert(device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some(self.label),
                        size: self.capacity,
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }))
            }
        };
        queue.write_buffer(buffer, 0, bytes);
    }

    /// Frees the GPU buffer.
    pub fn release(&mut self) {
        self.buffer = None;
        self.capacity = 0;
        self.vertex_count = 0;
    }

    pub fn draw<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        pipeline: &'pass wgpu::RenderPipeline,
    ) {
        if self.vertex_count == 0 {
            return;
        }
        if let Some(buffer) = &self.buffer {
            pass.set_pipeline(pipeline);
            pass.set_vertex_buffer(0, buffer.slice(..));
            pass.draw(0..self.vertex_count, 0..1);
        }
    }
}
