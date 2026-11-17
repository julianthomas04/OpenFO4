//! The collision boxes of item bases worked out so far, shared by the thread pool that prepares
//! them and the main thread that places drops.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard};

use super::world_edits::ItemBoxError;
use crate::sim::collision_box::CollisionBox;
use crate::utils::warn_once;

type ItemBox = Result<CollisionBox, ItemBoxError>;

#[derive(Default)]
struct Entries {
    known: HashMap<String, ItemBox>,
    /// Bases a preparation was handed out for that has not finished yet.
    preparing: HashSet<String>,
}

#[derive(Default)]
pub(super) struct ItemBoxes(Mutex<Entries>);

impl ItemBoxes {
    pub fn is_known(&self, form_key: &str) -> bool {
        self.lock().known.contains_key(form_key)
    }

    /// Whether the caller should prepare the box of `form_key`: true once, until it is known.
    pub fn claim_preparation(&self, form_key: &str) -> bool {
        let mut entries = self.lock();
        !entries.known.contains_key(form_key) && entries.preparing.insert(form_key.to_owned())
    }

    /// The box of `form_key`, worked out by `compute` unless known; the lock is not held while it
    /// runs, so a caller that finds a preparation still running works it out itself.
    pub fn get_or_compute(&self, form_key: &str, compute: impl FnOnce() -> ItemBox) -> ItemBox {
        if let Some(known) = self.lock().known.get(form_key) {
            return known.clone();
        }
        let computed = compute();
        let mut entries = self.lock();
        entries.preparing.remove(form_key);
        entries
            .known
            .entry(form_key.to_owned())
            .or_insert(computed)
            .clone()
    }

    fn lock(&self) -> MutexGuard<'_, Entries> {
        self.0.lock().unwrap_or_else(|poisoned| {
            warn_once("item-box-cache-poisoned", || {
                "a thread panicked while holding the item collision boxes; continuing with them"
                    .to_owned()
            });
            poisoned.into_inner()
        })
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;

    const KEY: &str = "0F3B01:Test.esm";
    const BOX: CollisionBox = CollisionBox {
        min: Vec3::ZERO,
        max: Vec3::ONE,
    };

    #[test]
    fn a_box_is_worked_out_once_and_then_known() {
        let boxes = ItemBoxes::default();
        assert!(!boxes.is_known(KEY));
        assert_eq!(boxes.get_or_compute(KEY, || Ok(BOX)), Ok(BOX));
        assert!(boxes.is_known(KEY));
        let again = boxes.get_or_compute(KEY, || panic!("worked out a second time"));
        assert_eq!(again, Ok(BOX));
    }

    #[test]
    fn a_base_without_a_box_keeps_its_error() {
        let boxes = ItemBoxes::default();
        let missing = Err(ItemBoxError::NoCollision(KEY.to_owned()));
        assert_eq!(boxes.get_or_compute(KEY, || missing.clone()), missing);
        assert_eq!(boxes.get_or_compute(KEY, || Ok(BOX)), missing);
    }

    #[test]
    fn a_base_is_handed_out_for_preparation_once_until_it_is_known() {
        let boxes = ItemBoxes::default();
        assert!(boxes.claim_preparation(KEY));
        assert!(!boxes.claim_preparation(KEY));
        boxes.get_or_compute(KEY, || Ok(BOX)).unwrap();
        assert!(!boxes.claim_preparation(KEY));
    }

    #[test]
    fn a_caller_finding_a_preparation_running_works_the_box_out_itself() {
        let boxes = ItemBoxes::default();
        assert!(boxes.claim_preparation(KEY));
        assert_eq!(boxes.get_or_compute(KEY, || Ok(BOX)), Ok(BOX));
        let late = CollisionBox {
            max: Vec3::splat(2.0),
            ..BOX
        };
        assert_eq!(boxes.get_or_compute(KEY, || Ok(late)), Ok(BOX));
    }
}
