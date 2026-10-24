"""Unit tests of the water-physics inputs the collision worker writes:
material hashes on shapes, the per-body layer / part / material histogram, and the manifest row."""

import contextlib
import copy
import io
import json
import os
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "src" / "installer" / "python"))
import fo4_collision_worker as worker  # noqa: E402

bhk = worker.bhk

def setUpModule():
    """main() lowers the priority of its own process; the tests that run it in-process must not do that to the test runner."""
    patcher = mock.patch.object(worker, "lower_own_priority")
    patcher.start()
    unittest.addModuleCleanup(patcher.stop)


STONE = 0xDF02F237
WOOD = 0x1DD9C611


def sphere(material_hash: int = 0):
    return bhk.CollisionShape(shape_type="sphere", name="s", transform=None, verts=[], faces=[],
                              convex_radius=0.0, children=[], sphere_radius=0.5,
                              material_hash=material_hash)


def compound(material_hash: int, children):
    return bhk.CollisionShape(shape_type="compound", name="c", transform=None, verts=[], faces=[],
                              convex_radius=0.0, children=children, material_hash=material_hash)


def simulated_body(layer: int, part: int = 0, mass: float = 1.0):
    return SimpleNamespace(layer=layer, part=part, motion=SimpleNamespace(anchor=False, mass=mass))


def anchor_body():
    return SimpleNamespace(layer=1, part=0, motion=SimpleNamespace(anchor=True, mass=0.0))


class ShapeMaterialTests(unittest.TestCase):
    def entries(self, shape):
        out = []
        worker._shape_to_entries(shape, None, out)
        return [entry["material"] for entry in out]

    def test_a_leaf_carries_its_own_hash(self):
        self.assertEqual(self.entries(sphere(STONE)), [STONE])

    def test_a_leaf_without_a_hash_inherits_the_compound_s(self):
        self.assertEqual(self.entries(compound(WOOD, [sphere(0), sphere(STONE)])), [WOOD, STONE])

    def test_a_shape_without_any_hash_is_material_zero(self):
        self.assertEqual(self.entries(compound(0, [sphere(0)])), [0])


def synthetic_nif(block_types, block_blobs=None) -> bytes:
    """A minimal FO4 NIF: header with the given block-type names, one block per name (zero-filled
    unless a blob is given), no strings."""
    blobs = block_blobs or [bytes(0x80)] * len(block_types)
    out = bytearray(b"Gamebryo File Format, Version 20.2.0.7\n")
    out += struct.pack("<IBIII", 0x14020007, 1, 12, len(block_types), 130)
    for _ in range(4):
        out += b"\x00"
    out += struct.pack("<H", len(block_types))
    for name in block_types:
        out += struct.pack("<I", len(name)) + name.encode()
    out += b"".join(struct.pack("<H", i) for i in range(len(block_types)))
    out += b"".join(struct.pack("<I", len(blob)) for blob in blobs)
    out += struct.pack("<II", 0, 0) + struct.pack("<I", 0)
    return bytes(out + b"".join(blobs))


class CollisionPrefilterTests(unittest.TestCase):
    def test_a_nif_with_a_physics_system_block_may_contain_collision(self):
        nif = synthetic_nif(["NiNode", "bhkNPCollisionObject", "bhkPhysicsSystem"])
        self.assertIn("bhkPhysicsSystem", [b["type"] for b in bhk._parse_nif_blocks(nif)[0]])
        self.assertTrue(worker._may_contain_collision(nif))

    def test_a_nif_without_a_physics_system_block_cannot_yield_collision(self):
        nif = synthetic_nif(["NiNode", "bhkNPCollisionObject", "BSTriShape"])
        self.assertFalse(worker._may_contain_collision(nif))
        self.assertEqual(bhk._resolve_collision_physics_pairs(bhk._parse_nif_blocks(nif)[0]), [])

    def process(self, name: str, data: bytes):
        with tempfile.TemporaryDirectory() as tmp:
            nif = Path(tmp) / name
            nif.write_bytes(data)
            out = Path(tmp) / "out" / "a.col.json"
            result = worker.process_nif(nif, out)
            return result, out.exists()

    def test_a_filtered_nif_writes_no_sidecar_and_is_not_parsed(self):
        worker._take_problem_count()
        with contextlib.redirect_stderr(io.StringIO()):
            result, written = self.process("a.nif", b"not a nif at all")
        self.assertEqual((result, written, worker._take_problem_count()), (None, False, 0))

    def test_an_unreadable_nif_that_names_a_physics_system_is_still_reported(self):
        worker._take_problem_count()
        with contextlib.redirect_stderr(io.StringIO()):
            result, written = self.process("a.nif", b"not a nif but bhkPhysicsSystem")
        self.assertEqual((result, written, worker._take_problem_count()), (None, False, 1))

    def test_a_nif_with_a_physics_system_but_no_collision_object_writes_nothing(self):
        result, written = self.process("a.nif", synthetic_nif(["NiNode", "bhkPhysicsSystem"]))
        self.assertEqual((result, written), (None, False))


def ninode_blob(extra_data_refs: int, collision_object: int, children=()) -> bytes:
    """NiNode payload: name, extra data refs, controller, flags, translation (1, 2, 3), identity rotation,
    scale 1, collision object, children."""
    out = struct.pack("<II", 0xFFFFFFFF, extra_data_refs) + b"".join(struct.pack("<i", 7 + i) for i in range(extra_data_refs))
    out += struct.pack("<iI", -1, 14)
    out += struct.pack("<3f", 1.0, 2.0, 3.0) + struct.pack("<9f", 1, 0, 0, 0, 1, 0, 0, 0, 1) + struct.pack("<f", 1.0)
    return out + struct.pack("<iI", collision_object, len(children)) + b"".join(struct.pack("<i", c) for c in children)


class NiNodeTransformTests(unittest.TestCase):
    def parse_root(self, extra_data_refs: int):
        nif = synthetic_nif(["NiNode", "bhkNPCollisionObject", "bhkPhysicsSystem"],
                            [ninode_blob(extra_data_refs, 1, (2,)), bytes(14), bytes(16)])
        return bhk._parse_nif_blocks(nif)[0]

    def test_the_transform_is_found_whatever_the_number_of_extra_data_refs(self):
        for extra_data_refs in range(0, 9):
            with self.subTest(extra_data_refs=extra_data_refs):
                transform = self.parse_root(extra_data_refs)[0]["transform"]
                self.assertEqual((transform["t"], transform["s"], transform["co"], transform["children"]),
                                 ((1.0, 2.0, 3.0), 1.0, 1, [2]))

    def test_a_node_with_four_extra_data_refs_still_pairs_with_its_physics_system(self):
        blob = bytearray(bytes(16))
        struct.pack_into("<I", blob, 4, 0)
        blocks = bhk._parse_nif_blocks(synthetic_nif(["NiNode", "bhkNPCollisionObject", "bhkPhysicsSystem"],
                                                    [ninode_blob(4, 1), struct.pack("<iHiI", 0, 128, 2, 0), bytes(16)]))[0]
        self.assertEqual(bhk._resolve_collision_physics_pairs(blocks), [(0, 2)])

    def test_an_extra_data_count_larger_than_the_block_is_rejected(self):
        nif = synthetic_nif(["NiNode"], [ninode_blob(0, -1)[:4] + struct.pack("<I", 1000) + bytes(0x40)])
        with self.assertRaisesRegex(RuntimeError, r"block 0 \(NiNode\).*1000 extra data refs"):
            bhk._parse_nif_blocks(nif)

    def test_blocks_that_only_end_in_node_are_not_parsed_as_nodes(self):
        nif = synthetic_nif(["BSFurnitureMarkerNode", "NiNode"], [struct.pack("<I", 0xFFFFFFFF) + struct.pack("<I", 1000) + bytes(0x40), ninode_blob(0, -1)])
        blocks = bhk._parse_nif_blocks(nif)[0]
        self.assertIsNone(blocks[0]["transform"])
        self.assertIsNotNone(blocks[1]["transform"])

    def test_node_subclasses_are_parsed_as_nodes(self):
        for node_type in ("NiBillboardNode", "BSValueNode", "BSLeafAnimNode", "BSDamageStage"):
            with self.subTest(node_type=node_type):
                blocks = bhk._parse_nif_blocks(synthetic_nif([node_type], [ninode_blob(1, -1)]))[0]
                self.assertEqual(blocks[0]["transform"]["t"], (1.0, 2.0, 3.0))


class PlaceEntriesTests(unittest.TestCase):
    ROT_Z_90 = ((0.0, -1.0, 0.0), (1.0, 0.0, 0.0), (0.0, 0.0, 1.0))
    IDENTITY = worker._IDENTITY_ROT

    def entries(self):
        return [
            {"type": "sphere", "center": [1.0, 2.0, 3.0], "radius": 0.5, "material": 7},
            {"type": "capsule", "center": [0.0, 1.0, 0.0], "rotation": [0.0, 0.0, 0.0, 1.0], "radius": 0.25,
             "height": 2.0, "material": 8},
            {"type": "convex", "vertices": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], "material": 9},
            {"type": "trimesh", "vertices": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
             "indices": [[0, 1, 2]], "material": 10},
        ]

    PLACEMENTS = {
        "zero shift": (IDENTITY, 1.0, (0.0, 0.0, 0.0)),
        "translation": (IDENTITY, 1.0, (10.0, 20.0, 30.0)),
        "rotation and scale": (ROT_Z_90, 2.0, (1.0, 0.0, 0.0)),
    }

    def test_the_inputs_are_never_modified_and_no_input_dict_is_returned(self):
        for name, (rot, scale, offset) in self.PLACEMENTS.items():
            with self.subTest(name):
                entries = self.entries()
                snapshot = copy.deepcopy(entries)
                placed = worker._place_entries(entries, rot, scale, offset)
                self.assertEqual(entries, snapshot)
                self.assertEqual(len(placed), len(entries))
                self.assertTrue(all(p is not e for p, e in zip(placed, entries)))

    def test_adding_a_node_name_to_a_placed_entry_leaves_the_node_local_entry_untouched(self):
        entries = self.entries()
        for placed in worker._place_entries(entries, self.IDENTITY, 1.0, (0.0, 0.0, 0.0)):
            placed["node"] = "Owner"
        self.assertTrue(all("node" not in e for e in entries))

    def test_a_zero_shift_keeps_every_entry_unchanged(self):
        entries = self.entries()
        self.assertEqual(worker._place_entries(entries, self.IDENTITY, 1.0, (0.0, 0.0, 0.0)), entries)

    def test_a_translation_moves_centers_and_vertices_and_keeps_sizes(self):
        placed = worker._place_entries(self.entries(), self.IDENTITY, 1.0, (10.0, 20.0, 30.0))
        self.assertEqual(placed[0]["center"], [11.0, 22.0, 33.0])
        self.assertEqual(placed[0]["radius"], 0.5)
        self.assertEqual(placed[1]["center"], [10.0, 21.0, 30.0])
        self.assertEqual(placed[2]["vertices"], [[11.0, 20.0, 30.0], [10.0, 21.0, 30.0]])

    def test_a_rotation_and_scale_move_and_scale_every_shape(self):
        placed = worker._place_entries(self.entries(), self.ROT_Z_90, 2.0, (1.0, 0.0, 0.0))
        self.assertEqual(placed[0]["center"], [-3.0, 2.0, 6.0])
        self.assertEqual(placed[0]["radius"], 1.0)
        self.assertEqual(placed[1]["radius"], 0.5)
        self.assertEqual(placed[1]["height"], 4.0)
        self.assertEqual(placed[2]["vertices"], [[1.0, 2.0, 0.0], [-1.0, 0.0, 0.0]])

    def test_the_key_order_of_a_placed_entry_matches_its_input(self):
        for name, (rot, scale, offset) in self.PLACEMENTS.items():
            with self.subTest(name):
                entries = self.entries()
                placed = worker._place_entries(entries, rot, scale, offset)
                self.assertEqual([list(p) for p in placed], [list(e) for e in entries])


class BodyFilterTests(unittest.TestCase):
    def test_layer_is_the_low_seven_bits_and_part_is_bits_eight_to_twelve(self):
        physics = bhk.BodyPhysics(filter_info=(7 << 8) | 0x80 | 8, motion=None)
        self.assertEqual((physics.layer, physics.part), (8, 7))

    def test_bits_above_the_part_id_do_not_leak_into_it(self):
        physics = bhk.BodyPhysics(filter_info=(0xFF << 8) | 4, motion=None)
        self.assertEqual(physics.part, 0x1F)


class WaterSummaryTests(unittest.TestCase):
    def test_material_counts_are_one_per_leaf_sorted_by_hash(self):
        entries = [{"material": WOOD}, {"material": STONE}, {"material": WOOD}]
        self.assertEqual(worker._material_counts(entries), [[WOOD, 2], [STONE, 1]])

    def test_body_water_carries_layer_part_and_materials(self):
        water = worker._body_water(simulated_body(layer=8, part=7), [{"material": STONE}])
        self.assertEqual(water, {"layer": 8, "part": 7, "materials": [[STONE, 1]]})

    def test_nif_water_pools_every_simulated_body_and_takes_the_heaviest_one_s_filter(self):
        bodies = {3: [simulated_body(4, mass=1.0), simulated_body(10, mass=5.0), anchor_body()]}
        shapes = {(3, 0): [{"material": WOOD}], (3, 1): [{"material": STONE}], (3, 2): [{"material": 7}]}
        self.assertEqual(worker._nif_water(bodies, shapes),
                         {"layer": 10, "part": 0, "materials": [[WOOD, 1], [STONE, 1]]})

    def test_nif_water_of_an_all_anchor_system_is_none(self):
        self.assertIsNone(worker._nif_water({3: [anchor_body()]}, {}))


class ShapeLayerTests(unittest.TestCase):
    BODIES = {3: [simulated_body(4), anchor_body()]}

    def test_the_layer_is_the_one_of_the_body_that_owns_the_shape(self):
        self.assertEqual([worker._shape_layer(self.BODIES, 3, 0), worker._shape_layer(self.BODIES, 3, 1)], [4, 1])

    def test_a_system_without_decoded_filters_has_no_layer(self):
        self.assertIsNone(worker._shape_layer(self.BODIES, 9, 0))

    def test_a_shape_without_a_valid_body_index_has_no_layer(self):
        for index in (None, -1, 2):
            with self.subTest(index=index):
                self.assertIsNone(worker._shape_layer(self.BODIES, 3, index))


class SidecarLayerTests(unittest.TestCase):
    """process_nif with the Havok decoding stubbed: what matters is how each shape's body layer reaches the sidecar."""

    def process(self, shapes, bodies_or_error):
        blocks = [{"id": 0, "type": "NiNode", "name": "Root", "transform": None},
                  {"id": 1, "type": "bhkPhysicsSystem", "blob": bhk.HAVOK_MAGIC + b"\x00"}]
        parse_body_physics = (mock.Mock(side_effect=bodies_or_error) if isinstance(bodies_or_error, Exception)
                              else mock.Mock(return_value=bodies_or_error))
        patches = [mock.patch.object(bhk, "_parse_nif_blocks", return_value=(blocks, None)),
                   mock.patch.object(bhk, "_resolve_collision_physics_pairs", return_value=[(0, 1)]),
                   mock.patch.object(bhk, "resolve_collision_body_owners", return_value={}),
                   mock.patch.object(bhk, "parse_bytes", return_value=shapes),
                   mock.patch.object(bhk, "has_supported_constraints", return_value=False),
                   mock.patch.object(bhk, "parse_body_physics", parse_body_physics)]
        worker._take_problem_count()
        with tempfile.TemporaryDirectory() as tmp, contextlib.ExitStack() as stack, \
                contextlib.redirect_stderr(io.StringIO()) as stderr:
            for patch in patches:
                stack.enter_context(patch)
            nif = Path(tmp) / "a.nif"
            nif.write_bytes(b"bhkPhysicsSystem")
            out = Path(tmp) / "a.col.json"
            worker.process_nif(nif, out)
            payload = json.loads(out.read_text())
        return payload, worker._take_problem_count(), stderr.getvalue()

    @staticmethod
    def shape_of_body(index):
        shape = sphere(STONE)
        shape.body_index = index
        return shape

    def test_every_shape_carries_the_layer_of_its_own_body(self):
        bodies = [simulated_body(4, mass=0.0), simulated_body(49, mass=0.0)]
        payload, problems, _ = self.process([self.shape_of_body(0), self.shape_of_body(1)], bodies)
        self.assertEqual(([s["layer"] for s in payload["shapes"]], problems), ([4, 49], 0))

    def test_a_dynamic_nif_s_shapes_carry_their_layer_too(self):
        shape = self.shape_of_body(0)
        shape.physics = SimpleNamespace(is_dynamic=True, mass=2.0)
        payload, _, _ = self.process([shape], [simulated_body(10, mass=2.0)])
        self.assertEqual((payload["shapes"][0]["layer"], payload["dynamic"]), (10, True))

    def test_undecodable_filters_omit_the_layer_and_are_reported(self):
        payload, problems, stderr = self.process([self.shape_of_body(0)], ValueError("bad filter"))
        self.assertNotIn("layer", payload["shapes"][0])
        self.assertEqual(problems, 2)
        self.assertIn("no decodable collision layer", stderr)

    def test_a_body_without_a_layer_is_reported_once_however_many_shapes_it_owns(self):
        shapes = [self.shape_of_body(0), self.shape_of_body(0), self.shape_of_body(1)]
        _, problems, stderr = self.process(shapes, ValueError("bad filter"))
        self.assertEqual(problems, 3)
        self.assertEqual(stderr.count("no decodable collision layer"), 2)

    def test_a_shape_without_a_body_index_gets_its_own_message_once_per_system(self):
        shapes = [self.shape_of_body(None), self.shape_of_body(None)]
        _, problems, stderr = self.process(shapes, [simulated_body(4)])
        self.assertEqual(problems, 1)
        self.assertEqual(stderr.count("has no body index"), 1)
        self.assertNotIn("no decodable collision layer", stderr)


class ManifestTests(unittest.TestCase):
    WATER = {"layer": 4, "part": 0, "materials": [[STONE, 2]]}

    def test_a_static_row_stays_two_elements(self):
        self.assertEqual(worker._manifest_row(0, None, False, None), [0, None])

    def test_a_ragdoll_row_gets_the_flag(self):
        self.assertEqual(worker._manifest_row(1, 2.0, True, None), [1, 2.0, 1])

    def test_a_water_row_gets_a_zero_ragdoll_flag_then_the_water(self):
        self.assertEqual(worker._manifest_row(1, 2.0, False, self.WATER), [1, 2.0, 0, self.WATER])

    def sidecar(self, **extra):
        return {"shapes": [], **extra}

    def write_sidecar(self, root: Path, name: str, payload: dict):
        path = root / f"{name}.col.json"
        path.write_text(json.dumps(payload, separators=(",", ":")))

    def test_build_manifest_reads_the_top_level_water_of_a_dynamic_sidecar(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.write_sidecar(root, "crate", self.sidecar(dynamic=True, mass=3.5, water=self.WATER))
            self.write_sidecar(root, "wall", self.sidecar())
            worker.build_manifest(root)
            manifest = json.loads((root / worker.MANIFEST_NAME).read_text())
        self.assertEqual(manifest["meshes\\crate.glb"], [1, 3.5, 0, self.WATER])
        self.assertEqual(manifest["meshes\\wall.glb"], [0, None])

    def test_a_ragdoll_body_s_water_is_not_taken_for_the_mesh_s(self):
        bodies = [{"node": "Bone", "water": {"layer": 10, "part": 3, "materials": [[WOOD, 1]]}}]
        constraints = [{"type": "ragdoll"}]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.write_sidecar(root, "doll", self.sidecar(dynamic=True, mass=2.0, version=2,
                                                          bodies=bodies, constraints=constraints))
            worker.build_manifest(root)
            manifest = json.loads((root / worker.MANIFEST_NAME).read_text())
        self.assertEqual(manifest["meshes\\doll.glb"], [1, 2.0, 1])


    def test_a_known_row_is_used_without_reading_its_sidecar_and_the_others_are_read(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "known.col.json").write_text("not a sidecar")
            self.write_sidecar(root, "other", self.sidecar(dynamic=True, mass=1.5, water=self.WATER))
            worker.build_manifest(root, {root / "known.col.json": [1, 9.0]})
            manifest = json.loads((root / worker.MANIFEST_NAME).read_text())
        self.assertEqual(manifest, {"meshes\known.glb": [1, 9.0], "meshes\other.glb": [1, 1.5, 0, self.WATER]})

    def test_the_manifest_lists_sidecars_sorted_by_relative_posix_path_whether_their_rows_are_known_or_read(self):
        names = ["b/z", "Z/a", "a", "a/b", "ä", "a b", "a-b"]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name in names:
                (root / name).parent.mkdir(parents=True, exist_ok=True)
                self.write_sidecar(root, name, self.sidecar())
            expected = sorted(f"{name}.col.json" for name in names)
            for known in ({}, {root / "a/b.col.json": [0, None]}):
                worker.build_manifest(root, known)
                manifest = json.loads((root / worker.MANIFEST_NAME).read_text())
                self.assertEqual(list(manifest), [worker._manifest_key(Path(relative)) for relative in expected])

    def test_an_unreadable_sidecar_is_reported_and_omitted_while_the_others_stay(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.write_sidecar(root, "wall", self.sidecar())
            (root / "broken.col.json").mkdir()
            stderr = io.StringIO()
            with contextlib.redirect_stderr(stderr):
                count = worker.build_manifest(root)
            manifest = json.loads((root / worker.MANIFEST_NAME).read_text())
        self.assertEqual((count, manifest), (1, {"meshes\wall.glb": [0, None]}))
        self.assertIn("broken.col.json", stderr.getvalue())


def write_nif_list(root: Path, relatives=None) -> Path:
    """A --files-from file naming the given paths (relative to root/in), by default every NIF below it."""
    if relatives is None:
        relatives = sorted(str(nif.relative_to(root / "in")) for nif in (root / "in").rglob("*.nif"))
    list_file = root / "nifs.txt"
    list_file.write_bytes("".join(f"{relative}\n" for relative in relatives).encode("utf-8"))
    return list_file


class MainTests(unittest.TestCase):
    def run_main(self, root: Path, *extra: str, relatives=None) -> str:
        list_file = write_nif_list(root, relatives)
        argv = ["fo4_collision_worker.py", "--in-dir", str(root / "in"), "--out-dir", str(root / "out"),
                "--files-from", str(list_file), *extra]
        stdout = io.StringIO()
        with contextlib.redirect_stdout(stdout):
            old_argv, sys.argv = sys.argv, argv
            try:
                self.assertEqual(worker.main(), 0)
            finally:
                sys.argv = old_argv
        return stdout.getvalue()

    def test_a_skipped_existing_sidecar_keeps_its_manifest_row_for_every_worker_count(self):
        for jobs in ("1", "2"):
            with self.subTest(jobs=jobs), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                (root / "in" / "meshes").mkdir(parents=True)
                (root / "out" / "meshes").mkdir(parents=True)
                (root / "in" / "meshes" / "crate.nif").write_bytes(b"x")
                (root / "in" / "meshes" / "plain.nif").write_bytes(b"x")
                (root / "out" / "meshes" / "crate.col.json").write_text(
                    json.dumps({"shapes": [], "dynamic": True, "mass": 2.5, "water": ManifestTests.WATER},
                               separators=(",", ":")))
                summary = self.run_main(root, "--skip-existing", "--jobs", jobs)
                manifest = json.loads((root / "out" / worker.MANIFEST_NAME).read_text())
            self.assertEqual(manifest, {"meshes\\meshes\\crate.glb": [1, 2.5, 0, ManifestTests.WATER]})
            self.assertIn("1 NIFs", summary)
            self.assertIn("1 skipped (existing)", summary)
            self.assertIn("1 NIFs to process (1 existing sidecars skipped)", summary)

    def test_progress_lines_carry_the_total_for_every_worker_count(self):
        for jobs in ("1", "2"):
            with self.subTest(jobs=jobs), tempfile.TemporaryDirectory() as tmp, \
                    mock.patch.object(worker, "PROGRESS_INTERVAL", 1):
                root = Path(tmp)
                (root / "in" / "meshes").mkdir(parents=True)
                (root / "out").mkdir()
                for name in ("a", "b", "c"):
                    (root / "in" / "meshes" / f"{name}.nif").write_bytes(b"x")
                output = self.run_main(root, "--jobs", jobs)
            self.assertIn("3 NIFs to process (0 existing sidecars skipped)", output)
            self.assertIn("... 3/3 NIFs processed", output)

    def test_only_the_listed_nifs_are_processed_whatever_else_sits_below_the_source_root(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "in" / "meshes").mkdir(parents=True)
            (root / "out").mkdir()
            for name in ("listed", "unlisted"):
                (root / "in" / "meshes" / f"{name}.nif").write_bytes(b"x")
            output = self.run_main(root, relatives=["meshes\\listed.nif"])
        self.assertIn("1 NIFs to process (0 existing sidecars skipped)", output)
        self.assertIn("Collision worker: 1 NIFs", output)

    def test_the_limit_applies_to_the_list(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "in").mkdir()
            (root / "out").mkdir()
            for name in ("a", "b", "c"):
                (root / "in" / f"{name}.nif").write_bytes(b"x")
            output = self.run_main(root, "--limit", "2")
        self.assertIn("2 NIFs to process", output)

    def test_non_ascii_paths_in_the_list_are_read_as_utf8(self):
        names = ["Gebäude Süd", "日本語", "café ☃"]
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "in" / "ünï").mkdir(parents=True)
            (root / "out" / "ünï").mkdir(parents=True)
            for name in names:
                (root / "in" / "ünï" / f"{name}.nif").write_bytes(b"x")
                (root / "out" / "ünï" / f"{name}.col.json").write_text(json.dumps({"shapes": []}), encoding="utf-8")
            output = self.run_main(root, "--skip-existing", relatives=[f"ünï\\{name}.nif" for name in names])
            manifest = json.loads((root / "out" / worker.MANIFEST_NAME).read_text(encoding="utf-8"))
        self.assertIn("0 NIFs to process (3 existing sidecars skipped)", output)
        self.assertEqual(sorted(manifest), sorted(f"meshes\\ünï\\{name}.glb".lower() for name in names))

    def run_with_list(self, list_bytes: bytes, create_list: bool = True) -> tuple:
        """(exit code, stderr, whether a manifest was written) of a run over an empty source root."""
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "in").mkdir()
            (root / "out").mkdir()
            list_file = root / "nifs.txt"
            if create_list:
                list_file.write_bytes(list_bytes)
            argv = ["fo4_collision_worker.py", "--in-dir", str(root / "in"), "--out-dir", str(root / "out"), "--files-from", str(list_file)]
            stderr = io.StringIO()
            with mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(stderr):
                code = worker.main()
            return code, stderr.getvalue(), (root / "out" / worker.MANIFEST_NAME).exists()

    def test_a_list_entry_that_leaves_the_source_root_fails_the_run_before_any_work(self):
        for entry in ("..\\outside.nif", "a\\..\\..\\outside.nif", "C:\\abs\\outside.nif", "\\rooted.nif", ""):
            with self.subTest(entry=entry):
                code, stderr, manifest_written = self.run_with_list(f"ok.nif\n{entry}\n".encode("utf-8"))
                self.assertEqual(code, 2)
                self.assertIn("not a path below the source root", stderr)
                self.assertFalse(manifest_written)

    def test_a_missing_or_undecodable_list_fails_the_run(self):
        for label, result in (("undecodable", self.run_with_list(b"\xff\xfe\x00bad")),
                              ("missing", self.run_with_list(b"", create_list=False))):
            with self.subTest(label):
                code, stderr, manifest_written = result
                self.assertEqual(code, 2)
                self.assertIn("cannot read --files-from", stderr)
                self.assertFalse(manifest_written)

    def test_an_empty_list_is_an_empty_run_that_still_writes_the_manifest(self):
        code, _, manifest_written = self.run_with_list(b"")
        self.assertEqual(code, 0)
        self.assertTrue(manifest_written)

    def test_the_files_from_option_is_required(self):
        argv = ["fo4_collision_worker.py", "--in-dir", "in", "--out-dir", "out"]
        with mock.patch.object(sys, "argv", argv), contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit):
                worker.main()

    def test_the_manifest_bytes_do_not_depend_on_whether_the_list_was_prefiltered(self):
        """Listing every staged NIF (what a warm run does) and listing only the candidates that carry the marker give the same manifest,
        stale sidecars and --skip-existing rows included."""
        manifests = {}
        for label, relatives in (("every NIF", None), ("candidates only", ["meshes\\crate.nif", "meshes\\broken.nif"])):
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                (root / "in" / "meshes").mkdir(parents=True)
                (root / "out" / "meshes").mkdir(parents=True)
                (root / "in" / "meshes" / "crate.nif").write_bytes(b"bhkPhysicsSystem?")
                (root / "in" / "meshes" / "broken.nif").write_bytes(b"not a nif but bhkPhysicsSystem")
                (root / "in" / "meshes" / "plain.nif").write_bytes(b"x")
                (root / "in" / "meshes" / "plain_with_old_sidecar.nif").write_bytes(b"x")
                crate = {"shapes": [], "dynamic": True, "mass": 2.5, "water": ManifestTests.WATER}
                for name, payload in (("crate", crate), ("plain_with_old_sidecar", {"shapes": []}),
                                      ("stale_nif_that_is_gone", {"shapes": [], "dynamic": True, "mass": 1.0})):
                    (root / "out" / "meshes" / f"{name}.col.json").write_text(json.dumps(payload, separators=(",", ":")))
                with contextlib.redirect_stderr(io.StringIO()):
                    self.run_main(root, "--skip-existing", "--jobs", "2", relatives=relatives)
                manifests[label] = (root / "out" / worker.MANIFEST_NAME).read_bytes()
        self.assertEqual(manifests["every NIF"], manifests["candidates only"])
        self.assertEqual(sorted(json.loads(manifests["every NIF"])),
                         ["meshes\\meshes\\crate.glb", "meshes\\meshes\\plain_with_old_sidecar.glb", "meshes\\meshes\\stale_nif_that_is_gone.glb"])


class SidecarWriteTests(unittest.TestCase):
    def test_a_written_sidecar_is_complete_and_leaves_no_partial_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "meshes" / "a.col.json"
            worker._write_atomically(out, b'{"shapes":[]}')
            self.assertEqual(out.read_bytes(), b'{"shapes":[]}')
            self.assertEqual([path.name for path in out.parent.iterdir()], ["a.col.json"])

    def test_a_write_that_cannot_be_completed_leaves_the_existing_sidecar_untouched(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "a.col.json"
            out.write_bytes(b"old")
            with mock.patch.object(worker.os, "replace", side_effect=OSError("killed")), self.assertRaises(OSError):
                worker._write_atomically(out, b"new")
            self.assertEqual(out.read_bytes(), b"old")

    def test_a_partial_file_is_never_taken_for_a_sidecar(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "a.col.json"
            with mock.patch.object(worker.os, "replace", side_effect=OSError("killed")), self.assertRaises(OSError):
                worker._write_atomically(out, b'{"shapes"')
            self.assertFalse(out.exists())
            self.assertEqual(list(Path(tmp).rglob("*.col.json")), [])

    def test_half_written_sidecars_of_a_killed_run_are_removed_at_the_next_start(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "in").mkdir()
            (root / "out" / "meshes").mkdir(parents=True)
            kept = root / "out" / "meshes" / "done.col.json"
            kept.write_bytes(b'{"shapes":[]}')
            stale = root / "out" / "meshes" / f"cut.col.json{worker.PARTIAL_SUFFIX}"
            stale.write_bytes(b'{"sha')
            argv = ["fo4_collision_worker.py", "--in-dir", str(root / "in"), "--out-dir", str(root / "out"),
                    "--files-from", str(write_nif_list(root))]
            stdout = io.StringIO()
            with mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(stdout):
                self.assertEqual(worker.main(), 0)
            self.assertFalse(stale.exists())
            self.assertTrue(kept.exists())
            self.assertIn("removed 1 half-written sidecars", stdout.getvalue())


class StreamProgressTimingTests(unittest.TestCase):
    class Clock:
        def __init__(self) -> None:
            self.now = 100.0

        def __call__(self) -> float:
            return self.now

    def progress_lines(self, steps) -> list:
        """The progress lines printed while each (seconds passed, NIFs done) step is applied to a tally with a far-off interval."""
        clock = self.Clock()
        tally = worker.StreamTally(clock)
        stdout = io.StringIO()
        with mock.patch.object(worker, "PROGRESS_INTERVAL", 10_000), contextlib.redirect_stdout(stdout):
            for seconds, done in steps:
                clock.now += seconds
                for _ in range(done):
                    tally.receive()
                tally.add_batch(worker.BatchResult(done, 0, 0, 0, []))
        return [line.strip() for line in stdout.getvalue().splitlines()]

    def test_progress_is_printed_once_a_second_has_passed_since_the_last_line_and_the_count_changed(self):
        lines = self.progress_lines([(0.3, 5), (0.3, 5), (0.5, 5), (0.2, 5), (1.0, 5)])
        self.assertEqual([line.split(" NIFs")[0] for line in lines], ["... 15/15", "... 25/25"])

    def test_no_line_is_printed_while_nothing_changed(self):
        self.assertEqual(self.progress_lines([(5.0, 0), (5.0, 0)]), [])

    def test_the_progress_line_keeps_the_format_the_installer_parses(self):
        self.assertEqual(self.progress_lines([(2.0, 3)]), ["... 3/3 NIFs processed, 0 sidecars written, 0 errors"])


class StdinListTests(unittest.TestCase):
    """--files-from - : the NIF paths arrive on stdin while the worker is already running."""

    def make_root(self, tmp: str, names=()) -> Path:
        root = Path(tmp)
        (root / "in" / "meshes").mkdir(parents=True)
        (root / "out" / "meshes").mkdir(parents=True)
        for name in names:
            (root / "in" / "meshes" / f"{name}.nif").write_bytes(b"x")
        return root

    @staticmethod
    def argv(root: Path, *extra: str) -> list:
        return ["fo4_collision_worker.py", "--in-dir", str(root / "in"), "--out-dir", str(root / "out"), "--files-from", "-", *extra]

    @staticmethod
    def list_bytes(relatives) -> bytes:
        return "".join(f"{relative}\n" for relative in relatives).encode("utf-8")

    def run_with_stdin(self, root: Path, stdin_bytes: bytes, *extra: str) -> tuple:
        """(exit code, stdout, stderr) of main() reading stdin_bytes."""
        stdin = SimpleNamespace(buffer=io.BytesIO(stdin_bytes))
        stdout, stderr = io.StringIO(), io.StringIO()
        with mock.patch.object(sys, "argv", self.argv(root, *extra)), mock.patch.object(sys, "stdin", stdin), \
                contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            code = worker.main()
        return code, stdout.getvalue(), stderr.getvalue()

    @staticmethod
    def fake_process_nif(nif_path: Path, out_path: Path):
        """Stands in for the decoder: every NIF whose name starts with "solid" yields a sidecar."""
        if not nif_path.name.startswith("solid"):
            return None
        out_path.parent.mkdir(parents=True, exist_ok=True)
        content = json.dumps({"shapes": [], "dynamic": True, "mass": 2.5}, separators=(",", ":"))
        out_path.write_text(content)
        return worker.SidecarResult(1, worker._sidecar_manifest_row(content.encode("utf-8")))

    def test_a_streamed_list_gives_the_output_and_manifest_of_the_same_list_file(self):
        names = ["solid_b", "plain", "solid_a", "solid_old"]
        relatives = [f"meshes\\{name}.nif" for name in names]
        outputs = {}
        for label in ("file", "stream"):
            with tempfile.TemporaryDirectory() as tmp, mock.patch.object(worker, "process_nif", self.fake_process_nif):
                root = self.make_root(tmp, names)
                (root / "out" / "meshes" / "solid_old.col.json").write_text(json.dumps({"shapes": []}))
                if label == "file":
                    argv = ["fo4_collision_worker.py", "--in-dir", str(root / "in"), "--out-dir", str(root / "out"),
                            "--files-from", str(write_nif_list(root, relatives)), "--skip-existing"]
                    with mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()):
                        self.assertEqual(worker.main(), 0)
                else:
                    code, _, _ = self.run_with_stdin(root, self.list_bytes(relatives), "--skip-existing")
                    self.assertEqual(code, 0)
                outputs[label] = {path.relative_to(root / "out").as_posix(): path.read_bytes() for path in (root / "out").rglob("*.json")}
        self.assertEqual(outputs["file"], outputs["stream"])
        self.assertEqual(sorted(json.loads(outputs["stream"][worker.MANIFEST_NAME])),
                         ["meshes\\meshes\\solid_a.glb", "meshes\\meshes\\solid_b.glb", "meshes\\meshes\\solid_old.glb"])

    def test_every_streamed_path_is_decoded_exactly_once_across_batches_for_every_worker_count(self):
        names = [f"n{index:03}" for index in range(50)]
        for jobs in ("1", "2"):
            with self.subTest(jobs=jobs), tempfile.TemporaryDirectory() as tmp, mock.patch.object(worker, "BATCH_SIZE", 7):
                root = self.make_root(tmp, names)
                code, stdout, _ = self.run_with_stdin(root, self.list_bytes(f"meshes\\{name}.nif" for name in names), "--jobs", jobs)
            self.assertEqual(code, 0)
            self.assertIn("Collision worker: 50 NIFs, 0 sidecars written, 0 skipped (existing), 0 errors", stdout)

    def test_progress_counts_skipped_sidecars_as_done_against_the_paths_received(self):
        for jobs in ("1", "2"):
            with self.subTest(jobs=jobs), tempfile.TemporaryDirectory() as tmp, mock.patch.object(worker, "PROGRESS_INTERVAL", 1):
                root = self.make_root(tmp, ["a", "b", "c"])
                (root / "out" / "meshes" / "b.col.json").write_text(json.dumps({"shapes": []}))
                code, stdout, _ = self.run_with_stdin(root, self.list_bytes(f"meshes\\{name}.nif" for name in "abc"),
                                                      "--skip-existing", "--jobs", jobs)
            self.assertEqual(code, 0)
            self.assertIn("... 3/3 NIFs processed", stdout)
            self.assertIn("Collision worker: 2 NIFs, 0 sidecars written, 1 skipped (existing)", stdout)

    def test_an_output_folder_that_does_not_exist_yet_is_created(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.make_root(tmp)
            (root / "out" / "meshes").rmdir()
            (root / "out").rmdir()
            code, _, _ = self.run_with_stdin(root, b"")
            self.assertEqual(code, 0)
            self.assertTrue((root / "out" / worker.MANIFEST_NAME).exists())

    def test_a_source_folder_that_does_not_exist_fails_the_run(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.make_root(tmp)
            (root / "in" / "meshes").rmdir()
            (root / "in").rmdir()
            code, _, stderr = self.run_with_stdin(root, b"")
            self.assertEqual(code, 2)
            self.assertIn("--in-dir not a directory", stderr)

    def test_a_streamed_list_may_be_empty_and_still_writes_the_manifest(self):
        for jobs in ("1", "2"):
            with self.subTest(jobs=jobs), tempfile.TemporaryDirectory() as tmp:
                root = self.make_root(tmp)
                code, _, _ = self.run_with_stdin(root, b"", "--jobs", jobs)
                self.assertEqual(code, 0)
                self.assertTrue((root / "out" / worker.MANIFEST_NAME).exists())

    def test_non_ascii_paths_are_read_as_utf8(self):
        names = ["Gebäude Süd", "日本語"]
        with tempfile.TemporaryDirectory() as tmp:
            root = self.make_root(tmp)
            for name in names:
                (root / "in" / "meshes" / f"{name}.nif").write_bytes(b"x")
                (root / "out" / "meshes" / f"{name}.col.json").write_text(json.dumps({"shapes": []}), encoding="utf-8")
            code, stdout, _ = self.run_with_stdin(root, self.list_bytes(f"meshes\\{name}.nif" for name in names), "--skip-existing")
        self.assertEqual(code, 0)
        self.assertIn("0 NIFs, 0 sidecars written, 2 skipped (existing)", stdout)

    def test_a_bad_line_fails_the_run_without_a_manifest_for_every_worker_count(self):
        cases = {
            "empty line": b"meshes\\ok.nif\n\n",
            "parent escape": b"meshes\\ok.nif\n..\\outside.nif\n",
            "absolute path": b"meshes\\ok.nif\nC:\\abs\\outside.nif\n",
            "cut-off last line": b"meshes\\ok.nif\nmeshes\\cut",
            "not utf-8": b"meshes\\ok.nif\n\xff\xfe\n",
        }
        for label, content in cases.items():
            for jobs in ("1", "2"):
                with self.subTest(case=label, jobs=jobs), tempfile.TemporaryDirectory() as tmp:
                    root = self.make_root(tmp, ["ok"])
                    code, _, stderr = self.run_with_stdin(root, content, "--jobs", jobs)
                    self.assertEqual(code, 2)
                    self.assertIn("cannot read the NIF list from stdin", stderr)
                    self.assertFalse((root / "out" / worker.MANIFEST_NAME).exists())

    def test_the_limit_cannot_be_combined_with_a_streamed_list(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.make_root(tmp)
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as raised:
                self.run_with_stdin(root, b"", "--limit", "5")
        self.assertEqual(raised.exception.code, 2)

    def test_paths_are_decoded_while_stdin_is_still_open(self):
        """A partial batch is dispatched after a short idle time, so the extraction's NIFs do not wait for the next full batch or the end."""
        read_fd, write_fd = os.pipe()
        stdout = io.StringIO()
        outcome = {}
        with tempfile.TemporaryDirectory() as tmp, mock.patch.object(worker, "PROGRESS_INTERVAL", 1), \
                os.fdopen(read_fd, "rb") as reader, mock.patch.object(sys, "stdin", SimpleNamespace(buffer=reader)):
            root = self.make_root(tmp, ["early", "late"])
            with mock.patch.object(sys, "argv", self.argv(root, "--jobs", "2")), contextlib.redirect_stdout(stdout):
                runner = threading.Thread(target=lambda: outcome.setdefault("code", worker.main()))
                runner.start()
                try:
                    os.write(write_fd, self.list_bytes(["meshes\\early.nif"]))
                    deadline = time.monotonic() + 30
                    while "... 1/1 NIFs processed" not in stdout.getvalue() and time.monotonic() < deadline:
                        time.sleep(0.05)
                    decoded_before_close = "... 1/1 NIFs processed" in stdout.getvalue()
                    os.write(write_fd, self.list_bytes(["meshes\\late.nif"]))
                finally:
                    os.close(write_fd)
                    runner.join(timeout=60)
        self.assertTrue(decoded_before_close)
        self.assertEqual(outcome.get("code"), 0)
        self.assertIn("Collision worker: 2 NIFs", stdout.getvalue())

    def test_a_real_worker_process_streams_from_its_stdin_and_ends_when_stdin_closes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.make_root(tmp, ["a", "b", "c"])
            command = [sys.executable, "-u", str(REPO_ROOT / "src" / "installer" / "python" / "fo4_collision_worker.py"),
                       "--in-dir", str(root / "in"), "--out-dir", str(root / "out"), "--files-from", "-", "--jobs", "2"]
            result = subprocess.run(command, input=self.list_bytes(f"meshes\\{name}.nif" for name in "abc"),
                                    capture_output=True, timeout=120)
            manifest_written = (root / "out" / worker.MANIFEST_NAME).exists()
        self.assertEqual(result.returncode, 0, result.stderr.decode("utf-8", "replace"))
        self.assertIn("Collision worker: 3 NIFs", result.stdout.decode("utf-8"))
        self.assertTrue(manifest_written)


class PriorityTests(unittest.TestCase):
    BELOW_NORMAL = 0x4000

    PROBE = "\n".join([
        "import ctypes, fo4_collision_worker as w",
        "w.lower_own_priority()",
        "k = ctypes.WinDLL('kernel32')",
        "k.GetCurrentProcess.restype = ctypes.c_void_p",
        "k.GetPriorityClass.argtypes = [ctypes.c_void_p]",
        "print(k.GetPriorityClass(k.GetCurrentProcess()))",
    ])

    @unittest.skipUnless(sys.platform == "win32", "priority classes exist on Windows only")
    def test_the_worker_lowers_its_own_priority_class_to_below_normal(self):
        env = {**os.environ, "PYTHONPATH": str(REPO_ROOT / "src" / "installer" / "python")}
        result = subprocess.run([sys.executable, "-c", self.PROBE], env=env, capture_output=True, text=True, check=True)
        self.assertEqual(int(result.stdout.strip()), self.BELOW_NORMAL)

    def run_main_without_work(self, *extra: str) -> str:
        stderr = io.StringIO()
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "in").mkdir()
            (root / "out").mkdir()
            argv = ["fo4_collision_worker.py", "--in-dir", str(root / "in"), "--out-dir", str(root / "out"),
                    "--files-from", str(write_nif_list(root)), *extra]
            with mock.patch.object(sys, "argv", argv), contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(stderr):
                self.assertEqual(worker.main(), 0)
        return stderr.getvalue()

    def test_main_lowers_the_priority_before_any_pool_is_created(self):
        calls = []
        with mock.patch.object(worker, "lower_own_priority", side_effect=lambda: calls.append("lower")):
            with mock.patch("multiprocessing.Pool", side_effect=lambda *a, **k: calls.append("pool") or mock.MagicMock()):
                self.run_main_without_work("--jobs", "2")
        self.assertEqual(calls, ["lower", "pool"])

    def test_a_failure_to_lower_the_priority_is_reported_on_stderr_and_the_run_continues(self):
        with mock.patch.object(worker, "lower_own_priority", side_effect=OSError("access denied")):
            stderr = self.run_main_without_work()
        self.assertIn("could not lower the priority", stderr)
        self.assertIn("access denied", stderr)


if __name__ == "__main__":
    unittest.main()
