# OpenFO4 documentation

OpenFO4 is made of two programs. The installer converts the user's Fallout 4
install into common file formats. The engine reads only that output and never
touches the original game files.

The conversion happens on the user's machine because there is far more
well-maintained and optimized infrastructure for loading common file formats.
That makes development easier and the engine more stable and performant.

Nothing derived from Bethesda assets is part of this repository or
redistributed, so a legitimate Fallout 4 install is always required.

OpenFO4 targets Windows desktop only for now.

## Codebases

- `src/installer/` (C#) converts the game install.
- `src/engine/` (Rust) runs the game from the converted files.

What the installer writes and the engine reads is described in
[install-contract.md](install-contract.md).

## Documents

| Document | Covers |
|---|---|
| [install-contract.md](install-contract.md) | The install layout shared by the installer and the engine |
| [engine/](engine/README.md) | Engine lifecycle, threading and module structure |
| [engine/game-flow.md](engine/game-flow.md) | Game phases from the main menu to in-game |
| [engine/world-streaming.md](engine/world-streaming.md) | Loading the world, textures and grass around the player |
| [engine/simulation.md](engine/simulation.md) | Gameplay state, physics and the player controller |
| [engine/navmesh.md](engine/navmesh.md) | Runtime navigation meshes |
| [engine/characters.md](engine/characters.md) | Animation, skinning and ragdolls |
| [engine/audio-and-video.md](engine/audio-and-video.md) | Music, sound and video playback |
| [engine/ui.md](engine/ui.md) | Menus, HUD and Pip-Boy |
| [engine/debugging.md](engine/debugging.md) | Debug views, profiling and environment switches |
| [engine/rendering/](engine/rendering/README.md) | GPU frame structure, culling and shader composition |
| [engine/rendering/lighting-and-shadows.md](engine/rendering/lighting-and-shadows.md) | Lighting and shadows |
| [engine/rendering/surfaces-and-effects.md](engine/rendering/surfaces-and-effects.md) | Water, refraction, grass, particles and effects |
| [engine/rendering/post-processing.md](engine/rendering/post-processing.md) | Post-processing |
| [installer/](installer/README.md) | Installer session, pipeline lanes and resumable conversion |
| [installer/archives.md](installer/archives.md) | Reading and extracting BA2 archives |
| [installer/meshes-and-materials.md](installer/meshes-and-materials.md) | Meshes, materials and textures |
| [installer/collision.md](installer/collision.md) | Collision extraction |
| [installer/animation.md](installer/animation.md) | Havok animation conversion |
| [installer/database.md](installer/database.md) | Converting plugin records into the database |
| [installer/interface.md](installer/interface.md) | UI layout, fonts and Pip-Boy art |
| [installer/audio-and-video.md](installer/audio-and-video.md) | Audio and video conversion |
| [installer/shader-translation.md](installer/shader-translation.md) | Translating the game's shaders to WGSL |

Setting up, building and testing are covered in
[development/README.md](development/README.md).
