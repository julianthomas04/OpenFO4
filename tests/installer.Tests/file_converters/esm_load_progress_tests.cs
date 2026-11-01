using AssetConverter;
using AssetConverter.Progress;
using Microsoft.Data.Sqlite;
using Mutagen.Bethesda.Fallout4;
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
            int phase = InstallProgress.BeginPhase(Step, null);
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

        // Tick throws on an unsealed phase, so a load that completes also sealed the total before its first tick; the phase
        // is still open here, so the seal can only have come from the loader.
        [Fact]
        public async Task Load_SealsTheTotalItselfWhileThePhaseIsStillOpen()
        {
            var (progress, _) = await LoadWithProgress();

            Assert.False(progress.IsPreparing);
            Assert.Equal(StepStatus.Running, progress.Status);
        }

        // The discovery counts placed refs with the helper the load flattens with; the fixture holds a deleted ref and teleports.
        [Fact]
        public async Task CountCellBatch_MatchesTheRowsTheFlattenWrote()
        {
            var (_, db) = await LoadWithProgress();
            using var file = Fallout4Mod.CreateFromBinaryOverlay(_fixture.EsmPath, Fallout4Release.Fallout4);

            var counts = ESMLoader.CountCellBatch(file.EnumerateMajorRecords<ICellGetter>().ToList());

            Assert.True(counts.Teleports > 0);
            Assert.Equal(Count(db, "cells"), counts.Cells);
            Assert.Equal(Count(db, "placed_objects"), counts.Placed);
            Assert.Equal(Count(db, "placed_teleports"), counts.Teleports);
            Assert.Equal(Count(db, "landscapes"), counts.Landscapes);
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
