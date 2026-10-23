use crate::assets::db::schema::{DbSchema, light_select_list};

pub struct Light {
    pub form_key: Option<String>,
    pub editor_id: Option<String>,
    pub model_path: Option<String>,
    pub radius: Option<f64>,
    pub color: Option<i64>,
    /// Raw DATA flag bits.
    pub flags: Option<i64>,
    pub fade_value: Option<f64>,
    pub falloff_exponent: Option<f64>,
    pub is_marker: bool,
    /// LIGH attenuation fields: constant, scalar, exponent.
    pub constant: Option<f64>,
    pub scalar: Option<f64>,
    pub exponent: Option<f64>,
    pub fov: Option<f64>,
    pub near_clip: Option<f64>,
    pub flicker_period: Option<f64>,
    pub flicker_intensity_amplitude: Option<f64>,
    pub flicker_movement_amplitude: Option<f64>,
    pub gobo: Option<String>,
    pub lens: Option<String>,
    pub god_rays: Option<String>,
    pub god_rays_near_clip: Option<f64>,
}

fn row_to_light(row: &rusqlite::Row) -> rusqlite::Result<Light> {
    Ok(Light {
        form_key: row.get(0)?,
        editor_id: row.get(1)?,
        model_path: row.get(2)?,
        radius: row.get(3)?,
        color: row.get(4)?,
        flags: row.get(5)?,
        fade_value: row.get(6)?,
        falloff_exponent: row.get(7)?,
        is_marker: row.get(8)?,
        constant: row.get(9)?,
        scalar: row.get(10)?,
        exponent: row.get(11)?,
        fov: row.get(12)?,
        near_clip: row.get(13)?,
        flicker_period: row.get(14)?,
        flicker_intensity_amplitude: row.get(15)?,
        flicker_movement_amplitude: row.get(16)?,
        gobo: row.get(17)?,
        lens: row.get(18)?,
        god_rays: row.get(19)?,
        god_rays_near_clip: row.get(20)?,
    })
}

/// `flags` is cast because databases converted before it became an INTEGER
/// column hold the same bits as text.
fn select_sql(schema: DbSchema) -> String {
    format!(
        "SELECT form_key, editor_id, model_path, radius, color,
                CAST(flags AS INTEGER), fade_value, falloff_exponent, is_marker,
                {}
         FROM lights",
        light_select_list(schema)
    )
}

impl Light {
    pub fn get(
        form_key: &str,
        conn: &rusqlite::Connection,
        schema: DbSchema,
    ) -> anyhow::Result<Self> {
        Ok(conn.query_row(
            &format!("{} WHERE form_key = ?", select_sql(schema)),
            rusqlite::params![form_key],
            row_to_light,
        )?)
    }

    pub fn get_all(conn: &rusqlite::Connection, schema: DbSchema) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(&select_sql(schema))?;

        Ok(stmt
            .query_map([], row_to_light)?
            .collect::<Result<Vec<_>, _>>()?)
    }
}
