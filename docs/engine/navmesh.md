# Navmesh

The navmesh (`navmesh/`) is the walkable surface that NPC pathfinding is based
on. It is made of triangles that each know their neighbours, and it is split
into meshes that each cover part of a cell. The meshes come from the database
and load and unload with the cells of the [world streamer](world-streaming.md).

`NavWorld` holds the loaded meshes and the links that connect the edges of
neighbouring meshes. A link to a mesh that isn't loaded yet waits and becomes
active once that mesh loads.
