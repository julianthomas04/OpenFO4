using AssetConverter.Pipeline;
using AssetConverter.Progress;
using Xunit;

namespace InstallerTests.Pipeline
{
    /// <summary>The lanes of asset groups: a kept group's lane ends at once without disturbing the lanes behind it.</summary>
    [Collection("Console redirection")]
    public sealed class InstallPipelineGroupLanesTests : IDisposable
    {
        private readonly TextWriter _originalOut = Console.Out;
        private readonly TextWriter _originalError = Console.Error;

        public InstallPipelineGroupLanesTests()
        {
            Console.SetOut(new StringWriter());
            Console.SetError(new StringWriter());
            InstallProgress.Start([new StepDefinition(InstallSteps.ConvertAudio), new StepDefinition(InstallSteps.ExtractInterface)]);
        }

        public void Dispose()
        {
            Console.SetOut(_originalOut);
            Console.SetError(_originalError);
            InstallProgress.Stop();
        }

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private static LaneRunner Lanes() => new(new PhaseLog(), Ct);

        [Fact]
        public async Task AKeptGroupOpensTheGatesItsLaneWouldOpenWithoutRunningItsWork()
        {
            var skeletons = new Gate("skeletons");
            bool ran = false;

            await InstallPipeline.RunGroup(Lanes(), reconverts: false, InstallSteps.MeshLane, () => { ran = true; }, opens: [skeletons]).WithTimeout();

            Assert.True(skeletons.IsOpen);
            Assert.False(ran);
        }

        [Fact]
        public async Task AConvertedGroupRunsItsWorkAndItsLaneEndsOnlyOnceItOpenedItsGates()
        {
            var skeletons = new Gate("skeletons");
            bool ran = false;

            await InstallPipeline.RunGroup(Lanes(), reconverts: true, InstallSteps.MeshLane, () => { ran = true; skeletons.Open(); }, opens: [skeletons]).WithTimeout();

            Assert.True(ran);
            Assert.True(skeletons.IsOpen);
        }

        [Fact]
        public async Task AKeptGroupsLaneWaitsForWhatItsRealLaneWouldWaitFor()
        {
            var inputs = new Gate("inputs");
            var skeletons = new Gate("skeletons");

            var lane = InstallPipeline.RunGroup(Lanes(), reconverts: false, InstallSteps.MeshLane, () => { }, after: [inputs], opens: [skeletons]);

            Assert.False(lane.IsCompleted);
            Assert.False(skeletons.IsOpen);
            inputs.Open();
            await lane.WithTimeout();
            Assert.True(skeletons.IsOpen);
        }

        [Fact]
        public async Task AKeptGroupsLaneNeverMarksItsProgressStepFailedEvenWhenTheLaneEndsInFailure()
        {
            var failure = new InvalidOperationException("an earlier lane failed");

            var thrown = await Assert.ThrowsAsync<InvalidOperationException>(() =>
                InstallPipeline.RunGroup(Lanes(), reconverts: false, [InstallSteps.ConvertAudio], () => { }, after: [Task.FromException(failure)]).WithTimeout());

            Assert.Same(failure, thrown);
            Assert.Equal(StepStatus.Pending, InstallProgress.Model!.GetSnapshot().Steps.Single(step => step.Name == InstallSteps.ConvertAudio).Status);
        }
    }
}
