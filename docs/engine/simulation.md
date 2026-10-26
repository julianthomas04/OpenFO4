# Simulation

The simulation (`sim/`) owns physics, the player and gameplay state. Every
change enters it as a `SimCommand`, and the renderer reads poses and snapshots
back out. Physics engine types never leave the module.

## Tick

`SimWorld::tick` advances in fixed steps. Each step runs the water pass, the
physics step, the player and the footstep clock, in that order. The renderer
blends between the last two steps, so motion stays smooth at any frame rate.
Slow frames are capped, so the simulation slows down rather than catching up.

The simulation pauses while the Pip-Boy is open and holds during loading (see
[game flow](game-flow.md)). The physics step is skipped when nothing in the
world is awake or changed.

## World clock

`WorldClock` (`sim/world_clock.rs`) is the game's time: hour, calendar date,
days passed and the time scale. It starts from the game's `GameHour`,
`GameDay`, `GameMonth`, `GameYear`, `GameDaysPassed` and `TimeScale` globals.

The time scale is game minutes per real minute. The clock advances by world
time, so it stands still in menus, while loading and while the Pip-Boy is open.
The step of one frame is capped, so a hitch cannot skip a stretch of the day.
At midnight the date and the days passed roll over; months have their real
lengths and there are no leap years.

The renderer reads the hour for the [time of day](rendering/lighting-and-shadows.md#time-of-day),
and [audio](audio-and-video.md) reads the hour and the days passed as the
`GameHour` and `GameDaysPassed` globals. The Pip-Boy shows the clock's date.

## Colliders

The [world streamer](world-streaming.md) holds each cell's collider payload and
adds it to the simulation as a command. Additions are queued and inserted in
order within a time budget, so a later one never overtakes an earlier one. Once
added, the simulation owns the physics bodies until the cell is removed.

## Player

The player is a kinematic capsule that slides along static geometry. A separate
contact pass lets it push dynamic bodies and stand on them, and an invisible
mirror body wakes sleeping clutter on contact.

## Ragdolls

A ragdoll-capable object starts as static collision. Inside the simulation
radius it becomes one physics body per bone and publishes bone poses for the
renderer. If it never moved, it reverts to static collision when it leaves the
radius.

## Water

`water_world/` is the single water model shared by the renderer and the
simulation, which queries it by position. Each step applies buoyancy, damping
and currents to bodies in water and reports splash events.

## Footsteps

A clock replays the player's gait timing, probes the surface under the feet and
emits footstep events, which [audio](audio-and-video.md) plays.
