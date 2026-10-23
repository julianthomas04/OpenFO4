using AssetConverter;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using Xunit;

namespace InstallerTests.Pipeline
{
    [Collection("Console redirection")]
    public sealed class PayloadUnpackStepTests : IDisposable
    {
        private readonly TextWriter _originalError = Console.Error;
        private readonly TempDirectory _root = new();

        public PayloadUnpackStepTests() => Console.SetError(new StringWriter());

        public void Dispose()
        {
            Console.SetError(_originalError);
            InstallProgress.Stop();
            _root.Dispose();
        }

        private InstallerResources Resources => InstallerResources.ForRepository(_root.Path);

        private static StepSnapshot UnpackStep() => InstallProgress.Model!.GetSnapshot().Steps.Single(step => step.Name == InstallSteps.Initializing).Parts.Single(part => part.Name == InstallSteps.UnpackPayload);

        [Fact]
        public async Task ThePackagedInstallersUnpackingShowsTheFileCountAndEachFileAsItsStepAndReturnsTheResources()
        {
            InstallProgress.Start(InstallSteps.Planned(keepStaging: false, unpacksPayload: true));
            StepSnapshot? midway = null;

            var resources = await PayloadUnpackStep.RunAsync(packaged: true, progress =>
            {
                progress!.FileCount(3);
                progress.FileUnpacked();
                progress.FileUnpacked();
                midway = UnpackStep();
                progress.FileUnpacked();
                return Resources;
            });

            Assert.Equal(_root.Path, resources.Root);
            Assert.Equal(StepStatus.Running, midway!.Status);
            Assert.Equal((2L, 3L), (midway.Completed, midway.Total));
            Assert.Equal(StepStatus.Done, UnpackStep().Status);
            Assert.Equal(3, UnpackStep().Completed);
        }

        [Fact]
        public async Task AnUnpackingThatFailsFailsTheStepAndPassesTheReasonOn()
        {
            InstallProgress.Start(InstallSteps.Planned(keepStaging: false, unpacksPayload: true));
            var reason = new InstallStartupException("The installer could not unpack its files into 'x': disk full");

            var thrown = await Assert.ThrowsAsync<InstallStartupException>(() => PayloadUnpackStep.RunAsync(packaged: true, _ => throw reason));

            Assert.Same(reason, thrown);
            Assert.Equal(StepStatus.Failed, UnpackStep().Status);
            Assert.Equal(RunState.Failed, InstallProgress.Conclude(thrown));
        }

        [Fact]
        public async Task AnInstallerWithoutAPayloadHasNothingToUnpackAndNoStep()
        {
            InstallProgress.Start(InstallSteps.Planned(keepStaging: false));
            UnpackProgress? given = new(_ => { }, () => { });

            var resources = await PayloadUnpackStep.RunAsync(packaged: false, progress =>
            {
                given = progress;
                return Resources;
            });

            Assert.Null(given);
            Assert.Equal(_root.Path, resources.Root);
            Assert.DoesNotContain(InstallProgress.Model!.GetSnapshot().Steps.SelectMany(step => step.Parts), part => part.Name == InstallSteps.UnpackPayload);
        }
    }
}
