use crate::assets::db::cells::Cell;
use crate::assets::db::schema::{
    DbSchema, placed_object_light_select_list, placed_object_ragdoll_pose_select_list,
};

pub struct PlacedObject {
    pub form_key: Option<String>,
    pub cell_form_key: Option<String>,
    pub base_object: Option<String>,
    pub object_type: Option<String>,
    pub is_persistent: bool,
    pub position_x: f64,
    pub position_y: f64,
    pub position_z: f64,
    pub rotation_x: f64,
    pub rotation_y: f64,
    pub rotation_z: f64,
    pub scale: f64,
    pub is_initially_disabled: bool,
    pub enable_parent: Option<String>,
    pub enable_parent_opposite: bool,
    pub is_in_precombined: bool,
    pub has_primitive: bool,
    /// MSWP `form_key` from the ref's XMSP subrecord; overrides the base
    /// record's swap (`base_material_swaps`).
    pub material_swap: Option<String>,
    pub extra_data: Option<String>,
    /// Installer-resolved initial enable state: `InitiallyDisabled` flag for
    /// roots, XESP chain verdict for enable-parented refs (parent state XOR
    /// opposite flag). The spawn filter — supersedes checking
    /// `is_initially_disabled` / `enable_parent` directly.
    pub initially_hidden: bool,
    /// XRDS of a light ref: a signed delta on the base LIGH radius (read
    /// from the `radius_override` column).
    pub radius_delta: Option<f64>,
    /// XLIG fields; `None` when the ref carries no XLIG.
    pub light_fov_pm: Option<f64>,
    pub light_fade_pm: Option<f64>,
    pub light_end_distance_cap: Option<f64>,
    pub light_shadow_depth_bias: Option<f64>,
    pub light_near_clip: Option<f64>,
    pub light_volumetric_intensity: Option<f64>,
    /// Raw major record flags (`CastsShadows` 0x200, `NeverFades` 0x10000,
    /// `DoesNotLightLandscape` 0x20000, `DoesNotLightWater` 0x100 on light refs).
    pub record_flags: Option<i64>,
    /// Raw XRGD subrecord: the authored pose of every ragdoll bone, 28 bytes
    /// per bone (`sim::ragdoll_pose` decodes it). `None` when the ref carries
    /// none or the database predates the column.
    pub ragdoll_pose: Option<Vec<u8>>,
}

fn row_to_placed_object(row: &rusqlite::Row) -> rusqlite::Result<PlacedObject> {
    Ok(PlacedObject {
        form_key: row.get(0)?,
        cell_form_key: row.get(1)?,
        base_object: row.get(2)?,
        object_type: row.get(3)?,
        is_persistent: row.get(4)?,
        position_x: row.get(5)?,
        position_y: row.get(6)?,
        position_z: row.get(7)?,
        rotation_x: row.get(8)?,
        rotation_y: row.get(9)?,
        rotation_z: row.get(10)?,
        scale: row.get(11)?,
        is_initially_disabled: row.get(12)?,
        enable_parent: row.get(13)?,
        enable_parent_opposite: row.get(14)?,
        is_in_precombined: row.get(15)?,
        has_primitive: row.get(16)?,
        material_swap: row.get(17)?,
        extra_data: row.get(18)?,
        initially_hidden: row.get(19)?,
        radius_delta: row.get(20)?,
        light_fov_pm: row.get(21)?,
        light_fade_pm: row.get(22)?,
        light_end_distance_cap: row.get(23)?,
        light_shadow_depth_bias: row.get(24)?,
        light_near_clip: row.get(25)?,
        light_volumetric_intensity: row.get(26)?,
        record_flags: row.get(27)?,
        ragdoll_pose: row.get(28)?,
    })
}

const BASE_COLUMNS: &[&str] = &[
    "form_key",
    "cell_form_key",
    "base_object",
    "object_type",
    "is_persistent",
    "position_x",
    "position_y",
    "position_z",
    "rotation_x",
    "rotation_y",
    "rotation_z",
    "scale",
    "is_initially_disabled",
    "enable_parent",
    "enable_parent_opposite",
    "is_in_precombined",
    "has_primitive",
    "material_swap",
    "extra_data",
    "initially_hidden",
];

/// Column list feeding [`row_to_placed_object`], whose positional `row.get`
/// indices follow this order. `prefix` qualifies each column (`"po."`).
fn select_columns(schema: DbSchema, prefix: &str) -> String {
    let base = BASE_COLUMNS
        .iter()
        .map(|column| format!("{prefix}{column}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{base}, {}, {}",
        placed_object_light_select_list(schema, prefix),
        placed_object_ragdoll_pose_select_list(schema, prefix)
    )
}

impl PlacedObject {
    pub fn get(
        cell: &str,
        conn: &rusqlite::Connection,
        schema: DbSchema,
    ) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {} FROM placed_objects WHERE cell_form_key = ?",
            select_columns(schema, "")
        ))?;

        Ok(stmt
            .query_map(rusqlite::params![cell], row_to_placed_object)?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn get_for_cells(
        cells: &[Cell],
        conn: &rusqlite::Connection,
        schema: DbSchema,
    ) -> anyhow::Result<Vec<Self>> {
        let keys: Vec<&str> = cells.iter().filter_map(|c| c.form_key.as_deref()).collect();

        if keys.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: String = keys.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT {} FROM placed_objects WHERE cell_form_key IN ({})",
            select_columns(schema, ""),
            placeholders
        );

        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = keys
            .iter()
            .map(|k| k as &dyn rusqlite::types::ToSql)
            .collect();

        Ok(stmt
            .query_map(params.as_slice(), row_to_placed_object)?
            .collect::<Result<Vec<_>, _>>()?)
    }

    /// Per-interior-cell anchor of its spawnable placements: the per-axis median
    /// of their positions, in Bethesda units `(cell_form_key, x, y, z)`. Used at
    /// boot to precompute each interior's start position. Only objects that
    /// actually spawn (`initially_hidden = 0`) count, and the median rather than
    /// the mean keeps far-flung outliers from dragging the start into empty space.
    /// Interiors with no spawnable object with a complete position are absent
    /// from the result.
    pub fn interior_anchors(
        conn: &rusqlite::Connection,
    ) -> anyhow::Result<Vec<(String, f64, f64, f64)>> {
        let mut stmt = conn.prepare(
            "SELECT po.cell_form_key, po.position_x, po.position_y, po.position_z
             FROM placed_objects po
             JOIN cells c ON c.form_key = po.cell_form_key
             WHERE c.is_interior = 1
               AND po.initially_hidden = 0
               AND po.position_x IS NOT NULL
               AND po.position_y IS NOT NULL
               AND po.position_z IS NOT NULL",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                [
                    row.get::<_, f64>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, f64>(3)?,
                ],
            ))
        })?;

        let mut axes_by_cell: std::collections::HashMap<String, [Vec<f64>; 3]> =
            std::collections::HashMap::new();
        for row in rows {
            let (cell_form_key, position) = row?;
            let axes = axes_by_cell.entry(cell_form_key).or_default();
            for (axis, value) in axes.iter_mut().zip(position) {
                axis.push(value);
            }
        }

        Ok(axes_by_cell
            .into_iter()
            .map(|(cell_form_key, mut axes)| {
                let [x, y, z] = axes.each_mut().map(|values| median(values));
                (cell_form_key, x, y, z)
            })
            .collect())
    }

    /// Worldspace-wide persistent REFRs. These live in WRLD.TopCell (flagged
    /// `is_worldspace_persistent_cell` by the installer), which the grid queries
    /// deliberately exclude — its grid coords collide with the real (0,0) cell.
    pub fn get_worldspace_persistent(
        worldspace: &str,
        conn: &rusqlite::Connection,
        schema: DbSchema,
    ) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {}
             FROM placed_objects po
             JOIN cells c ON c.form_key = po.cell_form_key
             WHERE c.worldspace = ?
               AND COALESCE(c.is_worldspace_persistent_cell, 0) = 1
               AND po.is_persistent = 1",
            select_columns(schema, "po.")
        ))?;

        Ok(stmt
            .query_map(rusqlite::params![worldspace], |row| {
                row_to_placed_object(row)
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }
}

/// Base objects of the refs spawn locations name: the map marker (STAT `MapMarker`) and the
/// heading marker (STAT `XMarkerHeading`). Filtering on them lets the lookup use `idx_placed_base`.
const SPAWN_MARKER_BASE_OBJECTS: [&str; 2] = ["000010:Fallout4.esm", "000034:Fallout4.esm"];

/// A marker ref found by its editor id, with the placement of the cell it sits in.
#[derive(Debug, Clone, PartialEq)]
pub struct MarkerRef {
    pub editor_id: String,
    pub form_key: String,
    /// `None` when the ref sits in no known cell or in a cell of no worldspace.
    pub worldspace: Option<String>,
    /// `None` when the ref sits in no known cell.
    pub cell_is_interior: Option<bool>,
    pub position: Option<[f64; 3]>,
    pub rotation_z: Option<f64>,
}

impl MarkerRef {
    /// The map and heading marker refs among `editor_ids`, in one query. Ids that match no such
    /// ref are absent from the result.
    pub fn get_by_editor_ids(
        editor_ids: &[&str],
        conn: &rusqlite::Connection,
    ) -> anyhow::Result<Vec<Self>> {
        if editor_ids.is_empty() {
            return Ok(Vec::new());
        }
        let base_placeholders = vec!["?"; SPAWN_MARKER_BASE_OBJECTS.len()].join(",");
        let id_placeholders = vec!["?"; editor_ids.len()].join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT r.editor_id, po.form_key, c.worldspace, c.is_interior,
                    po.position_x, po.position_y, po.position_z, po.rotation_z
             FROM placed_objects po
             JOIN records r ON r.form_key = po.form_key
             LEFT JOIN cells c ON c.form_key = po.cell_form_key
             WHERE po.base_object IN ({base_placeholders})
               AND r.editor_id IN ({id_placeholders})"
        ))?;
        let params = SPAWN_MARKER_BASE_OBJECTS.iter().chain(editor_ids);
        let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
            let coordinates: [Option<f64>; 3] = [row.get(4)?, row.get(5)?, row.get(6)?];
            Ok(Self {
                editor_id: row.get(0)?,
                form_key: row.get(1)?,
                worldspace: row.get(2)?,
                cell_is_interior: row.get(3)?,
                position: match coordinates {
                    [Some(x), Some(y), Some(z)] => Some([x, y, z]),
                    _ => None,
                },
                rotation_z: row.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

/// Median of a non-empty slice; the mean of the two middle values for an even count.
fn median(values: &mut [f64]) -> f64 {
    values.sort_unstable_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 1 {
        values[middle]
    } else {
        f64::midpoint(values[middle - 1], values[middle])
    }
}
