using AssetConverter;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using Xunit;

namespace InstallerTests.Pipeline
{
    public sealed class GameFileInstallerTests : IDisposable
    {
        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private string Source(string name, string content)
        {
            string dir = Path.Combine(_root.Path, "source");
            Directory.CreateDirectory(dir);
            string path = Path.Combine(dir, name);
            File.WriteAllText(path, content);
            return path;
        }

        private string InstallRoot
        {
            get
            {
                string dir = Path.Combine(_root.Path, "install");
                Directory.CreateDirectory(dir);
                return dir;
            }
        }

        [Fact]
        public void EveryFileIsCopiedUnderItsNameAndReported()
        {
            var installed = new List<string>();

            GameFileInstaller.Install(new GameFiles([Source("openfo4.exe", "exe"), Source("readme.txt", "text")], Required: true), InstallRoot, installed.Add, CancellationToken.None);

            Assert.Equal("exe", File.ReadAllText(Path.Combine(InstallRoot, "openfo4.exe")));
            Assert.Equal("text", File.ReadAllText(Path.Combine(InstallRoot, "readme.txt")));
            Assert.Equal(["openfo4.exe", "readme.txt"], installed);
            Assert.Empty(Directory.EnumerateFiles(InstallRoot, "*.installing"));
        }

        [Fact]
        public void FilesBelowTheRootKeepTheirFoldersAndAreReportedByTheirRelativePath()
        {
            string root = Path.Combine(_root.Path, "game");
            string exe = Path.Combine(root, "openfo4.exe");
            string nestedRelative = Path.Combine("subfolder", "nested", "file.bin");
            string nested = Path.Combine(root, nestedRelative);
            Directory.CreateDirectory(Path.GetDirectoryName(nested)!);
            File.WriteAllText(exe, "exe");
            File.WriteAllText(nested, "nested");
            var installed = new List<string>();

            GameFileInstaller.Install(new GameFiles([exe, nested], Required: true, Root: root), InstallRoot, installed.Add, CancellationToken.None);

            Assert.Equal("nested", File.ReadAllText(Path.Combine(InstallRoot, nestedRelative)));
            Assert.Equal("exe", File.ReadAllText(Path.Combine(InstallRoot, "openfo4.exe")));
            Assert.Equal(["openfo4.exe", nestedRelative], installed);
        }

        private string Plant(string relative)
        {
            string path = Path.Combine(InstallRoot, relative);
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllText(path, "old");
            return path;
        }

        [Fact]
        public void APayloadPathThatEscapesTheInstallFolderIsRefusedBeforeAnythingIsTouched()
        {
            string root = Path.Combine(_root.Path, "game");
            Directory.CreateDirectory(root);
            string outside = Path.Combine(_root.Path, "outside.dll");
            File.WriteAllText(outside, "x");
            string planted = Plant(Path.Combine("data", "old.bin"));

            Assert.Throws<InvalidOperationException>(() =>
                GameFileInstaller.Install(new GameFiles([outside], Required: true, Root: root), InstallRoot, _ => { }, CancellationToken.None));

            Assert.True(File.Exists(planted));
        }

        [Fact]
        public void AnExecutableNamedLikeTheGameInASubfolderIsNotTheGame()
        {
            string root = Path.Combine(_root.Path, "game");
            string nested = Path.Combine(root, "tools", "openfo4.exe");
            Directory.CreateDirectory(Path.GetDirectoryName(nested)!);
            File.WriteAllText(nested, "not the game");

            Assert.False(new GameFiles([nested], Required: true, Root: root).HasExecutable);
        }

        [Fact]
        public void AnEarlierCopyIsReplaced()
        {
            File.WriteAllText(Path.Combine(InstallRoot, "openfo4.exe"), "old");

            GameFileInstaller.Install(new GameFiles([Source("openfo4.exe", "new")], Required: true), InstallRoot, _ => { }, CancellationToken.None);

            Assert.Equal("new", File.ReadAllText(Path.Combine(InstallRoot, "openfo4.exe")));
        }

        [Fact]
        public void ACopyThatIsInUseFailsNamingTheFileAndKeepsTheOldOneWholeWithoutLeavingPartialFiles()
        {
            string destination = Path.Combine(InstallRoot, "openfo4.exe");
            File.WriteAllText(destination, "old");
            using var running = new FileStream(destination, FileMode.Open, FileAccess.Read, FileShare.Read);

            var error = Assert.Throws<IOException>(() =>
                GameFileInstaller.Install(new GameFiles([Source("openfo4.exe", "new")], Required: true), InstallRoot, _ => { }, CancellationToken.None));

            Assert.Contains("openfo4.exe", error.Message, StringComparison.Ordinal);
            running.Dispose();
            Assert.Equal("old", File.ReadAllText(destination));
            Assert.Empty(Directory.EnumerateFiles(InstallRoot, "*.installing"));
        }

        [Fact]
        public void ACancelledInstallCopiesNothing()
        {
            using var cancelled = new CancellationTokenSource();
            cancelled.Cancel();

            Assert.ThrowsAny<OperationCanceledException>(() =>
                GameFileInstaller.Install(new GameFiles([Source("openfo4.exe", "exe")], Required: true), InstallRoot, _ => { }, cancelled.Token));

            Assert.False(File.Exists(Path.Combine(InstallRoot, "openfo4.exe")));
        }
    }

    [Collection("Console redirection")]
    public sealed class InstallGameStageTests : IDisposable
    {
        private readonly TextWriter _originalOut = Console.Out;
        private readonly TextWriter _originalError = Console.Error;
        private readonly StringWriter _output = new();
        private readonly StringWriter _errors = new();
        private readonly TempDirectory _root = new();

        public InstallGameStageTests()
        {
            Console.SetOut(_output);
            Console.SetError(_errors);
            InstallProgress.Start([new StepDefinition(InstallSteps.InstallGame, "files")]);
        }

        public void Dispose()
        {
            Console.SetOut(_originalOut);
            Console.SetError(_originalError);
            InstallProgress.Stop();
            _root.Dispose();
        }

        private InstallLocations Locations => new(Path.Combine(_root.Path, "game"), Path.Combine(_root.Path, "install"));

        private static StepSnapshot Step() => InstallProgress.Model!.GetSnapshot().Steps.Single();

        private string SourceExe()
        {
            string path = Path.Combine(_root.Path, "built", GameFiles.ExecutableName);
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllText(path, "exe");
            return path;
        }

        [Fact]
        public void TheGameExecutableLandsInTheInstallRootAndTheStepIsDone()
        {
            Locations.CreateOutputDirectories();

            new PipelineStages(Locations, MemoryBudget.Unlimited(), CancellationToken.None).InstallGame(new GameFiles([SourceExe()], Required: true));

            Assert.Equal("exe", File.ReadAllText(Locations.GameExecutable));
            Assert.Equal(StepStatus.Done, Step().Status);
            Assert.Equal(1, Step().Completed);
        }

        [Fact]
        public void WithoutAGameAndNoneRequiredItWarnsClearlyAndTheInstallGoesOn()
        {
            Locations.CreateOutputDirectories();

            new PipelineStages(Locations, MemoryBudget.Unlimited(), CancellationToken.None).InstallGame(GameFiles.None);

            Assert.False(File.Exists(Locations.GameExecutable));
            Assert.Contains("WARNING: no openfo4.exe to install", _errors.ToString(), StringComparison.Ordinal);
            Assert.Equal(StepStatus.Done, Step().Status);
        }

        [Fact]
        public void WithoutAGameWhereItIsRequiredTheInstallFails()
        {
            Locations.CreateOutputDirectories();

            var error = Assert.Throws<FileNotFoundException>(() =>
                new PipelineStages(Locations, MemoryBudget.Unlimited(), CancellationToken.None).InstallGame(new GameFiles([], Required: true)));

            Assert.Contains("openfo4.exe", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void AFileThatIsNotTheGameExecutableDoesNotCountAsTheGame()
        {
            Locations.CreateOutputDirectories();
            string readme = Path.Combine(_root.Path, "readme.txt");
            File.WriteAllText(readme, "text");

            Assert.Throws<FileNotFoundException>(() =>
                new PipelineStages(Locations, MemoryBudget.Unlimited(), CancellationToken.None).InstallGame(new GameFiles([readme], Required: true)));
        }
    }
}
