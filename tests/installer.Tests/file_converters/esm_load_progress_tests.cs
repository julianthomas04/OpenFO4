using AssetConverter;
using AssetConverter.Pipeline;
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

        // The discovery batches the cells as the load does; the fixture holds a deleted ref and teleports.
        [Fact]
        public async Task CountCells_MatchesTheRowsTheLoadWrote()
        {
            var (_, db) = await LoadWithProgress();
            using var file = Fallout4Mod.CreateFromBinaryOverlay(_fixture.EsmPath, Fallout4Release.Fallout4);

            var counts = ESMLoader.CountCells(file, TestContext.Current.CancellationToken);

            Assert.True(counts.Teleports > 0);
            Assert.Equal(Count(db, "cells"), counts.Cells);
            Assert.Equal(Count(db, "placed_objects"), counts.Placed);
            Assert.Equal(Count(db, "placed_teleports"), counts.Teleports);
            Assert.Equal(Count(db, "landscapes"), counts.Landscapes);
        }

        [Fact]
        public void CountGroup_ARecordTypeMutagenDoesNotKnow_CountsAsEmpty()
        {
            Assert.Equal(0, ESMLoader.CountGroup(new UnknownRecordTypeGroup()));
        }

        // A collection reports its size without reading, which the load's materialization does not.
        [Fact]
        public void CountGroup_ACollectionThatFailsOnlyWhenRead_CountsAsEmptyLikeTheLoad()
        {
            Assert.Equal(0, ESMLoader.CountGroup(new UnknownRecordTypeCollection()));
        }

        [Fact]
        public void CountGroup_AnyOtherReadFailure_Propagates()
        {
            Assert.Throws<InvalidDataException>(() => ESMLoader.CountGroup(FailingGroup()));

            static IEnumerable<int> FailingGroup()
            {
                yield return 1;
                throw new InvalidDataException("truncated");
            }
        }

        [Fact]
        public async Task Load_ASystemicNavMeshFailure_IsRaisedByTheDiscoveryWithTheErrorsItFound()
        {
            var strict = new ESMLoader(Path.Combine(_directory.Path, "strict.db")) { Memory = MemoryBudget.Unlimited(), NavMeshMaxSkippedFraction = 0 };
            var originalError = Console.Error;
            var captured = new StringWriter();
            Console.SetError(captured);
            try
            {
                await Assert.ThrowsAsync<NavMeshExtractionException>(() => strict.LoadESMAsync(_fixture.EsmPath, ct: TestContext.Current.CancellationToken));
            }
            finally
            {
                Console.SetError(originalError);
            }

            Assert.Contains("NAVM ERROR", captured.ToString(), StringComparison.Ordinal);
        }

        [Fact]
        public async Task CountNavMeshes_LeavesOutDeletedMeshesAndMatchesTheRowsTheLoadWrote()
        {
            var (_, db) = await LoadWithProgress();
            var loader = NavMeshFixture.Loader(Path.Combine(_directory.Path, "count.db"));
            using (var file = Fallout4Mod.CreateFromBinaryOverlay(_fixture.EsmPath, Fallout4Release.Fallout4))
                Assert.Equal(Count(db, "navigation_meshes"), loader.CountNavMeshes(file));

            string withDeleted = Path.Combine(_directory.Path, "deleted.esm");
            File.Copy(_fixture.EsmPath, withDeleted);
            RawEsmBuilder.AddRecordFlags(withDeleted, "NAVM", _fixture.Plugin.FormKeyOf(NavMeshFixture.Neighbour).ID, RecordFlags.Deleted);
            using var edited = Fallout4Mod.CreateFromBinaryOverlay(withDeleted, Fallout4Release.Fallout4);

            Assert.Equal(Count(db, "navigation_meshes") - 1, loader.CountNavMeshes(edited));
        }

        sealed class UnknownRecordTypeGroup : IEnumerable<int>
        {
            public IEnumerator<int> GetEnumerator() => throw new InvalidDataException("Unknown major record type: XXXX");
            System.Collections.IEnumerator System.Collections.IEnumerable.GetEnumerator() => GetEnumerator();
        }

        sealed class UnknownRecordTypeCollection : ICollection<int>
        {
            public int Count => 3;
            public bool IsReadOnly => true;
            public void CopyTo(int[] array, int arrayIndex) => throw new InvalidDataException("Unknown major record type: XXXX");
            public IEnumerator<int> GetEnumerator() => throw new InvalidDataException("Unknown major record type: XXXX");
            System.Collections.IEnumerator System.Collections.IEnumerable.GetEnumerator() => GetEnumerator();
            public void Add(int item) => throw new NotSupportedException();
            public void Clear() => throw new NotSupportedException();
            public bool Contains(int item) => throw new NotSupportedException();
            public bool Remove(int item) => throw new NotSupportedException();
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
