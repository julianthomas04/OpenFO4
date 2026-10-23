"""Rules decided by a file's path alone."""

import posixpath

from finding import Finding

FORBIDDEN_EXTENSIONS = {
    ".nif": "Gamebryo model",
    ".dds": "DDS texture",
    ".ba2": "BA2 archive",
    ".bsa": "BSA archive",
    ".esm": "master plugin",
    ".esp": "plugin",
    ".esl": "light plugin",
    ".bgsm": "BGSM material",
    ".bgem": "BGEM material",
    ".hkx": "Havok file",
    ".hkt": "Havok file",
    ".pex": "compiled Papyrus script",
    ".psc": "Papyrus script source",
    ".fuz": "FUZ voice file",
    ".xwm": "xWMA audio",
    ".lip": "lip-sync file",
    ".swf": "Scaleform movie",
    ".gfx": "Scaleform movie",
    ".fxp": "shader package",
    ".fxc": "compiled shader",
    ".cso": "compiled shader",
    ".dxbc": "compiled shader",
    ".hlsl": "shader source",
    ".bto": "terrain LOD",
    ".btr": "terrain LOD",
    ".btt": "terrain LOD",
    ".strings": "string table",
    ".dlstrings": "string table",
    ".ilstrings": "string table",
    ".glb": "converted model",
    ".gltf": "converted model",
    ".anims": "converted animation",
    ".col": "converted collision",
    ".ttf": "extracted font",
    ".otf": "extracted font",
}

# Where the engine writes its runtime caches and where extraction or translation tools tend to dump their output.
FORBIDDEN_DIRECTORY_NAMES = frozenset({"cache", "translated", "converted", "extracted", "unpacked"})


def check_path(path: str) -> list[Finding]:
    findings = []
    extension = posixpath.splitext(path)[1].lower()
    if extension in FORBIDDEN_EXTENSIONS:
        findings.append(Finding("forbidden-extension", path, f"{extension} is a game or converted-asset format ({FORBIDDEN_EXTENSIONS[extension]})"))
    for directory in posixpath.dirname(path).split("/"):
        if directory.lower() in FORBIDDEN_DIRECTORY_NAMES:
            findings.append(Finding("forbidden-directory", path, f"'{directory}' is a directory name used for installer or extraction output"))
            break
    return findings
