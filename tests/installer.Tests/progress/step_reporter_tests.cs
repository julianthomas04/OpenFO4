using AssetConverter.Progress;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests.Progress
{
    public sealed class StepReporterTests : IDisposable
    {
        private readonly ManualTimeProvider _clock = new();
        private readonly StringWriter _output = new();

        public void Dispose() => _output.Dispose();

        private (ProgressModel Model, StepReporter Reporter) Create(params string[] stepNames)
        {
            var model = new ProgressModel([.. stepNames.Select(n => new StepDefinition(n, "files"))], _clock);
            return (model, new StepReporter(model, _output));
        }

        private string[] Lines() => _output.ToString().Split(Environment.NewLine, StringSplitOptions.RemoveEmptyEntries);

        [Fact]
        public void PendingStepsAreNotReported()
        {
            var (_, reporter) = Create("A", "B");

            reporter.Poll();

            Assert.Empty(Lines());
        }

        [Fact]
        public void AStartedStepIsReportedOnceWithItsOffset()
        {
            var (model, reporter) = Create("Converting meshes");
            _clock.Advance(TimeSpan.FromSeconds(65));
            model.BeginPhase("Converting meshes", 100, null);

            reporter.Poll();
            reporter.Poll();

            Assert.Equal(["[1m 05s] Started: Converting meshes"], Lines());
        }

        [Fact]
        public void AFinishedStepReportsItsDurationAndCounters()
        {
            var (model, reporter) = Create("Converting meshes");
            int phase = model.BeginPhase("Converting meshes", 63_000, null);
            reporter.Poll();
            _clock.Advance(TimeSpan.FromSeconds(125));
            model.Tick(phase, 63_000);
            model.EndPhase(phase);

            reporter.Poll();
            reporter.Poll();

            Assert.Equal(
                ["[0.0s] Started: Converting meshes", "[2m 05s] Finished: Converting meshes in 2m 05s (63,000 of 63,000 files)"],
                Lines());
        }

        private static void Work(ProgressModel model, string step)
        {
            int phase = model.BeginPhase(step, 1, null);
            model.Tick(phase, 1);
            model.EndPhase(phase);
        }

        [Fact]
        public void AStepThatStartsAndEndsBetweenPollsReportsBothLines()
        {
            var (model, reporter) = Create("Fast");
            Work(model, "Fast");

            reporter.Poll();

            Assert.Equal(["[0.0s] Started: Fast", "[0.0s] Finished: Fast in 0.0s (1 of 1 files)"], Lines());
        }

        [Fact]
        public void AStepThatEndsWithoutWorkBeforeItWasAnnouncedIsReportedAsSkippedOnly()
        {
            var (model, reporter) = Create("Cached");
            model.EndPhase(model.BeginPhase("Cached", 0, null));

            reporter.Poll();

            Assert.Equal(["[0.0s] Skipped: Cached"], Lines());
        }

        [Fact]
        public void AStepAnnouncedAsStartedThatThenEndsWithoutCountedWorkStillReportsHowLongItRan()
        {
            var (model, reporter) = Create("Collision");
            int phase = model.BeginPhase("Collision", 0, null);
            reporter.Poll();
            _clock.Advance(TimeSpan.FromSeconds(5));
            model.EndPhase(phase);

            reporter.Poll();

            Assert.Equal(["[0.0s] Started: Collision", "[5.0s] Finished: Collision in 5.0s"], Lines());
        }

        [Fact]
        public void FailedAndCancelledStepsAreReportedAsSuch()
        {
            var (model, reporter) = Create("Broken", "Interrupted");
            model.BeginPhase("Broken", 0, null);
            model.BeginPhase("Interrupted", 0, null);
            _clock.Advance(TimeSpan.FromSeconds(3));
            model.Fail(["Broken"]);
            model.Cancel(["Interrupted"]);

            reporter.Poll();

            Assert.Contains("[3.0s] FAILED: Broken after 3.0s", Lines());
            Assert.Contains("[3.0s] Cancelled: Interrupted after 3.0s", Lines());
        }

        [Fact]
        public void AStepThatNeverStartedBeforeAFailedRunEndedIsReportedAsNotStarted()
        {
            var (model, reporter) = Create("Done", "Never reached");
            Work(model, "Done");
            _clock.Advance(TimeSpan.FromSeconds(2));
            model.Finish(RunState.Failed);

            reporter.Poll();

            Assert.Equal(["[0.0s] Started: Done", "[0.0s] Finished: Done in 0.0s (1 of 1 files)", "[2.0s] Not started: Never reached"], Lines());
        }

        [Fact]
        public void AStepWithoutWorkInASuccessfulRunIsReportedAsSkippedWithoutADuration()
        {
            var (model, reporter) = Create("Ran", "Idle");
            Work(model, "Ran");
            _clock.Advance(TimeSpan.FromSeconds(2));
            model.Finish(RunState.Succeeded);

            reporter.Poll();

            Assert.Equal(["[0.0s] Started: Ran", "[0.0s] Finished: Ran in 0.0s (1 of 1 files)", "[2.0s] Skipped: Idle"], Lines());
        }

        [Fact]
        public async Task AnOutputThatFailsStopsTheLinesWithOneWarningAndDisposingDoesNotThrow()
        {
            var warnings = new List<string>();
            var firstWarning = new TaskCompletionSource();
            var model = new ProgressModel([new StepDefinition("A", "files")], _clock);
            var reporter = new StepReporter(model, new BrokenWriter(), message =>
            {
                lock (warnings) warnings.Add(message);
                firstWarning.TrySetResult();
            });
            model.BeginPhase("A", 1, null);
            reporter.Start();
            await firstWarning.Task.WaitAsync(TimeSpan.FromSeconds(10), TestContext.Current.CancellationToken);

            await reporter.DisposeAsync();

            lock (warnings)
            {
                Assert.Single(warnings);
                Assert.Contains("stopped", warnings[0], StringComparison.Ordinal);
            }
        }

        private sealed class BrokenWriter : StringWriter
        {
            public override void WriteLine(string? value) => throw new IOException("the console is closed");
        }

        [Fact]
        public void StepsAppendedWhilePollingAreReported()
        {
            var (model, reporter) = Create("A");
            reporter.Poll();

            model.BeginPhase("Late addition", 0, null);
            reporter.Poll();

            Assert.Equal(["[0.0s] Started: Late addition"], Lines());
        }

        [Fact]
        public async Task DisposingTheReporterReportsTheTransitionsItHadNotPolledYet()
        {
            var (model, reporter) = Create("A");
            reporter.Start();
            Work(model, "A");

            await reporter.DisposeAsync();

            Assert.Equal(["[0.0s] Started: A", "[0.0s] Finished: A in 0.0s (1 of 1 files)"], Lines());
        }
    }
}
