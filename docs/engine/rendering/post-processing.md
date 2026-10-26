# Post-processing

`PostChain` (`post/`) owns everything between the world pass and the screen:
the high-range colour target, the sky, depth of field, exposure and
tonemapping.

The sky is drawn as part of the world pass and only fills pixels where nothing
else was drawn. A gradient comes first, then the moon and the stars
(`night_sky/`) at the positions the clock gives them. A moon or star texture or
the star mesh that is missing from the install is logged and that object is
skipped.

Depth of field blurs the scene by distance. It runs after the translucent pass
and before exposure, and its shaders are translated from the game.

Auto exposure measures the scene's average brightness on the GPU and adapts
gradually, the way an eye adjusts. Its speed and range come from the weather.
Tonemapping then turns the high-range scene into colours a display can show,
with settings from the lighting state.
