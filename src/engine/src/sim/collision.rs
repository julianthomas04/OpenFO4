//! CPU-side collision specs: pure data (glam + serde only) so the streaming
//! worker thread can parse sidecars and assemble batches without touching
//! rapier. The sim turns these into colliders in `physics.rs`.

use std::sync::Arc;

use glam::{Quat, Vec3};

use super::water_materials::WaterBodyProps;

/// One collision shape in a GLB's local space. Sidecar writers already
/// applied the engine convention (Y-up, x0.1 scale, Z negated), so these
/// compose with the same REFR world transform the renderer uses.
#[derive(Debug, Clone)]
pub enum ColShape {
    Box {
        center: Vec3,
        rotation: Quat,
        half_extents: Vec3,
    },
    Sphere {
        center: Vec3,
        radius: f32,
    },
    /// `height` is the full capsule height including both caps (Godot
    /// `CapsuleShape3D` convention — what both sidecar writers emit).
    Capsule {
        center: Vec3,
        rotation: Quat,
        radius: f32,
        height: f32,
    },
    Convex {
        center: Vec3,
        rotation: Quat,
        vertices: Vec<Vec3>,
    },
    Trimesh {
        vertices: Vec<Vec3>,
        indices: Vec<[u32; 3]>,
    },
}

/// A collision shape plus the model node that owns it. Vertices stay in
/// model space at the rest pose; `joint` is the animated joint the shape
/// follows, resolved from `node` once the model's skeleton is known.
#[derive(Debug, Clone)]
pub struct PartShape {
    pub shape: ColShape,
    /// Owning node name from the sidecar; `None` for sidecars without node data.
    pub node: Option<String>,
    /// Index of the animated joint carrying the shape; `None` = static.
    pub joint: Option<u16>,
    /// CRC-32 of the lowercased MATT name from the sidecar; 0 = none.
    pub material: u32,
}

/// World placement of one REFR (uniform scale only — REFR scale is a single
/// float; shape data is pre-scaled at collider build time because rapier
/// poses carry no scale).
#[derive(Debug, Clone)]
pub struct Placement {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: f32,
    /// REFR form key ("001ED5:Fallout4.esm") — the game-wide unique
    /// placement id, attribution only (never a cache key).
    pub ref_key: Option<String>,
    /// The REFR's raw XRGD ragdoll pose (see `ragdoll_pose`); used only when
    /// the mesh is ragdoll-capable.
    pub ragdoll_pose: Option<Arc<[u8]>>,
    /// Set for the parts of an assembled weapon: where this part sits relative
    /// to the REFR. All parts of one REFR share its pose and `ref_key` and
    /// form one compound body.
    pub part_local: Option<PartLocal>,
}

/// A weapon part's pose relative to its REFR, translation unscaled (the
/// placement's scale applies to it, like the REFR scales the whole weapon).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartLocal {
    pub translation: Vec3,
    pub rotation: Quat,
}

impl Placement {
    /// Whether every number a collider pose is built from is finite. A degenerate REFR scale
    /// decomposes to a NaN rotation; a collider with a NaN pose has a NaN broad-phase centre,
    /// which makes parry's binned BVH build index past its last bin.
    pub fn is_finite(&self) -> bool {
        self.translation.is_finite()
            && self.rotation.is_finite()
            && self.scale.is_finite()
            && self
                .part_local
                .is_none_or(|local| local.translation.is_finite() && local.rotation.is_finite())
    }
}

/// All placements of one mesh path within a cell. Carries only the cheap
/// metadata (dynamic/mass, from the `collision` table's promoted columns); the
/// shapes themselves are loaded lazily at bake time, and only on a cold
/// `cache/physics` miss — a warm launch bakes from `.colbin` and never reads the
/// shape BLOB. See `ShapeBakeCache::bake_cell` and the loader passed to it.
pub struct ObjectColliderGroup {
    /// GLB mesh path — the shape-cache key, the lazy shape-loader key, and the
    /// diagnostics label.
    pub path: String,
    pub placements: Vec<Placement>,
    /// Havok dynamic-body flag from the `.col.json` sidecar (the installer
    /// derives it from the FO4 packfile's dyn-motion/dyn-inertia arrays). When
    /// true, every placement of this mesh becomes a rapier rigid body rather
    /// than a static collider.
    pub dynamic: bool,
    /// Body mass (engine units) when `dynamic`; `None` falls back to rapier's
    /// shape-volume × default density.
    pub mass: Option<f32>,
    /// Full path of the loose `.col.json` carrying the v2 `bodies`/`constraints`
    /// when the manifest marks the mesh ragdoll-capable. The streaming worker
    /// parses it while baking the cell (`ShapeBakeCache::bake_cell`); the sim never reads it.
    pub ragdoll_sidecar: Option<String>,
    /// Collision layer and materials of the dynamic body (manifest row element 4); `None` for a
    /// manifest written before water data existed or for a static mesh.
    pub water: Option<Arc<WaterBodyProps>>,
}

/// The cheap per-mesh collision metadata read at cell-assembly time: enough to
/// decide whether a mesh has collision at all and whether it is a dynamic body,
/// without touching the (potentially multi-MB) shape BLOB.
#[derive(Clone)]
pub struct CollisionMeta {
    pub dynamic: bool,
    pub mass: Option<f32>,
    /// The sidecar carries v2 ragdoll `bodies` (manifest row element 3).
    pub ragdoll: bool,
    /// The sidecar's top-level `water` object (manifest row element 4), present for dynamic meshes.
    pub water: Option<Arc<WaterBodyProps>>,
}

/// Terrain triangle mesh in engine space, copied from the same vertices the
/// renderer uploads so collision exactly matches the visual.
pub struct TerrainColliderSpec {
    pub vertices: Vec<Vec3>,
    pub indices: Vec<[u32; 3]>,
}

/// How a cell's ground is represented in its collision batch. `Absent` is a cell with no LAND
/// anywhere in its worldspace chain (or an interior): its ground, if any, is its objects.
/// `Failed` is a cell whose LAND exists but could not be turned into a mesh.
#[derive(Default)]
pub enum TerrainCollision {
    Present(TerrainColliderSpec),
    #[default]
    Absent,
    Failed,
}

impl TerrainCollision {
    pub fn spec(&self) -> Option<&TerrainColliderSpec> {
        match self {
            Self::Present(spec) => Some(spec),
            Self::Absent | Self::Failed => None,
        }
    }
}

#[derive(Default)]
pub struct CellColliderBatch {
    pub terrain: TerrainCollision,
    pub objects: Vec<ObjectColliderGroup>,
}

/// Parsed `.col.json`: just the shapes. The Havok `dynamic`/`mass` body metadata
/// that also lives in the sidecar is NOT read here — the engine takes those from
/// the collision manifest at cell assembly (see [`CollisionMeta`]); the sidecar is
/// read only lazily, for its geometry, on a cold bake. Cheap to clone (`Arc`-shared).
#[derive(Clone)]
pub struct SidecarData {
    pub shapes: Arc<Vec<PartShape>>,
}

// The `.col.json` may also carry top-level `dynamic`/`mass`; serde ignores them
// here on purpose (the manifest is the single source of truth for those).
#[derive(serde::Deserialize)]
struct RawSidecar {
    shapes: Vec<RawShape>,
}

/// Mirror of the `.col.json` schema written by `fo4_collision_worker.py`: a
/// `type` tag plus optional fields per type.
#[derive(serde::Deserialize)]
pub(super) struct RawShape {
    #[serde(rename = "type")]
    pub(super) kind: String,
    pub(super) center: Option<[f32; 3]>,
    /// Quaternion as [x, y, z, w].
    pub(super) rotation: Option<[f32; 4]>,
    pub(super) radius: Option<f32>,
    pub(super) half_extents: Option<[f32; 3]>,
    pub(super) height: Option<f32>,
    pub(super) vertices: Option<Vec<[f32; 3]>>,
    pub(super) indices: Option<Vec<[u32; 3]>>,
    /// Name of the model node owning the shape (animated models only).
    pub(super) node: Option<String>,
    /// BGS material hash; absent or 0 = none.
    pub(super) material: Option<u32>,
}

/// Read and parse a loose `.col.json` sidecar file. `None` = missing, unreadable,
/// or unparseable (caller spawns no collider). Read lazily — only on a cold
/// `cache/physics` miss, when the bake actually needs the geometry.
pub fn load_sidecar_file(col_json_path: &str) -> Option<SidecarData> {
    let bytes = std::fs::read(col_json_path).ok()?;
    parse_sidecar_bytes(&bytes, col_json_path)
}

/// The manifest key for a runtime mesh path: lowercased and backslash-separated,
/// matching how the installer's worker builds keys ("meshes\\<rel>.glb" lowercased,
/// `/`->`\`). The runtime path starts with the `meshes\` root and may carry a `/`
/// from a MODL string (`model_path` is lowercased in the DB but not separator-
/// normalized), so both transforms are needed.
pub fn manifest_key(mesh_path: &str) -> String {
    mesh_path.replace('/', "\\").to_lowercase()
}

/// Load the collision metadata manifest the installer emits next to the loose
/// `.col.json` sidecars: a JSON object mapping each collision mesh key
/// (see [`manifest_key`]) to `[is_dynamic (0|1), mass (or null)]` or `[is_dynamic, mass, ragdoll (0|1)]`. The returned
/// map answers, per mesh, "does this have collision" (key present) and "is it
/// dynamic + mass", with no filesystem access — so cell assembly never opens a
/// sidecar. A missing or unparseable manifest is a whole-world collision loss, so
/// it is logged at `error!` (not `warn!`) and yields an empty map.
pub fn load_manifest(path: &str) -> std::collections::HashMap<String, CollisionMeta> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(
                "collision: manifest {path} unreadable: {e}; ALL object collision disabled"
            );
            return std::collections::HashMap::new();
        }
    };
    let raw: std::collections::HashMap<String, ManifestRow> = match serde_json::from_slice(&bytes) {
        Ok(m) => m,
        Err(e) => {
            tracing::error!(
                "collision: manifest {path} unparseable: {e}; ALL object collision disabled"
            );
            return std::collections::HashMap::new();
        }
    };
    raw.into_iter()
        .map(|(k, row)| (k, row.into_meta()))
        .collect()
}

/// One manifest row: `[dynamic, mass]`, `[dynamic, mass, ragdoll]` for meshes whose sidecar
/// carries ragdoll bodies, or `[dynamic, mass, ragdoll, water]` for dynamic meshes whose sidecar
/// carries water data (the ragdoll flag is then 0 or 1). Every shape is valid; anything else fails
/// the whole manifest parse.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum ManifestRow {
    Plain((u8, Option<f32>)),
    WithRagdoll((u8, Option<f32>, u8)),
    WithWater((u8, Option<f32>, u8, WaterBodyProps)),
}

impl ManifestRow {
    fn into_meta(self) -> CollisionMeta {
        match self {
            Self::Plain((dynamic, mass)) => CollisionMeta {
                dynamic: dynamic != 0,
                mass,
                ragdoll: false,
                water: None,
            },
            Self::WithRagdoll((dynamic, mass, ragdoll)) => CollisionMeta {
                dynamic: dynamic != 0,
                mass,
                ragdoll: ragdoll != 0,
                water: None,
            },
            Self::WithWater((dynamic, mass, ragdoll, water)) => CollisionMeta {
                dynamic: dynamic != 0,
                mass,
                ragdoll: ragdoll != 0,
                water: Some(Arc::new(water)),
            },
        }
    }
}

/// Parse a `.col.json` collision payload from its raw bytes. `label` is the mesh
/// path, used only for diagnostics. `None` means the payload was unparseable —
/// the caller spawns no collider. Malformed individual shapes are skipped with a
/// warning instead of failing the whole sidecar.
pub fn parse_sidecar_bytes(bytes: &[u8], label: &str) -> Option<SidecarData> {
    let raw: RawSidecar = match serde_json::from_slice(bytes) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("collision: unparseable sidecar {label}: {e}");
            return None;
        }
    };
    let total = raw.shapes.len();
    let shapes: Vec<PartShape> = raw
        .shapes
        .into_iter()
        .filter_map(|raw| {
            let node = raw.node.clone();
            let material = raw.material.unwrap_or(0);
            convert_shape(raw).map(|shape| PartShape {
                shape,
                node,
                joint: None,
                material,
            })
        })
        .collect();
    if shapes.len() < total {
        tracing::warn!(
            "collision: {} of {} shapes malformed in {label}",
            total - shapes.len(),
            total
        );
    }
    Some(SidecarData {
        shapes: Arc::new(shapes),
    })
}

pub(super) fn convert_shape(s: RawShape) -> Option<ColShape> {
    let center = s.center.map_or(Vec3::ZERO, Vec3::from);
    let rotation = s.rotation.map_or(Quat::IDENTITY, |q| {
        Quat::from_xyzw(q[0], q[1], q[2], q[3]).normalize()
    });
    match s.kind.as_str() {
        // "aabb" is the C# converter's fallback-box type tag.
        "box" | "aabb" => Some(ColShape::Box {
            center,
            rotation,
            half_extents: Vec3::from(s.half_extents?),
        }),
        "sphere" => Some(ColShape::Sphere {
            center,
            radius: s.radius?,
        }),
        "capsule" => Some(ColShape::Capsule {
            center,
            rotation,
            radius: s.radius?,
            height: s.height?,
        }),
        "convex" => {
            let vertices = s.vertices?;
            if vertices.len() < 4 {
                return None;
            }
            Some(ColShape::Convex {
                center,
                rotation,
                vertices: vertices.into_iter().map(Vec3::from).collect(),
            })
        }
        "trimesh" => {
            let vertices = s.vertices?;
            let indices = s.indices?;
            let n = vertices.len() as u32;
            if vertices.is_empty() || indices.is_empty() {
                return None;
            }
            if indices.iter().any(|tri| tri.iter().any(|&i| i >= n)) {
                return None;
            }
            Some(ColShape::Trimesh {
                vertices: vertices.into_iter().map(Vec3::from).collect(),
                indices,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    //! Sidecar JSON follows the shape table of the collision sidecar format.

    use serde_json::{Value, json};

    use super::*;

    fn convert(shape: Value) -> Option<ColShape> {
        convert_shape(serde_json::from_value(shape).unwrap())
    }

    fn tetrahedron() -> Value {
        json!([
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0]
        ])
    }

    #[test]
    fn box_shape_defaults_to_the_origin_and_identity_rotation() {
        let shape = convert(json!({ "type": "box", "half_extents": [1.0, 2.0, 3.0] })).unwrap();
        assert!(matches!(
            shape,
            ColShape::Box { center, rotation, half_extents }
                if center == Vec3::ZERO && rotation == Quat::IDENTITY && half_extents == Vec3::new(1.0, 2.0, 3.0)
        ));
    }

    #[test]
    fn aabb_is_an_alias_for_box() {
        let shape = convert(json!({ "type": "aabb", "half_extents": [1.0, 1.0, 1.0] }));
        assert!(matches!(shape, Some(ColShape::Box { .. })));
    }

    #[test]
    fn rotations_are_normalised() {
        let shape = convert(json!({
            "type": "capsule", "radius": 1.0, "height": 4.0, "rotation": [0.0, 0.0, 0.0, 4.0]
        }))
        .unwrap();
        assert!(matches!(shape, ColShape::Capsule { rotation, .. } if rotation == Quat::IDENTITY));
    }

    #[test]
    fn shapes_missing_a_required_field_are_dropped() {
        let incomplete = [
            json!({ "type": "box" }),
            json!({ "type": "sphere" }),
            json!({ "type": "capsule", "radius": 1.0 }),
            json!({ "type": "capsule", "height": 2.0 }),
            json!({ "type": "convex" }),
            json!({ "type": "trimesh", "vertices": [[0.0, 0.0, 0.0]] }),
            json!({ "type": "trimesh", "indices": [[0, 0, 0]] }),
            json!({ "type": "cone", "radius": 1.0 }),
        ];
        for shape in incomplete {
            assert!(convert(shape.clone()).is_none(), "{shape}");
        }
    }

    #[test]
    fn convex_needs_at_least_four_points() {
        let three = json!({ "type": "convex", "vertices": [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] });
        assert!(convert(three).is_none());
        let four = json!({ "type": "convex", "vertices": tetrahedron() });
        assert!(
            matches!(convert(four), Some(ColShape::Convex { vertices, .. }) if vertices.len() == 4)
        );
    }

    #[test]
    fn trimesh_rejects_empty_geometry_and_out_of_range_indices() {
        let triangle = json!([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        let cases = [
            json!({ "type": "trimesh", "vertices": [], "indices": [[0, 1, 2]] }),
            json!({ "type": "trimesh", "vertices": triangle, "indices": [] }),
            json!({ "type": "trimesh", "vertices": triangle, "indices": [[0, 1, 3]] }),
        ];
        for case in cases {
            assert!(convert(case.clone()).is_none(), "{case}");
        }
        let valid = json!({ "type": "trimesh", "vertices": triangle, "indices": [[0, 1, 2]] });
        assert!(
            matches!(convert(valid), Some(ColShape::Trimesh { indices, .. }) if indices == [[0, 1, 2]])
        );
    }

    #[test]
    fn parse_keeps_valid_shapes_and_skips_malformed_ones() {
        let sidecar = json!({
            "shapes": [
                { "type": "sphere", "center": [1.0, 2.0, 3.0], "radius": 0.5, "node": "Door" },
                { "type": "sphere" },
            ],
        });
        let parsed = parse_sidecar_bytes(&serde_json::to_vec(&sidecar).unwrap(), "test").unwrap();
        assert_eq!(parsed.shapes.len(), 1);
        assert_eq!(parsed.shapes[0].node.as_deref(), Some("Door"));
        assert_eq!(parsed.shapes[0].joint, None);
    }

    #[test]
    fn parse_reads_the_material_and_defaults_to_none() {
        let sidecar = json!({
            "shapes": [
                { "type": "sphere", "radius": 0.5, "material": 2_052_480_620_u32 },
                { "type": "sphere", "radius": 0.5 },
            ],
        });
        let parsed = parse_sidecar_bytes(&serde_json::to_vec(&sidecar).unwrap(), "test").unwrap();
        assert_eq!(parsed.shapes[0].material, 2_052_480_620);
        assert_eq!(parsed.shapes[1].material, 0);
    }

    #[test]
    fn parse_ignores_the_dynamic_and_mass_keys() {
        let sidecar = json!({ "shapes": [], "dynamic": true, "mass": 12.5 });
        let parsed = parse_sidecar_bytes(&serde_json::to_vec(&sidecar).unwrap(), "test").unwrap();
        assert!(parsed.shapes.is_empty());
    }

    #[test]
    fn parse_rejects_unparseable_payloads() {
        assert!(parse_sidecar_bytes(b"{ nope", "test").is_none());
        assert!(
            parse_sidecar_bytes(b"{}", "test").is_none(),
            "no shapes key"
        );
    }

    #[test]
    fn load_sidecar_file_reads_from_disk_and_returns_none_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.col.json");
        std::fs::write(&path, br#"{"shapes":[{"type":"sphere","radius":1.0}]}"#).unwrap();
        assert_eq!(
            load_sidecar_file(path.to_str().unwrap())
                .unwrap()
                .shapes
                .len(),
            1
        );
        assert!(load_sidecar_file(dir.path().join("b.col.json").to_str().unwrap()).is_none());
    }

    #[test]
    fn manifest_key_lowercases_and_uses_backslashes() {
        assert_eq!(
            manifest_key("meshes/Architecture/Walls/Wall01.glb"),
            "meshes\\architecture\\walls\\wall01.glb"
        );
        assert_eq!(manifest_key("meshes\\a.glb"), "meshes\\a.glb");
    }

    fn manifest_file(contents: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("collision_manifest.json");
        std::fs::write(&path, contents).unwrap();
        let path = path.to_str().unwrap().to_owned();
        (dir, path)
    }

    #[test]
    fn manifest_rows_decode_dynamic_mass_and_ragdoll_flags() {
        let (_dir, path) = manifest_file(
            r#"{"meshes\\wall.glb":[0,null],"meshes\\bottle.glb":[1,0.5],"meshes\\skeleton.glb":[1,10.0,1],"meshes\\chain.glb":[1,2.0,0]}"#,
        );
        let manifest = load_manifest(&path);
        assert_eq!(manifest.len(), 4);
        let wall = &manifest["meshes\\wall.glb"];
        assert!(!wall.dynamic && wall.mass.is_none() && !wall.ragdoll && wall.water.is_none());
        let bottle = &manifest["meshes\\bottle.glb"];
        assert!(bottle.dynamic && bottle.mass == Some(0.5) && !bottle.ragdoll);
        assert!(manifest["meshes\\skeleton.glb"].ragdoll);
        assert!(!manifest["meshes\\chain.glb"].ragdoll);
    }

    #[test]
    fn a_four_element_row_carries_the_water_object_of_a_dynamic_body() {
        let (_dir, path) = manifest_file(
            r#"{"meshes\\crate.glb":[1,3.5,0,{"layer":4,"part":0,"materials":[[2086,1],[7,2]]}],"meshes\\doll.glb":[1,2.0,1,{"layer":10,"part":0,"materials":[[7,1]]}]}"#,
        );
        let manifest = load_manifest(&path);
        let crate_meta = &manifest["meshes\\crate.glb"];
        let water = crate_meta.water.as_ref().unwrap();
        assert_eq!((water.layer, water.part), (4, 0));
        assert_eq!(water.materials, [(2086, 1), (7, 2)]);
        assert!(!crate_meta.ragdoll);
        let doll = &manifest["meshes\\doll.glb"];
        assert!(doll.ragdoll && doll.water.is_some());
    }

    #[test]
    fn a_water_object_that_is_not_a_body_description_fails_the_whole_manifest() {
        let (_dir, path) = manifest_file(r#"{"a":[1,2.0,0,{"layer":"x"}]}"#);
        assert!(load_manifest(&path).is_empty());
    }

    #[test]
    fn an_unparseable_or_missing_manifest_disables_all_collision() {
        let (dir, truncated) = manifest_file(r#"{"meshes\\wall.glb":[0,nu"#);
        assert!(load_manifest(&truncated).is_empty());
        let missing = dir.path().join("absent.json");
        assert!(load_manifest(missing.to_str().unwrap()).is_empty());
    }

    #[test]
    fn a_row_of_the_wrong_arity_fails_the_whole_manifest() {
        let (_dir, path) = manifest_file(r#"{"a":[1,2.0],"b":[1]}"#);
        assert!(load_manifest(&path).is_empty());
    }

    proptest::proptest! {
        #[test]
        fn arbitrary_sidecar_bytes_never_panic(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..300)) {
            let _ = parse_sidecar_bytes(&bytes, "fuzz");
        }

        #[test]
        fn arbitrary_trimesh_indices_never_panic_the_converter(
            indices in proptest::collection::vec(proptest::prelude::any::<[u32; 3]>(), 0..8),
            vertex_count in 0_usize..6,
        ) {
            let vertices: Vec<[f32; 3]> = (0..vertex_count).map(|i| [i as f32, 0.0, 0.0]).collect();
            let shape = json!({ "type": "trimesh", "vertices": vertices, "indices": indices });
            let converted = convert(shape);
            if let Some(ColShape::Trimesh { vertices, indices }) = converted {
                proptest::prop_assert!(indices.iter().flatten().all(|&i| (i as usize) < vertices.len()));
            }
        }
    }
}
