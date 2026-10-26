# Lighting and shadows

`LightingState` (`lighting/`) owns the sun, sky, fog, exposure and shadow
settings on the CPU, and `frame_uniform` is the one place that turns them into
GPU data. Exteriors are lit from the worldspace's weather. Interiors are lit per
cell, including values inherited from lighting templates, with a neutral
fallback for cells that have no usable data.

## Time of day

The exterior follows the [world clock](../simulation.md#world-clock). The
weather's eight time slots (Sunrise, Day, Sunset, Night and the four Early/Late
slots) are loaded once. Each frame the two slots around the current hour are
blended in linear space: colours, light scales, exposure, ambient and depth of
field. Fog distances exist only for Day and Night and follow how much daylight
the moment has. Fog that is on in one and off in the other switches at the
midpoint.

The one directional light is the sun or the moon, whichever is up, coloured by
the current slot's Sunlight. Its direction turns from the sun to the moon as the
sun fades, and its strength follows the elevation of the body. Sun shadows
follow that light and switch off only when neither body gives light. The sun
disc always sits at the sun's own position. Its alpha and the water's daylight
factor ramp around sunrise and sunset over `fSunAlphaTransTime`.

Interiors keep their per-cell lighting and are not affected by the clock.

The game data does not pin down every rule of the sky. These are working
assumptions (`time_of_day/unmeasured.rs`) pending in-game measurement: the slot
keyframes inside the twilight windows, the sun's arc, the moon's path and phase
over the days, the star dome's rotation, the apparent size of the moon, the
brightness of moon and stars, and how the sun and moon light fade with
elevation.

## Placed lights

Placed lights (`local_lights/`) are handled by splitting the view into a 3D
grid. A compute pass builds a short list of lights for each grid cell, and
shaders read only their own cell's list, so many lights stay cheap.

### Falloff

A light's distance falloff is `(1 - saturate(max(C + S * x^E, x^2)))^2.2`, where
`x` is the distance divided by the light's radius and `C`, `S`, `E` are the
Constant, Scalar and Exponent of its LIGH record. The `x^2` term is a window
that brings every light to 0 at its radius with zero slope. The record term can
only remove more light than the window does. With the default `(0, 1, 2)` the two terms
coincide. `local_light_attenuation` in `lighting.wgsl` is the reference, and
`EffectLight::attenuation_at` mirrors the window on the CPU for effect lighting.

Flicker and pulse animation, flicker movement, the XLIG End Distance Cap, gobo
projection and god rays do not feed into the falloff.

## Shadows

Sun shadows (`shadows/`) split the view into zones from near to far, each with
its own shadow map, which keeps nearby shadows sharp. Far zones re-render less
often.

The most visible shadow-casting lamps render into a shared texture of tiles, and
a tile re-renders when something near it changes.

A single GPU cull covers every shadow view and reuses the world's instance data.
