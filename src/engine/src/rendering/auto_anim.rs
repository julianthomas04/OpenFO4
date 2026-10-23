//! Dev hook (`OPENFO4_AUTO_ANIM="ref1,ref2,..."`): once the player has spawned,
//! toggle every listed REFR's animation at a fixed interval. Exercises the
//! open/close path where OS input cannot be injected into the window.

use std::collections::HashSet;

use tracing::{info, warn};

use super::animated_objects::{ActivationError, AnimatedObjects};
use crate::sim::refr_entity_id;

const ENV_VAR: &str = "OPENFO4_AUTO_ANIM";
const TOGGLE_INTERVAL_SECONDS: f32 = 3.0;

pub(super) struct AutoAnim {
    refs: Vec<String>,
    elapsed: f32,
    /// Refs already reported as not resident, so the retry every interval
    /// stays quiet until the object streams in.
    reported_absent: HashSet<String>,
}

impl AutoAnim {
    pub fn from_env() -> Self {
        let refs = crate::utils::optional_env_var(ENV_VAR)
            .map(|list| {
                list.split(',')
                    .map(|key| key.trim().to_owned())
                    .filter(|key| !key.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Self {
            refs,
            elapsed: 0.0,
            reported_absent: HashSet::new(),
        }
    }

    /// `ready` reports whether the player has spawned into loaded content; it is
    /// only consulted when refs are configured.
    pub fn tick(&mut self, dt: f32, ready: impl FnOnce() -> bool, objects: &mut AnimatedObjects) {
        if self.refs.is_empty() || !ready() {
            return;
        }
        self.elapsed += dt;
        if self.elapsed < TOGGLE_INTERVAL_SECONDS {
            return;
        }
        self.elapsed = 0.0;
        for key in &self.refs {
            match objects.toggle(refr_entity_id(key)) {
                Ok(()) => info!("{ENV_VAR}: toggled {key}"),
                Err(ActivationError::NotResident(_)) => {
                    if self.reported_absent.insert(key.clone()) {
                        info!(
                            "{ENV_VAR}: {key} is not resident; retrying every {TOGGLE_INTERVAL_SECONDS}s"
                        );
                    }
                }
                Err(error) => warn!("{ENV_VAR}: {error}"),
            }
        }
    }
}
