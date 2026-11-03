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
    pack_glb_parts(&[GlbPart {
        positions,
        normals,
        uvs,
        indices,
        material: json!({"name": material}),
    }])
}

/// One primitive of [`pack_glb_parts`], with its glTF material object.
struct GlbPart<'a> {
    positions: &'a [f32],
    normals: &'a [f32],
    uvs: &'a [f32],
    indices: &'a [u16],
    material: serde_json::Value,
}

/// A GLB with one node and one primitive per part, each with its own material.
fn pack_glb_parts(parts: &[GlbPart<'_>]) -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut primitives = Vec::new();
    let mut push = |bytes: Vec<u8>, views: &mut Vec<serde_json::Value>| {
        let view = json!({"buffer": 0, "byteOffset": bin.len(), "byteLength": bytes.len()});
        bin.extend(bytes);
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        views.push(view);
        views.len() - 1
    };
    let floats =
        |values: &[f32]| -> Vec<u8> { values.iter().flat_map(|f| f.to_le_bytes()).collect() };
    for (index, part) in parts.iter().enumerate() {
        let vertex_count = part.positions.len() / 3;
        let axis = |axis: usize| part.positions.iter().skip(axis).step_by(3).copied();
        let min: Vec<f32> = (0..3).map(|a| axis(a).fold(f32::MAX, f32::min)).collect();
        let max: Vec<f32> = (0..3).map(|a| axis(a).fold(f32::MIN, f32::max)).collect();
        let position = push(floats(part.positions), &mut views);
        let normal = push(floats(part.normals), &mut views);
        let uv = push(floats(part.uvs), &mut views);
        let indices = push(
            part.indices.iter().flat_map(|i| i.to_le_bytes()).collect(),
            &mut views,
        );
        let first = accessors.len();
        accessors.extend([
            json!({"bufferView": position, "componentType": 5126, "count": vertex_count,
                   "type": "VEC3", "min": min, "max": max}),
            json!({"bufferView": normal, "componentType": 5126, "count": vertex_count,
                   "type": "VEC3"}),
            json!({"bufferView": uv, "componentType": 5126, "count": vertex_count,
                   "type": "VEC2"}),
            json!({"bufferView": indices, "componentType": 5123,
                   "count": part.indices.len(), "type": "SCALAR"}),
        ]);
        primitives.push(json!({
            "attributes": {"POSITION": first, "NORMAL": first + 1, "TEXCOORD_0": first + 2},
            "indices": first + 3,
            "material": index,
        }));
    }

    let document = json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": primitives}],
        "materials": parts.iter().map(|part| part.material.clone()).collect::<Vec<_>>(),
        "buffers": [{"byteLength": bin.len()}],
        "bufferViews": views,
        "accessors": accessors,
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

/// The effect material of one plane of a test stage: the fields the converter writes.
#[derive(Debug, Clone)]
pub struct TestEffect {
    /// Below the textures folder.
    pub texture: String,
    /// Additive by source alpha like the game's overlay, or opaque.
    pub additive: bool,
    pub base_color: [f32; 4],
}

impl TestEffect {
    fn extras(&self) -> serde_json::Value {
        json!({"effect": self.material()})
    }

    /// The material as the converter writes it into `extras.effect`.
    fn material(&self) -> serde_json::Value {
        let blend = if self.additive {
            json!({"mode": "add", "src": "src_alpha", "dst": "one", "alpha_test": false})
        } else {
            json!({"mode": "none", "alpha_test": false})
        };
        json!({
            "v": 1, "vs": "effect_vs_00000004", "ps": "effect_ps_00000004",
            "flags1": 0, "flags2": 0, "blend": blend,
            "z_test": !self.additive, "z_write": false, "double_sided": false,
            "clamp": "wrap_s_wrap_t",
            "textures": {"source": self.texture, "greyscale": "", "normal": "", "env": "",
                         "env_mask": "", "reflectance": "", "lighting": "",
                         "emit_gradient": ""},
            "base_color": self.base_color, "base_color_scale": 1.0,
            "falloff": {"start_angle": 1.0, "stop_angle": 1.0, "start_opacity": 0.0,
                        "stop_opacity": 0.0},
            "soft_falloff_depth": 10.0, "uv_offset": [0.0, 0.0], "uv_scale": [1.0, 1.0],
            "lighting_influence": 1.0, "env_map_scale": 1.0, "emittance_color": [0.0, 0.0, 0.0],
            "refraction_power": 0.0, "alpha": 1.0, "controllers": []
        })
    }
}

/// One plane of a test stage facing the camera, in engine units.
#[derive(Debug, Clone)]
pub struct TestStageLayer {
    pub half_extent: [f32; 2],
    pub depth: f32,
    pub u_max: f32,
    /// `None` writes a plain material, which no stage accepts.
    pub effect: Option<TestEffect>,
}

/// Below the textures folder: the background texture of [`TestStageLayer::vanilla`].
pub const STAGE_BACKGROUND_TEXTURE: &str = "interface/loadingmenubg.dds";
/// Below the textures folder: the overlay texture of [`TestStageLayer::vanilla`].
pub const STAGE_OVERLAY_TEXTURE: &str = "interface/objects/loadscreen/overlay01_d.dds";

impl TestStageLayer {
    /// The plane's effect material as the converter writes it.
    pub fn effect_json(&self) -> serde_json::Value {
        self.effect
            .as_ref()
            .expect("the plane has an effect material")
            .material()
    }

    /// The planes of the game's 16:9 stage in its file order: the additive overlay at 0.6,
    /// then the opaque background.
    pub fn vanilla() -> [Self; 2] {
        [
            Self {
                half_extent: [18.65, 10.5],
                depth: -1.6756,
                u_max: 0.89,
                effect: Some(TestEffect {
                    texture: STAGE_OVERLAY_TEXTURE.to_string(),
                    additive: true,
                    base_color: [1.0, 1.0, 1.0, 0.6],
                }),
            },
            Self {
                half_extent: [55.75, 31.375],
                depth: -100.0044,
                u_max: 0.889,
                effect: Some(TestEffect {
                    texture: STAGE_BACKGROUND_TEXTURE.to_string(),
                    additive: false,
                    base_color: [1.0; 4],
                }),
            },
        ]
    }
}

/// Writes a stage of `layers` to `dir/<name>` and returns its path. Each plane's u runs from 0
/// at its left edge to its `u_max` at its right edge, v from 0 at the top to 1 at the bottom.
pub fn write_stage(dir: &Path, name: &str, layers: &[TestStageLayer]) -> PathBuf {
    const CORNERS: [(f32, f32); 4] = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
    let geometry: Vec<(Vec<f32>, Vec<f32>, Vec<f32>)> = layers
        .iter()
        .map(|layer| {
            let [half_x, half_y] = layer.half_extent;
            let positions = CORNERS
                .iter()
                .flat_map(|(x, y)| [x * half_x, y * half_y, layer.depth])
                .collect();
            let normals = CORNERS.iter().flat_map(|_| [0.0, 0.0, 1.0]).collect();
            let uvs = CORNERS
                .iter()
                .flat_map(|(x, y)| {
                    [
                        f32::midpoint(*x, 1.0) * layer.u_max,
                        f32::midpoint(1.0, -*y),
                    ]
                })
                .collect();
            (positions, normals, uvs)
        })
        .collect();
    let indices = [0, 1, 2, 0, 2, 3];
    let parts: Vec<GlbPart<'_>> = layers
        .iter()
        .zip(&geometry)
        .map(|(layer, (positions, normals, uvs))| GlbPart {
            positions,
            normals,
            uvs,
            indices: &indices,
            material: match &layer.effect {
                Some(effect) => json!({"name": effect.texture, "extras": effect.extras()}),
                None => json!({"name": "plain"}),
            },
        })
        .collect();
    std::fs::create_dir_all(dir).expect("the temp folder is writable");
    let path = dir.join(name);
    std::fs::write(&path, pack_glb_parts(&parts)).expect("the temp folder is writable");
    path
}

/// The widths of the game's three stages relative to the 16:9 one, in the order of
/// [`super::stage::STAGE_MODELS`].
const STAGE_WIDTHS: [f32; 3] = [1.0, 48.4 / 37.3, 74.0 / 37.3];

/// Installs the game's three stage widths below `asset_root`, each [`TestStageLayer::vanilla`]
/// widened like the game's files, with one-colour `background` and `overlay` textures.
pub fn install_stage(asset_root: &Path, background: [u8; 4], overlay: [u8; 4]) {
    for (path, width) in super::stage::stage_paths(asset_root)
        .into_iter()
        .zip(STAGE_WIDTHS)
    {
        let path = PathBuf::from(path.to_string_lossy().replace('\\', "/"));
        let layers = TestStageLayer::vanilla().map(|mut layer| {
            layer.half_extent[0] *= width;
            layer
        });
        write_stage(
            path.parent().expect("a stage is in a folder"),
            &path.file_name().expect("a stage file").to_string_lossy(),
            &layers,
        );
    }
    write_solid_texture(asset_root, STAGE_BACKGROUND_TEXTURE, background);
    write_solid_texture(asset_root, STAGE_OVERLAY_TEXTURE, overlay);
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

/// A GLB of two overlapping quads facing the viewer, indexed near one first so that a pass
/// without a depth test would paint the far one over it. The near quad (normal towards the
/// viewer) is at depth 0, the far quad (normal up) 3 units behind it.
pub fn layered_quads_glb(material: &str) -> Vec<u8> {
    let mut positions: Vec<f32> = Vec::new();
    let mut normals: Vec<f32> = Vec::new();
    let mut uvs: Vec<f32> = Vec::new();
    let mut indices: Vec<u16> = Vec::new();
    for (quad, (depth, half, normal)) in [(0.0, 5.0, [0.0, 0.0, 1.0]), (-3.0, 6.0, [0.0, 1.0, 0.0])]
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
    write_solid_texture(dir, name, [255, 255, 255, alpha]);
}

/// Writes `textures/<name>`, a 2x2 texture whose texels all are `rgba`; `name` may name a
/// folder below `textures`.
pub fn write_solid_texture(dir: &Path, name: &str, rgba: [u8; 4]) {
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
    for texel in dds.data.as_chunks_mut::<4>().0 {
        *texel = rgba;
    }
    write_dds(dir, name, &dds);
}

fn write_dds(dir: &Path, name: &str, dds: &ddsfile::Dds) {
    let path = dir.join("textures").join(name);
    std::fs::create_dir_all(path.parent().expect("a texture is in a folder"))
        .expect("the temp folder is writable");
    let mut file = std::fs::File::create(path).expect("the texture is writable");
    dds.write(&mut file).expect("the texture is written");
}
