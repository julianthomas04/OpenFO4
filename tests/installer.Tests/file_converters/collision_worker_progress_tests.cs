using AssetConverter;
using AssetConverter.Progress;
using Xunit;

namespace InstallerTests
{
    // Drives the process-wide InstallProgress, so it runs serially with the other tests that use it.
    [Collection("Console redirection")]
    public sealed class CollisionWorkerProgressTests : IDisposable
    {
        private const string Step = "Extracting collision";

        public CollisionWorkerProgressTests() => InstallProgress.Start([new StepDefinition(Step)]);

        public void Dispose() => InstallProgress.Stop();

        private static StepSnapshot Snapshot() => InstallProgress.Model!.GetSnapshot().Steps.Single(s => s.Name == Step);

        private static int BeginPhase(long total) => InstallProgress.BeginPhase(Step, total);

        [Fact]
        public void EachProgressLineTicksTheNewNifsAgainstTheTotalTheStepBeganWith()
        {
            var progress = new CollisionWorkerProgress(BeginPhase(100), 100);

            progress.Handle("  ... 40/100 NIFs processed, 3 sidecars written, 0 errors");
            progress.Handle("  ... 75/100 NIFs processed, 5 sidecars written, 0 errors");

            Assert.Equal(100, Snapshot().Total);
            Assert.Equal(75, Snapshot().Completed);
        }

        [Fact]
        public void TheWorkersOwnTotalLineNeverChangesTheTotal()
        {
            var progress = new CollisionWorkerProgress(BeginPhase(100), 100);

            progress.Handle("  60 NIFs to process (40 existing sidecars skipped)");
            progress.Handle("  99999999999999999999999 NIFs to process (0 existing sidecars skipped)");

            Assert.Equal(100, Snapshot().Total);
            Assert.Equal(0, Snapshot().Completed);
        }

        [Fact]
        public void CompletingTheWorkerTicksTheNifsAfterTheLastProgressLineAndSkippedSidecarsToo()
        {
            var progress = new CollisionWorkerProgress(BeginPhase(100), 100);
            progress.Handle("  60 NIFs to process (40 existing sidecars skipped)");
            progress.Handle("  ... 30/100 NIFs processed, 3 sidecars written, 0 errors");

            progress.Complete();

            Assert.Equal(100, Snapshot().Completed);
        }

        [Fact]
        public void LinesThatAreNotProgressAreIgnored()
        {
            var progress = new CollisionWorkerProgress(BeginPhase(10), 10);

            progress.Handle("Collision worker: 10 NIFs, 4 sidecars written, 0 skipped (existing), 0 errors, 0 decode problems; manifest 4 entries");
            progress.Handle("  [manifest] skipping unreadable x");

            Assert.Equal(10, Snapshot().Total);
            Assert.Equal(0, Snapshot().Completed);
        }

        [Fact]
        public void ACountTooLargeToParseIsReportedAndNeverThrown()
        {
            var warnings = new List<string>();
            var progress = new CollisionWorkerProgress(BeginPhase(5), 5, warnings.Add);

            progress.Handle("  ... 99999999999999999999999/5 NIFs processed, 0 sidecars written, 0 errors");

            Assert.Single(warnings);
            Assert.Contains("unreadable count", warnings[0]);
            Assert.Equal(0, Snapshot().Completed);
        }

        [Fact]
        public async Task ProgressLinesAndCompletionFromDifferentThreadsTickEveryNifExactlyOnce()
        {
            var progress = new CollisionWorkerProgress(BeginPhase(4000), 4000);
            var lines = Task.Run(() =>
            {
                for (int done = 1; done <= 4000; done++)
                    progress.Handle($"  ... {done}/4000 NIFs processed, 0 sidecars written, 0 errors");
            }, TestContext.Current.CancellationToken);
            var completions = Task.Run(() =>
            {
                for (int i = 0; i < 4000; i++)
                    progress.Complete();
            }, TestContext.Current.CancellationToken);

            await Task.WhenAll(lines, completions);

            Assert.Equal(4000, Snapshot().Completed);
        }

        [Fact]
        public void ProgressBeyondTheTotalNeverTicksPastIt()
        {
            var progress = new CollisionWorkerProgress(BeginPhase(10), 10, _ => { });

            progress.Handle("  ... 12/10 NIFs processed, 0 sidecars written, 0 errors");

            Assert.Equal(10, Snapshot().Completed);
        }

        [Theory]
        [InlineData("  ... 5/12 NIFs processed, 0 sidecars written, 0 errors")]
        [InlineData("  ... 12/10 NIFs processed, 0 sidecars written, 0 errors")]
        public void AWorkerWhoseCountsDisagreeWithTheTotalIsWarnedOfOnceAndNeverThrown(string line)
        {
            var warnings = new List<string>();
            var progress = new CollisionWorkerProgress(BeginPhase(10), 10, warnings.Add);

            progress.Handle(line);
            progress.Handle(line);

            var warning = Assert.Single(warnings);
            Assert.StartsWith("WARNING:", warning, StringComparison.Ordinal);
            Assert.Contains("10 were announced", warning, StringComparison.Ordinal);
        }

        [Fact]
        public void AWorkerWhoseCountsAgreeWithTheTotalIsNotWarnedOf()
        {
            var warnings = new List<string>();
            var progress = new CollisionWorkerProgress(BeginPhase(10), 10, warnings.Add);

            progress.Handle("  ... 4/10 NIFs processed, 0 sidecars written, 0 errors");
            progress.Handle("  ... 10/10 NIFs processed, 0 sidecars written, 0 errors");

            Assert.Empty(warnings);
        }

        [Fact]
        public void WithoutAProgressPhaseTheWorkersOutputIsHandledAndNothingIsReported()
        {
            var progress = new CollisionWorkerProgress(-1, 10);

            progress.Handle("  10 NIFs to process (0 existing sidecars skipped)");
            progress.Handle("  ... 7/10 NIFs processed, 0 sidecars written, 0 errors");
            progress.Complete();

            Assert.Equal(0, Snapshot().Completed);
            Assert.Equal(0, Snapshot().Total);
        }
    }
}
