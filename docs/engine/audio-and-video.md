# Audio and video

## Audio

`AudioSystem` (`audio/`) follows the [game flow](game-flow.md): it plays menu
music in the menus, nothing while loading, and world music from the reveal on.
If no audio device is available, the game runs silent and is otherwise
unaffected.

World music depends on where the player is and the in-game hour, and crossfades
when the kind of music changes.

Footsteps start in the [simulation](simulation.md), which reports each step. The
engine looks up the surface under the foot and plays a matching sound. Footsteps
and world music only play during gameplay.

## Video

The main menu's background video (`video/`) is decoded by Windows Media
Foundation on its own thread, and the renderer always takes the newest frame.
The video restarts each time the menu opens. Nothing is shipped for it: Media
Foundation is part of Windows and is loaded only when the video starts. On a
Windows N edition without the Media Feature Pack the menu stays without video
and the log says why.
