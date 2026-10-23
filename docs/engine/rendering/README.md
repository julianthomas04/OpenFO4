# Rendering

`State::render` (`rendering/mod.rs`) sets the order of the whole frame: culling,
shadows, the opaque world, grass, water, translucent objects, depth of field,
exposure and tonemapping, and finally the HUD and overlays. Menu frames take a
separate, simpler path.

The scene renders into a high-range colour target and is converted to screen
colours at the very end. Depth is reversed: near is 1 and far is 0.

After the opaque pass, a snapshot of colour and depth is taken so that water,
heat haze and effects can show what is behind them.

## GPU-driven rendering

The CPU doesn't issue a draw per object. All instances sit in one GPU buffer, and
a compute shader tests each one and writes the draw commands for those that
survive. It drops objects outside the view and objects hidden behind what was
drawn in the previous frame (`hiz.rs`).

Draws are grouped by pipeline state. Each material is stored once and referenced
by id, and all textures sit in one large table that any shader can index, so
nothing is rebound between draws.

## Shaders

Shaders come from two places. The visual effects of the game, such as water,
reflections, depth of field, grass movement and effect materials, are translated
from the user's own game at install time. The shaders in this repository
(`rendering/shaders/`) are not taken from the game. They are written for this
engine, because they do work that is specific to how it draws: culling on the
GPU, drawing every material through one shared pipeline, and lighting, shadows
and tonemapping built around that.

The two meet through repository WGSL wrappers that own the entry points and
bindings and call the translated functions. `game_frame/cb12.rs` fills the
constant block the game's shaders read the camera and fog from, in the
conventions they expect.
