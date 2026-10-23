# Debugging

## Developer menu

The developer menu is an egui window available during gameplay. Esc frees the
mouse to use it, and common toggles also have hotkeys. It shows how long each
render pass takes on the GPU, and it can replace the final image with a
diagnostic view, such as surface colours only. Overlays draw normally invisible
data, such as collision and navmeshes, over the scene.

## Profiling

`OPENFO4_PERF_LOG=1` writes one line of frame statistics per second to the log.
Tracy can be used to profile the engine by building with the `tracy` feature.

## Logging

Each session writes a log to `<install>/logs/`, and only the most recent
sessions are kept. `RUST_LOG` sets the filter.

## Environment switches

All environment switches start with `OPENFO4_`. An invalid value for an optional
switch logs a warning and falls back to the default.
