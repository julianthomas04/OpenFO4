# Game flow

`GameFlow` (`game_flow/`) is the top-level state machine of the game: main menu,
spawn selection, loading, the reveal into the world, and gameplay. It consumes
input and readiness signals and returns actions for the caller to carry out.

## Phases

The flow moves through main menu, spawn select, loading, revealing and in game.
Back returns from spawn select to the main menu.

While a menu phase is showing, `State::update` returns early, so nothing loads
or simulates until a spawn is picked.

## Inputs and outputs

The flow receives decoded menu events (`FlowEvent`), a per-frame `Readiness`
feed and the frame time. It answers with a `FlowAction`, such as beginning play,
releasing the physics hold, locking the cursor or quitting, which `State`
carries out.

What to show and where input goes are queries on the flow, derived from the
current phase. The flow does not own the presentation itself.

## Loading

Loading ends through the load gate (`load_gate.rs`). Once the full-detail ring
around the spawn is loaded and no colliders are pending, the physics hold is
released. When the player is then ready on consecutive frames, the reveal
starts. If loading stops making progress, the gate logs which step is blocking.

## Reveal

A black overlay fades out once the full-detail ring is loaded. The fade hides
the lower-detail distant world that is still loading behind it.

## Spawn locations

Each spawn location resolves to a position from a map marker in the database,
or is given explicitly. A missing or ambiguous marker is an error.
