# World streaming

The world streamer (`rendering/world_streamer.rs`) loads and unloads the world
around the player. It decides which cells are resident and owns their lifetime
on the CPU and the GPU.

## Rings

Two squares of cells surround the player's cell: an inner ring at full detail
and an outer ring at LOD. When the player enters a new cell, only the difference
is requested, nearest cells first, and cells that leave both rings are released.

A cell changes detail level without popping: its old version is released only
after the new one is in place.

## Workers

A pool of worker threads does the heavy work for each cell, such as database
queries, building terrain, parsing meshes and baking colliders. Each worker has
its own read-only database connection. Workers share a size-bounded cache
(`shared_cache.rs`), so concurrent requests for the same file load it once.

Results return to the main thread through a bounded channel, which keeps memory
in check. The main thread integrates them into the GPU, the simulation and the
navmesh within a per-frame budget. Behind a loading screen these budgets are
wide; during play they are small and paced to the frame rate.

Every request carries a worldspace generation, so work started for a previous
world is discarded instead of corrupting the new one.

## What cells provide

- **Colliders.** A full-detail cell keeps its colliders while it is resident. A
  collision ring narrower than the render ring adds and removes them in the
  [simulation](simulation.md) as commands.
- **Navmeshes.** A full-detail cell's navmeshes join the
  [navmesh](navmesh.md) residency and leave it when the cell is released.
- **Grass.** Workers place grass for each full-detail exterior cell, and it is
  removed with the cell.

## Texture streaming

Texture streaming (`texture_streaming/`) keeps each texture resident at the
detail level that matches how much of the screen its surfaces cover. Cells only
add references to textures; uploads happen within a per-frame budget, and
placeholders show until a load finishes. Detail increases immediately, while
decreases are delayed so textures don't switch back and forth.

## Switching worlds

Entering an interior or changing worldspace releases every resident cell,
clears the navmeshes, discards outstanding requests and restarts loading at the
destination. An interior is loaded as a single cell.

## Readiness

The streamer reports once when the full-detail ring is complete. The load gate
in [game flow](game-flow.md) waits for that report.
