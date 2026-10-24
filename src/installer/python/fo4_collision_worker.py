"""
Batch FO4 collision extractor. Takes the list of NIFs below a source directory, decodes each NIF's
Havok 2014.1.0-r1 packfile (bhkPhysicsSystem.BinaryData), and writes one
`<relative>.col.json` sidecar per NIF-with-collision into a mirrored output
directory next to where the C# installer drops the corresponding .glb. After the
sidecars are written it indexes them into a small `collision_manifest.json` (mesh
key -> [is_dynamic, mass, ragdoll, water]) the engine loads once at startup, so the engine reads a
sidecar's shapes lazily (only on a cold physics-cache miss) but never opens one
just to learn whether a mesh collides or is dynamic.

Sidecar schema:
  { "shapes": [
      { "type": "sphere",  "center": [x,y,z], "radius": r },
      { "type": "capsule", "center": [x,y,z], "rotation": [x,y,z,w], "radius": r,
        "height": h },       // h = full height incl. both caps, axis = local Y
      { "type": "convex",  "vertices": [[x,y,z], ...] },
      { "type": "trimesh", "vertices": [[x,y,z], ...], "indices": [[a,b,c], ...] },
      // every shape also carries "node": the name of the NiNode that owns the
      // collision object, matching the node name in the converted GLB, and
      // "layer": the integer Havok collision layer (filter bits 0-6) of the body
      // that owns it, static and dynamic alike; omitted (and reported on stderr)
      // when the body's filter could not be decoded
    ],
    "dynamic": true,   // optional; present only for Havok-dynamic bodies
    "mass": 12.5        // optional; body mass, present only when dynamic
    "water": {"layer": 4, "part": 0, "materials": [[hash, shapes]]}  // dynamic only
    // ragdolls additionally carry "version": 2, "bodies", "constraints" - see
    // Ragdoll bodies (v2) contract
  }

Coordinates are converted Havok -> Godot:
  gx = hx * S
  gy = hz * S
  gz = -hy * S
  where S = HAVOK_SCALE = 7.0. PyNifly's hknp decoder emits positions in
  meters (Havok's internal unit). With Godot SCALE=0.1 and a FO4 game unit
  of ~1.43 cm, 1 m corresponds to ~7 Godot units — the same factor the
  legacy C# path uses for bhk*Shape.Dimensions (which are stored in 0.1 m
  Havok ticks and need HAVOK_SCALE*SCALING=0.7). Two different storage
  representations, same Godot-space result.

Shapes carry a transform (body-world or composed compound transform) which is
baked into the emitted vertices so the Godot loader can pass point clouds /
triangle lists straight into ConvexPolygonShape3D / ConcavePolygonShape3D.

Usage:
  python fo4_collision_worker.py --in-dir <nif-source-root>
                                 --files-from <list-file>
                                 --out-dir <glb-dest-root>
                                 [--jobs N] [--skip-existing] [--verbose] [--limit N]

<list-file> is UTF-8 with one NIF path per line, relative to --in-dir, ended by a line feed. The installer lists the NIFs
that may contain collision; the worker still rejects any listed NIF without the physics-system marker.

`--files-from -` reads that list from stdin instead, line by line as the lines arrive, and ends when stdin is closed:
the installer sends each NIF as soon as it is extracted, so the decoding overlaps the extraction. Every line is decoded
once, a malformed or cut-off line fails the run with exit code 2, and the output and the manifest are the same as for the
equivalent list file. Progress lines count existing sidecars that are skipped as done: "... done/received NIFs processed".

This is invoked by the installer's collision lane, which runs beside NifConverter.ConvertToGltfBatched.
"""

from __future__ import annotations

import argparse
import collections
import contextlib
import json
import math
import os
import re
import queue
import struct
import sys
import threading
import time
import traceback
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Iterable, Iterator, List, Mapping, NamedTuple, Optional, Tuple

# Make third_party/pynifly/bhk_autounpack.py importable (repo root = src/installer/python/../../..).
THIRD_PARTY = Path(__file__).resolve().parents[3] / "third_party" / "pynifly"
if str(THIRD_PARTY) not in sys.path:
    sys.path.insert(0, str(THIRD_PARTY))
import bhk_autounpack as bhk  # noqa: E402

HAVOK_SCALE = 7.0
SCALING = 0.1
# PyNifly emits hknp geometry in meters; 1 m ≈ HAVOK_SCALE Godot units.
S = HAVOK_SCALE  # 7.0

MANIFEST_NAME = "collision_manifest.json"
# NIFs between two progress lines; the installer turns "... done/total NIFs processed" lines into step progress.
PROGRESS_INTERVAL = 2500
# A streamed run also prints a progress line after this long without one when NIFs were done meanwhile, so the installer's bar keeps moving.
PROGRESS_MAX_SILENCE_SECONDS = 1.0
# A sidecar is written under this extra suffix and renamed when complete, so a killed worker never leaves a truncated sidecar.
PARTIAL_SUFFIX = ".partial"
# --files-from value that makes the worker read the NIF paths from stdin while they are still being produced.
STDIN_LIST = "-"
# NIFs per pool task, and how long a streamed partial task waits for more paths before it is dispatched anyway.
BATCH_SIZE = 64
STREAM_FLUSH_SECONDS = 0.2
# The name of the meshes folder of the installed layout (InstallLayout.MeshesFolder, the engine's assets/layout.rs).
MESHES_FOLDER = "meshes"
_MASS_RE = re.compile(rb'"mass":([0-9.eE+-]+)')
# The sidecar only writes a "constraints" array when it holds at least one constraint.
_RAGDOLL_MARKER = b'"constraints":[{'
# The top-level "water" object directly follows "dynamic"/"mass" (see process_nif), so a ragdoll
# body's own "water" can never match.
_WATER_RE = re.compile(rb'"dynamic":true,"mass":[^,]+,"water":(\{[^{}]*\})')


def _water_of(blob: bytes):
    match = _WATER_RE.search(blob)
    return json.loads(match.group(1)) if match else None


def _manifest_row(is_dyn: int, mass, is_ragdoll: bool, water) -> list:
    """[dynamic, mass], then the ragdoll flag (1/0) once a ragdoll or water element follows,
    then the water object. Plain static rows stay two elements."""
    row = [is_dyn, mass]
    if is_ragdoll or water is not None:
        row.append(1 if is_ragdoll else 0)
    if water is not None:
        row.append(water)
    return row


def _sidecar_manifest_row(blob: bytes) -> list:
    is_dyn = 1 if b'"dynamic":true' in blob else 0
    mass = None
    if is_dyn:
        m = _MASS_RE.search(blob)
        mass = float(m.group(1)) if m else None
    return _manifest_row(is_dyn, mass, _RAGDOLL_MARKER in blob, _water_of(blob) if is_dyn else None)


def _manifest_key(sidecar_rel: Path) -> str:
    glb_rel = str(sidecar_rel)[: -len(".col.json")] + ".glb"
    return (MESHES_FOLDER + "\\" + glb_rel).replace("/", "\\").lower()


# Reading a sidecar is dominated by per-file open latency, not by CPU, so reads overlap well.
_MANIFEST_READ_THREADS = 16
BELOW_NORMAL_PRIORITY_CLASS = 0x4000


def _read_sidecar_row(sidecar: Path):
    """(row, None) of a sidecar on disk, or (None, the OSError) when it cannot be read."""
    try:
        return _sidecar_manifest_row(sidecar.read_bytes()), None
    except OSError as e:
        return None, e


def build_manifest(out_root: Path, known_rows: Optional[Mapping[Path, list]] = None) -> int:
    """Scan the deployed `.col.json` sidecars and write the collision metadata
    manifest next to them: a JSON object mapping the engine's lowercased mesh key
    "meshes\\<rel>.glb" -> [is_dynamic (0|1), mass (or null)], then a ragdoll flag (1 for a sidecar with
    `bodies` and at least one constraint) and the sidecar's top-level `water` object, each only when
    needed (see _manifest_row). Keys are written sorted by relative path (ordinal) so the bytes do not depend on the
    directory traversal order of the Python version. The engine loads this
    once at startup so cell assembly never opens a sidecar. Lists the files on disk
    (not just this run's output) so it stays complete under --skip-existing; only a sidecar
    without a row in `known_rows` (sidecar path -> row, as this run's workers returned them) is read.

    Robustness matters because the manifest is whole-world: a truncated or
    incomplete manifest makes the engine drop ALL object collision behind one
    warning. So one unreadable sidecar is skipped (not fatal), and the write is
    atomic (temp + rename) so a crash mid-write can't leave a partial manifest."""
    known_rows = known_rows or {}
    sidecars = sorted(out_root.rglob("*.col.json"), key=lambda sidecar: sidecar.relative_to(out_root).as_posix())
    unknown = [col for col in sidecars if col not in known_rows]
    with ThreadPoolExecutor(max_workers=_MANIFEST_READ_THREADS) as pool:
        read_rows = dict(zip(unknown, pool.map(_read_sidecar_row, unknown)))
    manifest = {}
    errors = 0
    for col in sidecars:
        row, error = (known_rows[col], None) if col in known_rows else read_rows[col]
        if error is not None:
            errors += 1
            print(f"  [manifest] skipping unreadable {col}: {error}", file=sys.stderr)
            continue
        manifest[_manifest_key(col.relative_to(out_root))] = row
    tmp = out_root / (MANIFEST_NAME + ".tmp")
    tmp.write_text(json.dumps(manifest, separators=(",", ":")))
    tmp.replace(out_root / MANIFEST_NAME)  # atomic rename
    if errors:
        print(f"  [manifest] {errors} sidecar(s) unreadable, omitted", file=sys.stderr)
    return len(manifest)


# Failures PyNifly raises on malformed or unsupported Havok/NIF data. Anything else is a
# bug and propagates to the per-NIF boundary in main(), which logs and counts it.
DECODE_ERRORS = (RuntimeError, ValueError, IndexError, KeyError, struct.error, OverflowError,
                 ZeroDivisionError)

_problem_count = 0


def _report_message(message: str) -> None:
    """Log a recoverable problem to stderr and count it (never silent)."""
    global _problem_count
    _problem_count += 1
    print(f"  [collision] {message}", file=sys.stderr)


def _report_problem(context: str, exc: Exception) -> None:
    _report_message(f"{context}: {type(exc).__name__}: {exc}")


def _take_problem_count() -> int:
    global _problem_count
    count, _problem_count = _problem_count, 0
    return count


_IDENTITY_ROT = ((1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0))
_IDENTITY_EPSILON = 1e-6
_BONE_SCALE_TOLERANCE = 1e-3
_CAPSULE_AXIS = (0.0, 1.0, 0.0)
_DEGENERATE_CAPSULE_LENGTH = 1e-6
# A constraint's two pivots must meet at the rest pose within this fraction of the bone
# length (but never tighter than the floor, for bones that start at the same point).
_PIVOT_TOLERANCE_FRACTION = 0.02
_PIVOT_TOLERANCE_FLOOR = 0.01
_MAX_REPORTED_DEFECTS = 5

# FO4 shape vertices are node-local: the Havok BodyCInfo pose is redundant with the
# owning NiNode's world transform (its rotation equals the node's world rotation; its
# position is a centre-of-mass hint or zero). Re-applying it would double-rotate mirrored
# statics (RedRExtAllMirror01, RedRExtFlr03), whose visual verts are pre-baked. So a
# top-level body transform is replaced by this identity and the owning node's full world
# transform is applied instead (see _node_world_nif). Compound child transforms are
# instance-local and stay intact.
_NODE_LOCAL_POSE = bhk.BodyTransform(position=(0.0, 0.0, 0.0), rotation=_IDENTITY_ROT)


def _build_parent_map(blocks) -> dict:
    parent_of = {}
    for b in blocks:
        if bhk._is_ninode_like(b["type"]) and b["transform"] is not None:
            for c in b["transform"]["children"]:
                if 0 <= c < len(blocks):
                    parent_of[c] = b["id"]
    return parent_of


def _is_identity_rotation(r) -> bool:
    return all(abs(r[i][j] - (1.0 if i == j else 0.0)) < _IDENTITY_EPSILON
               for i in range(3) for j in range(3))


def _node_world_nif(blocks, parent_of, node_id: int):
    """World transform (rotation, scale, translation) of a node in NIF units:
    v_world = scale * R * v + t. nif_converter.cs strips the parentless root node's
    transform from the GLB (Bethesda's 180 deg Y wrapper), so the root contributes
    only its translation, as the original translation-only code did; rotation and
    scale of every other node on the chain are applied. A chain without any rotation or
    scale is a pure translation sum accumulated leaf-to-root exactly as before, so
    such sidecars stay byte-identical."""
    chain = []
    cur = node_id
    while True:
        b = blocks[cur] if 0 <= cur < len(blocks) else None
        transform = b.get("transform") if b is not None else None
        if cur not in parent_of:
            root_t = transform["t"] if transform is not None else (0.0, 0.0, 0.0)
            break
        if transform is not None:
            chain.append(transform)
        cur = parent_of[cur]

    if all(_is_identity_rotation(n["r"]) and abs(n["s"] - 1.0) < _IDENTITY_EPSILON for n in chain):
        t = [0.0, 0.0, 0.0]
        for n in chain:
            t[0] += n["t"][0]; t[1] += n["t"][1]; t[2] += n["t"][2]
        t[0] += root_t[0]; t[1] += root_t[1]; t[2] += root_t[2]
        return _IDENTITY_ROT, 1.0, (t[0], t[1], t[2])

    rot, scale, trans = _IDENTITY_ROT, 1.0, tuple(root_t)
    for n in reversed(chain):
        moved = _mat_vec(rot, n["t"])
        trans = (trans[0] + scale * moved[0], trans[1] + scale * moved[1], trans[2] + scale * moved[2])
        rot = bhk.mat3_mul(rot, n["r"])
        scale *= n["s"]
    return rot, scale, trans


def _mat_vec(m, v):
    return (m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
            m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
            m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2])


def _owning_node_name(blocks, node_id: int) -> str:
    """Name of the collision-owning NiNode exactly as the C# GLB writer names it
    (unnamed nodes become Node_<block id>)."""
    return blocks[node_id].get("name") or f"Node_{node_id}"


def _swap_axes(v):
    """The one Havok/NIF -> output axis change: (x, y, z) -> (x, z, -y). A proper
    rotation (determinant +1), so it never flips handedness."""
    return (v[0], v[2], -v[1])


def _nif_translation_to_godot(t_nif):
    """NIF (x, y, z) -> Godot (x*SCALING, z*SCALING, -y*SCALING).
    Matches nif_converter.cs GetTransform axis swap.
    """
    v = _swap_axes(t_nif)
    return (v[0] * SCALING, v[1] * SCALING, v[2] * SCALING)


def _rotation_columns_to_godot(rot):
    """C * R: re-express a rotation whose domain is already a converted local frame
    (columns are axes; column 0 stays column 0)."""
    cols = [_swap_axes((rot[0][j], rot[1][j], rot[2][j])) for j in range(3)]
    return tuple(tuple(cols[j][i] for j in range(3)) for i in range(3))


def _rotation_to_godot(rot):
    """C * R * C^T: a rotation between two spaces that both change axes (node world
    pose in model space)."""
    left = _rotation_columns_to_godot(rot)
    rows = [_swap_axes(left[i]) for i in range(3)]
    return tuple(tuple(r) for r in rows)


def _matrix_to_quat(rot):
    """Row-major proper rotation -> quaternion [x, y, z, w] with w >= 0."""
    m = rot
    trace = m[0][0] + m[1][1] + m[2][2]
    if trace > 0.0:
        k = math.sqrt(trace + 1.0) * 2.0
        q = ((m[2][1] - m[1][2]) / k, (m[0][2] - m[2][0]) / k, (m[1][0] - m[0][1]) / k, 0.25 * k)
    elif m[0][0] >= m[1][1] and m[0][0] >= m[2][2]:
        k = math.sqrt(1.0 + m[0][0] - m[1][1] - m[2][2]) * 2.0
        q = (0.25 * k, (m[0][1] + m[1][0]) / k, (m[0][2] + m[2][0]) / k, (m[2][1] - m[1][2]) / k)
    elif m[1][1] >= m[2][2]:
        k = math.sqrt(1.0 + m[1][1] - m[0][0] - m[2][2]) * 2.0
        q = ((m[0][1] + m[1][0]) / k, 0.25 * k, (m[1][2] + m[2][1]) / k, (m[0][2] - m[2][0]) / k)
    else:
        k = math.sqrt(1.0 + m[2][2] - m[0][0] - m[1][1]) * 2.0
        q = ((m[0][2] + m[2][0]) / k, (m[1][2] + m[2][1]) / k, 0.25 * k, (m[1][0] - m[0][1]) / k)
    norm = math.sqrt(sum(c * c for c in q))
    sign = -1.0 if q[3] < 0.0 else 1.0
    return [sign * c / norm for c in q]


def _quat_rotate(q, v):
    """Rotate v by the unit quaternion q = [x, y, z, w]."""
    qv = q[:3]
    t = [2.0 * c for c in _cross(qv, v)]
    u = _cross(qv, t)
    return [v[i] + q[3] * t[i] + u[i] for i in range(3)]


def _cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def _quat_from_axis(direction):
    """Shortest-arc unit quaternion [x, y, z, w] (w >= 0) taking local Y onto the unit
    `direction`. The antiparallel case turns half a revolution about X."""
    x, y, z = _cross(_CAPSULE_AXIS, direction)
    w = 1.0 + direction[1]
    if w < 1e-6:
        return [1.0, 0.0, 0.0, 0.0]
    norm = math.sqrt(x * x + y * y + z * z + w * w)
    return [x / norm, y / norm, z / norm, w / norm]


def _capsule_entry(a, b, radius: float, material: int) -> dict:
    """Sidecar capsule for the segment a-b with the given radius (all in output units);
    a point-like segment degenerates to a sphere."""
    delta = [b[i] - a[i] for i in range(3)]
    length = math.sqrt(sum(c * c for c in delta))
    center = [(a[i] + b[i]) * 0.5 for i in range(3)]
    if length < _DEGENERATE_CAPSULE_LENGTH:
        return {"type": "sphere", "center": center, "radius": radius, "material": material}
    return {
        "type": "capsule",
        "center": center,
        "rotation": _quat_from_axis([c / length for c in delta]),
        "radius": radius,
        "height": length + 2.0 * radius,
        "material": material,
    }


def _capsule_segment(entry):
    """The two segment endpoints of a capsule entry."""
    axis = _quat_rotate(entry["rotation"], _CAPSULE_AXIS)
    half = entry["height"] * 0.5 - entry["radius"]
    c = entry["center"]
    return ([c[i] - axis[i] * half for i in range(3)], [c[i] + axis[i] * half for i in range(3)])


def _place_entries(entries, rot, scale, offset) -> list:
    """New entries for node-local emitted shapes (Godot space) moved into model space:
    v' = scale * rot * v + offset. The inputs are never modified, and no returned dict is one of
    the inputs; unchanged lists and numbers are shared, which is safe because entries are only
    ever rebound, never edited in place. A pure translation takes the exact legacy path."""
    pure_shift = _is_identity_rotation(rot) and abs(scale - 1.0) < _IDENTITY_EPSILON
    if pure_shift and not any(abs(c) > 1e-6 for c in offset):
        return [dict(e) for e in entries]

    ox, oy, oz = offset
    (m00, m01, m02), (m10, m11, m12), (m20, m21, m22) = rot

    def place_all(points):
        if pure_shift:
            return [[x + ox, y + oy, z + oz] for x, y, z in points]
        return [[scale * (m00 * x + m01 * y + m02 * z) + ox,
                 scale * (m10 * x + m11 * y + m12 * z) + oy,
                 scale * (m20 * x + m21 * y + m22 * z) + oz] for x, y, z in points]

    def place(point):
        return place_all([point])[0]

    def placed(e):
        if e["type"] == "sphere":
            moved = {**e, "center": place(e["center"])}
            if not pure_shift:
                moved["radius"] = e["radius"] * scale
            return moved
        if e["type"] == "capsule":
            if pure_shift:
                return {**e, "center": place(e["center"])}
            a, b = _capsule_segment(e)
            return _capsule_entry(place(a), place(b), e["radius"] * scale, e["material"])
        return {**e, "vertices": place_all(e["vertices"])}

    return [placed(e) for e in entries]


def _havok_to_godot(v):
    """(hx, hy, hz) -> (gx, gy, gz) = (hx, hz, -hy) * S."""
    v = _swap_axes(v)
    return (v[0] * S, v[1] * S, v[2] * S)


def _compose(parent, child):
    """Compose two BodyTransforms: result = parent * child (applied rightmost first)."""
    if parent is None:
        return child
    if child is None:
        return parent
    pr = parent.rotation
    cr = child.rotation
    # rot = parent.rot * child.rot
    rr = (
        (pr[0][0] * cr[0][0] + pr[0][1] * cr[1][0] + pr[0][2] * cr[2][0],
         pr[0][0] * cr[0][1] + pr[0][1] * cr[1][1] + pr[0][2] * cr[2][1],
         pr[0][0] * cr[0][2] + pr[0][1] * cr[1][2] + pr[0][2] * cr[2][2]),
        (pr[1][0] * cr[0][0] + pr[1][1] * cr[1][0] + pr[1][2] * cr[2][0],
         pr[1][0] * cr[0][1] + pr[1][1] * cr[1][1] + pr[1][2] * cr[2][1],
         pr[1][0] * cr[0][2] + pr[1][1] * cr[1][2] + pr[1][2] * cr[2][2]),
        (pr[2][0] * cr[0][0] + pr[2][1] * cr[1][0] + pr[2][2] * cr[2][0],
         pr[2][0] * cr[0][1] + pr[2][1] * cr[1][1] + pr[2][2] * cr[2][1],
         pr[2][0] * cr[0][2] + pr[2][1] * cr[1][2] + pr[2][2] * cr[2][2]),
    )
    # pos = parent.rot * child.pos + parent.pos
    cp = child.position
    pp = parent.position
    rp = (
        pr[0][0] * cp[0] + pr[0][1] * cp[1] + pr[0][2] * cp[2] + pp[0],
        pr[1][0] * cp[0] + pr[1][1] * cp[1] + pr[1][2] * cp[2] + pp[1],
        pr[2][0] * cp[0] + pr[2][1] * cp[1] + pr[2][2] * cp[2] + pp[2],
    )
    return bhk.BodyTransform(position=rp, rotation=rr)


def _bake_verts(transform, verts):
    """Havok-space vertices -> output-space point lists: the BodyTransform (when there is one), then
    _havok_to_godot, written out per vertex because this is the hottest loop of the worker."""
    if transform is None:
        return [[v[0] * S, v[2] * S, -v[1] * S] for v in verts]
    (m00, m01, m02), (m10, m11, m12), (m20, m21, m22) = transform.rotation
    px, py, pz = transform.position
    return [[(m00 * v[0] + m01 * v[1] + m02 * v[2] + px) * S,
             (m20 * v[0] + m21 * v[1] + m22 * v[2] + pz) * S,
             -(m10 * v[0] + m11 * v[1] + m12 * v[2] + py) * S] for v in verts]


def _triangulate(faces) -> Tuple[list, list]:
    """Fan-triangulate index faces: (triangles, source face index of each triangle)."""
    tris = []
    source = []
    for face_index, face in enumerate(faces):
        for k in range(1, len(face) - 1):
            tris.append((int(face[0]), int(face[k]), int(face[k + 1])))
            source.append(face_index)
    return tris, source


def _compact_trimesh(vertices, tris) -> Tuple[list, list]:
    """The vertices the triangles use, renumbered in order of first use, with the renumbered triangles."""
    remap: dict = {}
    used = []
    compact = []
    for tri in tris:
        row = []
        for index in tri:
            if index not in remap:
                remap[index] = len(used)
                used.append(vertices[index])
            row.append(remap[index])
        compact.append(row)
    return used, compact


def _is_degenerate_triangle(vertices, tri) -> bool:
    """A triangle the engine's collision build deletes: a repeated index, a non-finite corner or zero area."""
    a, b, c = tri
    if a == b or b == c or a == c:
        return True
    pa, pb, pc = vertices[a], vertices[b], vertices[c]
    if not all(math.isfinite(x) for x in (*pa, *pb, *pc)):
        return True
    u = [pb[i] - pa[i] for i in range(3)]
    w = [pc[i] - pa[i] for i in range(3)]
    cross = (u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0])
    return not any(cross)


def _trimesh_entries(shape, transform, inherited_material: int) -> list:
    """One trimesh entry per material of a compressed mesh. The triangles' own material hashes
    (`shape.face_materials`) win; a mesh without a material table is one entry carrying the material
    inherited from the enclosing shape (0 = none). Entries are ordered by material hash. Degenerate
    triangles are dropped, and a material left with none emits no entry (reported, never a husk)."""
    if not shape.verts or not shape.faces:
        return []
    tris, source_face = _triangulate(shape.faces)
    if not tris:
        return []
    materials = [inherited_material if shape.face_materials is None else shape.face_materials[face]
                 for face in source_face]
    by_material: dict = {}
    for tri, material in zip(tris, materials):
        if not _is_degenerate_triangle(shape.verts, tri):
            by_material.setdefault(material, []).append(tri)
    for material in sorted(set(materials) - set(by_material)):
        _report_message(f"material {material:#010x}: every triangle is degenerate; no trimesh emitted")
    entries = []
    for material in sorted(by_material):
        used, compact = _compact_trimesh(shape.verts, by_material[material])
        entries.append({
            "type": "trimesh",
            "vertices": _bake_verts(transform, used),
            "indices": compact,
            "material": material,
        })
    return entries


def _shape_to_entries(shape, parent_transform, out, material: int = 0):
    """Flatten a CollisionShape tree into ColShapeEntry dicts.

    `material` is the BGS material hash inherited from the enclosing shape; a shape with its own
    hash (the shape's user data) overrides it. It is stamped onto every emitted non-compound entry."""
    material = shape.material_hash or material
    if shape.shape_type == "compound":
        composed = _compose(parent_transform, shape.transform)
        for c in shape.children:
            _shape_to_entries(c, composed, out, material=material)
        return

    tr = _compose(parent_transform, shape.transform)

    if shape.shape_type == "sphere":
        hv = tr.position if tr else (0.0, 0.0, 0.0)
        gv = _havok_to_godot(hv)
        out.append({
            "type": "sphere",
            "center": [gv[0], gv[1], gv[2]],
            "radius": shape.sphere_radius * S,
            "material": material,
        })
        return

    if shape.shape_type == "capsule":
        a, b = _bake_verts(tr, shape.verts)
        out.append(_capsule_entry(a, b, shape.sphere_radius * S, material))
        return

    if shape.shape_type == "polytope":
        if not shape.verts:
            return
        out.append({
            "type": "convex",
            "vertices": _bake_verts(tr, shape.verts),
            "material": material,
        })
        return

    if shape.shape_type == "compressed_mesh":
        out.extend(_trimesh_entries(shape, tr, material))
        return

    raise ValueError(f"unsupported shape type {shape.shape_type!r} ({shape.name})")


def _frame_to_output(frame) -> dict:
    """Constraint frame (Havok metres, body-local) -> output axes and units. The rotation
    keeps column 0 (twist/hinge axis) as the frame's local X axis."""
    rot = _rotation_columns_to_godot(frame.rotation)
    return {"t": list(_havok_to_godot(frame.position)), "r": _matrix_to_quat(rot)}


def _constraint_to_entry(nif_path: Path, c) -> dict:
    entry = {
        "type": c.kind,
        "child": c.child,
        "parent": c.parent,
        "frame_child": _frame_to_output(c.frame_child),
        "frame_parent": _frame_to_output(c.frame_parent),
    }
    if c.kind == "ragdoll":
        if c.cone_min is not None:
            _report_message(f"{nif_path}: ragdoll constraint {c.child}->{c.parent} has a lower cone "
                            f"bound ({c.cone_min}) the sidecar cannot express; ignored")
        entry["twist"] = list(c.twist)
        cone_bounded = math.isfinite(c.cone_max) and abs(c.cone_max) < -bhk.CONE_UNBOUNDED_SENTINEL
        entry["cone"] = c.cone_max if cone_bounded else None
        entry["plane"] = list(c.plane)
    else:
        entry["hinge"] = list(c.hinge)
    entry["friction"] = c.friction
    return entry


def _constraint_limit_defect(c):
    """Why a constraint's limits cannot be trusted, or None. Infinite cone bounds are a
    legitimate "unbounded"; NaN anywhere is corrupt data."""
    limits = {"friction": [c.friction], "twist": c.twist, "plane": c.plane, "hinge": c.hinge,
              "cone": None if c.cone_max is None else [c.cone_max]}
    for name, values in limits.items():
        if values is not None and any(math.isnan(v) for v in values):
            return f"constraint {c.child}->{c.parent} has a NaN {name} limit"
    return None


def _pivot_defect(bodies, constraint) -> str:
    """Why the constraint's two pivots do not meet at the rest pose, or None. A mismatch
    would make the joint snap at spawn."""
    child = bodies[constraint["child"]]
    parent = bodies[constraint["parent"]]

    def world_pivot(body, frame):
        offset = _quat_rotate(body["rest"]["r"], frame["t"])
        return [body["rest"]["t"][i] + offset[i] for i in range(3)]

    pc = world_pivot(child, constraint["frame_child"])
    pp = world_pivot(parent, constraint["frame_parent"])
    error = math.dist(pc, pp)
    bone_length = math.dist(child["rest"]["t"], parent["rest"]["t"])
    tolerance = max(_PIVOT_TOLERANCE_FRACTION * bone_length, _PIVOT_TOLERANCE_FLOOR)
    if error > tolerance:
        return (f"constraint {child['node']}->{parent['node']} pivots are {error:.3f} units apart "
                f"at the rest pose (tolerance {tolerance:.3f})")
    return None


def _disconnected_bodies(bodies, constraints) -> list:
    """Indices of dynamic bodies outside the largest group of constraint-connected bodies.
    A loose anchor never moves, so it cannot detach from the ragdoll."""
    body_count = len(bodies)
    group = list(range(body_count))

    def find(i):
        while group[i] != i:
            group[i] = group[group[i]]
            i = group[i]
        return i

    for c in constraints:
        group[find(c["child"])] = find(c["parent"])
    sizes: dict = {}
    for i in range(body_count):
        sizes.setdefault(find(i), []).append(i)
    if len(sizes) <= 1:
        return []
    biggest = max(sizes.values(), key=len)
    return [i for i in range(body_count) if i not in biggest and not bodies[i]["anchor"]]


def _ragdoll_defects(bodies, constraints, system) -> list:
    """Reasons this physics system cannot be driven as a complete ragdoll (empty when it
    can): unsupported constraints, shapeless bodies, free bodies, mismatched pivots."""
    defects = list(system.problems)
    for i, body in enumerate(bodies):
        if not body["anchor"] and not body["shapes"]:
            defects.append(f"body {i} ({body['node']}, shape class {system.shape_classes[i]}) "
                           f"has no decoded shape")
    free = _disconnected_bodies(bodies, constraints)
    if free:
        defects.append("dynamic bodies not connected by constraints to the rest: "
                       + ", ".join(f"{i} ({bodies[i]['node']})" for i in free))
    defects.extend(d for d in (_pivot_defect(bodies, c) for c in constraints) if d)
    return defects


def _node_parents(parent_of, owner_ids) -> list:
    """Per body, the index of the body owning the nearest NiNode ancestor of its owner
    that owns a body, or None when there is none. XRGD poses are relative to this
    node hierarchy, which can differ from the constraint hierarchy."""
    body_of_node = {}
    for index, owner_id in enumerate(owner_ids):
        body_of_node.setdefault(owner_id, index)
    parents = []
    for owner_id in owner_ids:
        cur = parent_of.get(owner_id)
        while cur is not None and cur not in body_of_node:
            cur = parent_of.get(cur)
        parents.append(None if cur is None else body_of_node[cur])
    return parents


def _material_counts(entries) -> list:
    """[[material hash, shape count], ...] sorted by hash: one count per emitted leaf shape, the
    "shape keys" counted per material for one body."""
    counts = collections.Counter(entry["material"] for entry in entries)
    return [[material, count] for material, count in sorted(counts.items())]


def _water_entry(layer: int, part: int, entries) -> dict:
    return {"layer": layer, "part": part, "materials": _material_counts(entries)}


def _body_water(physics, entries) -> dict:
    """The water inputs of one Havok body: collision layer, ragdoll part id (filter bits 8-12) and the
    material histogram of its shapes."""
    return _water_entry(physics.layer, physics.part, entries)


def _shape_layer(physics_by_pid: dict, pid: int, body_index: Optional[int]) -> Optional[int]:
    """The Havok collision layer (filter bits 0-6) of the body that owns a top-level shape, or None
    when the system's body filters were not decoded or the shape has no valid body index."""
    bodies = physics_by_pid.get(pid)
    if bodies is None or body_index is None or not 0 <= body_index < len(bodies):
        return None
    return bodies[body_index].layer


def _nif_water(physics_by_pid: dict, body_shapes: dict):
    """Water inputs of a Havok-dynamic NIF, or None without a simulated body. The engine turns the
    whole NIF into one rigid body, so the materials of every simulated body are pooled; layer and part
    are the heaviest simulated body's."""
    simulated = [(pid, index, physics) for pid, bodies in physics_by_pid.items()
                 for index, physics in enumerate(bodies) if not physics.motion.anchor]
    if not simulated:
        return None
    _, _, heaviest = max(simulated, key=lambda body: body[2].motion.mass)
    pooled = [entry for pid, index, _ in simulated for entry in body_shapes.get((pid, index), [])]
    return _water_entry(heaviest.layer, heaviest.part, pooled)


def _build_ragdoll(nif_path: Path, blocks, parent_of, body_owners, pid: int, hk: bytes,
                   body_shapes: dict, body_offset: int):
    """(bodies, constraints, defects) of one physics system, or None when it carries no
    supported constraints. `body_shapes` maps (pid, body index) -> node-local shape entries.
    A non-empty `defects` means the system is not a complete ragdoll. Raises DECODE_ERRORS
    on malformed structure; the caller reports it and keeps the legacy shapes."""
    system = bhk.parse_physics_system_bodies(hk)
    if system is None:
        return None
    physics = bhk.parse_body_physics(hk)
    if len(physics) != len(system.motions):
        raise ValueError(f"{len(physics)} body records for {len(system.motions)} motions")
    bodies = []
    owner_ids = []
    for index, motion in enumerate(system.motions):
        owner_id = body_owners.get((pid, index))
        if owner_id is None:
            raise ValueError(f"body {index} has no owning NiNode")
        owner_ids.append(owner_id)
        rot, scale, trans = _node_world_nif(blocks, parent_of, owner_id)
        if abs(scale - 1.0) > _BONE_SCALE_TOLERANCE:
            _report_message(f"{nif_path}: body {index} node scale {scale:.4f} is not representable; "
                            f"rest pose carries no scale")
        bodies.append({
            "node": _owning_node_name(blocks, owner_id),
            "mass": 0.0 if motion.anchor else round(motion.mass, 6),
            "anchor": motion.anchor,
            "damping": [round(motion.linear_damping, 6), round(motion.angular_damping, 6)],
            "rest": {"t": list(_nif_translation_to_godot(trans)),
                     "r": _matrix_to_quat(_rotation_to_godot(rot))},
            "shapes": body_shapes.get((pid, index), []),
            "water": _body_water(physics[index], body_shapes.get((pid, index), [])),
        })
    for body, node_parent in zip(bodies, _node_parents(parent_of, owner_ids)):
        body["node_parent"] = None if node_parent is None else node_parent + body_offset
    nan_defects = [d for d in map(_constraint_limit_defect, system.constraints) if d]
    constraints = [_constraint_to_entry(nif_path, c) for c in system.constraints]
    defects = nan_defects + _ragdoll_defects(bodies, constraints, system)
    for entry in constraints:
        entry["child"] += body_offset
        entry["parent"] += body_offset
    return bodies, constraints, defects


def _report_not_ragdoll(nif_path: Path, pid: int, defects: list) -> None:
    shown = "; ".join(defects[:_MAX_REPORTED_DEFECTS])
    extra = len(defects) - _MAX_REPORTED_DEFECTS
    suffix = f" (+{extra} more)" if extra > 0 else ""
    _report_message(f"{nif_path}: physics system {pid} is not a complete ragdoll, "
                    f"legacy shapes only: {shown}{suffix}")


class SidecarResult(NamedTuple):
    shape_count: int
    manifest_row: list


_PHYSICS_SYSTEM_BLOCK_TYPE = b"bhkPhysicsSystem"


def _may_contain_collision(nif_bytes: bytes) -> bool:
    """False when the NIF provably yields no sidecar. A sidecar needs a (NiNode, bhkPhysicsSystem)
    pair, i.e. a block whose type name in the header's block-type table is exactly
    "bhkPhysicsSystem" (bhk._resolve_collision_physics_pairs); that table stores the name as
    plain ASCII, so a file without those bytes has no such block."""
    return _PHYSICS_SYSTEM_BLOCK_TYPE in nif_bytes


def process_nif(nif_path: Path, out_path: Path) -> Optional[SidecarResult]:
    """Decode collision from one NIF and write a `.col.json` sidecar. Returns the shape count
    and manifest row of the sidecar written, or None if nothing was written (file not created on empty)."""
    data = nif_path.read_bytes()
    if not _may_contain_collision(data):
        return None
    try:
        blocks, _ = bhk._parse_nif_blocks(data)
    except DECODE_ERRORS as e:
        _report_problem(f"{nif_path}: NIF blocks unreadable", e)
        return None
    pairs = bhk._resolve_collision_physics_pairs(blocks)
    if not pairs:
        return None

    # Dedupe by physics-system id: some NIFs (stairs kits, catwalks, stadium
    # stands, self-destruct robots) have the same bhkPhysicsSystem referenced
    # from multiple NiNodes — typically via duplicate bhkNPCollisionObjects that
    # point back to one shared body. Instancing the collision at each node
    # produces phantom copies offset from the (single) visual mesh. Keep only
    # the first (smallest-id) pair for each pid; in practice that's the root
    # reference when both root and a child exist, and root matches where the
    # visual BSTriShapes sit.
    seen_pids = set()
    picks: list = []
    for node_id, pid in pairs:
        if pid in seen_pids:
            continue
        picks.append((node_id, pid))
        seen_pids.add(pid)

    body_owners = bhk.resolve_collision_body_owners(blocks)
    parent_of = _build_parent_map(blocks)
    node_to_phys = {}
    for node_id, pid in picks:
        node_to_phys.setdefault(node_id, []).append(pid)

    entries = []
    body_shapes: dict = {}       # (physics system id, body index) -> node-local entries
    ragdoll_systems = []         # (pid, hk) of every physics system to try as a ragdoll
    physics_by_pid: dict = {}    # pid -> [BodyPhysics], for the water summary of a dynamic NIF
    # Havok dynamic-body signal for the whole NIF. The FO4 packfile has no
    # MO_SYS_* enum (see third_party/pynifly/fo4_havok_packfile_format.md); the
    # decoder derives is_dynamic from the presence of the dyn-motion/dyn-inertia
    # arrays and reads the body mass alongside it. parse_physics_props attaches
    # the same PhysicsProps to every top-level shape (per-NIF, first-body
    # granularity), so any shape's `.physics` carries it. A NIF is dynamic when
    # a body is simulated (is_dynamic) and has a positive mass.
    nif_dynamic = False
    nif_mass = 0.0
    layerless_reported = set()   # (physics system id, body index) already reported as having no layer
    for node_id, pids in node_to_phys.items():
        seen_pid = set()
        node_shapes = []  # (physics system id, shape)
        for pid in pids:
            if pid in seen_pid:
                continue
            seen_pid.add(pid)
            blob = blocks[pid]["blob"]
            if blob.startswith(bhk.HAVOK_MAGIC):
                hk = blob
            elif len(blob) >= 12 and blob[4:12] == bhk.HAVOK_MAGIC:
                hk = blob[4:]
            else:
                continue
            try:
                node_shapes.extend((pid, shape) for shape in bhk.parse_bytes(hk))
            except DECODE_ERRORS as e:
                _report_problem(f"{nif_path}: physics system {pid} undecodable", e)
                continue
            if bhk.has_supported_constraints(hk):
                ragdoll_systems.append((pid, hk))
            try:
                physics_by_pid[pid] = bhk.parse_body_physics(hk)
            except DECODE_ERRORS as e:
                _report_problem(f"{nif_path}: physics system {pid} body filters undecodable", e)

        for pid, s in node_shapes:
            if s.physics is not None and s.physics.is_dynamic and s.physics.mass > 0.0:
                nif_dynamic = True
                if s.physics.mass > nif_mass:
                    nif_mass = s.physics.mass
            # Drop body transform on top-level shape. Compound-child
            # transforms are instance-local and stay untouched.
            if s.transform is not None:
                s.transform = _NODE_LOCAL_POSE
            local_entries: list = []
            try:
                _shape_to_entries(s, None, local_entries)
            except DECODE_ERRORS as e:
                _report_problem(f"{nif_path}: shape of physics system {pid} undecodable", e)
                continue

            # The name and the placement offset come from the same node: the one that
            # owns this body, so the entry lands where the GLB writer put that node.
            body_key = (pid, s.body_index)
            if s.body_index is not None:
                body_shapes.setdefault(body_key, []).extend(local_entries)

            owner_id = body_owners.get(body_key, node_id)
            rot, scale, trans = _node_world_nif(blocks, parent_of, owner_id)
            owner_name = _owning_node_name(blocks, owner_id)
            layer = _shape_layer(physics_by_pid, pid, s.body_index)
            if layer is None and body_key not in layerless_reported:
                layerless_reported.add(body_key)
                if s.body_index is None:
                    _report_message(f"{nif_path}: a shape of physics system {pid} has no body index; "
                                    f"its shapes carry no layer")
                else:
                    _report_message(f"{nif_path}: physics system {pid} body {s.body_index} has no decodable "
                                    f"collision layer; its shapes carry no layer")
            for placed in _place_entries(local_entries, _rotation_to_godot(rot), scale,
                                         _nif_translation_to_godot(trans)):
                placed["node"] = owner_name
                if layer is not None:
                    placed["layer"] = layer
                entries.append(placed)

    if not entries:
        return None

    # Static sidecars carry no `dynamic`/`mass` keys; only Havok-dynamic NIFs do,
    # which the engine consumes to spawn rigid bodies (the Rust reader defaults
    # the absent keys to static).
    payload = {"shapes": entries}
    if nif_dynamic:
        payload["dynamic"] = True
        payload["mass"] = nif_mass
        water = _nif_water(physics_by_pid, body_shapes)
        if water is not None:
            payload["water"] = water  # must directly follow "mass": _WATER_RE anchors on it

    bodies = []
    constraints = []
    complete = True
    for pid, hk in ragdoll_systems:
        try:
            built = _build_ragdoll(nif_path, blocks, parent_of, body_owners, pid, hk, body_shapes, len(bodies))
        except DECODE_ERRORS as e:
            _report_problem(f"{nif_path}: ragdoll bodies of physics system {pid} undecodable", e)
            complete = False
            continue
        if built is None:
            continue
        system_bodies, system_constraints, defects = built
        if defects:
            _report_not_ragdoll(nif_path, pid, defects)
            complete = False
            continue
        bodies.extend(system_bodies)
        constraints.extend(system_constraints)
    if complete and bodies:
        payload["version"] = 2
        payload["bodies"] = bodies
        payload["constraints"] = constraints

    blob = json.dumps(payload, separators=(",", ":")).encode("ascii")
    _write_atomically(out_path, blob)
    return SidecarResult(len(entries), _sidecar_manifest_row(blob))


def _write_atomically(out_path: Path, blob: bytes) -> None:
    out_path.parent.mkdir(parents=True, exist_ok=True)
    partial_path = out_path.with_name(out_path.name + PARTIAL_SUFFIX)
    partial_path.write_bytes(blob)
    os.replace(partial_path, out_path)


def remove_stale_partials(out_root: Path) -> int:
    """Deletes the half-written sidecars a killed run left behind; returns how many."""
    stale = list(out_root.rglob("*" + PARTIAL_SUFFIX))
    for path in stale:
        path.unlink()
    return len(stale)


class BatchResult(NamedTuple):
    done: int
    with_shapes: int
    errors: int
    problems: int
    rows: List[tuple]  # (sidecar path, manifest row) of every sidecar written


def _process_batch(batch) -> BatchResult:
    """Pool worker: decode a list of (nif_path_str, out_path_str), writing one
    .col.json per NIF with collision. Errors are NIFs that failed outright, problems are
    logged recoverable decode issues."""
    done = 0
    errors = 0
    rows = []
    for nif_str, out_str in batch:
        done += 1
        try:
            result = process_nif(Path(nif_str), Path(out_str))
            if result is not None:
                rows.append((out_str, result.manifest_row))
        except Exception as e:  # per-NIF boundary: log and count, never swallow
            errors += 1
            print(f"  [collision] {nif_str}: FAILED {type(e).__name__}: {e}", file=sys.stderr)
    return BatchResult(done, len(rows), errors, _take_problem_count(), rows)


def lower_own_priority() -> None:
    """Runs this process (and the pool workers it starts, which inherit the class) below the installer's other lanes.
    Needed here because the installer cannot be sure its handle is the real interpreter and not a launcher in front of it."""
    if sys.platform != "win32":
        return
    import ctypes
    from ctypes import wintypes
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.GetCurrentProcess.restype = wintypes.HANDLE
    kernel32.SetPriorityClass.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    kernel32.SetPriorityClass.restype = wintypes.BOOL
    if not kernel32.SetPriorityClass(kernel32.GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS):
        raise ctypes.WinError(ctypes.get_last_error())


class NifListError(Exception):
    """The list of NIFs could not be read, or holds a line that is not a path below the source root."""


def parse_list_entry(entry: str, origin: str, number: int) -> Path:
    """The relative path on a list line. Raises ValueError for a line that is empty, absolute or leaves the source root."""
    path = Path(entry)
    if not entry or path.anchor or ".." in path.parts:
        raise ValueError(f"{origin}:{number}: not a path below the source root: {entry!r}")
    return path


def read_nif_list(list_path: Path) -> List[Path]:
    """The relative NIF paths of a --files-from file. Raises ValueError for a line that is empty, absolute or leaves the source root."""
    with list_path.open(encoding="utf-8", newline="\n") as lines:
        entries = [line.removesuffix("\n") for line in lines]
    return [parse_list_entry(entry, str(list_path), number) for number, entry in enumerate(entries, 1)]


def iter_stdin_list(stream) -> Iterator[Path]:
    """The relative NIF paths of a binary stream in the --files-from format, yielded as their lines arrive. A last line
    without its line feed was cut off while it was written, so it is an error and never a path."""
    for number, raw in enumerate(stream, 1):
        if not raw.endswith(b"\n"):
            raise ValueError(f"<stdin>:{number}: the line is not terminated, the list was cut off")
        try:
            entry = raw[:-1].decode("utf-8")
        except UnicodeDecodeError as e:
            raise ValueError(f"<stdin>:{number}: not UTF-8: {e}") from e
        yield parse_list_entry(entry, "<stdin>", number)


def _list_errors_as_nif_list_error(entries: Iterable[Path]) -> Iterator[Path]:
    try:
        yield from entries
    except (OSError, ValueError) as e:
        raise NifListError(str(e)) from e


class RunSummary(NamedTuple):
    processed: int
    skipped: int
    with_shapes: int
    errors: int
    problems: int
    rows: dict  # sidecar path -> manifest row of every sidecar this run wrote


class StreamTally:
    """What a streamed run has seen so far. Skipped NIFs count as done, so done/received reaches 1 when every path
    received has been dealt with, which is the progress the installer shows against the paths it has sent."""

    def __init__(self, clock=time.monotonic) -> None:
        self._clock = clock
        self._last_report_time = clock()
        self._last_reported_done = 0
        self.received = 0
        self.skipped = 0
        self.processed = 0
        self.with_shapes = 0
        self.errors = 0
        self.problems = 0
        self.rows: dict = {}

    @property
    def done(self) -> int:
        return self.skipped + self.processed

    def summary(self) -> RunSummary:
        return RunSummary(self.processed, self.skipped, self.with_shapes, self.errors, self.problems, self.rows)

    def receive(self) -> None:
        self.received += 1

    def skip(self) -> None:
        self.received += 1
        self.skipped += 1
        self._report_progress_since(self.done - 1)

    def add_batch(self, batch: BatchResult) -> None:
        before = self.done
        self.processed += batch.done
        self.with_shapes += batch.with_shapes
        self.errors += batch.errors
        self.problems += batch.problems
        self.rows.update((Path(out_str), row) for out_str, row in batch.rows)
        self._report_progress_since(before)

    def add_single(self, out_path: Path, result: Optional[SidecarResult], failed: bool) -> None:
        before = self.done
        self.processed += 1
        self.errors += failed
        if result is not None:
            self.with_shapes += 1
            self.rows[out_path] = result.manifest_row
        self._report_progress_since(before)

    def _report_progress_since(self, done_before: int) -> None:
        now = self._clock()
        interval_crossed = self.done // PROGRESS_INTERVAL != done_before // PROGRESS_INTERVAL
        silent_too_long = self.done != self._last_reported_done and now - self._last_report_time >= PROGRESS_MAX_SILENCE_SECONDS
        if interval_crossed or silent_too_long:
            self._last_report_time = now
            self._last_reported_done = self.done
            print(f"  ... {self.done}/{self.received} NIFs processed, {self.with_shapes} sidecars written, {self.errors} errors")


def _process_single(nif_str: str, out_str: str, verbose: bool) -> Tuple[Optional[SidecarResult], bool]:
    """In-process decode of one NIF: (result, False), or (None, True) when it failed, which is logged and never raised."""
    try:
        result = process_nif(Path(nif_str), Path(out_str))
    except Exception as e:  # per-NIF boundary: log and count, never swallow
        print(f"  FAIL {nif_str}: {type(e).__name__}: {e}", file=sys.stderr)
        if verbose:
            traceback.print_exc(file=sys.stderr)
        return None, True
    if result is not None and verbose:
        print(f"  OK   {nif_str}  ({result.shape_count} shapes)")
    return result, False


def _work_item(rel: Path, in_root: Path, out_root: Path, skip_existing: bool, tally: StreamTally) -> Optional[Tuple[str, str]]:
    """(nif path, sidecar path) of a listed NIF to decode, or None when its sidecar exists and is kept."""
    out_path = out_root / rel.with_suffix(".col.json")
    if skip_existing and out_path.exists():
        tally.skip()
        return None
    tally.receive()
    return str(in_root / rel), str(out_path)


_END_OF_LIST = object()


class _ListFailure:
    def __init__(self, error: Exception) -> None:
        self.error = error


def _pump_entries(entries: Iterable[Path], sink: "queue.Queue") -> None:
    """Reader thread: hands every listed path to the dispatcher, then the end of the list or the exception that ended it,
    so the dispatcher never waits on a list that is already over."""
    try:
        for rel in entries:
            sink.put(rel)
    except Exception as e:  # raised again by the dispatcher on its own thread
        sink.put(_ListFailure(e))
    else:
        sink.put(_END_OF_LIST)


def _streamed_batches(source: "queue.Queue", in_root: Path, out_root: Path, skip_existing: bool,
                      tally: StreamTally) -> Iterator[Optional[list]]:
    """Groups the pending NIFs of the list into batches as the paths arrive. Yields a full batch at once and a partial one
    once STREAM_FLUSH_SECONDS pass without a new path; yields None at that point when there is nothing to dispatch, so the
    caller can collect results while it waits. Raises what ended the list."""
    batch: list = []
    while True:
        try:
            item = source.get(timeout=STREAM_FLUSH_SECONDS)
        except queue.Empty:
            yield batch or None
            batch = []
            continue
        if item is _END_OF_LIST:
            if batch:
                yield batch
            return
        if isinstance(item, _ListFailure):
            raise item.error
        work = _work_item(item, in_root, out_root, skip_existing, tally)
        if work is None:
            continue
        batch.append(work)
        if len(batch) >= BATCH_SIZE:
            yield batch
            batch = []


def _collect_ready(pending: list, tally: StreamTally) -> list:
    """Adds the finished batches of `pending` to the tally and returns the ones still running. A batch that raised
    (a worker process died or returned something unpicklable) raises here."""
    running = []
    for result in pending:
        if result.ready():
            tally.add_batch(result.get())
        else:
            running.append(result)
    return running


def _run_stream_in_pool(entries: Iterable[Path], jobs: int, in_root: Path, out_root: Path, skip_existing: bool,
                        tally: StreamTally) -> None:
    import multiprocessing as mp
    source: queue.Queue = queue.Queue()
    threading.Thread(target=_pump_entries, args=(entries, source), daemon=True, name="nif-list-reader").start()
    with contextlib.ExitStack() as pools:
        pool = None
        pending: list = []
        for batch in _streamed_batches(source, in_root, out_root, skip_existing, tally):
            if batch:
                if pool is None:  # the processes start with the first NIF, not while the list is still empty
                    pool = pools.enter_context(mp.Pool(processes=jobs))
                pending.append(pool.apply_async(_process_batch, (batch,)))
            pending = _collect_ready(pending, tally)
        for result in pending:
            tally.add_batch(result.get())


def _run_stream_in_process(entries: Iterable[Path], in_root: Path, out_root: Path, skip_existing: bool, verbose: bool,
                           tally: StreamTally) -> None:
    for rel in entries:
        work = _work_item(rel, in_root, out_root, skip_existing, tally)
        if work is None:
            continue
        result, failed = _process_single(*work, verbose)
        tally.add_single(Path(work[1]), result, failed)


def run_stream(entries: Iterable[Path], jobs: int, in_root: Path, out_root: Path, skip_existing: bool, verbose: bool) -> StreamTally:
    """Decodes the NIFs of `entries` as they arrive, in `jobs` processes (in this process for one). Raises NifListError when the list ends in an error."""
    tally = StreamTally()
    checked = _list_errors_as_nif_list_error(entries)
    if jobs > 1:
        _run_stream_in_pool(checked, jobs, in_root, out_root, skip_existing, tally)
    else:
        _run_stream_in_process(checked, in_root, out_root, skip_existing, verbose, tally)
    return tally


def run_listed(listed: List[Path], jobs: int, in_root: Path, out_root: Path, skip_existing: bool, limit: int,
               verbose: bool) -> RunSummary:
    """Decodes the NIFs of a list that is complete before the work starts, in `jobs` processes (in this process for one)."""
    work = []
    skipped = 0
    for rel in listed:
        if limit and len(work) >= limit:
            break
        out_path = out_root / rel.with_suffix(".col.json")
        if skip_existing and out_path.exists():
            skipped += 1
            continue
        work.append((str(in_root / rel), str(out_path)))

    print(f"  {len(work)} NIFs to process ({skipped} existing sidecars skipped)")

    total = 0
    with_shapes = 0
    err = 0
    problems = 0
    written_rows: dict = {}

    if jobs > 1:
        import multiprocessing as mp
        batches = [work[i:i + BATCH_SIZE] for i in range(0, len(work), BATCH_SIZE)]
        with mp.Pool(processes=jobs) as pool:
            for batch in pool.imap_unordered(_process_batch, batches):
                total += batch.done
                with_shapes += batch.with_shapes
                err += batch.errors
                problems += batch.problems
                written_rows.update((Path(out_str), row) for out_str, row in batch.rows)
                if total // PROGRESS_INTERVAL != (total - batch.done) // PROGRESS_INTERVAL:
                    print(f"  ... {total}/{len(work)} NIFs processed, {with_shapes} sidecars written, {err} errors")
    else:
        for nif_str, out_str in work:
            total += 1
            result, failed = _process_single(nif_str, out_str, verbose)
            err += failed
            if result is not None:
                with_shapes += 1
                written_rows[Path(out_str)] = result.manifest_row
            if total % PROGRESS_INTERVAL == 0:
                print(f"  ... {total}/{len(work)} NIFs processed, {with_shapes} sidecars written, {err} errors")
    return RunSummary(total, skipped, with_shapes, err, problems, written_rows)


def main() -> int:
    try:
        lower_own_priority()
    except OSError as e:
        print(f"WARNING: could not lower the priority of the collision worker: {e}", file=sys.stderr)
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--in-dir", required=True, help="NIF source root the listed paths are relative to")
    ap.add_argument("--files-from", required=True,
                    help=f"UTF-8 file listing the NIFs to process, one path relative to --in-dir per line; {STDIN_LIST} reads the "
                         "list from stdin while it is still being written and ends with it")
    ap.add_argument("--out-dir", required=True, help="Output root — mirrors .col.json next to .glb")
    ap.add_argument("--limit", type=int, default=0, help="Stop after N NIFs (0 = no limit); not with a list read from stdin")
    ap.add_argument("--skip-existing", action="store_true",
                    help="Skip NIFs whose .col.json already exists (cheap resume/re-run)")
    ap.add_argument("--jobs", type=int, default=1,
                    help="Worker processes (1 = in-process sequential, the historical behavior)")
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()
    streaming = args.files_from == STDIN_LIST
    if streaming and args.limit:
        ap.error(f"--limit cannot be combined with --files-from {STDIN_LIST}")

    in_root = Path(args.in_dir).resolve()
    out_root = Path(args.out_dir).resolve()
    if not in_root.is_dir():
        print(f"ERROR: --in-dir not a directory: {in_root}", file=sys.stderr)
        return 2
    out_root.mkdir(parents=True, exist_ok=True)
    stale = remove_stale_partials(out_root)
    if stale:
        print(f"  removed {stale} half-written sidecars of an interrupted run")

    if streaming:
        print("  reading NIF paths from stdin until it is closed")
        try:
            summary = run_stream(iter_stdin_list(sys.stdin.buffer), args.jobs, in_root, out_root, args.skip_existing,
                                 args.verbose).summary()
        except NifListError as e:
            print(f"ERROR: cannot read the NIF list from stdin: {e}", file=sys.stderr)
            return 2
    else:
        try:
            listed = read_nif_list(Path(args.files_from))
        except (OSError, UnicodeDecodeError, ValueError) as e:
            print(f"ERROR: cannot read --files-from: {e}", file=sys.stderr)
            return 2
        summary = run_listed(listed, args.jobs, in_root, out_root, args.skip_existing, args.limit, args.verbose)

    # Index the deployed sidecars into the collision metadata manifest the engine
    # loads at startup (covers this run's output plus any --skip-existing files).
    problems = summary.problems + _take_problem_count()
    entries = build_manifest(out_root, summary.rows)

    print(f"Collision worker: {summary.processed} NIFs, {summary.with_shapes} sidecars written, "
          f"{summary.skipped} skipped (existing), {summary.errors} errors, {problems} decode problems; manifest {entries} entries")
    return 0


if __name__ == "__main__":
    sys.exit(main())
