using AssetConverter;
using AssetConverter.Pipeline;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests.Pipeline
{
    [Collection("Console redirection")]
    public sealed class InstallOperationsTests
    {
        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private static string Touch(string root, string relative)
        {
            string path = Path.Combine(root, relative);
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllText(path, "x");
            return path;
        }

        [Fact]
        public void CopyGameInis_CopiesTheExistingCascadeFiles()
        {
            using var dir = new ArchiveTempDir();
            string game = Path.GetDirectoryName(Touch(dir.Path, "game/Fallout4_Default.ini"))!;
            string documents = Path.GetDirectoryName(Touch(dir.Path, "docs/Fallout4Prefs.ini"))!;
            string output = Path.Combine(dir.Path, "out");

            InstallOperations.CopyGameInis(game, output, documents);

            Assert.Equal(["Fallout4_Default.ini", "Fallout4Prefs.ini"], Directory.GetFiles(output).Select(Path.GetFileName).Order());
        }

        [Fact]
        public void CopyGameInis_FailsWithTheSetupMessageWhenTheGameWasNeverStarted()
        {
            using var dir = new ArchiveTempDir();
            string game = Path.GetDirectoryName(Touch(dir.Path, "game/Fallout4_Default.ini"))!;
            string documents = Directory.CreateDirectory(Path.Combine(dir.Path, "docs")).FullName;

            var error = Assert.Throws<InstallStartupException>(() => InstallOperations.CopyGameInis(game, Path.Combine(dir.Path, "out"), documents));

            Assert.Equal(GameIniFiles.MissingError(game, documents), error.Message);
            Assert.StartsWith("Fallout 4 has not been started yet.", error.Message);
            Assert.False(Directory.Exists(Path.Combine(dir.Path, "out")));
        }

        [Fact]
        public void PurgeByExtension_DeletesMatchingFilesInAllSubdirectoriesAndKeepsTheRest()
        {
            using var dir = new ArchiveTempDir();
            var matching = Enumerable.Range(0, 200).Select(i => Touch(dir.Path, $"a/b{i % 7}/model{i}.glb")).ToList();
            string collision = Touch(dir.Path, "a/model0.col.json");
            string animationPack = Touch(dir.Path, "Actors/Dog/CharacterAssets/walk.anims.glb");

            InstallOperations.PurgeByExtension(dir.Path, "*.glb", "GLB meshes", ct: Ct);

            Assert.All(matching, file => Assert.False(File.Exists(file)));
            Assert.False(File.Exists(animationPack));
            Assert.True(File.Exists(collision));
        }

        [Fact]
        public void PurgeByExtension_MissingDirectoryOrNoMatches_DoesNothing()
        {
            using var dir = new ArchiveTempDir();
            string keep = Touch(dir.Path, "readme.txt");

            InstallOperations.PurgeByExtension(Path.Combine(dir.Path, "missing"), "*.glb", "GLB meshes", ct: Ct);
            InstallOperations.PurgeByExtension(dir.Path, "*.glb", "GLB meshes", ct: Ct);

            Assert.True(File.Exists(keep));
        }

        [Fact]
        public void PurgeByExtension_FilesThatCannotBeDeleted_AreReportedTogetherAfterTheOthersWereDeleted()
        {
            Assert.SkipUnless(OperatingSystem.IsWindows(), "An open file blocks deletion only on Windows.");
            using var dir = new ArchiveTempDir();
            var deletable = Enumerable.Range(0, 50).Select(i => Touch(dir.Path, $"m/ok{i}.glb")).ToList();
            string lockedA = Touch(dir.Path, "m/locked_a.glb");
            string lockedB = Touch(dir.Path, "m/locked_b.glb");
            using var holdA = new FileStream(lockedA, FileMode.Open, FileAccess.Read, FileShare.None);
            using var holdB = new FileStream(lockedB, FileMode.Open, FileAccess.Read, FileShare.None);

            var ex = Assert.Throws<IOException>(() => InstallOperations.PurgeByExtension(dir.Path, "*.glb", "GLB meshes", wait: _ => { }, ct: Ct));

            Assert.Contains("2 of 52 existing GLB meshes", ex.Message);
            Assert.Contains("another process", ex.Message);
            Assert.Contains("locked_a.glb", ex.Message);
            Assert.Contains("locked_b.glb", ex.Message);
            Assert.NotNull(ex.InnerException);
            Assert.All(deletable, file => Assert.False(File.Exists(file)));
            Assert.True(File.Exists(lockedA));
            Assert.True(File.Exists(lockedB));
        }

        [Fact]
        public void PurgeByExtension_FileHeldBrieflyByAnotherProcess_IsDeletedOnceItIsReleased()
        {
            Assert.SkipUnless(OperatingSystem.IsWindows(), "An open file blocks deletion only on Windows.");
            using var dir = new ArchiveTempDir();
            string held = Touch(dir.Path, "m/held.glb");
            string other = Touch(dir.Path, "m/other.glb");
            var hold = new FileStream(held, FileMode.Open, FileAccess.Read, FileShare.None);
            int waits = 0;

            InstallOperations.PurgeByExtension(dir.Path, "*.glb", "GLB meshes", wait: _ =>
            {
                waits++;
                hold.Dispose();
            }, ct: Ct);

            Assert.Equal(1, waits);
            Assert.False(File.Exists(held));
            Assert.False(File.Exists(other));
        }

        [Fact]
        public void PurgeByExtension_FileThatStaysHeld_IsRetriedWithGrowingDelaysBeforeTheFailureIsReported()
        {
            Assert.SkipUnless(OperatingSystem.IsWindows(), "An open file blocks deletion only on Windows.");
            using var dir = new ArchiveTempDir();
            string locked = Touch(dir.Path, "m/locked.glb");
            using var hold = new FileStream(locked, FileMode.Open, FileAccess.Read, FileShare.None);
            var delays = new List<TimeSpan>();

            Assert.Throws<IOException>(() => InstallOperations.PurgeByExtension(dir.Path, "*.glb", "GLB meshes", wait: delays.Add, ct: Ct));

            Assert.Equal(4, delays.Count);
            Assert.Equal(delays.Order(), delays);
            Assert.True(delays[0] < delays[^1]);
        }

        [Fact]
        public void PurgeByExtension_Cancelled_ThrowsCancellationAndDeletesNothing()
        {
            using var dir = new ArchiveTempDir();
            string file = Touch(dir.Path, "m/a.glb");
            using var cts = new CancellationTokenSource();
            cts.Cancel();

            Assert.ThrowsAny<OperationCanceledException>(() => InstallOperations.PurgeByExtension(dir.Path, "*.glb", "GLB meshes", ct: cts.Token));

            Assert.True(File.Exists(file));
        }
    }
}
