using AssetConverter;
using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests
{
    /// <summary>The folder the user chooses is the parent: the install goes into an OpenFO4 folder inside it, unless the chosen folder is an install already.</summary>
    public sealed class InstallRootTests : IDisposable
    {
        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private string Chosen => Path.Combine(_root.Path, "Games");

        private string Game
        {
            get
            {
                string game = Path.Combine(_root.Path, "Fallout 4");
                Directory.CreateDirectory(game);
                return game;
            }
        }

        [Fact]
        public void TheInstallGoesIntoAnOpenFO4FolderInsideTheChosenFolder()
        {
            Assert.Equal(Path.Combine(Chosen, "OpenFO4"), InstallFolderRules.InstallRoot(Chosen));
        }

        [Fact]
        public void TrailingSeparatorsAndQuotesDoNotChangeTheRoot()
        {
            Assert.Equal(Path.Combine(Chosen, "OpenFO4"), InstallFolderRules.InstallRoot($"\"{Chosen}{Path.DirectorySeparatorChar}\""));
        }

        [Fact]
        public void AChosenFolderThatIsAlreadyAnInstallIsUsedItselfWithoutNesting()
        {
            Directory.CreateDirectory(Chosen);
            new InstallLocations("", Chosen).CreateOutputDirectories();

            Assert.Equal(Chosen, InstallFolderRules.InstallRoot(Chosen));
            Assert.Null(InstallFolderRules.ChosenFolderError(Chosen, Game));
        }

        [Fact]
        public void ALegacyInstallWithoutAMarkerCountsToo()
        {
            var legacy = new InstallLocations("", Chosen);
            Directory.CreateDirectory(legacy.DataOut);
            File.WriteAllText(legacy.LogFile, "log");
            File.WriteAllText(legacy.DatabasePath, "db");

            Assert.Equal(Chosen, InstallFolderRules.InstallRoot(Chosen));
        }

        [Fact]
        public void AnExistingOpenFO4FolderOfAnEarlierInstallInsideTheChosenFolderIsReused()
        {
            string existing = Path.Combine(Chosen, "OpenFO4");
            Directory.CreateDirectory(existing);
            new InstallLocations("", existing).CreateOutputDirectories();

            Assert.Equal(existing, InstallFolderRules.InstallRoot(Chosen));
            Assert.Null(InstallFolderRules.ChosenFolderError(Chosen, Game));
        }

        [Fact]
        public void TheChosenFolderMayBeNonEmpty()
        {
            Directory.CreateDirectory(Chosen);
            File.WriteAllText(Path.Combine(Chosen, "photos.jpg"), "x");

            Assert.Null(InstallFolderRules.ChosenFolderError(Chosen, Game));
        }

        [Fact]
        public void ANonEmptyOpenFO4FolderThatIsNoInstallIsRejectedAsTheFinalRoot()
        {
            Directory.CreateDirectory(Path.Combine(Chosen, "OpenFO4"));
            File.WriteAllText(Path.Combine(Chosen, "OpenFO4", "mine.txt"), "x");

            Assert.Equal($"{Path.Combine(Chosen, "OpenFO4")} already contains other files; choose an empty folder or a previous OpenFO4 install.", InstallFolderRules.ChosenFolderError(Chosen, Game));
        }

        [Fact]
        public void AChosenFolderInsideTheGameFolderIsRejectedBecauseTheRootWouldBeToo()
        {
            string inside = Path.Combine(Game, "mods", "OpenFO4");

            Assert.Equal($"{inside} is inside the Fallout 4 folder {Game}; choose a folder outside it.", InstallFolderRules.ChosenFolderError(Path.Combine(Game, "mods"), Game));
        }

        [Fact]
        public void AChosenFolderThatContainsTheGameFolderIsFineBecauseTheRootDoesNot()
        {
            Assert.Null(InstallFolderRules.ChosenFolderError(_root.Path, Game));
        }

        [Fact]
        public void AChosenFolderThatIsTheGameFoldersParentWithAnInstallThereIsFineButTheGameFolderItselfIsNot()
        {
            Assert.Equal($"{Path.Combine(Game, "OpenFO4")} is inside the Fallout 4 folder {Game}; choose a folder outside it.", InstallFolderRules.ChosenFolderError(Game, Game));
        }

        [Fact]
        public void AFileCannotBeChosen()
        {
            string file = Path.Combine(_root.Path, "file.txt");
            File.WriteAllText(file, "x");

            Assert.Equal($"{file} is a file, not a folder.", InstallFolderRules.ChosenFolderError(file, Game));
        }

        [Fact]
        public void ARelativePathNamesTheChosenFolderInItsExample()
        {
            Assert.Equal(@"Enter a full path, such as D:\Games.", InstallFolderRules.ChosenFolderError("games", Game));
        }

        [Fact]
        public void NothingChosenKeepsTheUsualMessageAndRootStaysEmpty()
        {
            Assert.Equal("Choose where to install.", InstallFolderRules.ChosenFolderError("  ", Game));
            Assert.Equal("", InstallFolderRules.InstallRoot("  "));
        }

        [Fact]
        public void ATextThatIsNoPathIsPassedOnForTheUsualChecksToReject()
        {
            Assert.Equal("nul*", InstallFolderRules.InstallRoot("nul*"));
            Assert.Equal("The path contains characters that are not allowed.", InstallFolderRules.ChosenFolderError("nul*", Game));
        }
    }

    public sealed class HeadlessLocationsTests
    {
        private static LaunchArguments Launch(string? install = null, string? game = null) =>
            new(Headless: true, KeepStaging: false, InstallDirectory: install, GameDirectory: game);

        private static Func<string, string?> Env(string? game = null, string? install = null) =>
            variable => variable switch
            {
                HeadlessLocations.GameFolderVariable => game,
                HeadlessLocations.InstallFolderVariable => install,
                _ => null,
            };

        [Fact]
        public void Headless_TheFlagsWinOverTheGameVariable()
        {
            var locations = HeadlessLocations.ForHeadlessInstall(Launch(@"D:\Out\OpenFO4", @"D:\Games\Fallout 4"), Env(@"E:\FromEnvironment"));

            Assert.Equal(@"D:\Games\Fallout 4", locations.Fo4Location);
            Assert.Equal(@"D:\Out\OpenFO4", locations.InstallRoot);
        }

        [Fact]
        public void Headless_TheGameFolderMayComeFromTheVariable()
        {
            var locations = HeadlessLocations.ForHeadlessInstall(Launch(@"D:\Out"), Env(@"E:\FromEnvironment"));

            Assert.Equal(@"E:\FromEnvironment", locations.Fo4Location);
        }

        [Fact]
        public void Headless_TheInstallVariableDoesNotNameTheInstallFolder()
        {
            var error = Assert.Throws<LaunchArgumentException>(() =>
                HeadlessLocations.ForHeadlessInstall(Launch(game: @"D:\Games\Fallout 4"), Env(install: @"E:\LiveInstall")));

            Assert.Contains("--install-dir", error.Message, StringComparison.Ordinal);
            Assert.DoesNotContain(HeadlessLocations.InstallFolderVariable, error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void Headless_TheInstallVariableDoesNotOverrideTheFlag()
        {
            var locations = HeadlessLocations.ForHeadlessInstall(Launch(@"D:\Out", @"D:\Games"), Env(install: @"E:\LiveInstall"));

            Assert.Equal(@"D:\Out", locations.InstallRoot);
        }

        [Fact]
        public void Headless_AnUnknownGameFolderIsRefused()
        {
            var error = Assert.Throws<LaunchArgumentException>(() => HeadlessLocations.ForHeadlessInstall(Launch(install: @"D:\Out"), Env()));

            Assert.Contains("--game-dir", error.Message, StringComparison.Ordinal);
            Assert.Contains(HeadlessLocations.GameFolderVariable, error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void TheSettingsFolderIsTheNamedOneOtherwiseTheDetectedDefault()
        {
            var named = HeadlessLocations.ForHeadlessInstall(Launch(@"D:\Out", @"D:\Games") with { SettingsDirectory = @"D:\Settings" }, Env());
            var detected = HeadlessLocations.ForHeadlessInstall(Launch(@"D:\Out", @"D:\Games"), Env());

            Assert.Equal(@"D:\Settings", named.SettingsDirectory);
            Assert.Equal(GameIniFiles.DocumentsDirectory(), detected.SettingsDirectory);
        }

        [Fact]
        public void APathThatCannotBeResolvedIsAnArgumentError()
        {
            Assert.Throws<LaunchArgumentException>(() => HeadlessLocations.ForHeadlessInstall(Launch("bad" + '\0' + "path", @"D:\G"), Env()));
        }

        [Fact]
        public void RelativeFoldersBecomeFullPathsAndSeparatorsAreTrimmed()
        {
            var locations = HeadlessLocations.ForHeadlessInstall(Launch(@"out\", @"game\"), Env());

            Assert.True(Path.IsPathFullyQualified(locations.InstallRoot));
            Assert.Equal(Path.GetFullPath("out"), locations.InstallRoot);
            Assert.Equal(Path.GetFullPath("game"), locations.Fo4Location);
        }
    }

    public sealed class LaunchArgumentFolderTests
    {
        private static Func<string, string?> NoEnvironment => _ => null;

        [Fact]
        public void TheFolderFlagsTakeTheNextArgumentOrAnEqualsValue()
        {
            var parsed = LaunchArguments.Parse(["--headless", "--install-dir", @"D:\A B", @"--game-dir=D:\Games\Fallout 4"], NoEnvironment);

            Assert.Equal(@"D:\A B", parsed.InstallDirectory);
            Assert.Equal(@"D:\Games\Fallout 4", parsed.GameDirectory);
        }

        [Fact]
        public void WithoutFolderFlagsThereAreNoFolders()
        {
            var parsed = LaunchArguments.Parse(["--headless"], NoEnvironment);

            Assert.Null(parsed.InstallDirectory);
            Assert.Null(parsed.GameDirectory);
        }

        [Theory]
        [InlineData("--install-dir")]
        [InlineData("--install-dir=")]
        [InlineData("--game-dir")]
        public void AFolderFlagWithoutAValueIsAnError(string flag)
        {
            var error = Assert.Throws<LaunchArgumentException>(() => LaunchArguments.Parse([flag], NoEnvironment));

            Assert.Contains(flag.Split('=')[0], error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void AFolderFlagFollowedByAnotherFlagIsAnErrorRatherThanSwallowingIt()
        {
            Assert.Throws<LaunchArgumentException>(() => LaunchArguments.Parse(["--install-dir", "--headless"], NoEnvironment));
        }

        [Fact]
        public void HeadlessKeepsTheTemporaryFilesUnlessToldToRemoveThem()
        {
            Assert.True(LaunchArguments.Parse(["--headless"], NoEnvironment).KeepsStagingWhenHeadless);
            Assert.False(LaunchArguments.Parse(["--headless", "--remove-staging"], NoEnvironment).KeepsStagingWhenHeadless);
        }

        [Fact]
        public void TheRemoveVariableSetToOneRemovesLikeTheFlag()
        {
            var parsed = LaunchArguments.Parse(["--headless"], name => name == "OPENFO4_REMOVE_STAGING" ? "1" : null);

            Assert.True(parsed.RemoveStaging);
            Assert.False(parsed.KeepsStagingWhenHeadless);
        }

        [Theory]
        [InlineData("--keep-staging", "--remove-staging")]
        [InlineData("--remove-staging", "--keep-staging")]
        public void KeepingAndRemovingTheTemporaryFilesAtOnceIsAnError(string first, string second)
        {
            var error = Assert.Throws<LaunchArgumentException>(() => LaunchArguments.Parse([first, second], NoEnvironment));

            Assert.Contains("contradict", error.Message, StringComparison.Ordinal);
        }

        [Fact]
        public void AFlagAndTheOppositeVariableContradictToo()
        {
            Assert.Throws<LaunchArgumentException>(() =>
                LaunchArguments.Parse(["--remove-staging"], name => name == "OPENFO4_KEEP_STAGING" ? "1" : null));
        }
    }
}
