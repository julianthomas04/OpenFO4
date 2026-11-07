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
                progress.GameFileCount(2);
                progress.FileUnpacked();
                progress.FileUnpacked();
                midway = UnpackStep();
                progress.FileUnpacked();
                return PayloadWithGameFiles(2);
            });

            Assert.Equal(_root.Path, resources.Root);
            Assert.Equal(StepStatus.Running, midway!.Status);
            Assert.Equal((2L, 3L), (midway.Completed, midway.Total));
            Assert.Equal(StepStatus.Done, UnpackStep().Status);
            Assert.Equal(3, UnpackStep().Completed);
        }

        private static StepSnapshot InitializingStep() => InstallProgress.Model!.GetSnapshot().Steps.Single(step => step.Name == InstallSteps.Initializing);

        private static StepSnapshot GamePart() => InitializingStep().Parts.Single(part => part.Name == InstallSteps.InstallGame);

        /// <summary>A payload folder whose game folder holds the given number of files, found by <see cref="InstallerResources.ForPayload"/>.</summary>
        private InstallerResources PayloadWithGameFiles(int count)
        {
            string game = Path.Combine(_root.Path, InstallerResources.GameFolder);
            Directory.CreateDirectory(game);
            for (int i = 0; i < count; i++) File.WriteAllText(Path.Combine(game, $"file{i}.dll"), "x");
            return InstallerResources.ForPayload(_root.Path);
        }

        [Fact]
        public async Task TheGameFileCountFromThePayloadListingGivesInitializingItsFinalTotalBeforeAnythingIsUnpacked()
        {
            InstallProgress.Start(InstallSteps.Planned(keepStaging: false, unpacksPayload: true));
            StepSnapshot? beforeTheListing = null;
            StepSnapshot? afterTheListing = null;

            await PayloadUnpackStep.RunAsync(packaged: true, progress =>
            {
                beforeTheListing = InitializingStep();
                progress!.FileCount(5);
                progress.GameFileCount(2);
                afterTheListing = InitializingStep();
                return PayloadWithGameFiles(2);
            });

            Assert.True(beforeTheListing!.IsPreparing);
            Assert.False(afterTheListing!.IsPreparing);
            Assert.Equal(5 + 2 + InstallSteps.CopyGameDataTotal, afterTheListing.Total);
            Assert.Equal(2, GamePart().Total);
        }

        [Fact]
        public async Task APayloadUnpackedByAnEarlierRunDeclaresTheGameFilesItFindsInstead()
        {
            InstallProgress.Start(InstallSteps.Planned(keepStaging: false, unpacksPayload: true));

            await PayloadUnpackStep.RunAsync(packaged: true, _ => PayloadWithGameFiles(4));

            Assert.Equal(4, GamePart().Total);
            Assert.False(InitializingStep().IsPreparing);
        }

        [Fact]
        public async Task AnInstallerWithoutAPayloadDeclaresTheGameFilesItHasSoTheGameStepBeginsWithThatTotal()
        {
            InstallProgress.Start(InstallSteps.Planned(keepStaging: false));

            await PayloadUnpackStep.RunAsync(packaged: false, _ => PayloadWithGameFiles(3));
            int phase = InstallProgress.BeginPhase(InstallSteps.InstallGame, 3);

            Assert.Equal(3, GamePart().Total);
            Assert.True(phase >= 0);
        }

        [Fact]
        public async Task AListingThatDisagreesWithTheUnpackedGameFilesIsRefusedInsteadOfShowingAWrongTotal()
        {
            InstallProgress.Start(InstallSteps.Planned(keepStaging: false, unpacksPayload: true));

            var thrown = await Assert.ThrowsAsync<InstallStartupException>(() => PayloadUnpackStep.RunAsync(packaged: true, progress =>
            {
                progress!.GameFileCount(2);
                return PayloadWithGameFiles(3);
            }));

            Assert.Contains("lists 2 game files, but 3 were unpacked", thrown.Message, StringComparison.Ordinal);
            Assert.Equal(StepStatus.Failed, UnpackStep().Status);
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
            UnpackProgress? given = new(_ => { }, _ => { }, () => { });

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
