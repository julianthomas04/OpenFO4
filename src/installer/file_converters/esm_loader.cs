using System.Collections;
using System.Collections.Concurrent;
using System.Data;
using System.Diagnostics;
using System.Dynamic;
using System.Globalization;
using System.Linq;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Text;
using System.Text.Json;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Mutagen.Bethesda.Plugins.Records;

namespace AssetConverter
{
    public partial class ESMLoader
    {
        private string _dbPath;
        private string? _materialsRoot; // extracted BA2 staging root containing Materials/; null disables BGSM-backed TXST resolution
        private string? _texturesOut;   // deployed textures/ root; null disables late palette-variant bakes for MSWP ColorRemappingIndex
        private int _progressPhase = -1;
        private CancellationToken _ct;

        /// <summary>The installed language's <c>Fallout4_&lt;lang&gt;.*STRINGS</c> tables; required when the plugin is localized.</summary>
        public StringTableSet? Strings { get; init; }

        /// <summary>Admits the import against the install's memory.</summary>
        public required MemoryBudget Memory { get; init; }

        public ESMLoader(string dbPath, string? materialsRoot = null, string? texturesOut = null)
        {
            _dbPath = dbPath;
            _materialsRoot = materialsRoot;
            _texturesOut = texturesOut;
            Console.WriteLine($"Database path: {_dbPath}");
        }

        public ESMLoader()
        {
            string installRoot = InstallerPaths.DevInstallFolder
                ?? throw new InvalidOperationException("A packaged installer has no development install folder; give the database path.");
            string dataDirectory = Path.Combine(installRoot, InstallLayout.AssetsFolder, InstallLayout.DataFolder);
            _dbPath = Path.Combine(dataDirectory, "data.db");
            Console.WriteLine($"Database path: {_dbPath}");
        }

        public void LoadESM(string path, int progressPhase = -1, CancellationToken ct = default)
        {
            LoadESMAsync(path, progressPhase, ct).GetAwaiter().GetResult();
        }

        public async Task LoadESMAsync(string path, int progressPhase = -1, CancellationToken ct = default)
        {
            _progressPhase = progressPhase;
            _ct = ct;
            Console.WriteLine($"Loading ESM from: {path}");
            using var memory = await Memory.ReserveAsync(MemoryEstimates.EsmImportBytes, ct).ConfigureAwait(false);
            await EsmTiming.RunReportedAsync(() => ImportAsync(path)).ConfigureAwait(false);
        }

        private async Task ImportAsync(string path)
        {
            using var dbManager = new DatabaseManager(_dbPath);
            using (EsmTiming.MeasurePhase("init.schema"))
                await dbManager.InitializeAsync();

            long overlayStart = EsmTiming.Now();
            using var file = OpenPlugin(path);
            EsmTiming.Add("init.overlay", overlayStart);

            Console.WriteLine("Importing records...");

            // The two raw-ESM scans (XCRI precombined refs, material-swap
            // links) need only the file bytes — start them now so they run
            // under Wave 1 instead of serializing after it.
            var rawScansTask = Task.Run(() => ScanRawEsm(path));

            await dbManager.Connection.ExecuteAsync("PRAGMA foreign_keys = OFF");
            Console.WriteLine("Wave 1: processing all independent record groups in parallel...");

            // Limit concurrent DB connections. SQLite WAL serialises commits via a
            // single writer mutex; beyond ~ProcessorCount active writers the mutex
            // becomes the only bottleneck and extra connections just add overhead /
            // contention. Cap at ProcessorCount (min 2, max 16).
            int maxWriters = Math.Clamp(Environment.ProcessorCount, 2, 16);
            using var writeSem = new SemaphoreSlim(maxWriters, maxWriters);

            // Local helper: acquire semaphore, open a fresh connection, run the group.
            async Task RunGroup(Func<SqliteConnection, Task> body)
            {
                await writeSem.WaitAsync(_ct);
                try
                {
                    using var c = DatabaseManager.OpenWorkerConnection(_dbPath);
                    await body(c);
                }
                finally { writeSem.Release(); }
            }

            // Wave 1 — all record groups that have no FK dependencies on each other.
            // Each task opens its own worker connection (private-cache WAL); commits
            // serialize at the SQLite WAL level, keeping I/O as the bottleneck.
            var importContext = new ImportContext(file, rawScansTask);
            using (EsmTiming.MeasurePhase("wave1.wall"))
            {
                await Task.WhenAll(RecordJobs.Select(job =>
                    Task.Run(() => RunGroup(async c =>
                    {
                        using var jobTiming = EsmTiming.Measure("wave1.job." + job.Label);
                        await job.Run(this, importContext, c, null);
                    }))));
            }
            Console.WriteLine("Wave 1 complete.");
            using (EsmTiming.MeasurePhase("omod.attach-point-names"))
                await WeaponModJson.ResolveAttachPointNamesAsync(dbManager.Connection, null);

            // Join the raw-ESM scans started before Wave 1 (XCRI precombined
            // refs + material-swap links; Mutagen exposes neither for FO4).
            RawScans rawScans;
            using (EsmTiming.MeasurePhase("wave1.rawscans-wait"))
                rawScans = await rawScansTask;
            Console.WriteLine($"  Raw scans: {rawScans.Precombined.Count} precombined REFR form keys, " +
                $"{rawScans.RefrSwaps.Count} placed refs with XMSP, {rawScans.BaseSwaps.Count} bases with MODS/MODC.");
            using (EsmTiming.MeasurePhase("base-material-swaps.insert"))
                InsertBaseMaterialSwaps(dbManager.Connection, null, rawScans.BaseSwaps);

            _ct.ThrowIfCancellationRequested();
            using (EsmTiming.MeasurePhase("worldspaces"))
                await WorldspaceJob.Run(this, importContext, dbManager.Connection, null);
            using (EsmTiming.MeasurePhase("cells.total"))
                await CellsJob.Run(this, importContext, dbManager.Connection, null);

            await dbManager.Connection.ExecuteAsync("PRAGMA foreign_keys = ON");
            InstallProgress.SetDetail(_progressPhase, "Finalizing database");
            await dbManager.FinalizeBulkLoadAsync();

            ReadFailureTally.Shared.WriteSummaryAndReset(Console.Out);
            Console.WriteLine("Import complete!");
        }

        // Computes placed_objects.initially_hidden: the ref's initial enable
        // state after resolving XESP chains. A ref with an enable parent
        // mirrors the parent's enable state, inverted when the
        // opposite-of-parent flag is set; its own InitiallyDisabled flag is
        // ignored (the CK keeps the two in sync — the parent is
        // authoritative). Chains root at refs without parents, whose state is
        // their InitiallyDisabled flag. PlayerRef (000014) and any parent not
        // in the table count as enabled; cycles resolve as visible.
        internal static async Task ResolveInitiallyHiddenAsync(SqliteConnection conn, SqliteTransaction? outerTransaction)
        {
            Console.WriteLine("Resolving initial enable-state visibility (XESP chains)...");
            long queryStart = EsmTiming.Now();
            var rows = (await conn.QueryAsync<(string FormKey, int Disabled, string? Parent, int Opposite)>(
                "SELECT form_key, is_initially_disabled, enable_parent, enable_parent_opposite FROM placed_objects",
                transaction: outerTransaction))
                .ToList();
            EsmTiming.Add("hidden.query", queryStart, rows.Count);
            long resolveStart = EsmTiming.Now();

            var byKey = new Dictionary<string, (string FormKey, int Disabled, string? Parent, int Opposite)>(rows.Count);
            foreach (var r in rows) byKey[r.FormKey] = r;

            var visible = new Dictionary<string, bool>(rows.Count);
            int cycles = 0;
            var path = new List<string>();

            foreach (var start in rows)
            {
                if (visible.ContainsKey(start.FormKey)) continue;

                path.Clear();
                string cur = start.FormKey;
                bool vis;
                while (true)
                {
                    if (visible.TryGetValue(cur, out var v)) { vis = v; break; }
                    if (!byKey.TryGetValue(cur, out var row)) { vis = true; break; }
                    if (row.Parent == null) { vis = row.Disabled == 0; visible[cur] = vis; break; }
                    if (path.Contains(cur)) { vis = true; cycles++; break; }
                    path.Add(cur);
                    cur = row.Parent;
                }
                // Unwind: each path entry's visibility derives from the node
                // below it (its parent), applying its own opposite flag.
                for (int i = path.Count - 1; i >= 0; i--)
                {
                    var row = byKey[path[i]];
                    vis ^= row.Opposite != 0;
                    visible[path[i]] = vis;
                }
            }

            var hidden = rows.Where(r => !visible[r.FormKey]).Select(r => r.FormKey).ToList();
            if (cycles > 0)
                Console.WriteLine($"  WARNING: {cycles} XESP cycle(s) resolved as visible.");
            EsmTiming.Add("hidden.resolve", resolveStart, rows.Count);

            long updateStart = EsmTiming.Now();
            MarkHidden(conn, outerTransaction, hidden);
            EsmTiming.Add("hidden.update", updateStart, hidden.Count);
            Console.WriteLine($"  {hidden.Count} of {rows.Count} placed refs initially hidden " +
                $"({rows.Count(r => r.Parent != null)} carry XESP enable parents).");
        }

        // One prepared statement rebound per key: a batched IN (@p0..@p499) statement costs quadratic binding time
        // (SQLite resolves named parameters by a linear scan), the same trap BatchInsert avoids.
        private static void MarkHidden(SqliteConnection conn, SqliteTransaction? outerTransaction, IReadOnlyList<string> formKeys)
        {
            if (formKeys.Count == 0) return;

            var ownTransaction = outerTransaction == null ? conn.BeginTransaction() : null;
            try
            {
                using var cmd = conn.CreateCommand();
                cmd.Transaction = outerTransaction ?? ownTransaction;
                cmd.CommandText = "UPDATE placed_objects SET initially_hidden = 1 WHERE form_key = $formKey";
                var formKey = cmd.CreateParameter();
                formKey.ParameterName = "$formKey";
                cmd.Parameters.Add(formKey);
                cmd.Prepare();
                foreach (var key in formKeys)
                {
                    formKey.Value = key;
                    cmd.ExecuteNonQuery();
                }
                ownTransaction?.Commit();
            }
            catch
            {
                if (ownTransaction != null) TransactionRollback.AfterFailure(ownTransaction);
                throw;
            }
        }

        // Used by parallel Wave 1 (each group gets its own worker connection) and by partial reloads.
        private async Task ProcessGroupAsync<T>(IEnumerable<T> records, string recordType, SqliteConnection conn, IRecordProcessor<T> processor, SqliteTransaction? transaction = null)
            where T : IMajorRecordGetter
        {
            List<T> list;
            try { list = records.ToList(); }
            catch (Exception ex) when (
                ex.Message.Contains("Unknown major record type") ||
                (ex.InnerException?.Message.Contains("Unknown major record type") ?? false))
            {
                Console.WriteLine($"Skipping {typeof(T).Name} — not a Fallout 4 record type.");
                return;
            }
            if (list.Count == 0) return;
            InstallProgress.AddTotal(_progressPhase, list.Count);
            InstallProgress.SetDetail(_progressPhase, recordType);
            Console.WriteLine($"Processing {typeof(T).Name} ({list.Count} records)...");
            await processor.ProcessRecordsAsync(list, recordType, conn, transaction, progressPhase: _progressPhase, ct: _ct);
        }

        private async Task ProcessWorldspaceRecordsAsync(IEnumerable<IWorldspaceGetter> worldspaces, SqliteConnection conn, SqliteTransaction? transaction)
        {
            var list = worldspaces.ToList();
            if (list.Count == 0) return;
            InstallProgress.AddTotal(_progressPhase, list.Count);
            InstallProgress.SetDetail(_progressPhase, "WRLD");
            Console.WriteLine($"Processing Worldspaces ({list.Count} records)...");

            var processor = new WorldspaceProcessor();
            await processor.ProcessRecordsAsync(list, "WRLD", conn, transaction, progressPhase: _progressPhase, ct: _ct);
        }

        // Cells (and everything placed in them) of every worldspace plus the
        // top-level interior cells, then the XESP enable-chain resolution that
        // needs every placed_objects row.
        private async Task ProcessCellsAsync(IFallout4ModGetter file, SqliteConnection conn, SqliteTransaction? transaction,
            HashSet<string> precombinedSet, Dictionary<string, string> refrSwaps, Dictionary<string, string> waterCurrents)
        {
            using (EsmTiming.MeasurePhase("cells.worldspace-cells"))
                await ProcessWorldspaceCellsAsync(file.Worldspaces, conn, transaction, precombinedSet, refrSwaps, waterCurrents);
            using (EsmTiming.MeasurePhase("cells.top-level-cells"))
                await ProcessTopLevelCellsAsync(file, conn, transaction, precombinedSet, refrSwaps, waterCurrents);
            using (EsmTiming.MeasurePhase("hidden.total"))
                await ResolveInitiallyHiddenAsync(conn, transaction);
        }

        private async Task ProcessTopLevelCellsAsync(IFallout4ModGetter file, SqliteConnection conn, SqliteTransaction? transaction,
            HashSet<string> precombinedSet, Dictionary<string, string> refrSwaps, Dictionary<string, string> waterCurrents)
        {
            _ct.ThrowIfCancellationRequested();
            var topCells = new List<ICellGetter>();
            if (file.Cells != null) ExtractCellsRecursive(file.Cells, topCells);
            if (topCells.Count == 0) return;

            InstallProgress.AddTotal(_progressPhase, topCells.Count);
            InstallProgress.SetDetail(_progressPhase, "CELL (interiors)");
            Console.WriteLine($"Processing Top-Level Cells ({topCells.Count})...");
            var cellProcessor = new CellProcessor(null, precombinedSet, refrSwaps, waterCurrents);
            await cellProcessor.ProcessRecordsAsync(topCells, "CELL", conn, transaction, progressPhase: _progressPhase, ct: _ct);
        }

        private async Task ProcessWorldspaceCellsAsync(IEnumerable<IWorldspaceGetter> worldspaces, SqliteConnection conn, SqliteTransaction? transaction,
            HashSet<string> precombinedSet, Dictionary<string, string> refrSwaps, Dictionary<string, string> waterCurrents)
        {
            if (!worldspaces.Any()) return;

            var topCellKeys = new List<string>();
            foreach (var wrld in worldspaces)
            {
                _ct.ThrowIfCancellationRequested();
                var cells = new List<ICellGetter>();
                if (wrld.TopCell != null)
                {
                    cells.Add(wrld.TopCell);
                    topCellKeys.Add(wrld.TopCell.FormKey.ToString());
                }
                if (wrld.SubCells != null) ExtractCellsRecursive(wrld.SubCells, cells);
                if (cells.Count == 0) continue;

                InstallProgress.AddTotal(_progressPhase, cells.Count);
                InstallProgress.SetDetail(_progressPhase, $"CELL ({wrld.EditorID ?? wrld.FormKey.ToString()})");
                Console.WriteLine($"  Processing {cells.Count} cells in worldspace {wrld.FormKey}...");
                var cellProcessor = new CellProcessor(wrld.FormKey.ToString(), precombinedSet, refrSwaps, waterCurrents);
                await cellProcessor.ProcessRecordsAsync(cells, "CELL", conn, transaction, progressPhase: _progressPhase, ct: _ct);
            }

            // Mutagen defaults TopCell's grid coords to (0,0) since it has no
            // XCLC subrecord, which collides with the real (0,0) sub-cell. Flag
            // TopCells explicitly so the streamer can exclude them from the
            // radius query and load their persistent refs separately.
            if (topCellKeys.Count > 0)
            {
                using var cmd = conn.CreateCommand();
                cmd.Transaction = transaction;
                cmd.CommandText =
                    "UPDATE cells SET is_worldspace_persistent_cell = 1 WHERE form_key = $fk";
                var p = cmd.CreateParameter();
                p.ParameterName = "$fk";
                cmd.Parameters.Add(p);
                foreach (var fk in topCellKeys)
                {
                    p.Value = fk;
                    await cmd.ExecuteNonQueryAsync();
                }
                Console.WriteLine($"  Flagged {topCellKeys.Count} worldspace persistent cells (TopCell).");
            }
        }

        private static void InsertBaseMaterialSwaps(SqliteConnection conn, SqliteTransaction? transaction,
            List<(string FormKey, string? SwapFormKey, float? ColorRemapIndex)> baseSwaps)
        {
            if (baseSwaps.Count == 0) return;
            var rows = baseSwaps
                .Select(b => new object?[] { b.FormKey, b.SwapFormKey, b.ColorRemapIndex })
                .ToArray();
            BatchInsert.Execute(conn, transaction, "base_material_swaps",
                EsmColumns.BaseMaterialSwaps,
                rows);
        }

        private static void ExtractCellsRecursive(object node, List<ICellGetter> results, int depth = 0)
        {
            if (node == null || depth > 5) return; // Limit depth to prevent infinite loops/stack overflow

            // 1. Is it a Cell? (Base Case)
            if (node is ICellGetter cell)
            {
                results.Add(cell);
                return;
            }

            // 2. Is it a collection?
            IEnumerable? collection = node as IEnumerable;

            // 3. If not directly enumerable (or is a Block/Group), check for known properties that return a collection
            // Mutagen 0.50+ often uses "Items" for Groups/Blocks
            if (collection == null)
            {
                try
                {
                    var type = node.GetType();
                    var prop = type.GetProperty("Items")
                            ?? type.GetProperty("SubBlocks")
                            ?? type.GetProperty("Cells")
                            ?? type.GetProperty("SubBlock"); // Legacy

                    if (prop != null)
                    {
                        collection = prop.GetValue(node) as IEnumerable;
                    }
                }
                catch (Exception ex) when (ReadFailureTally.IsReadFailure(ex))
                {
                    ReadFailureTally.Shared.Record("ESM tree walk: collection property lookup", ex, node.GetType().Name);
                }
            }

            // 4. Iterate if we found a collection
            if (collection != null)
            {
                foreach (var item in collection)
                {
                    ExtractCellsRecursive(item, results, depth + 1);
                }
            }
        }
    }

    public class DatabaseManager : IDisposable
    {
        private readonly SqliteConnection _connection;

        public DatabaseManager(string dbPath)
        {
            var connectionString = new SqliteConnectionStringBuilder
            {
                DataSource = dbPath,
                Mode = SqliteOpenMode.ReadWriteCreate,
                Cache = SqliteCacheMode.Shared
            }.ToString();

            _connection = new SqliteConnection(connectionString);
            _connection.Open();
        }

        public SqliteConnection Connection => _connection;

        public async Task InitializeAsync(bool bulkLoad = true)
        {
            await _connection.ExecuteAsync("PRAGMA busy_timeout = 300000");
            await _connection.ExecuteAsync("PRAGMA foreign_keys = ON");
            await _connection.ExecuteAsync("PRAGMA journal_mode = WAL");
            await _connection.ExecuteAsync("PRAGMA synchronous = NORMAL");
            await _connection.ExecuteAsync("PRAGMA cache_size = -64000");
            await _connection.ExecuteAsync("PRAGMA temp_store = MEMORY");

            if (bulkLoad && BulkLoadPragmasEnabled)
            {
                // From-scratch bulk build: the DB is deleted before every load,
                // so durability mid-build buys nothing. synchronous=OFF drops
                // the fsyncs; wal_autocheckpoint=0 stops mid-load checkpoint
                // stalls (FinalizeBulkLoadAsync runs one TRUNCATE checkpoint
                // at the end). Opt out with OPENFO4_SQLITE_FAST=0.
                await _connection.ExecuteAsync("PRAGMA synchronous = OFF");
                await _connection.ExecuteAsync("PRAGMA wal_autocheckpoint = 0");
            }

            string schemaPath = InstallerPaths.Require("src/installer/file_converters/esm_schema.sql");

            string schema = await File.ReadAllTextAsync(schemaPath);
            if (await GrassFlagsAreTextAsync())
                await RebuildGrassesWithIntegerFlagsAsync(schema);
            else
                await _connection.ExecuteAsync(schema);

            // SQLite's CREATE TABLE IF NOT EXISTS skips adding columns to a
            // pre-existing table, so any new optional column must be explicitly
            // added here.
            foreach (var (table, column, type) in ColumnMigrations)
                await AddColumnIfMissingAsync(table, column, type);

            // Indexes over migrated columns cannot live in the schema script: it runs before the columns exist on an old table.
            foreach (string index in IndexMigrations)
                await _connection.ExecuteAsync(index);
        }

        private static readonly string[] IndexMigrations =
        {
            "CREATE UNIQUE INDEX IF NOT EXISTS addon_nodes_node_index ON addon_nodes(node_index)",
            "CREATE INDEX IF NOT EXISTS idx_navmesh_ws_grid ON navigation_meshes(worldspace, grid_x, grid_y)",
            "CREATE INDEX IF NOT EXISTS idx_navmesh_cell ON navigation_meshes(cell)",
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_navmesh_form_id ON navigation_meshes(form_id)",
            "CREATE INDEX IF NOT EXISTS idx_collision_layers_index ON collision_layers(layer_index)",
        };

        // grasses.flags was TEXT (enum rendered as a string); the column is now the raw INTEGER bitfield. SQLite cannot
        // change a column type in place. Initialization also runs for partial reloads of other tables, so the rows are
        // carried over instead of dropped: rename, create the current table, copy the shared columns (flags cast to
        // INTEGER), drop the renamed table. The GRAS job refills every column on its next reload.
        private const string LegacyGrassTable = "grasses_text_flags";

        private async Task<bool> GrassFlagsAreTextAsync()
        {
            string? flagsType = await _connection.ExecuteScalarAsync<string?>(
                "SELECT type FROM pragma_table_info('grasses') WHERE name = 'flags'");
            return string.Equals(flagsType, "TEXT", StringComparison.OrdinalIgnoreCase);
        }

        private async Task RebuildGrassesWithIntegerFlagsAsync(string schema)
        {
            // Rows are carried over as they are; the pragma cannot change inside the transaction.
            await _connection.ExecuteAsync("PRAGMA foreign_keys = OFF");
            try
            {
                await CopyGrassesIntoCurrentSchemaAsync(schema);
            }
            finally
            {
                await _connection.ExecuteAsync("PRAGMA foreign_keys = ON");
            }
        }

        // The schema's leading PRAGMA statements already ran for this connection and end the batch early inside a transaction.
        private static string SchemaWithoutPragmas(string schema) =>
            System.Text.RegularExpressions.Regex.Replace(schema, @"^PRAGMA .*$", "", System.Text.RegularExpressions.RegexOptions.Multiline);

        private async Task CopyGrassesIntoCurrentSchemaAsync(string schema)
        {
            using var transaction = _connection.BeginTransaction();
            await _connection.ExecuteAsync($"ALTER TABLE grasses RENAME TO {LegacyGrassTable}", transaction: transaction);
            await _connection.ExecuteAsync(SchemaWithoutPragmas(schema), transaction: transaction);

            const string columnsOf = "SELECT name FROM pragma_table_info(@Table)";
            var legacyColumns = await _connection.QueryAsync<string>(columnsOf, new { Table = LegacyGrassTable }, transaction);
            var currentColumns = await _connection.QueryAsync<string>(columnsOf, new { Table = "grasses" }, transaction);
            string copied = string.Join(", ", currentColumns.Intersect(legacyColumns).Where(column => column != "flags"));
            string columnList = copied.Length == 0 ? "flags" : $"flags, {copied}";
            string selectList = copied.Length == 0 ? "CAST(flags AS INTEGER)" : $"CAST(flags AS INTEGER), {copied}";
            await _connection.ExecuteAsync(
                $"INSERT INTO grasses ({columnList}) SELECT {selectList} FROM {LegacyGrassTable}", transaction: transaction);
            await _connection.ExecuteAsync($"DROP TABLE {LegacyGrassTable}", transaction: transaction);
            transaction.Commit();
        }

        private static readonly (string Table, string Column, string Type)[] ColumnMigrations =
        {
            ("placed_objects", "is_initially_disabled", "INTEGER DEFAULT 0"),
            ("placed_objects", "enable_parent_opposite", "INTEGER DEFAULT 0"),
            ("placed_objects", "is_in_precombined", "INTEGER DEFAULT 0"),
            ("placed_objects", "radius_override", "REAL"),
            ("placed_objects", "light_fov_pm", "REAL"),
            ("placed_objects", "light_fade_pm", "REAL"),
            ("placed_objects", "light_end_distance_cap", "REAL"),
            ("placed_objects", "light_shadow_depth_bias", "REAL"),
            ("placed_objects", "light_near_clip", "REAL"),
            ("placed_objects", "light_volumetric_intensity", "REAL"),
            ("placed_objects", "record_flags", "INTEGER"),
            ("placed_objects", "ragdoll_pose", "BLOB"),
            ("cells", "is_worldspace_persistent_cell", "INTEGER DEFAULT 0"),
            ("cells", "lighting_template", "TEXT"),
            ("lights", "constant", "REAL"),
            ("lights", "scalar", "REAL"),
            ("lights", "exponent", "REAL"),
            ("lights", "fov", "REAL"),
            ("lights", "near_clip", "REAL"),
            ("lights", "flicker_period", "REAL"),
            ("lights", "flicker_intensity_amplitude", "REAL"),
            ("lights", "flicker_movement_amplitude", "REAL"),
            ("lights", "gobo", "TEXT"),
            ("lights", "lens", "TEXT"),
            ("lights", "god_rays", "TEXT"),
            ("lights", "god_rays_near_clip", "REAL"),
            ("material_swap_entries", "material_name", "TEXT"),
            ("weather", "directional_ambient", "TEXT"),
            ("worldspaces", "climate", "TEXT"),
            ("worldspaces", "water_type", "TEXT"),
            ("worldspaces", "lod_water_type", "TEXT"),
            ("worldspaces", "lod_water_height", "REAL"),
            ("worldspaces", "default_land_height", "REAL"),
            ("worldspaces", "default_water_height", "REAL"),
            ("worldspaces", "water_environment_map", "TEXT"),
            ("cells", "has_water", "INTEGER DEFAULT 0"),
            ("cells", "no_lod_water", "INTEGER DEFAULT 0"),
            ("cells", "water_height", "REAL"),
            ("cells", "water_type", "TEXT"),
            ("cells", "water_environment_map", "TEXT"),
            ("cells", "water_currents", "TEXT"),
            ("activators", "water_type", "TEXT"),
            ("placed_objects", "water_currents", "TEXT"),
            ("worldspaces", "parent_flags", "INTEGER"),
            ("worldspaces", "map_image", "TEXT"),
            ("perks", "level", "INTEGER"),
            ("perks", "num_ranks", "INTEGER"),
            ("perks", "playable", "INTEGER"),
            ("perks", "hidden", "INTEGER"),
            ("perks", "swf_path", "TEXT"),
            ("perks", "first_rank", "TEXT"),
            ("material_types", "material_name", "TEXT"),
            ("material_types", "material_hash", "INTEGER"),
            ("material_types", "buoyancy", "REAL"),
            ("material_types", "flags", "INTEGER"),
            ("material_types", "parent", "TEXT"),
            ("grasses", "density", "INTEGER"),
            ("grasses", "min_slope", "INTEGER"),
            ("grasses", "max_slope", "INTEGER"),
            ("grasses", "units_from_water", "INTEGER"),
            ("grasses", "units_from_water_type", "INTEGER"),
            ("grasses", "position_range", "REAL"),
            ("grasses", "height_range", "REAL"),
            ("grasses", "color_range", "REAL"),
            ("grasses", "wave_period", "REAL"),
            ("addon_nodes", "model_path", "TEXT"),
            ("addon_nodes", "node_index", "INTEGER"),
            ("addon_nodes", "light", "TEXT"),
            ("addon_nodes", "sound", "TEXT"),
            ("addon_nodes", "master_particle_system_cap", "INTEGER"),
            ("addon_nodes", "flags", "INTEGER"),
            ("navigation_meshes", "form_id", "INTEGER"),
            ("navigation_meshes", "cell", "TEXT"),
            ("navigation_meshes", "worldspace", "TEXT"),
            ("navigation_meshes", "grid_x", "INTEGER"),
            ("navigation_meshes", "grid_y", "INTEGER"),
            ("navigation_meshes", "record_flags", "INTEGER"),
            ("navigation_meshes", "format_version", "INTEGER"),
            ("navigation_meshes", "vertex_count", "INTEGER"),
            ("navigation_meshes", "triangle_count", "INTEGER"),
            ("navigation_meshes", "edge_link_count", "INTEGER"),
            ("navigation_meshes", "door_link_count", "INTEGER"),
            ("navigation_meshes", "vertices", "BLOB"),
            ("navigation_meshes", "triangles", "BLOB"),
            ("navigation_meshes", "edge_links", "BLOB"),
            ("navigation_meshes", "door_links", "BLOB"),
            ("weapons", "default_mods", "TEXT"),
            ("weapons", "object_template", "TEXT"),
            ("object_modifications", "model_path", "TEXT"),
            ("object_modifications", "attach_point_name", "TEXT"),
            ("object_modifications", "attach_parent_slots", "TEXT"),
            ("music_types", "flags", "INTEGER"),
            ("music_types", "priority", "INTEGER"),
            ("music_types", "ducking_centidb", "INTEGER"),
            ("music_types", "fade_seconds", "REAL"),
            ("music_tracks", "file_key", "TEXT"),
            ("music_tracks", "anam_path", "TEXT"),
            ("region_music", "record_order", "INTEGER NOT NULL DEFAULT 0"),
            ("music_track_conditions", "flags", "INTEGER NOT NULL DEFAULT 0"),
            ("sound_descriptor_conditions", "flags", "INTEGER NOT NULL DEFAULT 0"),
            ("sound_descriptors", "descriptor_type", "INTEGER"),
            ("sound_descriptors", "base_descriptor", "TEXT"),
            ("sound_descriptors", "freq_shift_pct", "INTEGER"),
            ("sound_descriptors", "freq_variance_pct", "INTEGER"),
            ("sound_descriptors", "priority", "INTEGER"),
            ("sound_descriptors", "db_variance", "INTEGER"),
            ("sound_descriptors", "static_attenuation_centidb", "INTEGER"),
            ("sound_descriptors", "looping_flags", "INTEGER"),
            ("sound_output_models", "output_type", "INTEGER"),
            ("sound_output_models", "static_attenuation_db", "REAL"),
            ("footsteps", "tag", "TEXT"),
            ("footsteps", "impact_data_set", "TEXT"),
            ("impacts", "player_sound", "TEXT"),
            ("impacts", "npc_sound", "TEXT"),
            ("collision_layers", "layer_index", "INTEGER"),
            ("collision_layers", "flags", "INTEGER"),
            ("collision_layers", "collides_with", "TEXT"),
            ("doors", "bounds", "TEXT"),
        };

        private async Task AddColumnIfMissingAsync(string table, string column, string type)
        {
            long present = await _connection.ExecuteScalarAsync<long>(
                "SELECT COUNT(*) FROM pragma_table_info(@Table) WHERE name = @Column",
                new { Table = table, Column = column });
            if (present == 0)
                await _connection.ExecuteAsync($"ALTER TABLE {table} ADD COLUMN {column} {type}");
        }

        public void Dispose()
        {
            _connection?.Dispose();
            GC.SuppressFinalize(this);
        }

        internal static bool BulkLoadPragmasEnabled =>
            Environment.GetEnvironmentVariable("OPENFO4_SQLITE_FAST") != "0";

        /// <summary>Checkpoint, analyze, and compact the DB after bulk load.</summary>
        public async Task FinalizeBulkLoadAsync()
        {
            if (!BulkLoadPragmasEnabled) return;
            using (EsmTiming.MeasurePhase("finalize.checkpoint"))
                await _connection.ExecuteAsync("PRAGMA wal_checkpoint(TRUNCATE)");
            using (EsmTiming.MeasurePhase("finalize.optimize"))
                await _connection.ExecuteAsync("PRAGMA optimize");
            using (EsmTiming.MeasurePhase("finalize.vacuum"))
                await _connection.ExecuteAsync("VACUUM");
            await _connection.ExecuteAsync("PRAGMA synchronous = NORMAL");
        }

        /// <summary>Checkpoint and re-analyze after an in-place partial reload (no VACUUM: the freed pages were reused).</summary>
        public async Task FinalizePartialReloadAsync()
        {
            await _connection.ExecuteAsync("PRAGMA wal_checkpoint(TRUNCATE)");
            await _connection.ExecuteAsync("PRAGMA optimize");
        }

        // Open a secondary connection for a parallel worker group.
        // Uses private cache (Default) instead of Shared to avoid SQLITE_LOCKED
        // under shared-cache + WAL when multiple connections commit concurrently.
        public static SqliteConnection OpenWorkerConnection(string dbPath)
        {
            var cs = new SqliteConnectionStringBuilder
            {
                DataSource = dbPath,
                Mode = SqliteOpenMode.ReadWrite,
                Cache = SqliteCacheMode.Default
            }.ToString();
            var conn = new SqliteConnection(cs);
            conn.Open();
            // Execute each PRAGMA separately — multi-statement commands are unreliable
            // across SQLite driver versions (some only run the first statement).
            // busy_timeout FIRST so subsequent PRAGMAs retry rather than failing.
            // journal_mode is omitted: main connection already put the DB into WAL;
            // re-running it here races other workers and can return SQLITE_BUSY.
            foreach (var pragma in new[]
            {
                "PRAGMA busy_timeout = 300000",  // 5 min — covers long-held write locks across 130 groups
                BulkLoadPragmasEnabled ? "PRAGMA synchronous = OFF" : "PRAGMA synchronous = NORMAL",
                "PRAGMA cache_size   = -64000",
                "PRAGMA temp_store   = MEMORY",
                "PRAGMA foreign_keys = OFF",
            })
            {
                using var cmd = conn.CreateCommand();
                cmd.CommandText = pragma;
                cmd.ExecuteNonQuery();
            }
            return conn;
        }
    }

    internal static class EsmColumns
    {
        public static readonly string[] Records = { "form_key", "record_type", "editor_id", "source_plugin" };
        public static readonly string[] BaseMaterialSwaps = { "form_key", "material_swap", "color_remap_index" };
        public static readonly string[] MaterialSwapEntries =
        {
            "swap_form_key", "original_material", "replacement_material", "diffuse",
            "normal", "is_blend", "alpha_mult", "alpha_test", "two_sided", "material_name",
        };
        public static readonly string[] Cells =
        {
            "form_key", "editor_id", "worldspace", "grid_x", "grid_y",
            "is_interior", "flags", "lighting", "lighting_template", "encounter_zone",
            "has_water", "no_lod_water", "water_height", "water_type", "water_environment_map", "water_currents",
        };
        public static readonly string[] PlacedObjects =
        {
            "form_key", "cell_form_key", "base_object", "object_type",
            "is_persistent", "position_x", "position_y", "position_z",
            "rotation_x", "rotation_y", "rotation_z",
            "scale", "is_initially_disabled", "enable_parent", "enable_parent_opposite",
            "has_primitive", "material_swap",
            "radius_override", "light_fov_pm", "light_fade_pm",
            "light_end_distance_cap", "light_shadow_depth_bias", "light_near_clip",
            "light_volumetric_intensity", "record_flags", "ragdoll_pose", "water_currents",
        };
        public static readonly string[] PlacedTeleports =
        {
            "source_ref", "destination_ref",
            "dest_position_x", "dest_position_y", "dest_position_z",
            "dest_rotation_x", "dest_rotation_y", "dest_rotation_z",
        };
    }

    // INSERT OR REPLACE of many rows through one prepared statement that is rebound per row, in insertion order. A
    // multi-row VALUES statement is not an option: SQLite resolves each named parameter by a linear scan of the
    // statement's parameters, so binding a statement of n parameters costs O(n^2) (500 rows x 27 columns took 700 us
    // per row against 6 us here).
    internal static class BatchInsert
    {
        internal const int ProgressInterval = 1000;

        internal static void Execute(
            SqliteConnection conn,
            SqliteTransaction? tx,
            string tableName,
            string[] columns,
            IReadOnlyList<object?[]> rows,
            Action<int>? rowsInserted = null)
        {
            if (rows.Count == 0) return;
            RequireRowWidth(tableName, columns, rows);

            var ownTransaction = tx == null ? conn.BeginTransaction() : null;
            try
            {
                InsertRows(conn, tx ?? ownTransaction!, tableName, columns, rows, rowsInserted);
                ownTransaction?.Commit();
            }
            catch
            {
                if (ownTransaction != null) TransactionRollback.AfterFailure(ownTransaction);
                throw;
            }
        }

        private static void RequireRowWidth(string tableName, string[] columns, IReadOnlyList<object?[]> rows)
        {
            for (int r = 0; r < rows.Count; r++)
                if (rows[r].Length != columns.Length)
                    throw new ArgumentException($"Row {r} for {tableName} has {rows[r].Length} values, expected {columns.Length}.", nameof(rows));
        }

        private static void InsertRows(
            SqliteConnection conn,
            SqliteTransaction tx,
            string tableName,
            string[] columns,
            IReadOnlyList<object?[]> rows,
            Action<int>? rowsInserted)
        {
            using var cmd = conn.CreateCommand();
            cmd.Transaction = tx;
            cmd.CommandText = $"INSERT OR REPLACE INTO {tableName} ({string.Join(", ", columns)}) VALUES ({string.Join(", ", columns.Select((_, c) => "$p" + c))})";
            var parameters = columns.Select((_, c) =>
            {
                var parameter = cmd.CreateParameter();
                parameter.ParameterName = "$p" + c;
                cmd.Parameters.Add(parameter);
                return parameter;
            }).ToArray();
            cmd.Prepare();

            int reported = 0;
            for (int r = 0; r < rows.Count; r++)
            {
                var row = rows[r];
                for (int c = 0; c < parameters.Length; c++)
                    parameters[c].Value = row[c] ?? DBNull.Value;
                cmd.ExecuteNonQuery();

                if (rowsInserted != null && r + 1 - reported >= ProgressInterval)
                {
                    rowsInserted(r + 1 - reported);
                    reported = r + 1;
                }
            }
            if (rowsInserted != null && reported < rows.Count) rowsInserted(rows.Count - reported);
        }
    }

    // A failed ROLLBACK must not replace the error that made the caller roll back. SQLite may already have rolled the
    // transaction back itself (disk full, I/O error), which Microsoft.Data.Sqlite tolerates; a rollback that still
    // throws (closed connection, I/O error during ROLLBACK) is reported here and the original error propagates.
    internal static class TransactionRollback
    {
        internal static void AfterFailure(SqliteTransaction transaction, TextWriter? log = null)
        {
            try
            {
                transaction.Rollback();
            }
            catch (Exception rollbackFailure)
            {
                (log ?? Console.Error).WriteLine($"Rollback after a failed transaction also failed: {rollbackFailure.GetType().Name}: {rollbackFailure.Message}");
            }
        }
    }

    public interface IRecordProcessor<T> where T : IMajorRecordGetter
    {
        Task ProcessRecordsAsync(IEnumerable<T> records, string recordType, SqliteConnection conn,
            SqliteTransaction? transaction = null, bool silent = false,
            int progressPhase = -1, CancellationToken ct = default);
    }

    public abstract class BaseRecordProcessor<T> : IRecordProcessor<T> where T : IMajorRecordGetter
    {
        protected static readonly JsonSerializerOptions _jsonOptions = new JsonSerializerOptions
        {
            ReferenceHandler = System.Text.Json.Serialization.ReferenceHandler.IgnoreCycles,
            WriteIndented = false,
            DefaultIgnoreCondition = System.Text.Json.Serialization.JsonIgnoreCondition.WhenWritingNull,
            Converters = { new TypeJsonConverter() }
        };

        private string? _currentFormKey;
        private string? _currentEditorId;

        protected void BeginRecord(string formKey, string? editorId)
        {
            _currentFormKey = formKey;
            _currentEditorId = editorId;
        }

        // Absent data is SQL NULL. A failed serialization is reported against the
        // record and column (the caller's argument expression) and also stored as
        // NULL, so a broken column is never mistaken for an empty "{}".
        protected string? SafeSerialize(object? obj, [CallerArgumentExpression(nameof(obj))] string column = "")
        {
            if (obj == null) return null;
            try
            {
                return JsonSerializer.Serialize(obj, _jsonOptions);
            }
            catch (Exception ex) when (ex is JsonException or NotSupportedException or InvalidOperationException or ArgumentException)
            {
                Console.Error.WriteLine(
                    $"  CONVERSION ERROR {typeof(T).Name} {_currentFormKey} ({_currentEditorId}): could not serialize `{column}`: {ex.GetType().Name}: {ex.Message}");
                return null;
            }
        }

        protected string? SerializeObjectBounds(IObjectBoundsGetter? bounds) =>
            SafeSerialize(bounds == null ? null : ObjectBoundsJson.From(bounds), "ObjectBounds");

        protected int TotalProcessed { get; set; }

        public virtual async Task ProcessRecordsAsync(IEnumerable<T> records, string recordType, SqliteConnection conn,
            SqliteTransaction? transaction = null, bool silent = false,
            int progressPhase = -1, CancellationToken ct = default)
        {
            // Materialize once — avoids double-enumeration of lazy Mutagen sequences.
            var list = records is IReadOnlyList<T> rl ? rl : records.ToList();
            int total = list.Count;
            if (total == 0) return;

            bool localTransaction = false;
            if (transaction == null)
            {
                // DEFERRED — BEGIN doesn't acquire a lock; first INSERT does.
                // With ~130 concurrent groups, BEGIN IMMEDIATE would serialise every
                // worker at BEGIN time and exhaust busy_timeout. DEFERRED lets workers
                // overlap JSON serialisation and queue at INSERT time instead.
                transaction = conn.BeginTransaction(IsolationLevel.ReadCommitted, deferred: true);
                localTransaction = true;
            }

            var sw = Stopwatch.StartNew();
            try
            {
                // Phase 1 — collect identifiers (parallel, thread-safe).
                var formKeys = new string[total];
                var editorIds = new string?[total];
                var plugins = new string[total];

                Parallel.For(0, total, i =>
                {
                    var r = list[i];
                    formKeys[i] = r.FormKey.ToString();
                    editorIds[i] = r.EditorID;
                    plugins[i] = r.FormKey.ModKey.FileName.ToString();
                });

                // Phase 2 — batch-insert all rows into the records table.
                var recordRows = new object?[total][];
                for (int i = 0; i < total; i++)
                    recordRows[i] = new object?[] { formKeys[i], recordType, editorIds[i], plugins[i] };

                BatchInsert.Execute(conn, transaction, "records",
                    EsmColumns.Records,
                    recordRows);

                // Phase 3 — per-record type-specific inserts (sequential; subclasses own their state).
                TotalProcessed = 0;
                for (int i = 0; i < total; i++)
                {
                    if (TotalProcessed % 1000 == 0) ct.ThrowIfCancellationRequested();
                    BeginRecord(formKeys[i], editorIds[i]);
                    await ProcessSpecificRecordAsync(list[i], formKeys[i], conn, transaction);
                    TotalProcessed++;
                    InstallProgress.Tick(progressPhase);
                    if (!silent && TotalProcessed % 5000 == 0)
                        Console.WriteLine($"  {TotalProcessed}/{total} {recordType} ({(100.0 * TotalProcessed / total):F1}%) — {sw.Elapsed:mm\\:ss} elapsed");
                }

                if (localTransaction)
                    transaction.Commit();

                if (!silent && TotalProcessed > 0)
                {
                    sw.Stop();
                    Console.WriteLine($"Done. {TotalProcessed} {recordType} processed in {sw.Elapsed:mm\\:ss}.");
                    PrintSummary();
                }
            }
            catch (Exception ex)
            {
                if (localTransaction)
                    TransactionRollback.AfterFailure(transaction);

                Console.WriteLine($"Error processing {recordType}: {ex.Message}");
                throw;
            }
        }

        protected virtual void PrintSummary() { }

        /// Per-record work of the default ProcessRecordsAsync loop; processors that
        /// override ProcessRecordsAsync with their own batched path never reach it.
        protected virtual Task ProcessSpecificRecordAsync(T record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
            => throw new NotSupportedException($"{GetType().Name} overrides ProcessRecordsAsync and has no per-record path");
    }

    public class SimpleRecordProcessor<T> : BaseRecordProcessor<T> where T : IMajorRecordGetter
    {
        private readonly string _table;
        public SimpleRecordProcessor(string table) => _table = table;

        protected override async Task ProcessSpecificRecordAsync(T record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            await conn.ExecuteAsync(
                $"INSERT OR REPLACE INTO {_table} (form_key, editor_id) VALUES (@FormKey, @EditorId)",
                new { FormKey = formKey, EditorId = record.EditorID }, transaction);
        }
    }

    // MSWP records: per-model material substitutions (red vs blue crates, car
    // paint variants, machine recolors). Substitutions are pre-resolved at
    // install time into final deployed texture paths (material_swap_entries)
    // so the engine only does exact string matching against the GLB "src:"
    // tokens — no BGSM parsing, no path repair at runtime. Needs the extracted
    // Materials staging root; without it this degrades to the old stub
    // behavior (form_key + editor_id only, no entries).
    public class MaterialSwapProcessor : BaseRecordProcessor<IMaterialSwapGetter>
    {
        private readonly string? _materialsRoot;
        private readonly string? _texturesOut;
        private readonly bool _resolveEntries;
        private int _entriesWritten, _subsSeen, _unresolvedOriginals, _unresolvedReplacements;
        private readonly HashSet<(string Diffuse, string Lut, float Row)> _lateBakeCombos = new();
        private readonly List<string> _unresolvedSamples = new();

        public MaterialSwapProcessor(string? materialsRoot, string? texturesOut)
        {
            _materialsRoot = materialsRoot;
            _texturesOut = texturesOut;
            _resolveEntries = materialsRoot != null;
            if (materialsRoot != null)
            {
                MaterialCensus.Build(materialsRoot);
                NifConverter.EnsureTexturePathContext(materialsRoot);
            }
        }

        protected override async Task ProcessSpecificRecordAsync(IMaterialSwapGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            await conn.ExecuteAsync(
                "INSERT OR REPLACE INTO material_swaps (form_key, editor_id) VALUES (@FormKey, @EditorId)",
                new { FormKey = formKey, EditorId = record.EditorID }, transaction);

            if (!_resolveEntries) return;

            foreach (var sub in record.Substitutions)
            {
                string? orig = sub.OriginalMaterial;
                string? repl = sub.ReplacementMaterial;
                // Vanilla has ~500 padding entries with null fields; skip them.
                if (string.IsNullOrWhiteSpace(orig) || string.IsNullOrWhiteSpace(repl))
                    continue;
                _subsSeen++;

                // FNAM color-remapping index: selects a different LUT band on a
                // grayscale-to-palette replacement. Absent = NaN/null.
                float? remap = null;
                try
                {
                    float? raw = sub.ColorRemappingIndex;
                    if (raw is float f && float.IsFinite(f)) remap = f;
                }
                catch (Exception ex) when (ReadFailureTally.IsReadFailure(ex))
                {
                    ReadFailureTally.Shared.Record("MSWP ColorRemappingIndex", ex, record.EditorID);
                }

                // MSWP paths are sloppy (bare filenames, missing materials/
                // prefix); a bare original may match several census files —
                // emit one entry per candidate, non-matching ones are inert.
                var originals = MaterialCensus.ResolveLoose(orig);
                if (originals.Count == 0)
                {
                    _unresolvedOriginals++;
                    if (_unresolvedSamples.Count < 20) _unresolvedSamples.Add($"orig '{orig}' ({record.EditorID})");
                    continue;
                }

                foreach (var original in originals)
                {
                    var replacement = MaterialCensus.ResolveLoose(repl, siblingKey: original.Key)
                        .OrderBy(d => d.Key, StringComparer.Ordinal)
                        .FirstOrDefault();
                    if (replacement == null)
                    {
                        _unresolvedReplacements++;
                        if (_unresolvedSamples.Count < 20) _unresolvedSamples.Add($"repl '{repl}' ({record.EditorID})");
                        continue;
                    }

                    string? diffuse = NifConverter.CleanTexturePathForDeploy(replacement.Diffuse, isDiffuse: true);
                    string? normal = NifConverter.CleanTexturePathForDeploy(replacement.Normal, isDiffuse: false);

                    if (replacement.IsPalette)
                    {
                        float row = remap ?? replacement.Row;
                        string texKey = MaterialCensus.NormalizeTexPath(replacement.Diffuse);
                        string baked = MaterialCensus.BakedDiffuseName(texKey, replacement.Lut, row);
                        if (baked != texKey)
                        {
                            diffuse = baked;
                            // Census-declared combos are baked by PaletteBaker.BakeAll;
                            // ColorRemappingIndex rows are only known here — bake late.
                            lock (_lateBakeCombos)
                                _lateBakeCombos.Add((texKey, replacement.Lut, MaterialCensus.QuantizeRow(row)));
                        }
                    }

                    ParseAlphaToken(replacement.AlphaToken, out bool isBlend, out double alphaMult, out int? alphaTest);

                    await conn.ExecuteAsync(@"
                        INSERT INTO material_swap_entries
                            (swap_form_key, original_material, replacement_material, diffuse, normal, is_blend, alpha_mult, alpha_test, two_sided, material_name)
                        VALUES (@Swap, @Original, @Replacement, @Diffuse, @Normal, @IsBlend, @AlphaMult, @AlphaTest, @TwoSided, @MaterialName)",
                        new
                        {
                            Swap = formKey,
                            Original = original.Key,
                            Replacement = replacement.Key,
                            Diffuse = diffuse,
                            Normal = normal,
                            IsBlend = isBlend ? 1 : 0,
                            AlphaMult = alphaMult,
                            AlphaTest = alphaTest,
                            TwoSided = replacement.TwoSided ? 1 : 0,
                            MaterialName = MaterialNameCodec.ForDefinition(replacement, diffuse, normal),
                        }, transaction);
                    _entriesWritten++;
                }
            }
        }

        // Inverse of MaterialCensus.GetAlphaToken — the engine reads numeric
        // columns instead of re-parsing the token grammar. Shared with the
        // MODC synthesizer below.
        internal static void ParseAlphaToken(string token, out bool isBlend, out double alphaMult, out int? alphaTest)
        {
            isBlend = false;
            alphaMult = 1.0;
            alphaTest = null;
            if (string.IsNullOrEmpty(token)) return;
            if (token.StartsWith("blend", StringComparison.Ordinal))
            {
                isBlend = true;
                var parts = token.Split('|');
                if (parts.Length > 1 && double.TryParse(parts[1], System.Globalization.NumberStyles.Float,
                        System.Globalization.CultureInfo.InvariantCulture, out var mult))
                    alphaMult = mult;
            }
            else if (token.StartsWith("test:", StringComparison.Ordinal) && int.TryParse(token.AsSpan(5), out var cutoff))
            {
                alphaTest = cutoff;
            }
        }

        public override async Task ProcessRecordsAsync(IEnumerable<IMaterialSwapGetter> records, string recordType, SqliteConnection conn,
            SqliteTransaction? transaction = null, bool silent = false,
            int progressPhase = -1, CancellationToken ct = default)
        {
            await base.ProcessRecordsAsync(records, recordType, conn, transaction, silent, progressPhase, ct);
            BakeLateCombos();
        }

        private void BakeLateCombos()
        {
            if (_lateBakeCombos.Count == 0) return;
            if (PaletteBaker.DeferLateBakes)
            {
                // Pipelined install: textures may still be extracting; the
                // installer drains the queue after texture deployment.
                foreach (var c in _lateBakeCombos) PaletteBaker.PendingLateBakes.Enqueue(c);
                Console.WriteLine($"  MSWP: deferred {_lateBakeCombos.Count} ColorRemappingIndex palette bakes until textures are deployed.");
                return;
            }
            if (_texturesOut == null)
            {
                Console.WriteLine($"  MSWP: {_lateBakeCombos.Count} ColorRemappingIndex palette variants need baking but no textures output was provided — run the full installer or bake-palette.");
                return;
            }
            var materialsRoot = _materialsRoot
                ?? throw new InvalidOperationException("Late palette bakes were queued without a materials root.");
            Console.WriteLine($"  MSWP: baking {_lateBakeCombos.Count} ColorRemappingIndex palette variants...");
            int ok = 0;
            foreach (var (diffuse, lut, row) in _lateBakeCombos)
                if (PaletteBaker.BakeCombo(materialsRoot, _texturesOut, diffuse, lut, row)) ok++;
            Console.WriteLine($"  MSWP: {ok}/{_lateBakeCombos.Count} late palette bakes done.");
        }

        protected override void PrintSummary()
        {
            Console.WriteLine($"  Substitutions resolved: {_entriesWritten} entries from {_subsSeen} subs " +
                              $"({_unresolvedOriginals} originals / {_unresolvedReplacements} replacements unresolved)");
            foreach (var s in _unresolvedSamples)
                Console.WriteLine($"    MISS {s}");
        }
    }

    public class GameSettingsProcessor : BaseRecordProcessor<IGameSettingGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IGameSettingGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"
                INSERT OR REPLACE INTO game_settings (form_key, editor_id, value_type, value_int, value_float, value_string)
                VALUES (@FormKey, @EditorId, @ValueType, @ValueInt, @ValueFloat, @ValueString)";

            string type = "unknown";
            int? vInt = null;
            double? vFloat = null;
            string? vString = null;

            if (record is IGameSettingIntGetter i) { type = "int"; vInt = i.Data; }
            else if (record is IGameSettingFloatGetter f) { type = "float"; vFloat = f.Data; }
            else if (record is IGameSettingStringGetter s) { type = "string"; vString = s.Data?.ToString(); }
            else if (record is IGameSettingBoolGetter b) { type = "bool"; vInt = (b.Data ?? false) ? 1 : 0; }

            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                ValueType = type,
                ValueInt = vInt,
                ValueFloat = vFloat,
                ValueString = vString
            }, transaction);
        }
    }

    public class GlobalsProcessor : BaseRecordProcessor<IGlobalGetter>
    {
        private double _min = double.MaxValue;
        private double _max = double.MinValue;

        internal static (string ValueType, double? Value) Read(IGlobalGetter record, string subject) => record switch
        {
            IGlobalFloatGetter global => ("float", global.Data),
            IGlobalShortGetter global => ("short", global.Data),
            IGlobalIntGetter global => ("int", global.Data),
            IGlobalBoolGetter global => ("bool", global.Data is bool flag ? (flag ? 1 : 0) : null),
            _ => throw new UnsupportedEsmDataException($"{subject}: {record.GetType().Name} is not a float, short, int or bool global."),
        };

        protected override async Task ProcessSpecificRecordAsync(IGlobalGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"
                INSERT OR REPLACE INTO globals (form_key, editor_id, value_type, value)
                VALUES (@FormKey, @EditorId, @ValueType, @Value)";

            var (valueType, value) = Read(record, $"GLOB {formKey} ({record.EditorID})");
            if (value is double number)
            {
                _min = Math.Min(_min, number);
                _max = Math.Max(_max, number);
            }

            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                ValueType = valueType,
                Value = value
            }, transaction);
        }

        protected override void PrintSummary()
        {
            Console.WriteLine($"  Globals Value Range: {_min} to {_max}");
        }
    }

    public class KeywordProcessor : BaseRecordProcessor<IKeywordGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IKeywordGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"
                INSERT OR REPLACE INTO keywords (form_key, editor_id, color)
                VALUES (@FormKey, @EditorId, @Color)";

            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Color = SafeSerialize(record.Color)
            }, transaction);
        }
    }

    public class RaceProcessor : BaseRecordProcessor<IRaceGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IRaceGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"
                INSERT OR REPLACE INTO races (form_key, editor_id, name, description, skin, body_data, flags, starting_stats, size, spells)
                VALUES (@FormKey, @EditorId, @Name, @Description, @Skin, @BodyData, @Flags, @StartingStats, @Size, @Spells)";

            var spells = record.ActorEffect?.Select(s => s.FormKey.ToString()).ToList();

            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Description = record.Description?.ToString(),
                Skin = record.Skin.IsNull ? null : record.Skin.FormKey.ToString(),
                BodyData = SafeSerialize(record.BodyData),
                Flags = SafeSerialize(record.Flags),
                StartingStats = (string?)null,
                Size = record.Size.ToString(),
                Spells = SafeSerialize(spells)
            }, transaction);
        }
    }

    public class NpcProcessor : BaseRecordProcessor<INpcGetter>
    {
        private int _maxLevel;
        private int _withHealth;

        protected override async Task ProcessSpecificRecordAsync(INpcGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"
                INSERT OR REPLACE INTO npcs (form_key, editor_id, name, race, class, voice_type, health, magicka, stamina, level, disposition, aggression, confidence, flags, appearance, ai_data)
                VALUES (@FormKey, @EditorId, @Name, @Race, @Class, @VoiceType, @Health, @Magicka, @Stamina, @Level, @Disposition, @Aggression, @Confidence, @Flags, @Appearance, @AiData)";

            short level = 0;
            level = ReadFailureTally.Shared.Read("NPC_ Level", record.EditorID, () => (short)((dynamic)record.Level).Level, (short)0);
            if (level > _maxLevel) _maxLevel = level;

            int health = (int)record.CalculatedHealth;
            if (health > 0) _withHealth++;

            string? flags = SafeSerialize(record.Flags);
            string? aiData = SafeSerialize(new
            {
                Mood = record.Mood,
                Assistance = record.Assistance,
                EnergyLevel = record.EnergyLevel,
                Responsibility = record.Responsibility
            });

            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Race = record.Race.FormKey.ToString(),
                Class = record.Class.IsNull ? null : record.Class.FormKey.ToString(),
                VoiceType = record.Voice.IsNull ? null : record.Voice.FormKey.ToString(),
                Health = health,
                Magicka = (int?)null,
                Stamina = (int)record.CalculatedActionPoints,
                Level = (int)level,
                Disposition = (int)record.DispositionBase,
                Aggression = (int)record.Aggression,
                Confidence = (int)record.Confidence,
                Flags = flags,
                Appearance = SafeSerialize(record.Weight),
                AiData = aiData
            }, transaction);
        }

        protected override void PrintSummary()
        {
            Console.WriteLine($"  Max Level: {_maxLevel}");
            Console.WriteLine($"  NPCs with explicit Health: {_withHealth}");
        }
    }

    public class WeaponProcessor : BaseRecordProcessor<IWeaponGetter>
    {
        private int _maxBaseDamage;

        protected override async Task ProcessSpecificRecordAsync(IWeaponGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string itemSql = @"
                INSERT OR REPLACE INTO items (form_key, item_type, editor_id, name, value, weight, model_path, pickup_sound, drop_sound, keywords)
                VALUES (@FormKey, 'WEAP', @EditorId, @Name, @Value, @Weight, @ModelPath, @PickupSound, @DropSound, @Keywords)";

            await conn.ExecuteAsync(itemSql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Value = (int)record.Value,
                Weight = record.Weight,
                ModelPath = record.Model?.File,
                PickupSound = record.PickUpSound.IsNull ? (string?)null : record.PickUpSound.FormKey.ToString(),
                DropSound = record.PutDownSound.IsNull ? (string?)null : record.PutDownSound.FormKey.ToString(),
                Keywords = SafeSerialize(FormKeyStrings.Of(record.Keywords))
            }, transaction);

            const string weapSql = @"
                INSERT OR REPLACE INTO weapons (form_key, weapon_type, damage, attack_speed, reach, crit_damage, crit_multiplier, ammo_type, magazine_size, fire_rate, range, accuracy, aim_model, enchantment, template, default_mods, object_template)
                VALUES (@FormKey, @WeaponType, @Damage, @AttackSpeed, @Reach, @CritDamage, @CritMultiplier, @AmmoType, @MagazineSize, @FireRate, @Range, @Accuracy, @AimModel, @Enchantment, @Template, @DefaultMods, @ObjectTemplate)";

            int currentDamage = (int)record.BaseDamage;
            if (currentDamage > _maxBaseDamage) _maxBaseDamage = currentDamage;

            await conn.ExecuteAsync(weapSql, new
            {
                FormKey = formKey,
                WeaponType = record.AnimationType.ToString(),
                Damage = currentDamage,
                AttackSpeed = record.Speed,
                Reach = record.Reach,
                CritDamage = (int)record.SecondaryDamage,
                CritMultiplier = record.CritDamageMult,
                AmmoType = record.Ammo.IsNull ? (string?)null : record.Ammo.FormKey.ToString(),
                MagazineSize = (int)record.Capacity,
                FireRate = record.AttackDelay,
                Range = record.MaxRange,
                Accuracy = (float)record.AccuracyBonus,
                AimModel = record.AimModel.IsNull ? (string?)null : record.AimModel.FormKey.ToString(),
                Enchantment = record.ObjectEffect.IsNull ? (string?)null : record.ObjectEffect.FormKey.ToString(),
                Template = record.Template.IsNull ? (string?)null : record.Template.FormKey.ToString(),
                DefaultMods = WeaponModJson.DefaultMods(record),
                ObjectTemplate = WeaponModJson.ObjectTemplates(record)
            }, transaction);
        }

        protected override void PrintSummary()
        {
            Console.WriteLine($"  Max Base Damage: {_maxBaseDamage}");
        }
    }

    public class ArmorProcessor : BaseRecordProcessor<IArmorGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IArmorGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string itemSql = @"
                INSERT OR REPLACE INTO items (form_key, item_type, editor_id, name, value, weight, model_path, keywords)
                VALUES (@FormKey, 'ARMO', @EditorId, @Name, @Value, @Weight, @ModelPath, @Keywords)";

            // Armor uses WorldModel (GenderedItem) usually
            string? modelPath = null;
            if (record.WorldModel != null && record.WorldModel.Any())
            {
                modelPath = record.WorldModel.FirstOrDefault()?.Model?.File;
            }

            await conn.ExecuteAsync(itemSql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Value = record.Value,
                Weight = record.Weight,
                ModelPath = modelPath,
                Keywords = SafeSerialize(FormKeyStrings.Of(record.Keywords))
            }, transaction);

            const string armorSql = @"
                INSERT OR REPLACE INTO armors (form_key, armor_rating, body_slot, armor_type, race, enchantment, template)
                VALUES (@FormKey, @ArmorRating, @BodySlot, @ArmorType, @Race, @Enchantment, @Template)";

            int rating = record.ArmorRating;

            int bodySlot = 0;
            if (record.BipedBodyTemplate != null)
            {
                bodySlot = (int)record.BipedBodyTemplate.FirstPersonFlags;
            }

            string? template = record.TemplateArmor.IsNull ? null : record.TemplateArmor.FormKey.ToString();
            string? enchantment = record.ObjectEffect.IsNull ? null : record.ObjectEffect.FormKey.ToString();

            await conn.ExecuteAsync(armorSql, new
            {
                FormKey = formKey,
                ArmorRating = rating,
                BodySlot = bodySlot,
                ArmorType = "Unknown",
                Race = record.Race.IsNull ? null : record.Race.FormKey.ToString(),
                Enchantment = enchantment,
                Template = template
            }, transaction);
        }
    }

    public class AmmoProcessor : BaseRecordProcessor<IAmmunitionGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IAmmunitionGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string itemSql = @"
                INSERT OR REPLACE INTO items (form_key, item_type, editor_id, name, value, weight, model_path, pickup_sound, drop_sound, keywords)
                VALUES (@FormKey, 'AMMO', @EditorId, @Name, @Value, @Weight, @ModelPath, @PickupSound, @DropSound, @Keywords)";

            await conn.ExecuteAsync(itemSql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Value = record.Value,
                Weight = record.Weight,
                ModelPath = record.Model?.File,
                PickupSound = record.PickUpSound.IsNull ? (string?)null : record.PickUpSound.FormKey.ToString(),
                DropSound = record.PutDownSound.IsNull ? (string?)null : record.PutDownSound.FormKey.ToString(),
                Keywords = SafeSerialize(FormKeyStrings.Of(record.Keywords))
            }, transaction);
        }
    }

    public class MiscItemProcessor : BaseRecordProcessor<IMiscItemGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IMiscItemGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string itemSql = @"
                INSERT OR REPLACE INTO items (form_key, item_type, editor_id, name, value, weight, model_path, pickup_sound, drop_sound, keywords)
                VALUES (@FormKey, 'MISC', @EditorId, @Name, @Value, @Weight, @ModelPath, @PickupSound, @DropSound, @Keywords)";

            await conn.ExecuteAsync(itemSql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Value = record.Value,
                Weight = record.Weight,
                ModelPath = record.Model?.File,
                PickupSound = record.PickUpSound.IsNull ? (string?)null : record.PickUpSound.FormKey.ToString(),
                DropSound = record.PutDownSound.IsNull ? (string?)null : record.PutDownSound.FormKey.ToString(),
                Keywords = SafeSerialize(FormKeyStrings.Of(record.Keywords))
            }, transaction);
        }
    }

    public class BookProcessor : BaseRecordProcessor<IBookGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IBookGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string itemSql = @"
                INSERT OR REPLACE INTO items (form_key, item_type, editor_id, name, value, weight, model_path, pickup_sound, drop_sound, keywords)
                VALUES (@FormKey, 'BOOK', @EditorId, @Name, @Value, @Weight, @ModelPath, @PickupSound, @DropSound, @Keywords)";

            await conn.ExecuteAsync(itemSql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Value = record.Value,
                Weight = record.Weight,
                ModelPath = record.Model?.File,
                PickupSound = record.PickUpSound.IsNull ? (string?)null : record.PickUpSound.FormKey.ToString(),
                DropSound = record.PutDownSound.IsNull ? (string?)null : record.PutDownSound.FormKey.ToString(),
                Keywords = SafeSerialize(FormKeyStrings.Of(record.Keywords))
            }, transaction);
        }
    }

    public class IngestibleProcessor : BaseRecordProcessor<IIngestibleGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IIngestibleGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string itemSql = @"
                INSERT OR REPLACE INTO items (form_key, item_type, editor_id, name, value, weight, model_path, pickup_sound, drop_sound, keywords)
                VALUES (@FormKey, 'ALCH', @EditorId, @Name, @Value, @Weight, @ModelPath, @PickupSound, @DropSound, @Keywords)";

            await conn.ExecuteAsync(itemSql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Value = record.Value,
                Weight = record.Weight,
                ModelPath = record.Model?.File,
                PickupSound = record.PickUpSound.IsNull ? (string?)null : record.PickUpSound.FormKey.ToString(),
                DropSound = record.PutDownSound.IsNull ? (string?)null : record.PutDownSound.FormKey.ToString(),
                Keywords = SafeSerialize(FormKeyStrings.Of(record.Keywords))
            }, transaction);
        }
    }

    public class WorldspaceProcessor : BaseRecordProcessor<IWorldspaceGetter>
    {
        private const int DeletedRecordFlag = 0x20;

        protected override async Task ProcessSpecificRecordAsync(IWorldspaceGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"
                INSERT OR REPLACE INTO worldspaces (form_key, editor_id, name, parent_worldspace, flags, world_map_data, climate,
                    water_type, lod_water_type, lod_water_height, default_land_height, default_water_height, water_environment_map, parent_flags, map_image)
                VALUES (@FormKey, @EditorId, @Name, @Parent, @Flags, @MapData, @Climate,
                    @WaterType, @LodWaterType, @LodWaterHeight, @DefaultLandHeight, @DefaultWaterHeight, @WaterEnvironmentMap, @ParentFlags, @MapImage)";

            string? parent = record.Parent?.Worldspace?.FormKey.ToString();
            string? mapData = SafeSerialize(record.MapData);

            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Parent = parent,
                ParentFlags = record.Parent is { } worldParent ? (int?)(ushort)worldParent.Flags : null,
                Flags = SafeSerialize(record.Flags),
                MapData = mapData,
                Climate = record.Climate.IsNull ? (string?)null : record.Climate.FormKey.ToString(),
                WaterType = record.Water.IsNull ? (string?)null : record.Water.FormKey.ToString(),
                LodWaterType = record.LodWater.IsNull ? (string?)null : record.LodWater.FormKey.ToString(),
                LodWaterHeight = record.LodWaterHeight,
                DefaultLandHeight = record.LandDefaults?.DefaultLandHeight,
                DefaultWaterHeight = record.LandDefaults?.DefaultWaterHeight,
                WaterEnvironmentMap = record.WaterEnvironmentMap,
                MapImage = record.MapImage
            }, transaction);

            await InsertMapMarkersAsync(record, formKey, conn, transaction);
        }

        private static async Task InsertMapMarkersAsync(IWorldspaceGetter worldspace, string worldspaceKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"
                INSERT OR REPLACE INTO map_markers (form_key, worldspace, name, marker_type, flags, position_x, position_y, position_z)
                VALUES (@FormKey, @Worldspace, @Name, @MarkerType, @Flags, @X, @Y, @Z)";

            foreach (var reference in worldspace.TopCell?.Persistent ?? [])
            {
                if (reference is not IPlacedObjectGetter { MapMarker: { } marker } placed) continue;
                if ((placed.MajorRecordFlagsRaw & DeletedRecordFlag) != 0) continue;

                await conn.ExecuteAsync(sql, new
                {
                    FormKey = placed.FormKey.ToString(),
                    Worldspace = worldspaceKey,
                    Name = marker.Name?.String,
                    MarkerType = (int)marker.Type,
                    Flags = (int)marker.Flags,
                    X = placed.Position.X,
                    Y = placed.Position.Y,
                    Z = placed.Position.Z
                }, transaction);
            }
        }
    }

    public class CellProcessor : BaseRecordProcessor<ICellGetter>
    {
        private int _interiorCount;
        private int _exteriorCount;
        private int _withGrid;
        private int _placedObjectsProcessed;
        private int _teleportsProcessed;
        private int _landscapesProcessed;
        private string? _worldspaceKey;
        private readonly HashSet<string> _precombinedRefs;
        // REFR form key -> MSWP form key, pre-built by ScanMaterialSwapLinks()
        // (XMSP is not exposed by Mutagen for FO4 placed objects).
        private readonly Dictionary<string, string> _refrMaterialSwaps;
        // CELL / REFR form key -> JSON [[x,y,z,w],...] of XWCN/XWCU, pre-built by WaterCurrentScan.
        private readonly Dictionary<string, string> _waterCurrents;

        public CellProcessor(string? worldspaceKey = null, HashSet<string>? precombinedRefs = null, Dictionary<string, string>? refrMaterialSwaps = null,
            Dictionary<string, string>? waterCurrents = null)
        {
            _waterCurrents = waterCurrents ?? new Dictionary<string, string>();
            _worldspaceKey = worldspaceKey;
            _precombinedRefs = precombinedRefs ?? new HashSet<string>();
            _refrMaterialSwaps = refrMaterialSwaps ?? new Dictionary<string, string>();
        }

        // Scans a raw ESM file for all XCRI subrecords in CELL records and returns
        // the set of precombined REFR form keys (e.g. "03B55E:Fallout4.esm").
        // Mutagen's CombinedMeshReferences fails to parse FO4 XCRI because the
        // subrecord format is: [uint32 meshCount][uint32 refrCount][meshCount × uint32][refrCount × uint32]
        // which doesn't match Mutagen's expected 8-byte-per-entry struct layout.
        public static HashSet<string> ScanXcriFromEsm(string esmPath)
        {
            var result = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            try
            {
                var data = File.ReadAllBytes(esmPath);
                string pluginName = Path.GetFileName(esmPath);

                // Read master list from TES4 header so we can resolve plugin indices.
                var masters = ReadFormIdPluginTable(data, pluginName);

                int fileLen = data.Length;
                int pos = 0;

                // Skip TES4 record (first record in file).
                if (fileLen < 24) return result;
                int tes4DataSize = BitConverter.ToInt32(data, pos + 4);
                pos += 24 + tes4DataSize;

                // Traverse top-level groups.
                ScanGroupForCells(data, pos, fileLen, masters, result);
            }
            catch (Exception ex)
            {
                Console.WriteLine($"[WARN] XCRI raw scan failed: {ex.Message}");
            }
            return result;
        }

        /// <summary>FormID plugin-index table: the MAST masters in order, then the plugin itself.</summary>
        internal static List<string> ReadFormIdPluginTable(byte[] data, string pluginName)
        {
            var plugins = new List<string>();
            ReadTes4Masters(data, plugins);
            plugins.Add(pluginName);
            return plugins;
        }

        private static void ReadTes4Masters(byte[] data, List<string> masters)
        {
            // TES4 subrecords start at offset 24.
            int i = 24;
            int tes4DataSize = BitConverter.ToInt32(data, 4);
            int tes4End = 24 + tes4DataSize;
            while (i + 6 <= tes4End && i + 6 <= data.Length)
            {
                string subSig = System.Text.Encoding.ASCII.GetString(data, i, 4);
                int subSize = BitConverter.ToUInt16(data, i + 4);
                if (subSig == "MAST" && subSize > 0)
                {
                    // Null-terminated string
                    int nullPos = Array.IndexOf(data, (byte)0, i + 6, subSize);
                    int nameLen = nullPos >= 0 ? nullPos - (i + 6) : subSize;
                    string masterName = System.Text.Encoding.ASCII.GetString(data, i + 6, nameLen);
                    masters.Add(masterName);
                }
                i += 6 + subSize;
            }
        }

        /// <summary>Inflates a compressed record's body (a u32 decompressed size, then a zlib stream); logs and returns false when the stream is corrupt.</summary>
        internal static bool TryInflateRecord(byte[] data, int pos, int recEnd, string scanName, [System.Diagnostics.CodeAnalysis.NotNullWhen(true)] out byte[]? recData)
        {
            try
            {
                using var src = new System.IO.MemoryStream(data, pos + 28, recEnd - (pos + 28));
                using var zlib = new System.IO.Compression.ZLibStream(src, System.IO.Compression.CompressionMode.Decompress);
                using var dst = new System.IO.MemoryStream();
                zlib.CopyTo(dst);
                recData = dst.ToArray();
                return true;
            }
            catch (InvalidDataException ex)
            {
                Console.WriteLine($"[WARN] {scanName}: skipping {System.Text.Encoding.ASCII.GetString(data, pos, 4)} record {BitConverter.ToUInt32(data, pos + 12):X8} at offset {pos}, zlib decompression failed: {ex.Message}");
                recData = null;
                return false;
            }
        }

        private static void ScanGroupForCells(byte[] data, int pos, int fileLen,
            List<string> masters, HashSet<string> result)
        {
            while (pos + 24 <= fileLen)
            {
                string sig = System.Text.Encoding.ASCII.GetString(data, pos, 4);
                if (sig == "GRUP")
                {
                    int groupSize = BitConverter.ToInt32(data, pos + 4);
                    if (groupSize < 24 || pos + groupSize > fileLen) { pos += 24; continue; }
                    // Recurse into group contents (skip the 24-byte group header).
                    ScanGroupForCells(data, pos + 24, pos + groupSize, masters, result);
                    pos += groupSize;
                }
                else if (sig == "CELL")
                {
                    int dataSize = BitConverter.ToInt32(data, pos + 4);
                    int flags = BitConverter.ToInt32(data, pos + 8);
                    int recEnd = pos + 24 + dataSize;
                    if (recEnd > fileLen) { pos += 24; continue; }

                    byte[]? recData;
                    if ((flags & 0x00040000) != 0) // compressed
                    {
                        if (!TryInflateRecord(data, pos, recEnd, "XCRI raw scan", out recData)) { pos = recEnd; continue; }
                    }
                    else
                    {
                        recData = data[(pos + 24)..recEnd];
                    }

                    ExtractXcriFromRecord(recData, masters, result);
                    pos = recEnd;
                }
                else
                {
                    // Unknown record: read its size and skip.
                    if (pos + 8 > fileLen) break;
                    int dataSize = BitConverter.ToInt32(data, pos + 4);
                    pos += 24 + dataSize;
                }
            }
        }

        private static void ExtractXcriFromRecord(byte[] recData, List<string> masters, HashSet<string> result)
        {
            int i = 0;
            while (i + 6 <= recData.Length)
            {
                string subSig = System.Text.Encoding.ASCII.GetString(recData, i, 4);
                int subSize = BitConverter.ToUInt16(recData, i + 4);
                int subStart = i + 6;
                if (subStart + subSize > recData.Length) break;

                if (subSig == "XCRI" && subSize >= 8)
                {
                    // Format: [u32 meshCount][u32 refrCount][meshCount × u32 meshIDs][refrCount × u32 refrIDs]
                    int meshCount = BitConverter.ToInt32(recData, subStart);
                    int refrCount = BitConverter.ToInt32(recData, subStart + 4);
                    int refrStart = subStart + 8 + meshCount * 4;
                    int expected = 8 + meshCount * 4 + refrCount * 4;
                    if (meshCount >= 0 && refrCount >= 0 && expected <= subSize)
                    {
                        for (int j = 0; j < refrCount; j++)
                        {
                            int off = refrStart + j * 4;
                            if (off + 4 > subStart + subSize) break;
                            result.Add(ResolveFormKey(BitConverter.ToUInt32(recData, off), masters));
                        }
                    }
                    return; // only one XCRI per CELL
                }
                i += 6 + subSize;
            }
        }

        // ---- Material-swap link scan -----------------------------------------
        // Mutagen 0.50.1 does not expose XMSP (placed-ref material swap) or the
        // FO4 form of MODS/MODC on base records. In FO4, MODS is a 4-byte MSWP
        // formid (NOT Skyrim's alternate-texture list) and MODC is a float32
        // color-remapping index. Scan them raw, like ScanXcriFromEsm above.

        /// <summary>Record types whose MODS/MODC base-level swap we collect.
        /// STAT/MSTT carry nearly all of them; the rest are cheap to include.</summary>
        private static readonly HashSet<string> _swapBaseTypes = new(StringComparer.Ordinal)
        {
            "STAT", "MSTT", "CONT", "FURN", "ACTI", "DOOR", "FLOR", "SCOL",
            "MISC", "TERM", "ALCH", "WEAP", "ARMO", "BOOK", "TACT",
        };

        /// <summary>Placed-ref types that can carry XMSP.</summary>
        private static readonly HashSet<string> _swapRefTypes = new(StringComparer.Ordinal)
        {
            "REFR", "ACHR", "PGRE",
        };

        public static (Dictionary<string, string> RefrSwaps,
                       List<(string FormKey, string? SwapFormKey, float? ColorRemapIndex)> BaseSwaps)
            ScanMaterialSwapLinks(string esmPath)
        {
            var refrSwaps = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            var baseSwaps = new List<(string, string?, float?)>();
            try
            {
                var data = File.ReadAllBytes(esmPath);

                // Formid plugin-index semantics: indices 0..N-1 are the masters
                // in MAST order, index N is the plugin itself. (Fallout4.esm has
                // no masters, so everything resolves to index 0 = itself.)
                var masters = ReadFormIdPluginTable(data, Path.GetFileName(esmPath));

                int fileLen = data.Length;
                if (fileLen < 24) return (refrSwaps, baseSwaps);
                int tes4DataSize = BitConverter.ToInt32(data, 4);
                ScanGroupForSwapLinks(data, 24 + tes4DataSize, fileLen, masters, refrSwaps, baseSwaps);
            }
            catch (Exception ex)
            {
                Console.WriteLine($"[WARN] material-swap raw scan failed: {ex.Message}");
            }
            return (refrSwaps, baseSwaps);
        }

        private static void ScanGroupForSwapLinks(byte[] data, int pos, int end,
            List<string> masters, Dictionary<string, string> refrSwaps,
            List<(string, string?, float?)> baseSwaps)
        {
            while (pos + 24 <= end)
            {
                string sig = System.Text.Encoding.ASCII.GetString(data, pos, 4);
                if (sig == "GRUP")
                {
                    int groupSize = BitConverter.ToInt32(data, pos + 4);
                    if (groupSize < 24 || pos + groupSize > end) { pos += 24; continue; }
                    ScanGroupForSwapLinks(data, pos + 24, pos + groupSize, masters, refrSwaps, baseSwaps);
                    pos += groupSize;
                    continue;
                }

                int dataSize = BitConverter.ToInt32(data, pos + 4);
                int recEnd = pos + 24 + dataSize;
                if (recEnd > end) break;

                bool isRef = _swapRefTypes.Contains(sig);
                bool isBase = !isRef && _swapBaseTypes.Contains(sig);
                if (isRef || isBase)
                {
                    int flags = BitConverter.ToInt32(data, pos + 8);
                    uint rawFormId = BitConverter.ToUInt32(data, pos + 12);

                    byte[]? recData = null;
                    if ((flags & 0x00040000) != 0) // compressed
                    {
                        TryInflateRecord(data, pos, recEnd, "material swap raw scan", out recData);
                    }
                    else
                    {
                        recData = data[(pos + 24)..recEnd];
                    }

                    if (recData != null)
                    {
                        var (swapId, modc) = ExtractSwapLinkFromRecord(recData, isRef);
                        if (swapId.HasValue || modc.HasValue)
                        {
                            string formKey = ResolveFormKey(rawFormId, masters);
                            string? swapKey = swapId.HasValue ? ResolveFormKey(swapId.Value, masters) : null;
                            if (isRef)
                            {
                                if (swapKey != null) refrSwaps[formKey] = swapKey;
                            }
                            else
                            {
                                baseSwaps.Add((formKey, swapKey, modc));
                            }
                        }
                    }
                }

                pos = recEnd;
            }
        }

        /// <summary>Walks subrecords for XMSP (refs) or MODS + MODC (bases).
        /// Handles the XXXX extended-size convention for completeness.</summary>
        private static (uint? SwapFormId, float? Modc) ExtractSwapLinkFromRecord(byte[] recData, bool isRef)
        {
            uint? swapId = null;
            float? modc = null;
            int i = 0;
            int extendedSize = 0;
            while (i + 6 <= recData.Length)
            {
                string subSig = System.Text.Encoding.ASCII.GetString(recData, i, 4);
                int subSize = BitConverter.ToUInt16(recData, i + 4);
                int subStart = i + 6;
                if (extendedSize > 0)
                {
                    subSize = extendedSize;
                    extendedSize = 0;
                }
                if (subStart + subSize > recData.Length) break;

                if (subSig == "XXXX" && subSize == 4)
                {
                    extendedSize = BitConverter.ToInt32(recData, subStart);
                }
                else if (isRef)
                {
                    if (subSig == "XMSP" && subSize == 4)
                    {
                        swapId = BitConverter.ToUInt32(recData, subStart);
                        return (swapId, null); // one XMSP per ref
                    }
                }
                else
                {
                    if (subSig == "MODS" && subSize == 4)
                        swapId = BitConverter.ToUInt32(recData, subStart);
                    else if (subSig == "MODC" && subSize == 4)
                        modc = BitConverter.ToSingle(recData, subStart);
                    if (swapId.HasValue && modc.HasValue)
                        return (swapId, modc);
                }

                i = subStart + subSize;
            }
            return (swapId, modc);
        }

        internal static string ResolveFormKey(uint rawFormId, List<string> masters)
        {
            int pluginIdx = (int)(rawFormId >> 24);
            uint localId = rawFormId & 0x00FFFFFFu;
            string plugin = pluginIdx < masters.Count ? masters[pluginIdx] : masters[^1];
            return $"{localId:X6}:{plugin}";
        }

        // XRDS radius and the XLIG block of a light REFR. All null for refs that
        // carry neither, so the ~600k non-light rows stay NULL in these columns.
        private readonly record struct PlacedLightFields(
            float? RadiusOverride, float? FovPlusMinus, float? FadePlusMinus,
            float? EndDistanceCap, float? ShadowDepthBias, float? NearClip, float? VolumetricIntensity)
        {
            public static PlacedLightFields From(IPlacedGetter obj)
            {
                if (obj is not IPlacedObjectGetter placedObject) return default;
                var lighting = placedObject.Lighting;
                return new PlacedLightFields(
                    placedObject.Radius,
                    lighting?.Fov90PlusMinus, lighting?.Fade1PlusMinus,
                    lighting?.EndDistanceCap, lighting?.ShadowDepthBias,
                    lighting?.NearClip, lighting?.VolumetricIntensity);
            }
        }

        // XRGD ragdoll pose: one 28-byte entry per skeleton bone (bone id byte, 3 unused
        // bytes, 3 f32 position, 3 f32 euler rotation), serialized back to the raw layout.
        private static class PlacedRagdollPose
        {
            private const int EntrySize = 28;
            private const int UnusedLength = 3;

            public static byte[]? From(IPlacedGetter obj)
            {
                IReadOnlyList<IRagdollDataGetter>? entries = obj switch
                {
                    IPlacedObjectGetter po => po.RagdollData,
                    IPlacedNpcGetter npc => npc.RagdollData,
                    _ => null,
                };
                if (entries == null || entries.Count == 0) return null;

                var bytes = new byte[entries.Count * EntrySize];
                var span = bytes.AsSpan();
                for (int i = 0; i < entries.Count; i++)
                {
                    var entry = entries[i];
                    if (entry.Unused.Length != UnusedLength)
                    {
                        Console.WriteLine($"  WARNING: XRGD of {obj.FormKey} entry {i} has {entry.Unused.Length} padding bytes (expected {UnusedLength}); ragdoll_pose left NULL.");
                        return null;
                    }
                    var e = span.Slice(i * EntrySize, EntrySize);
                    e[0] = entry.BoneId;
                    entry.Unused.Span.CopyTo(e.Slice(1, UnusedLength));
                    var floats = System.Runtime.InteropServices.MemoryMarshal.Cast<byte, float>(e.Slice(4));
                    floats[0] = entry.Position.X; floats[1] = entry.Position.Y; floats[2] = entry.Position.Z;
                    floats[3] = entry.Rotation.X; floats[4] = entry.Rotation.Y; floats[5] = entry.Rotation.Z;
                }
                return bytes;
            }
        }

        private readonly struct PlacedObjectRow
        {
            public readonly string FormKey;
            public readonly string CellFormKey;
            public readonly string BaseObject;
            public readonly string ObjectType;
            public readonly string? EditorId;
            public readonly string SourcePlugin;
            public readonly float PosX, PosY, PosZ;
            public readonly float RotX, RotY, RotZ;
            public readonly float Scale;
            public readonly int IsPersistent;
            public readonly int IsInitiallyDisabled;
            public readonly string? EnableParent;
            public readonly int EnableParentOpposite;
            public readonly int HasPrimitive;
            public readonly string? MaterialSwap;
            public readonly PlacedLightFields Light;
            public readonly int RecordFlags;
            public readonly byte[]? RagdollPose;
            public readonly string? WaterCurrents;

            public PlacedObjectRow(IPlacedGetter src, string cellFk, bool isPersistent,
                string baseObj, float px, float py, float pz,
                float rx, float ry, float rz, float scale,
                int initiallyDisabled, string? enableParent, int enableParentOpposite,
                int hasPrimitive, string? materialSwap, string? waterCurrents)
            {
                Light = PlacedLightFields.From(src);
                RecordFlags = src.MajorRecordFlagsRaw;
                RagdollPose = PlacedRagdollPose.From(src);
                WaterCurrents = waterCurrents;
                FormKey = src.FormKey.ToString();
                CellFormKey = cellFk;
                BaseObject = baseObj;
                ObjectType = src.GetType().Name;
                EditorId = src.EditorID;
                SourcePlugin = src.FormKey.ModKey.FileName.ToString();
                PosX = px; PosY = py; PosZ = pz;
                RotX = rx; RotY = ry; RotZ = rz;
                Scale = scale;
                IsPersistent = isPersistent ? 1 : 0;
                IsInitiallyDisabled = initiallyDisabled;
                EnableParent = enableParent;
                EnableParentOpposite = enableParentOpposite;
                HasPrimitive = hasPrimitive;
                MaterialSwap = materialSwap;
            }

            public object?[] ToPlacedObjectValues() => new object?[] {
                FormKey, CellFormKey, BaseObject, ObjectType,
                IsPersistent, PosX, PosY, PosZ,
                RotX, RotY, RotZ,
                Scale, IsInitiallyDisabled, EnableParent, EnableParentOpposite,
                HasPrimitive, MaterialSwap,
                Light.RadiusOverride, Light.FovPlusMinus, Light.FadePlusMinus,
                Light.EndDistanceCap, Light.ShadowDepthBias, Light.NearClip,
                Light.VolumetricIntensity, RecordFlags, RagdollPose, WaterCurrents
            };

            public object?[] ToRecordValues() => new object?[] { FormKey, "REFR", EditorId, SourcePlugin };
        }

        // One REFR XTEL (teleport destination): the door the player aims at, the door it
        // links to, and the position/rotation to arrive at. Collected alongside (not inside)
        // PlacedObjectRow so the ~600k placed_objects rows don't carry mostly-null teleport
        // fields through the hot insert path.
        private readonly struct TeleportRow
        {
            public readonly string SourceRef;
            public readonly string DestinationRef;
            public readonly float DestPosX, DestPosY, DestPosZ;
            public readonly float DestRotX, DestRotY, DestRotZ;

            public TeleportRow(string sourceRef, string destinationRef,
                float destPosX, float destPosY, float destPosZ,
                float destRotX, float destRotY, float destRotZ)
            {
                SourceRef = sourceRef;
                DestinationRef = destinationRef;
                DestPosX = destPosX; DestPosY = destPosY; DestPosZ = destPosZ;
                DestRotX = destRotX; DestRotY = destRotY; DestRotZ = destRotZ;
            }
        }

        // Replaces the base-class loop with a three-phase pipeline:
        //   1. Sequential: flatten all placed objects from all cells (Mutagen lazy reads)
        //   2. Parallel:   JSON-serialize all placed objects (CPU-bound)
        //   3. Sequential: batch-INSERT cells, placed_objects, records (I/O-bound)
        public override async Task ProcessRecordsAsync(IEnumerable<ICellGetter> records, string recordType, SqliteConnection conn,
            SqliteTransaction? transaction = null, bool silent = false,
            int progressPhase = -1, CancellationToken ct = default)
        {
            var list = records is IReadOnlyList<ICellGetter> rl ? rl : records.ToList();
            int cellCount = list.Count;
            if (cellCount == 0) return;

            bool localTx = transaction == null;
            transaction ??= conn.BeginTransaction(IsolationLevel.ReadCommitted, deferred: true);
            var sw = Stopwatch.StartNew();

            try
            {
                // ── Cell field extraction (sequential, cheap) ──────────────────────
                var cellFormKeys = new string[cellCount];
                var cellEditorIds = new string?[cellCount];
                var cellPlugins = new string[cellCount];
                var cellWorldspace = new string?[cellCount];
                var cellGridX = new int?[cellCount];
                var cellGridY = new int?[cellCount];
                var cellIsInterior = new int[cellCount];
                var cellFlags = new string?[cellCount];
                var cellLighting = new string?[cellCount];
                var cellLightingTemplate = new string?[cellCount];
                var cellEZone = new string?[cellCount];
                var cellHasWater = new int[cellCount];
                var cellNoLodWater = new int[cellCount];
                var cellWaterHeight = new float?[cellCount];
                var cellWaterType = new string?[cellCount];
                var cellWaterEnvMap = new string?[cellCount];
                var cellWaterCurrents = new string?[cellCount];

                long cellFieldsStart = EsmTiming.Now();
                for (int ci = 0; ci < cellCount; ci++)
                {
                    var cell = list[ci];
                    cellFormKeys[ci] = cell.FormKey.ToString();
                    cellEditorIds[ci] = cell.EditorID;
                    cellPlugins[ci] = cell.FormKey.ModKey.FileName.ToString();

                    bool isInterior = cell.Flags.HasFlag(Cell.Flag.IsInteriorCell);
                    if (isInterior) _interiorCount++; else _exteriorCount++;
                    cellIsInterior[ci] = isInterior ? 1 : 0;

                    cellWorldspace[ci] = _worldspaceKey;

                    var grid = cell.Grid;
                    if (grid != null)
                    {
                        try { cellGridX[ci] = grid.Point.X; cellGridY[ci] = grid.Point.Y; _withGrid++; }
                        catch (Exception ex) when (ReadFailureTally.IsReadFailure(ex))
                        {
                            ReadFailureTally.Shared.Record("CELL Grid.Point", ex, cell.EditorID);
                        }
                    }

                    BeginRecord(cellFormKeys[ci], cellEditorIds[ci]);
                    cellFlags[ci] = SafeSerialize(cell.Flags);
                    cellLighting[ci] = SafeSerialize(LightingJson.Project(cell.Lighting));
                    cellLightingTemplate[ci] = cell.LightingTemplate.IsNull ? null : cell.LightingTemplate.FormKey.ToString();
                    cellEZone[ci] = cell.EncounterZone.IsNull ? null : cell.EncounterZone.FormKey.ToString();
                    cellHasWater[ci] = cell.Flags.HasFlag(Cell.Flag.HasWater) ? 1 : 0;
                    cellNoLodWater[ci] = cell.Flags.HasFlag(Cell.Flag.NoLodWater) ? 1 : 0;
                    cellWaterHeight[ci] = cell.WaterHeight;
                    cellWaterType[ci] = cell.Water.IsNull ? null : cell.Water.FormKey.ToString();
                    cellWaterEnvMap[ci] = cell.WaterEnvironmentMap;
                    cellWaterCurrents[ci] = _waterCurrents.GetValueOrDefault(cellFormKeys[ci]);
                }
                EsmTiming.Add("cells.fields", cellFieldsStart, cellCount);

                // Batch-insert cells into records table
                var cellRecordRows = new object?[cellCount][];
                for (int ci = 0; ci < cellCount; ci++)
                    cellRecordRows[ci] = new object?[] { cellFormKeys[ci], "CELL", cellEditorIds[ci], cellPlugins[ci] };
                BatchInsert.Execute(conn, transaction, "records",
                    EsmColumns.Records,
                    cellRecordRows);

                // Batch-insert into cells table
                var cellRows = new object?[cellCount][];
                for (int ci = 0; ci < cellCount; ci++)
                    cellRows[ci] = new object?[] {
                        cellFormKeys[ci], cellEditorIds[ci], cellWorldspace[ci],
                        cellGridX[ci], cellGridY[ci], cellIsInterior[ci],
                        cellFlags[ci], cellLighting[ci], cellLightingTemplate[ci], cellEZone[ci],
                        cellHasWater[ci], cellNoLodWater[ci], cellWaterHeight[ci], cellWaterType[ci], cellWaterEnvMap[ci], cellWaterCurrents[ci]
                    };
                BatchInsert.Execute(conn, transaction, "cells",
                    EsmColumns.Cells,
                    cellRows);
                InstallProgress.Tick(progressPhase, cellCount);

                // ── Phase 1: Flatten all placed objects (sequential Mutagen reads) ──
                var allPlaced = new List<PlacedObjectRow>(capacity: 600_000);
                var allTeleports = new List<TeleportRow>(capacity: 4096);
                long flattenStart = EsmTiming.Now();
                for (int ci = 0; ci < cellCount; ci++)
                {
                    var cell = list[ci];
                    if (cell.Persistent != null) ExtractPlaced(cell.Persistent, cellFormKeys[ci], true, _refrMaterialSwaps, _waterCurrents, allPlaced, allTeleports);
                    if (cell.Temporary != null) ExtractPlaced(cell.Temporary, cellFormKeys[ci], false, _refrMaterialSwaps, _waterCurrents, allPlaced, allTeleports);
                }
                int poCount = allPlaced.Count;
                EsmTiming.Add("cells.flatten", flattenStart, poCount);
                _placedObjectsProcessed = poCount;

                if (poCount > 0)
                {
                    // ── Batch INSERT placed_objects ──────────────────────
                    var poRows = new object?[poCount][];
                    for (int i = 0; i < poCount; i++)
                    {
                        poRows[i] = allPlaced[i].ToPlacedObjectValues();
                    }
                    // Every row is inserted into placed_objects and records (REFR), plus one teleport row per door destination.
                    InstallProgress.AddTotal(progressPhase, 2L * poCount + allTeleports.Count);
                    Action<int> rowsInserted = count => InstallProgress.Tick(progressPhase, count);
                    BatchInsert.Execute(conn, transaction, "placed_objects",
                        EsmColumns.PlacedObjects,
                        poRows, rowsInserted);

                    // Batch INSERT REFR entries into records table
                    var refrRows = new object?[poCount][];
                    for (int i = 0; i < poCount; i++)
                    {
                        refrRows[i] = allPlaced[i].ToRecordValues();
                    }
                    BatchInsert.Execute(conn, transaction, "records",
                        EsmColumns.Records,
                        refrRows, rowsInserted);

                    // Batch INSERT teleport-door destinations (subset of the REFRs above, so
                    // placed_objects is already inserted to satisfy the source_ref reference).
                    if (allTeleports.Count > 0)
                    {
                        var teleRows = new object?[allTeleports.Count][];
                        for (int i = 0; i < allTeleports.Count; i++)
                        {
                            var t = allTeleports[i];
                            teleRows[i] = new object?[] {
                                t.SourceRef, t.DestinationRef,
                                t.DestPosX, t.DestPosY, t.DestPosZ,
                                t.DestRotX, t.DestRotY, t.DestRotZ
                            };
                        }
                        BatchInsert.Execute(conn, transaction, "placed_teleports",
                            EsmColumns.PlacedTeleports,
                            teleRows, rowsInserted);
                        _teleportsProcessed += allTeleports.Count;
                    }
                }

                // ── Landscape processing (sequential, small data) ──────────────────
                long landscapesStart = EsmTiming.Now();
                var landscapes = Enumerable.Range(0, cellCount)
                    .Select(ci => (CellFormKey: cellFormKeys[ci], Landscape: list[ci].Landscape))
                    .Where(cell => cell.Landscape != null)
                    .ToList();
                InstallProgress.AddTotal(progressPhase, landscapes.Count);
                foreach (var (cellFormKey, landscape) in landscapes)
                {
                    var landProcessor = new LandscapeProcessor();
                    landProcessor.CellFormKey = cellFormKey;
                    await landProcessor.ProcessRecordsAsync(new[] { landscape! }, "LAND", conn, transaction, true, ct: ct);
                    _landscapesProcessed++;
                    InstallProgress.Tick(progressPhase);
                }
                EsmTiming.Add("cells.landscapes", landscapesStart, _landscapesProcessed);

                if (localTx) transaction.Commit();

                if (!silent)
                {
                    sw.Stop();
                    Console.WriteLine($"Done. {cellCount} CELL, {poCount} REFR processed in {sw.Elapsed:mm\\:ss}.");
                    PrintSummary();
                }
            }
            catch (Exception ex)
            {
                if (localTx) TransactionRollback.AfterFailure(transaction);
                Console.WriteLine($"Error processing CELL: {ex.Message}");
                throw;
            }
        }

        private static void ExtractPlaced(IEnumerable<IPlacedGetter> objects, string cellFk, bool isPersistent, Dictionary<string, string> refrSwaps, Dictionary<string, string> waterCurrents, List<PlacedObjectRow> results, List<TeleportRow> teleports)
        {
            foreach (var obj in objects)
            {
                // Deleted refs (record flag 0x20) never spawn in-game. Vanilla
                // Fallout4.esm contains none; DLC/mod plugins can.
                if ((obj.MajorRecordFlagsRaw & 0x20) != 0) continue;

                // No `dynamic` anywhere below: Mutagen binary overlays implement
                // getters the runtime binder cannot bind (and float? -> float
                // dynamic assignment always throws), so dynamic fallbacks
                // silently dropped every ACHR row, every XSCL scale (~300k
                // non-1.0 refs flattened to 1.0) and every XESP enable parent.
                // IPlacedObjectGetter / IPlacedNpcGetter are sibling interfaces;
                // each field needs explicit `is` branches.
                string baseObj = "";
                if (obj is IPlacedObjectGetter bPo && !bPo.Base.IsNull) baseObj = bPo.Base.FormKey.ToString();
                else if (obj is IPlacedNpcGetter bNpc && !bNpc.Base.IsNull) baseObj = bNpc.Base.FormKey.ToString();
                if (string.IsNullOrEmpty(baseObj)) continue;

                float px = 0, py = 0, pz = 0, rx = 0, ry = 0, rz = 0, scale = 1;
                if (obj is IPlacedObjectGetter pPo)
                {
                    px = pPo.Position.X; py = pPo.Position.Y; pz = pPo.Position.Z;
                    rx = pPo.Rotation.X; ry = pPo.Rotation.Y; rz = pPo.Rotation.Z;
                }
                else if (obj is IPlacedNpcGetter pNpc)
                {
                    px = pNpc.Position.X; py = pNpc.Position.Y; pz = pNpc.Position.Z;
                    rx = pNpc.Rotation.X; ry = pNpc.Rotation.Y; rz = pNpc.Rotation.Z;
                }

                if (obj is IPlacedObjectGetter sPo && sPo.Scale.HasValue) scale = sPo.Scale.Value;
                else if (obj is IPlacedNpcGetter sNpc && sNpc.Scale.HasValue) scale = sNpc.Scale.Value;

                int disabled = (obj.MajorRecordFlagsRaw & 0x800) != 0 ? 1 : 0;

                // XESP: the ref's visibility mirrors the parent's enable state
                // (inverted when the opposite-of-parent flag is set). The final
                // verdict lands in initially_hidden via ResolveInitiallyHiddenAsync.
                string? enableParent = null;
                int enableParentOpposite = 0;
                IEnableParentGetter? ep = obj switch
                {
                    IPlacedObjectGetter epPo => epPo.EnableParent,
                    IPlacedNpcGetter epNpc => epNpc.EnableParent,
                    _ => null,
                };
                if (ep != null && !ep.Reference.IsNull)
                {
                    enableParent = ep.Reference.FormKey.ToString();
                    enableParentOpposite = ep.Flags.HasFlag(EnableParent.Flag.SetEnableStateToOppositeOfParent) ? 1 : 0;
                }

                // XPRM subrecord = trigger volume / sound zone / fog volume / occlusion primitive.
                // NOT a visibility signal: ~730 refs of real geometry (rock piles, buildings,
                // trucks) carry occlusion primitives and render normally in-game; the volume is
                // extra data alongside the base model, only the CK draws it as a colored box.
                // Kept as informational data — do not use it to hide refs.
                int hasPrimitive = 0;
                try
                {
                    if (obj is IPlacedObjectGetter po && po.Primitive != null)
                        hasPrimitive = 1;
                }
                catch (Exception ex) when (ReadFailureTally.IsReadFailure(ex))
                {
                    ReadFailureTally.Shared.Record("REFR Primitive", ex, obj.FormKey.ToString());
                }


                string? materialSwap = refrSwaps.TryGetValue(obj.FormKey.ToString(), out var swapKey) ? swapKey : null;

                // XTEL: load-door teleport. Only IPlacedObjectGetter carries it (ACHR/PlacedNpc
                // cannot). The filter is "has a non-null destination door link" — NOT base==DOOR,
                // so modded teleport activators are kept. Resolution of the destination cell is
                // left to the runtime (via destination_ref -> placed_objects.cell_form_key).
                if (obj is IPlacedObjectGetter teleObj
                    && teleObj.TeleportDestination is { } dest
                    && !dest.Door.IsNull)
                {
                    teleports.Add(new TeleportRow(
                        obj.FormKey.ToString(), dest.Door.FormKey.ToString(),
                        dest.Position.X, dest.Position.Y, dest.Position.Z,
                        dest.Rotation.X, dest.Rotation.Y, dest.Rotation.Z));
                }

                results.Add(new PlacedObjectRow(obj, cellFk, isPersistent,
                    baseObj, px, py, pz, rx, ry, rz, scale, disabled, enableParent, enableParentOpposite,
                    hasPrimitive, materialSwap, waterCurrents.GetValueOrDefault(obj.FormKey.ToString())));
            }
        }

        protected override void PrintSummary()
        {
            Console.WriteLine($"  Interior Cells: {_interiorCount}");
            Console.WriteLine($"  Exterior Cells: {_exteriorCount}");
            Console.WriteLine($"  Cells with Grid data: {_withGrid}");
            Console.WriteLine($"  Placed Objects Processed: {_placedObjectsProcessed}");
            Console.WriteLine($"  Teleport Doors Processed: {_teleportsProcessed}");
            Console.WriteLine($"  Landscapes Processed: {_landscapesProcessed}");
        }
    }

    internal static class ObjectBoundsJson
    {
        internal readonly record struct Point(short X, short Y, short Z);
        internal readonly record struct Box(Point Min, Point Max);

        internal static Box From(IObjectBoundsGetter bounds) => new(
            new Point(bounds.First.X, bounds.First.Y, bounds.First.Z),
            new Point(bounds.Second.X, bounds.Second.Y, bounds.Second.Z));
    }

    public class CollisionLayerProcessor : BaseRecordProcessor<ICollisionLayerGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(ICollisionLayerGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"
                INSERT OR REPLACE INTO collision_layers (form_key, editor_id, layer_index, flags, collides_with)
                VALUES (@FormKey, @EditorId, @LayerIndex, @Flags, @CollidesWith)";

            var tally = ReadFailureTally.Shared;
            long? layerIndex = tally.Read("COLL Index", record.EditorID, () => (long?)record.Index, (long?)null);
            long? flags = tally.Read("COLL Flags", record.EditorID, () => (long?)(uint)record.Flags, (long?)null);
            string? collidesWith = tally.Read("COLL CollidesWith", record.EditorID, () => SafeSerialize(CollidesWithKeys(record), "CollidesWith"), (string?)null);

            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                LayerIndex = layerIndex,
                Flags = flags,
                CollidesWith = collidesWith
            }, transaction);
        }

        internal static List<string>? CollidesWithKeys(ICollisionLayerGetter record) =>
            record.CollidesWith?.Select(link => link.IsNull
                ? throw new InvalidDataException("CNAM contains a null collision layer link")
                : link.FormKey.ToString()).ToList();
    }

    public class StaticProcessor : BaseRecordProcessor<IStaticGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IStaticGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"
                INSERT OR REPLACE INTO statics (form_key, editor_id, model_path, material_object, has_distant_lod, bounds, alternate_textures, is_marker)
                VALUES (@FormKey, @EditorId, @ModelPath, @MaterialObject, @HasDistantLOD, @Bounds, @AlternateTextures, @IsMarker)";

            string? bounds = SerializeObjectBounds(record.ObjectBounds);

            string? material = record.Material.IsNull ? null : record.Material.FormKey.ToString();
            int hasDistantLod = (record.DistantLods?.Count > 0) ? 1 : 0;

            // Mutagen 0.50.1 does not expose the MODS (alternate textures) subrecord for FO4
            // STAT records — IModelGetter only has File and raw MODT bytes. Reserved for future
            // implementation that parses MODS manually.
            string? altTexJson = null;

            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                ModelPath = record.Model?.File,
                MaterialObject = material,
                HasDistantLOD = hasDistantLod,
                Bounds = bounds,
                AlternateTextures = altTexJson,
                IsMarker = (record.MajorRecordFlagsRaw & 0x800000) != 0 ? 1 : 0
            }, transaction);
        }
    }

    public class TextureSetProcessor : BaseRecordProcessor<ITextureSetGetter>
    {
        private readonly record struct MaterialTextures(string? Diffuse, string? Normal, string? Specular);

        private readonly string? _materialsRoot; // null disables MNAM resolution
        private readonly ConcurrentDictionary<string, MaterialTextures?> _matCache = new(); // null value = known miss
        private int _resolvedFromMaterial;
        private int _materialMisses;

        public TextureSetProcessor(string? materialsRoot = null)
        {
            _materialsRoot = materialsRoot;
        }

        protected override async Task ProcessSpecificRecordAsync(ITextureSetGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"
                INSERT OR REPLACE INTO texture_sets (form_key, editor_id, diffuse, normal, specular, glow, environment_mask, material, flags)
                VALUES (@FormKey, @EditorId, @Diffuse, @Normal, @Specular, @Glow, @EnvironmentMask, @Material, @Flags)";

            string? diffuse = record.Diffuse;
            string? normal = record.NormalOrGloss;
            string? specular = record.Environment;
            string? materialPath = record.Material;

            // MNAM-only texture sets (common for landscape debris/rubble) carry their
            // texture paths inside a BGSM/BGEM material instead of TX00-TX02.
            if (string.IsNullOrWhiteSpace(diffuse) && !string.IsNullOrWhiteSpace(materialPath) && _materialsRoot != null)
            {
                var mat = ResolveMaterial(materialPath);
                if (mat is { } m)
                {
                    if (string.IsNullOrWhiteSpace(diffuse)) diffuse = m.Diffuse;
                    if (string.IsNullOrWhiteSpace(normal)) normal = m.Normal;
                    if (string.IsNullOrWhiteSpace(specular)) specular = m.Specular;
                    Interlocked.Increment(ref _resolvedFromMaterial);
                }
                else
                {
                    Interlocked.Increment(ref _materialMisses);
                }
            }

            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Diffuse = diffuse,
                Normal = normal,
                Specular = specular,
                Glow = record.Glow,
                EnvironmentMask = (string?)null, // 'EnvironmentMask' property not found on ITextureSetGetter
                Material = materialPath,
                Flags = SafeSerialize(record.Flags)
            }, transaction);
        }

        private MaterialTextures? ResolveMaterial(string mnamPath)
        {
            string rel = mnamPath.Trim().Replace('/', '\\').TrimStart('\\');
            if (rel.StartsWith("materials\\", StringComparison.OrdinalIgnoreCase))
                rel = rel.Substring("materials\\".Length);

            return _matCache.GetOrAdd(rel.ToLowerInvariant(), _ => ParseMaterialFile(rel));
        }

        private MaterialTextures? ParseMaterialFile(string rel)
        {
            if (_materialsRoot == null) return null;
            string path = Path.Combine(_materialsRoot, "Materials", rel);
            if (!File.Exists(path))
            {
                path = Path.Combine(_materialsRoot, rel);
                if (!File.Exists(path)) return null;
            }

            try
            {
                using var stream = new FileStream(path, FileMode.Open, FileAccess.Read);
                using var reader = new BinaryReader(stream);
                uint signature = reader.ReadUInt32();
                stream.Position = 0;

                if (signature == BGSM.Signature)
                {
                    var bgsm = new BGSM();
                    bgsm.Open(stream);
                    return new MaterialTextures(
                        NormalizeTexturePath(bgsm.DiffuseTexture),
                        NormalizeTexturePath(bgsm.NormalTexture),
                        NormalizeTexturePath(bgsm.SmoothSpecTexture));
                }
                if (signature == BGEM.Signature)
                {
                    var bgem = new BGEM();
                    bgem.Open(stream);
                    return new MaterialTextures(
                        NormalizeTexturePath(bgem.BaseTexture),
                        NormalizeTexturePath(bgem.NormalTexture),
                        NormalizeTexturePath(bgem.SpecularTexture));
                }
                return null;
            }
            catch (Exception e) when (BaseMaterialFile.IsReadFailure(e))
            {
                Console.WriteLine($"[WARN] Unreadable material '{path}': {e.Message}");
                return null;
            }
        }

        // Match the TX00-derived row convention: backslashes, relative to the textures folder
        // (the engine prepends "textures/" when loading).
        private static string? NormalizeTexturePath(string? p)
        {
            if (string.IsNullOrWhiteSpace(p)) return null;
            var clean = p.Replace('/', '\\').TrimStart('\\');
            if (clean.StartsWith("textures\\", StringComparison.OrdinalIgnoreCase))
                clean = clean.Substring("textures\\".Length);
            return clean;
        }

        protected override void PrintSummary()
        {
            Console.WriteLine($"  Texture sets resolved via BGSM/BGEM material: {_resolvedFromMaterial}");
            if (_materialMisses > 0)
                Console.WriteLine($"  Texture sets with unresolvable material path: {_materialMisses}");
        }
    }

    public class LandscapeTextureProcessor : BaseRecordProcessor<ILandscapeTextureGetter>
    {
        private int _withFriction;

        protected override async Task ProcessSpecificRecordAsync(ILandscapeTextureGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"
                INSERT OR REPLACE INTO landscape_textures (form_key, editor_id, texture_set, material_type, friction, restitution, havok_data)
                VALUES (@FormKey, @EditorId, @TextureSet, @MaterialType, @Friction, @Restitution, @HavokData)";

            var tally = ReadFailureTally.Shared;
            string? textureSet = tally.Read("LTEX TextureSet", record.EditorID, () => record.TextureSet.IsNull ? null : record.TextureSet.FormKey.ToString(), (string?)null);
            string? materialType = tally.Read("LTEX MaterialType", record.EditorID, () => record.MaterialType.FormKey.ToString(), (string?)null);
            byte friction = tally.Read("LTEX HavokFriction", record.EditorID, () => record.HavokFriction, (byte)0);
            byte restitution = tally.Read("LTEX HavokRestitution", record.EditorID, () => record.HavokRestitution, (byte)0);

            if (friction > 0 || restitution > 0) _withFriction++;

            var grasses = tally.Read("LTEX Grasses", record.EditorID,
                () => record.Grasses.Select(g => g.FormKey.ToString()).ToList(), new List<string>());
            // An override with fewer grasses must not inherit the tail of the list it replaces.
            await conn.ExecuteAsync("DELETE FROM ltex_grasses WHERE ltex_form_key = @Ltex", new { Ltex = formKey }, transaction);
            for (int order = 0; order < grasses.Count; order++)
                await conn.ExecuteAsync(
                    "INSERT OR REPLACE INTO ltex_grasses (ltex_form_key, grass_form_key, ord) VALUES (@Ltex, @Grass, @Ord)",
                    new { Ltex = formKey, Grass = grasses[order], Ord = order }, transaction);

            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                TextureSet = textureSet,
                MaterialType = materialType,
                Friction = friction,
                Restitution = restitution,
                HavokData = SafeSerialize(new { SpecularExponent = record.TextureSpecularExponent })
            }, transaction);
        }

        protected override void PrintSummary()
        {
            Console.WriteLine($"  Textures with Physics Data: {_withFriction}");
        }
    }

    public class LandscapeProcessor : BaseRecordProcessor<ILandscapeGetter>
    {
        public string? CellFormKey { get; set; }

        protected override async Task ProcessSpecificRecordAsync(ILandscapeGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"
                INSERT OR REPLACE INTO landscapes (form_key, editor_id, cell, vertex_height_map, vertex_colors, layers, flags)
                VALUES (@FormKey, @EditorId, @Cell, @VertexHeightMap, @VertexColors, @Layers, @Flags)";

            const int VERTS = 33;

            // VertexHeightMap → BLOB: 4-byte f32 offset + 1089 i8 deltas (33x33, row-major y,x).
            byte[]? heightBlob = null;
            if (record.VertexHeightMap != null)
            {
                heightBlob = new byte[4 + VERTS * VERTS];
                BitConverter.GetBytes(record.VertexHeightMap.Offset).CopyTo(heightBlob, 0);
                if (record.VertexHeightMap.HeightMap != null)
                {
                    foreach (var kv in record.VertexHeightMap.HeightMap)
                    {
                        int idx = 4 + kv.Key.Y * VERTS + kv.Key.X;
                        heightBlob[idx] = unchecked((byte)kv.Value);
                    }
                }
            }

            // VertexColors → BLOB: 3267 bytes (33x33x3 RGB, row-major).
            byte[]? colorsBlob = null;
            if (record.VertexColors != null)
            {
                colorsBlob = new byte[VERTS * VERTS * 3];
                int i = 0;
                foreach (var kv in record.VertexColors)
                {
                    var c = kv.Value;
                    colorsBlob[i++] = c.X;
                    colorsBlob[i++] = c.Y;
                    colorsBlob[i++] = c.Z;
                }
            }

            // Layers → BLOB: u16 count, then per layer:
            //   NUL-terminated UTF-8 texture key, u8 quadrant, u8 flags (bit0=is_base),
            //   u16 alpha_data_length, raw alpha bytes.
            byte[]? layersBlob = null;
            if (record.Layers != null && record.Layers.Count > 0)
            {
                using var ms = new MemoryStream();
                using var bw = new BinaryWriter(ms);
                bw.Write((ushort)record.Layers.Count);
                foreach (var layer in record.Layers)
                {
                    var hdr = layer.Header
                        ?? throw new InvalidDataException($"LAND {formKey}: texture layer without a header.");
                    string texKey = hdr.Texture.IsNull ? "" : hdr.Texture.FormKey.ToString();
                    bw.Write(Encoding.UTF8.GetBytes(texKey));
                    bw.Write((byte)0); // NUL terminator
                    bw.Write((byte)hdr.Quadrant);
                    bool isBase = hdr.LayerNumber == ushort.MaxValue;
                    bw.Write((byte)(isBase ? 1 : 0));
                    if (layer is IAlphaLayerGetter alphaLayer && alphaLayer.AlphaLayerData.HasValue)
                    {
                        byte[] alphaBytes = alphaLayer.AlphaLayerData.Value.Span.ToArray();
                        bw.Write((ushort)alphaBytes.Length);
                        bw.Write(alphaBytes);
                    }
                    else
                    {
                        bw.Write((ushort)0);
                    }
                }
                bw.Flush();
                layersBlob = ms.ToArray();
            }

            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Cell = CellFormKey,
                VertexHeightMap = (object?)heightBlob ?? DBNull.Value,
                VertexColors = (object?)colorsBlob ?? DBNull.Value,
                Layers = (object?)layersBlob ?? DBNull.Value,
                Flags = "{}",
            }, transaction);
        }
    }

    public class ActivatorProcessor : BaseRecordProcessor<IActivatorGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IActivatorGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO activators (form_key, editor_id, name, model_path, activation_sound, flags, is_marker, water_type)
                VALUES (@FormKey, @EditorId, @Name, @ModelPath, @ActivationSound, @Flags, @IsMarker, @WaterType)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                ModelPath = record.Model?.File,
                ActivationSound = record.ActivationSound.IsNull ? (string?)null : record.ActivationSound.FormKey.ToString(),
                Flags = SafeSerialize(record.Flags),
                IsMarker = (record.MajorRecordFlagsRaw & 0x800000) != 0 ? 1 : 0,
                WaterType = record.WaterType.IsNull ? (string?)null : record.WaterType.FormKey.ToString()
            }, transaction);
        }
    }

    public class ContainerProcessor : BaseRecordProcessor<IContainerGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IContainerGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO containers (form_key, editor_id, name, model_path, open_sound, close_sound, flags, items, is_marker)
                VALUES (@FormKey, @EditorId, @Name, @ModelPath, @OpenSound, @CloseSound, @Flags, @Items, @IsMarker)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                ModelPath = record.Model?.File,
                OpenSound = record.OpenSound.IsNull ? (string?)null : record.OpenSound.FormKey.ToString(),
                CloseSound = record.CloseSound.IsNull ? (string?)null : record.CloseSound.FormKey.ToString(),
                Flags = SafeSerialize(record.Flags),
                Items = SafeSerialize(record.Items?.Select(i => new { item = i.Item.Item.FormKey.ToString(), count = i.Item.Count }).ToList()),
                IsMarker = (record.MajorRecordFlagsRaw & 0x800000) != 0 ? 1 : 0
            }, transaction);
        }
    }

    public class DoorProcessor : BaseRecordProcessor<IDoorGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IDoorGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO doors (form_key, editor_id, name, model_path, open_sound, close_sound, flags, is_marker, bounds)
                VALUES (@FormKey, @EditorId, @Name, @ModelPath, @OpenSound, @CloseSound, @Flags, @IsMarker, @Bounds)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                ModelPath = record.Model?.File,
                OpenSound = record.OpenSound.IsNull ? (string?)null : record.OpenSound.FormKey.ToString(),
                CloseSound = record.CloseSound.IsNull ? (string?)null : record.CloseSound.FormKey.ToString(),
                Flags = SafeSerialize(record.Flags),
                IsMarker = (record.MajorRecordFlagsRaw & 0x800000) != 0 ? 1 : 0,
                Bounds = SerializeObjectBounds(record.ObjectBounds)
            }, transaction);
        }
    }

    public class FurnitureProcessor : BaseRecordProcessor<IFurnitureGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IFurnitureGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO furniture (form_key, editor_id, name, model_path, furniture_flags, markers, is_marker)
                VALUES (@FormKey, @EditorId, @Name, @ModelPath, @FurnitureFlags, @Markers, @IsMarker)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                ModelPath = record.Model?.File,
                FurnitureFlags = SafeSerialize(record.Flags),
                Markers = SafeSerialize(record.MarkerParameters?.Select(m => new
                {
                    Offset = new { m.Offset.X, m.Offset.Y, m.Offset.Z },
                    m.RotationZ,
                    Keyword = m.Keyword.IsNull ? null : m.Keyword.FormKey.ToString()
                }).ToList()),
                IsMarker = (record.MajorRecordFlagsRaw & 0x800000) != 0 ? 1 : 0
            }, transaction);
        }
    }

    public class LightProcessor : BaseRecordProcessor<ILightGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(ILightGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO lights (form_key, editor_id, model_path, radius, color, flags, fade_value, falloff_exponent, is_marker,
                    constant, scalar, exponent, fov, near_clip, flicker_period, flicker_intensity_amplitude, flicker_movement_amplitude,
                    gobo, lens, god_rays, god_rays_near_clip)
                VALUES (@FormKey, @EditorId, @ModelPath, @Radius, @Color, @Flags, @FadeValue, @FalloffExponent, @IsMarker,
                    @Constant, @Scalar, @Exponent, @Fov, @NearClip, @FlickerPeriod, @FlickerIntensityAmplitude, @FlickerMovementAmplitude,
                    @Gobo, @Lens, @GodRays, @GodRaysNearClip)";
            int packedColor = (record.Color.R << 16) | (record.Color.G << 8) | record.Color.B;
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                ModelPath = record.Model?.File,
                Radius = (double)record.Radius,
                Color = packedColor,
                Flags = (int)record.Flags,
                FadeValue = record.FadeValue,
                FalloffExponent = record.FalloffExponent,
                IsMarker = (record.MajorRecordFlagsRaw & 0x800000) != 0 ? 1 : 0,
                Constant = record.Constant,
                Scalar = record.Scalar,
                Exponent = record.Exponent,
                Fov = record.FOV,
                NearClip = record.NearClip,
                FlickerPeriod = record.FlickerPeriod,
                FlickerIntensityAmplitude = record.FlickerIntensityAmplitude,
                FlickerMovementAmplitude = record.FlickerMovementAmplitude,
                Gobo = record.Gobo,
                Lens = record.Lens.IsNull ? null : record.Lens.FormKey.ToString(),
                GodRays = record.GodRays.IsNull ? null : record.GodRays.FormKey.ToString(),
                GodRaysNearClip = record.GodRaysNearClip
            }, transaction);
        }
    }

    public class MovableStaticProcessor : BaseRecordProcessor<IMovableStaticGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IMovableStaticGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO movable_statics (form_key, editor_id, model_path, destruction_data, is_marker)
                VALUES (@FormKey, @EditorId, @ModelPath, @DestructionData, @IsMarker)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                ModelPath = record.Model?.File,
                DestructionData = SafeSerialize(record.Destructible),
                IsMarker = (record.MajorRecordFlagsRaw & 0x800000) != 0 ? 1 : 0
            }, transaction);
        }
    }

    public class TalkingActivatorProcessor : BaseRecordProcessor<ITalkingActivatorGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(ITalkingActivatorGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO talking_activators (form_key, editor_id, name, model_path, voice_type, object)
                VALUES (@FormKey, @EditorId, @Name, @ModelPath, @VoiceType, @Object)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                ModelPath = record.Model?.File,
                VoiceType = record.VoiceType.IsNull ? (string?)null : record.VoiceType.FormKey.ToString(),
                Object = record.LoopingSound.IsNull ? (string?)null : record.LoopingSound.FormKey.ToString()
            }, transaction);
        }
    }

    public class StaticCollectionProcessor : BaseRecordProcessor<IStaticCollectionGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IStaticCollectionGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO static_collections (form_key, editor_id, model_path, is_marker, parts)
                VALUES (@FormKey, @EditorId, @ModelPath, @IsMarker, @Parts)";
            var parts = record.Parts?.Select(p => new
            {
                static_ref = p.Static.FormKey.ToString(),
            }).ToList();
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                ModelPath = record.Model?.File,
                IsMarker = (record.MajorRecordFlagsRaw & 0x800000) != 0 ? 1 : 0,
                Parts = SafeSerialize(parts)
            }, transaction);
        }
    }

    public class FactionProcessor : BaseRecordProcessor<IFactionGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IFactionGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO factions (form_key, editor_id, name, flags, ranks)
                VALUES (@FormKey, @EditorId, @Name, @Flags, @Ranks)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Flags = SafeSerialize(record.Flags),
                Ranks = SafeSerialize(record.Ranks?.Select(r => new
                {
                    number = r.Number,
                    male = r.Title?.Male?.String,
                    female = r.Title?.Female?.String
                }).ToList())
            }, transaction);
        }
    }

    public class EncounterZoneProcessor : BaseRecordProcessor<IEncounterZoneGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IEncounterZoneGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO encounter_zones (form_key, editor_id, owner_faction, min_level, max_level, flags)
                VALUES (@FormKey, @EditorId, @OwnerFaction, @MinLevel, @MaxLevel, @Flags)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                OwnerFaction = record.Owner.IsNull ? (string?)null : record.Owner.FormKey.ToString(),
                MinLevel = (int)record.MinLevel,
                MaxLevel = (int)record.MaxLevel,
                Flags = SafeSerialize(record.Flags)
            }, transaction);
        }
    }

    public class LocationProcessor : BaseRecordProcessor<ILocationGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(ILocationGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO locations (form_key, editor_id, name, parent_location, faction, keywords, flags)
                VALUES (@FormKey, @EditorId, @Name, @ParentLocation, @Faction, @Keywords, @Flags)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                ParentLocation = record.ParentLocation.IsNull ? (string?)null : record.ParentLocation.FormKey.ToString(),
                Faction = (string?)null,
                Keywords = SafeSerialize(FormKeyStrings.Of(record.Keywords)),
                Flags = (string?)null
            }, transaction);
        }
    }

    public class FormListProcessor : BaseRecordProcessor<IFormListGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IFormListGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO form_lists (form_key, editor_id, entries)
                VALUES (@FormKey, @EditorId, @Entries)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Entries = SafeSerialize(record.Items?.Select(i => i.FormKey.ToString()).ToList())
            }, transaction);
        }
    }

    public class ObjectModificationProcessor : BaseRecordProcessor<IAObjectModificationGetter>
    {
        private readonly Dictionary<string, int> _untargeted = new();

        protected override async Task ProcessSpecificRecordAsync(IAObjectModificationGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO object_modifications (form_key, editor_id, name, mod_type, target_type, attach_point, includes, properties, model_path, attach_parent_slots)
                VALUES (@FormKey, @EditorId, @Name, @ModType, @TargetType, @AttachPoint, @Includes, @Properties, @ModelPath, @AttachParentSlots)";
            string? targetType = WeaponModJson.TargetType(record);
            if (targetType == null)
                _untargeted[record.GetType().Name] = _untargeted.GetValueOrDefault(record.GetType().Name) + 1;
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                ModType = record.GetType().Name,
                TargetType = targetType,
                AttachPoint = record.AttachPoint.IsNull ? (string?)null : record.AttachPoint.FormKey.ToString(),
                Includes = WeaponModJson.ModIncludes(record.Includes),
                Properties = WeaponModJson.ModProperties(record),
                ModelPath = record.Model?.File,
                AttachParentSlots = SafeSerialize(FormKeyStrings.Of(record.AttachParentSlots))
            }, transaction);
        }

        protected override void PrintSummary()
        {
            foreach (string line in UntargetedSummaryLines())
                Console.WriteLine(line);
        }

        // Fallout4.esm holds 393 of these (the `modcol_*` collection mods and the `Bot_*` decals): the Mutagen generic
        // overlay type, with no weapon, armor or NPC target and no property list. Expected data, not a conversion failure.
        internal IEnumerable<string> UntargetedSummaryLines() =>
            _untargeted.Select(entry => $"  WARNING: {entry.Value} {entry.Key} object modification(s) have no weapon, armor or NPC target; target_type stored as NULL.");
    }

    public class QuestProcessor : BaseRecordProcessor<IQuestGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IQuestGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO quests (form_key, editor_id, name, priority, quest_type, flags, conditions)
                VALUES (@FormKey, @EditorId, @Name, @Priority, @QuestType, @Flags, @Conditions)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Priority = 0,
                QuestType = record.Type.ToString(),
                Flags = (string?)null,
                Conditions = (string?)null
            }, transaction);
        }
    }

    public class MagicEffectProcessor : BaseRecordProcessor<IMagicEffectGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IMagicEffectGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO magic_effects (form_key, editor_id, name, description, base_cost, archetype, resist_value, flags, properties)
                VALUES (@FormKey, @EditorId, @Name, @Description, @BaseCost, @Archetype, @ResistValue, @Flags, @Properties)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Description = record.Description?.ToString(),
                BaseCost = record.BaseCost,
                Archetype = record.Archetype.ToString(),
                ResistValue = (string?)null,
                Flags = SafeSerialize(record.Flags),
                Properties = (string?)null
            }, transaction);
        }
    }

    public class SpellProcessor : BaseRecordProcessor<ISpellGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(ISpellGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO spells (form_key, editor_id, name, spell_type, cast_type, delivery, charge_time, cast_duration, range, flags)
                VALUES (@FormKey, @EditorId, @Name, @SpellType, @CastType, @Delivery, @ChargeTime, @CastDuration, @Range, @Flags)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                SpellType = record.Type.ToString(),
                CastType = record.CastType.ToString(),
                Delivery = record.TargetType.ToString(),
                ChargeTime = record.ChargeTime,
                CastDuration = record.CastDuration,
                Range = record.Range,
                Flags = SafeSerialize(record.Flags)
            }, transaction);
        }
    }

    public class PerkProcessor : BaseRecordProcessor<IPerkGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IPerkGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO perks (form_key, editor_id, name, description, perk_type, next_perk, flags, conditions, effects,
                    level, num_ranks, playable, hidden, swf_path)
                VALUES (@FormKey, @EditorId, @Name, @Description, @PerkType, @NextPerk, @Flags, @Conditions, @Effects,
                    @Level, @NumRanks, @Playable, @Hidden, @SwfPath)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Description = record.Description?.ToString(),
                PerkType = (string?)null,
                NextPerk = record.NextPerk.IsNull ? (string?)null : record.NextPerk.FormKey.ToString(),
                Flags = (string?)null,
                Conditions = SafeSerialize(record.Conditions?.ToList()),
                Effects = SafeSerialize(record.Effects?.ToList()),
                Level = (int)record.Level,
                NumRanks = (int)record.NumRanks,
                Playable = record.Playable ? 1 : 0,
                Hidden = record.Hidden ? 1 : 0,
                SwfPath = record.Swf
            }, transaction);
        }
    }

    public class WeatherProcessor : BaseRecordProcessor<IWeatherGetter>
    {
        private static readonly string[] TimesOfDay =
            { "Sunrise", "Day", "Sunset", "Night", "EarlySunrise", "LateSunrise", "EarlySunset", "LateSunset" };

        internal static object Rgb(System.Drawing.Color c) => new { c.R, c.G, c.B };

        private static Dictionary<string, T> ByTimeOfDay<T>(Func<string, T> select) =>
            TimesOfDay.ToDictionary(t => t, select);

        private static System.Drawing.Color Slot(IWeatherColorGetter group, string time) => time switch
        {
            "Sunrise" => group.Sunrise,
            "Day" => group.Day,
            "Sunset" => group.Sunset,
            "Night" => group.Night,
            "EarlySunrise" => group.EarlySunrise,
            "LateSunrise" => group.LateSunrise,
            "EarlySunset" => group.EarlySunset,
            "LateSunset" => group.LateSunset,
            _ => throw new ArgumentOutOfRangeException(nameof(time), time, "Unknown time of day")
        };

        private static IAmbientColorsGetter Slot(IWeatherAmbientColorSetGetter set, string time) => time switch
        {
            "Sunrise" => set.Sunrise,
            "Day" => set.Day,
            "Sunset" => set.Sunset,
            "Night" => set.Night,
            "EarlySunrise" => set.EarlySunrise,
            "LateSunrise" => set.LateSunrise,
            "EarlySunset" => set.EarlySunset,
            "LateSunset" => set.LateSunset,
            _ => throw new ArgumentOutOfRangeException(nameof(time), time, "Unknown time of day")
        };

        private static IFormLinkGetter<IImageSpaceGetter> Slot(IWeatherGetter w, string time) => time switch
        {
            "Sunrise" => w.ImageSpaceSunrise,
            "Day" => w.ImageSpaceDay,
            "Sunset" => w.ImageSpaceSunset,
            "Night" => w.ImageSpaceNight,
            "EarlySunrise" => w.ImageSpaceEarlySunrise,
            "LateSunrise" => w.ImageSpaceLateSunrise,
            "EarlySunset" => w.ImageSpaceEarlySunset,
            "LateSunset" => w.ImageSpaceLateSunset,
            _ => throw new ArgumentOutOfRangeException(nameof(time), time, "Unknown time of day")
        };

        private static Dictionary<string, Dictionary<string, object>> ColorGroups(IWeatherGetter w)
        {
            var groups = new (string Name, IWeatherColorGetter Group)[]
            {
                ("SkyUpper", w.SkyUpperColor), ("FogNear", w.FogNearColor), ("Unknown", w.UnknownColor),
                ("Ambient", w.AmbientColor), ("Sunlight", w.SunlightColor), ("Sun", w.SunColor),
                ("Stars", w.StarsColor), ("SkyLower", w.SkyLowerColor), ("Horizon", w.HorizonColor),
                ("EffectLighting", w.EffectLightingColor), ("CloudLodDiffuse", w.CloudLodDiffuseColor),
                ("CloudLodAmbient", w.CloudLodAmbientColor), ("FogFar", w.FogFarColor),
                ("SkyStatics", w.SkyStaticsColor), ("WaterMultiplier", w.WaterMultiplierColor),
                ("SunGlare", w.SunGlareColor), ("MoonGlare", w.MoonGlareColor),
                ("FogNearHigh", w.FogNearHigh), ("FogFarHigh", w.FogFarHigh)
            };
            return groups.ToDictionary(g => g.Name, g => ByTimeOfDay(t => Rgb(Slot(g.Group, t))));
        }

        private static object DirectionalAmbient(IAmbientColorsGetter a) => new
        {
            XPlus = Rgb(a.DirectionalXPlus),
            XMinus = Rgb(a.DirectionalXMinus),
            YPlus = Rgb(a.DirectionalYPlus),
            YMinus = Rgb(a.DirectionalYMinus),
            ZPlus = Rgb(a.DirectionalZPlus),
            ZMinus = Rgb(a.DirectionalZMinus),
            Specular = Rgb(a.Specular),
            a.Scale
        };

        private static object Fog(IWeatherGetter w) => new
        {
            Day = new
            {
                Near = w.FogDistanceDayNear,
                Far = w.FogDistanceDayFar,
                Power = w.FogDistanceDayPower,
                Max = w.FogDistanceDayMax,
                NearHeightMid = w.FogDistanceDayNearHeightMid,
                NearHeightRange = w.FogDistanceDayNearHeightRange,
                FarHeightMid = w.FogDistanceDayFarHeightMid,
                FarHeightRange = w.FogDistanceDayFarHeightRange,
                HighDensityScale = w.FogDistanceDayHighDensityScale
            },
            Night = new
            {
                Near = w.FogDistanceNightNear,
                Far = w.FogDistanceNightFar,
                Power = w.FogDistanceNightPower,
                Max = w.FogDistanceNightMax,
                NearHeightMid = w.FogDistanceNightNearHeightMid,
                NearHeightRange = w.FogDistanceNightNearHeightRange,
                FarHeightMid = w.FogDistanceNightFarHeightMid,
                FarHeightRange = w.FogDistanceNightFarHeightRange,
                HighDensityScale = w.FogDistanceNightHighDensityScale
            }
        };

        protected override async Task ProcessSpecificRecordAsync(IWeatherGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO weather (form_key, editor_id, precipitation, fog, colors, image_spaces, sounds, god_rays, directional_ambient)
                VALUES (@FormKey, @EditorId, @Precipitation, @Fog, @Colors, @ImageSpaces, @Sounds, @GodRays, @DirectionalAmbient)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Precipitation = record.Precipitation.IsNull ? (string?)null : record.Precipitation.FormKey.ToString(),
                Fog = SafeSerialize(Fog(record)),
                Colors = SafeSerialize(ColorGroups(record)),
                ImageSpaces = SafeSerialize(ByTimeOfDay(t => Slot(record, t) is { IsNull: false } link ? link.FormKey.ToString() : null)),
                Sounds = SafeSerialize(record.Sounds?.ToList()),
                GodRays = (string?)null,
                DirectionalAmbient = record.DirectionalAmbientLightingColors is { } ambientColors
                    ? SafeSerialize(ByTimeOfDay(t => DirectionalAmbient(Slot(ambientColors, t))))
                    : null
            }, transaction);
        }
    }

    public class MaterialObjectProcessor : BaseRecordProcessor<IMaterialObjectGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IMaterialObjectGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO material_objects (form_key, editor_id, model_path, material_file, properties)
                VALUES (@FormKey, @EditorId, @ModelPath, @MaterialFile, @Properties)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                ModelPath = record.Model?.File?.ToString(),
                MaterialFile = (string?)null,
                Properties = (string?)null
            }, transaction);
        }
    }

    public class LeveledItemProcessor : BaseRecordProcessor<ILeveledItemGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(ILeveledItemGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO leveled_items (form_key, editor_id, chance_none, flags, entries)
                VALUES (@FormKey, @EditorId, @ChanceNone, @Flags, @Entries)";
            var entries = record.Entries?.Select(e => new
            {
                @ref = e.Data?.Reference.FormKey.ToString(),
                level = (int)(e.Data?.Level ?? 0),
                count = (int)(e.Data?.Count ?? 1)
            }).ToList();
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                ChanceNone = 0,
                Flags = SafeSerialize(record.Flags),
                Entries = SafeSerialize(entries)
            }, transaction);
        }
    }

    public class LeveledNpcProcessor : BaseRecordProcessor<ILeveledNpcGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(ILeveledNpcGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO leveled_npcs (form_key, editor_id, chance_none, flags, entries)
                VALUES (@FormKey, @EditorId, @ChanceNone, @Flags, @Entries)";
            var entries = record.Entries?.Select(e => new
            {
                @ref = e.Data?.Reference.FormKey.ToString(),
                level = (int)(e.Data?.Level ?? 0),
                count = (int)(e.Data?.Count ?? 1)
            }).ToList();
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                ChanceNone = 0,
                Flags = SafeSerialize(record.Flags),
                Entries = SafeSerialize(entries)
            }, transaction);
        }
    }

    public class ConstructibleObjectProcessor : BaseRecordProcessor<IConstructibleObjectGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IConstructibleObjectGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO constructible_objects (form_key, editor_id, created_object, created_count, workbench_keyword, components)
                VALUES (@FormKey, @EditorId, @CreatedObject, @CreatedCount, @WorkbenchKeyword, @Components)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                CreatedObject = record.CreatedObject.IsNull ? (string?)null : record.CreatedObject.FormKey.ToString(),
                CreatedCount = 1,
                WorkbenchKeyword = record.WorkbenchKeyword.IsNull ? (string?)null : record.WorkbenchKeyword.FormKey.ToString(),
                Components = (string?)null
            }, transaction);
        }
    }

    public class ComponentProcessor : BaseRecordProcessor<IComponentGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IComponentGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO components (form_key, editor_id, name, scrap_item, scrap_scalar)
                Values (@FormKey, @EditorId, @Name, @ScrapItem, @ScrapScalar)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                ScrapItem = record.ScrapItem.IsNull ? (string?)null : record.ScrapItem.FormKey.ToString(),
                ScrapScalar = (double)0
            }, transaction);
        }
    }

    public class FloraProcessor : BaseRecordProcessor<IFloraGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IFloraGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO flora (form_key, editor_id, name, model_path, harvest_sound, ingredient, flags, is_marker)
                Values (@FormKey, @EditorId, @Name, @ModelPath, @HarvestSound, @Ingredient, @Flags, @IsMarker)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                ModelPath = record.Model?.File?.ToString(),
                HarvestSound = record.HarvestSound.IsNull ? (string?)null : record.HarvestSound.FormKey.ToString(),
                Ingredient = record.Ingredient.IsNull ? (string?)null : record.Ingredient.FormKey.ToString(),
                Flags = (string?)null,
                IsMarker = (record.MajorRecordFlagsRaw & 0x800000) != 0 ? 1 : 0
            }, transaction);
        }
    }

    public class TreeProcessor : BaseRecordProcessor<ITreeGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(ITreeGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO trees (form_key, editor_id, name, model_path, ingredient, is_marker)
                Values (@FormKey, @EditorId, @Name, @ModelPath, @Ingredient, @IsMarker)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                ModelPath = record.Model?.File?.ToString(),
                Ingredient = (string?)null,
                IsMarker = (record.MajorRecordFlagsRaw & 0x800000) != 0 ? 1 : 0
            }, transaction);
        }
    }

    public class GrassProcessor : BaseRecordProcessor<IGrassGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IGrassGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO grasses
                    (form_key, editor_id, model_path, flags, density, min_slope, max_slope,
                     units_from_water, units_from_water_type, position_range, height_range, color_range, wave_period)
                Values (@FormKey, @EditorId, @ModelPath, @Flags, @Density, @MinSlope, @MaxSlope,
                     @UnitsFromWater, @UnitsFromWaterType, @PositionRange, @HeightRange, @ColorRange, @WavePeriod)";

            var tally = ReadFailureTally.Shared;
            string? id = record.EditorID;
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = id,
                ModelPath = record.Model?.File?.ToString(),
                Flags = tally.Read("GRAS Flags", id, () => (int)record.Flags, 0),
                Density = tally.Read("GRAS Density", id, () => (int)record.Density, 0),
                MinSlope = tally.Read("GRAS MinSlope", id, () => (int)record.MinSlope, 0),
                MaxSlope = tally.Read("GRAS MaxSlope", id, () => (int)record.MaxSlope, 0),
                UnitsFromWater = tally.Read("GRAS UnitsFromWater", id, () => (int)record.UnitsFromWater, 0),
                UnitsFromWaterType = tally.Read("GRAS UnitsFromWaterType", id, () => (long)(uint)record.UnitsFromWaterType, 0L),
                PositionRange = tally.Read("GRAS PositionRange", id, () => (double)record.PositionRange, 0.0),
                HeightRange = tally.Read("GRAS HeightRange", id, () => (double)record.HeightRange, 0.0),
                ColorRange = tally.Read("GRAS ColorRange", id, () => (double)record.ColorRange, 0.0),
                WavePeriod = tally.Read("GRAS WavePeriod", id, () => (double)record.WavePeriod, 0.0)
            }, transaction);
        }
    }

    public class ClassProcessor : BaseRecordProcessor<IClassGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IClassGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO classes (form_key, editor_id, name, description, flags)
                Values (@FormKey, @EditorId, @Name, @Description, @Flags)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Description = record.Description?.ToString(),
                Flags = (string?)null
            }, transaction);
        }
    }

    public class CombatStyleProcessor : BaseRecordProcessor<ICombatStyleGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(ICombatStyleGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO combat_styles (form_key, editor_id, flags, data)
                Values (@FormKey, @EditorId, @Flags, @Data)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Flags = SafeSerialize(record.Flags),
                Data = (string?)null
            }, transaction);
        }
    }

    public class EquipTypeProcessor : BaseRecordProcessor<IEquipTypeGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IEquipTypeGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO equip_types (form_key, editor_id, flags)
                Values (@FormKey, @EditorId, @Flags)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Flags = (string?)null
            }, transaction);
        }
    }

    public class VoiceTypeProcessor : BaseRecordProcessor<IVoiceTypeGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IVoiceTypeGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO voice_types (form_key, editor_id, flags)
                Values (@FormKey, @EditorId, @Flags)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Flags = (string?)null
            }, transaction);
        }
    }

    public class ExplosionProcessor : BaseRecordProcessor<IExplosionGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IExplosionGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO explosions (form_key, editor_id, name, model_path, light, sound1, sound2, inner_radius, outer_radius, damage, flags)
                Values (@FormKey, @EditorId, @Name, @ModelPath, @Light, @Sound1, @Sound2, @InnerRadius, @OuterRadius, @Damage, @Flags)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                ModelPath = record.Model?.File?.ToString(),
                Light = record.Light.IsNull ? (string?)null : record.Light.FormKey.ToString(),
                Sound1 = record.Sound1.IsNull ? (string?)null : record.Sound1.FormKey.ToString(),
                Sound2 = record.Sound2.IsNull ? (string?)null : record.Sound2.FormKey.ToString(),
                InnerRadius = record.InnerRadius,
                OuterRadius = record.OuterRadius,
                Damage = record.Damage,
                Flags = SafeSerialize(record.Flags)
            }, transaction);
        }
    }

    public class ProjectileProcessor : BaseRecordProcessor<IProjectileGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IProjectileGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO projectiles (form_key, editor_id, name, model_path, flags, gravity, speed, range, explosion, light, muzzle_flash_light)
                Values (@FormKey, @EditorId, @Name, @ModelPath, @Flags, @Gravity, @Speed, @Range, @Explosion, @Light, @MuzzleFlashLight)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                ModelPath = record.Model?.File?.ToString(),
                Flags = SafeSerialize(record.Flags),
                Gravity = record.Gravity,
                Speed = record.Speed,
                Range = record.Range,
                Explosion = record.Explosion.IsNull ? (string?)null : record.Explosion.FormKey.ToString(),
                Light = record.Light.IsNull ? (string?)null : record.Light.FormKey.ToString(),
                MuzzleFlashLight = record.MuzzleFlash.IsNull ? (string?)null : record.MuzzleFlash.FormKey.ToString()
            }, transaction);
        }
    }

    public class HazardProcessor : BaseRecordProcessor<IHazardGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IHazardGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO hazards (form_key, editor_id, name, model_path, radius, duration, target_limit, spell, light, flags)
                Values (@FormKey, @EditorId, @Name, @ModelPath, @Radius, @Duration, @Limit, @Spell, @Light, @Flags)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                ModelPath = record.Model?.File?.ToString(),
                Radius = record.Radius,
                Duration = (double)0,
                Limit = (int)record.Limit,
                Spell = (string?)null,
                Light = record.Light.IsNull ? (string?)null : record.Light.FormKey.ToString(),
                Flags = SafeSerialize(record.Flags)
            }, transaction);
        }
    }

    public class IngredientProcessor : BaseRecordProcessor<IIngredientGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IIngredientGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO items (form_key, item_type, editor_id, name, value, weight, model_path, pickup_sound, drop_sound, keywords)
                VALUES (@FormKey, 'INGR', @EditorId, @Name, @Value, @Weight, @ModelPath, @PickupSound, @DropSound, @Keywords)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Value = record.Value,
                Weight = record.Weight,
                ModelPath = record.Model?.File,
                PickupSound = record.PickUpSound.IsNull ? (string?)null : record.PickUpSound.FormKey.ToString(),
                DropSound = record.PutDownSound.IsNull ? (string?)null : record.PutDownSound.FormKey.ToString(),
                Keywords = SafeSerialize(FormKeyStrings.Of(record.Keywords))
            }, transaction);
        }
    }

    public class KeyProcessor : BaseRecordProcessor<IKeyGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IKeyGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO items (form_key, item_type, editor_id, name, value, weight, model_path, pickup_sound, drop_sound, keywords)
                VALUES (@FormKey, 'KEYM', @EditorId, @Name, @Value, @Weight, @ModelPath, @PickupSound, @DropSound, @Keywords)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Value = record.Value,
                Weight = record.Weight,
                ModelPath = record.Model?.File,
                PickupSound = record.PickUpSound.IsNull ? (string?)null : record.PickUpSound.FormKey.ToString(),
                DropSound = record.PutDownSound.IsNull ? (string?)null : record.PutDownSound.FormKey.ToString(),
                Keywords = SafeSerialize(FormKeyStrings.Of(record.Keywords))
            }, transaction);
        }
    }

    public class HolotapeProcessor : BaseRecordProcessor<IHolotapeGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IHolotapeGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO items (form_key, item_type, editor_id, name, value, weight, model_path, pickup_sound, drop_sound, keywords)
                VALUES (@FormKey, 'HOLT', @EditorId, @Name, @Value, @Weight, @ModelPath, @PickupSound, @DropSound, @Keywords)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Value = record.Value,
                Weight = record.Weight,
                ModelPath = record.Model?.File,
                PickupSound = record.PickUpSound.IsNull ? (string?)null : record.PickUpSound.FormKey.ToString(),
                DropSound = record.PutDownSound.IsNull ? (string?)null : record.PutDownSound.FormKey.ToString(),
                Keywords = (string?)null
            }, transaction);
        }
    }

    public class OutfitProcessor : BaseRecordProcessor<IOutfitGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IOutfitGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO outfits (form_key, editor_id, items)
                VALUES (@FormKey, @EditorId, @Items)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Items = SafeSerialize(record.Items?.Select(i => i.FormKey.ToString()).ToList())
            }, transaction);
        }
    }

    public class RelationshipProcessor : BaseRecordProcessor<IRelationshipGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IRelationshipGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO relationships (form_key, editor_id, parent, child, association_type, flags)
                VALUES (@FormKey, @EditorId, @Parent, @Child, @AssociationType, @Flags)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Parent = record.Parent.IsNull ? (string?)null : record.Parent.FormKey.ToString(),
                Child = record.Child.IsNull ? (string?)null : record.Child.FormKey.ToString(),
                AssociationType = record.AssociationType.IsNull ? (string?)null : record.AssociationType.FormKey.ToString(),
                Flags = SafeSerialize(record.Flags)
            }, transaction);
        }
    }

    public class ObjectEffectProcessor : BaseRecordProcessor<IObjectEffectGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IObjectEffectGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO object_effects (form_key, editor_id, name, flags, effects)
                VALUES (@FormKey, @EditorId, @Name, @Flags, @Effects)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Flags = SafeSerialize(record.Flags),
                Effects = SafeSerialize(record.Effects?.Select(e => new
                {
                    effect = e.BaseEffect.FormKey.ToString(),
                    magnitude = e.Data?.Magnitude,
                    area = e.Data?.Area,
                    duration = e.Data?.Duration
                }).ToList())
            }, transaction);
        }
    }

    public class LightingTemplateProcessor : BaseRecordProcessor<ILightingTemplateGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(ILightingTemplateGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            await conn.ExecuteAsync(@"INSERT OR REPLACE INTO lighting_templates (form_key, editor_id, data) VALUES (@FormKey, @EditorId, @Data)",
                new { FormKey = formKey, EditorId = record.EditorID, Data = SafeSerialize(LightingJson.Project(record)) }, transaction);
        }
    }

    public class ImageSpaceProcessor : BaseRecordProcessor<IImageSpaceGetter>
    {
        private readonly IReadOnlyDictionary<string, ImageSpaceDofBlock> _depthOfField;

        internal ImageSpaceProcessor(IReadOnlyDictionary<string, ImageSpaceDofBlock> depthOfField) => _depthOfField = depthOfField;

        protected override async Task ProcessSpecificRecordAsync(IImageSpaceGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            var depthOfField = _depthOfField.GetValueOrDefault(formKey) ?? ImageSpaceDofBlock.None;
            var data = new
            {
                Hdr = new
                {
                    EyeAdaptSpeed = record.HdrEyeAdaptSpeed,
                    TonemapE = record.HdrTonemapE,
                    BloomThreshold = record.HdrBloomThreshold,
                    BloomScale = record.HdrBloomScale,
                    AutoExposureMax = record.HdrAutoExposureMax,
                    AutoExposureMin = record.HdrAutoExposureMin,
                    SunlightScale = record.HdrSunlightScale,
                    SkyScale = record.HdrSkyScale,
                    MiddleGray = record.HdrMiddleGray
                },
                Cinematic = new
                {
                    Saturation = record.CinematicSaturation,
                    Brightness = record.CinematicBrightness,
                    Contrast = record.CinematicContrast
                },
                Tint = new
                {
                    Amount = record.TintAmount,
                    Color = WeatherProcessor.Rgb(record.TintColor)
                },
                DepthOfField = new
                {
                    Strength = record.DepthOfFieldStrength,
                    Distance = record.DepthOfFieldDistance,
                    Range = record.DepthOfFieldRange,
                    depthOfField.Mode,
                    depthOfField.VignetteRadius,
                    depthOfField.VignetteStrength
                }
            };
            await conn.ExecuteAsync(@"INSERT OR REPLACE INTO image_spaces (form_key, editor_id, data) VALUES (@FormKey, @EditorId, @Data)",
                new { FormKey = formKey, EditorId = record.EditorID, Data = SafeSerialize(data) }, transaction);
        }
    }

    public class VisualEffectProcessor : BaseRecordProcessor<IVisualEffectGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IVisualEffectGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            await conn.ExecuteAsync(@"INSERT OR REPLACE INTO visual_effects (form_key, editor_id, model_path, data) VALUES (@FormKey, @EditorId, @ModelPath, @Data)",
                new { FormKey = formKey, EditorId = record.EditorID, ModelPath = (string?)null, Data = (string?)null }, transaction);
        }
    }

    public class WaterProcessor : BaseRecordProcessor<IWaterGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IWaterGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            await conn.ExecuteAsync(@"INSERT OR REPLACE INTO waters (form_key, editor_id, name, material, data) VALUES (@FormKey, @EditorId, @Name, @Material, @Data)",
                new
                {
                    FormKey = formKey,
                    EditorId = record.EditorID,
                    Name = record.Name?.ToString(),
                    Material = record.Material.IsNull ? (string?)null : record.Material.FormKey.ToString(),
                    Data = WaterJson.Project(record).ToJsonString()
                }, transaction);
        }
    }

    public class ClimateProcessor : BaseRecordProcessor<IClimateGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IClimateGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            var data = new
            {
                Weathers = record.Weathers?.Select(w => new
                {
                    Weather = w.Weather.IsNull ? null : w.Weather.FormKey.ToString(),
                    w.Chance,
                    Global = w.Global.IsNull ? null : w.Global.FormKey.ToString()
                }).ToList(),
                record.SunriseBegin,
                record.SunriseEnd,
                record.SunsetBegin,
                record.SunsetEnd
            };
            await conn.ExecuteAsync(@"INSERT OR REPLACE INTO climates (form_key, editor_id, sun_texture, sun_glare_texture, data) VALUES (@FormKey, @EditorId, @SunTex, @SunGlareTex, @Data)",
                new { FormKey = formKey, EditorId = record.EditorID, SunTex = record.SunTexture, SunGlareTex = record.SunGlareTexture, Data = SafeSerialize(data) }, transaction);
        }
    }

    public class HeadPartProcessor : BaseRecordProcessor<IHeadPartGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IHeadPartGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO head_parts (form_key, editor_id, name, type, flags, model_path, texture_set)
                VALUES (@FormKey, @EditorId, @Name, @Type, @Flags, @ModelPath, @TextureSet)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Type = record.Type.ToString(),
                Flags = SafeSerialize(record.Flags),
                ModelPath = record.Model?.File?.ToString(),
                TextureSet = record.TextureSet.IsNull ? (string?)null : record.TextureSet.FormKey.ToString()
            }, transaction);
        }
    }

    public class PackageProcessor : BaseRecordProcessor<IPackageGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IPackageGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            await conn.ExecuteAsync(@"INSERT OR REPLACE INTO packages (form_key, editor_id, flags, procedure_type, interrupt_override, data) VALUES (@FormKey, @EditorId, @Flags, @ProcType, @IntOverride, @Data)",
                new { FormKey = formKey, EditorId = record.EditorID, Flags = SafeSerialize(record.Flags), ProcType = (string?)null, IntOverride = (string?)null, Data = (string?)null }, transaction);
        }
    }

    public class SoundCategoryProcessor : BaseRecordProcessor<ISoundCategoryGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(ISoundCategoryGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO sound_categories (form_key, editor_id, flags, parent, static_attenuation, default_menu_volume)
                VALUES (@FormKey, @EditorId, @Flags, @Parent, @StaticAtten, @DefaultVol)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Flags = SafeSerialize(record.Flags),
                Parent = record.Parent.IsNull ? (string?)null : record.Parent.FormKey.ToString(),
                StaticAtten = (double?)record.StaticVolumeMultiplier,
                DefaultVol = (double?)record.DefaultMenuVolume
            }, transaction);
        }
    }

    public class RegionProcessor : BaseRecordProcessor<IRegionGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IRegionGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO regions (form_key, editor_id, name, worldspace, objects, flags)
                VALUES (@FormKey, @EditorId, @Name, @Worldspace, @Objects, @Flags)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = (string?)null,
                Worldspace = record.Worldspace.IsNull ? (string?)null : record.Worldspace.FormKey.ToString(),
                Objects = (string?)null,
                Flags = (string?)null
            }, transaction);
        }
    }

    public class MessageProcessor : BaseRecordProcessor<IMessageGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IMessageGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO messages (form_key, editor_id, title, body, flags)
                VALUES (@FormKey, @EditorId, @Title, @Body, @Flags)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Title = record.Name?.ToString(),
                Body = (string?)null,
                Flags = SafeSerialize(record.Flags)
            }, transaction);
        }
    }

    public class TerminalProcessor : BaseRecordProcessor<ITerminalGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(ITerminalGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            const string sql = @"INSERT OR REPLACE INTO terminals (form_key, editor_id, name, description, flags)
                VALUES (@FormKey, @EditorId, @Name, @Description, @Flags)";
            await conn.ExecuteAsync(sql, new
            {
                FormKey = formKey,
                EditorId = record.EditorID,
                Name = record.Name?.ToString(),
                Description = (string?)null,
                Flags = SafeSerialize(record.Flags)
            }, transaction);
        }
    }

    public class MovementTypeProcessor : BaseRecordProcessor<IMovementTypeGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IMovementTypeGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            // SPED payload: directional speeds in units/s (Left/Right/Forward/Back)
            // and deg/s (Pitch/Roll/Yaw), each at Standing/Walk/Run/Sprint gaits.
            string? data = SafeSerialize(new
            {
                record.LeftStanding,
                record.LeftWalk,
                record.LeftRun,
                record.LeftSprint,
                record.RightStanding,
                record.RightWalk,
                record.RightRun,
                record.RightSprint,
                record.ForwardStanding,
                record.ForwardWalk,
                record.ForwardRun,
                record.ForwardSprint,
                record.BackStanding,
                record.BackWalk,
                record.BackRun,
                record.BackSprint,
                record.PitchStanding,
                record.PitchWalk,
                record.PitchRun,
                record.PitchSprint,
                record.RollStanding,
                record.RollWalk,
                record.RollRun,
                record.RollSprint,
                record.YawStanding,
                record.YawWalk,
                record.YawRun,
                record.YawSprint,
                record.FloatHeight,
                record.FlightAngleGain,
                AnimationChangeThresholds = record.AnimationChangeThresholds == null ? null : new
                {
                    record.AnimationChangeThresholds.Directional,
                    record.AnimationChangeThresholds.MovementSpeed,
                    record.AnimationChangeThresholds.RotationSpeed
                }
            });
            await conn.ExecuteAsync(@"INSERT OR REPLACE INTO movement_types (form_key, editor_id, name, data) VALUES (@FormKey, @EditorId, @Name, @Data)",
                new { FormKey = formKey, EditorId = record.EditorID, Name = record.Name?.ToString(), Data = data }, transaction);
        }
    }

    // MATT records: Havok material types. The engine joins collision shapes to them by material_hash
    // and reads buoyancy for the water simulation.
    public class MaterialTypeProcessor : BaseRecordProcessor<IMaterialTypeGetter>
    {
        protected override async Task ProcessSpecificRecordAsync(IMaterialTypeGetter record, string formKey, SqliteConnection conn, SqliteTransaction transaction)
        {
            string? materialName = record.Name;
            long? materialHash = string.IsNullOrEmpty(materialName) ? null : BgsMaterialHash.Compute(materialName);
            await conn.ExecuteAsync(
                @"INSERT OR REPLACE INTO material_types (form_key, editor_id, material_name, material_hash, buoyancy, flags, parent)
                  VALUES (@FormKey, @EditorId, @MaterialName, @MaterialHash, @Buoyancy, @Flags, @Parent)",
                new
                {
                    FormKey = formKey,
                    EditorId = record.EditorID,
                    MaterialName = materialName,
                    MaterialHash = materialHash,
                    Buoyancy = record.Buoyancy,
                    Flags = record.Flags is { } flags ? (long)(uint)flags : 0L,
                    Parent = record.Parent.FormKeyNullable?.ToString(),
                }, transaction);
        }
    }

    public class TypeJsonConverter : System.Text.Json.Serialization.JsonConverter<Type>
    {
        public override Type? Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options) => null;
        public override void Write(Utf8JsonWriter writer, Type value, JsonSerializerOptions options)
        {
            writer.WriteStringValue(value?.ToString());
        }
    }
}
