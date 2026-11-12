//! Per-frame rewrite of the instance rows that moving entities own: simulated
//! clutter (rigid poses from the sim) and animated objects (joint poses from
//! the animation tick) share one write path.

use glam::{Mat4, Quat, Vec3};
use tracing::info_span;

#[cfg(test)]
use super::index_staged_rows;
use super::{
    Aabb, CellObjectGroup, CellResources, DynRow, POSE_EPS_BASIS, POSE_EPS_TRANSLATION,
    PendingHide, Ring, StreamTargets, WorldStreamer, instance_aabb, world_aabb,
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
            // A staged cell has no slab range yet: only its CPU mirror moves, and promotion
            // uploads it.
            let res = match self.loaded.get_mut(&grid) {
                Some(res) => res,
                None => match self.staging.get_mut(Ring::Full, grid) {
                    Some(staged) => &mut staged.res,
                    None => continue,
                },
            };
            let slab = res.instance_range;
            self.pose_writes.clear();
            let pose_indices = cell_run.iter().map(|&(_, pose_index)| pose_index);
            let invalidations = &mut self.shadow_invalidations;
            rewrite_cell_rows(res, poses, pose_indices, &mut self.pose_writes, |bounds| {
                invalidations.region(bounds);
            });
            if let Some((slab_offset, _)) = slab {
                self.flush_pose_writes(slab_offset, targets);
            }
        }
        self.pose_order = order;
    }

    /// Whether `remove_world_item` queued a hide that `apply_pending_hides` has not written yet.
    #[expect(
        dead_code,
        reason = "lane E1 wires item pickup into the frame in rendering/mod.rs"
    )]
    pub(crate) fn has_pending_hides(&self) -> bool {
        !self.pending_hides.is_empty()
    }

    /// Hides the rows of every item removed since the last call (CPU mirror and, once the cell
    /// has a slab range, the slab; rows are never deleted), then forgets the entities' rows. A
    /// hide whose cell left or was reloaded since has nothing left to hide.
    #[expect(
        dead_code,
        reason = "lane E1 wires item pickup into the frame in rendering/mod.rs"
    )]
    pub(crate) fn apply_pending_hides(&mut self, targets: &mut StreamTargets) {
        let _span = info_span!("apply_pending_hides").entered();
        let mut hides = std::mem::take(&mut self.pending_hides);
        for (index, hide) in hides.as_slice().iter().enumerate() {
            let res = match self.loaded.get_mut(&hide.grid) {
                Some(res) => res,
                None => match self.staging.get_mut(Ring::Full, hide.grid) {
                    Some(staged) => &mut staged.res,
                    None => continue,
                },
            };
            if res.residency != hide.residency {
                continue;
            }
            let slab = res.instance_range;
            self.pose_writes.clear();
            let invalidations = &mut self.shadow_invalidations;
            hide_entity_rows(
                res,
                hides.as_slice(),
                index,
                &mut self.pose_writes,
                |bounds| {
                    invalidations.region(bounds);
                },
            );
            if self.entity_cell.get(&hide.entity) == Some(&hide.grid) {
                self.entity_cell.remove(&hide.entity);
            }
            if let Some((slab_offset, _)) = slab {
                self.flush_pose_writes(slab_offset, targets);
            }
        }
        hides.clear();
        self.pending_hides = hides;
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

/// Move the rows of `res` that the poses at `pose_indices` place: its CPU mirror always, and,
/// once the cell has a slab range, queue the slab writes into `writes`. `moved` gets the bounds
/// each moved body touched.
fn rewrite_cell_rows<P: PoseSource + ?Sized>(
    res: &mut CellResources,
    poses: &P,
    pose_indices: impl Iterator<Item = usize>,
    writes: &mut Vec<(u32, GpuInstance)>,
    mut moved: impl FnMut(Aabb),
) {
    let CellResources {
        object_groups,
        dynamic_instances,
        group_base,
        instance_range,
        ..
    } = res;
    let mut slots = SlabSlots {
        object_groups,
        group_base,
        slab_count: instance_range.map(|(_, count)| count),
    };
    for pose_index in pose_indices {
        let Some(rows) = dynamic_instances.get_mut(&poses.entity(pose_index)) else {
            continue;
        };
        let Some(pose) = poses.pose(pose_index) else {
            continue;
        };
        if let Some(bounds) = rewrite_rows(rows, &pose, &mut slots, writes) {
            moved(bounds);
        }
    }
}

/// Collapse the rows of the entity `hides[index]` names and drop them from the cell's
/// dynamic rows, so no later pose moves them again.
fn hide_entity_rows(
    res: &mut CellResources,
    hides: &[PendingHide],
    index: usize,
    writes: &mut Vec<(u32, GpuInstance)>,
    moved: impl FnMut(Aabb),
) {
    rewrite_cell_rows(res, hides, std::iter::once(index), writes, moved);
    res.dynamic_instances.remove(&hides[index].entity);
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
    fn a_staged_cell_moves_its_rows_in_the_cpu_mirror_only() {
        use bytemuck::Zeroable;

        use crate::sim::{DynamicPose, EntityId};

        let group = |instances: usize| CellObjectGroup {
            mega_key: "mesh".into(),
            instances: vec![GpuInstance::zeroed(); instances],
        };
        let body = EntityId(7);
        let mut res = CellResources {
            object_groups: vec![group(2), group(1)],
            dynamic_instances: [(
                body,
                vec![DynRow {
                    group_idx: 1,
                    ..row(1.0, Mat4::IDENTITY)
                }],
            )]
            .into(),
            ..CellResources::default()
        };
        index_staged_rows(&mut res);
        let poses = [DynamicPose {
            entity: body,
            translation: Vec3::new(5.0, 6.0, 7.0),
            rotation: Quat::IDENTITY,
        }];
        let mut writes = Vec::new();
        let mut moves = 0;

        rewrite_cell_rows(&mut res, &poses[..], 0..1, &mut writes, |_| moves += 1);

        let moved = Mat4::from_cols_array_2d(&res.object_groups[1].instances[0].transform);
        assert_eq!(moved.w_axis.truncate(), Vec3::new(5.0, 6.0, 7.0));
        assert_eq!(moves, 1);
        assert!(
            writes.is_empty(),
            "a staged cell has no slab range to write"
        );
    }

    fn cell_with_two_resting_bodies(slab_count: Option<u32>) -> CellResources {
        use bytemuck::Zeroable;

        use crate::sim::EntityId;

        let mut instances = vec![GpuInstance::zeroed(); 3];
        for instance in &mut instances {
            instance.transform =
                Mat4::from_translation(Vec3::new(3.0, 1.0, 2.0)).to_cols_array_2d();
        }
        let mut res = CellResources {
            object_groups: vec![CellObjectGroup {
                mega_key: "mesh".into(),
                instances,
            }],
            dynamic_instances: [
                (
                    EntityId(11),
                    vec![
                        DynRow {
                            instance_idx: 0,
                            ..row(1.0, Mat4::IDENTITY)
                        },
                        DynRow {
                            instance_idx: 2,
                            ..row(1.0, Mat4::IDENTITY)
                        },
                    ],
                ),
                (
                    EntityId(12),
                    vec![DynRow {
                        instance_idx: 1,
                        ..row(1.0, Mat4::IDENTITY)
                    }],
                ),
            ]
            .into(),
            instance_range: slab_count.map(|count| (40, count)),
            ..CellResources::default()
        };
        index_staged_rows(&mut res);
        res
    }

    fn hide_of(entity: u64, at: Vec3) -> PendingHide {
        PendingHide {
            entity: crate::sim::EntityId(entity),
            grid: (0, 0),
            residency: 1,
            at,
        }
    }

    #[test]
    fn a_hidden_item_collapses_every_row_to_its_position_without_any_body_pose() {
        use crate::sim::EntityId;

        let mut res = cell_with_two_resting_bodies(Some(3));
        let at = Vec3::new(-4.0, 0.5, 9.0);
        let hides = [hide_of(11, at)];
        let mut writes = Vec::new();
        let mut moves = 0;

        hide_entity_rows(&mut res, &hides, 0, &mut writes, |_| moves += 1);

        for slot in [0, 2] {
            let hidden = &res.object_groups[0].instances[slot];
            let transform = Mat4::from_cols_array_2d(&hidden.transform);
            assert_eq!(transform.w_axis.truncate(), at);
            assert_eq!(transform.transform_vector3(Vec3::ONE), Vec3::ZERO);
            assert_eq!(hidden.aabb_min, (at - Vec3::ONE).to_array());
        }
        let mut written: Vec<u32> = writes.iter().map(|&(local, _)| local).collect();
        written.sort_unstable();
        assert_eq!(written, vec![0, 2]);
        assert_eq!(moves, 1);
        assert!(!res.dynamic_instances.contains_key(&EntityId(11)));
        assert!(res.dynamic_instances.contains_key(&EntityId(12)));
        let untouched = Mat4::from_cols_array_2d(&res.object_groups[0].instances[1].transform);
        assert_eq!(untouched.w_axis.truncate(), Vec3::new(3.0, 1.0, 2.0));
    }

    #[test]
    fn a_hidden_item_of_a_staged_cell_moves_only_the_cpu_mirror() {
        let mut res = cell_with_two_resting_bodies(None);
        let at = Vec3::new(1.0, 2.0, 3.0);
        let mut writes = Vec::new();

        hide_entity_rows(&mut res, &[hide_of(12, at)], 0, &mut writes, |_| {});

        let hidden = Mat4::from_cols_array_2d(&res.object_groups[0].instances[1].transform);
        assert_eq!(hidden.w_axis.truncate(), at);
        assert!(writes.is_empty());
    }

    #[test]
    fn a_body_pose_after_the_hide_no_longer_moves_the_rows() {
        use crate::sim::{DynamicPose, EntityId};

        let mut res = cell_with_two_resting_bodies(Some(3));
        let at = Vec3::new(1.0, 2.0, 3.0);
        hide_entity_rows(&mut res, &[hide_of(11, at)], 0, &mut Vec::new(), |_| {});
        let poses = [DynamicPose {
            entity: EntityId(11),
            translation: Vec3::new(50.0, 0.0, 0.0),
            rotation: Quat::IDENTITY,
        }];
        let mut writes = Vec::new();

        rewrite_cell_rows(&mut res, &poses[..], 0..1, &mut writes, |_| {});

        assert!(writes.is_empty());
        let hidden = Mat4::from_cols_array_2d(&res.object_groups[0].instances[0].transform);
        assert_eq!(hidden.w_axis.truncate(), at);
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
