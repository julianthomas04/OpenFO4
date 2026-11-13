//! A diagnostic trace of what the crosshair ray hits and how the aim code classifies it. Logged at
//! trace level under its own target, only when the aimed ref or the target kind changes.

use super::grab_input::AimTarget;
use crate::sim::RayHit;

pub(super) const AIM_TRACE_TARGET: &str = "openfo4::aim_trace";

fn target_name(target: AimTarget) -> String {
    match target {
        AimTarget::Nothing => "Nothing".to_owned(),
        AimTarget::Activatable => "Activatable".to_owned(),
        AimTarget::Grabbable(_) => "Grabbable".to_owned(),
        AimTarget::Item { body, .. } => format!("Item({body:?})"),
    }
}

fn aim_line(ref_id: Option<&str>, target: &str, dist: Option<f32>) -> String {
    format!(
        "aim: ref={} target={target} dist={}",
        ref_id.unwrap_or("none"),
        dist.map_or_else(|| "none".to_owned(), |dist| format!("{dist:.2}")),
    )
}

/// Remembers the last traced aim so only changes produce a line.
#[derive(Default)]
pub(super) struct AimTrace {
    last: Option<(Option<String>, String)>,
}

impl AimTrace {
    /// The line for this frame's aim, or `None` when the ref and the target kind are unchanged.
    pub(super) fn observe(&mut self, hit: Option<&RayHit>, target: AimTarget) -> Option<String> {
        let ref_id = hit.and_then(|hit| hit.ref_id.clone());
        let target = target_name(target);
        let key = (ref_id, target);
        if self.last.as_ref() == Some(&key) {
            return None;
        }
        let line = aim_line(key.0.as_deref(), &key.1, hit.map(|hit| hit.dist));
        self.last = Some(key);
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::super::grab_input::ItemBody;
    use super::*;
    use crate::sim::EntityId;

    fn hit(ref_id: Option<&str>, dist: f32) -> RayHit {
        RayHit {
            mesh_path: "meshes_x.glb".to_owned(),
            ref_id: ref_id.map(str::to_owned),
            dist,
        }
    }

    const ITEM: AimTarget = AimTarget::Item {
        entity: EntityId(5),
        body: ItemBody::Carriable,
    };

    #[test]
    fn the_line_names_ref_target_kind_and_distance() {
        let mut trace = AimTrace::default();
        assert_eq!(
            trace.observe(Some(&hit(Some("000001:*dropped"), 10.345)), ITEM),
            Some("aim: ref=000001:*dropped target=Item(Carriable) dist=10.35".to_owned())
        );
    }

    #[test]
    fn a_ray_into_nothing_logs_none_for_ref_and_distance() {
        let mut trace = AimTrace::default();
        assert_eq!(
            trace.observe(None, AimTarget::Nothing),
            Some("aim: ref=none target=Nothing dist=none".to_owned())
        );
    }

    #[test]
    fn an_unchanged_ref_and_target_logs_once_whatever_the_distance() {
        let mut trace = AimTrace::default();
        assert!(trace.observe(Some(&hit(Some("A:x"), 1.0)), ITEM).is_some());
        assert!(trace.observe(Some(&hit(Some("A:x"), 2.0)), ITEM).is_none());
    }

    #[test]
    fn a_new_ref_or_a_new_target_kind_logs_again() {
        let mut trace = AimTrace::default();
        trace.observe(Some(&hit(Some("A:x"), 1.0)), ITEM);
        assert!(trace.observe(Some(&hit(Some("B:x"), 1.0)), ITEM).is_some());
        assert!(
            trace
                .observe(
                    Some(&hit(Some("B:x"), 1.0)),
                    AimTarget::Grabbable(EntityId(5))
                )
                .is_some()
        );
    }
}
