//! Pre-baking of rapier collision shapes off the main thread.
//!
//! Trimesh construction (vertex merge + internal-edge fix + BVH build) is the
//! expensive half of collider registration and used to run on the main thread
//! inside `PhysicsContext::add_cell` — a multi-ms stall per first-encounter
//! building. The streaming worker now calls [`ShapeBakeCache::bake_cell`] and
//! ships the result alongside the pure-data `CellColliderBatch`; `add_cell`
//! consumes the pre-built shapes and only falls back to building when a bake
//! is absent.
//!
//! Boundary note: baking is a pure function of the
//! batch — no sim state is read or written, so the streaming worker merely
//! executes sim-owned code on sim-owned data. Every field of
//! [`BakedCellColliders`] is private to `sim`; rapier types remain unnameable
//! outside this module tree even though the value travels through rendering.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;

use glam::Vec3;
use rapier3d::prelude::*;
use serde::{Deserialize, Serialize};

use super::cache_purge::{spawn_purge, versioned_dir_name};
use super::collision::{CellColliderBatch, ColShape, PartShape, TerrainColliderSpec};
use super::ragdoll_spec::{RagdollSpec, load_ragdoll_spec};
use super::sidecar_hash::{SidecarHasher, SidecarLookup};
use crate::assets::layout::{CACHE_DIR, PHYSICS_CACHE_DIR};
use crate::shared_cache::SharedCache;

/// Bump when the serialized collider format changes in a way the bytes alone
/// can't express — a rapier/parry upgrade that alters a shape's wire layout, or
/// a change to `CachedShape`. It is folded into every disk-cache key, so a bump
/// orphans the old blobs and forces a rebuild. (Trimesh flag changes are
/// already in the key via `TriMeshFlags::bits()`, so they invalidate on their
/// own without a version bump.) A `rustc` upgrade that changes `DefaultHasher`'s
/// algorithm also silently re-keys the cache — harmless (the key is re-checked
/// on read, so a stale hash can only cause a rebuild, never wrong geometry), so
/// no bump is needed for that. Respelling the mesh path inside the key (it starts
/// with the layout's `meshes\` root) re-keys every blob the same way, but is
/// bumped so that the purge removes the orphans.
const COLLIDER_CACHE_VERSION: u32 = 5;

pub(crate) const TRIMESH_FLAGS: TriMeshFlags = TriMeshFlags::FIX_INTERNAL_EDGES
    .union(TriMeshFlags::MERGE_DUPLICATE_VERTICES)
    .union(TriMeshFlags::DELETE_DEGENERATE_TRIANGLES);

/// Terrain additionally gets `ORIENTED`: a plain parry trimesh is a
/// zero-thickness, double-sided membrane with no inside/outside, so once a
/// body's centre crosses the surface there is no restoring contact and it
/// rests below the ground. `ORIENTED` computes outward pseudo-normals, giving
/// the surface a signed inside/outside so penetration resolves back out along
/// the normal. Safe for terrain because it is heightmap-derived with
/// consistent winding. Object collision meshes are *not* given `ORIENTED` until
/// their winding (FO4 compressed-mesh chunks) is verified — inconsistent
/// winding would resolve penetration the wrong way.
pub(crate) const TERRAIN_TRIMESH_FLAGS: TriMeshFlags = TRIMESH_FLAGS.union(TriMeshFlags::ORIENTED);

/// One baked shape plus its pose in GLB-local space. A GLB's primitive
/// shapes merge into a single compound so the broad-phase leaf count stays
/// near REFR granularity (per-shape colliders ballooned to 200k+ leaves and
/// made every broad-phase refit a frame killer); trimeshes are composite
/// shapes themselves and parry forbids nesting them in a compound, so each
/// rides as its own collider.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct CachedShape {
    pub(crate) shape: SharedShape,
    pub(crate) local_pose: Pose,
    /// Animated joint the shape follows (a kinematic body per joint); `None`
    /// = static.
    pub(crate) joint: Option<u16>,
    /// CRC-32 of the lowercased MATT name every sub-shape of this collider
    /// carries; 0 = no material.
    pub(crate) material: u32,
}

/// On-disk blob: the baked shapes plus the cache key they were built for. The
/// key is re-checked on read so a hash collision — or a stale file whose key no
/// longer matches — is detected and rebuilt instead of silently loaded as the
/// wrong geometry.
#[derive(Deserialize)]
struct CachedBlob {
    key: String,
    shapes: Vec<CachedShape>,
}

/// Borrowed serialize-side twin of `CachedBlob`: writing only reads the key and
/// shapes, so this avoids cloning the whole `Vec<CachedShape>` per store. The
/// owned `CachedBlob` is kept for the deserialize side.
#[derive(Serialize)]
struct CachedBlobRef<'a> {
    key: &'a str,
    shapes: &'a [CachedShape],
}

/// Per-process counter for unique temp-file names, so two engine instances
/// sharing one cache directory never collide on the same temp path.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Shape cache key: mesh path + REFR scale bits. Shapes are baked per scale
/// because rapier poses carry no scale.
pub(crate) type ShapeKey = (String, u32);

/// Opaque pre-baked collider payload for one cell. Carried by
/// `SimCommand::AddCellColliders` next to the batch it was baked from.
/// Clone is cheap (Arc/SharedShape clones) — the streamer keeps each
/// resident cell's payload and clones it whenever the cell re-enters the
/// collision ring.
#[derive(Clone)]
pub struct BakedCellColliders {
    pub(crate) terrain: Option<SharedShape>,
    pub(crate) objects: Vec<BakedObjectGroup>,
    pub(crate) ragdolls: Vec<BakedRagdollGroup>,
}

/// Baked shapes for one (mesh path, scale) pair. Placements stay in the
/// batch; this only accelerates the shape construction.
#[derive(Clone)]
pub(crate) struct BakedObjectGroup {
    pub(crate) path: String,
    pub(crate) scale_bits: u32,
    pub(crate) shapes: Arc<Vec<CachedShape>>,
}

/// A ragdoll's validated sidecar plus its per-body rapier shapes at one
/// placement scale: everything the sim needs to build the bodies, so the sim
/// thread parses and hulls nothing.
pub(crate) struct BakedRagdoll {
    pub(crate) spec: Arc<RagdollSpec>,
    /// Shapes of `spec.bodies[i]`, in the body's node-local frame.
    pub(crate) body_shapes: Vec<Vec<CachedShape>>,
}

/// A failed ragdoll bake carries its reason; the loader already logged it.
pub(crate) type RagdollBake = Result<Arc<BakedRagdoll>, Arc<str>>;

/// Baked ragdoll for one (mesh path, scale) pair.
#[derive(Clone)]
pub(crate) struct BakedRagdollGroup {
    pub(crate) path: String,
    pub(crate) scale_bits: u32,
    pub(crate) outcome: RagdollBake,
}

/// Bounded (path, scale) -> baked-shapes cache, the rapier twin of the
/// streamer's mesh/collider/DDS caches: without it every cell that places
/// the same building would re-bake its trimesh. Shared by the whole worker
/// pool (`SharedCache` dedups concurrent misses); the Arcs are shared with
/// `PhysicsContext`'s refcounted shape cache on insert, so a shape resident
/// in both costs its memory once.
pub struct ShapeBakeCache {
    cache: SharedCache<ShapeKey, Arc<Vec<CachedShape>>>,
    /// Ragdoll sidecar parse + per-body hull builds, once per (mesh, scale).
    /// A failed bake is cached too, so a broken sidecar is read and logged once.
    ragdoll_cache: SharedCache<ShapeKey, RagdollBake>,
    /// Persistent on-disk blob directory. The in-memory `cache` lives for one
    /// process only; this lets a baked shape — BVH and pseudo-normals included
    /// — survive across runs, so the expensive `trimesh_with_flags` build runs
    /// once ever per (mesh, scale, flags) instead of once per launch. `None`
    /// disables disk caching (e.g. the directory couldn't be created); the
    /// in-memory cache is unaffected.
    disk_dir: Option<PathBuf>,
    /// Content hashes of the `.col.json` sidecars that key the object blobs.
    sidecar_hashes: SidecarHasher,
}

const BAKE_CACHE_CAP: u64 = 512;
/// Distinct ragdoll (mesh, scale) pairs kept baked; skeleton props are few.
const RAGDOLL_BAKE_CACHE_CAP: u64 = 64;

/// The on-disk collider-cache directory of the current format version:
/// `cache/physics/v<N>` under the install root (the parent of `assets`). Owning
/// the path policy here, in `sim`, keeps the install-layout knowledge out of the
/// render layer. Blobs of other versions live in sibling `v<N>` directories; a
/// newer build purges the older ones in the background, so alternating between
/// builds of different cache versions re-bakes every time.
pub fn physics_cache_dir(asset_root: &str) -> PathBuf {
    let assets = Path::new(asset_root);
    assets
        .parent()
        .unwrap_or(assets)
        .join(CACHE_DIR)
        .join(PHYSICS_CACHE_DIR)
        .join(versioned_dir_name(COLLIDER_CACHE_VERSION))
}

/// The directories around a cache directory laid out as `physics_cache_dir`
/// builds it. Only that layout is recognized, so nothing derived from it (the
/// purge root, the index path base) can point at an unrelated directory.
struct CacheLayout<'a> {
    install_root: &'a Path,
    physics_root: &'a Path,
}

impl<'a> CacheLayout<'a> {
    fn of_current_version_dir(dir: &'a Path) -> Option<Self> {
        let named = |path: &Path, name: &str| path.file_name().is_some_and(|n| n == name);
        let physics_root = dir.parent()?;
        let cache_root = physics_root.parent()?;
        (named(dir, &versioned_dir_name(COLLIDER_CACHE_VERSION))
            && named(physics_root, PHYSICS_CACHE_DIR)
            && named(cache_root, CACHE_DIR))
        .then_some(Self {
            install_root: cache_root.parent()?,
            physics_root,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TerrainShapeError {
    #[error(
        "triangle {triangle} references vertex {index} but the mesh has {vertex_count} vertices"
    )]
    IndexOutOfRange {
        triangle: usize,
        index: u32,
        vertex_count: usize,
    },
    #[error("trimesh construction failed: {0}")]
    Trimesh(TriMeshBuilderError),
}

/// Build the terrain trimesh. Indices are validated first because parry indexes
/// the vertex array unchecked while building the BVH, which aborts the process
/// (`panic = "abort"`) on a malformed mesh instead of returning an error.
pub(crate) fn build_terrain_shape(
    terrain: &TerrainColliderSpec,
) -> Result<SharedShape, TerrainShapeError> {
    let vertex_count = terrain.vertices.len();
    let bad = terrain
        .indices
        .iter()
        .enumerate()
        .find_map(|(triangle, tri)| {
            let index = *tri.iter().find(|&&v| v as usize >= vertex_count)?;
            Some((triangle, index))
        });
    if let Some((triangle, index)) = bad {
        return Err(TerrainShapeError::IndexOutOfRange {
            triangle,
            index,
            vertex_count,
        });
    }
    SharedShape::trimesh_with_flags(
        terrain.vertices.clone(),
        terrain.indices.clone(),
        TERRAIN_TRIMESH_FLAGS,
    )
    .map_err(TerrainShapeError::Trimesh)
}

/// Starts the background purge of older cache versions. Any directory that is
/// not the current version of the standard layout (a test directory, a custom
/// location) has no sibling versions to clean up and is left strictly alone.
fn purge_older_versions(disk_dir: &Path) -> Option<JoinHandle<()>> {
    let layout = CacheLayout::of_current_version_dir(disk_dir)?;
    spawn_purge(layout.physics_root, COLLIDER_CACHE_VERSION, disk_dir)
}

impl ShapeBakeCache {
    pub fn new(disk_dir: PathBuf) -> Self {
        let disk_dir = match std::fs::create_dir_all(&disk_dir) {
            Ok(()) => {
                drop(purge_older_versions(&disk_dir));
                Some(disk_dir)
            }
            Err(e) => {
                tracing::warn!(
                    "collision cache: disabled — can't create {}: {e}",
                    disk_dir.display()
                );
                None
            }
        };
        Self {
            cache: SharedCache::new(BAKE_CACHE_CAP),
            ragdoll_cache: SharedCache::new(RAGDOLL_BAKE_CACHE_CAP),
            sidecar_hashes: SidecarHasher::new(
                disk_dir.as_deref(),
                disk_dir
                    .as_deref()
                    .and_then(CacheLayout::of_current_version_dir)
                    .map(|layout| layout.install_root),
            ),
            disk_dir,
        }
    }

    /// Persists the sidecar hash index if it grew. Call once the initial load
    /// is done; it also runs when the cache is dropped.
    pub fn flush_sidecar_index(&self) {
        self.sidecar_hashes.flush();
    }

    // Cache keys are pipe-joined strings (`terrain|…` / `obj|…`). They are pure
    // identity tokens — hashed for the filename here and re-checked verbatim on
    // read — never parsed back into fields, so a typed key buys little. The `|`
    // delimiter is unambiguous because the only free-form component is a mesh
    // path, and Windows reserves `|` so a path can never contain one.
    fn blob_path(&self, key: &str) -> Option<PathBuf> {
        let dir = self.disk_dir.as_ref()?;
        let mut h = DefaultHasher::new();
        key.hash(&mut h);
        Some(dir.join(format!("{:016x}.colbin", h.finish())))
    }

    /// Read a baked shape list from disk. Distinguishes the error kinds the
    /// project rules require: a missing file is a normal cold miss (silent); a
    /// key mismatch is an expected hash-collision/stale case (silent); but a
    /// read error or a decode failure means a genuinely corrupt blob, which is
    /// logged before falling back to rebuilding.
    fn disk_load(&self, key: &str) -> Option<Vec<CachedShape>> {
        let path = self.blob_path(key)?;
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                tracing::warn!("collision cache: read failed for {}: {e}", path.display());
                return None;
            }
        };
        match bincode::deserialize::<CachedBlob>(&bytes) {
            Ok(blob) if blob.key == key => Some(blob.shapes),
            Ok(_) => None,
            Err(e) => {
                // Decode failure = genuinely bad bytes. Delete it so the
                // rebuild that follows can write a fresh blob — otherwise
                // `disk_store`'s exists-guard would leave the corrupt file in
                // place and we'd re-read, re-warn, and re-rebuild it every run.
                tracing::warn!(
                    "collision cache: corrupt blob {} ({e}); purging and rebuilding",
                    path.display()
                );
                let _ = std::fs::remove_file(&path);
                None
            }
        }
    }

    /// Write a baked shape list to disk (best-effort). Skips empty results (a
    /// degenerate-mesh failure shouldn't be cached) and writes via a
    /// process-unique temp + rename so a concurrent reader never sees a torn
    /// file. A blob already at the destination was written for the same key by
    /// this build (same shapes → same bytes), so if another writer won the race
    /// the temp is dropped rather than replacing it; a genuine rename failure
    /// (permissions, disk full) is logged rather than silently swallowed.
    fn disk_store(&self, key: &str, shapes: &[CachedShape]) {
        if shapes.is_empty() {
            return;
        }
        let Some(path) = self.blob_path(key) else {
            return;
        };
        let bytes = match bincode::serialize(&CachedBlobRef { key, shapes }) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("collision cache: serialize failed for {key}: {e}");
                return;
            }
        };
        let tmp = path.with_extension(format!(
            "{}.{}.tmp",
            std::process::id(),
            TMP_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        if let Err(e) = std::fs::write(&tmp, &bytes) {
            tracing::warn!(
                "collision cache: temp write failed for {}: {e}",
                tmp.display()
            );
            return;
        }
        if path.exists() {
            let _ = std::fs::remove_file(&tmp); // another writer won — benign
        } else if let Err(e) = std::fs::rename(&tmp, &path) {
            tracing::warn!("collision cache: rename failed for {}: {e}", path.display());
            let _ = std::fs::remove_file(&tmp);
        }
    }

    /// Disk-cached build: load the blob if present, otherwise build, persist,
    /// and return. The `trimesh_with_flags` BVH/pseudo-normal construction only
    /// runs on a true cold miss.
    fn cached_or_build(
        &self,
        key: &str,
        build: impl FnOnce() -> Vec<CachedShape>,
    ) -> Vec<CachedShape> {
        if let Some(hit) = self.disk_load(key) {
            return hit;
        }
        let built = build();
        self.disk_store(key, &built);
        built
    }

    /// Bake every shape `add_cell` will need for this batch, building
    /// uncached (path, scale) pairs in parallel. `load_shapes` fetches a mesh's
    /// raw collision shapes by path; `joint_fingerprint` names the animated
    /// joints those shapes bind to (it is part of the disk key, since the joint
    /// of a baked shape comes from the rig, not the sidecar); `sidecar_path`
    /// names the mesh's `.col.json`, whose content hash (memoized, and indexed
    /// on disk by file stamp) is also part of the disk key so a sidecar with
    /// new content re-keys its blobs. It runs once per in-memory miss, not per
    /// placement. `load_shapes` is called ONLY for a (path, scale) pair
    /// that misses both the in-memory and the on-disk (`.colbin`) cache — i.e.
    /// only on a genuinely cold build. A warm launch hits `.colbin` and never
    /// calls it, so the shape BLOB is never read. It runs on the cache's loader
    /// pool, so it must be `Sync`.
    pub fn bake_cell(
        &self,
        batch: &CellColliderBatch,
        load_shapes: impl Fn(&str) -> Arc<Vec<PartShape>> + Sync,
        joint_fingerprint: impl Fn(&str) -> Option<u64> + Sync,
        sidecar_path: impl Fn(&str) -> Option<String> + Sync,
    ) -> BakedCellColliders {
        // Terrain has no (path, scale) identity, so it's keyed by a content
        // hash of its geometry — content-addressing the disk blob. Unique per
        // cell in memory, but shared on disk across runs (and across any two
        // cells that happen to have identical terrain).
        let terrain = batch.terrain.as_ref().and_then(|t| {
            let key = format!(
                "terrain|{:016x}|{}|v{}",
                content_hash(&t.vertices, &t.indices),
                TERRAIN_TRIMESH_FLAGS.bits(),
                COLLIDER_CACHE_VERSION,
            );
            let mut shapes = self.cached_or_build(&key, || match build_terrain_shape(t) {
                Ok(shape) => vec![CachedShape {
                    shape,
                    local_pose: Pose::IDENTITY,
                    joint: None,
                    material: 0,
                }],
                Err(e) => {
                    // add_cell's fallback will retry (and re-warn) once on
                    // the main thread; accepted for a degenerate-mesh rarity.
                    tracing::warn!("collision: terrain trimesh bake failed: {e}");
                    Vec::new()
                }
            });
            shapes.pop().map(|c| c.shape)
        });

        // Unique (path, scale) pairs this cell needs. The actual shapes are
        // pulled lazily via `load_shapes` inside the build closure, so a pair
        // already on disk as a `.colbin` never triggers a shape read.
        let mut sources: HashMap<ShapeKey, f32> = HashMap::new();
        for group in &batch.objects {
            for placement in &group.placements {
                sources
                    .entry((group.path.clone(), placement.scale.to_bits()))
                    .or_insert(placement.scale);
            }
        }
        let keys: Vec<ShapeKey> = sources.keys().cloned().collect();
        let resolved = self.cache.resolve(
            &keys,
            |key| {
                let scale = sources[key];
                let sidecar = sidecar_path(&key.0).map_or(SidecarLookup::Absent, |path| {
                    self.sidecar_hashes.lookup(&path)
                });
                let build = || build_shapes(&load_shapes(&key.0), scale, &key.0);
                Arc::new(
                    match object_disk_key(&key.0, key.1, joint_fingerprint(&key.0), sidecar) {
                        Some(disk_key) => self.cached_or_build(&disk_key, build),
                        None => build(),
                    },
                )
            },
            |_| 1,
        );

        let objects = resolved
            .into_iter()
            .map(|((path, scale_bits), shapes)| BakedObjectGroup {
                path,
                scale_bits,
                shapes,
            })
            .collect();

        BakedCellColliders {
            terrain,
            objects,
            ragdolls: self.bake_ragdolls(batch),
        }
    }

    /// Parses each ragdoll sidecar of the batch and builds its body shapes, so
    /// the sim thread only instantiates bodies. Runs on the streaming worker.
    fn bake_ragdolls(&self, batch: &CellColliderBatch) -> Vec<BakedRagdollGroup> {
        let mut sources: HashMap<ShapeKey, (&str, f32)> = HashMap::new();
        for group in &batch.objects {
            let Some(sidecar) = group.ragdoll_sidecar.as_deref() else {
                continue;
            };
            for placement in &group.placements {
                sources
                    .entry((group.path.clone(), placement.scale.to_bits()))
                    .or_insert((sidecar, placement.scale));
            }
        }
        let keys: Vec<ShapeKey> = sources.keys().cloned().collect();
        self.ragdoll_cache
            .resolve(
                &keys,
                |key| bake_ragdoll(sources[key].0, sources[key].1),
                |_| 1,
            )
            .into_iter()
            .map(|((path, scale_bits), outcome)| BakedRagdollGroup {
                path,
                scale_bits,
                outcome,
            })
            .collect()
    }
}

fn bake_ragdoll(sidecar_path: &str, scale: f32) -> RagdollBake {
    let spec = load_ragdoll_spec(sidecar_path).map_err(|error| {
        let reason: Arc<str> = format!("{sidecar_path}: {error}").into();
        tracing::error!("ragdoll: error {reason}");
        reason
    })?;
    let body_shapes = spec
        .bodies
        .iter()
        .map(|body| merge_shapes_of_material(body.shapes.iter(), 0, scale, sidecar_path))
        .collect();
    Ok(Arc::new(BakedRagdoll {
        spec: Arc::new(spec),
        body_shapes,
    }))
}

/// Disk-cache key of one (mesh, scale) object bake, or `None` when the bake
/// must not be disk-cached: its sidecar exists but could not be read, so no key
/// can describe its content, and a blob stored under a sidecar-less key would be
/// served for the real content later. The sidecar component is the content hash
/// of the mesh's `.col.json` (absent when it has none); the joint component
/// covers the rig, which the sidecar does not describe.
fn object_disk_key(
    mesh_path: &str,
    scale_bits: u32,
    joint_fingerprint: Option<u64>,
    sidecar: SidecarLookup,
) -> Option<String> {
    let sidecar = match sidecar {
        SidecarLookup::Absent => String::new(),
        SidecarLookup::Hashed(hash) => format!("|s{}", hash.key_component()),
        SidecarLookup::Unreadable => return None,
    };
    let joints = joint_fingerprint
        .map(|fingerprint| format!("|j{fingerprint:016x}"))
        .unwrap_or_default();
    Some(format!(
        "obj|{mesh_path}|{scale_bits}|{}{joints}{sidecar}|v{COLLIDER_CACHE_VERSION}",
        TRIMESH_FLAGS.bits(),
    ))
}

/// Stable hash of a trimesh's geometry, for content-addressing its disk blob.
/// Hashes the raw f32 bit patterns so it's exact and reproducible across runs.
fn content_hash(vertices: &[Vec3], indices: &[[u32; 3]]) -> u64 {
    let mut h = DefaultHasher::new();
    vertices.len().hash(&mut h);
    for v in vertices {
        h.write_u32(v.x.to_bits());
        h.write_u32(v.y.to_bits());
        h.write_u32(v.z.to_bits());
    }
    indices.len().hash(&mut h);
    for t in indices {
        h.write_u32(t[0]);
        h.write_u32(t[1]);
        h.write_u32(t[2]);
    }
    h.finish()
}

/// Bake one GLB's shape list at a given uniform scale. Shapes following the
/// same animated joint (or none) and carrying the same material merge together,
/// so each kinematic body stays near REFR granularity and every collider has
/// exactly one material.
pub(crate) fn build_shapes(shapes: &[PartShape], scale: f32, path: &str) -> Vec<CachedShape> {
    let mut groups: Vec<(Option<u16>, u32)> = Vec::new();
    for part in shapes {
        let group = (part.joint, part.material);
        if !groups.contains(&group) {
            groups.push(group);
        }
    }
    let baked: Vec<CachedShape> = groups
        .into_iter()
        .flat_map(|(joint, material)| {
            let group = shapes
                .iter()
                .filter(move |part| part.joint == joint && part.material == material)
                .map(|part| &part.shape);
            merge_shapes_of_material(group, material, scale, path)
                .into_iter()
                .map(move |cached| CachedShape { joint, ..cached })
        })
        .collect();
    tracing::debug!(
        "collision: {path} bakes {} shapes into {} colliders",
        shapes.len(),
        baked.len()
    );
    baked
}

/// Whether the shape's local bounds are finite and non-inverted. The broad phase bins leaf
/// centres, and an empty trimesh (every triangle deleted as degenerate) has inverted infinite bounds
/// whose NaN centre makes parry's binned BVH build index past its last bin.
fn has_finite_extent(shape: &SharedShape) -> bool {
    let aabb = shape.compute_local_aabb();
    aabb.mins.is_finite() && aabb.maxs.is_finite() && aabb.mins.cmple(aabb.maxs).all()
}

/// Merge shapes sharing one local space and one `material` into as few rapier
/// shapes as possible: all primitives merge into one compound; trimeshes
/// (composite shapes — parry forbids nesting them) stay standalone. Failed
/// sub-shapes are skipped with a warning — a partial collider beats a crash.
/// Material 0 means the shapes carry none.
pub(crate) fn merge_shapes_of_material<'a>(
    shapes: impl Iterator<Item = &'a ColShape>,
    material: u32,
    scale: f32,
    path: &str,
) -> Vec<CachedShape> {
    let mut out: Vec<CachedShape> = Vec::new();
    let mut primitives: Vec<(Pose, SharedShape)> = Vec::new();
    // All of a GLB's trimeshes share its local space (transforms are
    // pre-baked by the sidecar writer), so they concatenate into one mesh —
    // FO4 compressed-mesh collision often splits a building into dozens of
    // chunks, and one collider each would blow the leaf count right back up.
    let mut tri_vertices: Vec<Vec3> = Vec::new();
    let mut tri_indices: Vec<[u32; 3]> = Vec::new();
    for shape in shapes {
        match shape {
            ColShape::Box {
                center,
                rotation,
                half_extents,
            } => {
                let he = *half_extents * scale;
                primitives.push((
                    Pose::from_parts(*center * scale, *rotation),
                    SharedShape::cuboid(he.x, he.y, he.z),
                ));
            }
            ColShape::Sphere { center, radius } => primitives.push((
                Pose::from_translation(*center * scale),
                SharedShape::ball(radius * scale),
            )),
            ColShape::Capsule {
                center,
                rotation,
                radius,
                height,
            } => {
                // Sidecar height is the full capsule (Godot convention);
                // rapier wants the cylinder half-length.
                let r = radius * scale;
                let half_cyl = ((height * scale) * 0.5 - r).max(0.001);
                primitives.push((
                    Pose::from_parts(*center * scale, *rotation),
                    SharedShape::capsule_y(half_cyl, r),
                ));
            }
            ColShape::Convex {
                center,
                rotation,
                vertices,
            } => {
                let points: Vec<Vec3> = vertices.iter().map(|v| *v * scale).collect();
                if let Some(shape) = SharedShape::convex_hull(&points) {
                    primitives.push((Pose::from_parts(*center * scale, *rotation), shape));
                } else {
                    tracing::warn!("collision: degenerate convex hull in {path}");
                }
            }
            ColShape::Trimesh { vertices, indices } => {
                let base = tri_vertices.len() as u32;
                tri_vertices.extend(vertices.iter().map(|v| *v * scale));
                tri_indices.extend(
                    indices
                        .iter()
                        .map(|t| [t[0] + base, t[1] + base, t[2] + base]),
                );
            }
        }
    }
    if tri_vertices.iter().any(|v| !v.is_finite()) {
        tracing::warn!("collision: trimesh in {path} has non-finite vertices; skipped");
        tri_vertices.clear();
        tri_indices.clear();
    }
    if !tri_vertices.is_empty() {
        match SharedShape::trimesh_with_flags(tri_vertices, tri_indices, TRIMESH_FLAGS) {
            Ok(shape) if !has_finite_extent(&shape) => tracing::warn!(
                "collision: trimesh in {path} has no usable triangles after degenerate removal; skipped"
            ),
            Ok(shape) => out.push(CachedShape {
                shape,
                local_pose: Pose::IDENTITY,
                joint: None,
                material,
            }),
            Err(e) => tracing::warn!("collision: trimesh failed in {path}: {e}"),
        }
    }
    match primitives.len() {
        0 => {}
        1 => {
            let (local_pose, shape) = primitives.pop().unwrap();
            out.push(CachedShape {
                shape,
                local_pose,
                joint: None,
                material,
            });
        }
        _ => out.push(CachedShape {
            shape: SharedShape::compound(primitives),
            local_pose: Pose::IDENTITY,
            joint: None,
            material,
        }),
    }
    out
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixture values are exactly representable in f32"
)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use glam::Quat;
    use serde_json::json;

    use super::*;
    use crate::sim::collision::{CellColliderBatch, ObjectColliderGroup, Placement};
    use crate::sim::physics::test_support::{
        SIDECAR, box_shape, ground_batch, object_group, placement, set_mtime, sidecar_file,
    };
    use crate::sim::sidecar_hash::SidecarHash;

    const MESH: &str = "meshes\\crate.glb";

    fn colbin_files(dir: &Path) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "colbin"))
            .collect();
        files.sort();
        files
    }

    fn crate_batch(scale: f32) -> CellColliderBatch {
        let mut at_scale: Placement = placement("0A0001:Fallout4.esm", Vec3::ZERO);
        at_scale.scale = scale;
        CellColliderBatch {
            terrain: None,
            objects: vec![object_group(MESH, vec![at_scale], false, None)],
        }
    }

    /// Bakes the crate mesh with a loader that counts its calls and answers with a box of `half_extent`.
    fn bake_crate(
        cache: &ShapeBakeCache,
        batch: &CellColliderBatch,
        half_extent: f32,
        sidecar: Option<&Path>,
        loads: &AtomicUsize,
    ) -> BakedCellColliders {
        cache.bake_cell(
            batch,
            |_| {
                loads.fetch_add(1, Ordering::SeqCst);
                Arc::new(vec![box_shape(half_extent)])
            },
            |_| None,
            |_| sidecar.map(|path| path.to_str().unwrap().to_owned()),
        )
    }

    fn baked_half_extent(baked: &BakedCellColliders) -> f32 {
        baked.objects[0].shapes[0]
            .shape
            .compute_local_aabb()
            .half_extents()
            .x
    }

    fn loads(counter: &AtomicUsize) -> usize {
        counter.load(Ordering::SeqCst)
    }

    #[test]
    fn a_cold_bake_reads_the_shapes_and_writes_one_blob() {
        let dir = tempfile::tempdir().unwrap();
        let counter = AtomicUsize::new(0);
        let (_sidecar_dir, sidecar) = sidecar_file(SIDECAR);
        let cache = ShapeBakeCache::new(dir.path().to_owned());
        let baked = bake_crate(&cache, &crate_batch(1.0), 0.5, Some(&sidecar), &counter);
        assert_eq!(loads(&counter), 1);
        assert_eq!(colbin_files(dir.path()).len(), 1);
        assert!((baked_half_extent(&baked) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn a_relaunch_loads_from_the_disk_blob_without_reading_the_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let counter = AtomicUsize::new(0);
        let (_sidecar_dir, sidecar) = sidecar_file(SIDECAR);
        let first = ShapeBakeCache::new(dir.path().to_owned());
        bake_crate(&first, &crate_batch(1.0), 0.5, Some(&sidecar), &counter);
        let relaunched = ShapeBakeCache::new(dir.path().to_owned());
        let baked = bake_crate(
            &relaunched,
            &crate_batch(1.0),
            0.5,
            Some(&sidecar),
            &counter,
        );
        assert_eq!(
            loads(&counter),
            1,
            "the warm launch must not call the shape loader"
        );
        assert!((baked_half_extent(&baked) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn the_in_memory_cache_serves_repeat_bakes_of_the_same_mesh_and_scale() {
        let dir = tempfile::tempdir().unwrap();
        let counter = AtomicUsize::new(0);
        let (_sidecar_dir, sidecar) = sidecar_file(SIDECAR);
        let cache = ShapeBakeCache::new(dir.path().to_owned());
        bake_crate(&cache, &crate_batch(1.0), 0.5, Some(&sidecar), &counter);
        bake_crate(&cache, &crate_batch(1.0), 0.5, Some(&sidecar), &counter);
        assert_eq!(loads(&counter), 1);
    }

    /// Regression: the cache was once keyed by mesh path alone, so a regenerated
    /// `.col.json` kept serving the shapes of the old file.
    #[test]
    fn a_regenerated_sidecar_invalidates_the_blob_instead_of_serving_stale_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let counter = AtomicUsize::new(0);
        let (_sidecar_dir, sidecar) = sidecar_file(SIDECAR);
        let before = ShapeBakeCache::new(dir.path().to_owned());
        let old = bake_crate(&before, &crate_batch(1.0), 0.5, Some(&sidecar), &counter);
        std::fs::write(&sidecar, br#"{"shapes":[{"type":"sphere","radius":2.0}]}"#).unwrap();
        let relaunched = ShapeBakeCache::new(dir.path().to_owned());
        let new = bake_crate(
            &relaunched,
            &crate_batch(1.0),
            2.0,
            Some(&sidecar),
            &counter,
        );
        assert_eq!(
            loads(&counter),
            2,
            "changed sidecar content is a cache miss"
        );
        assert!((baked_half_extent(&old) - 0.5).abs() < 1e-5);
        assert!(
            (baked_half_extent(&new) - 2.0).abs() < 1e-5,
            "stale geometry was served"
        );
        assert_eq!(
            colbin_files(dir.path()).len(),
            2,
            "each content keeps its own blob"
        );
    }

    #[test]
    fn a_sidecar_that_appears_later_does_not_reuse_the_blob_baked_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let counter = AtomicUsize::new(0);
        let (_sidecar_dir, sidecar) = sidecar_file(SIDECAR);
        let before = ShapeBakeCache::new(dir.path().to_owned());
        bake_crate(&before, &crate_batch(1.0), 0.5, None, &counter);
        let relaunched = ShapeBakeCache::new(dir.path().to_owned());
        bake_crate(
            &relaunched,
            &crate_batch(1.0),
            0.5,
            Some(&sidecar),
            &counter,
        );
        assert_eq!(loads(&counter), 2);
    }

    /// Regression: the installer rewrites byte-identical sidecars, which used to
    /// re-key (and so re-bake) every collider.
    #[test]
    fn a_rewritten_identical_sidecar_still_hits_the_blob() {
        let dir = tempfile::tempdir().unwrap();
        let counter = AtomicUsize::new(0);
        let (_sidecar_dir, sidecar) = sidecar_file(SIDECAR);
        set_mtime(&sidecar, 1_000);
        let first = ShapeBakeCache::new(dir.path().to_owned());
        bake_crate(&first, &crate_batch(1.0), 0.5, Some(&sidecar), &counter);
        drop(first);
        std::fs::write(&sidecar, SIDECAR).unwrap();
        set_mtime(&sidecar, 2_000);
        let relaunched = ShapeBakeCache::new(dir.path().to_owned());
        bake_crate(
            &relaunched,
            &crate_batch(1.0),
            0.5,
            Some(&sidecar),
            &counter,
        );
        assert_eq!(loads(&counter), 1, "same bytes, new mtime must stay warm");
    }

    #[test]
    fn the_object_key_follows_sidecar_content_and_ignores_nothing_else() {
        let hashed = |bytes: &[u8]| SidecarLookup::Hashed(SidecarHash::of_bytes(bytes));
        let key = |sidecar: SidecarLookup, joints: Option<u64>, scale: u32| {
            object_disk_key("a.glb", scale, joints, sidecar).unwrap()
        };
        let base = key(hashed(b"a"), None, 1);
        assert_eq!(base, key(hashed(b"a"), None, 1));
        assert_ne!(base, key(hashed(b"b"), None, 1));
        assert_ne!(base, key(SidecarLookup::Absent, None, 1));
        assert_ne!(base, key(hashed(b"a"), Some(1), 1));
        assert_ne!(base, key(hashed(b"a"), None, 2));
        assert!(base.ends_with(&format!("|v{COLLIDER_CACHE_VERSION}")));
    }

    #[test]
    fn an_unreadable_sidecar_gets_no_disk_key() {
        assert_eq!(
            object_disk_key("a.glb", 1, None, SidecarLookup::Unreadable),
            None
        );
    }

    #[test]
    fn a_bake_with_an_unreadable_sidecar_is_not_stored_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let unreadable = tempfile::tempdir().unwrap();
        let counter = AtomicUsize::new(0);
        let cache = ShapeBakeCache::new(dir.path().to_owned());
        let baked = bake_crate(
            &cache,
            &crate_batch(1.0),
            0.5,
            Some(unreadable.path()),
            &counter,
        );
        assert!((baked_half_extent(&baked) - 0.5).abs() < 1e-5);
        assert!(colbin_files(dir.path()).is_empty());
    }

    #[test]
    fn only_the_current_version_dir_of_the_standard_layout_is_purged() {
        let version = versioned_dir_name(COLLIDER_CACHE_VERSION);
        let install = tempfile::tempdir().unwrap();
        let physics = install.path().join(CACHE_DIR).join(PHYSICS_CACHE_DIR);
        let older = physics.join("v0");
        std::fs::create_dir_all(&older).unwrap();
        std::fs::write(older.join("a.colbin"), b"x").unwrap();
        std::fs::write(physics.join("legacy.colbin"), b"x").unwrap();
        let current = physics.join(&version);
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("b.colbin"), b"x").unwrap();

        purge_older_versions(&current).unwrap().join().unwrap();
        assert!(!older.exists());
        assert!(!physics.join("legacy.colbin").exists());
        assert!(current.join("b.colbin").exists());

        for other in [
            install
                .path()
                .join(CACHE_DIR)
                .join("elsewhere")
                .join(&version),
            install.path().join(PHYSICS_CACHE_DIR).join(&version),
            install
                .path()
                .join(CACHE_DIR)
                .join(PHYSICS_CACHE_DIR)
                .join("v999"),
        ] {
            let sibling = other.parent().unwrap().join("v0");
            std::fs::create_dir_all(&sibling).unwrap();
            std::fs::create_dir_all(&other).unwrap();
            assert!(purge_older_versions(&other).is_none());
            assert!(sibling.exists(), "{} must be untouched", sibling.display());
        }
    }

    #[test]
    fn different_joint_fingerprints_do_not_share_a_blob() {
        let dir = tempfile::tempdir().unwrap();
        let counter = AtomicUsize::new(0);
        let bake_with_joints = |fingerprint: Option<u64>| {
            ShapeBakeCache::new(dir.path().to_owned()).bake_cell(
                &crate_batch(1.0),
                |_| {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Arc::new(vec![box_shape(0.5)])
                },
                |_| fingerprint,
                |_| None,
            )
        };
        bake_with_joints(Some(1));
        bake_with_joints(Some(2));
        bake_with_joints(Some(1));
        assert_eq!(loads(&counter), 2, "the third bake hits the first blob");
    }

    #[test]
    fn each_scale_is_baked_separately_and_scales_the_geometry() {
        let dir = tempfile::tempdir().unwrap();
        let counter = AtomicUsize::new(0);
        let cache = ShapeBakeCache::new(dir.path().to_owned());
        let unit = bake_crate(&cache, &crate_batch(1.0), 0.5, None, &counter);
        let doubled = bake_crate(&cache, &crate_batch(2.0), 0.5, None, &counter);
        assert_eq!(loads(&counter), 2);
        assert!((baked_half_extent(&unit) - 0.5).abs() < 1e-5);
        assert!((baked_half_extent(&doubled) - 1.0).abs() < 1e-5);
        assert_eq!(colbin_files(dir.path()).len(), 2);
    }

    #[test]
    fn a_corrupt_blob_is_purged_and_rebuilt_once() {
        let dir = tempfile::tempdir().unwrap();
        let counter = AtomicUsize::new(0);
        bake_crate(
            &ShapeBakeCache::new(dir.path().to_owned()),
            &crate_batch(1.0),
            0.5,
            None,
            &counter,
        );
        let blob = colbin_files(dir.path()).remove(0);
        std::fs::write(&blob, b"definitely not bincode").unwrap();
        let rebuilt = bake_crate(
            &ShapeBakeCache::new(dir.path().to_owned()),
            &crate_batch(1.0),
            0.5,
            None,
            &counter,
        );
        assert_eq!(loads(&counter), 2, "the bad blob forced a rebuild");
        assert!((baked_half_extent(&rebuilt) - 0.5).abs() < 1e-5);
        bake_crate(
            &ShapeBakeCache::new(dir.path().to_owned()),
            &crate_batch(1.0),
            0.5,
            None,
            &counter,
        );
        assert_eq!(loads(&counter), 2, "the rewritten blob is good again");
    }

    #[test]
    fn a_blob_stored_under_another_key_is_ignored_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ShapeBakeCache::new(dir.path().to_owned());
        let shapes = merge_shapes_of_material(
            [ColShape::Sphere {
                center: Vec3::ZERO,
                radius: 1.0,
            }]
            .iter(),
            0,
            1.0,
            "t",
        );
        cache.disk_store("obj|a", &shapes);
        // Simulate a hash collision: key B's file holds key A's blob.
        std::fs::copy(
            cache.blob_path("obj|a").unwrap(),
            cache.blob_path("obj|b").unwrap(),
        )
        .unwrap();
        assert!(cache.disk_load("obj|b").is_none());
        assert!(cache.disk_load("obj|a").is_some());
        assert!(
            cache.blob_path("obj|b").unwrap().exists(),
            "a key mismatch is not corruption"
        );
    }

    #[test]
    fn empty_results_are_not_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ShapeBakeCache::new(dir.path().to_owned());
        cache.disk_store("obj|empty", &[]);
        assert_eq!(colbin_files(dir.path()), Vec::<std::path::PathBuf>::new());
        assert!(cache.disk_load("obj|empty").is_none());
    }

    #[test]
    fn stores_leave_no_temp_files_behind() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ShapeBakeCache::new(dir.path().to_owned());
        let shapes = merge_shapes_of_material(
            [ColShape::Sphere {
                center: Vec3::ZERO,
                radius: 1.0,
            }]
            .iter(),
            0,
            1.0,
            "t",
        );
        cache.disk_store("obj|a", &shapes);
        cache.disk_store("obj|a", &shapes);
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().collect();
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn creating_a_cache_in_a_plain_directory_deletes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("legacy.colbin"), b"keep").unwrap();
        let _cache = ShapeBakeCache::new(dir.path().to_owned());
        assert!(dir.path().join("legacy.colbin").exists());
    }

    #[test]
    fn an_uncreatable_cache_directory_disables_only_the_disk_tier() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("a-file");
        std::fs::write(&blocker, b"x").unwrap();
        let cache = ShapeBakeCache::new(blocker.join(PHYSICS_CACHE_DIR));
        let counter = AtomicUsize::new(0);
        let baked = bake_crate(&cache, &crate_batch(1.0), 0.5, None, &counter);
        assert!((baked_half_extent(&baked) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn the_cache_directory_sits_beside_the_assets_folder() {
        assert_eq!(
            physics_cache_dir("C:/Games/OpenFO4/assets"),
            Path::new("C:/Games/OpenFO4")
                .join("cache")
                .join("physics")
                .join(format!("v{COLLIDER_CACHE_VERSION}"))
        );
    }

    #[test]
    fn terrain_blobs_are_content_addressed() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ShapeBakeCache::new(dir.path().to_owned());
        let load = |_: &str| Arc::new(Vec::new());
        let first = cache.bake_cell(&ground_batch(), load, |_| None, |_| None);
        cache.bake_cell(&ground_batch(), load, |_| None, |_| None);
        assert!(first.terrain.is_some());
        assert_eq!(
            colbin_files(dir.path()).len(),
            1,
            "identical terrain shares one blob"
        );
        let mut shifted = ground_batch();
        shifted.terrain.as_mut().unwrap().vertices[0].y = 1.0;
        cache.bake_cell(&shifted, load, |_| None, |_| None);
        assert_eq!(colbin_files(dir.path()).len(), 2);
    }

    #[test]
    fn a_terrain_without_triangles_bakes_to_nothing_and_is_not_cached() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ShapeBakeCache::new(dir.path().to_owned());
        let mut broken = ground_batch();
        broken.terrain.as_mut().unwrap().indices = vec![];
        let baked = cache.bake_cell(&broken, |_| Arc::new(Vec::new()), |_| None, |_| None);
        assert!(baked.terrain.is_none());
        assert_eq!(colbin_files(dir.path()), Vec::<std::path::PathBuf>::new());
    }

    #[test]
    fn a_terrain_triangle_indexing_past_the_vertices_is_rejected_with_its_position() {
        let mut broken = ground_batch().terrain.unwrap();
        broken.indices[1] = [0, 2, 4];
        let error = build_terrain_shape(&broken).unwrap_err();
        assert!(matches!(
            error,
            TerrainShapeError::IndexOutOfRange {
                triangle: 1,
                index: 4,
                vertex_count: 4
            }
        ));
    }

    #[test]
    fn a_terrain_with_an_out_of_range_index_bakes_to_nothing_instead_of_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ShapeBakeCache::new(dir.path().to_owned());
        let mut broken = ground_batch();
        broken.terrain.as_mut().unwrap().indices[0] = [0, 1, 99];
        let baked = cache.bake_cell(&broken, |_| Arc::new(Vec::new()), |_| None, |_| None);
        assert!(baked.terrain.is_none());
        assert_eq!(colbin_files(dir.path()), Vec::<std::path::PathBuf>::new());
    }

    #[test]
    fn primitives_merge_into_one_compound() {
        let shapes = [
            ColShape::Sphere {
                center: Vec3::ZERO,
                radius: 1.0,
            },
            ColShape::Box {
                center: Vec3::X,
                rotation: Quat::IDENTITY,
                half_extents: Vec3::ONE,
            },
        ];
        let merged = merge_shapes_of_material(shapes.iter(), 0, 1.0, "t");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].shape.as_compound().unwrap().shapes().len(), 2);
    }

    #[test]
    fn a_single_primitive_keeps_its_own_pose_instead_of_becoming_a_compound() {
        let shape = ColShape::Sphere {
            center: Vec3::new(1.0, 2.0, 3.0),
            radius: 0.5,
        };
        let merged = merge_shapes_of_material([shape].iter(), 0, 2.0, "t");
        assert_eq!(merged.len(), 1);
        assert!(merged[0].shape.as_ball().is_some());
        assert_eq!(
            merged[0].local_pose.translation,
            Vec3::new(2.0, 4.0, 6.0),
            "centre scaled"
        );
        assert_eq!(
            merged[0].shape.as_ball().unwrap().radius,
            1.0,
            "radius scaled"
        );
    }

    #[test]
    fn trimeshes_of_one_mesh_concatenate_with_offset_indices() {
        let triangle = |z: f32| ColShape::Trimesh {
            vertices: vec![
                Vec3::new(0.0, 0.0, z),
                Vec3::new(1.0, 0.0, z),
                Vec3::new(0.0, 1.0, z),
            ],
            indices: vec![[0, 1, 2]],
        };
        let merged = merge_shapes_of_material([triangle(0.0), triangle(5.0)].iter(), 0, 1.0, "t");
        assert_eq!(merged.len(), 1);
        let mesh = merged[0].shape.as_trimesh().unwrap();
        assert_eq!(mesh.indices().len(), 2);
        assert_eq!(mesh.vertices().len(), 6);
        assert_eq!(mesh.indices()[1], [3, 4, 5]);
    }

    fn husk_trimesh() -> ColShape {
        ColShape::Trimesh {
            vertices: vec![Vec3::new(150.0, 7.0, -6.0)],
            indices: vec![[0, 0, 0]],
        }
    }

    /// Regression: a material whose triangles were all degenerate became a trimesh with inverted
    /// infinite bounds; its NaN centre made parry's binned broad-phase build panic (bin index == bin count).
    #[test]
    fn a_trimesh_of_only_degenerate_triangles_is_skipped_and_the_step_does_not_panic() {
        assert!(merge_shapes_of_material([husk_trimesh()].iter(), 0, 1.0, "t").is_empty());

        let mut bodies = RigidBodySet::new();
        let mut colliders = ColliderSet::new();
        for i in 0..9 {
            let shapes = [
                husk_trimesh(),
                ColShape::Box {
                    center: Vec3::ZERO,
                    rotation: Quat::IDENTITY,
                    half_extents: Vec3::ONE,
                },
            ];
            let baked = merge_shapes_of_material(shapes.iter(), 0, 1.0, "t");
            assert_eq!(baked.len(), 1, "only the box survives");
            let body = bodies.insert(RigidBodyBuilder::fixed().translation(Vec3::new(
                i as f32 * 3.0,
                0.0,
                0.0,
            )));
            for cached in &baked {
                colliders.insert_with_parent(
                    ColliderBuilder::new(cached.shape.clone()),
                    body,
                    &mut bodies,
                );
            }
        }
        let mut pipeline = PhysicsPipeline::new();
        let mut islands = IslandManager::new();
        let mut broad_phase = BroadPhaseBvh::new();
        let mut narrow_phase = NarrowPhase::new();
        let mut impulses = ImpulseJointSet::new();
        let mut multibody = MultibodyJointSet::new();
        let mut ccd = CCDSolver::new();
        for _ in 0..3 {
            pipeline.step(
                Vec3::ZERO,
                &IntegrationParameters::default(),
                &mut islands,
                &mut broad_phase,
                &mut narrow_phase,
                &mut bodies,
                &mut colliders,
                &mut impulses,
                &mut multibody,
                &mut ccd,
                &(),
                &(),
            );
        }
    }

    #[test]
    fn a_trimesh_with_a_non_finite_vertex_is_skipped() {
        let broken = ColShape::Trimesh {
            vertices: vec![
                Vec3::new(f32::NAN, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            indices: vec![[0, 1, 2]],
        };
        assert!(merge_shapes_of_material([broken].iter(), 0, 1.0, "t").is_empty());
    }

    #[test]
    fn a_trimesh_keeps_its_valid_triangles_when_a_degenerate_one_is_deleted() {
        let mixed = ColShape::Trimesh {
            vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            indices: vec![[0, 0, 0], [0, 1, 2]],
        };
        let merged = merge_shapes_of_material([mixed].iter(), 0, 1.0, "t");
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].shape.as_trimesh().unwrap().indices().len(), 1);
    }

    #[test]
    fn a_sidecar_capsule_height_is_the_full_height_including_both_caps() {
        let capsule = ColShape::Capsule {
            center: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            radius: 1.0,
            height: 4.0,
        };
        let merged = merge_shapes_of_material([capsule].iter(), 0, 1.0, "t");
        let shape = merged[0].shape.as_capsule().unwrap();
        assert!(
            (shape.half_height() - 1.0).abs() < 1e-6,
            "cylinder half-length is 4/2 - 1"
        );
        assert!((shape.radius - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_capsule_shorter_than_its_caps_keeps_a_sliver_of_cylinder() {
        let capsule = ColShape::Capsule {
            center: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            radius: 1.0,
            height: 1.0,
        };
        let merged = merge_shapes_of_material([capsule].iter(), 0, 1.0, "t");
        assert!((merged[0].shape.as_capsule().unwrap().half_height() - 0.001).abs() < 1e-6);
    }

    #[test]
    fn a_degenerate_convex_hull_is_skipped_without_losing_the_other_shapes() {
        let flat = ColShape::Convex {
            center: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            vertices: vec![Vec3::ZERO, Vec3::X, Vec3::X * 2.0, Vec3::X * 3.0],
        };
        let sphere = ColShape::Sphere {
            center: Vec3::ZERO,
            radius: 1.0,
        };
        let merged = merge_shapes_of_material([flat, sphere].iter(), 0, 1.0, "t");
        assert_eq!(merged.len(), 1);
        assert!(merged[0].shape.as_ball().is_some());
    }

    #[test]
    fn shapes_following_different_joints_bake_to_separate_parts() {
        let part = |joint| PartShape {
            shape: ColShape::Sphere {
                center: Vec3::ZERO,
                radius: 1.0,
            },
            node: None,
            joint,
            material: 0,
        };
        let baked = build_shapes(
            &[part(None), part(Some(3)), part(Some(3)), part(Some(7))],
            1.0,
            "t",
        );
        let joints: Vec<Option<u16>> = baked.iter().map(|cached| cached.joint).collect();
        assert_eq!(joints, [None, Some(3), Some(7)]);
    }

    fn material_sphere(material: u32) -> PartShape {
        PartShape {
            shape: ColShape::Sphere {
                center: Vec3::ZERO,
                radius: 1.0,
            },
            node: None,
            joint: None,
            material,
        }
    }

    #[test]
    fn shapes_bake_to_one_collider_per_material() {
        let baked = build_shapes(
            &[
                material_sphere(7),
                material_sphere(9),
                material_sphere(7),
                material_sphere(0),
            ],
            1.0,
            "t",
        );
        let materials: Vec<u32> = baked.iter().map(|cached| cached.material).collect();
        assert_eq!(materials, [7, 9, 0]);
        assert_eq!(
            baked[0].shape.as_compound().unwrap().shapes().len(),
            2,
            "same-material shapes still merge"
        );
    }

    #[test]
    fn the_material_survives_the_disk_cache() {
        let dir = tempfile::tempdir().unwrap();
        let batch = crate_batch(1.0);
        let load = |_: &str| Arc::new(vec![material_sphere(42)]);
        ShapeBakeCache::new(dir.path().to_owned()).bake_cell(&batch, load, |_| None, |_| None);
        let reloaded = ShapeBakeCache::new(dir.path().to_owned()).bake_cell(
            &batch,
            |_| panic!("served from disk"),
            |_| None,
            |_| None,
        );
        assert_eq!(reloaded.objects[0].shapes[0].material, 42);
    }

    #[test]
    fn blobs_baked_without_materials_live_in_an_older_version_directory() {
        let current = physics_cache_dir("install/assets");
        assert_eq!(current.file_name().unwrap(), "v5");
        assert_ne!(current.file_name().unwrap(), "v4");
    }

    fn ragdoll_batch(sidecar: &Path) -> CellColliderBatch {
        let mut group: ObjectColliderGroup = object_group(
            "meshes\\skeleton.glb",
            vec![placement("0B0001:Fallout4.esm", Vec3::ZERO)],
            true,
            Some(6.0),
        );
        group.ragdoll_sidecar = Some(sidecar.to_str().unwrap().to_owned());
        CellColliderBatch {
            terrain: None,
            objects: vec![group],
        }
    }

    fn one_bone_sidecar() -> Vec<u8> {
        let sidecar = json!({
            "bodies": [{
                "node": "Pelvis", "mass": 2.0, "anchor": false, "damping": [0.1, 0.1],
                "rest": { "t": [0, 1, 0], "r": [0, 0, 0, 1] },
                "shapes": [{ "type": "sphere", "radius": 0.5 }],
            }],
            "constraints": [],
        });
        serde_json::to_vec(&sidecar).unwrap()
    }

    #[test]
    fn ragdoll_sidecars_bake_to_a_spec_and_per_body_shapes_once_per_session() {
        let cache_dir = tempfile::tempdir().unwrap();
        let sidecar_dir = tempfile::tempdir().unwrap();
        let sidecar = sidecar_dir.path().join("skeleton.col.json");
        std::fs::write(&sidecar, one_bone_sidecar()).unwrap();
        let cache = ShapeBakeCache::new(cache_dir.path().to_owned());
        let batch = ragdoll_batch(&sidecar);
        let first = cache.bake_cell(&batch, |_| Arc::new(vec![]), |_| None, |_| None);
        let baked = first.ragdolls[0].outcome.as_ref().unwrap();
        assert_eq!(baked.spec.bodies.len(), 1);
        assert_eq!(baked.body_shapes[0].len(), 1);
        std::fs::remove_file(&sidecar).unwrap();
        let second = cache.bake_cell(&batch, |_| Arc::new(vec![]), |_| None, |_| None);
        assert!(
            second.ragdolls[0].outcome.is_ok(),
            "served from memory, the file is not read again"
        );
    }

    #[test]
    fn a_broken_ragdoll_sidecar_is_a_cached_failure_naming_the_file() {
        let cache_dir = tempfile::tempdir().unwrap();
        let sidecar_dir = tempfile::tempdir().unwrap();
        let sidecar = sidecar_dir.path().join("skeleton.col.json");
        std::fs::write(&sidecar, b"{ broken").unwrap();
        let cache = ShapeBakeCache::new(cache_dir.path().to_owned());
        let batch = ragdoll_batch(&sidecar);
        let first = cache.bake_cell(&batch, |_| Arc::new(vec![]), |_| None, |_| None);
        let reason = first.ragdolls[0].outcome.as_ref().err().unwrap();
        assert!(reason.contains("skeleton.col.json"), "{reason}");
        std::fs::write(&sidecar, one_bone_sidecar()).unwrap();
        let second = cache.bake_cell(&batch, |_| Arc::new(vec![]), |_| None, |_| None);
        assert!(
            second.ragdolls[0].outcome.is_err(),
            "failures are cached so the file is read and logged once"
        );
    }
}
