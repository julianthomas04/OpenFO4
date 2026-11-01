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
            int phase = model.BeginPhase("A", null, null);

            model.AddTotal(phase, 100);
            model.AddTotal(phase, 50);
            model.SealTotal(phase);
            model.Tick(phase);
            model.Tick(phase, 9);

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
            model.SealTotal(phase);
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
        public void AStepThatIsStillDiscoveringItsTotalIsPreparingWithoutAFraction()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", null, null);
            model.AddTotal(phase, 40);

            var step = model.GetSnapshot().Steps.Single();

            Assert.True(step.IsPreparing);
            Assert.False(step.TotalIsFinal);
            Assert.Equal(StepStatus.Running, step.Status);
            Assert.Equal(0, step.Fraction);
        }

        [Fact]
        public void SealingTheTotalEndsPreparingAndLetsTheStepFill()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", null, null);
            model.AddTotal(phase, 40);

            model.SealTotal(phase);
            model.Tick(phase, 10);

            var step = model.GetSnapshot().Steps.Single();
            Assert.False(step.IsPreparing);
            Assert.True(step.TotalIsFinal);
            Assert.Equal(0.25, step.Fraction, precision: 10);
        }

        [Fact]
        public void APhaseBegunWithANumberIsSealedAtOnce()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", 10, null);

            model.Tick(phase, 5);

            var step = model.GetSnapshot().Steps.Single();
            Assert.False(step.IsPreparing);
            Assert.Equal(0.5, step.Fraction, precision: 10);
        }

        [Fact]
        public void EndingADiscoveringPhaseSealsItImplicitly()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", null, null);

            model.EndPhase(phase);

            var step = model.GetSnapshot().Steps.Single();
            Assert.False(step.IsPreparing);
            Assert.True(step.TotalIsFinal);
            Assert.Equal(StepStatus.Done, step.Status);
            Assert.True(step.WasSkipped);
            Assert.Throws<InvalidOperationException>(() => model.AddTotal(phase, 1));
        }

        [Fact]
        public void SealingTwiceChangesNothing()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", null, null);
            model.AddTotal(phase, 4);

            model.SealTotal(phase);
            model.SealTotal(phase);

            Assert.False(model.GetSnapshot().Steps.Single().IsPreparing);
        }

        [Fact]
        public void AStepStaysPreparingWhileAnyOfItsPhasesIsStillDiscovering()
        {
            var model = Model(Step("A"));
            int discovering = model.BeginPhase("A", null, null);
            int sealedPhase = model.BeginPhase("A", 10, null);

            Assert.True(model.GetSnapshot().Steps.Single().IsPreparing);
            model.SealTotal(discovering);

            Assert.False(model.GetSnapshot().Steps.Single().IsPreparing);
            model.EndPhase(sealedPhase);
        }

        [Fact]
        public void AddingTotalToASealedPhaseThrows()
        {
            var model = Model(Step("A"));
            int discovered = model.BeginPhase("A", null, null);
            int numbered = model.BeginPhase("A", 10, null);
            model.SealTotal(discovered);

            Assert.Throws<InvalidOperationException>(() => model.AddTotal(numbered, 1));
            Assert.Throws<InvalidOperationException>(() => model.AddTotal(discovered, 1));
            Assert.Equal(10, model.GetSnapshot().Steps.Single().Total);
        }

        [Fact]
        public void TickingAPhaseThatIsStillDiscoveringThrowsAndCountsNothing()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", null, null);

            Assert.Throws<InvalidOperationException>(() => model.Tick(phase));

            Assert.Equal(0, model.GetSnapshot().Steps.Single().Completed);
        }

        [Fact]
        public void AFinishedStepKeepsItsCountersAndRejectsLateWork()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", 9_500, null);
            model.Tick(phase, 9_500);
            model.EndPhase(phase);

            Assert.Throws<InvalidOperationException>(() => model.AddTotal(phase, 500));
            model.Tick(phase, 3);
            var step = model.GetSnapshot().Steps.Single();

            Assert.Equal((9_500L, 9_500L), (step.Completed, step.Total));
        }

        [Fact]
        public void ANegativeTotalIsRejected()
        {
            Assert.Throws<ArgumentOutOfRangeException>(() => Model(Step("A")).BeginPhase("A", -1, null));
        }

        [Fact]
        public void ADeclaredTotalIsFinalBeforeTheStepBeginsAndTheStepMustBeginWithIt()
        {
            var model = Model(Step("A"));
            model.Declare("A", 6);

            var pending = model.GetSnapshot().Steps.Single();
            Assert.Equal(StepStatus.Pending, pending.Status);
            Assert.True(pending.TotalIsFinal);
            Assert.False(pending.IsPreparing);
            Assert.Equal(6, pending.Total);

            int phase = model.BeginPhase("A", 6, null);
            model.Tick(phase, 3);
            var running = model.GetSnapshot().Steps.Single();
            Assert.Equal(6, running.Total);
            Assert.Equal(0.5, running.Fraction, precision: 10);
        }

        [Fact]
        public void ABeginThatContradictsTheDeclaredTotalThrows()
        {
            var model = Model(Step("A"), Step("B"));
            model.Declare("A", 6);
            model.Declare("B", 2);

            Assert.Throws<InvalidOperationException>(() => model.BeginPhase("A", 5, null));
            Assert.Throws<InvalidOperationException>(() => model.BeginPhase("B", null, null));
            Assert.All(model.GetSnapshot().Steps, s => Assert.Equal(StepStatus.Pending, s.Status));
        }

        [Fact]
        public void DeclaringIsRejectedForUnknownStepsStepsThatBegunAndConflictingTotals()
        {
            var model = Model(Step("A"), Step("B"));
            model.BeginPhase("A", 1, null);
            model.Declare("B", 3);

            Assert.Throws<ArgumentException>(() => model.Declare("Nope", 1));
            Assert.Throws<InvalidOperationException>(() => model.Declare("A", 1));
            Assert.Throws<InvalidOperationException>(() => model.Declare("B", 4));
            model.Declare("B", 3);
        }

        [Fact]
        public void AKnownTotalOfTheDefinitionActsLikeADeclaration()
        {
            var model = Model(new StepDefinition("A", "tasks", 1, null, KnownTotal: 6));

            Assert.True(model.GetSnapshot().Steps.Single().TotalIsFinal);
            Assert.Throws<InvalidOperationException>(() => model.BeginPhase("A", 7, null));
            model.BeginPhase("A", 6, null);
            Assert.Equal(6, model.GetSnapshot().Steps.Single().Total);
        }

        [Fact]
        public void AStepWithoutADeclaredTotalIsNotFinalBeforeItBegins()
        {
            var step = Model(Step("A")).GetSnapshot().Steps.Single();

            Assert.False(step.TotalIsFinal);
            Assert.False(step.IsPreparing);
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
        public void ADoneStepCannotBeReopened()
        {
            var model = Model(Step("A"));
            model.EndPhase(model.BeginPhase("A", 10, null));

            Assert.Throws<InvalidOperationException>(() => model.BeginPhase("A", 5, null));
            Assert.Throws<InvalidOperationException>(() => model.BeginPhase("A", null, null));

            Assert.Equal(StepStatus.Done, model.GetSnapshot().Steps.Single().Status);
        }

        [Fact]
        public void AStepThatHasMadeProgressCannotChangeItsTotalWithAnotherPhase()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", 10, null);
            model.Tick(phase, 8);

            Assert.Throws<InvalidOperationException>(() => model.BeginPhase("A", 10, null));
            Assert.Throws<InvalidOperationException>(() => model.BeginPhase("A", null, null));

            var step = model.GetSnapshot().Steps.Single();
            Assert.Equal((8L, 10L), (step.Completed, step.Total));
            Assert.Equal(0.8, step.Fraction, precision: 10);
        }

        [Fact]
        public void AStepWithAFinalTotalCannotGoBackToDiscovering()
        {
            var model = Model(Step("A"));
            model.BeginPhase("A", 10, null);

            Assert.Throws<InvalidOperationException>(() => model.BeginPhase("A", null, null));

            Assert.False(model.GetSnapshot().Steps.Single().IsPreparing);
        }

        [Fact]
        public void ARunningStepReadsNinetyNinePercentWhenEveryUnitIsCountedUntilItEnds()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", 10, null);

            model.Tick(phase, 10);

            var step = model.GetSnapshot().Steps.Single();
            Assert.Equal((10L, 10L), (step.Completed, step.Total));
            Assert.Equal(0.99, step.Fraction);
            Assert.Equal(0.99, ProgressModel.RunningFractionCap);
        }

        [Fact]
        public void TheOverallFractionNeverDecreasesEvenWhenACounterIsCorrectedDownwards()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", 10, null);
            model.Tick(phase, 8);
            double high = model.GetSnapshot().OverallFraction;

            model.Tick(phase, -4);
            var after = model.GetSnapshot();

            Assert.Equal(0.4, after.Steps.Single().Fraction, precision: 10);
            Assert.Equal(0.8, high, precision: 10);
            Assert.Equal(high, after.OverallFraction);
        }

        [Fact]
        public void TheOverallFractionNeverGoesBackwardsFromDiscoveryThroughTheLastTick()
        {
            var model = Model(Step("A"), Step("B"));
            int a = model.BeginPhase("A", null, null);
            var seen = new List<double> { model.GetSnapshot().OverallFraction };

            model.AddTotal(a, 40);
            model.AddTotal(a, 60);
            seen.Add(model.GetSnapshot().OverallFraction);
            model.SealTotal(a);
            for (int i = 0; i < 100; i++)
            {
                model.Tick(a);
                seen.Add(model.GetSnapshot().OverallFraction);
            }
            model.EndPhase(a);
            seen.Add(model.GetSnapshot().OverallFraction);

            Assert.Equal(0, seen[1]);
            Assert.Equal(seen.Order(), seen);
            Assert.Equal(0.5, seen[^1], precision: 10);
        }

        [Fact]
        public void ARunningStepNeverReadsAsCompleteUntilItEnds()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", null, null);
            model.AddTotal(phase, 10);
            model.SealTotal(phase);

            model.Tick(phase, 10);
            double whileRunning = model.GetSnapshot().Steps.Single().Fraction;
            model.EndPhase(phase);

            Assert.Equal(ProgressModel.RunningFractionCap, whileRunning);
            Assert.Equal(1.0, model.GetSnapshot().Steps.Single().Fraction);
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
        public void ConcurrentTicksAfterSealingAreAllCountedWhileSnapshotsReadTheTotal()
        {
            var model = Model(Step("A"));
            int phase = model.BeginPhase("A", null, null);
            const int threads = 8;
            const int ticksPerThread = 25_000;
            model.AddTotal(phase, threads * ticksPerThread);
            model.SealTotal(phase);
            var fractions = new System.Collections.Concurrent.ConcurrentBag<double>();

            Parallel.For(0, threads, new ParallelOptions { MaxDegreeOfParallelism = threads }, _ =>
            {
                for (int i = 0; i < ticksPerThread; i++)
                {
                    model.Tick(phase);
                    if (i % 5000 == 0) fractions.Add(model.GetSnapshot().Steps.Single().Fraction);
                }
            });

            var step = model.GetSnapshot().Steps.Single();
            Assert.Equal(threads * ticksPerThread, step.Completed);
            Assert.Equal(threads * ticksPerThread, step.Total);
            Assert.All(fractions, f => Assert.InRange(f, 0.0, ProgressModel.RunningFractionCap));
        }

        [Fact]
        public void PhasesCanBeginAndEndConcurrentlyWithTicksAndSnapshots()
        {
            var model = Model(Step("A"));
            int ticker = model.BeginPhase("A", 0, null);
            const int phases = 200;

            Parallel.For(0, phases, i =>
            {
                int phase = model.BeginPhase($"Dynamic {i}", 10, null);
                model.Tick(phase, 10);
                model.Tick(ticker);
                model.GetSnapshot();
                model.EndPhase(phase);
            });

            var snapshot = model.GetSnapshot();
            Assert.Equal(phases, Find(snapshot, "A").Completed);
            Assert.Equal(phases + 1, snapshot.Steps.Count);
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
            int worked = model.BeginPhase("Worked", null, null);
            model.AddTotal(worked, 2);
            model.SealTotal(worked);
            model.Tick(worked, 2);
            model.EndPhase(worked);

            var snapshot = model.GetSnapshot();

            Assert.True(Find(snapshot, "Cached").WasSkipped);
            Assert.NotNull(Find(snapshot, "Cached").StartedAt);
            Assert.False(Find(snapshot, "Worked").WasSkipped);
        }
    }
}
