using AssetConverter;
using Dapper;
using Microsoft.Data.Sqlite;
using Xunit;

namespace InstallerTests
{
    // DatabaseManager.InitializeAsync applies the real esm_schema.sql and then adds columns that were introduced after
    // a database was created (CREATE TABLE IF NOT EXISTS never adds columns).
    public sealed class DatabaseManagerTests : IDisposable
    {
        readonly TempDirectory _directory = new();

        string DatabasePath => Path.Combine(_directory.Path, "data.db");

        public void Dispose()
        {
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
        }

        static List<string> Columns(SqliteConnection connection, string table) =>
            connection.Query<string>("SELECT name FROM pragma_table_info(@table)", new { table }).ToList();

        [Fact]
        public async Task Initialize_OnEmptyDatabase_CreatesTheCoreTables()
        {
            using var manager = new DatabaseManager(DatabasePath);

            await manager.InitializeAsync();

            var tables = manager.Connection.Query<string>("SELECT name FROM sqlite_master WHERE type = 'table'").ToHashSet();
            Assert.All(new[] { "records", "cells", "placed_objects", "placed_teleports", "statics", "lights", "weather", "landscapes" },
                table => Assert.Contains(table, tables));
        }

        [Fact]
        public async Task Initialize_Twice_KeepsExistingRows()
        {
            using var manager = new DatabaseManager(DatabasePath);
            await manager.InitializeAsync();
            manager.Connection.Execute("INSERT INTO records (form_key, record_type, editor_id, source_plugin) VALUES ('000001:A.esm', 'STAT', 'X', 'A.esm')");

            await manager.InitializeAsync(bulkLoad: false);

            Assert.Equal(1, manager.Connection.QuerySingle<long>("SELECT COUNT(*) FROM records"));
        }

        [Fact]
        public async Task Initialize_DatabaseFromOlderInstaller_GainsTheOptionalColumnsWithoutLosingRows()
        {
            using (var legacy = new SqliteConnection($"Data Source={DatabasePath}"))
            {
                legacy.Open();
                legacy.Execute("CREATE TABLE records (form_key TEXT PRIMARY KEY, record_type TEXT NOT NULL, editor_id TEXT, source_plugin TEXT NOT NULL)");
                legacy.Execute("CREATE TABLE lights (form_key TEXT PRIMARY KEY, editor_id TEXT, model_path TEXT, radius REAL, color INTEGER, flags INTEGER, fade_value REAL, falloff_exponent REAL, is_marker INTEGER DEFAULT 0)");
                legacy.Execute("INSERT INTO lights (form_key, editor_id, radius) VALUES ('000001:A.esm', 'OldLamp', 100)");
            }
            SqliteConnection.ClearAllPools();
            using var manager = new DatabaseManager(DatabasePath);

            await manager.InitializeAsync(bulkLoad: false);

            var columns = Columns(manager.Connection, "lights");
            Assert.Contains("constant", columns);
            Assert.Contains("god_rays_near_clip", columns);
            Assert.Equal("OldLamp", manager.Connection.QuerySingle<string>("SELECT editor_id FROM lights"));
        }

        [Fact]
        public async Task Initialize_DatabaseWithoutTheWeaponAssemblyColumns_GainsThemWithoutLosingRows()
        {
            using (var legacy = new SqliteConnection($"Data Source={DatabasePath}"))
            {
                legacy.Open();
                legacy.Execute("CREATE TABLE weapons (form_key TEXT PRIMARY KEY, weapon_type TEXT)");
                legacy.Execute("CREATE TABLE object_modifications (form_key TEXT PRIMARY KEY, editor_id TEXT, target_type TEXT)");
                legacy.Execute("INSERT INTO weapons (form_key, weapon_type) VALUES ('000001:A.esm', 'Gun')");
                legacy.Execute("INSERT INTO object_modifications (form_key, editor_id) VALUES ('000002:A.esm', 'OldMod')");
            }
            SqliteConnection.ClearAllPools();
            using var manager = new DatabaseManager(DatabasePath);

            await manager.InitializeAsync(bulkLoad: false);

            Assert.Contains("default_mods", Columns(manager.Connection, "weapons"));
            Assert.Contains("object_template", Columns(manager.Connection, "weapons"));
            Assert.All(new[] { "model_path", "attach_point_name", "attach_parent_slots" },
                column => Assert.Contains(column, Columns(manager.Connection, "object_modifications")));
            Assert.Equal("Gun", manager.Connection.QuerySingle<string>("SELECT weapon_type FROM weapons"));
            Assert.Equal("OldMod", manager.Connection.QuerySingle<string>("SELECT editor_id FROM object_modifications"));
        }

        [Fact]
        public async Task Initialize_DatabaseWithTheOldMusicStubs_GainsTheMusicColumnsAndKeepsRows()
        {
            using (var legacy = new SqliteConnection($"Data Source={DatabasePath}"))
            {
                legacy.Open();
                legacy.Execute("CREATE TABLE music_types (form_key TEXT PRIMARY KEY, editor_id TEXT)");
                legacy.Execute("CREATE TABLE music_tracks (form_key TEXT PRIMARY KEY, editor_id TEXT, track_type TEXT, duration REAL, fade_in REAL, fade_out REAL, file_path TEXT, conditions TEXT)");
                legacy.Execute("INSERT INTO music_types (form_key, editor_id) VALUES ('000001:A.esm', 'OldType')");
            }
            SqliteConnection.ClearAllPools();
            using var manager = new DatabaseManager(DatabasePath);

            await manager.InitializeAsync(bulkLoad: false);

            Assert.All(new[] { "flags", "priority", "ducking_centidb", "fade_seconds" }, column => Assert.Contains(column, Columns(manager.Connection, "music_types")));
            Assert.All(new[] { "file_key", "anam_path" }, column => Assert.Contains(column, Columns(manager.Connection, "music_tracks")));
            Assert.Equal("OldType", manager.Connection.QuerySingle<string>("SELECT editor_id FROM music_types"));
        }

        [Fact]
        public async Task Initialize_AfterMigration_IsIdempotent()
        {
            using var manager = new DatabaseManager(DatabasePath);
            await manager.InitializeAsync();
            var before = Columns(manager.Connection, "placed_objects");

            await manager.InitializeAsync();

            Assert.Equal(before, Columns(manager.Connection, "placed_objects"));
        }

        [Fact]
        public void OpenWorkerConnection_ToMissingDatabase_Throws()
        {
            Assert.Throws<SqliteException>(() => DatabaseManager.OpenWorkerConnection(Path.Combine(_directory.Path, "absent.db")));
        }

        [Fact]
        public async Task OpenWorkerConnection_ToInitializedDatabase_CanWriteWhileTheMainConnectionIsOpen()
        {
            using var manager = new DatabaseManager(DatabasePath);
            await manager.InitializeAsync();

            using (var worker = DatabaseManager.OpenWorkerConnection(DatabasePath))
                worker.Execute("INSERT INTO records (form_key, record_type, editor_id, source_plugin) VALUES ('000001:A.esm', 'STAT', 'X', 'A.esm')");

            Assert.Equal(1, manager.Connection.QuerySingle<long>("SELECT COUNT(*) FROM records"));
        }

        [Fact]
        public async Task OpenWorkerConnection_DoesNotEnforceForeignKeysDuringBulkImport()
        {
            using var manager = new DatabaseManager(DatabasePath);
            await manager.InitializeAsync();

            using var worker = DatabaseManager.OpenWorkerConnection(DatabasePath);

            Assert.Equal(0, worker.QuerySingle<long>("PRAGMA foreign_keys"));
        }
    }
}
