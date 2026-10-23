//! Per-frame rewrite of the instance rows that moving entities own: simulated
//! clutter (rigid poses from the sim) and animated objects (joint poses from
//! the animation tick) share one write path.

use glam::{Mat4, Quat, Vec3};
use tracing::info_span;

use super::{
    Aabb, CellObjectGroup, CellResources, DynRow, POSE_EPS_BASIS, POSE_EPS_TRANSLATION,
    StreamTargets, WorldStreamer, instance_aabb, world_aabb,
};
use crate::rendering::gpu_driven::GpuInstance;
use crate::rendering::row_pose::{PoseSource, RowPose};
use crate::sim::DynamicPose;

/// The row's world matrix under `pose`, or `None` when the pose cannot place
/// the row (a joint outside the pose, or a row without a joint under a
/// skeletal pose).
fn row_transform(row: &DynRow, pose: &RowPose) -> Option<Mat4> {
    match pose {
        RowPose::Rigid {
            translation,
            rotation,
        } => Some(
            Mat4::from_scale_rotation_translation(Vec3::splat(row.scale), *rotation, *translation)
                * row.local,
        ),
        RowPose::Hidden { at } => Some(Mat4::from_scale_rotation_translation(
            Vec3::ZERO,
            Quat::IDENTITY,
            *at,
        )),
        RowPose::Skeletal {
            refr_world,
            model_pose,
        } => {
            let joint = row.joint?;
            model_pose
                .get(joint as usize)
                .map(|node| *refr_world * *node)
        }
    }
}

/// Whether `transform` (scale and rotation included) matches the one last
/// written for the row. A NaN `last_transform` never matches.
fn is_unchanged(row: &DynRow, transform: &Mat4) -> bool {
    let last = &row.last_transform;
    last.w_axis.truncate().distance(transform.w_axis.truncate()) <= POSE_EPS_TRANSLATION
        && last.x_axis.abs_diff_eq(transform.x_axis, POSE_EPS_BASIS)
        && last.y_axis.abs_diff_eq(transform.y_axis, POSE_EPS_BASIS)
        && last.z_axis.abs_diff_eq(transform.z_axis, POSE_EPS_BASIS)
}

impl WorldStreamer {
    /// Rewrite the instance rows of every simulating dynamic body with its
    /// freshly-read pose.
    pub(crate) fn apply_dynamic_poses(
        &mut self,
        poses: &[DynamicPose],
        targets: &mut StreamTargets,
    ) {
        let _span = info_span!("apply_dynamic_poses", poses = poses.len()).entered();
        self.write_row_poses(poses, targets);
    }

    /// Rewrite the instance rows of every animated object that moved this frame.
    pub(crate) fn apply_animated_poses(
        &mut self,
        poses: &impl PoseSource,
        targets: &mut StreamTargets,
    ) {
        let _span = info_span!("apply_animated_poses", poses = poses.len()).entered();
        self.write_row_poses(poses, targets);
    }

    /// The CPU mirror (`object_groups`) is updated first so a later repack
    /// rebuilds the slab with the moved pose, then the matching GPU slab slot is
    /// written. Poses are grouped by cell so each cell's flattened group-base
    /// offsets are computed once.
    fn write_row_poses<P: PoseSource + ?Sized>(&mut self, poses: &P, targets: &mut StreamTargets) {
        let mut order = std::mem::take(&mut self.pose_order);
        order.clear();
        for index in 0..poses.len() {
            if let Some(&grid) = self.entity_cell.get(&poses.entity(index)) {
                order.push((grid, index));
            }
        }
        order.sort_unstable_by_key(|&(grid, _)| grid);
        for cell_run in order.chunk_by(|a, b| a.0 == b.0) {
            let grid = cell_run[0].0;
            let Some(res) = self.loaded.get_mut(&grid) else {
                continue;
            };
            let slab = res.instance_range;
            let CellResources {
                object_groups,
                dynamic_instances,
                group_base,
                ..
            } = res;
            self.pose_writes.clear();
            let mut slots = SlabSlots {
                object_groups,
                group_base,
                slab_count: slab.map(|(_, count)| count),
            };
            for &(_, pose_index) in cell_run {
                let Some(rows) = dynamic_instances.get_mut(&poses.entity(pose_index)) else {
                    continue;
                };
                let Some(pose) = poses.pose(pose_index) else {
                    continue;
                };
                if let Some(bounds) = rewrite_rows(rows, &pose, &mut slots, &mut self.pose_writes) {
                    self.shadow_invalidations.region(bounds);
                }
            }
            if let Some((slab_offset, _)) = slab {
                self.flush_pose_writes(slab_offset, targets);
            }
        }
        self.pose_order = order;
    }

    /// Coalesce the queued row writes into contiguous runs, one slab write each.
    fn flush_pose_writes(&mut self, slab_offset: u32, targets: &mut StreamTargets) {
        self.pose_writes.sort_unstable_by_key(|&(local, _)| local);
        let mut start = 0;
        while start < self.pose_writes.len() {
            let first = self.pose_writes[start].0;
            self.pose_run.clear();
            let mut end = start;
            while end < self.pose_writes.len()
                && self.pose_writes[end].0 == first + (end - start) as u32
            {
                self.pose_run.push(self.pose_writes[end].1);
                end += 1;
            }
            targets
                .object_instances
                .write(slab_offset + first, &self.pose_run, targets.queue);
            start = end;
        }
    }
}

/// The slab layout of one cell, needed to resolve a row to its instance slot.
/// Without a slab range (not allocated yet) only the CPU mirror is updated, and
/// the repack that assigns the range uploads the moved poses.
struct SlabSlots<'a> {
    object_groups: &'a mut [CellObjectGroup],
    group_base: &'a [u32],
    slab_count: Option<u32>,
}

/// Update the CPU mirror of one entity's rows and queue their slab writes;
/// returns the union of the old and new bounds of every row that moved.
fn rewrite_rows(
    rows: &mut [DynRow],
    pose: &RowPose,
    slots: &mut SlabSlots<'_>,
    writes: &mut Vec<(u32, GpuInstance)>,
) -> Option<Aabb> {
    let mut body_bounds: Option<Aabb> = None;
    for row in rows {
        let Some(transform) = row_transform(row, pose) else {
            continue;
        };
        if is_unchanged(row, &transform) {
            continue;
        }
        // Resolve the slab slot defensively: every index is bounds-checked and
        // the write is confined to this cell's reserved range, so a stale row
        // (or a shrunk slab) skips rather than writes out of bounds.
        let Some(&base) = slots.group_base.get(row.group_idx) else {
            continue;
        };
        let local = base + row.instance_idx as u32;
        if slots.slab_count.is_some_and(|count| local >= count) {
            continue;
        }
        let Some(inst) = slots
            .object_groups
            .get_mut(row.group_idx)
            .and_then(|g| g.instances.get_mut(row.instance_idx))
        else {
            continue;
        };
        let (aabb_min, aabb_max) = world_aabb(row.local_aabb.as_ref(), &transform);
        let old = instance_aabb(inst);
        let new = Aabb {
            min: Vec3::from(aabb_min),
            max: Vec3::from(aabb_max),
        };
        let union = Aabb {
            min: old.min.min(new.min),
            max: old.max.max(new.max),
        };
        body_bounds = Some(match body_bounds {
            Some(acc) => Aabb {
                min: acc.min.min(union.min),
                max: acc.max.max(union.max),
            },
            None => union,
        });
        inst.transform = transform.to_cols_array_2d();
        inst.aabb_min = aabb_min;
        inst.aabb_max = aabb_max;
        row.last_transform = transform;
        if slots.slab_count.is_some() {
            writes.push((local, *inst));
        }
    }
    body_bounds
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(scale: f32, local: Mat4) -> DynRow {
        DynRow {
            group_idx: 0,
            instance_idx: 0,
            local_aabb: None,
            scale,
            local,
            joint: None,
            last_transform: Mat4::NAN,
        }
    }

    fn rigid(translation: Vec3) -> RowPose<'static> {
        RowPose::Rigid {
            translation,
            rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        }
    }

    #[test]
    fn a_plain_row_takes_the_body_pose_scaled() {
        let world = row_transform(&row(2.0, Mat4::IDENTITY), &rigid(Vec3::new(10.0, 0.0, 0.0)));
        let moved = world.unwrap().transform_point3(Vec3::X);
        assert!(
            moved.abs_diff_eq(Vec3::new(10.0, 0.0, -2.0), 1e-5),
            "{moved}"
        );
    }

    #[test]
    fn a_weapon_part_row_is_offset_inside_the_scaled_and_rotated_body() {
        let local = Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0));
        let world = row_transform(&row(2.0, local), &rigid(Vec3::new(10.0, 0.0, 0.0)));
        let origin = world.unwrap().transform_point3(Vec3::ZERO);
        assert!(
            origin.abs_diff_eq(Vec3::new(10.0, 0.0, -2.0), 1e-5),
            "{origin}"
        );
    }
}
