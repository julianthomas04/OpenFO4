pub struct WeatherRow {
    pub form_key: String,
    pub editor_id: Option<String>,
    /// JSON `{"<Group>":{"<Time>":{R,G,B}}}`; NULL until the database is re-converted with weather colours.
    pub colors: Option<String>,
    /// JSON `{"<Time>":{XPlus..ZMinus,Specular,Scale}}`.
    pub directional_ambient: Option<String>,
    /// JSON `{"<Time>":"<image space form key>"|null}`.
    pub image_spaces: Option<String>,
    /// JSON `{"<Time>":{Near,Far,Power,Max,...}}` in raw Bethesda units; NULL until the database is re-converted with fog data.
    pub fog: Option<String>,
    /// JSON `{"<Time>":"<god rays form key>"|null}`; NULL when the weather has none or the
    /// database predates the god rays export.
    pub god_rays: Option<String>,
}

/// `Ok(None)` when the worldspace has no climate; `QueryReturnedNoRows` when the worldspace is unknown.
pub fn get_worldspace_climate(
    worldspace: &str,
    conn: &rusqlite::Connection,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT climate FROM worldspaces WHERE form_key = ?",
        rusqlite::params![worldspace],
        |row| row.get(0),
    )
}

/// JSON `{"Weathers":[{"Weather":fk,"Chance":n}],...}`; `QueryReturnedNoRows` when the climate is unknown.
pub fn get_climate_data(
    climate: &str,
    conn: &rusqlite::Connection,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT data FROM climates WHERE form_key = ?",
        rusqlite::params![climate],
        |row| row.get(0),
    )
}

/// The sun and sun glare texture columns of a climate; `QueryReturnedNoRows` when the climate is unknown.
pub fn get_climate_sun_textures(
    climate: &str,
    conn: &rusqlite::Connection,
) -> rusqlite::Result<(Option<String>, Option<String>)> {
    conn.query_row(
        "SELECT sun_texture, sun_glare_texture FROM climates WHERE form_key = ?",
        rusqlite::params![climate],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
}

/// Which optional columns of the weather table the database has.
#[derive(Debug, Clone, Copy)]
pub struct WeatherColumns {
    pub fog: bool,
    pub god_rays: bool,
}

/// `QueryReturnedNoRows` when the weather is unknown.
pub fn get_weather(
    form_key: &str,
    columns: WeatherColumns,
    conn: &rusqlite::Connection,
) -> rusqlite::Result<WeatherRow> {
    let fog_column = if columns.fog { "fog" } else { "NULL" };
    let god_rays_column = if columns.god_rays { "god_rays" } else { "NULL" };
    conn.query_row(
        &format!("SELECT form_key, editor_id, colors, directional_ambient, image_spaces, {fog_column}, {god_rays_column} FROM weather WHERE form_key = ?"),
        rusqlite::params![form_key],
        |row| {
            Ok(WeatherRow {
                form_key: row.get(0)?,
                editor_id: row.get(1)?,
                colors: row.get(2)?,
                directional_ambient: row.get(3)?,
                image_spaces: row.get(4)?,
                fog: row.get(5)?,
                god_rays: row.get(6)?,
            })
        },
    )
}

/// JSON `{"Hdr":{...},"Cinematic":{...},"Tint":{...}}`; `QueryReturnedNoRows` when the image space is unknown.
pub fn get_image_space_data(
    form_key: &str,
    conn: &rusqlite::Connection,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT data FROM image_spaces WHERE form_key = ?",
        rusqlite::params![form_key],
        |row| row.get(0),
    )
}

/// JSON of the `DATA` of a god rays record; `QueryReturnedNoRows` when the record is unknown.
pub fn get_god_rays_data(
    form_key: &str,
    conn: &rusqlite::Connection,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT data FROM god_rayses WHERE form_key = ?",
        rusqlite::params![form_key],
        |row| row.get(0),
    )
}
