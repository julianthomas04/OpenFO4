use std::sync::Once;

use rusqlite::Connection;

static MISSING_GROUPS_LOGGED: Once = Once::new();

const LIGHT_COLUMNS: &[&str] = &[
    "constant",
    "scalar",
    "exponent",
    "fov",
    "near_clip",
    "flicker_period",
    "flicker_intensity_amplitude",
    "flicker_movement_amplitude",
    "gobo",
    "lens",
    "god_rays",
    "god_rays_near_clip",
];

const PLACED_OBJECT_LIGHT_COLUMNS: &[&str] = &[
    "radius_override",
    "light_fov_pm",
    "light_fade_pm",
    "light_end_distance_cap",
    "light_shadow_depth_bias",
    "light_near_clip",
    "light_volumetric_intensity",
    "record_flags",
];

const PLACED_OBJECT_RAGDOLL_POSE_COLUMNS: &[&str] = &["ragdoll_pose"];
const MATERIAL_SWAP_COLUMNS: &[&str] = &["material_name"];
const WORLDSPACE_WEATHER_COLUMNS: &[&str] = &["climate"];
const WEATHER_COLUMNS: &[&str] = &["directional_ambient"];
const WEATHER_FOG_COLUMNS: &[&str] = &["fog"];
const WEATHER_GOD_RAYS_COLUMNS: &[&str] = &["god_rays"];
const GOD_RAYS_DATA_COLUMNS: &[&str] = &["data"];
const WEATHER_LIBRARY_COLUMNS: &[&str] = &["clouds", "data"];
const CLIMATE_VOLATILITY_COLUMNS: &[&str] = &["volatility"];
const PARTICLE_GEOMETRY_COLUMNS: &[&str] = &["data", "particle_texture", "particle_material"];
const PARTICLE_MATERIAL_DATA_COLUMNS: &[&str] = &["particle_material_data"];
const CELL_WATER_COLUMNS: &[&str] = &[
    "has_water",
    "no_lod_water",
    "water_height",
    "water_type",
    "water_environment_map",
    "water_currents",
];
const WORLDSPACE_WATER_COLUMNS: &[&str] = &[
    "water_type",
    "lod_water_type",
    "lod_water_height",
    "default_land_height",
    "default_water_height",
    "water_environment_map",
    "parent_flags",
];
const ACTIVATOR_WATER_COLUMNS: &[&str] = &["water_type"];
const MATERIAL_TYPE_COLUMNS: &[&str] = &["material_hash", "buoyancy"];
const PERK_PIPBOY_COLUMNS: &[&str] = &["level", "playable", "hidden", "swf_path", "first_rank"];
const PERK_CHART_COLUMNS: &[&str] = &["perk", "special", "required_special"];
const SPECIAL_ATTRIBUTE_COLUMNS: &[&str] = &["special", "name", "description"];
const MAP_MARKER_COLUMNS: &[&str] = &[
    "form_key",
    "worldspace",
    "name",
    "marker_type",
    "flags",
    "position_x",
    "position_y",
    "position_z",
];
const WORLDSPACE_MAP_IMAGE_COLUMNS: &[&str] = &["map_image"];

/// The installer command that fills the weather library columns.
pub const WEATHER_LIBRARY_TABLES_FIX: &str =
    "esm-only --tables weather,climates,shader_particle_geometries";

/// The installer command that fills the weather god rays and their data.
const GOD_RAYS_TABLES_FIX: &str = "esm-only --tables weather,god_rayses";

/// The installer command that fills the Pip-Boy tables and columns.
const PIPBOY_TABLES_FIX: &str =
    "esm-only --tables worldspaces,map_markers,perks,perk_chart,special_attributes";
const WEAPON_DEFAULT_MODS_COLUMNS: &[&str] = &["default_mods"];
const OBJECT_MOD_MODEL_COLUMNS: &[&str] = &["model_path", "attach_point_name"];
const DOOR_TRIGGER_BOUNDS_COLUMNS: &[&str] = &["trigger_bounds"];
const COLLISION_LAYER_COLUMNS: &[&str] = &["layer_index", "flags", "collides_with"];
const NAVMESH_COLUMNS: &[&str] = &[
    "form_id",
    "cell",
    "worldspace",
    "grid_x",
    "grid_y",
    "record_flags",
    "format_version",
    "vertex_count",
    "triangle_count",
    "edge_link_count",
    "door_link_count",
    "vertices",
    "triangles",
    "edge_links",
    "door_links",
];

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ColumnGroup {
    Light,
    PlacedObjectLight,
    PlacedObjectRagdollPose,
    MaterialSwap,
    WorldspaceWeather,
    Weather,
    WeatherFog,
    WeatherGodRays,
    GodRaysData,
    WeatherLibrary,
    ClimateVolatility,
    ParticleGeometry,
    ParticleMaterialData,
    CellWater,
    WorldspaceWater,
    ActivatorWater,
    MaterialType,
    PerkPipboy,
    PerkChart,
    SpecialAttribute,
    MapMarker,
    WorldspaceMapImage,
    WeaponDefaultMods,
    ObjectModModel,
    Navmesh,
    CollisionLayer,
    DoorTriggerBounds,
}

impl std::fmt::Display for ColumnGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Light => "light",
            Self::PlacedObjectLight => "placed object light",
            Self::PlacedObjectRagdollPose => "placed object ragdoll pose",
            Self::MaterialSwap => "material swap",
            Self::WorldspaceWeather => "worldspace weather",
            Self::Weather => "weather",
            Self::WeatherFog => "weather fog",
            Self::WeatherGodRays => "weather god rays",
            Self::GodRaysData => "god rays data",
            Self::WeatherLibrary => "weather library",
            Self::ClimateVolatility => "climate volatility",
            Self::ParticleGeometry => "particle geometry",
            Self::ParticleMaterialData => "particle material data",
            Self::CellWater => "cell water",
            Self::WorldspaceWater => "worldspace water",
            Self::ActivatorWater => "activator water",
            Self::MaterialType => "material type",
            Self::PerkPipboy => "perk Pip-Boy",
            Self::PerkChart => "perk chart",
            Self::SpecialAttribute => "special attribute",
            Self::MapMarker => "map marker",
            Self::WorldspaceMapImage => "worldspace map image",
            Self::WeaponDefaultMods => "weapon default mods",
            Self::ObjectModModel => "object modification model",
            Self::Navmesh => "navmesh",
            Self::CollisionLayer => "collision layer",
            Self::DoorTriggerBounds => "door trigger bounds",
        })
    }
}

const MUSIC_TABLES: &[(&str, &[&str])] = &[
    (
        "music_types",
        &[
            "form_key",
            "editor_id",
            "flags",
            "priority",
            "ducking_centidb",
            "fade_seconds",
        ],
    ),
    ("music_type_tracks", &["music_type", "ord", "track"]),
    (
        "music_tracks",
        &[
            "form_key",
            "editor_id",
            "track_type",
            "duration",
            "fade_out",
            "file_key",
        ],
    ),
    (
        "music_track_layers",
        &["track", "layer", "ord", "sub_track"],
    ),
    (
        "music_track_conditions",
        &[
            "track",
            "ord",
            "function",
            "compare_op",
            "or_flag",
            "compare_value",
            "compare_global",
            "run_on",
            "reference",
            "param1_num",
            "param1_record",
            "param1_string",
            "param2_num",
            "param2_record",
            "param2_string",
            "param3",
        ],
    ),
    ("cell_places", &["cell", "music_type", "location"]),
    ("location_music", &["location", "music_type"]),
    ("worldspace_music", &["worldspace", "music_type"]),
    (
        "region_music",
        &[
            "region",
            "music_type",
            "priority",
            "override",
            "record_order",
        ],
    ),
    ("region_areas", &["region", "area", "points"]),
    (
        "sound_categories",
        &["form_key", "editor_id", "parent", "static_attenuation"],
    ),
    ("regions", &["form_key", "worldspace"]),
    ("globals", &["form_key", "editor_id", "value"]),
    (
        "cells",
        &["form_key", "worldspace", "grid_x", "grid_y", "is_interior"],
    ),
];

/// The footstep tables besides the sound ones.
const FOOTSTEP_TABLES: &[(&str, &[&str])] = &[
    ("default_object_slots", &["manager", "slot", "target"]),
    ("footstep_sets", &["form_key", "editor_id"]),
    (
        "footstep_set_entries",
        &["set_key", "gait", "ord", "footstep"],
    ),
    (
        "footsteps",
        &["form_key", "editor_id", "tag", "impact_data_set"],
    ),
    ("impact_data_sets", &["form_key", "editor_id"]),
    (
        "impact_data_set_entries",
        &["set_key", "ord", "material", "impact"],
    ),
    ("impacts", &["form_key", "editor_id", "player_sound"]),
    (
        "material_types",
        &["form_key", "editor_id", "material_hash", "parent"],
    ),
    ("landscape_textures", &["form_key", "material_type"]),
];

/// The sound descriptors, their output models and the sound categories: what a sound that plays
/// from a descriptor reads, footsteps and weather alike.
const SOUND_TABLES: &[(&str, &[&str])] = &[
    (
        "sound_descriptors",
        &[
            "form_key",
            "editor_id",
            "descriptor_type",
            "base_descriptor",
            "freq_shift_pct",
            "freq_variance_pct",
            "priority",
            "db_variance",
            "static_attenuation_centidb",
            "looping_flags",
            "category",
            "output_model",
            "sound_files",
        ],
    ),
    ("sound_descriptor_children", &["descriptor", "ord", "child"]),
    (
        "sound_descriptor_conditions",
        &[
            "descriptor",
            "ord",
            "function",
            "compare_op",
            "or_flag",
            "compare_value",
            "compare_global",
            "run_on",
            "reference",
            "param1_num",
            "param1_record",
            "param1_string",
            "param2_num",
            "param2_record",
            "param2_string",
            "param3",
        ],
    ),
    (
        "sound_output_models",
        &[
            "form_key",
            "editor_id",
            "output_type",
            "static_attenuation_db",
        ],
    ),
    (
        "sound_output_channels",
        &["model", "ord", "speaker", "percent"],
    ),
    (
        "sound_categories",
        &["form_key", "editor_id", "parent", "static_attenuation"],
    ),
];

#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("table `{table}` lacks the footstep columns {}; the database predates footstep extraction, re-run the installer", missing.join(", "))]
    FootstepColumnsMissing {
        table: &'static str,
        missing: Vec<&'static str>,
    },
    #[error("table `{table}` lacks the sound columns {}; the database predates sound extraction, re-run the installer", missing.join(", "))]
    WeatherSoundColumnsMissing {
        table: &'static str,
        missing: Vec<&'static str>,
    },
    #[error("table `{table}` lacks the ambient music columns {}; the database predates music extraction, re-run the installer", missing.join(", "))]
    MusicColumnsMissing {
        table: &'static str,
        missing: Vec<&'static str>,
    },
    #[error("table `{table}` has only some of its {group} columns; missing: {}", missing.join(", "))]
    PartialColumnGroup {
        group: ColumnGroup,
        table: &'static str,
        missing: Vec<&'static str>,
    },
}

/// Which optional column groups the opened database carries. A group is absent
/// for databases converted before the installer extracted it; its fields then
/// read as `None` because the value is unknown.
#[derive(Debug, Clone, Copy)]
pub struct DbSchema {
    pub light_columns_present: bool,
    pub placed_object_light_columns_present: bool,
    pub placed_object_ragdoll_pose_column_present: bool,
    pub material_swap_name_column_present: bool,
    pub weather_columns_present: bool,
    pub weather_fog_column_present: bool,
    /// The god rays of the weathers and the data of the god rays records, both or neither.
    pub weather_god_rays_columns_present: bool,
    /// The weather clouds and data, the climate volatility and the particle geometry
    /// columns the weather library needs, all or none.
    pub weather_library_columns_present: bool,
    /// The material data of the particle geometries, which the weather library reads when present.
    pub particle_material_data_column_present: bool,
    pub cell_water_columns_present: bool,
    pub worldspace_water_columns_present: bool,
    pub activator_water_column_present: bool,
    pub material_type_columns_present: bool,
    pub perk_pipboy_columns_present: bool,
    pub perk_chart_columns_present: bool,
    pub special_attribute_columns_present: bool,
    pub map_marker_columns_present: bool,
    pub worldspace_map_image_column_present: bool,
    pub weapon_default_mods_column_present: bool,
    pub object_mod_model_columns_present: bool,
    pub navmesh_columns_present: bool,
    pub collision_layer_columns_present: bool,
    pub door_trigger_bounds_column_present: bool,
}

impl DbSchema {
    pub fn detect(conn: &Connection) -> anyhow::Result<Self> {
        Ok(Self {
            light_columns_present: column_group_present(
                conn,
                ColumnGroup::Light,
                "lights",
                LIGHT_COLUMNS,
            )?,
            placed_object_light_columns_present: column_group_present(
                conn,
                ColumnGroup::PlacedObjectLight,
                "placed_objects",
                PLACED_OBJECT_LIGHT_COLUMNS,
            )?,
            placed_object_ragdoll_pose_column_present: column_group_present(
                conn,
                ColumnGroup::PlacedObjectRagdollPose,
                "placed_objects",
                PLACED_OBJECT_RAGDOLL_POSE_COLUMNS,
            )?,
            material_swap_name_column_present: column_group_present(
                conn,
                ColumnGroup::MaterialSwap,
                "material_swap_entries",
                MATERIAL_SWAP_COLUMNS,
            )?,
            weather_columns_present: column_group_present(
                conn,
                ColumnGroup::WorldspaceWeather,
                "worldspaces",
                WORLDSPACE_WEATHER_COLUMNS,
            )? && column_group_present(
                conn,
                ColumnGroup::Weather,
                "weather",
                WEATHER_COLUMNS,
            )?,
            weather_fog_column_present: column_group_present(
                conn,
                ColumnGroup::WeatherFog,
                "weather",
                WEATHER_FOG_COLUMNS,
            )?,
            weather_god_rays_columns_present: column_group_present(
                conn,
                ColumnGroup::WeatherGodRays,
                "weather",
                WEATHER_GOD_RAYS_COLUMNS,
            )? & column_group_present(
                conn,
                ColumnGroup::GodRaysData,
                "god_rayses",
                GOD_RAYS_DATA_COLUMNS,
            )?,
            weather_library_columns_present: column_group_present(
                conn,
                ColumnGroup::WeatherLibrary,
                "weather",
                WEATHER_LIBRARY_COLUMNS,
            )? & column_group_present(
                conn,
                ColumnGroup::ClimateVolatility,
                "climates",
                CLIMATE_VOLATILITY_COLUMNS,
            )? & column_group_present(
                conn,
                ColumnGroup::ParticleGeometry,
                "shader_particle_geometries",
                PARTICLE_GEOMETRY_COLUMNS,
            )?,
            particle_material_data_column_present: column_group_present(
                conn,
                ColumnGroup::ParticleMaterialData,
                "shader_particle_geometries",
                PARTICLE_MATERIAL_DATA_COLUMNS,
            )?,
            cell_water_columns_present: column_group_present(
                conn,
                ColumnGroup::CellWater,
                "cells",
                CELL_WATER_COLUMNS,
            )?,
            worldspace_water_columns_present: column_group_present(
                conn,
                ColumnGroup::WorldspaceWater,
                "worldspaces",
                WORLDSPACE_WATER_COLUMNS,
            )?,
            activator_water_column_present: column_group_present(
                conn,
                ColumnGroup::ActivatorWater,
                "activators",
                ACTIVATOR_WATER_COLUMNS,
            )?,
            material_type_columns_present: column_group_present(
                conn,
                ColumnGroup::MaterialType,
                "material_types",
                MATERIAL_TYPE_COLUMNS,
            )?,
            perk_pipboy_columns_present: column_group_present(
                conn,
                ColumnGroup::PerkPipboy,
                "perks",
                PERK_PIPBOY_COLUMNS,
            )?,
            perk_chart_columns_present: column_group_present(
                conn,
                ColumnGroup::PerkChart,
                "perk_chart",
                PERK_CHART_COLUMNS,
            )?,
            special_attribute_columns_present: column_group_present(
                conn,
                ColumnGroup::SpecialAttribute,
                "special_attributes",
                SPECIAL_ATTRIBUTE_COLUMNS,
            )?,
            map_marker_columns_present: column_group_present(
                conn,
                ColumnGroup::MapMarker,
                "map_markers",
                MAP_MARKER_COLUMNS,
            )?,
            worldspace_map_image_column_present: column_group_present(
                conn,
                ColumnGroup::WorldspaceMapImage,
                "worldspaces",
                WORLDSPACE_MAP_IMAGE_COLUMNS,
            )?,
            weapon_default_mods_column_present: column_group_present(
                conn,
                ColumnGroup::WeaponDefaultMods,
                "weapons",
                WEAPON_DEFAULT_MODS_COLUMNS,
            )?,
            object_mod_model_columns_present: column_group_present(
                conn,
                ColumnGroup::ObjectModModel,
                "object_modifications",
                OBJECT_MOD_MODEL_COLUMNS,
            )?,
            navmesh_columns_present: column_group_present(
                conn,
                ColumnGroup::Navmesh,
                "navigation_meshes",
                NAVMESH_COLUMNS,
            )?,
            collision_layer_columns_present: column_group_present(
                conn,
                ColumnGroup::CollisionLayer,
                "collision_layers",
                COLLISION_LAYER_COLUMNS,
            )?,
            door_trigger_bounds_column_present: column_group_present(
                conn,
                ColumnGroup::DoorTriggerBounds,
                "doors",
                DOOR_TRIGGER_BOUNDS_COLUMNS,
            )?,
        })
    }

    /// Whether placed weapons can be assembled from their default mods.
    pub fn weapon_assembly_available(&self) -> bool {
        self.weapon_default_mods_column_present && self.object_mod_model_columns_present
    }

    /// Logs once per process: the database is opened by several subsystems.
    pub fn log_missing_groups(&self) {
        MISSING_GROUPS_LOGGED.call_once(|| {
            if !self.light_columns_present {
                tracing::warn!(
                    "lights light-data columns missing (database predates light data conversion); light attenuation, cone and flicker fields are unavailable"
                );
            }
            if !self.placed_object_light_columns_present {
                tracing::warn!(
                    "placed_objects light columns missing (database predates light data conversion); light radius overrides, XLIG data and record flags are unavailable"
                );
            }
            if !self.placed_object_ragdoll_pose_column_present {
                tracing::warn!(
                    "placed_objects.ragdoll_pose column missing (database predates ragdoll pose conversion); ragdoll props show and collide in their model's rest pose"
                );
            }
            if !(self.cell_water_columns_present
                && self.worldspace_water_columns_present
                && self.activator_water_column_present)
            {
                tracing::warn!(
                    "water columns missing (database predates water extraction); cell and worldspace water planes and placed water meshes cannot name their water type"
                );
            }
            if !self.weather_god_rays_columns_present {
                tracing::warn!(
                    "weather.god_rays / god_rayses.data columns missing (database predates the god rays export); exterior fog has no sun in-scatter. Fix: run the installer with `{GOD_RAYS_TABLES_FIX}`"
                );
            }
            if !self.material_swap_name_column_present {
                tracing::warn!(
                    "material_swap_entries.material_name column missing (database predates material data conversion); material swaps render matte with plain texture fields"
                );
            }
            if !self.material_type_columns_present {
                tracing::warn!(
                    "material_types.material_hash/buoyancy columns missing (database predates material type conversion); every dynamic body gets the default buoyancy of 1.0"
                );
            }
            for (present, what) in [
                (self.perk_pipboy_columns_present, "perks"),
                (self.perk_chart_columns_present, "perk chart"),
                (self.special_attribute_columns_present, "SPECIAL descriptions"),
                (self.map_marker_columns_present, "map markers"),
                (self.worldspace_map_image_column_present, "the world map image"),
            ] {
                if !present {
                    tracing::warn!(
                        "Pip-Boy {what} data missing (database predates the Pip-Boy extraction); the Pip-Boy shows none of it. Fix: run the installer with `{PIPBOY_TABLES_FIX}`"
                    );
                }
            }
            if !self.weapon_assembly_available() {
                tracing::warn!(
                    "weapons.default_mods / object_modifications model columns missing (database predates weapon assembly); placed weapons are not drawn, reinstall"
                );
            }
            if !self.navmesh_columns_present {
                tracing::warn!(
                    "navigation_meshes geometry columns missing (database predates navmesh extraction); no navigation meshes are loaded"
                );
            }
        });
    }
}

/// Select expressions for a column group: each column when the group is present,
/// otherwise `NULL AS <column>` so row mappers keep fixed positions.
fn select_list(columns: &[&str], present: bool, prefix: &str) -> String {
    columns
        .iter()
        .map(|column| {
            if present {
                format!("{prefix}{column}")
            } else {
                format!("NULL AS {column}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn light_select_list(schema: DbSchema) -> String {
    select_list(LIGHT_COLUMNS, schema.light_columns_present, "")
}

pub fn material_swap_select_list(schema: DbSchema) -> String {
    select_list(
        MATERIAL_SWAP_COLUMNS,
        schema.material_swap_name_column_present,
        "",
    )
}

pub fn placed_object_ragdoll_pose_select_list(schema: DbSchema, prefix: &str) -> String {
    select_list(
        PLACED_OBJECT_RAGDOLL_POSE_COLUMNS,
        schema.placed_object_ragdoll_pose_column_present,
        prefix,
    )
}

pub fn cell_water_select_list(schema: DbSchema) -> String {
    select_list(CELL_WATER_COLUMNS, schema.cell_water_columns_present, "")
}

pub fn worldspace_water_select_list(schema: DbSchema) -> String {
    select_list(
        WORLDSPACE_WATER_COLUMNS,
        schema.worldspace_water_columns_present,
        "",
    )
}

pub fn activator_water_select_list(schema: DbSchema, prefix: &str) -> String {
    select_list(
        ACTIVATOR_WATER_COLUMNS,
        schema.activator_water_column_present,
        prefix,
    )
}

pub fn door_trigger_bounds_select_list(schema: DbSchema, prefix: &str) -> String {
    select_list(
        DOOR_TRIGGER_BOUNDS_COLUMNS,
        schema.door_trigger_bounds_column_present,
        prefix,
    )
}

pub fn placed_object_light_select_list(schema: DbSchema, prefix: &str) -> String {
    select_list(
        PLACED_OBJECT_LIGHT_COLUMNS,
        schema.placed_object_light_columns_present,
        prefix,
    )
}

/// Every table and column the ambient music catalog reads; a missing table reads as all its columns missing.
pub fn require_music_tables(conn: &Connection) -> anyhow::Result<()> {
    for &(table, columns) in MUSIC_TABLES {
        let existing = table_columns(conn, table)?;
        let missing: Vec<&'static str> = columns
            .iter()
            .copied()
            .filter(|column| !existing.iter().any(|name| name == column))
            .collect();
        if !missing.is_empty() {
            return Err(SchemaError::MusicColumnsMissing { table, missing }.into());
        }
    }
    Ok(())
}

/// Every table and column the footstep catalog reads; a missing table reads as all its columns missing.
pub fn require_footstep_tables(conn: &Connection) -> anyhow::Result<()> {
    match first_table_with_missing_columns(conn, FOOTSTEP_TABLES.iter().chain(SOUND_TABLES))? {
        Some((table, missing)) => {
            Err(SchemaError::FootstepColumnsMissing { table, missing }.into())
        }
        None => Ok(()),
    }
}

/// Every table and column the weather sounds read; a missing table reads as all its columns
/// missing. The footstep tables are not needed.
pub fn require_weather_sound_tables(conn: &Connection) -> anyhow::Result<()> {
    match first_table_with_missing_columns(conn, SOUND_TABLES)? {
        Some((table, missing)) => {
            Err(SchemaError::WeatherSoundColumnsMissing { table, missing }.into())
        }
        None => Ok(()),
    }
}

/// The first of `tables` that lacks columns, with the columns it lacks.
fn first_table_with_missing_columns<'a>(
    conn: &Connection,
    tables: impl IntoIterator<Item = &'a (&'static str, &'static [&'static str])>,
) -> anyhow::Result<Option<(&'static str, Vec<&'static str>)>> {
    for &(table, columns) in tables {
        let missing = missing_columns(conn, table, columns)?;
        if !missing.is_empty() {
            return Ok(Some((table, missing)));
        }
    }
    Ok(None)
}

fn missing_columns(
    conn: &Connection,
    table: &str,
    columns: &[&'static str],
) -> anyhow::Result<Vec<&'static str>> {
    let existing = table_columns(conn, table)?;
    Ok(columns
        .iter()
        .copied()
        .filter(|column| !existing.iter().any(|name| name == column))
        .collect())
}

fn table_columns(conn: &Connection, table: &str) -> anyhow::Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info(?)")?;
    Ok(stmt
        .query_map([table], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?)
}

fn column_group_present(
    conn: &Connection,
    group: ColumnGroup,
    table: &'static str,
    columns: &[&'static str],
) -> anyhow::Result<bool> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info(?)")?;
    let existing = stmt
        .query_map([table], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;

    let missing: Vec<&'static str> = columns
        .iter()
        .copied()
        .filter(|column| !existing.iter().any(|name| name == column))
        .collect();

    if missing.is_empty() {
        Ok(true)
    } else if missing.len() == columns.len() {
        Ok(false)
    } else {
        Err(SchemaError::PartialColumnGroup {
            group,
            table,
            missing,
        }
        .into())
    }
}
