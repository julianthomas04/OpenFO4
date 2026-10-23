using AssetConverter;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using Xunit;

namespace InstallerTests.Pipeline
{
    /// <summary>The game is copied before any other work starts, so a game that is still running stops the install early.</summary>
    [Collection("Console redirection")]
    public sealed class InstallPipelineGameFirstTests : IDisposable
    {
        private readonly TextWriter _originalError = Console.Error;
        private readonly TempDirectory _root = new();

        public InstallPipelineGameFirstTests() => Console.SetError(new StringWriter());

        public void Dispose()
        {
            Console.SetError(_originalError);
            InstallProgress.Stop();
            _root.Dispose();
        }

        private InstallLocations Locations => new(Path.Combine(_root.Path, "game"), Path.Combine(_root.Path, "install"));

        [Fact]
        public async Task AGameThatCannotBeReplacedFailsTheInstallBeforeAnyOtherStepStarts()
        {
            string source = Path.Combine(_root.Path, "built", GameFiles.ExecutableName);
            Directory.CreateDirectory(Path.GetDirectoryName(source)!);
            File.WriteAllText(source, "new game");
            Directory.CreateDirectory(Locations.InstallRoot);
            File.WriteAllText(Locations.GameExecutable, "old game");
            using var runningGame = new FileStream(Locations.GameExecutable, FileMode.Open, FileAccess.Read, FileShare.Read);
            var options = new InstallOptions(Game: new GameFiles([source], Required: true));
            InstallProgress.Start(InstallSteps.Planned(options.KeepStaging));
            var pipeline = new InstallPipeline(Locations, options, MemoryBudget.Unlimited(), InstallProgress.CancelToken);

            var failure = await Assert.ThrowsAsync<IOException>(pipeline.RunAsync);

            Assert.Contains("is the game running?", failure.Message, StringComparison.Ordinal);
            var steps = InstallProgress.Model!.GetSnapshot().Steps;
            Assert.Equal(StepStatus.Failed, steps.Single(step => step.Name == InstallSteps.Initializing).Status);
            Assert.All(steps.Where(step => step.Name != InstallSteps.Initializing), step => Assert.Equal(StepStatus.Pending, step.Status));
            Assert.Equal("old game", File.ReadAllText(Locations.GameExecutable));
        }
    }
}
