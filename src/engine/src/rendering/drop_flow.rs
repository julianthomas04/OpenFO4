//! What becomes of the units of a dropped stack: the world takes it or the units go back to the
//! inventory. The world and the inventory come in as closures, so none of it needs a GPU.

use std::fmt::Display;

use super::drop_log::drop_failure_line;
use super::world_streamer::{AbandonedDrop, DroppedItemSpec, SpawnItemError};
use crate::sim::{EntityId, ItemGrantError};

/// Hands a dropped stack to the world. When the world refuses it the whole count goes back to
/// the inventory. Returns whether the world took it.
pub(super) fn settle_drop(
    spec: DroppedItemSpec,
    spawn: impl FnOnce(DroppedItemSpec) -> Result<EntityId, SpawnItemError>,
    mut restore: impl FnMut(&str, u32) -> Result<(), ItemGrantError>,
) -> bool {
    let (form_key, count) = (spec.base.clone(), spec.count);
    match spawn(spec) {
        Ok(_) => true,
        Err(error) => {
            let returned = restore(&form_key, count);
            let reason = format!("refused by the world: {error}");
            tracing::error!(
                "{}",
                drop_failure_line(&form_key, count, &reason, &returned)
            );
            false
        }
    }
}

/// Gives back the units of every drop the world never took.
pub(super) fn return_abandoned(
    abandoned: Vec<AbandonedDrop>,
    mut restore: impl FnMut(&str, u32) -> Result<(), ItemGrantError>,
) {
    for drop in abandoned {
        let returned = restore(&drop.base, drop.count);
        let line = drop_failure_line(&drop.base, drop.count, &drop.reason, &returned);
        if returned.is_ok() {
            tracing::warn!("{line}");
        } else {
            tracing::error!("{line}");
        }
    }
}

/// Runs `handle` on every event, whatever an earlier one did. Every error is reported; the first
/// is returned after the last event.
pub(super) fn handle_all<T, E: Display>(
    events: impl IntoIterator<Item = T>,
    mut handle: impl FnMut(T) -> Result<(), E>,
) -> Result<(), E> {
    let mut first_error = None;
    for event in events {
        if let Err(error) = handle(event) {
            if first_error.is_some() {
                tracing::error!("a further inventory event failed as well: {error}");
            } else {
                first_error = Some(error);
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;

    const KEY: &str = "0F3B01:Test.esm";

    fn spec(count: u32) -> DroppedItemSpec {
        DroppedItemSpec {
            base: KEY.to_owned(),
            count,
            position: Vec3::ZERO,
            yaw: 0.0,
        }
    }

    #[derive(Default)]
    struct Restored(Vec<(String, u32)>);

    impl Restored {
        fn restore(&mut self, key: &str, count: u32) {
            self.0.push((key.to_owned(), count));
        }
    }

    #[test]
    fn a_drop_the_world_took_keeps_its_units_out_of_the_inventory() {
        let mut restored = Restored::default();
        let took = settle_drop(
            spec(30),
            |_| Ok(EntityId(1)),
            |key, count| {
                restored.restore(key, count);
                Ok(())
            },
        );
        assert!(took);
        assert_eq!(restored.0, Vec::new());
    }

    #[test]
    fn a_drop_the_world_refuses_returns_the_whole_count_whatever_the_reason() {
        let refusals = [
            SpawnItemError::NotPickable(KEY.to_owned()),
            SpawnItemError::NoUnits(KEY.to_owned()),
            SpawnItemError::NoOwnerCell(Vec3::ONE),
            SpawnItemError::WorkerChannelClosed,
        ];
        for refusal in refusals {
            let mut restored = Restored::default();
            let described = refusal.to_string();
            let took = settle_drop(
                spec(30),
                |_| Err(refusal),
                |key, count| {
                    restored.restore(key, count);
                    Ok(())
                },
            );
            assert!(!took, "{described}");
            assert_eq!(restored.0, [(KEY.to_owned(), 30)], "{described}");
        }
    }

    #[test]
    fn the_world_is_offered_the_spec_it_was_given() {
        let mut offered = None;
        settle_drop(
            spec(7),
            |spec| {
                offered = Some(spec);
                Ok(EntityId(1))
            },
            |_, _| Ok(()),
        );
        assert_eq!(offered, Some(spec(7)));
    }

    #[test]
    fn a_failing_give_back_does_not_turn_a_refusal_into_a_taken_drop() {
        let took = settle_drop(
            spec(3),
            |_| Err(SpawnItemError::WorkerChannelClosed),
            |_, _| {
                Err(ItemGrantError::UnknownItem {
                    form_key: KEY.to_owned(),
                })
            },
        );
        assert!(!took);
    }

    #[test]
    fn every_abandoned_drop_returns_all_of_its_units_in_order() {
        let mut restored = Restored::default();
        return_abandoned(
            vec![
                AbandonedDrop {
                    base: KEY.to_owned(),
                    count: 30,
                    reason: "its cell left before the item was built".to_owned(),
                },
                AbandonedDrop {
                    base: "0F3B02:Test.esm".to_owned(),
                    count: 1,
                    reason: "its payload holds no body".to_owned(),
                },
            ],
            |key, count| {
                restored.restore(key, count);
                Ok(())
            },
        );
        assert_eq!(
            restored.0,
            [(KEY.to_owned(), 30), ("0F3B02:Test.esm".to_owned(), 1)]
        );
    }

    #[test]
    fn an_early_error_does_not_drop_the_remaining_events() {
        let mut handled = Vec::new();
        let result = handle_all([1, 2, 3, 4], |event| {
            handled.push(event);
            if event == 2 || event == 3 {
                Err(format!("event {event}"))
            } else {
                Ok(())
            }
        });
        assert_eq!(handled, [1, 2, 3, 4]);
        assert_eq!(result, Err("event 2".to_owned()));
    }

    #[test]
    fn events_that_all_succeed_give_ok() {
        let result: Result<(), String> = handle_all([1, 2], |_| Ok(()));
        assert_eq!(result, Ok(()));
    }
}
