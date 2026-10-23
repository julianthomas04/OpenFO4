//! Debug overlay drawing the skeletons of animated objects and characters as
//! bone lines and joint crosses. A skeleton shows through its own object's
//! mesh but is hidden by scene geometry in front of the object: a fragment is
//! hidden when the scene is nearer than the fragment by more than the reach of
//! its joint, the distance from the joint to the mesh it drives.

use std::time::{Duration, Instant};

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use tracing::{debug, info, warn};

use crate::anim::JointIndex;
use crate::rendering::animated_objects::AnimatedObjects;
use crate::rendering::frustum::Aabb;
use crate::rendering::lighting::frame_binding::FrameBinding;
use crate::rendering::shaders;
use crate::rendering::skinning::CharacterSystem;

const ENV_VAR: &str = "OPENFO4_SKELETON_OVERLAY";
/// About 50 m.
const DEFAULT_MAX_DISTANCE: f32 = 350.0;
const MAX_DISTANCE_RANGE: std::ops::RangeInclusive<f32> = 35.0..=1400.0;
/// Joint cross arm half-length as a share of the skeleton's median bone length.
const JOINT_SIZE_PER_BONE: f32 = 0.15;
/// Bounds of the joint cross arm half-length, engine units.
const JOINT_HALF_SIZE_RANGE: std::ops::RangeInclusive<f32> = 0.05..=0.7;
const LOG_INTERVAL: Duration = Duration::from_secs(2);
/// Reach of a joint that drives no geometry, as a share of the skeleton's extent.
const IDLE_REACH_PER_EXTENT: f32 = 0.1;
/// Bounds of that default reach, engine units.
const IDLE_REACH_RANGE: std::ops::RangeInclusive<f32> = 0.35..=7.0;
const VERTEX_STRIDE: u64 = std::mem::size_of::<OverlayVertex>() as u64;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OverlayVertex {
    position: [f32; 3],
    color: [u8; 4],
    /// How far the mesh driven by this vertex's joint reaches from it, world units.
    reach: f32,
}

/// One skeleton in its current pose: joint hierarchy, model-space joint
/// matrices and the model-to-world transform.
pub struct PosedSkeleton<'a> {
    pub parents: &'a [Option<JointIndex>],
    pub model_pose: &'a [Mat4],
    pub world: Mat4,
    /// Per joint, the largest model-space distance to a vertex it drives; 0 for
    /// a joint that drives no geometry.
    pub joint_reach: &'a [f32],
}

#[derive(Clone, Copy)]
struct Palette {
    bone: [u8; 4],
    joint: [u8; 4],
}

const CHARACTER_PALETTE: Palette = Palette {
    bone: [64, 220, 255, 255],
    joint: [255, 240, 80, 255],
};
const OBJECT_PALETTE: Palette = Palette {
    bone: [255, 150, 40, 255],
    joint: [255, 70, 220, 255],
};

/// Camera data the overlay culls against.
pub struct OverlayView {
    pub eye: Vec3,
    pub planes: [Vec4; 6],
}

pub struct SkeletonOverlay {
    pub enabled: bool,
    max_distance: f32,
    pipeline: wgpu::RenderPipeline,
    depth_layout: wgpu::BindGroupLayout,
    depth_bind_group: wgpu::BindGroup,
    vertices: Vec<OverlayVertex>,
    joint_scratch: Vec<Vec3>,
    bone_lengths: Vec<f32>,
    reach_scratch: Vec<f32>,
    buffer: Option<wgpu::Buffer>,
    capacity: u64,
    vertex_count: u32,
    last_log: Option<Instant>,
}

impl SkeletonOverlay {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        frame_layout: &wgpu::BindGroupLayout,
        scene_depth: &wgpu::TextureView,
    ) -> Self {
        let depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("skeleton_overlay_depth_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Depth,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Skeleton Overlay Pipeline Layout"),
            bind_group_layouts: &[Some(&depth_layout), Some(frame_layout)],
            immediate_size: 0,
        });
        Self {
            enabled: enabled_from_env(),
            max_distance: DEFAULT_MAX_DISTANCE,
            pipeline: create_pipeline(device, format, &layout),
            depth_bind_group: depth_bind_group(device, &depth_layout, scene_depth),
            depth_layout,
            vertices: Vec::new(),
            joint_scratch: Vec::new(),
            bone_lengths: Vec::new(),
            reach_scratch: Vec::new(),
            buffer: None,
            capacity: 0,
            vertex_count: 0,
            last_log: None,
        }
    }

    /// Rebuilds this frame's line list from the current poses. Does nothing
    /// (and holds no GPU memory) while disabled.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        view: impl FnOnce() -> OverlayView,
        objects: &AnimatedObjects,
        characters: &CharacterSystem,
    ) {
        if !self.enabled {
            self.release();
            return;
        }
        let view = view();
        self.vertices.clear();
        let mut object_count = 0;
        for skeleton in objects.skeletons() {
            object_count += usize::from(self.append(&view, &skeleton, OBJECT_PALETTE));
        }
        let mut character_count = 0;
        for character in characters
            .characters()
            .iter()
            .filter(|character| !character.is_faulted())
        {
            let skeleton = PosedSkeleton {
                parents: character.rig().skeleton.parents(),
                model_pose: character.model_pose(),
                world: *character.transform(),
                joint_reach: character.rig().joint_reach(),
            };
            character_count += usize::from(self.append(&view, &skeleton, CHARACTER_PALETTE));
        }
        self.upload(device, queue);
        self.log(object_count, character_count);
    }

    /// Rebinds the scene depth after the depth texture was recreated.
    pub fn resize(&mut self, device: &wgpu::Device, scene_depth: &wgpu::TextureView) {
        self.depth_bind_group = depth_bind_group(device, &self.depth_layout, scene_depth);
    }

    /// Draws the lines over the finished world pass in a pass of their own, so
    /// the fragment shader can read the scene depth. Encodes nothing while
    /// disabled or when no skeleton is in view.
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        frame_binding: &FrameBinding,
    ) {
        let Some(buffer) = self.buffer.as_ref().filter(|_| self.vertex_count > 0) else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Skeleton Overlay Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.depth_bind_group, &[]);
        frame_binding.bind(&mut pass);
        pass.set_vertex_buffer(
            0,
            buffer.slice(..u64::from(self.vertex_count) * VERTEX_STRIDE),
        );
        pass.draw(0..self.vertex_count, 0..1);
    }

    fn release(&mut self) {
        if self.buffer.is_some() {
            self.vertices = Vec::new();
            self.joint_scratch = Vec::new();
            self.bone_lengths = Vec::new();
            self.reach_scratch = Vec::new();
            self.buffer = None;
            self.capacity = 0;
        }
        self.vertex_count = 0;
        self.last_log = None;
    }

    /// Appends one skeleton unless it is out of range or off screen; returns
    /// whether it was drawn.
    fn append(&mut self, view: &OverlayView, skeleton: &PosedSkeleton, palette: Palette) -> bool {
        let joints = &mut self.joint_scratch;
        joints.clear();
        joints.extend(
            skeleton
                .model_pose
                .iter()
                .map(|joint| skeleton.world.transform_point3(joint.w_axis.truncate())),
        );
        let Some(tight) = Aabb::from_points(joints.iter().copied()) else {
            return false;
        };
        let joint_half_size = joint_half_size(skeleton.parents, joints, &mut self.bone_lengths);
        let bounds = Aabb {
            min: tight.min - Vec3::splat(joint_half_size),
            max: tight.max + Vec3::splat(joint_half_size),
        };
        let nearest = view.eye.clamp(bounds.min, bounds.max);
        if nearest.distance(view.eye) > self.max_distance || bounds.is_outside(&view.planes) {
            return false;
        }
        let reaches = &mut self.reach_scratch;
        world_reaches(skeleton, tight.half_extents().length() * 2.0, reaches);
        let reach_of = |joint: usize| reaches.get(joint).copied().unwrap_or(0.0);
        for (joint, &position) in joints.iter().enumerate() {
            let parent = skeleton
                .parents
                .get(joint)
                .copied()
                .flatten()
                .map(usize::from);
            if let Some((parent, &parent_position)) =
                parent.and_then(|parent| Some((parent, joints.get(parent)?)))
            {
                push_line(
                    &mut self.vertices,
                    parent_position,
                    position,
                    palette.bone,
                    reach_of(parent),
                    reach_of(joint),
                );
            }
            push_cross(
                &mut self.vertices,
                position,
                joint_half_size,
                palette.joint,
                reach_of(joint),
            );
        }
        true
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        self.vertex_count = self.vertices.len() as u32;
        if self.vertices.is_empty() {
            return;
        }
        let bytes: &[u8] = bytemuck::cast_slice(&self.vertices);
        let needed = bytes.len() as u64;
        if self.buffer.is_none() || needed > self.capacity {
            self.capacity = needed.next_power_of_two();
            self.buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("skeleton_overlay_lines"),
                size: self.capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        if let Some(buffer) = &self.buffer {
            queue.write_buffer(buffer, 0, bytes);
        }
    }

    fn log(&mut self, objects: usize, characters: usize) {
        match self.last_log {
            None => info!("skeleton overlay: on objects={objects} characters={characters}"),
            Some(last) if last.elapsed() >= LOG_INTERVAL => {
                debug!("skeleton overlay: on objects={objects} characters={characters}");
            }
            Some(_) => return,
        }
        self.last_log = Some(Instant::now());
    }
}

/// "Skeletons" checkbox and distance slider for the developer menu.
pub fn skeleton_overlay_section(ui: &mut egui::Ui, overlay: &mut SkeletonOverlay) {
    ui.checkbox(&mut overlay.enabled, "Skeletons");
    ui.add_enabled(
        overlay.enabled,
        egui::Slider::new(&mut overlay.max_distance, MAX_DISTANCE_RANGE).text("skeleton distance"),
    );
}

fn enabled_from_env() -> bool {
    match std::env::var(ENV_VAR) {
        Ok(value) if value == "1" => true,
        Ok(value) if value.is_empty() || value == "0" => false,
        Ok(value) => {
            warn!("{ENV_VAR}='{value}' is not 0 or 1; skeleton overlay stays off");
            false
        }
        Err(_) => false,
    }
}

/// Per-joint reach in world units; joints without geometry get a share of the
/// skeleton's extent.
fn world_reaches(skeleton: &PosedSkeleton, extent: f32, out: &mut Vec<f32>) {
    let scale = skeleton.world.x_axis.truncate().length();
    let idle =
        (extent * IDLE_REACH_PER_EXTENT).clamp(*IDLE_REACH_RANGE.start(), *IDLE_REACH_RANGE.end());
    out.clear();
    out.extend((0..skeleton.model_pose.len()).map(|joint| {
        match skeleton.joint_reach.get(joint).copied() {
            Some(reach) if reach > 0.0 => reach * scale,
            _ => idle,
        }
    }));
}

fn push_line(
    out: &mut Vec<OverlayVertex>,
    from: Vec3,
    to: Vec3,
    color: [u8; 4],
    from_reach: f32,
    to_reach: f32,
) {
    out.push(OverlayVertex {
        position: from.to_array(),
        color,
        reach: from_reach,
    });
    out.push(OverlayVertex {
        position: to.to_array(),
        color,
        reach: to_reach,
    });
}

fn push_cross(
    out: &mut Vec<OverlayVertex>,
    center: Vec3,
    half_size: f32,
    color: [u8; 4],
    reach: f32,
) {
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        push_line(
            out,
            center - axis * half_size,
            center + axis * half_size,
            color,
            reach,
            reach,
        );
    }
}

fn depth_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    scene_depth: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("skeleton_overlay_depth_bind_group"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(scene_depth),
        }],
    })
}

/// Cross arm half-length scaled to the skeleton: coincident parent-child joints
/// (twist and helper bones) do not count towards the median bone length.
fn joint_half_size(parents: &[Option<JointIndex>], joints: &[Vec3], lengths: &mut Vec<f32>) -> f32 {
    lengths.clear();
    for (joint, &position) in joints.iter().enumerate() {
        let parent = parents
            .get(joint)
            .copied()
            .flatten()
            .and_then(|parent| joints.get(usize::from(parent)));
        if let Some(&parent_position) = parent {
            let length = parent_position.distance(position);
            if length > f32::EPSILON {
                lengths.push(length);
            }
        }
    }
    if lengths.is_empty() {
        return *JOINT_HALF_SIZE_RANGE.start();
    }
    let middle = lengths.len() / 2;
    let (_, median, _) = lengths.select_nth_unstable_by(middle, f32::total_cmp);
    (*median * JOINT_SIZE_PER_BONE)
        .clamp(*JOINT_HALF_SIZE_RANGE.start(), *JOINT_HALF_SIZE_RANGE.end())
}

fn create_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    layout: &wgpu::PipelineLayout,
) -> wgpu::RenderPipeline {
    let shader = shaders::SKELETON_OVERLAY.compile(device);
    let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Unorm8x4, 2 => Float32];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Skeleton Overlay Pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: VERTEX_STRIDE,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &attributes,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::LineList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}
