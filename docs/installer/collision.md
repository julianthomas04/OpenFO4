# Collision

Each mesh with collision gets a sidecar file next to its GLB. It holds the
collision shapes, plus physics data for objects that can move or ragdoll.

A Python worker (`python/fo4_collision_worker.py`) runs as a child process and
reads the collision out of the meshes using `third_party/pynifly`. While meshes
are extracted, the C# side marks the ones that contain collision, and the worker
opens only those.

A small manifest indexes all sidecars. The engine reads it once at startup to
know which meshes have collision without opening every sidecar.

A mesh that already has a sidecar is skipped, and sidecars from an older
extractor version are rewritten.
