# Engine

The engine is the Rust program in `src/engine/` that runs the game from the
converted install.

## Lifecycle

`main` resolves the install and starts logging, then hands control to the winit
event loop in `platform/`. A startup worker builds the game state while the main
thread keeps presenting black frames. From then on every frame runs
`State::update`, then `State::render`.

## State

`State` is the application core. It owns the simulation, the game flow, audio,
the world streamer and the renderer, and it runs the frame. It lives in
`rendering/`.

## Simulation boundary

Gameplay state lives in `sim/`. Every change enters it as a `SimCommand`, and
physics engine types never leave the module.

## Threading

The main thread runs the event loop, the update, the simulation tick and
rendering. Background threads only load and prepare data.

## Errors

Each module defines its own error type. An error during the update is fatal: the
loop ends and the engine shows a dialog. Losing the GPU device is fatal as well.
A render error is logged and the next frame continues.

## Coordinates

Game data is Z-up. The engine is Y-up and uses one engine unit per ten game
units. `bethesda_space.rs` owns the conversion.

## Modules

| Module | Responsibility | Doc |
|---|---|---|
| `platform/` | Window, event loop, boot phases and fatal-error display | |
| `startup/` | Building the game state off the main thread | |
| `assets/` | Locating and reading the install | [install contract](../install-contract.md) |
| `game_flow/` | Game phases from the main menu to in-game | [game-flow.md](game-flow.md) |
| `rendering/` | `State` and the GPU frame | [rendering/](rendering/README.md) |
| `sim/` | Gameplay state and physics | [simulation.md](simulation.md) |
| `water_world/` | The world-side water model | [simulation.md](simulation.md) |
| `navmesh/` | Runtime navigation meshes | [navmesh.md](navmesh.md) |
| `anim/` | Skeletons, clips and pose playback | [characters.md](characters.md) |
| `audio/`, `footsteps/` | Music and sound | [audio-and-video.md](audio-and-video.md) |
| `video/` | Video playback | [audio-and-video.md](audio-and-video.md) |
| `ui_layout/` | Reading the converted UI layout | [ui.md](ui.md) |
