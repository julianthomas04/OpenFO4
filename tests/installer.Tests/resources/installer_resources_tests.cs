using AssetConverter;
using Xunit;

namespace InstallerTests.Resources
{
    public sealed class InstallerResourcesTests : IDisposable
    {
        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private string Touch(params string[] relativePath)
        {
            string path = Path.Combine(_root.Path, Path.Combine(relativePath));
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllText(path, "x");
            return path;
        }

        [Fact]
        public void Repository_RequireResolvesRelativeToTheRepoAndMissingOnesFailNamingThem()
        {
            string schema = Touch("src", "installer", "file_converters", "esm_schema.sql");
            var resources = InstallerResources.ForRepository(_root.Path);

            Assert.Equal(schema, resources.Require("src/installer/file_converters/esm_schema.sql"));
            var error = Assert.Throws<FileNotFoundException>(() => resources.Require("src/installer/missing.sql"));
            Assert.Contains("src/installer/missing.sql", error.Message, StringComparison.Ordinal);
            Assert.False(resources.IsPackaged);
        }

        [Fact]
        public void Repository_FfdecRunsItsJarOnTheSystemJavaWithTheSameOptionsAsThePayload()
        {
            const string systemJava = @"C:\jdk\bin\java.exe";
            var command = InstallerResources.ForRepository(_root.Path, () => systemJava).Ffdec();
            var payload = InstallerResources.ForPayload(_root.Path).Ffdec();
            string jar = Path.Combine(_root.Path, "build", "ffdec", "ffdec-cli.jar");

            Assert.Equal(systemJava, command.FileName);
            Assert.Equal(payload.LeadingArguments.Take(payload.LeadingArguments.Count - 1), command.LeadingArguments.Take(command.LeadingArguments.Count - 1));
            Assert.Equal(Path.Combine(_root.Path, "third_party", "ffdec", "ffdec-cli.jar"), payload.LeadingArguments[^1]);
            Assert.Equal(jar, command.LeadingArguments[^1]);
            Assert.Contains("-Xmx1024m", command.LeadingArguments);
            Assert.Empty(command.Environment);
            Assert.Equal([systemJava, jar], command.RequiredFiles);
        }

        [Fact]
        public void Repository_FfmpegIsTheOneBuiltIntoTheBuildFolder()
        {
            var command = InstallerResources.ForRepository(_root.Path).Ffmpeg();
            string ffmpeg = Path.Combine(_root.Path, "build", "librempeg", "ffmpeg.exe");

            Assert.Equal(ffmpeg, command.FileName);
            Assert.Equal([ffmpeg], command.RequiredFiles);
            Assert.Empty(command.LeadingArguments);
        }

        [Fact]
        public void Payload_FfmpegIsTheBundledOneBesideFfdec()
        {
            var command = InstallerResources.ForPayload(_root.Path).Ffmpeg();

            Assert.Equal(Path.Combine(_root.Path, "third_party", "librempeg", "ffmpeg.exe"), command.FileName);
        }

        [Fact]
        public void Repository_TheBink2DecoderIsTheReleaseBuildOfItsToolFolder()
        {
            var command = InstallerResources.ForRepository(_root.Path).Bink2Decoder();
            string decoder = Path.Combine(_root.Path, "tools", "bink2-decoder", "bin", "Release", "net8.0", "Bink2Decoder.exe");

            Assert.Equal(decoder, command.FileName);
            Assert.Equal([decoder], command.RequiredFiles);
            Assert.Empty(command.LeadingArguments);
        }

        [Fact]
        public void Payload_TheBink2DecoderIsTheBundledOneBesideTheOtherThirdPartyTools()
        {
            var command = InstallerResources.ForPayload(_root.Path).Bink2Decoder();

            Assert.Equal(Path.Combine(_root.Path, "third_party", "bink2-decoder", "Bink2Decoder.exe"), command.FileName);
        }

        [Fact]
        public void Repository_FfdecWithoutAnyJavaFailsExplicitly()
        {
            var resources = InstallerResources.ForRepository(_root.Path, () => null);

            var error = Assert.Throws<FfdecException>(() => resources.Ffdec());
            Assert.Contains("JAVA_HOME", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void MissingJavaMessage_NamesAJavaHomeWithoutJava()
        {
            string message = InstallerResources.MissingJavaMessage(@"C:\jdk");

            Assert.Contains("JAVA_HOME is set but contains no bin", message, StringComparison.Ordinal);
            Assert.Contains(Path.Combine(@"C:\jdk", "bin", "java.exe"), message, StringComparison.Ordinal);
            Assert.DoesNotContain("is set but", InstallerResources.MissingJavaMessage(null), StringComparison.Ordinal);
        }

        [Fact]
        public void SystemJava_PrefersJavaHomeOverPathAndFallsBackToPath()
        {
            string home = Touch("jdk", "bin", "java.exe");
            string onPath = Touch("other", "java.exe");
            string pathVariable = Path.GetDirectoryName(onPath)!;

            Assert.Equal(home, InstallerResources.FindSystemJava(Path.Combine(_root.Path, "jdk"), pathVariable));
            Assert.Equal(onPath, InstallerResources.FindSystemJava(Path.Combine(_root.Path, "missing"), pathVariable));
            Assert.Null(InstallerResources.FindSystemJava(null, null));
        }

        [Fact]
        public void Repository_WithoutABuiltEngineThereIsNoGameAndItIsNotRequired()
        {
            var game = InstallerResources.ForRepository(_root.Path).Game;

            Assert.Empty(game.Files);
            Assert.False(game.Required);
            Assert.False(game.HasExecutable);
        }

        [Fact]
        public void Repository_TheReleaseBuildOfTheEngineIsPreferredOverTheDebugBuild()
        {
            Touch("src", "engine", "target", "debug", "openfo4.exe");
            string release = Touch("src", "engine", "target", "release", "openfo4.exe");

            var game = InstallerResources.ForRepository(_root.Path).Game;

            Assert.Equal([release], game.Files);
            Assert.True(game.HasExecutable);
            Assert.False(game.Required);
        }

        [Fact]
        public void Repository_TheDebugBuildIsUsedWhenThereIsNoReleaseBuild()
        {
            string debug = Touch("src", "engine", "target", "debug", "openfo4.exe");

            Assert.Equal([debug], InstallerResources.ForRepository(_root.Path).Game.Files);
        }

        [Fact]
        public void Payload_PythonIsTheBundledOneWhetherOrNotAnyIsOnPath()
        {
            var resources = InstallerResources.ForPayload(_root.Path);

            Assert.Equal(Path.Combine(_root.Path, "runtime", "python", "python.exe"), resources.PythonExecutable());
            Assert.True(resources.IsPackaged);
        }

        [Fact]
        public void Payload_FfdecRunsItsJarOnTheBundledJavaAndPointsJavaHomeThere()
        {
            var command = InstallerResources.ForPayload(_root.Path).Ffdec();
            string javaHome = Path.Combine(_root.Path, "runtime", "jre");

            Assert.Equal(Path.Combine(javaHome, "bin", "java.exe"), command.FileName);
            Assert.Equal(Path.Combine(_root.Path, "third_party", "ffdec", "ffdec-cli.jar"), command.LeadingArguments[^1]);
            Assert.Equal("-jar", command.LeadingArguments[^2]);
            Assert.Equal(javaHome, command.Environment["JAVA_HOME"]);
            Assert.Equal([command.FileName, command.LeadingArguments[^1]], command.RequiredFiles);
        }

        [Fact]
        public void Payload_TheGameFilesAreEverythingInTheGameFolderAndAreRequired()
        {
            string exe = Touch("game", "openfo4.exe");
            string readme = Touch("game", "readme.txt");

            var game = InstallerResources.ForPayload(_root.Path).Game;

            Assert.Equal([exe, readme], game.Files);
            Assert.True(game.Required);
            Assert.True(game.HasExecutable);
        }

        [Fact]
        public void Payload_TheGameFilesIncludeTheSubfoldersBesideTheExecutable()
        {
            string exe = Touch("game", "openfo4.exe");
            string nested = Touch("game", "subfolder", "nested", "file.bin");

            var game = InstallerResources.ForPayload(_root.Path).Game;

            Assert.Equal([exe, nested], game.Files);
            Assert.Equal(Path.Combine("subfolder", "nested", "file.bin"), game.InstalledPath(nested));
            Assert.True(game.HasExecutable);
        }

        [Fact]
        public void Payload_WithoutAGameFolderTheGameIsRequiredButEmpty()
        {
            var game = InstallerResources.ForPayload(_root.Path).Game;

            Assert.Empty(game.Files);
            Assert.True(game.Required);
            Assert.False(game.HasExecutable);
        }

        [Fact]
        public void ToolCommand_MissingFileNamesTheFirstRequiredFileThatIsAbsent()
        {
            string present = Touch("present.exe");
            string absent = Path.Combine(_root.Path, "absent.jar");
            var command = new ToolCommand(present, [], new Dictionary<string, string>(), [present, absent]);

            Assert.Equal(absent, command.MissingFile());
            Assert.Null(ToolCommand.Program(present).MissingFile());
        }

        [Fact]
        public void ToolCommand_ApplySetsTheProgramPutsLeadingArgumentsFirstAndAddsTheEnvironment()
        {
            var command = new ToolCommand("tool.exe", ["-a", "-b"], new Dictionary<string, string> { ["JAVA_HOME"] = "j" }, ["tool.exe"]);
            var startInfo = new System.Diagnostics.ProcessStartInfo();

            command.Apply(startInfo, ["x", "y"]);

            Assert.Equal("tool.exe", startInfo.FileName);
            Assert.Equal(["-a", "-b", "x", "y"], startInfo.ArgumentList);
            Assert.Equal("j", startInfo.Environment["JAVA_HOME"]);
        }
    }

    public sealed class InstallerResourceLocatorTests : IDisposable
    {
        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        [Fact]
        public void WithoutAPayloadTheRepositoryIsUsed()
        {
            var resources = InstallerResourceLocator.Locate(null, Path.Combine(_root.Path, "cache"), "1.0.0", () => _root.Path);

            Assert.Equal(_root.Path, resources.Root);
            Assert.False(resources.IsPackaged);
        }

        [Fact]
        public void WithAPayloadItIsUnpackedAndNoRepositoryIsLookedFor()
        {
            var payload = PayloadExtractorTests.Payload(PayloadExtractorTests.Zip(("game/openfo4.exe", "exe")));

            var resources = InstallerResourceLocator.Locate(payload, Path.Combine(_root.Path, "cache"), "1.0.0",
                () => throw new InvalidOperationException("the repository must not be looked for"));

            Assert.True(resources.IsPackaged);
            Assert.StartsWith(Path.Combine(_root.Path, "cache"), resources.Root, StringComparison.Ordinal);
            Assert.True(resources.Game.HasExecutable);
            Assert.Equal("exe", File.ReadAllText(resources.Game.Files.Single()));
        }

        [Fact]
        public void ThisTestAssemblyCarriesNoPayload()
        {
            Assert.False(InstallerResourceLocator.IsPackaged(typeof(InstallerResources).Assembly));
        }
    }

    public sealed class InstallerVersionTests
    {
        [Fact]
        public void ThisTestAssemblyCarriesTheDevelopmentVersion()
        {
            Assert.Equal("0.0.0-dev", InstallerVersion.Current);
        }
    }
}
