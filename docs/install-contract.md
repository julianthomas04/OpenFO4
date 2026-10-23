# Install contract

The installer and the engine share one folder, the install root. This page
describes who owns which part of it.

## Layout

The OpenFO4 executable sits at the install root.

| Path | Owner | Contents |
|---|---|---|
| `assets/` | Installer | Everything the engine reads |
| `assets/meshes/` | Installer | Meshes and their collision sidecars |
| `assets/textures/` | Installer | Textures |
| `assets/audio/` | Installer | Sound and music |
| `assets/video/` | Installer | Videos |
| `assets/interface/` | Installer | Fonts, UI layout documents and Pip-Boy art |
| `assets/data/` | Installer | The database and the game's INI files |
| `assets/shaders/` | Installer | Shaders |
| `staging/` | Installer | Intermediate files, removed after a successful install |
| `install.log` | Installer | The installer's log |
| `.openfo4-install` | Installer | Marks the folder as an OpenFO4 install |
| `cache/` | Engine | Data the engine derives at runtime |
| `logs/` | Engine | The engine's session logs |

Neither program writes to the other's paths. `cache/` can be deleted at any
time because the engine regenerates whatever is missing, and a reinstall leaves
it in place.

## Shared names

The folder and file names are defined in `src/installer/install_layout.cs`. The
tests in the engine's `src/engine/src/assets/layout.rs` compare its names
against that file and fail when the two sides disagree.

## Install marker

The installer writes `.openfo4-install` empty when an install starts. Only after
every part of the install has succeeded does it write the asset version into the
file. An empty marker therefore means an incomplete install.

## Finding the install

The engine uses `OPENFO4_INSTALL_DIR` if it is set, and otherwise the `assets/`
folder next to its executable.
