using AssetConverter;
using AssetConverter.Pipeline;
using Microsoft.Data.Sqlite;
using Xunit;

namespace InstallerTests
{
    // The ESM import is one memory-heavy unit: it holds its estimate for the whole load and waits for it while others hold the budget.
    public sealed class EsmLoadMemoryTests : IClassFixture<LoadedFixtureDatabase>, IDisposable
    {
        private static readonly TimeSpan Guard = TimeSpan.FromSeconds(60);

        private readonly LoadedFixtureDatabase _fixture;
        private readonly TempDirectory _directory = new();

        public EsmLoadMemoryTests(LoadedFixtureDatabase fixture)
        {
            _fixture = fixture;
        }

        public void Dispose()
        {
            SqliteConnection.ClearAllPools();
            _directory.Dispose();
        }

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private ESMLoader Loader(MemoryBudget budget) =>
            new(Path.Combine(_directory.Path, "data.db")) { NavMeshMaxSkippedFraction = NavMeshFixture.SkipToleranceForTheRejectedMeshes, Memory = budget };

        [Fact]
        public async Task Load_WaitsWhileOtherWorkHoldsTheBudgetAndRunsOnceItIsReleased()
        {
            var budget = new MemoryBudget(MemoryEstimates.EsmImportBytes);
            var other = await budget.ReserveAsync(1, Ct);

            var load = Loader(budget).LoadESMAsync(_fixture.EsmPath, ct: Ct);
            Assert.True(SpinWait.SpinUntil(() => budget.WaitingCount == 1, Guard));
            Assert.False(load.IsCompleted);

            other.Dispose();
            await load.WaitAsync(Guard, Ct);

            Assert.Equal(0, budget.ReservedBytes);
            Assert.True(File.Exists(Path.Combine(_directory.Path, "data.db")));
        }

        [Fact]
        public async Task Load_CancelledWhileWaitingReservesNothingAndWritesNoDatabase()
        {
            var budget = new MemoryBudget(MemoryEstimates.EsmImportBytes);
            using var other = await budget.ReserveAsync(1, Ct);
            using var cancel = new CancellationTokenSource();

            var load = Loader(budget).LoadESMAsync(_fixture.EsmPath, ct: cancel.Token);
            Assert.True(SpinWait.SpinUntil(() => budget.WaitingCount == 1, Guard));
            await cancel.CancelAsync();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => load.WaitAsync(Guard, Ct));
            Assert.Equal(1, budget.ReservedBytes);
            Assert.Equal(0, budget.WaitingCount);
            Assert.False(File.Exists(Path.Combine(_directory.Path, "data.db")));
        }

        [Fact]
        public async Task Load_ReleasesItsReservationWhenTheImportFails()
        {
            var budget = new MemoryBudget(MemoryEstimates.EsmImportBytes);

            await Assert.ThrowsAnyAsync<Exception>(() => Loader(budget).LoadESMAsync(Path.Combine(_directory.Path, "missing.esm"), ct: Ct));

            Assert.Equal(0, budget.ReservedBytes);
        }
    }
}
