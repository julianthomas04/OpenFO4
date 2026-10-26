using AssetConverter;
using AssetConverter.Pipeline;
using Dapper;
using Microsoft.Data.Sqlite;
using Xunit;

namespace InstallerTests
{
    // ESMLoader.ReloadTablesAsync against a copy of the freshly loaded fixture database: only the planned tables
    // (and their records rows) are rebuilt, inside one transaction.
    [Collection(FixtureEsm.Name)]
    public sealed class EsmTableReloadTests : IDisposable
    {
        readonly LoadedFixtureDatabase _fixture;
        readonly TempDirectory _directory = new();
        readonly string _dbPath;

        public EsmTableReloadTests(LoadedFixtureDatabase fixture)
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

        Task Reload(params string[] tables) =>
            new ESMLoader(_dbPath) { Memory = MemoryBudget.Unlimited() }.ReloadTablesAsync(_fixture.EsmPath, TableReloadPlan.Create(tables), TestContext.Current.CancellationToken);

        SqliteConnection Open()
        {
            var connection = new SqliteConnection(new SqliteConnectionStringBuilder { DataSource = _dbPath, Pooling = false }.ToString());
            connection.Open();
            return connection;
        }

        T Scalar<T>(string sql, object? parameters = null)
        {
            using var connection = Open();
            return connection.QuerySingle<T>(sql, parameters);
        }

        void Execute(string sql)
        {
            using var connection = Open();
            connection.Execute(sql);
        }

        List<string> Dump(string sql)
        {
            using var connection = Open();
            return connection.Query(sql).Select(row => string.Join("|", ((IDictionary<string, object?>)row).Values)).ToList();
        }

        [Fact]
        public async Task Reload_Statics_RemovesStaleRowsAndRestoresEditedOnes()
        {
            Execute("INSERT INTO records (form_key, record_type, editor_id, source_plugin) VALUES ('0FFFFF:Fixture.esm', 'STAT', 'Stale', 'Fixture.esm')");
            Execute("INSERT INTO statics (form_key, editor_id, model_path) VALUES ('0FFFFF:Fixture.esm', 'Stale', 'stale.nif')");
            Execute($"UPDATE statics SET model_path = 'tampered.nif' WHERE form_key = '{_fixture.KeyOf("WallStatic")}'");

            await Reload("statics");

            Assert.Equal(0, Scalar<long>("SELECT COUNT(*) FROM statics WHERE form_key = '0FFFFF:Fixture.esm'"));
            Assert.Equal(0, Scalar<long>("SELECT COUNT(*) FROM records WHERE form_key = '0FFFFF:Fixture.esm'"));
            Assert.Equal(LoadedFixtureDatabase.ModelOfWall,
                Scalar<string>("SELECT model_path FROM statics WHERE form_key = @key", new { key = _fixture.KeyOf("WallStatic") }));
        }

        [Fact]
        public async Task Reload_ObjectModifications_ResolvesAttachPointNamesFromTheKeywordsAlreadyInTheDatabase()
        {
            Execute("UPDATE object_modifications SET attach_point_name = NULL, attach_parent_slots = NULL");

            await Reload("object_modifications");

            Assert.Equal(LoadedFixtureDatabase.GripAttachPoint,
                Scalar<string>("SELECT attach_point_name FROM object_modifications WHERE form_key = @key", new { key = _fixture.KeyOf("ModGrip") }));
            Assert.Equal($"[\"{LoadedFixtureDatabase.MuzzleAttachPoint}\",\"{LoadedFixtureDatabase.GripAttachPoint}\"]",
                Scalar<string>("SELECT attach_parent_slots FROM object_modifications WHERE form_key = @key", new { key = _fixture.KeyOf("ModGrip") }));
        }

        [Fact]
        public async Task Reload_Statics_LeavesOtherTablesAndTheirRecordsRowsAlone()
        {
            Execute($"UPDATE lights SET radius = 1 WHERE form_key = '{_fixture.KeyOf("LampLight")}'");
            long placedBefore = Scalar<long>("SELECT COUNT(*) FROM placed_objects");
            long recordsOfOtherTypesBefore = Scalar<long>("SELECT COUNT(*) FROM records WHERE record_type <> 'STAT'");

            await Reload("statics");

            Assert.Equal(1.0, Scalar<double>("SELECT radius FROM lights"));
            Assert.Equal(placedBefore, Scalar<long>("SELECT COUNT(*) FROM placed_objects"));
            Assert.Equal(recordsOfOtherTypesBefore, Scalar<long>("SELECT COUNT(*) FROM records WHERE record_type <> 'STAT'"));
        }

        [Fact]
        public async Task Reload_Cells_RebuildsPlacedObjectsIdenticallyIncludingResolvedVisibility()
        {
            var before = Dump("SELECT * FROM placed_objects ORDER BY form_key");
            Execute("UPDATE placed_objects SET initially_hidden = 1, position_x = -1, scale = 9");
            Execute("DELETE FROM placed_teleports");

            await Reload("cells");

            Assert.Equal(before, Dump("SELECT * FROM placed_objects ORDER BY form_key"));
            Assert.Equal(2, Scalar<long>("SELECT COUNT(*) FROM placed_teleports"));
            Assert.Equal(1, Scalar<long>("SELECT COUNT(*) FROM landscapes"));
        }

        [Fact]
        public async Task Reload_Cells_KeepsCellRowsAndWorldspacePersistentFlag()
        {
            await Reload("cells");

            Assert.Equal(1, Scalar<long>("SELECT is_worldspace_persistent_cell FROM cells WHERE editor_id = 'FixtureWorldTopCell'"));
            Assert.Equal(3, Scalar<long>("SELECT COUNT(*) FROM cells"));
        }

        [Fact]
        public async Task Reload_FailureInsideJob_RollsBackToThePreviousContents()
        {
            Execute("INSERT INTO records (form_key, record_type, editor_id, source_plugin) VALUES ('0FFFFF:Fixture.esm', 'STAT', 'Stale', 'Fixture.esm')");
            Execute("INSERT INTO statics (form_key, editor_id, model_path) VALUES ('0FFFFF:Fixture.esm', 'Stale', 'stale.nif')");
            Execute("CREATE TRIGGER reject_inserts BEFORE INSERT ON statics BEGIN SELECT RAISE(ABORT, 'rejected for the test'); END");
            long staticsBefore = Scalar<long>("SELECT COUNT(*) FROM statics");

            var error = await Assert.ThrowsAsync<SqliteException>(() => Reload("statics"));

            Assert.Contains("rejected for the test", error.Message);
            Assert.Equal(staticsBefore, Scalar<long>("SELECT COUNT(*) FROM statics"));
            Assert.Equal(1, Scalar<long>("SELECT COUNT(*) FROM records WHERE form_key = '0FFFFF:Fixture.esm'"));
        }

        [Fact]
        public async Task Reload_MissingDatabase_ThrowsNamingThePath()
        {
            string missing = Path.Combine(_directory.Path, "absent.db");

            var error = await Assert.ThrowsAsync<MissingDatabaseException>(() =>
                new ESMLoader(missing) { Memory = MemoryBudget.Unlimited() }.ReloadTablesAsync(_fixture.EsmPath, TableReloadPlan.Create(["statics"]), TestContext.Current.CancellationToken));

            Assert.Contains(missing, error.Message);
            Assert.False(File.Exists(missing));
        }

        [Fact]
        public async Task Reload_MaterialSwaps_RestoresSwapStubAndReturnsWithoutSynthesisWhenRootsAreMissing()
        {
            Execute("DELETE FROM material_swaps");

            await Reload("material_swaps");

            Assert.Equal("RedSwap", Scalar<string>("SELECT editor_id FROM material_swaps"));
        }
    }
}
