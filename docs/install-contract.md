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

`.openfo4-install` records which asset groups of the install are complete. The
installer writes it empty when an install starts in a folder without one; when
it updates an existing install, it first rewrites the marker to list only the
groups it keeps. Its content is a first line
`Asset Version <installer version>`, naming the installer that last wrote the
file, then a line `Game Language <code>` and a line `Game Files <hash>`, naming the
language the game is installed in and a SHA-256 fingerprint of the game files
the installer reads, and then one `<group id>=<revision>` line per asset group
whose converted output is complete, at the revision it was converted at. The
installer adds a group's line when that group finishes, unless an archive of its
input failed to extract in a way a rerun can repair, in which case the group
stays unlisted and the next install converts it again, so a failed or cancelled update leaves
the finished groups listed and the unfinished ones absent. An empty marker means
an install started and no group has finished.

The installer keeps a group when the marker's language and game files match the
current ones and the group's revision matches the installer's. Otherwise it
converts the group again, together with every group built from it. A different or
missing language or fingerprint converts every group. A marker from before
groups or game inputs were recorded, an unreadable one (a malformed line, a
repeated id, a revision that is not a positive number, only one of the two game
lines) and an empty one all count as no record, and every group is rebuilt. The engine does
not read the marker.

## Finding the install

The engine uses `OPENFO4_INSTALL_DIR` if it is set, and otherwise the `assets/`
folder next to its executable.
