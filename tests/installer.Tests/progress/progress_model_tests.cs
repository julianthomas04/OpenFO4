using AssetConverter.Progress;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests.Progress
{
    public class ProgressModelTests
    {
        private readonly ManualTimeProvider _clock = new();

        private ProgressModel Model(params StepDefinition[] steps) => new(steps, _clock);

        private static StepDefinition Step(string name, double weight = 1, string? unit = null) => new(name, unit, weight);

        private static StepSnapshot Find(ProgressSnapshot snapshot, string name) => snapshot.Steps.Single(s => s.Name == name);

        [Fact]
        public void DeclaredStepsStartPendingWithoutProgress()
        {
            var snapshot = Model(Step("A"), Step("B")).GetSnapshot();

            Assert.All(snapshot.Steps, s => Assert.Equal(StepStatus.Pending, s.Status));
            Assert.Equal(0, snapshot.OverallFraction);
            Assert.Equal(RunState.Running, snapshot.State);
        }

        [Fact]
        public void APhaseBindsToTheDeclaredStepOfTheSameNameAndStartsIt()
        {
            var model = Model(Step("Converting meshes", unit: "meshes"));

            int phase = model.BeginPhase("converting MESHES", 63_000, null);

            var step = model.GetSnapshot().Steps.Single();
            Assert.Equal(0, phase);
            Assert.Equal(StepStatus.Running, step.Status);
            Assert.Equal(63_000, step.Total);
            Assert.Equal("meshes", step.Unit);
        }

        [Fact]
        public void AnUndeclaredPhaseNameAppendsAStep()
        {
            var model = Model(Step("A"));

            model.BeginPhase("Extra", 10, "things");

            var snapshot = model.GetSnapshot();
            Assert.Equal(["A", "Extra"], snapshot.Steps.Select(s => s.Name));
            Assert.Equal("things", snapshot.Steps[1].Unit);
            Assert.Equal(StepStatus.Running, snapshot.Steps[1].Status);
        }

        [Fact]
        public void TicksAreCountedWithoutAnyObserver()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", 100, null);

            model.Tick(phase);
            model.Tick(phase, 9);
            model.AddTotal(phase, 50);

            var step = model.GetSnapshot().Steps.Single();
            Assert.Equal(10, step.Completed);
            Assert.Equal(150, step.Total);
        }

        [Theory]
        [InlineData(-1)]
        [InlineData(7)]
        public void CallsWithAnUnknownPhaseIdAreIgnored(int phase)
        {
            var model = Model(Step("A"));
            model.BeginPhase("A", 10, null);

            model.Tick(phase);
            model.AddTotal(phase, 5);
            model.SetDetail(phase, "x");
            model.EndPhase(phase);

            var step = model.GetSnapshot().Steps.Single();
            Assert.Equal(0, step.Completed);
            Assert.Equal(10, step.Total);
            Assert.Null(step.Detail);
            Assert.Equal(StepStatus.Running, step.Status);
        }

        [Fact]
        public void TheDetailTextIsReportedAndReplaced()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", 0, null);

            model.SetDetail(phase, "Fallout4 - Meshes.ba2");
            model.SetDetail(phase, "Fallout4 - Textures1.ba2");

            Assert.Equal("Fallout4 - Textures1.ba2", model.GetSnapshot().Steps.Single().Detail);
        }

        [Fact]
        public void EndingThePhaseFinishesTheStepAtFullFractionRegardlessOfTheCounters()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", 100, null);
            model.Tick(phase, 40);

            model.EndPhase(phase);

            var step = model.GetSnapshot().Steps.Single();
            Assert.Equal(StepStatus.Done, step.Status);
            Assert.Equal(1.0, step.Fraction);
        }

        [Fact]
        public void EndingAPhaseTwiceChangesNothing()
        {
            var model = Model(Step("A"));
            int first = model.BeginPhase("A", 0, null);
            model.BeginPhase("A", 0, null);

            model.EndPhase(first);
            model.EndPhase(first);

            Assert.Equal(StepStatus.Running, model.GetSnapshot().Steps.Single().Status);
        }

        [Fact]
        public void AStepWithTwoPhasesIsDoneOnlyWhenBothHaveEnded()
        {
            var model = Model(Step("Building database"));
            int records = model.BeginPhase("Building database", 100, null);
            int cells = model.BeginPhase("Building database", 50, null);
            model.Tick(records, 100);
            model.Tick(cells, 25);

            model.EndPhase(records);
            Assert.Equal(StepStatus.Running, model.GetSnapshot().Steps.Single().Status);
            model.EndPhase(cells);

            var step = model.GetSnapshot().Steps.Single();
            Assert.Equal(StepStatus.Done, step.Status);
            Assert.Equal(125, step.Completed);
            Assert.Equal(150, step.Total);
        }

        [Fact]
        public void ALaterPhaseReopensAFinishedStepAndKeepsItsStartTime()
        {
            var model = Model(Step("A"));
            _clock.Advance(TimeSpan.FromSeconds(2));
            model.EndPhase(model.BeginPhase("A", 0, null));
            _clock.Advance(TimeSpan.FromSeconds(3));

            model.BeginPhase("A", 0, null);

            var step = model.GetSnapshot().Steps.Single();
            Assert.Equal(StepStatus.Running, step.Status);
            Assert.Equal(TimeSpan.FromSeconds(2), step.StartedAt);
            Assert.Null(step.FinishedAt);
        }

        [Fact]
        public void AStepWithoutATotalIsIndeterminate()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", 0, null);
            model.Tick(phase, 5);

            var step = model.GetSnapshot().Steps.Single();

            Assert.True(step.IsIndeterminate);
            Assert.Equal(0, step.Fraction);
        }

        [Fact]
        public void TheOverallFractionIsTheWeightedAverageOfTheStepFractions()
        {
            var model = Model(Step("Small", weight: 1), Step("Big", weight: 3));
            model.EndPhase(model.BeginPhase("Small", 0, null));
            int big = model.BeginPhase("Big", 200, null);
            model.Tick(big, 100);

            Assert.Equal((1.0 * 1 + 3.0 * 0.5) / 4, model.GetSnapshot().OverallFraction, precision: 10);
        }

        [Fact]
        public void TheOverallFractionNeverGoesBackwardsWhenTotalsGrow()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", 100, null);
            model.Tick(phase, 100);
            double before = model.GetSnapshot().OverallFraction;

            model.AddTotal(phase, 900);
            var after = model.GetSnapshot();

            Assert.Equal(ProgressModel.RunningFractionCap, before);
            Assert.Equal(before, after.OverallFraction);
            Assert.Equal(0.1, after.Steps.Single().Fraction, precision: 10);
        }

        [Fact]
        public void ARunningStepNeverReadsAsCompleteUntilItEnds()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", 10, null);

            model.Tick(phase, 25);
            double whileRunning = model.GetSnapshot().Steps.Single().Fraction;
            model.EndPhase(phase);

            Assert.Equal(ProgressModel.RunningFractionCap, whileRunning);
            Assert.Equal(1.0, model.GetSnapshot().Steps.Single().Fraction);
        }

        [Fact]
        public void AFinishedStepKeepsItsCountersWhenTheTotalGrowsLate()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", 9_500, null);
            model.Tick(phase, 9_500);
            model.EndPhase(phase);

            model.AddTotal(phase, 500);
            model.Tick(phase, 3);
            var step = model.GetSnapshot().Steps.Single();

            Assert.Equal((9_500L, 9_500L), (step.Completed, step.Total));
        }

        [Fact]
        public void AStepThatIsReopenedCountsOnFromItsFinishedCounters()
        {
            var model = Model(Step("A"));
            int first = model.BeginPhase("A", 10, null);
            model.Tick(first, 10);
            model.EndPhase(first);

            int second = model.BeginPhase("A", 5, null);
            model.Tick(second, 2);
            var step = model.GetSnapshot().Steps.Single();

            Assert.Equal((12L, 15L), (step.Completed, step.Total));
        }

        [Fact]
        public void AReopenedStepKeepsItsOriginalStartAndItsElapsedTimeRunsOnAgain()
        {
            var model = Model(Step("A"));
            _clock.Advance(TimeSpan.FromSeconds(1));
            int first = model.BeginPhase("A", 0, null);
            _clock.Advance(TimeSpan.FromSeconds(4));
            model.EndPhase(first);
            var finished = model.GetSnapshot().Steps.Single();

            _clock.Advance(TimeSpan.FromSeconds(10));
            model.BeginPhase("A", 0, null);
            _clock.Advance(TimeSpan.FromSeconds(2));
            var reopened = model.GetSnapshot().Steps.Single();

            Assert.Equal(TimeSpan.FromSeconds(4), finished.Elapsed);
            Assert.Equal(TimeSpan.FromSeconds(5), finished.FinishedAt);
            Assert.Equal(StepStatus.Running, reopened.Status);
            Assert.Equal(TimeSpan.FromSeconds(1), reopened.StartedAt);
            Assert.Null(reopened.FinishedAt);
            Assert.Equal(TimeSpan.FromSeconds(16), reopened.Elapsed);
        }

        [Fact]
        public void StepsThatNeverRanInASuccessfulRunAreSkippedWithoutAStartTime()
        {
            var model = Model(Step("Ran"), Step("Idle"));
            model.EndPhase(model.BeginPhase("Ran", 1, null));
            _clock.Advance(TimeSpan.FromSeconds(4));

            model.Finish(RunState.Succeeded);
            var idle = Find(model.GetSnapshot(), "Idle");

            Assert.Equal(StepStatus.Done, idle.Status);
            Assert.True(idle.WasSkipped);
            Assert.Null(idle.StartedAt);
            Assert.Equal(TimeSpan.Zero, idle.Elapsed);
            Assert.False(Find(model.GetSnapshot(), "Ran").WasSkipped);
        }

        [Fact]
        public void StepTimesFollowTheClock()
        {
            var model = Model(Step("A"));
            _clock.Advance(TimeSpan.FromSeconds(5));
            int phase = model.BeginPhase("A", 0, null);
            _clock.Advance(TimeSpan.FromSeconds(7));
            Assert.Equal(TimeSpan.FromSeconds(7), model.GetSnapshot().Steps.Single().Elapsed);

            model.EndPhase(phase);
            _clock.Advance(TimeSpan.FromSeconds(100));

            var snapshot = model.GetSnapshot();
            var step = snapshot.Steps.Single();
            Assert.Equal(TimeSpan.FromSeconds(5), step.StartedAt);
            Assert.Equal(TimeSpan.FromSeconds(12), step.FinishedAt);
            Assert.Equal(TimeSpan.FromSeconds(7), step.Elapsed);
            Assert.Equal(TimeSpan.FromSeconds(112), snapshot.Elapsed);
        }

        [Fact]
        public void FailAndCancelOnlyTouchTheNamedRunningSteps()
        {
            var model = Model(Step("Failing"), Step("Interrupted"), Step("Bystander"), Step("Waiting"));
            model.BeginPhase("Failing", 0, null);
            model.BeginPhase("Interrupted", 0, null);
            model.EndPhase(model.BeginPhase("Bystander", 0, null));

            model.Fail(["Failing", "Bystander", "Waiting"]);
            model.Cancel(["Interrupted"]);

            var snapshot = model.GetSnapshot();
            Assert.Equal(StepStatus.Failed, Find(snapshot, "Failing").Status);
            Assert.Equal(StepStatus.Cancelled, Find(snapshot, "Interrupted").Status);
            Assert.Equal(StepStatus.Done, Find(snapshot, "Bystander").Status);
            Assert.Equal(StepStatus.Pending, Find(snapshot, "Waiting").Status);
            Assert.NotNull(Find(snapshot, "Failing").FinishedAt);
        }

        [Fact]
        public void AFailedStepStaysFailedWhenItsPhaseEndsLater()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", 0, null);
            model.Fail(["A"]);

            model.EndPhase(phase);

            Assert.Equal(StepStatus.Failed, model.GetSnapshot().Steps.Single().Status);
        }

        [Fact]
        public void SuccessMakesEveryStepDoneIncludingOnesThatHadNothingToDo()
        {
            var model = Model(Step("Worked"), Step("Skipped"));
            model.EndPhase(model.BeginPhase("Worked", 0, null));

            model.Finish(RunState.Succeeded);

            var snapshot = model.GetSnapshot();
            Assert.All(snapshot.Steps, s => Assert.Equal(StepStatus.Done, s.Status));
            Assert.Equal(1.0, snapshot.OverallFraction);
            Assert.Equal(RunState.Succeeded, snapshot.State);
            Assert.True(snapshot.IsFinished);
        }

        [Theory]
        [InlineData(RunState.Failed)]
        [InlineData(RunState.Cancelled)]
        public void AnUnsuccessfulEndCancelsRunningAndPendingStepsAndKeepsFinishedOnes(RunState outcome)
        {
            var model = Model(Step("Done"), Step("Running"), Step("Pending"));
            model.EndPhase(model.BeginPhase("Done", 0, null));
            model.BeginPhase("Running", 0, null);

            model.Finish(outcome);

            var snapshot = model.GetSnapshot();
            Assert.Equal(StepStatus.Done, Find(snapshot, "Done").Status);
            Assert.Equal(StepStatus.Cancelled, Find(snapshot, "Running").Status);
            Assert.Equal(StepStatus.Cancelled, Find(snapshot, "Pending").Status);
            Assert.Equal(outcome, snapshot.State);
        }

        [Fact]
        public void StepsNeverReachedBeforeAnUnsuccessfulEndHaveNoStartTime()
        {
            var model = Model(Step("Reached"), Step("Never reached"));
            model.BeginPhase("Reached", 0, null);

            model.Finish(RunState.Failed);

            var snapshot = model.GetSnapshot();
            Assert.NotNull(Find(snapshot, "Reached").StartedAt);
            Assert.Null(Find(snapshot, "Never reached").StartedAt);
            Assert.Equal(TimeSpan.Zero, Find(snapshot, "Never reached").Elapsed);
        }

        [Fact]
        public void AnAlreadyFailedStepKeepsItsStatusWhenTheRunEnds()
        {
            var model = Model(Step("A"));
            model.BeginPhase("A", 0, null);
            model.Fail(["A"]);

            model.Finish(RunState.Failed);

            Assert.Equal(StepStatus.Failed, model.GetSnapshot().Steps.Single().Status);
        }

        [Fact]
        public void AFinishedRunIsFrozen()
        {
            var model = Model(Step("A"));
            model.BeginPhase("A", 0, null);
            _clock.Advance(TimeSpan.FromSeconds(4));
            model.Finish(RunState.Cancelled);
            _clock.Advance(TimeSpan.FromSeconds(60));

            int lateOne = model.BeginPhase("A", 0, null);
            model.Finish(RunState.Succeeded);

            var snapshot = model.GetSnapshot();
            Assert.Equal(-1, lateOne);
            Assert.Equal(RunState.Cancelled, snapshot.State);
            Assert.Equal(TimeSpan.FromSeconds(4), snapshot.Elapsed);
        }

        [Fact]
        public void ARunCannotFinishAsRunning()
        {
            Assert.Throws<ArgumentException>(() => Model(Step("A")).Finish(RunState.Running));
        }

        [Fact]
        public void ConcurrentTicksAreAllCounted()
        {
            var model = Model(Step("A"), Step("B"));
            int a = model.BeginPhase("A", 0, null);
            int b = model.BeginPhase("B", 0, null);
            const int threads = 8;
            const int ticksPerThread = 50_000;

            Parallel.For(0, threads, new ParallelOptions { MaxDegreeOfParallelism = threads }, worker =>
            {
                for (int i = 0; i < ticksPerThread; i++)
                {
                    model.Tick(worker % 2 == 0 ? a : b);
                    if (i % 5000 == 0) model.GetSnapshot();
                }
            });

            var snapshot = model.GetSnapshot();
            Assert.Equal(threads / 2 * ticksPerThread, Find(snapshot, "A").Completed);
            Assert.Equal(threads / 2 * ticksPerThread, Find(snapshot, "B").Completed);
        }

        [Fact]
        public void PhasesCanBeginAndEndConcurrentlyWithTicksAndSnapshots()
        {
            var model = Model(Step("A"));
            int ticker = model.BeginPhase("A", 0, null);
            const int phases = 200;

            Parallel.For(0, phases, i =>
            {
                int phase = model.BeginPhase($"Dynamic {i % 10}", 10, null);
                model.Tick(phase, 10);
                model.Tick(ticker);
                model.GetSnapshot();
                model.EndPhase(phase);
            });

            var snapshot = model.GetSnapshot();
            Assert.Equal(phases, Find(snapshot, "A").Completed);
            Assert.Equal(11, snapshot.Steps.Count);
            Assert.All(snapshot.Steps.Skip(1), s => Assert.Equal(StepStatus.Done, s.Status));
        }

        [Fact]
        public void DuplicateStepNamesAndNonPositiveWeightsAreRejected()
        {
            Assert.Throws<ArgumentException>(() => Model(Step("A"), Step("a")));
            Assert.Throws<ArgumentException>(() => Model(Step("A", weight: 0)));
            Assert.Throws<ArgumentException>(() => Model(Step("A", weight: double.NaN)));
            Assert.Throws<ArgumentException>(() => Model(Step("A", weight: double.PositiveInfinity)));
        }

        [Fact]
        public void AStepWhosePhasesEndedWithoutATotalOrAnyTickIsSkippedButKeepsItsTimes()
        {
            var model = Model(Step("Cached"), Step("Worked"));
            model.EndPhase(model.BeginPhase("Cached", 0, null));
            int worked = model.BeginPhase("Worked", 0, null);
            model.AddTotal(worked, 2);
            model.Tick(worked, 2);
            model.EndPhase(worked);

            var snapshot = model.GetSnapshot();

            Assert.True(Find(snapshot, "Cached").WasSkipped);
            Assert.NotNull(Find(snapshot, "Cached").StartedAt);
            Assert.False(Find(snapshot, "Worked").WasSkipped);
        }
    }
}
