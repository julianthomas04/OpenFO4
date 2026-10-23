use serde::Deserialize;

/// One WATR record. `data` is the installer's `WaterJson.Project` object; it is
/// `None` for databases converted before water extraction (the column exists
/// but holds NULL), which is distinct from a record that fails to decode.
#[derive(Debug, Clone, PartialEq)]
pub struct Water {
    pub form_key: String,
    pub editor_id: Option<String>,
    pub name: Option<String>,
    pub material: Option<String>,
    pub data: Option<WaterData>,
}

/// An RGBA colour as the installer writes it, `[r, g, b, a]` bytes.
pub type Rgba8 = [u8; 4];

/// The WATR fields the water renderer reads, under Mutagen's property names
/// (`WaterJson.Project`). Every field is required: the installer omits a value
/// it could not read, and a record missing one fails loudly instead of
/// rendering with an invented default. Properties not listed here
/// (`Flags`, spells, sounds, ...) are ignored.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct WaterData {
    pub opacity: u8,
    /// `INAM`: the image space applied while the camera is fully under this
    /// water (only its depth of field is consumed). Absent when the WATR has
    /// none.
    #[serde(default)]
    pub image_space: Option<String>,
    pub fog_depth_amount: f32,
    pub fog_shallow_color: Rgba8,
    pub fog_deep_color: Rgba8,
    pub fog_color_shallow_range: f32,
    pub fog_color_deep_range: f32,
    pub fog_shallow_alpha: f32,
    pub fog_deep_alpha: f32,
    pub fog_alpha_shallow_range: f32,
    pub fog_alpha_deep_range: f32,
    pub fog_underwater_color: Rgba8,
    pub fog_underwater_amount: f32,
    pub fog_underwater_near: f32,
    pub fog_underwater_far: f32,
    pub physical_normal_magnitude: f32,
    pub physical_shallow_normal_falloff: f32,
    pub physical_deep_normal_falloff: f32,
    pub physical_reflectivity_amount: f32,
    pub physical_fresnel_amount: f32,
    pub physical_surface_effect_falloff: f32,
    pub physical_displacement_simulator_dampener: f32,
    pub physical_reflection_color: Rgba8,
    pub specular_sun_specular_power: f32,
    pub specular_sun_specular_magnitude: f32,
    pub specular_sun_sparkle_power: f32,
    pub specular_sun_sparkle_magnitude: f32,
    pub specular_interior_specular_radius: f32,
    pub specular_interior_specular_brightness: f32,
    pub specular_interior_specular_power: f32,
    pub silt_amount: f32,
    pub silt_light_color: Rgba8,
    pub silt_dark_color: Rgba8,
    pub screen_space_reflections: bool,
    pub noise_layer_one: WaterNoiseLayer,
    pub noise_layer_two: WaterNoiseLayer,
    pub noise_layer_three: WaterNoiseLayer,
    /// `NAM0`: the global current, Havok metres per second on the game axes.
    pub linear_velocity: [f32; 3],
    /// `NAM1`: the global angular current, radians per second on the game axes.
    pub angular_velocity: [f32; 3],
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct WaterNoiseLayer {
    pub wind_direction: f32,
    pub wind_speed: f32,
    pub amplitude_scale: f32,
    pub uv_scale: f32,
    pub noise_falloff: f32,
    /// Raw path as stored in the plugin (`data\Textures\Water\x.dds`).
    pub texture: Option<String>,
}

impl WaterData {
    /// The three normal layers in order.
    pub fn noise_layers(&self) -> [&WaterNoiseLayer; 3] {
        [
            &self.noise_layer_one,
            &self.noise_layer_two,
            &self.noise_layer_three,
        ]
    }

    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

/// A `waters` row before its JSON column is decoded.
struct WaterRow {
    form_key: String,
    editor_id: Option<String>,
    name: Option<String>,
    material: Option<String>,
    data_json: Option<String>,
}

impl WaterRow {
    fn read(row: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            form_key: row.get(0)?,
            editor_id: row.get(1)?,
            name: row.get(2)?,
            material: row.get(3)?,
            data_json: row.get(4)?,
        })
    }

    /// Names the record when its JSON is unusable, so one bad WATR is attributable.
    fn into_water(self) -> anyhow::Result<Water> {
        let data = self
            .data_json
            .map(|json| {
                WaterData::parse(&json).map_err(|e| {
                    anyhow::anyhow!("WATR {} has unusable data JSON: {e}", self.form_key)
                })
            })
            .transpose()?;
        Ok(Water {
            form_key: self.form_key,
            editor_id: self.editor_id,
            name: self.name,
            material: self.material,
            data,
        })
    }
}

impl Water {
    const COLUMNS: &'static str = "form_key, editor_id, name, material, data";

    pub fn get(form_key: &str, conn: &rusqlite::Connection) -> anyhow::Result<Self> {
        let mut stmt = conn.prepare_cached(&format!(
            "SELECT {} FROM waters WHERE form_key = ?",
            Self::COLUMNS
        ))?;
        stmt.query_row(rusqlite::params![form_key], WaterRow::read)?
            .into_water()
    }

    pub fn get_all(conn: &rusqlite::Connection) -> anyhow::Result<Vec<Self>> {
        let mut stmt = conn.prepare(&format!("SELECT {} FROM waters", Self::COLUMNS))?;
        stmt.query_map([], WaterRow::read)?
            .map(|row| row?.into_water())
            .collect()
    }
}
