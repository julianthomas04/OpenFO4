# Copyright 2026 Julian Thomas and OpenFO4 contributors
# SPDX-License-Identifier: Apache-2.0
"""Only permitted nondeterminism: TTF head timestamps and their checksums."""
import struct
import xml.etree.ElementTree as ET


def normalized(path):
    data = bytearray(path.read_bytes())
    if path.suffix == ".ttf":
        tables = struct.unpack_from(">H", data, 4)[0]
        for index in range(tables):
            at = 12 + 16 * index
            if data[at:at + 4] == b"head":
                offset = struct.unpack_from(">I", data, at + 8)[0]
                if offset + 36 > len(data):
                    raise ValueError("Truncated TTF head table")
                data[at + 4:at + 8] = b"\0" * 4
                data[offset + 8:offset + 12] = b"\0" * 4
                data[offset + 20:offset + 36] = b"\0" * 16
    elif path.suffix == ".svg":
        if ET.fromstring(data).tag != "{http://www.w3.org/2000/svg}svg":
            raise ValueError(f"Not SVG: {path}")
    return bytes(data)
