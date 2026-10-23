# Copyright 2026 Julian Thomas and OpenFO4 contributors
# SPDX-License-Identifier: Apache-2.0
"""Generate small original SWFs; no game data or external packages are used."""
from pathlib import Path
import struct
import sys


class Bits:
    def __init__(self):
        self.bits = []

    def put(self, value, count):
        self.bits.extend((value >> bit) & 1 for bit in reversed(range(count)))

    def signed(self, value, count):
        self.put(value & ((1 << count) - 1), count)

    def data(self):
        padded = self.bits + [0] * (-len(self.bits) % 8)
        return bytes(sum(padded[i + j] << (7 - j) for j in range(8)) for i in range(0, len(padded), 8))


def rect(xmin, xmax, ymin, ymax):
    width = max(abs(value).bit_length() + 1 for value in (xmin, xmax, ymin, ymax))
    bits = Bits()
    bits.put(width, 5)
    for value in (xmin, xmax, ymin, ymax):
        bits.signed(value, width)
    return bits.data()


def outline():
    bits = Bits()
    bits.put(1, 4)
    bits.put(0, 4)
    bits.put(0b000101, 6)  # Move and select fill style 1.
    bits.put(1, 5)
    bits.put(0, 2)
    bits.put(1, 1)
    for dx, dy in ((400, 0), (0, 400), (-400, 0), (0, -400)):
        width = max(abs(dx), abs(dy)).bit_length() + 1
        bits.put(0b11, 2)
        bits.put(width - 2, 4)
        bits.put(0, 1)
        bits.put(int(dy != 0), 1)
        bits.signed(dy if dy else dx, width)
    bits.put(0, 6)
    return bits.data()


def tag(code, data=b""):
    if len(data) < 63:
        return struct.pack("<H", code << 6 | len(data)) + data
    return struct.pack("<HI", code << 6 | 63, len(data)) + data


def movie(tags, frame_count):
    body = rect(0, 800, 0, 800) + struct.pack("<HH", 24 << 8, frame_count) + tags + tag(0)
    return b"FWS\x0a" + struct.pack("<I", len(body) + 8) + body


def create(destination):
    destination.mkdir(parents=True, exist_ok=True)
    shape = struct.pack("<H", 1) + rect(0, 400, 0, 400) + b"\x01\x00\x00\xff\x00\xff\x00" + outline()
    matrix = Bits()
    matrix.put(0, 2)
    matrix.put(1, 5)
    matrix.put(0, 2)
    place_shape = tag(26, b"\x06" + struct.pack("<HH", 1, 1) + matrix.data())
    sprite = struct.pack("<HH", 2, 2) + place_shape + tag(1) + tag(1) + tag(0)
    glyph = outline()
    name = b"OpenFO4Synthetic"
    font = struct.pack("<HBBB", 3, 0, 0, len(name)) + name + struct.pack("<HHH", 1, 4, 4 + len(glyph)) + glyph + b"A"
    root_place = tag(26, b"\x06" + struct.pack("<HH", 1, 2) + matrix.data())
    definitions = tag(32, shape) + tag(39, sprite) + tag(48, font)
    (destination / "exports.swf").write_bytes(movie(definitions + root_place + tag(1) + tag(1), 2))
    (destination / "nested.swf").write_bytes(movie(definitions + root_place + tag(1), 1))
    video = tag(60, struct.pack("<HHHHBB", 4, 1, 20, 20, 0, 2))
    (destination / "unused-video.swf").write_bytes(movie(definitions + video + root_place + tag(1) + tag(1), 2))
    place_video = tag(26, b"\x06" + struct.pack("<HH", 1, 4) + matrix.data())
    (destination / "visible-video.swf").write_bytes(movie(definitions + video + place_video + tag(1), 1))
    (destination / "malformed.swf").write_bytes(b"FWS\x0a\xff\xff\xff\xff")


if __name__ == "__main__":
    create(Path(sys.argv[1]))
