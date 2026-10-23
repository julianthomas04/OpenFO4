# Surfaces and effects

## Water

The water stage (`water/`) draws all water surfaces, sorted back to front and
shaded from the scene snapshot. Where water is lives in `water_world`, part of
the [simulation](../simulation.md). The look comes from the game's water
records, and the shaders are translated from the game.

A height field around the player is stirred by things moving through the water,
and the water shader reads it to draw ripples. Water reflects what is already on
screen, and reflections are only computed when visible water needs them. While
the camera is under water, fog and blur take over.

## Heat haze

Heat haze (`refraction/`) draws flat cards that distort the scene behind them.

## Grass

Grass (`grass/`) is drawn once per cell and grass type. It fades with distance,
is culled by view and sways in the wind.

## Particles and effects

Particles (`particles/`) are simulated on the CPU as flat quads facing the
camera, and each belongs to the object that spawned it. The effect stage
(`effects/`) draws effect meshes and particles in the translucent pass, with
blended draws ordered far to near. Effect shaders and textures are prepared on a
worker thread, and a draw is skipped until they are ready.
