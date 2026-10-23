using AssetConverter;
using AssetConverter.Pipeline;
using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;
using Xunit;

namespace InstallerTests
{
    // ADDN -> addon_nodes row. A real binary plugin is loaded through the real loader.
    public sealed class EsmLoadAddonNodeTests : IDisposable
    {
        const string FireModel = @"Effects\FireMed01.nif";
        const int NodeIndex = 5;
        const int Cap = 3;
        const int Flags = 0x1;

        sealed record AddonNodeRow(string? EditorId, string? ModelPath, long? NodeIndex, string? Light, string? Sound, long? Cap, long? Flags);

        readonly TempDirectory _directory = new();
        readonly string _dbPath;

        public EsmLoadAddonNodeTests() => _dbPath = Path.Combine(_directory.Path, "data.db");

        public void Dispose()
        {
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
        }

        SqliteConnection Open()
        {
            var connection = new SqliteConnection(new SqliteConnectionStringBuilder { DataSource = _dbPath, Pooling = false }.ToString());
            connection.Open();
            return connection;
        }

        const string RowSql = @"SELECT editor_id AS EditorId, model_path AS ModelPath, node_index AS NodeIndex, light AS Light,
                                       sound AS Sound, master_particle_system_cap AS Cap, flags AS Flags
                                FROM addon_nodes WHERE form_key = @key";

        async Task<AddonNodeRow> LoadAddonNodeRowAsync(TinyPlugin plugin, string editorId)
        {
            string esmPath = plugin.Write(_directory.Path);
            await new ESMLoader(_dbPath) { Memory = MemoryBudget.Unlimited() }.LoadESMAsync(esmPath, ct: TestContext.Current.CancellationToken);

            using var connection = Open();
            return connection.QuerySingle<AddonNodeRow>(RowSql, new { key = plugin[editorId] });
        }

        [Fact]
        public async Task AddonNode_MapsModelIndexLightAndDnamFields()
        {
            var plugin = new TinyPlugin();
            var light = plugin.AddLight("FixtureFireLight", _ => { });
            plugin.AddAddonNode("FixtureFireNode", FireModel, node =>
            {
                node.NodeIndex = NodeIndex;
                node.Light.FormKey = light.FormKey;
                node.MasterParticleSystemCap = Cap;
                node.Flags = (AddonNode.Flag)Flags;
            });

            var row = await LoadAddonNodeRowAsync(plugin, "FixtureFireNode");

            Assert.Equal(new AddonNodeRow("FixtureFireNode", FireModel, NodeIndex, plugin["FixtureFireLight"], null, Cap, Flags), row);
        }

        [Fact]
        public async Task AddonNode_WithoutLightOrSound_StoresNullLinks()
        {
            var plugin = new TinyPlugin();
            plugin.AddAddonNode("FixtureBareNode", FireModel, node => node.NodeIndex = 0);

            var row = await LoadAddonNodeRowAsync(plugin, "FixtureBareNode");

            Assert.Equal(new AddonNodeRow("FixtureBareNode", FireModel, 0, null, null, 0, 0), row);
        }

        [Fact]
        public async Task DuplicateNodeIndex_KeepsOnlyTheLaterRecordInLoadOrder()
        {
            var plugin = new TinyPlugin();
            plugin.AddAddonNode("FixtureEarlierNode", FireModel, node => node.NodeIndex = NodeIndex);
            plugin.AddAddonNode("FixtureLaterNode", FireModel, node => node.NodeIndex = NodeIndex);

            string esmPath = plugin.Write(_directory.Path);
            await new ESMLoader(_dbPath) { Memory = MemoryBudget.Unlimited() }.LoadESMAsync(esmPath, ct: TestContext.Current.CancellationToken);

            using var connection = Open();
            var editorIds = connection.Query<string>("SELECT editor_id FROM addon_nodes WHERE node_index = @index", new { index = NodeIndex }).ToList();
            Assert.Equal(["FixtureLaterNode"], editorIds);
        }

        [Fact]
        public async Task ReloadAddonNodes_OnDatabaseWithOldTwoColumnTable_MigratesAndFills()
        {
            using (var connection = Open())
            {
                connection.Execute("CREATE TABLE addon_nodes (form_key TEXT PRIMARY KEY, editor_id TEXT)");
                connection.Execute("INSERT INTO addon_nodes (form_key, editor_id) VALUES ('0FFFFF:Fixture.esm', 'Stale')");
            }
            var plugin = new TinyPlugin();
            plugin.AddAddonNode("FixtureFireNode", FireModel, node => node.NodeIndex = NodeIndex);
            string esmPath = plugin.Write(_directory.Path);

            await new ESMLoader(_dbPath) { Memory = MemoryBudget.Unlimited() }.ReloadTablesAsync(esmPath, TableReloadPlan.Create(["addon_nodes"]), TestContext.Current.CancellationToken);

            using var verify = Open();
            Assert.Equal(1, verify.QuerySingle<int>("SELECT COUNT(*) FROM addon_nodes"));
            Assert.Equal(new AddonNodeRow("FixtureFireNode", FireModel, NodeIndex, null, null, 0, 0),
                verify.QuerySingle<AddonNodeRow>(RowSql, new { key = plugin["FixtureFireNode"] }));
        }
    }
}
