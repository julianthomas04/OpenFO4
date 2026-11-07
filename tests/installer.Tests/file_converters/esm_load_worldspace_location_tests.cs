using AssetConverter;
using AssetConverter.Pipeline;
using Dapper;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;
using Xunit;

namespace InstallerTests
{
    public sealed class EsmLoadWorldspaceLocationTests : IDisposable
    {
        readonly TempDirectory _directory = new();
        readonly string _dbPath;

        public EsmLoadWorldspaceLocationTests() => _dbPath = Path.Combine(_directory.Path, "data.db");

        public void Dispose()
        {
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
        }

        async Task<string?> LocationOfLoadedWorldspace(TinyPlugin plugin)
        {
            await new ESMLoader(_dbPath) { Memory = MemoryBudget.Unlimited() }.LoadESMAsync(plugin.Write(_directory.Path), ct: TestContext.Current.CancellationToken);
            using var connection = new SqliteConnection(new SqliteConnectionStringBuilder { DataSource = _dbPath, Pooling = false }.ToString());
            connection.Open();
            return connection.QuerySingle<string?>("SELECT location FROM worldspaces WHERE form_key = @key", new { key = plugin["World"] });
        }

        [Fact]
        public async Task Worldspace_WithALocation_StoresItsFormKey()
        {
            var plugin = new TinyPlugin();
            var location = plugin.AddLocation("WorldLocation", _ => { });
            plugin.AddWorldspace("World").Location.SetTo(location.FormKey);

            Assert.Equal(plugin["WorldLocation"], await LocationOfLoadedWorldspace(plugin));
        }

        [Fact]
        public async Task Worldspace_WithoutALocation_StoresNull()
        {
            var plugin = new TinyPlugin();
            plugin.AddWorldspace("World");

            Assert.Null(await LocationOfLoadedWorldspace(plugin));
        }
    }
}
