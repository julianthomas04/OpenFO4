# UI

The HUD, the menus and the loading overlay are drawn by a custom wgpu pass as
coloured shapes and text. The Pip-Boy and the developer menu use egui. The HUD,
or the Pip-Boy in its place, sits over the world; the fade and loading overlay
sit above that, and the developer menu is on top.

## Layout

Positions, sizes, fonts and strings come from layout documents that the
installer extracts from the game's interface files, read through `ui_layout/`.

## HUD

The HUD's shapes and animation come from the game's HUD layout. The engine
supplies only the live values, such as fill levels and colour.

## Pip-Boy

The Pip-Boy is a full-screen overlay, and opening it pauses the
[simulation](simulation.md). Its data lives in the simulation (`sim/pipboy/`);
the screen reads a snapshot of it and changes it only through simulation
commands.
