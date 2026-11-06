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

## Carrying physics objects

The player can pick up a loose dynamic body and carry it in front of the
camera. The rules are pure functions in `sim/carry.rs`, the rapier side is
`sim/physics/carried.rs`, and the key handling lives on the render side in
`rendering/grab_input.rs`. Behaviour and values follow measurements of vanilla.

### Controls

- Holding E on a loose body for 0.16 s grabs it. The grab latches, so E can be
  let go afterwards. A shorter press does nothing.
- Pressing E while carrying drops the body when the key is let go.
- R charges a throw while it is held and throws when it is let go.
- The left and right mouse buttons spin the body while held. Both held cancel
  out.
- A door or animated object under the crosshair still activates on the E press,
  without waiting for the grab hold. While carrying, E drops the body instead.

E and R only count in walk mode during gameplay, with the Pip-Boy and the
loading-screen preview closed. Key repeats and the synthetic key events a focus
change produces are ignored. Leaving these conditions drops the body; losing
window focus does not.

### Carrying the body

The carried body stays an ordinary dynamic body, so walls and other bodies stop
it. Before each physics step a velocity servo sets its linear and angular
velocity so the step brings it onto its hold pose. The servo aims at the eye
position of the coming player step, so the body does not trail the player by a
step. Against something immovable the servo only presses the body on gently, so
it rests there without chatter and keeps following its hold pose along the
obstacle. The body stays dynamic while held, even if its cell leaves the dynamic
radius, and it cannot fall asleep.

The hold pose is camera-relative. The distance from the eye is taken at grab
time, less a measured inset, and the body's orientation relative to the camera
is kept, with the spin applied on top. The mouse buttons turn the body about the
body-local axis nearest the camera's right. That axis rule is provisional: it
fits an upright object, and which axis vanilla picks is not established.

### Values

Measured against vanilla, as a bracket where the measurement only bounds the
value:

| Value | Measured | Used |
|---|---|---|
| E hold that grabs | 150 ms does not, 175 ms does | 0.16 s |
| Mass from which carrying forces walking | 70 runs, 80 walks | 75 |
| Mass above which a grab is refused | 100 lifts, 120 refuses | 110 |
| Separation that strains the grip | 150 to 200 units | 175 units |
| Strain time before the body is torn away | 0.6 to 1.1 s | 0.8 s |
| Spin rate | about 172 deg/s | 172 deg/s |
| Throw speed | linear in the charge time, then a plateau | 220 units/s plus 783 units/s per second of charge, capped at 1470 units/s |

The throw speed is the same for every mass. The walking speed forced by a heavy
body is the walk speed of the `Player_Default_MT` movement type, which also
overrides sprinting.

The speed of a player-driven body is capped once, at the throw plateau. Shoves,
the carry servo and throws share that cap, which stays within the distance
soft-CCD can catch in one step.

### Releasing

Every way of letting go goes through one release that restores the body's own
sleep thresholds, keeps its velocity and records the reason.

| Reason | Trigger |
|---|---|
| `Dropped` | E tap, or the controls going dead (walk mode left, gameplay input locked, loading-screen preview opened) |
| `Thrown` | R let go |
| `TornAway` | The body stays beyond the break separation for the break time |
| `CellUnloaded` | The cell owning the body is removed |
| `Teleported` | The player is spawned or teleported |
| `MenuOpened` | The Pip-Boy opens |
| `BodyLost` | The body left the world, for example quarantined by the physics audit |
| `InvalidHold` | The hold pose could not be computed from non-finite eye or view input |

A grab can also be refused: already carrying, not a loose simulating body, too
heavy, player not placed yet, or the simulation not stepping. The refusal is
read back like a release.

### Player isolation

The carried body never blocks, supports or carries the player and is never
shoved by it: the player's dynamic-contact pass, the support probe, the
static-scene queries and the footstep ground probe all ignore it. As in vanilla,
walking it into a wall brings the player right up to the wall, with the body
squeezed between them and still held.

Out of scope: carrying ragdolls and a HUD prompt for the grab.

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
