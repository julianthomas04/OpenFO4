# Interface

The game's UI is built from Flash movies run by Scaleform. The interface lane
exports three things from the game's UI archive: fonts, UI layout documents and
Pip-Boy art.

ffdec, a Java Flash decompiler built by `tools/ffdec`, exports the fonts and
vector art embedded in the movies. Fonts are exported as TTF, and the game's
font configuration maps UI font classes to font families, which the engine
loads by name.

`file_converters/swf/` and `file_converters/abc/` are readers for Flash movies
and the scripts inside them that carry UI values. `file_converters/ui_layout/`
uses them to turn each interface movie into a
layout document the engine reads.

Pip-Boy art is exported as image frames plus a manifest, with each piece found
by the names the movies use for it.
