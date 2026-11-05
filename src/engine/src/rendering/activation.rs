//! What the object under the crosshair does when activated.

use std::collections::HashMap;

use thiserror::Error;

use super::ResolvedTarget;
use super::animated_objects::AnimatedObjects;
use crate::sim::{EntityId, refr_entity_id};

pub(super) enum Activation {
    /// A load door: enter its destination.
    Teleport,
    /// An animated object: toggle its open/close (or on/off) clip.
    Animate(EntityId),
}

/// A load door wins over animation: it must teleport, not swing.
pub(super) fn resolve(
    ref_id: &str,
    teleports: &HashMap<String, ResolvedTarget>,
    animated: &AnimatedObjects,
) -> Option<Activation> {
    if teleports.contains_key(ref_id) {
        return Some(Activation::Teleport);
    }
    let entity = refr_entity_id(ref_id);
    animated
        .contains(entity)
        .then_some(Activation::Animate(entity))
}

/// Why a teleport request was not queued.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(super) enum TeleportRefusal {
    #[error("a loading screen is up")]
    TransitionActive,
    #[error("the player is not spawned yet")]
    PlayerNotSpawned,
    #[error("another teleport is already pending")]
    TeleportPending,
    #[error("ref {0} is not a load door")]
    UnknownSource(String),
}

/// Whether a teleport request may be queued now. A running load comes first: whatever else is
/// the case, the request is stale. Spawning is checked next, since a request made while the
/// player is still loading in is stale whatever else is queued.
pub(super) fn teleport_gate(
    transition_active: bool,
    player_spawned: bool,
    teleport_pending: bool,
) -> Result<(), TeleportRefusal> {
    if transition_active {
        return Err(TeleportRefusal::TransitionActive);
    }
    if !player_spawned {
        return Err(TeleportRefusal::PlayerNotSpawned);
    }
    if teleport_pending {
        return Err(TeleportRefusal::TeleportPending);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawned_player_without_pending_teleport_may_teleport() {
        assert_eq!(teleport_gate(false, true, false), Ok(()));
    }

    #[test]
    fn unspawned_player_is_refused() {
        assert_eq!(
            teleport_gate(false, false, false),
            Err(TeleportRefusal::PlayerNotSpawned)
        );
    }

    #[test]
    fn pending_teleport_blocks_a_second_request() {
        assert_eq!(
            teleport_gate(false, true, true),
            Err(TeleportRefusal::TeleportPending)
        );
    }

    #[test]
    fn unspawned_player_is_reported_before_a_pending_teleport() {
        assert_eq!(
            teleport_gate(false, false, true),
            Err(TeleportRefusal::PlayerNotSpawned)
        );
    }

    #[test]
    fn a_running_load_refuses_the_request_even_for_a_spawned_player() {
        assert_eq!(
            teleport_gate(true, true, false),
            Err(TeleportRefusal::TransitionActive)
        );
    }

    #[test]
    fn a_running_load_is_reported_before_every_other_reason() {
        for (spawned, pending) in [(false, false), (false, true), (true, true)] {
            assert_eq!(
                teleport_gate(true, spawned, pending),
                Err(TeleportRefusal::TransitionActive),
                "spawned {spawned}, pending {pending}"
            );
        }
    }
}
