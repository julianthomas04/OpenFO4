"""Compressed-mesh materials: the hknpBSMaterialProperties table, the per-quad primitive runs and the
one-trimesh-per-material split of the collision worker, against synthetic packfiles."""

import contextlib
import io
import sys
import types
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "src" / "installer" / "python"))
sys.path.insert(0, str(Path(__file__).resolve().parent))
import fo4_collision_worker as worker  # noqa: E402
from synthetic_packfile import NO_USER_DATA, build_compressed_mesh_packfile, pack_run  # noqa: E402

bhk = worker.bhk

STONE = 0xDF02F237
WOOD = 0x1DD9C611
METAL = 0x064003D4

# Two quads: (0,1,2,3) is two triangles, (4,5,6,6) is one.
VERTICES = [(0, 0, 0), (10, 0, 0), (10, 10, 0), (0, 10, 0), (20, 0, 0), (30, 0, 0), (30, 10, 0)]
QUADS = [(0, 1, 2, 3), (4, 5, 6, 6)]


def decode(material_hashes, runs, **kwargs):
    blob = build_compressed_mesh_packfile(material_hashes, QUADS, runs, VERTICES, **kwargs)
    shapes = bhk.extract_bhk_physics_system(raw_data=blob)
    assert len(shapes) == 1 and shapes[0].shape_type == "compressed_mesh"
    return shapes[0]


class PrimitiveRunTests(unittest.TestCase):
    def test_runs_expand_to_one_material_index_per_quad(self):
        runs = [pack_run(2, 0, 3), pack_run(0, 3, 1), pack_run(1, 4, 2)]
        self.assertEqual(bhk.expand_primitive_runs(runs, 6), [2, 2, 2, 0, 1, 1])

    def test_a_run_may_carry_a_material_index_above_255(self):
        self.assertEqual(bhk.expand_primitive_runs([pack_run(0x1FF, 0, 1)], 1), [0x1FF])

    def test_runs_that_leave_a_gap_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "starts at quad 2, expected 1"):
            bhk.expand_primitive_runs([pack_run(0, 0, 1), pack_run(0, 2, 1)], 3)

    def test_runs_that_do_not_cover_the_section_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "cover 2 quads, the section has 3"):
            bhk.expand_primitive_runs([pack_run(0, 0, 2)], 3)

    def test_a_section_without_runs_but_with_quads_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "cover 0 quads"):
            bhk.expand_primitive_runs([], 1)


class SectionQuadMaterialTests(unittest.TestCase):
    SECTION = {"first_run": 1, "num_runs": 2, "num_quads": 3}

    @staticmethod
    def run_array(*runs):
        return b"".join(run.to_bytes(4, "little") for run in runs)

    def test_each_quad_gets_the_hash_of_its_material_index(self):
        data = self.run_array(pack_run(0, 0, 9), pack_run(1, 0, 2), pack_run(0, 2, 1))
        self.assertEqual(bhk.section_quad_materials(data, 0, 3, self.SECTION, [STONE, WOOD]), [WOOD, WOOD, STONE])

    def test_a_shape_without_a_material_table_has_no_quad_materials(self):
        self.assertIsNone(bhk.section_quad_materials(b"", None, 0, self.SECTION, None))

    def test_a_material_index_outside_the_table_is_rejected(self):
        data = self.run_array(pack_run(0, 0, 9), pack_run(2, 0, 2), pack_run(0, 2, 1))
        with self.assertRaisesRegex(ValueError, "material index 2 outside the 2 material entries"):
            bhk.section_quad_materials(data, 0, 3, self.SECTION, [STONE, WOOD])

    def test_runs_beyond_the_run_array_are_rejected(self):
        with self.assertRaisesRegex(ValueError, "outside the run array"):
            bhk.section_quad_materials(b"", 0, 2, self.SECTION, [STONE])


class CompressedMeshDecodingTests(unittest.TestCase):
    def test_every_triangle_carries_the_material_of_its_quad(self):
        shape = decode([STONE, WOOD], [pack_run(1, 0, 1), pack_run(0, 1, 1)])
        self.assertEqual(shape.face_materials, [WOOD, WOOD, STONE])
        self.assertEqual(len(shape.faces), 3)

    def test_the_shape_user_data_constant_is_ignored(self):
        shape = decode([STONE], [pack_run(0, 0, 2)], user_data=NO_USER_DATA)
        self.assertEqual(shape.material_hash, 0)
        self.assertEqual(shape.face_materials, [STONE] * 3)

    def test_a_mesh_without_material_properties_has_no_face_materials(self):
        shape = decode([], [pack_run(0, 0, 2)], with_materials=False)
        self.assertIsNone(shape.face_materials)

    def test_an_entry_with_hash_zero_stays_material_zero(self):
        shape = decode([0], [pack_run(0, 0, 2)])
        self.assertEqual(shape.face_materials, [0] * 3)

    def test_a_run_past_the_material_table_fails_the_decode(self):
        with self.assertRaisesRegex(ValueError, "material index 1 outside"):
            decode([STONE], [pack_run(0, 0, 1), pack_run(1, 1, 1)])


class TrimeshSplitTests(unittest.TestCase):
    def entries(self, shape, inherited=0):
        return worker._trimesh_entries(shape, None, inherited)

    def test_a_multi_material_mesh_becomes_one_trimesh_per_material_ordered_by_hash(self):
        entries = self.entries(decode([STONE, WOOD], [pack_run(1, 0, 1), pack_run(0, 1, 1)]))
        self.assertEqual([entry["material"] for entry in entries], [WOOD, STONE])
        self.assertEqual([len(entry["indices"]) for entry in entries], [2, 1])

    def test_each_entry_keeps_only_the_vertices_its_triangles_use(self):
        wood, stone = self.entries(decode([STONE, WOOD], [pack_run(1, 0, 1), pack_run(0, 1, 1)]))
        self.assertEqual(len(wood["vertices"]), 4)
        self.assertEqual(len(stone["vertices"]), 3)
        for entry in (wood, stone):
            self.assertTrue(all(0 <= index < len(entry["vertices"]) for tri in entry["indices"] for index in tri))

    def test_entries_are_baked_to_output_space(self):
        _, stone = self.entries(decode([STONE, WOOD], [pack_run(1, 0, 1), pack_run(0, 1, 1)]))
        self.assertEqual(stone["vertices"][:2], [[20.0 * worker.S, 0.0, 0.0], [30.0 * worker.S, 0.0, 0.0]])

    def test_two_table_entries_with_one_hash_share_one_trimesh(self):
        entries = self.entries(decode([WOOD, WOOD], [pack_run(0, 0, 1), pack_run(1, 1, 1)]))
        self.assertEqual([(entry["material"], len(entry["indices"])) for entry in entries], [(WOOD, 3)])

    def test_a_mesh_without_a_material_table_is_one_trimesh_with_the_inherited_material(self):
        entries = self.entries(decode([], [pack_run(0, 0, 2)], with_materials=False), inherited=METAL)
        self.assertEqual([(entry["material"], len(entry["indices"])) for entry in entries], [(METAL, 3)])

    def test_the_triangles_of_all_entries_are_the_triangles_of_the_mesh(self):
        shape = decode([STONE, WOOD], [pack_run(1, 0, 1), pack_run(0, 1, 1)])
        baked = worker._bake_verts(None, shape.verts)
        expected = sorted(sorted(tuple(baked[i]) for i in face) for face in shape.faces)
        placed = sorted(sorted(tuple(entry["vertices"][i]) for i in tri)
                        for entry in self.entries(shape) for tri in entry["indices"])
        self.assertEqual(placed, expected)


class DegenerateTriangleTests(unittest.TestCase):
    VERTICES = [(0, 0, 0), (1, 0, 0), (0, 1, 0), (5, 5, 5), (float("nan"), 0, 0)]
    GOOD, HUSK = WOOD, STONE

    def entries(self, faces, materials):
        shape = types.SimpleNamespace(verts=self.VERTICES, faces=faces, face_materials=materials)
        return worker._trimesh_entries(shape, None, 0)

    def test_a_material_whose_triangles_are_all_degenerate_emits_no_entry_and_is_reported(self):
        faces = [(0, 1, 2), (3, 3, 3), (0, 0, 1), (0, 1, 4)]
        with contextlib.redirect_stderr(io.StringIO()) as stderr:
            entries = self.entries(faces, [self.GOOD, self.HUSK, self.HUSK, self.HUSK])
        self.assertEqual([entry["material"] for entry in entries], [self.GOOD])
        self.assertIn(f"{self.HUSK:#010x}", stderr.getvalue())
        self.assertNotIn(f"{self.GOOD:#010x}", stderr.getvalue())

    def test_a_degenerate_triangle_is_dropped_from_a_material_that_has_valid_ones(self):
        entries = self.entries([(0, 1, 2), (0, 1, 1), (0, 1, 3)], [self.GOOD] * 3)
        self.assertEqual(len(entries), 1)
        self.assertEqual(len(entries[0]["indices"]), 2)

    def test_a_mesh_of_only_degenerate_triangles_emits_nothing(self):
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(self.entries([(3, 3, 3)], None), [])

    def test_a_collinear_triangle_has_zero_area_and_is_dropped(self):
        shape = types.SimpleNamespace(verts=[(0, 0, 0), (1, 0, 0), (2, 0, 0)], faces=[(0, 1, 2)], face_materials=None)
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(worker._trimesh_entries(shape, None, 0), [])


if __name__ == "__main__":
    unittest.main()
