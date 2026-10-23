using AssetConverter.Progress;
using Xunit;

namespace InstallerTests.Progress
{
    public class ProgressTextTests
    {
        private static StepSnapshot Step(string name = "Converting meshes", StepStatus status = StepStatus.Running, long completed = 0, long total = 0,
            string? unit = null, TimeSpan? elapsed = null) =>
            new(name, unit, status, completed, total, null, status == StepStatus.Pending ? null : TimeSpan.Zero, null, elapsed ?? TimeSpan.Zero, 0);

        private static ProgressSnapshot Snapshot(RunState state, params StepSnapshot[] steps) =>
            new(steps, 0, TimeSpan.Zero, state);

        [Fact]
        public void DetailedShowsCompletedOfTotalWithTheUnit()
        {
            Assert.Equal("12,345 of 63,000 meshes", ProgressFormatter.Invariant.Detailed(Step(completed: 12_345, total: 63_000, unit: "meshes")));
        }

        [Fact]
        public void DetailedWithoutATotalShowsOnlyTheCount()
        {
            Assert.Equal("12,345 files", ProgressFormatter.Invariant.Detailed(Step(completed: 12_345, unit: "files")));
            Assert.Equal("", ProgressFormatter.Invariant.Detailed(Step(completed: 0, unit: "files")));
        }

        [Fact]
        public void DetailedWithoutAUnitOmitsIt()
        {
            Assert.Equal("5 of 10", ProgressFormatter.Invariant.Detailed(Step(completed: 5, total: 10)));
        }

        [Fact]
        public void TheOverallLabelCountsDoneAndRunningSteps()
        {
            var snapshot = Snapshot(RunState.Running,
                Step("Extracting", StepStatus.Done), Step("Shaders", StepStatus.Done),
                Step("Converting meshes"), Step("Building database"), Step("Finalizing", StepStatus.Pending));

            Assert.Equal("2 of 5 steps done · 2 running", ProgressText.Overall(snapshot));
        }

        [Fact]
        public void TheOverallLabelBeforeAnythingStartedSaysStarting()
        {
            var snapshot = Snapshot(RunState.Running, Step("A", StepStatus.Pending), Step("B", StepStatus.Pending));

            Assert.Equal("0 of 2 steps done · Starting", ProgressText.Overall(snapshot));
        }

        [Fact]
        public void TheOverallLabelBetweenStepsSaysWorking()
        {
            var snapshot = Snapshot(RunState.Running, Step("A", StepStatus.Done), Step("B", StepStatus.Done));

            Assert.Equal("2 of 2 steps done · Working", ProgressText.Overall(snapshot));
        }

        [Fact]
        public void TheOverallLabelAfterTheRunStatesTheOutcome()
        {
            var steps = new[] { Step("A", StepStatus.Done), Step("Converting meshes", StepStatus.Failed), Step("C", StepStatus.Cancelled) };

            Assert.Equal("Installation complete — 3 of 3 steps", ProgressText.Overall(Snapshot(RunState.Succeeded, steps)));
            Assert.Equal("Installation failed — Converting meshes", ProgressText.Overall(Snapshot(RunState.Failed, steps)));
            Assert.Equal("Installation failed", ProgressText.Overall(Snapshot(RunState.Failed, Step("A", StepStatus.Cancelled))));
            Assert.Equal("Installation cancelled — 1 of 3 steps completed", ProgressText.Overall(Snapshot(RunState.Cancelled, steps)));
        }
    }
}
