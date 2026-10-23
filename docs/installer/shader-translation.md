# Shader translation

`shader_translator/` translates the game's shaders from its shader archive into
WGSL. `archive/` reads the shader package, `dxbc/` decodes the compiled DirectX
bytecode, and `wgsl/` writes the WGSL.

`shader_manifest.json` lists which shaders to translate and where each of their
resources lands in WGSL. The output is functions and their interface types
rather than complete shaders; the engine's wrappers supply the rest (see
[rendering](../engine/rendering/README.md)).

The shaders folder is replaced as a whole, so a failed run leaves the previous
one untouched.
