using AssetConverter.Progress;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests.Progress
{
    public sealed class StepGroupingTests
    {
        private const string Group = "Setting up";
        private readonly ManualTimeProvider _clock = new();

        private ProgressModel Model() => new(
        [
            new StepDefinition("First", "files", Weight: 1, Group: Group),
            new StepDefinition("Second", "tasks", Weight: 3, Group: Group),
            new StepDefinition("Alone", "items", Weight: 4),
            new StepDefinition("Late", "tasks", Weight: 2, Group: Group),
        ], _clock);

        private static StepSnapshot GroupOf(ProgressModel model) => model.GetSnapshot().Steps.Single(s => s.Name == Group);

        [Fact]
        public void UnitsOfAGroupAreShownAsOneStepAtThePositionOfTheFirst()
        {
            var steps = Model().GetSnapshot().Steps;

            Assert.Equal([Group, "Alone"], steps.Select(s => s.Name));
            Assert.Equal(["First", "Second", "Late"], steps[0].Parts.Select(p => p.Name));
            Assert.Empty(steps[1].Parts);
        }

        [Fact]
        public void TheGroupRunsWhileAnyUnitRunsAndAddsUpTheCounters()
        {
            var model = Model();
            int first = model.BeginPhase("First", 10, null);
            model.Tick(first, 4);
            int second = model.BeginPhase("Second", 5, null);
            model.Tick(second, 1);

            var group = GroupOf(model);

            Assert.Equal(StepStatus.Running, group.Status);
            Assert.Equal((5L, 15L), (group.Completed, group.Total));
            Assert.Equal("items", group.Unit);
        }

        [Fact]
        public void TheGroupIsDoneOnlyWhenEveryUnitIsDoneAndStaysRunningBetweenUnits()
        {
            var model = Model();
            foreach (var name in new[] { "First", "Second" })
            {
                int phase = model.BeginPhase(name, 1, null);
                model.Tick(phase);
                model.EndPhase(phase);
            }

            Assert.Equal(StepStatus.Running, GroupOf(model).Status);

            int last = model.BeginPhase("Late", 1, null);
            model.Tick(last);
            model.EndPhase(last);

            var group = GroupOf(model);
            Assert.Equal(StepStatus.Done, group.Status);
            Assert.Equal(1.0, group.Fraction);
            Assert.NotNull(group.FinishedAt);
        }

        [Fact]
        public void TheFractionIsTheWeightedFractionOfTheUnitsSoUnitsNotBegunKeepTheirShare()
        {
            var model = Model();
            int first = model.BeginPhase("First", 10, null);
            model.Tick(first, 10);
            model.EndPhase(first);

            Assert.Equal(1.0 / 6.0, GroupOf(model).Fraction, precision: 6);
        }

        [Fact]
        public void AFailureOfOneUnitFailsTheGroupAndACancellationCancelsIt()
        {
            var model = Model();
            model.BeginPhase("Second", 1, null);
            model.Fail(["Second"]);

            Assert.Equal(StepStatus.Failed, GroupOf(model).Status);

            var cancelled = Model();
            cancelled.BeginPhase("Late", 1, null);
            cancelled.Cancel(["Late"]);

            Assert.Equal(StepStatus.Cancelled, GroupOf(cancelled).Status);
        }

        [Fact]
        public void UnitsRunInParallelAndTheGroupTimesTheirUnion()
        {
            var model = Model();
            int first = model.BeginPhase("First", 1, null);
            _clock.Advance(TimeSpan.FromSeconds(10));
            int late = model.BeginPhase("Late", 1, null);
            _clock.Advance(TimeSpan.FromSeconds(5));
            model.EndPhase(first);

            var group = GroupOf(model);

            Assert.Equal(TimeSpan.FromSeconds(15), group.Elapsed);
            Assert.Equal(StepStatus.Running, group.Status);
            model.EndPhase(late);
        }

        [Fact]
        public void TheOverallProgressStillCountsEveryUnitByItsOwnWeight()
        {
            var model = Model();
            int alone = model.BeginPhase("Alone", 1, null);
            model.Tick(alone);
            model.EndPhase(alone);

            Assert.Equal(0.4, model.GetSnapshot().OverallFraction, precision: 6);
        }
    }
}
