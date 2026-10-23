use super::*;
use crate::assets::types::texture::cube_test_support::{DdsSpec, FACE_COLORS, Format, Header};
use crate::rendering::gpu_test::test_device;

fn root_with_cubes(names: &[&str]) -> tempfile::TempDir {
    let root = tempfile::TempDir::new().unwrap();
    let textures = root.path().join("textures");
    std::fs::create_dir_all(&textures).unwrap();
    let spec = DdsSpec::cube(4, 3, Format::Bgra8, Header::Legacy);
    for name in names {
        spec.write(&textures, name);
    }
    root
}

#[test]
fn fresh_array_holds_only_the_black_slot() {
    let Some((device, queue)) = test_device("fresh_array_holds_only_the_black_slot") else {
        return;
    };
    let root = root_with_cubes(&[]);
    let mut cubes = CubeArray::new(&device, &queue, root.path());
    assert_eq!(cubes.views().len(), CUBE_SLOTS as usize);
    assert!(!cubes.take_changed());
}

#[test]
fn acquire_dedups_by_path_ignoring_case_and_separators() {
    let Some((device, queue)) = test_device("acquire_dedups_by_path") else {
        return;
    };
    let root = root_with_cubes(&["a.dds", "b.dds"]);
    let mut cubes = CubeArray::new(&device, &queue, root.path());

    let a = cubes.acquire("a.dds");
    assert_ne!(a, CubeHandle::NONE);
    assert_eq!(cubes.acquire("A.DDS"), a);
    let b = cubes.acquire("b.dds");
    assert_eq!((a.slot(), b.slot()), (1, 2));
    assert_eq!(cubes.views().len(), CUBE_SLOTS as usize);
    assert!(cubes.take_changed());
    assert!(!cubes.take_changed());
    assert_eq!(cubes.acquire("a.dds"), a);
    assert!(!cubes.take_changed());
}

#[test]
fn unloadable_cube_resolves_to_black_without_marking_changed() {
    let Some((device, queue)) = test_device("unloadable_cube_resolves_to_black") else {
        return;
    };
    let root = root_with_cubes(&[]);
    std::fs::write(root.path().join("textures").join("bad.dds"), [0u8; 64]).unwrap();
    let mut cubes = CubeArray::new(&device, &queue, root.path());

    assert_eq!(cubes.acquire("absent.dds"), CubeHandle::NONE);
    assert_eq!(cubes.acquire("bad.dds"), CubeHandle::NONE);
    assert!(!cubes.take_changed());
}

#[test]
fn exhausted_array_hands_out_the_black_slot() {
    let Some((device, queue)) = test_device("exhausted_array_hands_out_the_black_slot") else {
        return;
    };
    let names: Vec<String> = (0..CUBE_SLOTS).map(|i| format!("c{i}.dds")).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let root = root_with_cubes(&name_refs);
    let mut cubes = CubeArray::new(&device, &queue, root.path());

    let handles: Vec<CubeHandle> = names.iter().map(|name| cubes.acquire(name)).collect();
    let last = CUBE_SLOTS as usize - 1;
    assert!(
        handles[..last]
            .iter()
            .enumerate()
            .all(|(i, handle)| handle.slot() == i as u32 + 1)
    );
    assert_eq!(handles[last], CubeHandle::NONE);
    assert_eq!(cubes.views().len(), CUBE_SLOTS as usize);
}

const SAMPLE_SHADER: &str = "
    @group(0) @binding(0) var cube: texture_cube<f32>;
    @group(0) @binding(1) var cube_sampler: sampler;
    @group(0) @binding(2) var<storage, read_write> colors: array<vec4<f32>, 6>;

    @compute @workgroup_size(6)
    fn main(@builtin(local_invocation_index) i: u32) {
        var dirs = array<vec3<f32>, 6>(
            vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(-1.0, 0.0, 0.0),
            vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, -1.0, 0.0),
            vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 0.0, -1.0));
        colors[i] = textureSampleLevel(cube, cube_sampler, dirs[i], 1.0);
    }";

fn sample_axes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view: &wgpu::TextureView,
) -> [[f32; 4]; 6] {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("cube_sample_test"),
        source: wgpu::ShaderSource::Wgsl(SAMPLE_SHADER.into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("cube_sample_test"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let output_size = std::mem::size_of::<[[f32; 4]; 6]>() as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cube_sample_out"),
        size: output_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cube_sample_staging"),
        size: output_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, output_size);
    queue.submit(std::iter::once(encoder.finish()));
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
        .expect("cube sample poll failed");
    receiver.recv().unwrap().expect("staging buffer maps");
    let mapped = staging
        .slice(..)
        .get_mapped_range()
        .expect("staging range is mapped");
    *bytemuck::from_bytes::<[[f32; 4]; 6]>(&mapped)
}

#[test]
fn loaded_cube_returns_each_face_colour_along_its_axis() {
    let Some((device, queue)) = test_device("loaded_cube_returns_each_face_colour") else {
        return;
    };
    let root = root_with_cubes(&["faces.dds"]);
    let mut cubes = CubeArray::new(&device, &queue, root.path());
    let handle = cubes.acquire("faces.dds");

    let sampled = sample_axes(&device, &queue, cubes.views()[handle.slot() as usize]);
    for (face, color) in sampled.iter().enumerate() {
        let expected = FACE_COLORS[face].map(|channel| f32::from(channel) / 255.0);
        for channel in 0..3 {
            assert!(
                (color[channel] - expected[channel]).abs() < 0.02,
                "face {face}: got {color:?}, expected {expected:?}"
            );
        }
    }
}

#[test]
fn black_slot_samples_black_on_every_axis() {
    let Some((device, queue)) = test_device("black_slot_samples_black") else {
        return;
    };
    let root = root_with_cubes(&[]);
    let cubes = CubeArray::new(&device, &queue, root.path());
    let sampled = sample_axes(&device, &queue, cubes.views()[0]);
    assert!(sampled.iter().all(|c| c[..3].iter().all(|&v| v == 0.0)));
}

#[test]
fn the_shader_declares_the_cube_array_with_one_element_per_slot() {
    let declaration = include_str!("../shaders/env_bindings.wgsl");
    let expected = format!("binding_array<texture_cube<f32>, {CUBE_SLOTS}>");
    assert!(
        declaration.contains(&expected),
        "env_bindings.wgsl must declare {CUBE_SLOTS} cube elements: {declaration}"
    );
}
