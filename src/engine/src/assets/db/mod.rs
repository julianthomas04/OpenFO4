pub mod activators;
pub mod addon_nodes;
pub mod cells;
pub mod containers;
pub mod doors;
pub mod flora;
pub mod footsteps;
pub mod furniture;
pub mod grass;
pub mod items;
pub mod landscape_textures;
pub mod landscapes;
pub mod lighting_templates;
pub mod lights;
pub mod map_markers;
pub mod material_swaps;
pub mod material_types;
pub mod movable_statics;
pub mod movement_types;
pub mod music;
pub mod navigation_meshes;
pub mod perks;
pub mod placed_objects;
pub mod schema;
pub mod special_attributes;
pub mod static_collections;
pub mod statics;
pub mod teleports;
pub mod trees;
pub mod water_placement;
pub mod waters;
pub mod weapons;
pub mod weather;
pub mod worldspace_lineage;
pub mod worldspaces;

use rusqlite::{Connection, OpenFlags};
use tracing::instrument;

use crate::assets::db::{
    activators::Activator,
    cells::{Cell, InteriorLightingRows},
    containers::Container,
    doors::Door,
    flora::Flora,
    furniture::Furniture,
    grass::{GrassRow, LtexGrassRow},
    landscape_textures::LandscapeTextureDiffuse,
    landscapes::Landscape,
    lighting_templates::LightingTemplate,
    lights::Light,
    material_types::MaterialBuoyancyRow,
    movable_statics::MovableStatic,
    movement_types::MovementType,
    placed_objects::{MarkerRef, PlacedObject},
    schema::DbSchema,
    static_collections::StaticCollection,
    statics::Static,
    trees::Tree,
    water_placement::{CellWater, LodWaterQuad, PlacedWaterRef, WorldspaceWater},
    waters::Water,
    weapons::{ModModelRow, WeaponRow},
    worldspaces::Worldspace,
};
use crate::navmesh::NavMeshRecord;

pub struct DbClient {
    conn: Connection,
    /// Kept so `try_clone` can reopen the same database; `None` for a
    /// connection handed in by the caller (e.g. in-memory), which cannot be
    /// reopened. Never exposed — all DB access goes through `DbClient` methods.
    path: Option<String>,
    schema: DbSchema,
}

impl DbClient {
    pub fn new(path: &str) -> anyhow::Result<Self> {
        let conn = Self::open(path)?;
        Self::with_connection(conn, Some(path.to_string()))
    }

    /// Wraps an already open connection (in-memory databases, tests). Such a
    /// client cannot `try_clone`, since there is no path to reopen.
    pub fn from_connection(conn: Connection) -> anyhow::Result<Self> {
        Self::with_connection(conn, None)
    }

    fn with_connection(conn: Connection, path: Option<String>) -> anyhow::Result<Self> {
        let schema = DbSchema::detect(&conn)?;
        schema.log_missing_groups();
        Ok(Self { conn, path, schema })
    }

    fn open(path: &str) -> anyhow::Result<Connection> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;

        conn.execute_batch(
            "
            PRAGMA journal_mode=WAL;
            PRAGMA mmap_size=1073741824;
            PRAGMA synchronous=OFF;
        ",
        )?;

        Ok(conn)
    }

    pub fn schema(&self) -> DbSchema {
        self.schema
    }

    /// A second read-only connection to the same database, for handing to
    /// another thread (the DB is read-only, so connections never conflict;
    /// `Connection` is Send but not Sync, so each thread needs its own).
    pub fn try_clone(&self) -> anyhow::Result<Self> {
        let path = self.path.as_deref().ok_or_else(|| {
            anyhow::anyhow!("cannot clone a DbClient built from an existing connection")
        })?;
        Ok(Self {
            conn: Self::open(path)?,
            path: self.path.clone(),
            schema: self.schema,
        })
    }

    /// The ambient music tables; errors when the installer has not extracted them.
    #[instrument(skip(self))]
    pub fn get_music_catalog(&self) -> anyhow::Result<music::MusicCatalog> {
        music::MusicCatalog::from_connection(&self.conn)
    }

    /// The footstep, impact and sound tables; errors when the installer has not extracted them.
    #[instrument(skip(self))]
    pub fn get_footstep_catalog(&self) -> anyhow::Result<footsteps::FootstepCatalog> {
        footsteps::FootstepCatalog::from_connection(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_perks(&self) -> anyhow::Result<Vec<perks::PerkRow>> {
        if !self.schema.perk_pipboy_columns_present {
            return Ok(Vec::new());
        }
        perks::PerkRow::list_all(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_perk_chart(&self) -> anyhow::Result<Vec<perks::PerkChartRow>> {
        if !self.schema.perk_chart_columns_present {
            return Ok(Vec::new());
        }
        perks::PerkChartRow::list_all(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_special_attributes(
        &self,
    ) -> anyhow::Result<Vec<special_attributes::SpecialAttributeRow>> {
        if !self.schema.special_attribute_columns_present {
            return Ok(Vec::new());
        }
        special_attributes::SpecialAttributeRow::list_all(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_items(&self) -> anyhow::Result<Vec<items::ItemRow>> {
        items::ItemRow::list_all(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_map_markers(
        &self,
        worldspace: &str,
    ) -> anyhow::Result<Vec<map_markers::MapMarkerRow>> {
        if !self.schema.map_marker_columns_present {
            return Ok(Vec::new());
        }
        map_markers::MapMarkerRow::list_for_worldspace(worldspace, &self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_worldspace_map_image(&self, worldspace: &str) -> anyhow::Result<Option<String>> {
        if !self.schema.worldspace_map_image_column_present {
            return Ok(None);
        }
        Worldspace::map_image(worldspace, &self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_cell(&self, x: i32, y: i32, worldspace: &str) -> anyhow::Result<Cell> {
        Cell::get(x, y, worldspace, &self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_cell_by_form_key(&self, form_key: &str) -> anyhow::Result<Cell> {
        Cell::get_by_form_key(form_key, &self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_cell_by_editor_id(&self, editor_id: &str) -> anyhow::Result<Option<Cell>> {
        Cell::get_by_editor_id(editor_id, &self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_cells(&self, worldspace: &str) -> anyhow::Result<Vec<Cell>> {
        Cell::get_for_worldspace(worldspace, &self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_interior_cells(&self) -> anyhow::Result<Vec<Cell>> {
        Cell::list_interiors(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_interior_lighting(&self) -> anyhow::Result<InteriorLightingRows> {
        Cell::list_interior_lighting(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_populated_cell(&self, worldspace: &str) -> anyhow::Result<Cell> {
        Cell::get_populated_cell(worldspace, &self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_cells_in_range(
        &self,
        worldspace: &str,
        x_min: i32,
        x_max: i32,
        y_min: i32,
        y_max: i32,
    ) -> anyhow::Result<Vec<Cell>> {
        Cell::get_in_range(worldspace, x_min, x_max, y_min, y_max, &self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_landscape(&self, cell: &str, grid_x: i32, grid_y: i32) -> anyhow::Result<Landscape> {
        Landscape::get(cell, grid_x, grid_y, &self.conn)
    }

    #[instrument(skip(self, cells))]
    pub fn get_landscapes(&self, cells: &[Cell]) -> anyhow::Result<Vec<Landscape>> {
        Landscape::get_for_cells(cells, &self.conn)
    }

    /// The navigation meshes of one cell (exterior or interior), or none for a database
    /// converted before the installer extracted them. Stub rows without geometry are skipped.
    #[instrument(skip(self))]
    pub fn get_navmeshes_for_cell(
        &self,
        cell_form_key: &str,
    ) -> anyhow::Result<Vec<NavMeshRecord>> {
        if !self.schema.navmesh_columns_present {
            return Ok(Vec::new());
        }
        navigation_meshes::get_for_cell(cell_form_key, &self.conn)
    }

    /// Every navigation mesh with geometry; see [`Self::get_navmeshes_for_cell`].
    #[instrument(skip(self))]
    pub fn get_all_navmeshes(&self) -> anyhow::Result<Vec<NavMeshRecord>> {
        if !self.schema.navmesh_columns_present {
            return Ok(Vec::new());
        }
        navigation_meshes::get_all(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_placed_objects(&self, cell: &str) -> anyhow::Result<Vec<PlacedObject>> {
        PlacedObject::get(cell, &self.conn, self.schema)
    }

    #[instrument(skip(self, cells))]
    pub fn get_placed_objects_batched(&self, cells: &[Cell]) -> anyhow::Result<Vec<PlacedObject>> {
        PlacedObject::get_for_cells(cells, &self.conn, self.schema)
    }

    #[instrument(skip(self))]
    pub fn get_worldspace_persistent_objects(
        &self,
        worldspace: &str,
    ) -> anyhow::Result<Vec<PlacedObject>> {
        PlacedObject::get_worldspace_persistent(worldspace, &self.conn, self.schema)
    }

    /// The map and heading marker refs with these editor ids, resolved in one query.
    #[instrument(skip(self))]
    pub fn get_marker_refs_by_editor_ids(
        &self,
        editor_ids: &[&str],
    ) -> anyhow::Result<Vec<MarkerRef>> {
        MarkerRef::get_by_editor_ids(editor_ids, &self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_interior_anchors(&self) -> anyhow::Result<Vec<(String, f64, f64, f64)>> {
        PlacedObject::interior_anchors(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_placed_teleports(&self) -> anyhow::Result<Vec<teleports::TeleportRecord>> {
        teleports::get_all(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_teleport_into_cell(
        &self,
        cell_form_key: &str,
    ) -> anyhow::Result<Option<teleports::TeleportRecord>> {
        teleports::get_into_cell(cell_form_key, &self.conn)
    }

    #[instrument(skip(self))]
    pub fn count_placed_teleports(&self) -> anyhow::Result<usize> {
        teleports::count(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_static(&self, form_key: &str) -> anyhow::Result<Static> {
        Static::get(form_key, &self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_statics(&self) -> anyhow::Result<Vec<Static>> {
        Static::get_all(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_furniture(&self) -> anyhow::Result<Vec<Furniture>> {
        Furniture::get_all(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_doors(&self) -> anyhow::Result<Vec<Door>> {
        Door::get_all(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_containers(&self) -> anyhow::Result<Vec<Container>> {
        Container::get_all(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_movable_statics(&self) -> anyhow::Result<Vec<MovableStatic>> {
        MovableStatic::get_all(&self.conn)
    }

    /// The MATT buoyancy rows, or `None` for a database converted before the
    /// installer extracted them.
    #[instrument(skip(self))]
    pub fn get_material_buoyancies(&self) -> anyhow::Result<Option<Vec<MaterialBuoyancyRow>>> {
        if !self.schema.material_type_columns_present {
            return Ok(None);
        }
        MaterialBuoyancyRow::get_all(&self.conn).map(Some)
    }

    /// Every weapon base with its default mod list, or `None` for a database
    /// converted before the installer extracted the default combination.
    #[instrument(skip(self))]
    pub fn get_weapons(&self) -> anyhow::Result<Option<Vec<WeaponRow>>> {
        if !self.schema.weapon_assembly_available() {
            return Ok(None);
        }
        WeaponRow::get_all(&self.conn).map(Some)
    }

    /// The model and attach point of each named modification (unknown keys
    /// have no row), or `None` for a database converted before the installer
    /// extracted them.
    #[instrument(skip(self, form_keys))]
    pub fn get_mod_models(&self, form_keys: &[String]) -> anyhow::Result<Option<Vec<ModModelRow>>> {
        if !self.schema.weapon_assembly_available() {
            return Ok(None);
        }
        ModModelRow::get_for_keys(form_keys, &self.conn).map(Some)
    }

    #[instrument(skip(self))]
    pub fn get_movement_type(&self, editor_id: &str) -> anyhow::Result<MovementType> {
        MovementType::get(editor_id, &self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_activators(&self) -> anyhow::Result<Vec<Activator>> {
        Activator::get_all(&self.conn, self.schema)
    }

    #[instrument(skip(self))]
    pub fn get_flora(&self) -> anyhow::Result<Vec<Flora>> {
        Flora::get_all(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_trees(&self) -> anyhow::Result<Vec<Tree>> {
        Tree::get_all(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_lights(&self) -> anyhow::Result<Vec<Light>> {
        Light::get_all(&self.conn, self.schema)
    }

    #[instrument(skip(self))]
    pub fn get_addon_nodes(&self) -> anyhow::Result<addon_nodes::AddonNodes> {
        Ok(addon_nodes::AddonNodes::load(&self.conn)?)
    }

    #[instrument(skip(self))]
    pub fn get_lighting_templates(&self) -> anyhow::Result<Vec<LightingTemplate>> {
        LightingTemplate::get_all(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_static_collections(&self) -> anyhow::Result<Vec<StaticCollection>> {
        StaticCollection::get_all(&self.conn)
    }

    pub fn get_worldspace_climate(&self, worldspace: &str) -> rusqlite::Result<Option<String>> {
        weather::get_worldspace_climate(worldspace, &self.conn)
    }

    pub fn get_climate_data(&self, climate: &str) -> rusqlite::Result<Option<String>> {
        weather::get_climate_data(climate, &self.conn)
    }

    pub fn get_image_space_data(&self, form_key: &str) -> rusqlite::Result<Option<String>> {
        weather::get_image_space_data(form_key, &self.conn)
    }

    pub fn get_weather(&self, form_key: &str) -> rusqlite::Result<weather::WeatherRow> {
        weather::get_weather(
            form_key,
            self.schema().weather_fog_column_present,
            &self.conn,
        )
    }

    #[instrument(skip(self))]
    pub fn get_worldspace(&self, form_key: &str) -> anyhow::Result<Worldspace> {
        Worldspace::get(form_key, &self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_worldspaces(&self) -> anyhow::Result<Vec<Worldspace>> {
        Worldspace::list_all(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_water(&self, form_key: &str) -> anyhow::Result<Water> {
        Water::get(form_key, &self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_waters(&self) -> anyhow::Result<Vec<Water>> {
        Water::get_all(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_cell_water(&self, cell_form_key: &str) -> anyhow::Result<CellWater> {
        CellWater::get(cell_form_key, &self.conn, self.schema)
    }

    #[instrument(skip(self))]
    pub fn get_worldspace_water(&self, form_key: &str) -> anyhow::Result<WorldspaceWater> {
        WorldspaceWater::get(form_key, &self.conn, self.schema)
    }

    #[instrument(skip(self))]
    pub fn get_exterior_cell_waters(
        &self,
        worldspace_form_key: &str,
    ) -> anyhow::Result<std::collections::HashMap<(i32, i32), CellWater>> {
        CellWater::exterior_with_water(worldspace_form_key, &self.conn, self.schema)
    }

    #[instrument(skip(self))]
    pub fn get_lod_water_quads(
        &self,
        worldspace_form_key: &str,
    ) -> anyhow::Result<Vec<LodWaterQuad>> {
        LodWaterQuad::for_worldspace(worldspace_form_key, &self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_placed_water_refs(
        &self,
        cell_form_key: &str,
    ) -> anyhow::Result<Vec<PlacedWaterRef>> {
        PlacedWaterRef::get_for_cell(cell_form_key, &self.conn, self.schema)
    }

    #[instrument(skip(self))]
    pub fn get_cells_with_placed_water(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<(String, usize)>> {
        PlacedWaterRef::cells_with_placed_water(limit, &self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_landscape_textures(&self) -> anyhow::Result<Vec<LandscapeTextureDiffuse>> {
        LandscapeTextureDiffuse::get_all(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_grasses(&self) -> anyhow::Result<Vec<GrassRow>> {
        GrassRow::get_all(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_ltex_grasses(&self) -> anyhow::Result<Vec<LtexGrassRow>> {
        LtexGrassRow::get_all(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_base_material_swaps(
        &self,
    ) -> anyhow::Result<std::collections::HashMap<String, String>> {
        material_swaps::get_base_material_swaps(&self.conn)
    }

    #[instrument(skip(self))]
    pub fn get_material_swap_entries(
        &self,
    ) -> anyhow::Result<
        std::collections::HashMap<String, std::sync::Arc<Vec<material_swaps::SwapEntry>>>,
    > {
        material_swaps::get_material_swap_entries(&self.conn, self.schema)
    }
}
