"""Compressed-mesh materials of real meshes, read straight from the Fallout 4 install named by OPENFO4_FO4_PATH
(the archives are read, nothing is converted or written to the repository). Skipped when the variable is unset."""

import json
import os
import struct
import sys
import tempfile
import unittest
import zlib
from pathlib import Path
from typing import Dict, Optional

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "src" / "installer" / "python"))
import fo4_collision_worker as worker  # noqa: E402

MESH_ARCHIVES = ("Fallout4 - Meshes.ba2", "Fallout4 - MeshesExtra.ba2")
BA2_HEADER = struct.Struct("<4sI4sIQ")
BA2_GENERAL_ENTRY = struct.Struct("<IIIIQIII")

WOOD = 0x1DD9C611
STONE = 0xDF02F237
METAL = 0x064003D4
DIRT = 0xB9233EAA
DIRT_MUD = 0x58987081
DIRT_STAIRS = 0x4FE3937B
METAL_LIGHT = 0xE538F7DB
WOOD_HEAVY = 0xCF81E009


def read_general_ba2_entries(archive: Path, wanted: set) -> Dict[str, bytes]:
    found: Dict[str, bytes] = {}
    with archive.open("rb") as stream:
        _, _, kind, count, names_offset = BA2_HEADER.unpack(stream.read(BA2_HEADER.size))
        if kind != b"GNRL":
            return found
        entries = [BA2_GENERAL_ENTRY.unpack(stream.read(BA2_GENERAL_ENTRY.size)) for _ in range(count)]
        stream.seek(names_offset)
        names = []
        for _ in range(count):
            (length,) = struct.unpack("<H", stream.read(2))
            names.append(stream.read(length).decode().lower().replace("\\", "/"))
        for name, entry in zip(names, entries):
            if name in wanted:
                stream.seek(entry[4])
                packed, unpacked = entry[5], entry[6]
                raw = stream.read(packed or unpacked)
                found[name] = zlib.decompress(raw) if packed else raw
    return found


@unittest.skipUnless(os.environ.get("OPENFO4_FO4_PATH"), "OPENFO4_FO4_PATH names no Fallout 4 install.")
class RealMeshMaterialTests(unittest.TestCase):
    PROOF_MESHES = {
        "single material static (fence post)": ("meshes/setdressing/fencesgrates/wroughtiron/fencewipostlarge01.nif", {STONE}),
        "single material wall kit": ("meshes/interiors/building/woodb/med_wallkit/bldwoodbmdwallbotalcove02.nif", {WOOD}),
        "precombined object collision": ("meshes/precombined/000503c6_b18d3078_oc.nif", {METAL}),
        "precombined physics (nine entries)": ("meshes/precombined/0000e6ae_physics.nif",
                                                {WOOD, STONE, METAL, DIRT, DIRT_MUD, DIRT_STAIRS, METAL_LIGHT, WOOD_HEAVY}),
        "static collection": ("meshes/scol/fallout4.esm/cm00030fbe.nif", {DIRT_STAIRS, STONE, DIRT}),
    }

    @classmethod
    def setUpClass(cls):
        data_dir = Path(os.environ["OPENFO4_FO4_PATH"]) / "Data"
        wanted = {name for name, _ in cls.PROOF_MESHES.values()}
        cls.nifs: Dict[str, bytes] = {}
        for archive in MESH_ARCHIVES:
            cls.nifs.update(read_general_ba2_entries(data_dir / archive, wanted - set(cls.nifs)))

    def trimesh_materials(self, name: str) -> Optional[set]:
        with tempfile.TemporaryDirectory() as folder:
            nif = Path(folder) / "mesh.nif"
            sidecar = Path(folder) / "mesh.col.json"
            nif.write_bytes(self.nifs[name])
            worker.process_nif(nif, sidecar)
            shapes = json.loads(sidecar.read_text())["shapes"]
        return {shape["material"] for shape in shapes if shape["type"] == "trimesh"}

    def test_every_proof_mesh_resolves_to_the_materials_of_its_entry_table(self):
        for label, (name, expected) in self.PROOF_MESHES.items():
            with self.subTest(label):
                self.assertIn(name, self.nifs)
                self.assertEqual(self.trimesh_materials(name), expected)


NAVCUT_LAYER = 49  # bhkRigidBody layer L_NAVCUT, https://ck.uesp.net/wiki/NAVCUT (not in third_party/pynifly)
JUNK_WALL_DOOR = "meshes/setdressing/junkwalls/junkwalldoor01.nif"
NAVCUT_NODES = ("c_navcut01", "c_navcut02", "c_navcut03")


@unittest.skipUnless(os.environ.get("OPENFO4_FO4_PATH"), "OPENFO4_FO4_PATH names no Fallout 4 install.")
class RealShapeLayerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        data_dir = Path(os.environ["OPENFO4_FO4_PATH"]) / "Data"
        nifs: Dict[str, bytes] = {}
        for archive in MESH_ARCHIVES:
            nifs.update(read_general_ba2_entries(data_dir / archive, {JUNK_WALL_DOOR} - set(nifs)))
        with tempfile.TemporaryDirectory() as folder:
            nif = Path(folder) / "mesh.nif"
            sidecar = Path(folder) / "mesh.col.json"
            nif.write_bytes(nifs[JUNK_WALL_DOOR])
            worker.process_nif(nif, sidecar)
            cls.shapes = json.loads(sidecar.read_text())["shapes"]

    def test_the_navcut_shapes_carry_the_navcut_layer(self):
        navcut = {shape["node"].lower(): shape for shape in self.shapes if shape["node"].lower() in NAVCUT_NODES}
        self.assertEqual(sorted(navcut), sorted(NAVCUT_NODES))
        self.assertEqual({shape["layer"] for shape in navcut.values()}, {NAVCUT_LAYER})

    def test_the_wall_trimesh_does_not_carry_the_navcut_layer(self):
        walls = [shape for shape in self.shapes if shape["type"] == "trimesh" and shape["node"].lower() not in NAVCUT_NODES]
        self.assertTrue(walls)
        for wall in walls:
            self.assertIn("layer", wall)
            self.assertNotEqual(wall["layer"], NAVCUT_LAYER)


if __name__ == "__main__":
    unittest.main()
