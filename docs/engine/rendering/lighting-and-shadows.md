# Lighting and shadows

`LightingState` (`lighting/`) owns the sun, sky, fog, exposure and shadow
settings on the CPU, and `frame_uniform` is the one place that turns them into
GPU data. Exteriors are lit from the worldspace's weather. Interiors are lit per
cell, including values inherited from lighting templates, with a neutral
fallback for cells that have no usable data.

## Time of day

The exterior follows the world clock. The weather's eight time slots (Sunrise,
Day, Sunset, Night and the four Early/Late slots) are loaded once. Each frame the
clock picks the two slots around the current time and blends their colours, light
scales, exposure, ambient and depth of field in linear space. Fog distances exist
only for Day and Night and are blended by how much daylight the moment has; fog that
is on in one of the two and off in the other switches at the midpoint instead of
blending.

The one directional light is whichever of the sun and the moon is up, with the
weather's Sunlight colour of the current slot, so the night slot lights the world
by the moon. Its direction turns from the sun to the moon as the sun fades, its strength
fades with the elevation of the sun or the moon, and the
sun shadows follow it and turn off only when neither body gives light. The sun
disc in the sky always sits at the sun's own position, and its alpha and the
water's daylight factor ramp around sunrise and sunset over the game's
`fSunAlphaTransTime`.

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
