//! What the object under the crosshair does when activated.

use std::collections::HashMap;

use super::ResolvedTarget;
use super::animated_objects::AnimatedObjects;
use crate::sim::{EntityId, refr_entity_id};

pub(super) enum Activation<'a> {
    /// A load door: enter its destination.
    Teleport(&'a ResolvedTarget),
    /// An animated object: toggle its open/close (or on/off) clip.
    Animate(EntityId),
}

/// A load door wins over animation: it must teleport, not swing.
pub(super) fn resolve<'a>(
    ref_id: &str,
    teleports: &'a HashMap<String, ResolvedTarget>,
    animated: &AnimatedObjects,
) -> Option<Activation<'a>> {
    if let Some(target) = teleports.get(ref_id) {
        return Some(Activation::Teleport(target));
    }
    let entity = refr_entity_id(ref_id);
    animated
        .contains(entity)
        .then_some(Activation::Animate(entity))
}
