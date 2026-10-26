using System.Reflection;
using AssetConverter;
using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;
using Xunit;

namespace InstallerTests
{
    // Grass extraction contract. Every GRAS DATA field is a typed column of
    // `grasses`, and each LTEX keeps its grass list in authored order in `ltex_grasses`.
    [Collection(FixtureEsm.Name)]
    public class EsmLoadGrassTests(LoadedFixtureDatabase db)
    {
        sealed class GrassRow
        {
            public string? EditorId { get; init; }
            public string? ModelPath { get; init; }
            public long Flags { get; init; }
            public long Density { get; init; }
            public long MinSlope { get; init; }
            public long MaxSlope { get; init; }
            public long UnitsFromWater { get; init; }
            public long UnitsFromWaterType { get; init; }
            public double PositionRange { get; init; }
            public double HeightRange { get; init; }
            public double ColorRange { get; init; }
            public double WavePeriod { get; init; }
        }

        GrassRow Grass(string editorId) =>
            db.QuerySingle<GrassRow>("SELECT * FROM grasses WHERE form_key = @key", new { key = db.KeyOf(editorId) });

        IReadOnlyList<string> GrassesOf(string ltexEditorId) =>
            db.Query<string>("SELECT grass_form_key FROM ltex_grasses WHERE ltex_form_key = @key ORDER BY ord", new { key = db.KeyOf(ltexEditorId) });

        [Fact]
        public void Grass_StoresEveryDataFieldAsATypedColumn()
        {
            var row = Grass(LoadedFixtureDatabase.FirstGrass);

            Assert.Equal(LoadedFixtureDatabase.FirstGrass, row.EditorId);
            Assert.Equal(@"Landscape\Grass\Tall.nif", row.ModelPath);
            Assert.Equal(GrassFixture.FlagsRaw, row.Flags);
            Assert.Equal(GrassFixture.Density, row.Density);
            Assert.Equal(GrassFixture.MinSlope, row.MinSlope);
            Assert.Equal(GrassFixture.MaxSlope, row.MaxSlope);
            Assert.Equal(GrassFixture.UnitsFromWater, row.UnitsFromWater);
            Assert.Equal(GrassFixture.UnitsFromWaterType, (uint)row.UnitsFromWaterType);
            Assert.Equal(GrassFixture.PositionRange, row.PositionRange);
            Assert.Equal(GrassFixture.HeightRange, row.HeightRange);
            Assert.Equal(GrassFixture.ColorRange, row.ColorRange);
            Assert.Equal(GrassFixture.WavePeriod, row.WavePeriod);
        }

        [Fact]
        public void Grass_Flags_AreTheRawNumericBitfield()
        {
            string type = db.QuerySingle<string>("SELECT typeof(flags) FROM grasses WHERE form_key = @key",
                new { key = db.KeyOf(LoadedFixtureDatabase.FirstGrass) });

            Assert.Equal("integer", type);
        }

        [Fact]
        public void Grass_Columns_CoverEveryDataPropertyMutagenDeclares()
        {
            string[] notData = ["BinaryWriteTranslator", "StaticRegistration", "EditorID", "FormKey", "Model", "ObjectBounds"];
            string[] unusedDataPadding = ["Unknown", "Unknown2", "Unknown3"];
            var declared = typeof(IGrassGetter)
                .GetProperties(BindingFlags.Public | BindingFlags.Instance | BindingFlags.DeclaredOnly)
                .Select(p => p.Name)
                .Except(notData).Except(unusedDataPadding);
            var columns = db.Query<string>("SELECT name FROM pragma_table_info('grasses')")
                .Select(c => c.Replace("_", "")).ToHashSet(StringComparer.OrdinalIgnoreCase);

            Assert.Equal(Array.Empty<string>(), declared.Where(name => !columns.Contains(name)));
        }

        [Fact]
        public void LandscapeTexture_WithTwoGrasses_KeepsAuthoredOrder()
        {
            Assert.Equal(
                [db.KeyOf(LoadedFixtureDatabase.SecondGrass), db.KeyOf(LoadedFixtureDatabase.FirstGrass)],
                GrassesOf("TwoGrassTexture"));
        }

        [Fact]
        public void LandscapeTexture_WithOneGrass_HasOneRow()
        {
            Assert.Equal([db.KeyOf(LoadedFixtureDatabase.FirstGrass)], GrassesOf("OneGrassTexture"));
        }

        [Fact]
        public void LandscapeTexture_WithoutGrass_HasNoRows()
        {
            Assert.Empty(GrassesOf("GrassTexture"));
        }
    }

    public sealed class GrassFlagsMigrationTests : IDisposable
    {
        readonly TempDirectory _directory = new();

        public void Dispose()
        {
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
        }

        string DatabasePath => Path.Combine(_directory.Path, "data.db");

        void CreateLegacyGrasses(string columns, string insertSql)
        {
            using var legacy = new SqliteConnection($"Data Source={DatabasePath}");
            legacy.Open();
            legacy.Execute($"CREATE TABLE grasses ({columns})");
            legacy.Execute(insertSql);
            SqliteConnection.ClearAllPools();
        }

        [Fact]
        public async Task Initialize_DatabaseWithTextGrassFlags_RebuildsGrassesWithIntegerFlagsAndNewColumns()
        {
            CreateLegacyGrasses(
                "form_key TEXT PRIMARY KEY, editor_id TEXT, model_path TEXT, flags TEXT",
                "INSERT INTO grasses (form_key, flags) VALUES ('000001:A.esm', '7')");
            using var manager = new DatabaseManager(DatabasePath);

            await manager.InitializeAsync(bulkLoad: false);

            var types = manager.Connection.Query<(string name, string type)>("SELECT name, type FROM pragma_table_info('grasses')")
                .ToDictionary(c => c.name, c => c.type);
            Assert.Equal("INTEGER", types["flags"]);
            Assert.Equal("INTEGER", types["density"]);
            Assert.Equal("REAL", types["wave_period"]);
        }

        [Fact]
        public async Task Initialize_DatabaseWithTextGrassFlags_KeepsItsRowsAndCastsTheFlags()
        {
            CreateLegacyGrasses(
                "form_key TEXT PRIMARY KEY, editor_id TEXT, model_path TEXT, flags TEXT, density INTEGER",
                "INSERT INTO grasses VALUES ('000001:A.esm', 'Tall', 'Landscape\\Grass\\Tall.nif', '5', 40), ('000002:A.esm', 'Short', NULL, '0', NULL)");
            using var manager = new DatabaseManager(DatabasePath);

            await manager.InitializeAsync(bulkLoad: false);

            var rows = manager.Connection.Query<(string form_key, string? editor_id, string? model_path, string flagsType, long flags, long? density)>(
                "SELECT form_key, editor_id, model_path, typeof(flags), flags, density FROM grasses ORDER BY form_key").ToList();
            Assert.Equal(("000001:A.esm", "Tall", @"Landscape\Grass\Tall.nif", "integer", 5L, (long?)40), rows[0]);
            Assert.Equal(("000002:A.esm", "Short", (string?)null, "integer", 0L, (long?)null), rows[1]);
            Assert.Equal(2, rows.Count);
            Assert.Equal(0L, manager.Connection.ExecuteScalar<long>(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'grasses_text_flags'"));
        }

        [Fact]
        public async Task Initialize_DatabaseWithIntegerGrassFlags_LeavesTheTableUntouched()
        {
            using (var first = new DatabaseManager(DatabasePath))
            {
                await first.InitializeAsync(bulkLoad: false);
                first.Connection.Execute(
                    "INSERT INTO records (form_key, record_type, editor_id, source_plugin) VALUES ('000001:A.esm', 'GRAS', 'Tall', 'A.esm')");
                first.Connection.Execute(
                    "INSERT INTO grasses (form_key, flags, density, wave_period) VALUES ('000001:A.esm', 5, 40, 3.5)");
            }
            SqliteConnection.ClearAllPools();
            using var manager = new DatabaseManager(DatabasePath);

            await manager.InitializeAsync(bulkLoad: false);

            var row = manager.Connection.QuerySingle<(long flags, long density, double wave_period)>(
                "SELECT flags, density, wave_period FROM grasses");
            Assert.Equal((5L, 40L, 3.5), row);
            Assert.Equal(0L, manager.Connection.ExecuteScalar<long>(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'grasses_text_flags'"));
        }
    }

    [Collection(FixtureEsm.Name)]
    public sealed class LandscapeTextureGrassListReloadTests : IDisposable
    {
        readonly LoadedFixtureDatabase _fixture;
        readonly TempDirectory _directory = new();
        readonly string _dbPath;

        public LandscapeTextureGrassListReloadTests(LoadedFixtureDatabase fixture)
        {
            _fixture = fixture;
            _dbPath = Path.Combine(_directory.Path, "data.db");
            fixture.CopyDatabaseTo(_dbPath);
        }

        public void Dispose()
        {
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
        }

        [Fact]
        public async Task LoadingALandscapeTextureAgain_ReplacesItsGrassList_WithoutStaleEntries()
        {
            string ltex = _fixture.KeyOf("OneGrassTexture");
            using (var connection = new SqliteConnection($"Data Source={_dbPath}"))
            {
                connection.Open();
                connection.Execute("INSERT INTO ltex_grasses (ltex_form_key, grass_form_key, ord) VALUES (@ltex, 'stale', 1), (@ltex, 'stale2', 2)",
                    new { ltex });
            }
            SqliteConnection.ClearAllPools();

            await NavMeshFixture.Loader(_dbPath).LoadESMAsync(_fixture.EsmPath, ct: TestContext.Current.CancellationToken);

            using var check = new SqliteConnection($"Data Source={_dbPath}");
            check.Open();
            Assert.Equal([_fixture.KeyOf(LoadedFixtureDatabase.FirstGrass)],
                check.Query<string>("SELECT grass_form_key FROM ltex_grasses WHERE ltex_form_key = @ltex ORDER BY ord", new { ltex }));
        }
    }
}
