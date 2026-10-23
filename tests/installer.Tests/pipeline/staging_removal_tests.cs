using AssetConverter;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using Xunit;

namespace InstallerTests.Pipeline
{
    // Reports progress through the process-wide InstallProgress, so it runs serially with the other tests that use it.
    [Collection("Console redirection")]
    public sealed class StagingRemovalTests : IDisposable
    {
        private static readonly IReadOnlyList<TimeSpan> NoRetries = [];

        private readonly TempDirectory _root = new();

        public StagingRemovalTests()
        {
            File.WriteAllText(Path.Combine(_root.Path, InstallLayout.OwnershipMarkerFileName), "");
        }

        public void Dispose()
        {
            InstallProgress.Stop();
            _root.Dispose();
        }

        private string MakeFolderWithAFile(string name)
        {
            string folder = Path.Combine(_root.Path, name);
            Directory.CreateDirectory(folder);
            File.WriteAllText(Path.Combine(folder, "keep.txt"), "x");
            return folder;
        }

        private static void CreateJunction(string link, string target)
        {
            var start = new System.Diagnostics.ProcessStartInfo("cmd.exe", $"/c mklink /J \"{link}\" \"{target}\"")
            {
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                CreateNoWindow = true,
            };
            using var process = System.Diagnostics.Process.Start(start)!;
            process.WaitForExit();
            Assert.SkipUnless(process.ExitCode == 0 && Directory.Exists(link), "This system refused to create a junction.");
        }

        private static void CreateDirectorySymbolicLink(string link, string target)
        {
            try
            {
                Directory.CreateSymbolicLink(link, target);
            }
            catch (Exception ex) when (ex is IOException or UnauthorizedAccessException)
            {
                Assert.Skip($"This system refused to create a symbolic link: {ex.Message}");
            }
        }

        private string Staging => Path.Combine(_root.Path, InstallLayout.StagingFolder);

        private string Write(string relativePath, string content = "x")
        {
            string path = Path.Combine(Staging, relativePath);
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllText(path, content);
            return path;
        }

        private void PopulateLikeAnInstall()
        {
            Write(".Fallout4 - Meshes.ba2.extracted", "");
            Write(".Fallout4 - Textures1.ba2.extracted-direct", "");
            Write("Fallout4.esm");
            Write(Path.Combine("Meshes", "Actors", "Dog", "dog.nif"));
            Write(Path.Combine("Materials", "a.bgsm"));
            Write(Path.Combine("palette_sources", "textures", "t.dds"));
        }

        [Fact]
        public void EverythingBelowTheStagingFolderAndTheFolderItselfIsRemoved()
        {
            PopulateLikeAnInstall();
            File.WriteAllText(Path.Combine(_root.Path, "install.log"), "log");

            var result = StagingRemoval.Run(Staging, ct: TestContext.Current.CancellationToken);

            Assert.False(Directory.Exists(Staging));
            Assert.True(File.Exists(Path.Combine(_root.Path, "install.log")));
            Assert.Equal(6, result.FilesRemoved);
            Assert.Equal(7, result.FoldersRemoved);
            Assert.Empty(result.Failures);
        }

        [Fact]
        public void ASiblingOfTheStagingFolderIsLeftAlone()
        {
            PopulateLikeAnInstall();
            string sibling = Path.Combine(_root.Path, "assets", "data.db");
            Directory.CreateDirectory(Path.GetDirectoryName(sibling)!);
            File.WriteAllText(sibling, "db");

            StagingRemoval.Run(Staging, ct: TestContext.Current.CancellationToken);

            Assert.True(File.Exists(sibling));
        }

        [Fact]
        public void ReadOnlyFilesAreRemovedToo()
        {
            File.SetAttributes(Write("locked.bin"), FileAttributes.ReadOnly);

            var result = StagingRemoval.Run(Staging, ct: TestContext.Current.CancellationToken);

            Assert.False(Directory.Exists(Staging));
            Assert.Empty(result.Failures);
        }

        [Fact]
        public void AMissingStagingFolderIsAnEmptyRemoval()
        {
            var result = StagingRemoval.Run(Staging, ct: TestContext.Current.CancellationToken);

            Assert.Equal(new StagingRemovalResult(0, 0, []), result);
        }

        [Fact]
        public void AFileHeldOpenIsReportedWhileEverythingElseIsStillRemovedAndTheMarkersGoRegardless()
        {
            PopulateLikeAnInstall();
            string held = Write(Path.Combine("Meshes", "held.nif"));
            using var holder = new FileStream(held, FileMode.Open, FileAccess.Read, FileShare.None);

            var result = StagingRemoval.Run(Staging, retryDelays: NoRetries, ct: TestContext.Current.CancellationToken);

            Assert.True(File.Exists(held));
            Assert.Single(Directory.EnumerateFiles(Staging, "*", SearchOption.AllDirectories));
            Assert.Empty(Directory.EnumerateFiles(Staging, ".*"));
            Assert.Contains(result.Failures, failure => failure.StartsWith(held, StringComparison.Ordinal));
            Assert.Equal(6, result.FilesRemoved);
        }

        [Fact]
        public void AFileHeldOnlyBrieflyIsRetriedUntilItCanBeRemoved()
        {
            string held = Write("briefly.bin");
            var holder = new FileStream(held, FileMode.Open, FileAccess.Read, FileShare.None);
            var pauses = new List<TimeSpan>();

            var result = StagingRemoval.Run(Staging, retryDelays: [TimeSpan.FromMilliseconds(1), TimeSpan.FromMilliseconds(2)],
                wait: pause =>
                {
                    lock (pauses) pauses.Add(pause);
                    holder.Dispose();
                }, ct: TestContext.Current.CancellationToken);

            Assert.Equal([TimeSpan.FromMilliseconds(1)], pauses);
            Assert.Empty(result.Failures);
            Assert.False(Directory.Exists(Staging));
        }

        [Fact]
        public void AMarkerThatCannotBeRemovedStopsTheRemovalSoNoMarkerOutlivesItsFiles()
        {
            PopulateLikeAnInstall();
            string heldMarker = Path.Combine(Staging, ".Fallout4 - Meshes.ba2.extracted");
            using var holder = new FileStream(heldMarker, FileMode.Open, FileAccess.Read, FileShare.None);
            string[] before = [.. Directory.EnumerateFiles(Staging, "*", SearchOption.AllDirectories).Where(f => !Path.GetFileName(f).StartsWith('.'))];

            var result = StagingRemoval.Run(Staging, retryDelays: NoRetries, ct: TestContext.Current.CancellationToken);

            Assert.True(File.Exists(heldMarker));
            Assert.All(before, file => Assert.True(File.Exists(file), file));
            Assert.Single(result.Failures);
            Assert.StartsWith(heldMarker, result.Failures[0], StringComparison.Ordinal);
            Assert.Equal(0, result.FoldersRemoved);
        }

        [Fact]
        public void AJunctionInsideTheStagingFolderIsDeletedWithoutTouchingItsTarget()
        {
            string outside = MakeFolderWithAFile("outside");
            Directory.CreateDirectory(Staging);
            CreateJunction(Path.Combine(Staging, "link"), outside);
            Write("own.bin");

            var result = StagingRemoval.Run(Staging, ct: TestContext.Current.CancellationToken);

            Assert.Empty(result.Failures);
            Assert.False(Directory.Exists(Staging));
            Assert.True(File.Exists(Path.Combine(outside, "keep.txt")));
        }

        [Fact]
        public void ADirectorySymbolicLinkInsideTheStagingFolderIsDeletedWithoutTouchingItsTarget()
        {
            string outside = MakeFolderWithAFile("outside");
            Directory.CreateDirectory(Staging);
            CreateDirectorySymbolicLink(Path.Combine(Staging, "link"), outside);

            var result = StagingRemoval.Run(Staging, ct: TestContext.Current.CancellationToken);

            Assert.Empty(result.Failures);
            Assert.False(Directory.Exists(Staging));
            Assert.True(File.Exists(Path.Combine(outside, "keep.txt")));
        }

        [Fact]
        public void ALinkNestedBelowAnotherFolderOfTheStagingFolderIsNotFollowedEither()
        {
            string outside = MakeFolderWithAFile("outside");
            Write(Path.Combine("a", "b", "file.bin"));
            CreateJunction(Path.Combine(Staging, "a", "b", "link"), outside);

            StagingRemoval.Run(Staging, ct: TestContext.Current.CancellationToken);

            Assert.False(Directory.Exists(Staging));
            Assert.True(File.Exists(Path.Combine(outside, "keep.txt")));
        }

        [Fact]
        public void AStagingFolderThatIsItselfAJunctionIsRemovedWithoutTouchingItsTarget()
        {
            string outside = MakeFolderWithAFile("outside");
            CreateJunction(Staging, outside);

            var result = StagingRemoval.Run(Staging, ct: TestContext.Current.CancellationToken);

            Assert.Empty(result.Failures);
            Assert.False(Directory.Exists(Staging));
            Assert.True(File.Exists(Path.Combine(outside, "keep.txt")));
        }

        [Fact]
        public void AStagingFolderOutsideAnInstallFolderIsNeverDeleted()
        {
            File.Delete(Path.Combine(_root.Path, InstallLayout.OwnershipMarkerFileName));
            Write("keep.bin");

            Assert.Throws<ArgumentException>(() => StagingRemoval.Run(Staging, ct: TestContext.Current.CancellationToken));

            Assert.True(File.Exists(Path.Combine(Staging, "keep.bin")));
        }

        [Fact]
        public void AnInstallFromBeforeTheMarkerExistedIsRecognisedByItsLogAndDatabase()
        {
            File.Delete(Path.Combine(_root.Path, InstallLayout.OwnershipMarkerFileName));
            var locations = new InstallLocations("", _root.Path);
            Directory.CreateDirectory(locations.DataOut);
            File.WriteAllText(locations.LogFile, "log");
            File.WriteAllText(locations.DatabasePath, "db");
            Write("a.bin");

            StagingRemoval.Run(Staging, ct: TestContext.Current.CancellationToken);

            Assert.False(Directory.Exists(Staging));
            Assert.True(File.Exists(locations.DatabasePath));
        }

        [Fact]
        public void ACancelledRemovalLeavesNoMarkerBehindWhoseFilesCouldBeGone()
        {
            PopulateLikeAnInstall();
            using var cancelled = new CancellationTokenSource();
            cancelled.Cancel();

            Assert.ThrowsAny<OperationCanceledException>(() => StagingRemoval.Run(Staging, ct: cancelled.Token));

            Assert.Empty(Directory.EnumerateFiles(Staging, ".*"));
        }

        [Fact]
        public void ProgressCountsEveryFileAndFolder()
        {
            InstallProgress.Start([new StepDefinition(InstallSteps.RemoveStaging, "items")]);
            PopulateLikeAnInstall();
            int phase = InstallProgress.BeginPhase(InstallSteps.RemoveStaging, 0);

            StagingRemoval.Run(Staging, phase, ct: TestContext.Current.CancellationToken);

            var step = InstallProgress.Model!.GetSnapshot().Steps.Single();
            Assert.Equal((6 + 7, 6 + 7), (step.Completed, step.Total));
        }

        [Theory]
        [InlineData("assets")]
        [InlineData("")]
        public void AFolderThatIsNotAStagingFolderIsNeverDeleted(string folderName)
        {
            string other = Path.Combine(_root.Path, folderName);
            Directory.CreateDirectory(other);
            File.WriteAllText(Path.Combine(other, "keep.txt"), "x");

            Assert.Throws<ArgumentException>(() => StagingRemoval.Run(other, ct: TestContext.Current.CancellationToken));

            Assert.True(File.Exists(Path.Combine(other, "keep.txt")));
        }

        [Fact]
        public void ARelativeStagingPathIsNeverDeleted()
        {
            Assert.Throws<ArgumentException>(() => StagingRemoval.Run(InstallLayout.StagingFolder, ct: TestContext.Current.CancellationToken));
        }
    }
}
