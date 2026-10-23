"""Rules decided by raw file bytes: game-format signatures and compiled shaders hidden inside other files."""

import struct
from dataclasses import dataclass

from finding import Finding

BINARY_SNIFF_BYTES = 8192
DXBC_HEADER_BYTES = 32
DXBC_MAX_CHUNKS = 64


@dataclass(frozen=True)
class Signature:
    label: str
    parts: tuple[tuple[int, bytes], ...]
    version_byte: tuple[int, range] | None = None

    def matches(self, data: bytes) -> bool:
        if not all(data[offset:offset + len(magic)] == magic for offset, magic in self.parts):
            return False
        if self.version_byte is None:
            return True
        offset, accepted = self.version_byte
        return offset < len(data) and data[offset] in accepted


def _swf(label: str, magic: bytes) -> Signature:
    return Signature(label, ((0, magic),), version_byte=(len(magic), range(1, 64)))


GAME_SIGNATURES = (
    Signature("DXBC shader container", ((0, b"DXBC"),)),
    Signature("BA2 archive", ((0, b"BTDX"),)),
    Signature("BSA archive", ((0, b"BSA\0"),)),
    Signature("plugin / master (TES4)", ((0, b"TES4"),)),
    Signature("plugin / master (TES3)", ((0, b"TES3"),)),
    Signature("Gamebryo NIF model", ((0, b"Gamebryo File Format"),)),
    Signature("NetImmerse NIF model", ((0, b"NetImmerse File Format"),)),
    Signature("DDS texture", ((0, b"DDS "),)),
    Signature("Havok packfile", ((0, bytes([0x57, 0xE0, 0xE0, 0x57, 0x10, 0xC0, 0xC0, 0x10])),)),
    Signature("Havok tagfile", ((4, b"TAG0"),)),
    Signature("compiled Papyrus script (PEX)", ((0, bytes.fromhex("DEC057FA")),)),
    Signature("compiled Papyrus script (PEX, big-endian)", ((0, bytes.fromhex("FA57C0DE")),)),
    Signature("xWMA audio", ((0, b"RIFF"), (8, b"XWMA"))),
    Signature("FUZ voice file", ((0, b"FUZE"),)),
    Signature("BGSM material", ((0, b"BGSM"),)),
    Signature("BGEM material", ((0, b"BGEM"),)),
    _swf("SWF movie (uncompressed)", b"FWS"),
    _swf("SWF movie (zlib)", b"CWS"),
    _swf("SWF movie (LZMA)", b"ZWS"),
    _swf("Scaleform GFX movie", b"GFX"),
    _swf("Scaleform GFX movie (compressed)", b"CFX"),
)


def is_binary(data: bytes) -> bool:
    return b"\0" in data[:BINARY_SNIFF_BYTES]


def check_signatures(path: str, data: bytes) -> list[Finding]:
    """Every game format has NUL bytes in its first kilobytes, so text files that merely begin with a magic word (a note starting with 'DDS ') are not flagged."""
    if not is_binary(data):
        return []
    return [
        Finding("game-magic", path, f"starts with the signature of a {signature.label}")
        for signature in GAME_SIGNATURES
        if signature.matches(data)
    ]


def check_embedded_dxbc(path: str, data: bytes) -> list[Finding]:
    """A DXBC container at a non-zero offset (shader package, archive or blob smuggled into another file)."""
    findings = []
    start = data.find(b"DXBC", 1)
    while start != -1:
        if _is_plausible_container(data, start):
            findings.append(Finding("embedded-dxbc", path, f"contains a DXBC shader container at byte offset {start}"))
            break
        start = data.find(b"DXBC", start + 1)
    return findings


def _is_plausible_container(data: bytes, start: int) -> bool:
    if len(data) - start < DXBC_HEADER_BYTES:
        return False
    version, total_size, chunk_count = struct.unpack_from("<III", data, start + 20)
    return version == 1 and DXBC_HEADER_BYTES <= total_size <= len(data) - start and 1 <= chunk_count <= DXBC_MAX_CHUNKS
