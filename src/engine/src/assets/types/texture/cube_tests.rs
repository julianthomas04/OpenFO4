use tempfile::TempDir;

use super::cube_test_support::{DdsSpec, Format, Header};
use super::{TextureError, load_cube, read_cube_layout, read_layout};

fn load(spec: DdsSpec) -> Result<super::CubeImage, TextureError> {
    let directory = TempDir::new().unwrap();
    let path = spec.write(directory.path(), "cube.dds");
    load_cube(path.to_str().unwrap())
}

fn assert_faces_match(spec: &DdsSpec, image: &super::CubeImage) {
    assert_eq!(image.size, spec.width);
    assert_eq!(image.mip_count, spec.mips);
    for face in 0..6 {
        assert_eq!(image.faces[face].len(), spec.mips as usize);
        for level in 0..spec.mips {
            assert_eq!(
                image.faces[face][level as usize],
                spec.mip_pixels(face, level),
                "face {face} mip {level}"
            );
        }
    }
}

#[test]
fn legacy_uncompressed_cube_loads_every_face_and_mip() {
    let spec = DdsSpec::cube(16, 5, Format::Bgra8, Header::Legacy);
    let image = load(spec).unwrap();
    assert_eq!(image.format, wgpu::TextureFormat::Bgra8Unorm);
    assert_faces_match(&spec, &image);
}

#[test]
fn legacy_bc1_cube_loads_full_chain_down_to_one_pixel() {
    let spec = DdsSpec::cube(16, 5, Format::Bc1, Header::Legacy);
    let image = load(spec).unwrap();
    assert_eq!(image.format, wgpu::TextureFormat::Bc1RgbaUnorm);
    assert_eq!((image.block_bytes, image.block_dim), (8, 4));
    assert_faces_match(&spec, &image);
}

#[test]
fn dx10_cube_loads_in_both_formats() {
    for format in [Format::Bgra8, Format::Bc1] {
        let spec = DdsSpec::cube(8, 4, format, Header::Dx10 { array_size: 1 });
        assert_faces_match(&spec, &load(spec).unwrap());
    }
}

#[test]
fn cube_layout_reports_face_geometry() {
    let directory = TempDir::new().unwrap();
    let spec = DdsSpec::cube(8, 4, Format::Bgra8, Header::Legacy);
    let path = spec.write(directory.path(), "cube.dds");
    let layout = read_cube_layout(path.to_str().unwrap()).unwrap();
    assert_eq!((layout.width, layout.height, layout.mip_count), (8, 8, 4));
    assert_eq!(layout.data_offset, 128);
}

#[test]
fn cube_array_of_more_than_one_cube_is_rejected() {
    let spec = DdsSpec::cube(8, 1, Format::Bgra8, Header::Dx10 { array_size: 2 });
    assert!(matches!(
        load(spec),
        Err(TextureError::CubeArray { array_size: 2, .. })
    ));
}

#[test]
fn non_square_cube_is_rejected() {
    let mut spec = DdsSpec::cube(8, 1, Format::Bgra8, Header::Legacy);
    spec.height = 4;
    assert!(matches!(
        load(spec),
        Err(TextureError::NonSquareCube {
            width: 8,
            height: 4,
            ..
        })
    ));
}

#[test]
fn legacy_cube_without_all_face_flags_is_incomplete() {
    let mut spec = DdsSpec::cube(8, 1, Format::Bgra8, Header::Legacy);
    spec.face_flags = false;
    assert!(matches!(
        load(spec),
        Err(TextureError::IncompleteCube { .. })
    ));
}

#[test]
fn truncated_cube_reports_expected_and_actual_bytes() {
    let mut spec = DdsSpec::cube(8, 4, Format::Bgra8, Header::Legacy);
    spec.drop_tail_bytes = 1;
    let expected: u64 = (0..4)
        .map(|level| spec.mip_bytes(level) as u64)
        .sum::<u64>()
        * 6;
    match load(spec) {
        Err(TextureError::Truncated {
            expected: e,
            actual: a,
            ..
        }) => assert_eq!((e, a), (expected, expected - 1)),
        other => panic!("expected Truncated, got {other:?}"),
    }
}

#[test]
fn compressed_cube_with_misaligned_base_is_rejected() {
    let spec = DdsSpec::cube(6, 1, Format::Bc1, Header::Legacy);
    assert!(matches!(
        load(spec),
        Err(TextureError::BlockMisaligned { width: 6, .. })
    ));
}

#[test]
fn plain_2d_texture_through_the_cube_entry_point_is_not_a_cube() {
    let mut spec = DdsSpec::cube(8, 1, Format::Bgra8, Header::Legacy);
    spec.cube = false;
    assert!(matches!(load(spec), Err(TextureError::NotCube { .. })));

    let mut dx10 = DdsSpec::cube(8, 1, Format::Bgra8, Header::Dx10 { array_size: 1 });
    dx10.cube = false;
    assert!(matches!(load(dx10), Err(TextureError::NotCube { .. })));
}

#[test]
fn cube_through_the_2d_entry_point_is_not_a_single_surface() {
    let directory = TempDir::new().unwrap();
    for header in [Header::Legacy, Header::Dx10 { array_size: 1 }] {
        let spec = DdsSpec::cube(8, 1, Format::Bgra8, header);
        let path = spec.write(directory.path(), "cube.dds");
        assert!(matches!(
            read_layout(path.to_str().unwrap()),
            Err(TextureError::NotSingleSurface { layers: 6, .. })
        ));
    }
}

#[test]
fn missing_file_is_an_io_error_and_garbage_is_bad_magic() {
    let directory = TempDir::new().unwrap();
    let missing = directory.path().join("absent.dds");
    assert!(matches!(
        load_cube(missing.to_str().unwrap()),
        Err(TextureError::Io { .. })
    ));
    let garbage = directory.path().join("garbage.dds");
    std::fs::write(&garbage, [0u8; 200]).unwrap();
    assert!(matches!(
        load_cube(garbage.to_str().unwrap()),
        Err(TextureError::BadMagic { .. })
    ));
}

#[test]
fn uncompressed_dx10_2d_textures_load_on_the_2d_path() {
    let directory = TempDir::new().unwrap();
    for (format, expected) in [
        (Format::Rgba8, wgpu::TextureFormat::Rgba8Unorm),
        (Format::Bgra8, wgpu::TextureFormat::Bgra8Unorm),
    ] {
        let spec = DdsSpec {
            cube: false,
            ..DdsSpec::cube(8, 4, format, Header::Dx10 { array_size: 1 })
        };
        let path = spec.write(directory.path(), "flat.dds");
        let layout = read_layout(path.to_str().unwrap()).expect("2D layout");
        assert_eq!(layout.format, expected);
        assert_eq!((layout.width, layout.height, layout.mip_count), (8, 8, 4));
    }
}
