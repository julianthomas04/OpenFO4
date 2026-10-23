# Animation

`HavokAnimationStage` converts creature animations from Havok (`.hkx`, the
game's animation format) into glTF. The `.hkx` reader (`file_converters/havok/`) is written in
this repository.

The skeleton comes from the [mesh conversion](meshes-and-materials.md), so
animation waits for it, and clips are bound to the skeleton by bone name. Clips
are grouped into packs, with an index file that lists the packs and their clips.
