use std::collections::HashMap;
use std::sync::Arc;

use super::schema::{DbSchema, material_swap_select_list};

/// One MSWP substitution, fully pre-resolved by the installer:
/// `original_material` is the normalized BGSM census key matched against the
/// GLB material-name `src:` token; `diffuse`/`normal` are final deployed
/// texture paths (palette-baked variant filenames already substituted), so
/// applying a swap is pure string replacement — no material parsing here.
pub struct SwapEntry {
    pub original_material: String,
    pub diffuse: Option<String>,
    pub normal: Option<String>,
    pub is_blend: bool,
    pub alpha_mult: f32,
    /// 0-255 scissor cutoff; None = no alpha test.
    pub alpha_test: Option<u8>,
    pub two_sided: bool,
    /// GLB-grammar material name of the replacement (textures, alpha and
    /// lighting). None for databases converted before the column existed, or
    /// when the installer left it empty.
    pub material_name: Option<String>,
}

/// Base-record material swaps (FO4 MODS subrecord): base `form_key` -> MSWP
/// `form_key`. A placed ref's own swap (`placed_objects.material_swap`, from
/// XMSP) takes precedence over this.
pub fn get_base_material_swaps(
    conn: &rusqlite::Connection,
) -> anyhow::Result<HashMap<String, String>> {
    let mut stmt = conn.prepare(
        "SELECT form_key, material_swap FROM base_material_swaps
         WHERE material_swap IS NOT NULL",
    )?;
    let mut map = HashMap::new();
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (form_key, swap) = row?;
        map.insert(form_key, swap);
    }
    Ok(map)
}

/// All MSWP substitution entries, grouped by swap `form_key`. ~6k rows total —
/// loaded once at worker startup, shared by reference per mesh group.
pub fn get_material_swap_entries(
    conn: &rusqlite::Connection,
    schema: DbSchema,
) -> anyhow::Result<HashMap<String, Arc<Vec<SwapEntry>>>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT swap_form_key, original_material, diffuse, normal,
                is_blend, alpha_mult, alpha_test, two_sided, {}
         FROM material_swap_entries",
        material_swap_select_list(schema)
    ))?;
    let mut map: HashMap<String, Vec<SwapEntry>> = HashMap::new();
    let rows = stmt.query_map([], |row| {
        let swap_key: String = row.get(0)?;
        let diffuse: Option<String> = row.get(2)?;
        let normal: Option<String> = row.get(3)?;
        Ok((
            swap_key,
            SwapEntry {
                original_material: row.get(1)?,
                diffuse: diffuse.filter(|s| !s.is_empty()),
                normal: normal.filter(|s| !s.is_empty()),
                is_blend: row.get::<_, i64>(4)? != 0,
                alpha_mult: row.get::<_, f64>(5)? as f32,
                alpha_test: row.get::<_, Option<i64>>(6)?.map(|n| n.clamp(0, 255) as u8),
                two_sided: row.get::<_, i64>(7)? != 0,
                material_name: row.get::<_, Option<String>>(8)?.filter(|s| !s.is_empty()),
            },
        ))
    })?;
    for row in rows {
        let (swap_key, entry) = row?;
        map.entry(swap_key).or_default().push(entry);
    }
    Ok(map.into_iter().map(|(k, v)| (k, Arc::new(v))).collect())
}
