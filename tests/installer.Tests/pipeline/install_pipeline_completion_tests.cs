using AssetConverter;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using Xunit;

namespace InstallerTests.Pipeline
{
    /// <summary>What happens to the staging folder once the lanes that do the install have ended.</summary>
    [Collection("Console redirection")]
    public sealed class InstallPipelineCompletionTests : IDisposable
    {
        private readonly TextWriter _originalOut = Console.Out;
        private readonly TextWriter _originalError = Console.Error;
        private readonly StringWriter _output = new();
        private readonly StringWriter _errors = new();
        private readonly TempDirectory _root = new();
        private readonly CancellationTokenSource _cancel = new();

        public InstallPipelineCompletionTests()
        {
            Console.SetOut(_output);
            Console.SetError(_errors);
        }

        public void Dispose()
        {
            Console.SetOut(_originalOut);
            Console.SetError(_originalError);
            InstallProgress.Stop();
            _cancel.Dispose();
            _root.Dispose();
        }

        private InstallLocations Locations => new(Path.Combine(_root.Path, "game"), Path.Combine(_root.Path, "install"));

        private string Marker => Path.Combine(Locations.StagingRoot, ".Fallout4 - Meshes.ba2.extracted");

        private string StagedNif => Path.Combine(Locations.StagedMeshes, "a.nif");

        private InstallPipeline Pipeline(InstallOptions options)
        {
            InstallProgress.Start(InstallSteps.Planned(options.KeepStaging));
            Locations.CreateOutputDirectories();
            Directory.CreateDirectory(Locations.StagedMeshes);
            File.WriteAllText(StagedNif, "nif");
            File.WriteAllText(Marker, "");
            return new InstallPipeline(Locations, options, MemoryBudget.Unlimited(), _cancel.Token);
        }

        private static StepSnapshot RemovalStep() => InstallProgress.Model!.GetSnapshot().Steps.Single(s => s.Name == InstallSteps.RemoveStaging);

        [Fact]
        public async Task AfterASuccessfulInstallTheStagingFolderIsRemovedAndTheStepSaysSo()
        {
            var pipeline = Pipeline(new InstallOptions());

            await pipeline.CompleteAsync([Task.CompletedTask, Task.CompletedTask]);

            Assert.False(Directory.Exists(Locations.StagingRoot));
            Assert.Equal(StepStatus.Done, RemovalStep().Status);
            Assert.Matches(@"Removed temporary files: 2 files and 2 folders in ", _output.ToString());
            Assert.Matches(@"Raw staging\s+removed", _output.ToString());
        }

        [Fact]
        public async Task TheInstalledOutputIsNotTouchedByTheRemoval()
        {
            var pipeline = Pipeline(new InstallOptions());
            Directory.CreateDirectory(Locations.DataOut);
            File.WriteAllText(Locations.DatabasePath, "db");

            await pipeline.CompleteAsync([Task.CompletedTask]);

            Assert.True(File.Exists(Locations.DatabasePath));
        }

        [Fact]
        public async Task WhenToldToKeepThemTheTemporaryFilesStayAndThereIsNoStepForThem()
        {
            var pipeline = Pipeline(new InstallOptions(KeepStaging: true));

            await pipeline.CompleteAsync([Task.CompletedTask]);

            Assert.True(File.Exists(StagedNif));
            Assert.True(File.Exists(Marker));
            Assert.DoesNotContain(InstallProgress.Model!.GetSnapshot().Steps, step => step.Name == InstallSteps.RemoveStaging);
            Assert.DoesNotContain("Removed temporary files", _output.ToString(), StringComparison.Ordinal);
        }

        [Fact]
        public async Task AFailedInstallKeepsTheTemporaryFilesAndTheStepNeverStarts()
        {
            var pipeline = Pipeline(new InstallOptions());
            var failure = new InvalidOperationException("converter exploded");

            var thrown = await Assert.ThrowsAsync<InvalidOperationException>(() => pipeline.CompleteAsync([Task.CompletedTask, Task.FromException(failure)]));

            Assert.Same(failure, thrown);
            Assert.True(File.Exists(StagedNif));
            Assert.True(File.Exists(Marker));
            Assert.Equal(StepStatus.Pending, RemovalStep().Status);
        }

        [Fact]
        public async Task ACancelledInstallKeepsTheTemporaryFiles()
        {
            var pipeline = Pipeline(new InstallOptions());
            await _cancel.CancelAsync();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => pipeline.CompleteAsync([Task.FromCanceled(_cancel.Token)]));

            Assert.True(File.Exists(StagedNif));
            Assert.True(File.Exists(Marker));
        }

        [Fact]
        public async Task AnInstallThatSucceededButCannotRemoveEverythingStillCompletesWithAWarning()
        {
            var pipeline = Pipeline(new InstallOptions());
            using var holder = new FileStream(StagedNif, FileMode.Open, FileAccess.Read, FileShare.None);

            await pipeline.CompleteAsync([Task.CompletedTask]);

            Assert.True(File.Exists(StagedNif));
            Assert.False(File.Exists(Marker));
            Assert.Equal(StepStatus.Done, RemovalStep().Status);
            Assert.Contains("WARNING: 3 temporary files or folders could not be removed", _errors.ToString(), StringComparison.Ordinal);
            Assert.Contains(Locations.StagingRoot, _errors.ToString(), StringComparison.Ordinal);
            Assert.Matches(@"Raw staging\s+1 files", _output.ToString());
        }

        private string? MarkerVersion() => AssetVersionMarker.TryRead(Locations.OwnershipMarker, out var version) ? version : null;

        [Fact]
        public async Task ASuccessfulInstallRecordsTheAssetVersionInTheMarker()
        {
            var pipeline = Pipeline(new InstallOptions(AssetVersion: "3.1.4-alpha"));
            Assert.Null(MarkerVersion());

            await pipeline.CompleteAsync([Task.CompletedTask]);

            Assert.Equal("3.1.4-alpha", MarkerVersion());
        }

        [Fact]
        public async Task ACancelledInstallLeavesTheMarkerEmpty()
        {
            var pipeline = Pipeline(new InstallOptions(AssetVersion: "3.1.4-alpha"));
            await _cancel.CancelAsync();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => pipeline.CompleteAsync([Task.FromCanceled(_cancel.Token)]));

            Assert.Null(MarkerVersion());
        }

        [Fact]
        public async Task AFailedLaneLeavesTheMarkerEmpty()
        {
            var pipeline = Pipeline(new InstallOptions(AssetVersion: "3.1.4-alpha"));

            await Assert.ThrowsAsync<InvalidOperationException>(() => pipeline.CompleteAsync([Task.FromException(new InvalidOperationException("lane failed"))]));

            Assert.Null(MarkerVersion());
        }

        [Fact]
        public async Task TheMarkerIsWrittenBeforeTheRunCanEndAsSucceeded()
        {
            var pipeline = Pipeline(new InstallOptions(AssetVersion: "3.1.4-alpha"));

            await pipeline.CompleteAsync([Task.CompletedTask]);

            Assert.Equal(RunState.Running, InstallProgress.Model!.State);
            Assert.Equal("3.1.4-alpha", MarkerVersion());
        }

        [Fact]
        public async Task AnInstallWhoseMarkerCannotBeWrittenFailsInsteadOfSucceeding()
        {
            var pipeline = Pipeline(new InstallOptions(KeepStaging: true, AssetVersion: "3.1.4-alpha"));
            File.Delete(Locations.OwnershipMarker);
            Directory.CreateDirectory(Locations.OwnershipMarker);

            var failure = await Assert.ThrowsAsync<IOException>(() => pipeline.CompleteAsync([Task.CompletedTask]));

            Assert.Contains(Locations.OwnershipMarker, failure.Message, StringComparison.Ordinal);
            Assert.Equal(RunState.Failed, InstallProgress.Conclude(failure));
        }
    }
}
