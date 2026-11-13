"""The Havok convex radius of a convex polytope: read from a synthetic packfile, written to the sidecar entry in
output units, and scaled with the node it is placed under."""

import math
import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "src" / "installer" / "python"))
sys.path.insert(0, str(Path(__file__).resolve().parent))
import fo4_collision_worker as worker  # noqa: E402
from synthetic_packfile import build_polytope_packfile  # noqa: E402

bhk = worker.bhk

TETRAHEDRON_VERTICES = [(0.0, 0.0, 0.0), (1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)]
TETRAHEDRON_FACES = [(0, 2, 1), (0, 1, 3), (0, 3, 2), (1, 2, 3)]
STONE = 0xDF02F237


def polytope(radius: float, name: str = "p"):
    return bhk.CollisionShape(shape_type="polytope", name=name, transform=None, verts=list(TETRAHEDRON_VERTICES),
                              faces=[], convex_radius=radius, children=[])


def compound(*children, convex_radius: float = 0.0):
    return bhk.CollisionShape(shape_type="compound", name="c", transform=None, verts=[], faces=[],
                              convex_radius=convex_radius, children=list(children))


def entries_of(shape):
    out = []
    worker._shape_to_entries(shape, None, out)
    return out


class PolytopePackfileTests(unittest.TestCase):
    def decode(self, radius, user_data=0):
        blob = build_polytope_packfile(TETRAHEDRON_VERTICES, TETRAHEDRON_FACES, radius, user_data)
        shapes = bhk.parse_bytes(blob)
        self.assertEqual([shape.shape_type for shape in shapes], ["polytope"])
        return shapes[0]

    def test_the_radius_is_read_from_the_shape_at_offset_0x14(self):
        self.assertAlmostEqual(self.decode(0.0625).convex_radius, 0.0625)

    def test_a_zero_radius_is_read_as_zero(self):
        self.assertEqual(self.decode(0.0).convex_radius, 0.0)

    def test_the_hull_and_the_material_are_read_next_to_the_radius(self):
        shape = self.decode(0.07, user_data=STONE)
        self.assertEqual(shape.verts, TETRAHEDRON_VERTICES)
        self.assertEqual(len(shape.faces), 4)
        self.assertEqual(shape.material_hash, STONE)

    def test_the_decoded_radius_reaches_the_sidecar_entry_in_output_units(self):
        (entry,) = entries_of(self.decode(0.0625))
        self.assertEqual(entry["convex_radius"], 0.0625 * worker.S)


class ConvexRadiusEntryTests(unittest.TestCase):
    def test_a_convex_entry_carries_the_radius_in_the_units_of_its_vertices(self):
        (entry,) = entries_of(polytope(0.07))
        self.assertEqual(entry["convex_radius"], 0.07 * worker.S)
        self.assertEqual(entry["vertices"][1], [1.0 * worker.S, 0.0, 0.0])

    def test_a_shape_without_a_radius_writes_zero_rather_than_omitting_the_key(self):
        (entry,) = entries_of(polytope(0.0))
        self.assertEqual(entry["convex_radius"], 0.0)

    def test_the_radius_sits_between_the_vertices_and_the_material(self):
        (entry,) = entries_of(polytope(0.07))
        self.assertEqual(list(entry), ["type", "vertices", "convex_radius", "material"])

    def test_compound_leaves_keep_their_own_radius(self):
        entries = entries_of(compound(polytope(0.02, "small"), polytope(0.35, "large")))
        self.assertEqual([entry["convex_radius"] for entry in entries], [0.02 * worker.S, 0.35 * worker.S])

    def test_a_compound_s_own_radius_field_is_not_written_onto_its_leaves(self):
        entries = entries_of(compound(polytope(0.07), convex_radius=0.9))
        self.assertEqual([entry["convex_radius"] for entry in entries], [0.07 * worker.S])

    def test_a_leaf_transform_moves_the_hull_but_not_the_radius(self):
        shape = polytope(0.07)
        shape.transform = bhk.BodyTransform(position=(1.0, 2.0, 3.0), rotation=worker._IDENTITY_ROT)
        (entry,) = entries_of(shape)
        self.assertEqual(entry["convex_radius"], 0.07 * worker.S)
        self.assertEqual(entry["vertices"][0], [1.0 * worker.S, 3.0 * worker.S, -2.0 * worker.S])

    def test_a_radius_that_cannot_be_trusted_fails_the_shape_naming_it(self):
        for radius in (-0.01, math.nan, math.inf, -math.inf, worker._MAX_CONVEX_RADIUS * 1.01):
            with self.subTest(radius=radius):
                with self.assertRaisesRegex(ValueError, "convex radius .* of p is not within"):
                    entries_of(polytope(radius))

    def test_the_largest_accepted_radius_is_written(self):
        (entry,) = entries_of(polytope(worker._MAX_CONVEX_RADIUS))
        self.assertEqual(entry["convex_radius"], worker._MAX_CONVEX_RADIUS * worker.S)

    def test_one_bad_leaf_fails_the_whole_top_level_shape(self):
        with self.assertRaises(ValueError):
            entries_of(compound(polytope(0.07), polytope(math.nan)))

    def test_a_sphere_and_a_capsule_carry_their_radius_in_the_shape_not_a_convex_radius(self):
        sphere = bhk.CollisionShape(shape_type="sphere", name="s", transform=None, verts=[], faces=[],
                                    convex_radius=0.5, children=[], sphere_radius=0.5)
        capsule = bhk.CollisionShape(shape_type="capsule", name="k", transform=None, verts=[(0.0, 0.0, 0.0), (0.0, 1.0, 0.0)],
                                     faces=[], convex_radius=0.5, children=[], sphere_radius=0.5)
        for entry in entries_of(sphere) + entries_of(capsule):
            self.assertNotIn("convex_radius", entry)

    def test_an_empty_polytope_still_emits_nothing(self):
        shape = polytope(0.07)
        shape.verts = []
        self.assertEqual(entries_of(shape), [])


class ConvexRadiusPlacementTests(unittest.TestCase):
    IDENTITY = worker._IDENTITY_ROT
    ROT_Z_90 = ((0.0, -1.0, 0.0), (1.0, 0.0, 0.0), (0.0, 0.0, 1.0))
    ORIGIN = (0.0, 0.0, 0.0)

    @staticmethod
    def convex():
        return [{"type": "convex", "vertices": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], "convex_radius": 0.5, "material": 9}]

    def test_a_pure_shift_leaves_the_radius_alone(self):
        (placed,) = worker._place_entries(self.convex(), self.IDENTITY, 1.0, (10.0, 0.0, 0.0))
        self.assertEqual(placed["convex_radius"], 0.5)

    def test_a_rotation_alone_leaves_the_radius_alone(self):
        (placed,) = worker._place_entries(self.convex(), self.ROT_Z_90, 1.0, self.ORIGIN)
        self.assertEqual(placed["convex_radius"], 0.5)

    def test_the_node_scale_scales_the_radius_like_the_vertices(self):
        (placed,) = worker._place_entries(self.convex(), self.IDENTITY, 2.0, self.ORIGIN)
        self.assertEqual(placed["convex_radius"], 1.0)
        self.assertEqual(placed["vertices"], [[2.0, 0.0, 0.0], [0.0, 2.0, 0.0]])

    def test_scale_and_rotation_together_scale_the_radius_once(self):
        (placed,) = worker._place_entries(self.convex(), self.ROT_Z_90, 3.0, (1.0, 0.0, 0.0))
        self.assertEqual(placed["convex_radius"], 1.5)

    def test_the_placed_entry_keeps_the_key_order_of_its_input(self):
        (placed,) = worker._place_entries(self.convex(), self.ROT_Z_90, 3.0, (1.0, 0.0, 0.0))
        self.assertEqual(list(placed), ["type", "vertices", "convex_radius", "material"])

    def test_an_entry_without_the_key_stays_without_it(self):
        old = [{"type": "convex", "vertices": [[1.0, 0.0, 0.0]], "material": 9}]
        (placed,) = worker._place_entries(old, self.IDENTITY, 2.0, self.ORIGIN)
        self.assertNotIn("convex_radius", placed)

    def test_a_trimesh_gains_no_radius(self):
        mesh = [{"type": "trimesh", "vertices": [[1.0, 0.0, 0.0]], "indices": [[0, 0, 0]], "material": 1}]
        (placed,) = worker._place_entries(mesh, self.IDENTITY, 2.0, self.ORIGIN)
        self.assertNotIn("convex_radius", placed)


if __name__ == "__main__":
    unittest.main()
