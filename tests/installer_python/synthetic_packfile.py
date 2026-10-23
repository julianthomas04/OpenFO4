"""Builds a minimal Havok 2014.1 packfile around one hknpCompressedMeshShape, from plain numbers.

Only the fields the collision decoder reads are written; every byte is synthetic.
"""

import struct
from typing import List, Sequence, Tuple

DATA_ALIGN = 16
SHAPE_SIZE = 0xC0
PROPERTIES_SIZE = 0x20
MATERIAL_PROPERTIES_SIZE = 0x40
MESH_DATA_SIZE = 0xB0
SECTION_SIZE = 0x60
HEADER_SIZE = 0x100
CLASSNAMES = ("hknpCompressedMeshShape", "hkRefCountedProperties", "hknpBSMaterialProperties",
              "hknpCompressedMeshShapeData")
NO_USER_DATA = 0xFCB37EA0


def pack_run(material_index: int, first_quad: int, quad_count: int) -> int:
    return material_index | (first_quad << 16) | (quad_count << 24)


def pack_vertex(qx: int, qy: int, qz: int) -> int:
    return qx | (qy << 11) | (qz << 22)


class _DataSection:
    def __init__(self) -> None:
        self.bytes = bytearray()
        self.local: List[Tuple[int, int]] = []
        self.global_: List[Tuple[int, int]] = []

    def reserve(self, size: int) -> int:
        offset = len(self.bytes)
        self.bytes.extend(b"\0" * size)
        self.bytes.extend(b"\0" * (-len(self.bytes) % DATA_ALIGN))
        return offset

    def put(self, offset: int, fmt: str, *values) -> None:
        struct.pack_into("<" + fmt, self.bytes, offset, *values)

    def array(self, field: int, target: int, count: int) -> None:
        self.local.append((field, target))
        self.put(field + 8, "II", count, count | 0x80000000)


def build_compressed_mesh_packfile(
        material_hashes: Sequence[int], quads: Sequence[Tuple[int, int, int, int]],
        runs: Sequence[int], vertices: Sequence[Tuple[int, int, int]],
        user_data: int = NO_USER_DATA, with_materials: bool = True) -> bytes:
    """One shape, one section holding every quad; `runs` are the section's primitive runs."""
    data = _DataSection()
    shape = data.reserve(SHAPE_SIZE)
    properties = data.reserve(PROPERTIES_SIZE)
    property_entries = data.reserve(0x10)
    material_properties = data.reserve(MATERIAL_PROPERTIES_SIZE)
    material_entries = data.reserve(0x18 * max(1, len(material_hashes)))
    mesh = data.reserve(MESH_DATA_SIZE)
    sections = data.reserve(SECTION_SIZE)
    quad_array = data.reserve(4 * len(quads))
    run_array = data.reserve(4 * len(runs))
    vertex_array = data.reserve(4 * len(vertices))

    data.put(shape + 0x18, "I", user_data)
    data.global_.append((shape + 0x60, mesh))
    if with_materials:
        data.global_.append((shape + 0x20, properties))
        data.array(properties, property_entries, 1)
        data.global_.append((property_entries, material_properties))
        data.array(material_properties + 0x10, material_entries, len(material_hashes))
        for i, material_hash in enumerate(material_hashes):
            data.put(material_entries + 0x18 * i + 0x10, "II", 1, material_hash)

    data.array(mesh + 0x50, sections, 1)
    data.array(mesh + 0x60, quad_array, len(quads))
    data.array(mesh + 0xA0, run_array, len(runs))
    data.array(mesh + 0x80, vertex_array, len(vertices))
    data.put(mesh + 0x20, "ffff", 0.0, 0.0, 0.0, 0.0)
    data.put(mesh + 0x30, "ffff", 1.0, 1.0, 1.0, 0.0)
    data.put(sections + 0x30, "fff", 0.0, 0.0, 0.0)
    data.put(sections + 0x3C, "fff", 1.0, 1.0, 1.0)
    data.put(sections + 0x48, "I", 0)
    data.put(sections + 0x4C, "I", len(vertices))
    data.put(sections + 0x50, "I", len(quads))
    data.put(sections + 0x54, "I", len(runs))
    for i, (a, b, c, d) in enumerate(quads):
        data.put(quad_array + 4 * i, "BBBB", a, b, c, d)
    for i, run in enumerate(runs):
        data.put(run_array + 4 * i, "I", run)
    for i, (qx, qy, qz) in enumerate(vertices):
        data.put(vertex_array + 4 * i, "I", pack_vertex(qx, qy, qz))

    objects = [(shape, CLASSNAMES[0]), (mesh, CLASSNAMES[3])]
    if with_materials:
        objects += [(properties, CLASSNAMES[1]), (material_properties, CLASSNAMES[2])]
    return _assemble(data, objects)


def _assemble(data: _DataSection, objects: List[Tuple[int, str]]) -> bytes:
    names = bytearray()
    name_offsets = {}
    for name in CLASSNAMES:
        name_offsets[name] = len(names)
        names += struct.pack("<IB", 0, 9) + name.encode() + b"\0"
    names += b"\xff" * (-len(names) % DATA_ALIGN)

    local = b"".join(struct.pack("<II", src, dst) for src, dst in data.local) + b"\xff" * 8
    global_ = b"".join(struct.pack("<III", src, 2, dst) for src, dst in data.global_) + b"\xff" * 12
    virtual = b"".join(struct.pack("<III", rel, 0, name_offsets[name] + 5) for rel, name in objects) + b"\xff" * 12

    classnames_start = HEADER_SIZE
    data_start = classnames_start + len(names)
    local_rel = len(data.bytes)
    global_rel = local_rel + len(local)
    virtual_rel = global_rel + len(global_)
    exports_rel = virtual_rel + len(virtual)

    header = bytearray(HEADER_SIZE)
    header[0x40:0x40 + 14] = b"__classnames__"
    struct.pack_into("<I", header, 0x40 + 0x14, classnames_start)
    header[0xC0:0xC0 + 8] = b"__data__"
    struct.pack_into("<IIIIII", header, 0xC0 + 0x14, data_start, local_rel, global_rel, virtual_rel, exports_rel, exports_rel)
    struct.pack_into("<I", header, 0xC0 + 0x2C, exports_rel)
    return bytes(header) + bytes(names) + bytes(data.bytes) + local + global_ + virtual
