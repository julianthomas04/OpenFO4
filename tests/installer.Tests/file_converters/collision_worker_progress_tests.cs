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

        private readonly int _phase;

        public CollisionWorkerProgressTests()
        {
            InstallProgress.Start([new StepDefinition(Step)]);
            _phase = InstallProgress.BeginPhase(Step, 0);
        }

        public void Dispose() => InstallProgress.Stop();

        private static StepSnapshot Snapshot() => InstallProgress.Model!.GetSnapshot().Steps.Single(s => s.Name == Step);

        [Fact]
        public void TheTotalLineSetsTheTotalAndEachProgressLineTicksTheNewNifs()
        {
            var progress = new CollisionWorkerProgress(_phase);

            progress.Handle("  100 NIFs to process (20 existing sidecars skipped)");
            progress.Handle("  ... 40/100 NIFs processed, 3 sidecars written, 0 errors");
            progress.Handle("  ... 75/100 NIFs processed, 5 sidecars written, 0 errors");

            Assert.Equal(100, Snapshot().Total);
            Assert.Equal(75, Snapshot().Completed);
        }

        [Fact]
        public void CompletingTheWorkerTicksTheNifsAfterTheLastProgressLine()
        {
            var progress = new CollisionWorkerProgress(_phase);
            progress.Handle("  100 NIFs to process (0 existing sidecars skipped)");
            progress.Handle("  ... 60/100 NIFs processed, 3 sidecars written, 0 errors");

            progress.Complete();

            Assert.Equal(100, Snapshot().Completed);
        }

        [Fact]
        public void LinesThatAreNotProgressAreIgnored()
        {
            var progress = new CollisionWorkerProgress(_phase);

            progress.Handle("Collision worker: 10 NIFs, 4 sidecars written, 0 skipped (existing), 0 errors, 0 decode problems; manifest 4 entries");
            progress.Handle("  [manifest] skipping unreadable x");
            progress.Complete();

            Assert.Equal(0, Snapshot().Total);
            Assert.Equal(0, Snapshot().Completed);
        }

        [Fact]
        public void ACountTooLargeToParseIsReportedAndNeverThrown()
        {
            var warnings = new List<string>();
            var progress = new CollisionWorkerProgress(_phase, warnings.Add);

            progress.Handle("  99999999999999999999999 NIFs to process (0 existing sidecars skipped)");
            progress.Handle("  ... 99999999999999999999999/5 NIFs processed, 0 sidecars written, 0 errors");
            progress.Complete();

            Assert.Equal(2, warnings.Count);
            Assert.All(warnings, warning => Assert.Contains("unreadable count", warning));
            Assert.Equal(0, Snapshot().Completed);
        }

        [Fact]
        public async Task ProgressLinesAndCompletionFromDifferentThreadsTickEveryNifExactlyOnce()
        {
            var progress = new CollisionWorkerProgress(_phase);
            progress.Handle("  4000 NIFs to process (0 existing sidecars skipped)");
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
            var progress = new CollisionWorkerProgress(_phase);
            progress.Handle("  10 NIFs to process (0 existing sidecars skipped)");

            progress.Handle("  ... 12/12 NIFs processed, 0 sidecars written, 0 errors");

            Assert.Equal(10, Snapshot().Completed);
        }
    }
}
