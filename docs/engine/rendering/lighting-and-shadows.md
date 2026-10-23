# Lighting and shadows

`LightingState` (`lighting/`) owns the sun, sky, fog, exposure and shadow
settings on the CPU, and `frame_uniform` is the one place that turns them into
GPU data. Exteriors are lit from the worldspace's weather. Interiors are lit per
cell, including values inherited from lighting templates, with a neutral
fallback for cells that have no usable data.

## Placed lights

Placed lights (`local_lights/`) are handled by splitting the view into a 3D
grid. A compute pass builds a short list of lights for each grid cell, and
shaders read only their own cell's list, so many lights stay cheap.

## Shadows

Sun shadows (`shadows/`) split the view into zones from near to far, each with
its own shadow map, which keeps nearby shadows sharp. Far zones re-render less
often.

The most visible shadow-casting lamps render into a shared texture of tiles, and
a tile re-renders when something near it changes.

A single GPU cull covers every shadow view and reuses the world's instance data.
