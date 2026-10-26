/// The columns of a weather beyond those [`super::weather::WeatherRow`] reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeatherLibraryRow {
    /// Form key of the shader particle geometry record of the weather's precipitation.
    pub precipitation: Option<String>,
    /// JSON `{"model":..,"textures":[..],"layers":[..]}`; NULL when the weather has no clouds.
    pub clouds: Option<String>,
    /// JSON with the wind, fade, thunder and flag values of the weather's `DATA`.
    pub data: Option<String>,
    /// JSON `[{"kind":"default"|"precipitation"|"wind"|"thunder","sound":form key}]`.
    pub sounds: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticleGeometryRow {
    /// JSON of the particle geometry's `DATA`.
    pub data: Option<String>,
    pub particle_texture: Option<String>,
    /// The authored BGEM path.
    pub particle_material: Option<String>,
}

/// `QueryReturnedNoRows` when the weather is unknown.
pub fn get_weather_library_row(
    form_key: &str,
    conn: &rusqlite::Connection,
) -> rusqlite::Result<WeatherLibraryRow> {
    conn.query_row(
        "SELECT precipitation, clouds, data, sounds FROM weather WHERE form_key = ?",
        rusqlite::params![form_key],
        |row| {
            Ok(WeatherLibraryRow {
                precipitation: row.get(0)?,
                clouds: row.get(1)?,
                data: row.get(2)?,
                sounds: row.get(3)?,
            })
        },
    )
}

/// `QueryReturnedNoRows` when the particle geometry is unknown.
pub fn get_particle_geometry(
    form_key: &str,
    conn: &rusqlite::Connection,
) -> rusqlite::Result<ParticleGeometryRow> {
    conn.query_row(
        "SELECT data, particle_texture, particle_material FROM shader_particle_geometries WHERE form_key = ?",
        rusqlite::params![form_key],
        |row| {
            Ok(ParticleGeometryRow {
                data: row.get(0)?,
                particle_texture: row.get(1)?,
                particle_material: row.get(2)?,
            })
        },
    )
}

/// `Ok(None)` when the climate has no volatility; `QueryReturnedNoRows` when it is unknown.
pub fn get_climate_volatility(
    climate: &str,
    conn: &rusqlite::Connection,
) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT volatility FROM climates WHERE form_key = ?",
        rusqlite::params![climate],
        |row| row.get(0),
    )
}
