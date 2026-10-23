using AssetConverter;
using AssetConverter.ShaderTranslation;
using AssetConverter.ShaderTranslation.Archive;
using AssetConverter.ShaderTranslation.Dxbc;
using AssetConverter.ShaderTranslation.Installation;
using AssetConverter.ShaderTranslation.Wgsl;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests.ShaderTranslation
{
    public class ShaderOutputSwapTests
    {
        private const string OldMarkerFile = "old-shaders.txt";

        private sealed class ScriptedDirectoryOperations : DirectoryOperations
        {
            public Func<string, string, Exception?> MoveFailure { get; set; } = (_, _) => null;

            public Func<string, Exception?> DeleteFailure { get; set; } = _ => null;

            public List<TimeSpan> Sleeps { get; } = [];

            public override void Move(string source, string destination)
            {
                var failure = MoveFailure(source, destination);
                if (failure != null) throw failure;
                base.Move(source, destination);
            }

            public override void DeleteRecursive(string path)
            {
                var failure = DeleteFailure(path);
                if (failure != null) throw failure;
                base.DeleteRecursive(path);
            }

            public override void Sleep(TimeSpan duration) => Sleeps.Add(duration);
        }

        private static FxpFile TinyArchive() =>
            new([], [new FxpEntry(0, 0, 0, ReadOnlyMemory<byte>.Empty, FxpBuilder.TinyDxbc(ShaderStage.Vertex))]);

        private static ShaderManifest TinyManifest() => ShaderManifestReader.Parse("""{"shaders":[{"name":"tiny_vs","blob":0,"stage":"vertex"}]}""");

        private static ShaderManifest MissingBlobManifest() => ShaderManifestReader.Parse("""{"shaders":[{"name":"gone","blob":5,"stage":"vertex"}]}""");

        private static int Translate(TempAssetRoot root, ScriptedDirectoryOperations operations, List<string>? warnings = null, ShaderManifest? manifest = null) =>
            ShaderTranslateStep.Translate(TinyArchive(), manifest ?? TinyManifest(), root.Path, operations, message => warnings?.Add(message), -1, TestContext.Current.CancellationToken);

        private static string Sibling(TempAssetRoot root, string suffix) => Path.Combine(root.Path, InstallLayout.ShadersFolder + suffix);

        private static void PlantShaders(string directory)
        {
            Directory.CreateDirectory(directory);
            File.WriteAllText(Path.Combine(directory, OldMarkerFile), "previous");
        }

        private static bool Targets(string path, string directory) => Path.GetFullPath(path) == Path.GetFullPath(directory);

        private static IEnumerable<string> Leftovers(TempAssetRoot root) =>
            Directory.GetDirectories(root.Path).Select(Path.GetFileName).Where(n => n != InstallLayout.ShadersFolder)!;

        [Fact]
        public void Publish_TransientRenameFailures_AreRetriedWithGrowingBackoff()
        {
            using var root = new TempAssetRoot();
            PlantShaders(root.Shaders);
            int failures = 2;
            var operations = new ScriptedDirectoryOperations
            {
                MoveFailure = (_, destination) => Targets(destination, root.Shaders) && failures-- > 0 ? new IOException("sharing violation") : null,
            };

            Translate(root, operations);

            Assert.True(File.Exists(Path.Combine(root.Shaders, "tiny_vs.wgsl")));
            Assert.Equal([TimeSpan.FromMilliseconds(100), TimeSpan.FromMilliseconds(200)], operations.Sleeps);
            Assert.Empty(Leftovers(root));
        }

        [Fact]
        public void Publish_AccessDeniedWhileScannerHoldsTheDirectory_IsRetriedToo()
        {
            using var root = new TempAssetRoot();
            int failures = 1;
            var operations = new ScriptedDirectoryOperations
            {
                MoveFailure = (_, destination) => Targets(destination, root.Shaders) && failures-- > 0 ? new UnauthorizedAccessException("denied") : null,
            };

            Translate(root, operations);

            Assert.True(File.Exists(Path.Combine(root.Shaders, "manifest.json")));
        }

        [Fact]
        public void Publish_DirectoryNotFound_IsNotRetried()
        {
            using var root = new TempAssetRoot();
            PlantShaders(root.Shaders);
            int attempts = 0;
            var operations = new ScriptedDirectoryOperations
            {
                MoveFailure = (source, _) =>
                {
                    if (!Targets(source, root.Shaders)) return null;
                    attempts++;
                    return new DirectoryNotFoundException("gone");
                },
            };

            Assert.ThrowsAny<DirectoryNotFoundException>(() => Translate(root, operations));

            Assert.Equal(1, attempts);
            Assert.Empty(operations.Sleeps);
        }

        [Fact]
        public void Publish_RenameKeepsFailing_ThrowsActionableErrorAndRestoresPreviousShaders()
        {
            using var root = new TempAssetRoot();
            PlantShaders(root.Shaders);
            var operations = new ScriptedDirectoryOperations
            {
                MoveFailure = (source, _) => source.Contains(".tmp-", StringComparison.Ordinal) ? new IOException("locked by scanner") : null,
            };

            var ex = Assert.Throws<ShaderTranslateStepException>(() => Translate(root, operations));

            Assert.IsType<IOException>(ex.InnerException);
            Assert.Contains($"after {OutputDirectorySwap.MaxAttempts} attempts", ex.Message);
            Assert.Contains("Close the game", ex.Message);
            Assert.Equal(OutputDirectorySwap.MaxAttempts - 1, operations.Sleeps.Count);
            Assert.Equal("previous", File.ReadAllText(Path.Combine(root.Shaders, OldMarkerFile)));
            Assert.Empty(Leftovers(root));
        }

        [Fact]
        public void Publish_RestoreFailsToo_KeepsBothFailuresAndLeavesTheBackupInPlace()
        {
            using var root = new TempAssetRoot();
            PlantShaders(root.Shaders);
            var operations = new ScriptedDirectoryOperations
            {
                MoveFailure = (_, destination) => Targets(destination, root.Shaders) ? new IOException($"locked {Path.GetFileName(destination)}") : null,
            };

            var ex = Assert.Throws<ShaderTranslateStepException>(() => Translate(root, operations));

            var aggregate = Assert.IsType<AggregateException>(ex.InnerException);
            Assert.Equal(2, aggregate.InnerExceptions.Count);
            Assert.All(aggregate.InnerExceptions, inner => Assert.IsType<ShaderTranslateStepException>(inner));
            string backup = Assert.Single(Directory.GetDirectories(root.Path), d => d.Contains(".old-", StringComparison.Ordinal));
            Assert.Contains(backup, ex.Message);
            Assert.Equal("previous", File.ReadAllText(Path.Combine(backup, OldMarkerFile)));
        }

        [Fact]
        public void Publish_CannotMoveExistingShadersAside_LeavesItUntouched()
        {
            using var root = new TempAssetRoot();
            PlantShaders(root.Shaders);
            var operations = new ScriptedDirectoryOperations
            {
                MoveFailure = (source, _) => Targets(source, root.Shaders) ? new IOException("game has it open") : null,
            };

            Assert.Throws<ShaderTranslateStepException>(() => Translate(root, operations));

            Assert.Equal("previous", File.ReadAllText(Path.Combine(root.Shaders, OldMarkerFile)));
            Assert.Empty(Leftovers(root));
        }

        [Fact]
        public void KilledRun_LeftBackupAndNoShadersFolder_BackupIsRestoredBeforeAnythingElseHappens()
        {
            using var root = new TempAssetRoot();
            PlantShaders(Sibling(root, ".old-aaaa"));

            Assert.Throws<ShaderTranslateStepException>(() => Translate(root, new ScriptedDirectoryOperations(), manifest: MissingBlobManifest()));

            Assert.Equal("previous", File.ReadAllText(Path.Combine(root.Shaders, OldMarkerFile)));
            Assert.Empty(Leftovers(root));
        }

        [Fact]
        public void KilledRun_SeveralBackups_NewestIsRestoredAndTheRestDiscarded()
        {
            using var root = new TempAssetRoot();
            string older = Sibling(root, ".old-1111");
            string newer = Sibling(root, ".old-2222");
            PlantShaders(older);
            PlantShaders(newer);
            File.WriteAllText(Path.Combine(newer, OldMarkerFile), "newer");
            Directory.SetLastWriteTimeUtc(older, new DateTime(2020, 1, 1, 0, 0, 0, DateTimeKind.Utc));
            Directory.SetLastWriteTimeUtc(newer, new DateTime(2021, 1, 1, 0, 0, 0, DateTimeKind.Utc));

            Assert.Throws<ShaderTranslateStepException>(() => Translate(root, new ScriptedDirectoryOperations(), manifest: MissingBlobManifest()));

            Assert.Equal("newer", File.ReadAllText(Path.Combine(root.Shaders, OldMarkerFile)));
            Assert.Empty(Leftovers(root));
        }

        [Fact]
        public void KilledRun_LeftoversNextToALiveShadersFolder_AreRemovedAndTheLiveFolderWins()
        {
            using var root = new TempAssetRoot();
            PlantShaders(root.Shaders);
            Directory.CreateDirectory(Sibling(root, ".tmp-cafe"));
            PlantShaders(Sibling(root, ".old-cafe"));

            Assert.Throws<ShaderTranslateStepException>(() => Translate(root, new ScriptedDirectoryOperations(), manifest: MissingBlobManifest()));

            Assert.Equal("previous", File.ReadAllText(Path.Combine(root.Shaders, OldMarkerFile)));
            Assert.Empty(Leftovers(root));
        }

        [Fact]
        public void Translate_WhileAnotherRunHoldsTheLock_FailsWithoutTouchingTheShadersFolder()
        {
            using var root = new TempAssetRoot();
            PlantShaders(root.Shaders);
            using var otherRun = new FileStream(Sibling(root, ".lock"), FileMode.OpenOrCreate, FileAccess.ReadWrite, FileShare.None);

            var ex = Assert.Throws<ShaderOutputLockedException>(() => Translate(root, new ScriptedDirectoryOperations()));

            Assert.Equal(Sibling(root, ".lock"), ex.LockPath);
            Assert.Equal("previous", File.ReadAllText(Path.Combine(root.Shaders, OldMarkerFile)));
            Assert.Empty(Leftovers(root));
        }

        [Fact]
        public void Translate_SecondRunStartedWhileTheFirstIsPublishing_IsRefused()
        {
            using var root = new TempAssetRoot();
            Exception? secondRunFailure = null;
            var operations = new ScriptedDirectoryOperations
            {
                MoveFailure = (_, _) =>
                {
                    secondRunFailure ??= Record.Exception(() => Translate(root, new ScriptedDirectoryOperations()));
                    return null;
                },
            };

            Translate(root, operations);

            Assert.IsType<ShaderOutputLockedException>(secondRunFailure);
            Assert.True(File.Exists(Path.Combine(root.Shaders, "tiny_vs.wgsl")));
        }

        [Fact]
        public void Translate_FailedRun_ReleasesTheLockForTheNextRun()
        {
            using var root = new TempAssetRoot();
            Assert.Throws<ShaderTranslateStepException>(() => Translate(root, new ScriptedDirectoryOperations(), manifest: MissingBlobManifest()));

            Translate(root, new ScriptedDirectoryOperations());

            Assert.True(File.Exists(Path.Combine(root.Shaders, "tiny_vs.wgsl")));
        }

        [Fact]
        public void Translate_LockFileLeftByAKilledRun_DoesNotBlockTheNextRun()
        {
            using var root = new TempAssetRoot();
            Directory.CreateDirectory(root.Path);
            File.WriteAllText(Sibling(root, ".lock"), "left behind by a run that was killed");

            Translate(root, new ScriptedDirectoryOperations());

            Assert.True(File.Exists(Path.Combine(root.Shaders, "tiny_vs.wgsl")));
        }

        [Fact]
        public void Translate_Finished_LeavesNoLockFileBehind()
        {
            using var root = new TempAssetRoot();

            Translate(root, new ScriptedDirectoryOperations());

            Assert.Equal([InstallLayout.ShadersFolder], Directory.GetFileSystemEntries(root.Path).Select(Path.GetFileName));
        }

        [Fact]
        public void Translate_CannotDeleteOldBackup_WarnsAndStillSucceeds()
        {
            using var root = new TempAssetRoot();
            PlantShaders(root.Shaders);
            var warnings = new List<string>();
            var operations = new ScriptedDirectoryOperations
            {
                DeleteFailure = path => path.Contains(".old-", StringComparison.Ordinal) ? new IOException("in use") : null,
            };

            Translate(root, operations, warnings);

            string warning = Assert.Single(warnings);
            Assert.Contains(".old-", warning);
            Assert.Contains("in use", warning);
            Assert.True(File.Exists(Path.Combine(root.Shaders, "tiny_vs.wgsl")));
        }

        [Fact]
        public void Translate_UnexpectedExceptionWhileWritingAShader_NamesTheShader()
        {
            using var root = new TempAssetRoot();
            var manifest = new ShaderManifest([new ManifestShader("bad\0name", 0, ShaderStage.Vertex, NanPolicy.Unspecified, [])]);

            var ex = Assert.Throws<ShaderTranslateStepException>(() => Translate(root, new ScriptedDirectoryOperations(), manifest: manifest));

            Assert.Contains("bad", ex.Message);
            Assert.IsAssignableFrom<ArgumentException>(ex.InnerException);
            Assert.Empty(Leftovers(root));
        }

        [Fact]
        public void OutputManifest_HasExactlyTheFieldsTheEngineReads()
        {
            using var root = new TempAssetRoot();
            Translate(root, new ScriptedDirectoryOperations());

            using var doc = System.Text.Json.JsonDocument.Parse(File.ReadAllText(Path.Combine(root.Shaders, "manifest.json")));

            Assert.Equal(["translator_version", "shaders"], doc.RootElement.EnumerateObject().Select(p => p.Name));
            var shader = doc.RootElement.GetProperty("shaders").EnumerateArray().Single();
            Assert.Equal(["name", "file", "stage", "source_blob", "source_sha256", "inputs", "outputs"], shader.EnumerateObject().Select(p => p.Name));
        }
    }
}
