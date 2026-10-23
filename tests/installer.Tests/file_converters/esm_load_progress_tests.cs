using AssetConverter;
using AssetConverter.Progress;
using Microsoft.Data.Sqlite;
using Xunit;

namespace InstallerTests
{
    // The "Building database" bar of the installer: every counted unit of work is ticked exactly once. Drives the
    // process-wide InstallProgress, so it runs serially with the other tests that use it.
    [Collection("Console redirection")]
    public sealed class EsmLoadProgressTests : IClassFixture<LoadedFixtureDatabase>, IDisposable
    {
        const string Step = "Building database";

        readonly LoadedFixtureDatabase _fixture;
        readonly TempDirectory _directory = new();

        public EsmLoadProgressTests(LoadedFixtureDatabase fixture)
        {
            _fixture = fixture;
            InstallProgress.Start([new StepDefinition(Step)]);
        }

        public void Dispose()
        {
            InstallProgress.Stop();
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
        }

        async Task<(StepSnapshot Progress, string DatabasePath)> LoadWithProgress()
        {
            int phase = InstallProgress.BeginPhase(Step, 0);
            string databasePath = Path.Combine(_directory.Path, "data.db");
            await NavMeshFixture.Loader(databasePath).LoadESMAsync(_fixture.EsmPath, phase, TestContext.Current.CancellationToken);
            return (InstallProgress.Model!.GetSnapshot().Steps.Single(s => s.Name == Step), databasePath);
        }

        static long Count(string databasePath, string table)
        {
            using var connection = new SqliteConnection(new SqliteConnectionStringBuilder { DataSource = databasePath, Mode = SqliteOpenMode.ReadOnly, Pooling = false }.ToString());
            connection.Open();
            using var command = connection.CreateCommand();
            command.CommandText = $"SELECT COUNT(*) FROM {table}";
            return (long)command.ExecuteScalar()!;
        }

        [Fact]
        public async Task Load_EveryCountedUnitOfWorkIsTickedExactlyOnce()
        {
            var (progress, _) = await LoadWithProgress();

            Assert.True(progress.Total > 0);
            Assert.Equal(progress.Total, progress.Completed);
        }

        [Fact]
        public async Task Load_ExplainsTheClosingStepInTheDetailLine()
        {
            var (progress, _) = await LoadWithProgress();

            Assert.Equal("Finalizing database", progress.Detail);
        }

        // Records (one unit each; a placed ref is a records row and a placed_objects row), plus one unit per teleport row.
        [Fact]
        public async Task Load_TotalCoversRecordsPlacedObjectsAndTeleports()
        {
            var (progress, db) = await LoadWithProgress();

            long expected = Count(db, "records") + Count(db, "placed_objects") + Count(db, "placed_teleports");
            Assert.Equal(expected, progress.Total);
        }
    }
}
