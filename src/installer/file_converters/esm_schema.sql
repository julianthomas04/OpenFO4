-- ============================================
-- DATABASE CONFIGURATION (run at connection)
-- ============================================
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA cache_size = -64000;  -- 64MB cache
PRAGMA temp_store = MEMORY;

-- ============================================
-- CORE TABLES
-- ============================================

-- Master registry of all records
CREATE TABLE IF NOT EXISTS records (
    form_key TEXT PRIMARY KEY,          -- e.g., '03C0F0:Fallout4.esm'
    record_type TEXT NOT NULL,          -- e.g., 'NPC_', 'WEAP', 'CELL'
    editor_id TEXT,
    source_plugin TEXT NOT NULL
);

-- ============================================
-- WORLD STRUCTURE
-- ============================================

-- Worldspaces (open world areas)
CREATE TABLE IF NOT EXISTS worldspaces (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    parent_worldspace TEXT REFERENCES worldspaces(form_key),
    flags TEXT,                         -- JSON
    world_map_data TEXT,                -- JSON
    climate TEXT REFERENCES climates(form_key),
    water_type TEXT,                    -- NAM2: default WATR form_key of the worldspace
    lod_water_type TEXT,                -- NAM3: WATR form_key used by distant LOD water
    lod_water_height REAL,              -- NAM4: LOD water plane height, NULL when absent
    default_land_height REAL,           -- DNAM: land height of cells without LAND data
    default_water_height REAL,          -- DNAM: water plane height of cells without an XCLW override
    water_environment_map TEXT,         -- XWEM: water cubemap path
    parent_flags INTEGER,               -- PNAM raw u16: which data the worldspace inherits from parent_worldspace (0x01 land, 0x02 LOD, 0x04 map, 0x08 water, 0x10 climate, 0x40 sky cell)
    map_image TEXT                      -- ICON: world map texture path relative to Textures, NULL when the worldspace has none
);

-- Map-marker references of a worldspace's persistent cell (Pip-Boy MAP).
CREATE TABLE IF NOT EXISTS map_markers (
    form_key TEXT PRIMARY KEY,
    worldspace TEXT NOT NULL REFERENCES worldspaces(form_key),
    name TEXT,                          -- FULL, resolved
    marker_type INTEGER NOT NULL,       -- TNAM raw u16
    flags INTEGER NOT NULL,             -- FNAM raw u8
    position_x REAL NOT NULL,           -- game units
    position_y REAL NOT NULL,
    position_z REAL NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_map_markers_worldspace ON map_markers(worldspace);

-- Distant-LOD water: one row per 4096-unit quad of the level-4 terrain LOD (BTR) water shapes, read from the
-- game archives by LodWaterStage (not from the ESM). Height is the baked vertex z in game units.
CREATE TABLE IF NOT EXISTS lod_water_quads (
    worldspace TEXT NOT NULL REFERENCES worldspaces(form_key),
    cell_x INTEGER NOT NULL,
    cell_y INTEGER NOT NULL,
    height REAL NOT NULL,
    PRIMARY KEY (worldspace, cell_x, cell_y)
) WITHOUT ROWID;

-- Cells (interior/exterior locations)
CREATE TABLE IF NOT EXISTS cells (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    worldspace TEXT REFERENCES worldspaces(form_key),
    grid_x INTEGER,
    grid_y INTEGER,
    is_interior INTEGER DEFAULT 0,      -- Boolean: 0 or 1
    is_worldspace_persistent_cell INTEGER DEFAULT 0,  -- 1 = WRLD.TopCell (holds worldspace-wide persistent refs, not a real grid cell)
    flags TEXT,                         -- JSON
    lighting TEXT,                      -- JSON: Mutagen CellLighting (XCLL)
    lighting_template TEXT,             -- FormKey string ("0300E2:Fallout4.esm") of the LTMP lighting template; NULL when unset
    encounter_zone TEXT,
    has_water INTEGER DEFAULT 0,        -- cell flag HasWater (0x2): the CK "Has Water" checkbox; interiors only carry water when it is set
    no_lod_water INTEGER DEFAULT 0,     -- cell flag NoLodWater: no distant LOD water over this cell
    water_height REAL,                  -- XCLW raw: 3.4028235E+38 (float max, 0x7F7FFFFF) is the "no value" sentinel; NULL = no XCLW subrecord
    water_type TEXT,                    -- XCWT: WATR form_key overriding the worldspace water type
    water_environment_map TEXT,         -- XWEM: water cubemap path
    water_currents TEXT                 -- JSON [[x,y,z,w],...]: XWCN count + XWCU entries (16 B each), NULL when absent
);

CREATE INDEX IF NOT EXISTS idx_cells_worldspace ON cells(worldspace);
CREATE INDEX IF NOT EXISTS idx_cells_grid ON cells(grid_x, grid_y);
CREATE INDEX IF NOT EXISTS idx_cells_ws_grid ON cells(worldspace, grid_x, grid_y);
CREATE INDEX IF NOT EXISTS idx_cells_wpc ON cells(is_worldspace_persistent_cell);

-- Placed objects in cells
CREATE TABLE IF NOT EXISTS placed_objects (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    cell_form_key TEXT REFERENCES cells(form_key),
    base_object TEXT NOT NULL,          -- What object is placed
    object_type TEXT NOT NULL,          -- Type of placement (PlacedNpc, PlacedObject, etc.)
    is_persistent INTEGER DEFAULT 0,
    position_x REAL,
    position_y REAL,
    position_z REAL,
    rotation_x REAL,
    rotation_y REAL,
    rotation_z REAL,
    scale REAL DEFAULT 1.0,
    is_initially_disabled INTEGER DEFAULT 0,
    enable_parent TEXT,
    enable_parent_opposite INTEGER DEFAULT 0,
    initially_hidden INTEGER DEFAULT 0,  -- resolved initial enable state (InitiallyDisabled + XESP chain); the engine's spawn filter
    is_in_precombined INTEGER DEFAULT 0,
    has_primitive INTEGER DEFAULT 0,    -- REFR has an XPRM subrecord: trigger volume / sound zone / fog volume
    material_swap TEXT,                 -- XMSP: MSWP form_key overriding the base record's swap
    radius_override REAL,               -- XRDS: radius of a light ref, NULL when absent
    light_fov_pm REAL,                  -- XLIG: FOV 90 +/-, NULL when the ref has no XLIG
    light_fade_pm REAL,                 -- XLIG: fade 1 +/-
    light_end_distance_cap REAL,        -- XLIG: end distance cap
    light_shadow_depth_bias REAL,       -- XLIG: shadow depth bias
    light_near_clip REAL,               -- XLIG: near clip
    light_volumetric_intensity REAL,    -- XLIG: volumetric intensity
    record_flags INTEGER,               -- raw major record flags (CastsShadows 0x200, NeverFades 0x10000, ...)
    ragdoll_pose BLOB,                  -- XRGD: raw N x 28-byte ragdoll pose, NULL when absent
    water_currents TEXT,                -- JSON [[x,y,z,w],...]: XWCN count + XWCU entries (16 B each), NULL when absent
    extra_data TEXT                     -- JSON
);

CREATE INDEX IF NOT EXISTS idx_placed_cell ON placed_objects(cell_form_key);
CREATE INDEX IF NOT EXISTS idx_placed_base ON placed_objects(base_object);

-- Teleport-door destinations (REFR XTEL). One row per source door REFR. Read only on
-- door activation (cold path), never during cell streaming -- a side table so the hot
-- placed_objects read stays narrow. destination_ref resolves to placed_objects.cell_form_key
-- -> cells (is_interior / worldspace) at load time. destination_ref is intentionally NOT a
-- foreign key: it is resolved across separate CellProcessor passes (the destination door
-- usually lives in a different cell, inserted by a later pass), and may be a deleted/absent
-- ref the engine then simply drops. The dest position/rotation are always written (Mutagen
-- P3Float, never null), so they are NOT NULL.
CREATE TABLE IF NOT EXISTS placed_teleports (
    source_ref      TEXT PRIMARY KEY REFERENCES placed_objects(form_key),
    destination_ref TEXT NOT NULL,
    dest_position_x REAL NOT NULL,
    dest_position_y REAL NOT NULL,
    dest_position_z REAL NOT NULL,
    dest_rotation_x REAL NOT NULL,
    dest_rotation_y REAL NOT NULL,
    dest_rotation_z REAL NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_teleports_dest ON placed_teleports(destination_ref);

-- ============================================
-- CHARACTERS
-- ============================================

-- NPCs
CREATE TABLE IF NOT EXISTS npcs (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    race TEXT,
    class TEXT,
    voice_type TEXT,
    health INTEGER,
    magicka INTEGER,
    stamina INTEGER,
    level INTEGER,
    disposition INTEGER,
    aggression INTEGER,
    confidence INTEGER,
    flags TEXT,                         -- JSON
    appearance TEXT,                    -- JSON: Face morphs, hair, etc.
    ai_data TEXT                        -- JSON
);

CREATE INDEX IF NOT EXISTS idx_npcs_race ON npcs(race);
CREATE INDEX IF NOT EXISTS idx_npcs_class ON npcs(class);

-- Factions
CREATE TABLE IF NOT EXISTS factions (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    flags TEXT,                         -- JSON
    ranks TEXT                          -- JSON
);

-- ============================================
-- RACES & CHARACTER CUSTOMIZATION
-- ============================================

CREATE TABLE IF NOT EXISTS races (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    description TEXT,
    spells TEXT,                        -- JSON
    skin TEXT,
    body_data TEXT,                     -- JSON
    flags TEXT,                         -- JSON
    starting_stats TEXT,                -- JSON
    size TEXT                           -- JSON: Height, weight
);

-- ============================================
-- ITEMS
-- ============================================

-- Base items table (for common fields)
CREATE TABLE IF NOT EXISTS items (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    item_type TEXT NOT NULL,            -- WEAP, ARMO, MISC, etc.
    editor_id TEXT,
    name TEXT,
    value INTEGER DEFAULT 0,
    weight REAL DEFAULT 0,
    model_path TEXT,
    pickup_sound TEXT,
    drop_sound TEXT,
    keywords TEXT                       -- JSON
);

CREATE INDEX IF NOT EXISTS idx_items_type ON items(item_type);
CREATE INDEX IF NOT EXISTS idx_items_name ON items(name);

-- Weapons (extends items)
CREATE TABLE IF NOT EXISTS weapons (
    form_key TEXT PRIMARY KEY REFERENCES items(form_key),
    weapon_type TEXT,
    damage INTEGER,
    attack_speed REAL,
    reach REAL,
    crit_damage INTEGER,
    crit_multiplier REAL,
    ammo_type TEXT,
    magazine_size INTEGER,
    fire_rate REAL,
    range REAL,
    accuracy REAL,
    aim_model TEXT,
    enchantment TEXT,
    template TEXT,
    default_mods TEXT,                  -- JSON array of OMOD form keys (default object-template combination)
    object_template TEXT                -- JSON array of every object-template combination
);

-- Armor (extends items)
CREATE TABLE IF NOT EXISTS armors (
    form_key TEXT PRIMARY KEY REFERENCES items(form_key),
    armor_rating INTEGER,
    body_slot INTEGER,
    armor_type TEXT,                    -- Light, Heavy, Clothing
    race TEXT,
    enchantment TEXT,
    template TEXT
);

-- Object modifications (weapon/armor mods)
CREATE TABLE IF NOT EXISTS object_modifications (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    mod_type TEXT,
    target_type TEXT,                   -- WEAP, ARMO
    attach_point TEXT,
    includes TEXT,                      -- JSON: Modifications included
    properties TEXT,                    -- JSON: Stat changes
    model_path TEXT,                    -- lower-case Meshes-relative .nif path, NULL for null mods
    attach_point_name TEXT,             -- editor id of the attach point keyword
    attach_parent_slots TEXT            -- JSON array of keyword editor ids
);

-- ============================================
-- QUESTS
-- ============================================

CREATE TABLE IF NOT EXISTS quests (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    priority INTEGER DEFAULT 0,
    quest_type TEXT,
    flags TEXT,                         -- JSON
    conditions TEXT                     -- JSON
);

-- ============================================
-- MAGIC & EFFECTS
-- ============================================

CREATE TABLE IF NOT EXISTS magic_effects (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    description TEXT,
    base_cost REAL,
    archetype TEXT,
    resist_value TEXT,
    flags TEXT,                         -- JSON
    properties TEXT                     -- JSON
);

CREATE TABLE IF NOT EXISTS spells (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    spell_type TEXT,
    cast_type TEXT,
    delivery TEXT,
    charge_time REAL,
    cast_duration REAL,
    range REAL,
    flags TEXT                          -- JSON
);

CREATE TABLE IF NOT EXISTS perks (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    description TEXT,
    perk_type TEXT,
    next_perk TEXT,
    flags TEXT,                         -- JSON
    conditions TEXT,                    -- JSON
    effects TEXT,                       -- JSON
    level INTEGER,                      -- player level this rank requires
    num_ranks INTEGER,                  -- as authored; unreliable, count ranks by walking next_perk from first_rank
    playable INTEGER,                   -- 0/1
    hidden INTEGER,                     -- 0/1
    swf_path TEXT,                      -- Vault Boy movie, Interface-relative: as authored, or derived from the form id for chart perks without one
    first_rank TEXT                     -- form key of the first rank of this record's rank chain (the record itself for a first rank)
);

-- Perk chart placement: one row per chart perk, keyed by the first-rank record (perks.first_rank = perks.form_key).
CREATE TABLE IF NOT EXISTS perk_chart (
    perk TEXT PRIMARY KEY REFERENCES perks(form_key),
    special TEXT NOT NULL CHECK (special IN ('strength', 'perception', 'endurance', 'charisma', 'intelligence', 'agility', 'luck')),
    required_special INTEGER NOT NULL CHECK (required_special BETWEEN 1 AND 10)
);

-- The seven SPECIAL attribute records (AVIF): the name and description the Pip-Boy shows.
CREATE TABLE IF NOT EXISTS special_attributes (
    special TEXT PRIMARY KEY CHECK (special IN ('strength', 'perception', 'endurance', 'charisma', 'intelligence', 'agility', 'luck')),
    form_key TEXT NOT NULL REFERENCES actor_value_informations(form_key),
    name TEXT,
    description TEXT
);

-- ============================================
-- AUDIO
-- ============================================

CREATE TABLE IF NOT EXISTS sound_descriptors (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    category TEXT,                      -- GNAM sound category form key
    output_model TEXT,
    alternate_sounds TEXT,              -- JSON
    sound_files TEXT,                   -- JSON array of the ANAM paths in file order
    descriptor_type INTEGER,            -- CNAM u32: 0x1EEF540A Standard, 0x54651A43 Compound, 0xED157AE3 Autoweapon
    base_descriptor TEXT,               -- Autoweapon only: the 4-byte BNAM descriptor
    freq_shift_pct INTEGER,             -- Standard BNAM s8
    freq_variance_pct INTEGER,          -- Standard BNAM s8
    priority INTEGER,                   -- Standard BNAM u8
    db_variance INTEGER,                -- Standard BNAM u8
    static_attenuation_centidb INTEGER, -- Standard BNAM u16, hundredths of a decibel
    looping_flags INTEGER               -- LNAM u32 as stored (little-endian): byte 0 unknown, byte 1 looping type, byte 2 sidechain, byte 3 rumble send
);

CREATE TABLE IF NOT EXISTS sound_descriptor_children (
    descriptor TEXT NOT NULL,           -- sound_descriptors.form_key of the compound descriptor
    ord INTEGER NOT NULL,               -- position in the DNAM list
    child TEXT NOT NULL,                -- sound_descriptors.form_key
    PRIMARY KEY (descriptor, ord)
);

CREATE TABLE IF NOT EXISTS sound_descriptor_conditions (
    descriptor TEXT NOT NULL,           -- sound_descriptors.form_key
    ord INTEGER NOT NULL,
    function INTEGER NOT NULL,
    compare_op INTEGER NOT NULL,
    or_flag INTEGER NOT NULL,
    flags INTEGER NOT NULL DEFAULT 0,
    compare_value REAL,
    compare_global TEXT,
    run_on INTEGER NOT NULL,
    reference TEXT,
    param1_num INTEGER,
    param1_record TEXT,
    param1_string TEXT,
    param2_num INTEGER,
    param2_record TEXT,
    param2_string TEXT,
    param3 INTEGER,
    PRIMARY KEY (descriptor, ord)
);

CREATE TABLE IF NOT EXISTS footstep_set_entries (
    set_key TEXT NOT NULL,              -- footstep_sets.form_key
    gait TEXT NOT NULL,                 -- walk, run, sprint, sneak, swim
    ord INTEGER NOT NULL,               -- position inside the gait group
    footstep TEXT NOT NULL,             -- footsteps.form_key
    PRIMARY KEY (set_key, gait, ord)
);

CREATE TABLE IF NOT EXISTS footsteps (
    form_key TEXT PRIMARY KEY,
    editor_id TEXT,
    tag TEXT,                           -- ANAM animation event name
    impact_data_set TEXT                -- DATA impact_data_sets.form_key
);

CREATE TABLE IF NOT EXISTS impact_data_set_entries (
    set_key TEXT NOT NULL,              -- impact_data_sets.form_key
    ord INTEGER NOT NULL,
    material TEXT NOT NULL,             -- material_types.form_key
    impact TEXT NOT NULL,               -- impacts.form_key
    PRIMARY KEY (set_key, ord)
);

CREATE TABLE IF NOT EXISTS impacts (
    form_key TEXT PRIMARY KEY,
    editor_id TEXT,
    player_sound TEXT,                  -- NAM1 sound_descriptors.form_key
    npc_sound TEXT                      -- SNAM sound_descriptors.form_key
);

CREATE TABLE IF NOT EXISTS default_object_slots (
    manager TEXT NOT NULL,              -- default_object_managers.form_key
    slot TEXT NOT NULL,                 -- four-character key such as DFTS
    target TEXT,                        -- form key; NULL when the slot is empty
    PRIMARY KEY (manager, slot)
);

-- Ambient music: blobs, flag layouts and the engine's reading rules.
CREATE TABLE IF NOT EXISTS music_types (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    flags INTEGER,                      -- raw MUSC FNAM bit set
    priority INTEGER,                   -- PNAM: lower number wins
    ducking_centidb INTEGER,            -- PNAM: raw u16, hundredths of a decibel
    fade_seconds REAL                   -- WNAM
);

CREATE TABLE IF NOT EXISTS music_type_tracks (
    music_type TEXT NOT NULL REFERENCES music_types(form_key),
    ord INTEGER NOT NULL,               -- position in the TNAM list; the same track may repeat
    track TEXT NOT NULL REFERENCES music_tracks(form_key),
    PRIMARY KEY (music_type, ord)
);

CREATE TABLE IF NOT EXISTS music_tracks (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    track_type TEXT,                    -- Single, Palette, Silent
    duration REAL,                      -- FLTV seconds
    fade_out REAL,                      -- DNAM seconds
    file_key TEXT,                      -- audio output key of ANAM (Single only), below assets/audio
    anam_path TEXT                      -- ANAM exactly as the record spells it
);

CREATE TABLE IF NOT EXISTS music_track_layers (
    track TEXT NOT NULL REFERENCES music_tracks(form_key),
    layer INTEGER NOT NULL,             -- 0-based, SNAM split at the null entries
    ord INTEGER NOT NULL,               -- 0-based position inside the layer
    sub_track TEXT NOT NULL REFERENCES music_tracks(form_key),
    PRIMARY KEY (track, layer, ord)
);

CREATE TABLE IF NOT EXISTS music_track_conditions (
    track TEXT NOT NULL REFERENCES music_tracks(form_key),
    ord INTEGER NOT NULL,
    function INTEGER NOT NULL,          -- condition function index
    compare_op INTEGER NOT NULL,        -- 0 ==, 1 !=, 2 >, 3 >=, 4 <, 5 <=
    or_flag INTEGER NOT NULL,           -- 1 when this condition is OR-ed with the next
    flags INTEGER NOT NULL DEFAULT 0,   -- other CTDA type bits: 2 parameters use aliases, 8 use pack data, 16 swap subject and target
    compare_value REAL,                 -- set for a literal comparison
    compare_global TEXT,                -- set instead when the comparison value is a global form key
    run_on INTEGER NOT NULL,            -- 0 Subject, 1 Target, 2 Reference, ...
    reference TEXT,
    param1_num INTEGER,
    param1_record TEXT,
    param1_string TEXT,
    param2_num INTEGER,
    param2_record TEXT,
    param2_string TEXT,
    param3 INTEGER,
    PRIMARY KEY (track, ord)
);

CREATE TABLE IF NOT EXISTS cell_places (
    cell TEXT PRIMARY KEY REFERENCES cells(form_key),
    music_type TEXT,                    -- XCMO, NULL when the cell names no music
    location TEXT                       -- XLCN, NULL when the cell names no location
);

CREATE TABLE IF NOT EXISTS location_music (
    location TEXT PRIMARY KEY REFERENCES locations(form_key),
    music_type TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS worldspace_music (
    worldspace TEXT PRIMARY KEY REFERENCES worldspaces(form_key),
    music_type TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS region_music (
    region TEXT PRIMARY KEY REFERENCES regions(form_key),
    music_type TEXT NOT NULL,           -- RDMO
    priority INTEGER NOT NULL,          -- RDAT priority of the sound block
    override INTEGER NOT NULL,          -- RDAT flags bit 0
    record_order INTEGER NOT NULL       -- 0-based position of the REGN record in the plugin's record order
);

CREATE TABLE IF NOT EXISTS region_areas (
    region TEXT NOT NULL REFERENCES regions(form_key),
    area INTEGER NOT NULL,              -- 0-based RPLI/RPLD pair
    edge_falloff INTEGER,               -- RPLI, raw units
    points BLOB NOT NULL,               -- little-endian f32 x,y pairs, raw units
    PRIMARY KEY (region, area)
);

-- ============================================
-- VISUALS
-- ============================================

CREATE TABLE IF NOT EXISTS texture_sets (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    diffuse TEXT,
    normal TEXT,
    specular TEXT,
    glow TEXT,
    environment_mask TEXT,
    material TEXT,                      -- raw MNAM path (BGSM/BGEM)
    flags TEXT                          -- JSON
);

CREATE TABLE IF NOT EXISTS material_objects (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    model_path TEXT,
    material_file TEXT,                 -- .bgsm or .bgem file path
    properties TEXT                     -- JSON
);

CREATE TABLE IF NOT EXISTS weather (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    precipitation TEXT,
    fog TEXT,                           -- JSON
    colors TEXT,                        -- JSON
    image_spaces TEXT,                  -- JSON
    sounds TEXT,                        -- JSON
    god_rays TEXT,
    directional_ambient TEXT            -- JSON
);

-- ============================================
-- GAME SETTINGS
-- ============================================

CREATE TABLE IF NOT EXISTS game_settings (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT UNIQUE,
    value_type TEXT,                    -- int, float, string
    value_int INTEGER,
    value_float REAL,
    value_string TEXT
);

CREATE TABLE IF NOT EXISTS globals (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT UNIQUE,
    value_type TEXT,
    value REAL
);

CREATE TABLE IF NOT EXISTS keywords (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    color TEXT                          -- JSON
);

-- Form lists
CREATE TABLE IF NOT EXISTS form_lists (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    entries TEXT                        -- JSON: Array of FormKeys
);

-- ============================================
-- LOCATIONS
-- ============================================

CREATE TABLE IF NOT EXISTS locations (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    parent_location TEXT REFERENCES locations(form_key),
    faction TEXT,
    keywords TEXT,                      -- JSON
    flags TEXT                          -- JSON
);

CREATE INDEX IF NOT EXISTS idx_locations_parent ON locations(parent_location);

CREATE TABLE IF NOT EXISTS encounter_zones (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    owner_faction TEXT,
    min_level INTEGER,
    max_level INTEGER,
    flags TEXT                          -- JSON
);

-- ============================================
-- STATICS & WORLD OBJECTS
-- ============================================

CREATE TABLE IF NOT EXISTS statics (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    model_path TEXT,
    material_object TEXT,
    has_distant_lod INTEGER DEFAULT 0,
    bounds TEXT,                        -- JSON: min/max coordinates
    alternate_textures TEXT,            -- JSON: [{name, texture_set}] shape→TXST overrides
    is_marker INTEGER DEFAULT 0,        -- ESM major record flag 0x800000: CK-only, never rendered in-game
    lod_model_path TEXT                 -- lod\...\_lod.nif below the meshes folder; backfilled post-conversion from on-disk GLBs
);

CREATE TABLE IF NOT EXISTS static_collections (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    model_path TEXT,                    -- SCOL\<plugin>\CM<8-hex-formid>.nif merged combo mesh
    is_marker INTEGER DEFAULT 0,        -- ESM major record flag 0x800000
    parts TEXT                          -- JSON: Array of static FormKeys with positions
);

CREATE TABLE IF NOT EXISTS movable_statics (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    model_path TEXT,
    destruction_data TEXT,              -- JSON
    is_marker INTEGER DEFAULT 0         -- ESM major record flag 0x800000
);

CREATE TABLE IF NOT EXISTS activators (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    model_path TEXT,
    script TEXT,
    activation_sound TEXT,
    flags TEXT,                         -- JSON
    is_marker INTEGER DEFAULT 0,        -- ESM major record flag 0x800000
    water_type TEXT                     -- WNAM: WATR form_key of a placed water mesh (NIF with BSWaterShaderProperty)
);

CREATE TABLE IF NOT EXISTS talking_activators (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    model_path TEXT,
    voice_type TEXT,
    object TEXT
);

CREATE TABLE IF NOT EXISTS containers (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    model_path TEXT,
    open_sound TEXT,
    close_sound TEXT,
    flags TEXT,                         -- JSON
    items TEXT,                         -- JSON: Default inventory
    is_marker INTEGER DEFAULT 0         -- ESM major record flag 0x800000
);

CREATE TABLE IF NOT EXISTS doors (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    model_path TEXT,
    open_sound TEXT,
    close_sound TEXT,
    flags TEXT,                         -- JSON
    is_marker INTEGER DEFAULT 0,        -- ESM major record flag 0x800000
    bounds TEXT,                        -- JSON: min/max coordinates (OBND)
    trigger_bounds TEXT                 -- JSON: min/max coordinates of the model's editor-marker geometry, automatic doors only
);

CREATE TABLE IF NOT EXISTS lights (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    model_path TEXT,
    radius REAL,
    color INTEGER,                      -- RGB packed
    flags INTEGER,                      -- raw DATA flag bits
    fade_value REAL,
    falloff_exponent REAL,
    is_marker INTEGER DEFAULT 0,        -- ESM major record flag 0x800000
    constant REAL,                      -- attenuation: C in 1 - saturate(C + S * (d/r)^E)
    scalar REAL,
    exponent REAL,
    fov REAL,
    near_clip REAL,
    flicker_period REAL,
    flicker_intensity_amplitude REAL,
    flicker_movement_amplitude REAL,
    gobo TEXT,
    lens TEXT,                          -- LENS form_key
    god_rays TEXT,                      -- GDRY form_key
    god_rays_near_clip REAL
);

CREATE TABLE IF NOT EXISTS addon_nodes (
    form_key TEXT PRIMARY KEY,
    editor_id TEXT,
    model_path TEXT,                    -- MODL, lowercased by ModelPathNormalizer
    node_index INTEGER,                 -- DATA, matches BSValueNode value
    light TEXT,                         -- LNAM -> lights.form_key, NULL if none
    sound TEXT,                         -- SNAM form_key, NULL if none
    master_particle_system_cap INTEGER, -- DNAM u16
    flags INTEGER                       -- DNAM u16 raw
);

CREATE TABLE IF NOT EXISTS furniture (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    model_path TEXT,
    furniture_flags TEXT,               -- JSON
    markers TEXT,                       -- JSON: Sit/sleep positions
    is_marker INTEGER DEFAULT 0         -- ESM major record flag 0x800000
);

-- ============================================
-- LANDSCAPE & TEXTURES
-- ============================================


CREATE TABLE IF NOT EXISTS landscape_textures (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    texture_set TEXT,
    material_type TEXT,
    friction INTEGER,
    restitution INTEGER,
    havok_data TEXT                     -- JSON
);

CREATE TABLE IF NOT EXISTS ltex_grasses (
    ltex_form_key TEXT NOT NULL,        -- landscape_textures.form_key
    grass_form_key TEXT NOT NULL,       -- grasses.form_key
    ord INTEGER NOT NULL,               -- GNAM order within the LTEX
    PRIMARY KEY (ltex_form_key, ord)
);

CREATE TABLE IF NOT EXISTS landscapes (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    cell TEXT REFERENCES cells(form_key),
    vertex_height_map BLOB,             -- Binary: 4-byte f32 offset + 1089 i8 deltas (33x33)
    vertex_colors BLOB,                 -- Binary: 3267 bytes (33x33x3 RGB)
    layers BLOB,                        -- Binary: u16 count + per-layer records with raw VTXT alpha
    flags TEXT                          -- JSON
);
CREATE INDEX IF NOT EXISTS idx_landscapes_cell ON landscapes(cell);

-- ============================================
-- LEVELED LISTS
-- ============================================

CREATE TABLE IF NOT EXISTS leveled_items (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    chance_none INTEGER DEFAULT 0,
    flags TEXT,                         -- JSON
    entries TEXT                        -- JSON: [{ref, level, count}, ...]
);

CREATE TABLE IF NOT EXISTS leveled_npcs (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    chance_none INTEGER DEFAULT 0,
    flags TEXT,                         -- JSON
    entries TEXT                        -- JSON: [{ref, level, count}, ...]
);

-- ============================================
-- CRAFTING
-- ============================================

CREATE TABLE IF NOT EXISTS constructible_objects (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    created_object TEXT,                -- FormKey of output item
    created_count INTEGER DEFAULT 1,
    workbench_keyword TEXT,             -- FormKey
    components TEXT                     -- JSON: [{item, count}, ...]
);

CREATE TABLE IF NOT EXISTS components (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    scrap_item TEXT,                    -- FormKey
    scrap_scalar REAL
);

-- ============================================
-- WORLD OBJECTS (plants, etc.)
-- ============================================

CREATE TABLE IF NOT EXISTS flora (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    model_path TEXT,
    harvest_sound TEXT,                 -- FormKey
    ingredient TEXT,                    -- FormKey (what it yields)
    flags TEXT,                         -- JSON
    is_marker INTEGER DEFAULT 0         -- ESM major record flag 0x800000
);

CREATE TABLE IF NOT EXISTS trees (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    model_path TEXT,
    ingredient TEXT,                    -- FormKey
    is_marker INTEGER DEFAULT 0         -- ESM major record flag 0x800000
);

CREATE TABLE IF NOT EXISTS grasses (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    model_path TEXT,
    flags INTEGER,                      -- raw DATA bitfield: 0x1 vertex lighting, 0x2 uniform scaling, 0x4 fit to slope
    density INTEGER,                    -- u8 percent
    min_slope INTEGER,                  -- u8 degrees
    max_slope INTEGER,                  -- u8 degrees
    units_from_water INTEGER,           -- u16
    units_from_water_type INTEGER,      -- raw u32 rule: 0-5 filter by water height, >= 6 no filter
    position_range REAL,
    height_range REAL,
    color_range REAL,
    wave_period REAL
);

-- ============================================
-- NPC SUPPORT TYPES
-- ============================================

CREATE TABLE IF NOT EXISTS classes (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    description TEXT,
    flags TEXT                          -- JSON
);

CREATE TABLE IF NOT EXISTS combat_styles (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    flags TEXT,                         -- JSON
    data TEXT                           -- JSON: all CS sub-records
);

CREATE TABLE IF NOT EXISTS equip_types (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    flags TEXT                          -- JSON
);

CREATE TABLE IF NOT EXISTS voice_types (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    flags TEXT                          -- JSON
);

CREATE TABLE IF NOT EXISTS impact_data_sets (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT
);

CREATE TABLE IF NOT EXISTS explosions (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    model_path TEXT,
    light TEXT,                         -- FormKey
    sound1 TEXT,                        -- FormKey
    sound2 TEXT,                        -- FormKey
    inner_radius REAL,
    outer_radius REAL,
    damage REAL,
    flags TEXT                          -- JSON
);

CREATE TABLE IF NOT EXISTS projectiles (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    model_path TEXT,
    flags TEXT,                         -- JSON
    gravity REAL,
    speed REAL,
    range REAL,
    explosion TEXT,                     -- FormKey
    light TEXT,                         -- FormKey
    muzzle_flash_light TEXT             -- FormKey
);

CREATE TABLE IF NOT EXISTS hazards (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    model_path TEXT,
    radius REAL,
    duration REAL,
    target_limit INTEGER,
    spell TEXT,                         -- FormKey
    light TEXT,                         -- FormKey
    flags TEXT                          -- JSON
);

-- ============================================
-- ENCHANTMENTS / OBJECT EFFECTS
-- ============================================

CREATE TABLE IF NOT EXISTS object_effects (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    flags TEXT,                         -- JSON
    effects TEXT                        -- JSON: [{effect_fk, magnitude, area, duration}]
);

-- ============================================
-- OUTFITS
-- ============================================

CREATE TABLE IF NOT EXISTS outfits (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    items TEXT                          -- JSON: [FormKey, ...]
);

-- ============================================
-- RELATIONSHIPS
-- ============================================

CREATE TABLE IF NOT EXISTS relationships (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    parent TEXT,                        -- FormKey (NPC)
    child TEXT,                         -- FormKey (NPC)
    association_type TEXT,              -- FormKey
    flags TEXT                          -- JSON
);

-- ============================================
-- VISUAL / LIGHTING
-- ============================================

CREATE TABLE IF NOT EXISTS lighting_templates (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    data TEXT                           -- JSON: Mutagen LightingTemplate lighting fields (same serializer as cells.lighting; see field-name differences in esm_loader.cs LightingTemplateProcessor)
);

CREATE TABLE IF NOT EXISTS image_spaces (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    data TEXT                           -- JSON
);

CREATE TABLE IF NOT EXISTS visual_effects (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    model_path TEXT,
    data TEXT                           -- JSON
);

-- ============================================
-- WATER / CLIMATE
-- ============================================

CREATE TABLE IF NOT EXISTS waters (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    material TEXT,                      -- FormKey
    data TEXT                           -- JSON: every Mutagen IWaterGetter field (WaterJson.Project)
);

CREATE TABLE IF NOT EXISTS climates (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    sun_texture TEXT,
    sun_glare_texture TEXT,
    data TEXT                           -- JSON: WLST, FNAM, GNAM, TNAM
);

-- ============================================
-- NPC CUSTOMIZATION
-- ============================================

CREATE TABLE IF NOT EXISTS head_parts (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    type TEXT,
    flags TEXT,                         -- JSON
    model_path TEXT,
    texture_set TEXT                    -- FormKey
);

CREATE TABLE IF NOT EXISTS packages (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    flags TEXT,                         -- JSON
    procedure_type TEXT,
    interrupt_override TEXT,
    data TEXT                           -- JSON: conditions etc.
);

-- ============================================
-- AUDIO (EXTENDED)
-- ============================================

CREATE TABLE IF NOT EXISTS sound_categories (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    flags TEXT,                         -- JSON
    parent TEXT,                        -- FormKey (parent category)
    static_attenuation REAL,            -- VNAM static volume multiplier, 0..1 (stored u16 / 65535); NULL when absent
    default_menu_volume REAL            -- UNAM default menu value, 0..1 (stored u16 / 65535); NULL when absent
);

CREATE TABLE IF NOT EXISTS sound_output_models (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    data TEXT,                          -- JSON, unused
    output_type INTEGER,                -- MNAM
    static_attenuation_db REAL          -- VNAM / 100
);

CREATE TABLE IF NOT EXISTS sound_output_channels (
    model TEXT NOT NULL,                -- sound_output_models.form_key
    ord INTEGER NOT NULL,               -- input channel 0..2 of the ONAM matrix
    speaker TEXT NOT NULL,              -- L, R, C, LFE, RL, RR, BL, BR
    percent INTEGER NOT NULL,           -- share of the input channel sent to the speaker
    PRIMARY KEY (model, ord, speaker)
);

-- ============================================
-- WORLD / REGIONS
-- ============================================

CREATE TABLE IF NOT EXISTS regions (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    worldspace TEXT,                    -- FormKey
    objects TEXT,                       -- JSON: region data objects
    flags TEXT                          -- JSON
);

-- ============================================
-- MESSAGES / TERMINALS / HOLOTAPES
-- ============================================

CREATE TABLE IF NOT EXISTS messages (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    title TEXT,
    body TEXT,
    flags TEXT                          -- JSON
);

CREATE TABLE IF NOT EXISTS terminals (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    description TEXT,
    flags TEXT                          -- JSON
);

-- ============================================
-- NAVIGATION
-- ============================================

CREATE TABLE IF NOT EXISTS material_types (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    material_name TEXT,                 -- MNAM, the Havok material name the shapes' hash derives from
    material_hash INTEGER,              -- CRC-32 of lower-case MNAM (unsigned 32-bit): the key the collision sidecars carry
    buoyancy REAL,                      -- BNAM, a required subrecord (all 147 vanilla records carry it)
    flags INTEGER,                      -- FNAM: 0x1 stair material, 0x2 arrows stick, 0x4 can tunnel
    parent TEXT                         -- PNAM form key
);

CREATE TABLE IF NOT EXISTS movement_types (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    name TEXT,
    data TEXT                           -- JSON
);

-- ============================================
-- WAVE 4 — minimal tracking tables (form_key + editor_id)
-- ============================================

CREATE TABLE IF NOT EXISTS acoustic_spaces           (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS action_records            (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS actor_value_informations  (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS aim_models                (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS animated_objects          (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS animation_sound_tag_sets  (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS armor_addons              (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS art_objects               (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS association_types         (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS attraction_rules          (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS audio_category_snapshots  (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS audio_effect_chains       (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS bendable_splines          (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS body_part_datas           (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS camera_paths              (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS camera_shots              (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS collision_layers (
    form_key TEXT PRIMARY KEY REFERENCES records(form_key),
    editor_id TEXT,
    layer_index INTEGER,                -- COLL BNAM layer index (matches the layer id Havok bodies carry)
    flags INTEGER,                      -- COLL FNAM raw flags
    collides_with TEXT                  -- JSON: Array of collision layer form keys (CNAM)
);
CREATE TABLE IF NOT EXISTS color_records             (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS debrises                  (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS default_object_managers   (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS default_objects           (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS dialog_branches           (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS dialog_responseses        (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS dialog_topics             (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS dialog_views              (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS dual_cast_datas           (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS effect_shaders            (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS footstep_sets             (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS god_rayses                (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS idle_animations           (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS idle_markers              (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS image_space_adapters      (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS instance_naming_ruleses   (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS layers                    (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS lens_flares               (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS load_screens              (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS location_reference_types  (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS material_swaps            (form_key TEXT PRIMARY KEY, editor_id TEXT);

-- MSWP substitutions, pre-resolved at install time: original_material is the
-- normalized census key matched against GLB "src:" material-name tokens;
-- diffuse/normal are final deployed texture paths (palette-baked variant
-- filenames already substituted). Numeric alpha fields mirror the GLB
-- material-name token grammar so the engine parses no strings.
CREATE TABLE IF NOT EXISTS material_swap_entries (
    swap_form_key TEXT NOT NULL REFERENCES material_swaps(form_key),
    original_material TEXT NOT NULL,    -- "materials/.../foo.bgsm"
    replacement_material TEXT,          -- census key of the replacement (diagnostic)
    diffuse TEXT,
    normal TEXT,
    is_blend INTEGER DEFAULT 0,
    alpha_mult REAL DEFAULT 1.0,
    alpha_test INTEGER,                 -- 0-255 scissor cutoff, NULL = no alpha test
    two_sided INTEGER DEFAULT 0,
    material_name TEXT                  -- full GLB material name of the replacement (MaterialNameCodec)
);
CREATE INDEX IF NOT EXISTS idx_mswp_entries ON material_swap_entries(swap_form_key);

-- Base-record material swaps: FO4's Model MODS subrecord is a 4-byte MSWP
-- formid (NOT Skyrim's alternate-texture list) and MODC is a float32 color
-- remapping index. Mutagen doesn't expose either, so a raw ESM scan fills
-- this. A placed ref's XMSP (placed_objects.material_swap) takes precedence.
CREATE TABLE IF NOT EXISTS base_material_swaps (
    form_key TEXT PRIMARY KEY,
    material_swap TEXT,                 -- MSWP form_key (NULL when only MODC present)
    color_remap_index REAL              -- MODC: not consumed by the engine yet
);
CREATE TABLE IF NOT EXISTS navigation_mesh_info_maps         (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS navigation_mesh_obstacle_managers (form_key TEXT PRIMARY KEY, editor_id TEXT);
-- One row per NAVM.
CREATE TABLE IF NOT EXISTS navigation_meshes (
    form_key TEXT PRIMARY KEY,
    editor_id TEXT,
    form_id INTEGER,                    -- 24-bit FormID the link blobs use for targets
    cell TEXT,                          -- owning cell form_key
    worldspace TEXT,                    -- NULL for interior meshes
    grid_x INTEGER,                     -- exterior only
    grid_y INTEGER,
    record_flags INTEGER,               -- raw NAVM major record flags
    format_version INTEGER,             -- NVNM version
    vertex_count INTEGER,
    triangle_count INTEGER,
    edge_link_count INTEGER,
    door_link_count INTEGER,
    vertices BLOB,
    triangles BLOB,
    edge_links BLOB,
    door_links BLOB
);
CREATE TABLE IF NOT EXISTS object_visibility_managers (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS pack_ins                  (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS reference_groups          (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS reverb_parameterses       (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS scene_collections         (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS scenes                    (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS shader_particle_geometries (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS sound_keyword_mappings    (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS sound_markers             (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS transforms                (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS zooms                     (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS damage_types              (form_key TEXT PRIMARY KEY, editor_id TEXT);
CREATE TABLE IF NOT EXISTS story_manager_nodes       (form_key TEXT PRIMARY KEY, editor_id TEXT);
