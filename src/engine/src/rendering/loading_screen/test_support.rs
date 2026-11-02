//! Synthetic models for the loading art's tests: a cube written as a GLB, never game data.

use std::path::{Path, PathBuf};

use serde_json::json;

use super::art::{LoadingArt, ModelTransform};

/// Half the edge of the test cube, in engine units: at the fixture's 30 units distance it spans
/// about a fifth of the view's height.
const HALF_EDGE: f32 = 5.0;

/// One face per (normal, u axis, v axis) triple of a cube.
const FACES: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
    ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
    ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
    ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
    ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
    ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
];

fn scaled(vector: [f32; 3], by: f32) -> [f32; 3] {
    vector.map(|component| component * by)
}

fn plus(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// A GLB of a cube centred on the origin whose one material is named `material`.
pub fn cube_glb(material: &str) -> Vec<u8> {
    let mut positions: Vec<f32> = Vec::new();
    let mut normals: Vec<f32> = Vec::new();
    let mut uvs: Vec<f32> = Vec::new();
    let mut indices: Vec<u16> = Vec::new();
    for (face, (normal, u, v)) in FACES.iter().enumerate() {
        let first = u16::try_from(face * 4).expect("a cube has few vertices");
        for (du, dv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let corner = plus(
                plus(scaled(*normal, HALF_EDGE), scaled(*u, du * HALF_EDGE)),
                scaled(*v, dv * HALF_EDGE),
            );
            positions.extend(corner);
            normals.extend(normal);
            uvs.extend([du * 0.5 + 0.5, dv * 0.5 + 0.5]);
        }
        indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    }

    pack_glb(&positions, &normals, &uvs, &indices, material)
}

/// A single-primitive GLB of the given vertex data, whose one material is named `material`.
fn pack_glb(
    positions: &[f32],
    normals: &[f32],
    uvs: &[f32],
    indices: &[u16],
    material: &str,
) -> Vec<u8> {
    let vertex_count = positions.len() / 3;
    let axis = |axis: usize| positions.iter().skip(axis).step_by(3).copied();
    let min: Vec<f32> = (0..3).map(|a| axis(a).fold(f32::MAX, f32::min)).collect();
    let max: Vec<f32> = (0..3).map(|a| axis(a).fold(f32::MIN, f32::max)).collect();
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut push = |bytes: Vec<u8>| {
        let view = json!({"buffer": 0, "byteOffset": bin.len(), "byteLength": bytes.len()});
        bin.extend(bytes);
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        views.push(view);
    };
    push(positions.iter().flat_map(|f| f.to_le_bytes()).collect());
    push(normals.iter().flat_map(|f| f.to_le_bytes()).collect());
    push(uvs.iter().flat_map(|f| f.to_le_bytes()).collect());
    push(indices.iter().flat_map(|i| i.to_le_bytes()).collect());

    let document = json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{
            "attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2},
            "indices": 3,
            "material": 0,
        }]}],
        "materials": [{"name": material}],
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": views,
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": vertex_count, "type": "VEC3",
             "min": min, "max": max},
            {"bufferView": 1, "componentType": 5126, "count": vertex_count, "type": "VEC3"},
            {"bufferView": 2, "componentType": 5126, "count": vertex_count, "type": "VEC2"},
            {"bufferView": 3, "componentType": 5123, "count": indices.len(), "type": "SCALAR"},
        ],
    });
    let mut json = serde_json::to_vec(&document).expect("the document serialises");
    json.resize(json.len().next_multiple_of(4), b' ');
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut glb = Vec::with_capacity(total);
    glb.extend(b"glTF");
    glb.extend(2_u32.to_le_bytes());
    glb.extend(u32::try_from(total).expect("a small file").to_le_bytes());
    glb.extend(
        u32::try_from(json.len())
            .expect("a small file")
            .to_le_bytes(),
    );
    glb.extend(b"JSON");
    glb.extend(&json);
    glb.extend(
        u32::try_from(bin.len())
            .expect("a small file")
            .to_le_bytes(),
    );
    glb.extend(b"BIN\0");
    glb.extend(&bin);
    glb
}

/// Writes the cube to `dir` and returns its path.
pub fn write_cube(dir: &Path, material: &str) -> PathBuf {
    let path = dir.join("cube.glb");
    std::fs::write(&path, cube_glb(material)).expect("the temp folder is writable");
    path
}

/// Art placing the cube 30 engine units (300 game units) straight ahead, turnable freely.
pub fn cube_art(dir: &Path, material: &str) -> LoadingArt {
    LoadingArt {
        model_path: write_cube(dir, material),
        asset_root: dir.to_path_buf(),
        transform: ModelTransform {
            position: [0.0, 300.0, 0.0],
            rotation_radians: [0.0; 3],
            scale: 1.0,
            around_origin: false,
        },
        rotation_limits_deg: (-180, 180),
        zoom_limits: (-0.5, 1.5),
        no_rotation: false,
    }
}

/// A GLB of two overlapping quads facing the viewer, indexed far one first so that a pass
/// that blends in index order draws the near one over the far one. The near quad (normal
/// towards the viewer) is at depth 0, the far quad (normal up) 3 units behind it.
pub fn layered_quads_glb(material: &str) -> Vec<u8> {
    let mut positions: Vec<f32> = Vec::new();
    let mut normals: Vec<f32> = Vec::new();
    let mut uvs: Vec<f32> = Vec::new();
    let mut indices: Vec<u16> = Vec::new();
    for (quad, (depth, half, normal)) in [(-3.0, 6.0, [0.0, 1.0, 0.0]), (0.0, 5.0, [0.0, 0.0, 1.0])]
        .into_iter()
        .enumerate()
    {
        let first = u16::try_from(quad * 4).expect("two quads have few vertices");
        for (u, v) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            positions.extend([u * half, v * half, depth]);
            normals.extend(normal);
            uvs.extend([u * 0.5 + 0.5, v * 0.5 + 0.5]);
        }
        indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    }
    pack_glb(&positions, &normals, &uvs, &indices, material)
}

/// Art like [`cube_art`] but of the layered quads.
pub fn layered_quads_art(dir: &Path) -> LoadingArt {
    let mut art = cube_art(dir, "");
    art.model_path = dir.join("quads.glb");
    std::fs::write(&art.model_path, layered_quads_glb("")).expect("the temp folder is writable");
    art
}

/// Writes `textures/<name>`, a 2x2 texture whose texels all have the given alpha and are white.
pub fn write_texture(dir: &Path, name: &str, alpha: u8) {
    let mut dds = ddsfile::Dds::new_dxgi(ddsfile::NewDxgiParams {
        height: 2,
        width: 2,
        depth: None,
        format: ddsfile::DxgiFormat::R8G8B8A8_UNorm,
        mipmap_levels: Some(1),
        array_layers: None,
        caps2: None,
        is_cubemap: false,
        resource_dimension: ddsfile::D3D10ResourceDimension::Texture2D,
        alpha_mode: ddsfile::AlphaMode::Unknown,
    })
    .expect("a valid texture description");
    for texel in dds.data.chunks_exact_mut(4) {
        texel.copy_from_slice(&[255, 255, 255, alpha]);
    }
    let folder = dir.join("textures");
    std::fs::create_dir_all(&folder).expect("the temp folder is writable");
    let mut file = std::fs::File::create(folder.join(name)).expect("the texture is writable");
    dds.write(&mut file).expect("the texture is written");
}
