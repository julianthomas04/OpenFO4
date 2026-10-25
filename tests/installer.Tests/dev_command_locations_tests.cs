using AssetConverter;
using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests
{
    public sealed class DevCommandLocationsTests : IDisposable
    {
        const string SteamGame = @"S:\Steam\Fallout 4";

        readonly TempDirectory _directory = new();

        public void Dispose() => _directory.Dispose();

        static string? NoEnvironment(string _) => null;

        static string? SteamIsNotAsked() => throw new InvalidOperationException("Steam was asked although a game folder was named.");

        static DevCommandTarget Resolve(string[] args, Func<string, string?>? environment = null, Func<string?>? detectGameFolder = null) =>
            DevCommandLocations.Resolve(args, environment ?? NoEnvironment, detectGameFolder ?? (() => SteamGame));

        static Func<string, string?> Environment(string? game = null, string? install = null) =>
            variable => variable switch
            {
                HeadlessLocations.GameFolderVariable => game,
                HeadlessLocations.InstallFolderVariable => install,
                _ => null,
            };

        string MakeGameFolder()
        {
            string game = Path.Combine(_directory.Path, "game");
            Directory.CreateDirectory(Path.Combine(game, "Data"));
            return game;
        }

        string MakeInstall(string name = "install")
        {
            string root = Path.Combine(_directory.Path, name);
            var locations = new InstallLocations("", root);
            Directory.CreateDirectory(root);
            File.WriteAllText(locations.OwnershipMarker, "");
            return root;
        }

        [Fact]
        public void WithoutOptionsTheGameFolderIsSteamsAndTheInstallFolderTheVariables()
        {
            string install = MakeInstall();

            var target = Resolve(["collision-only"], Environment(install: install));

            Assert.Equal(SteamGame, target.Locations.Fo4Location);
            Assert.Equal(install, target.Locations.InstallRoot);
            Assert.Equal(["collision-only"], target.Arguments);
        }

        [Fact]
        public void TheVariablesWinOverSteamAndTheOptionsWinOverTheVariables()
        {
            string envGame = MakeGameFolder();
            string envInstall = MakeInstall("env-install");
            string flagInstall = MakeInstall("flag-install");

            var fromVariables = Resolve(["collision-only"], Environment(envGame, envInstall), SteamIsNotAsked);
            var flagOverVariable = Resolve(["collision-only", "--install-dir", flagInstall], Environment(envGame, envInstall), SteamIsNotAsked);

            Assert.Equal(envGame, fromVariables.Locations.Fo4Location);
            Assert.Equal(envInstall, fromVariables.Locations.InstallRoot);
            Assert.Equal(flagInstall, flagOverVariable.Locations.InstallRoot);
        }

        [Fact]
        public void ABlankVariableCountsAsUnset()
        {
            string install = MakeInstall();

            var blankGame = Resolve(["collision-only"], Environment(game: "  ", install: install));
            var blankInstall = Assert.Throws<LaunchArgumentException>(() => Resolve(["collision-only"], Environment(install: "  ")));

            Assert.Equal(SteamGame, blankGame.Locations.Fo4Location);
            Assert.Contains(HeadlessLocations.InstallFolderVariable, blankInstall.Message);
        }

        [Fact]
        public void AGameFolderNamedByTheVariableIsValidatedLikeTheOption()
        {
            string notAGame = Path.Combine(_directory.Path, "not-a-game");
            Directory.CreateDirectory(notAGame);

            var failure = Assert.Throws<LaunchArgumentException>(() => Resolve(["collision-only"], Environment(game: notAGame, install: MakeInstall())));

            Assert.Contains("no Data folder", failure.Message);
        }

        [Fact]
        public void AGameFolderNoOneKnowsIsRefusedWithTheOptionAndTheVariableToUse()
        {
            var failure = Assert.Throws<LaunchArgumentException>(() => Resolve(["collision-only", "--install-dir", MakeInstall()], detectGameFolder: () => null));

            Assert.Contains("--game-dir", failure.Message);
            Assert.Contains(HeadlessLocations.GameFolderVariable, failure.Message);
        }

        [Fact]
        public void TheOptionsInBothSpellingsChooseTheFolders()
        {
            string game = MakeGameFolder();
            string install = MakeInstall();

            var spaced = Resolve(["collision-only", "--install-dir", install, "--game-dir", game]);
            var equals = Resolve(["collision-only", $"--install-dir={install}", $"--game-dir={game}"]);

            Assert.All(new[] { spaced, equals }, target =>
            {
                Assert.Equal(game, target.Locations.Fo4Location);
                Assert.Equal(install, target.Locations.InstallRoot);
                Assert.Equal(["collision-only"], target.Arguments);
            });
        }

        [Fact]
        public void AFolderThatHoldsAnInstallIsTheInstallRootItself()
        {
            string chosen = MakeInstall("chosen");
            string nested = MakeInstall(Path.Combine("chosen", "OpenFO4"));

            var target = Resolve(["esm-only", "--install-dir", Path.Combine(_directory.Path, "chosen")]);

            Assert.Equal(chosen, target.Locations.InstallRoot);
            Assert.NotEqual(nested, target.Locations.InstallRoot);
        }

        [Fact]
        public void AFolderWithoutAnInstallButWithOneInTheDefaultSubfolderIsResolvedIntoIt()
        {
            string chosen = Path.Combine(_directory.Path, "parent");
            Directory.CreateDirectory(chosen);
            string nested = MakeInstall(Path.Combine("parent", InstallFolderRules.InstallFolderName));

            var target = Resolve(["esm-only", "--install-dir", chosen]);

            Assert.Equal(nested, target.Locations.InstallRoot);
        }

        [Fact]
        public void EsmOnlyKeepsItsTablesOptionOnceTheFolderOptionsAreTakenOut()
        {
            string install = MakeInstall();

            var target = Resolve(["esm-only", "--install-dir", install, "--tables", "statics,doors"]);

            Assert.True(EsmOnlyArguments.TryParse(target.Arguments, out string? tables));
            Assert.Equal("statics,doors", tables);
        }

        [Fact]
        public void AnInstallFolderThatHoldsNoInstallIsRefusedWithItsPath()
        {
            string empty = Path.Combine(_directory.Path, "empty");
            Directory.CreateDirectory(empty);

            var failure = Assert.Throws<LaunchArgumentException>(() =>
                Resolve(["collision-only", "--install-dir", empty]));

            Assert.Contains("does not hold an OpenFO4 install", failure.Message);
            Assert.Contains(empty, failure.Message);
        }

        [Fact]
        public void AGameFolderWithoutADataFolderIsRefused()
        {
            string notAGame = Path.Combine(_directory.Path, "not-a-game");
            Directory.CreateDirectory(notAGame);

            var failure = Assert.Throws<LaunchArgumentException>(() =>
                Resolve(["collision-only", "--game-dir", notAGame]));

            Assert.Contains("no Data folder", failure.Message);
        }

        [Fact]
        public void AMissingGameFolderIsRefused()
        {
            Assert.Throws<LaunchArgumentException>(() =>
                Resolve(["collision-only", "--game-dir", Path.Combine(_directory.Path, "absent")]));
        }

        [Theory]
        [InlineData("--install-dir")]
        [InlineData("--game-dir")]
        public void AnOptionWithoutAValueIsRefused(string option)
        {
            Assert.Throws<LaunchArgumentException>(() => Resolve(["collision-only", option]));
        }

        [Fact]
        public void WithoutAnyInstallFolderTheCommandIsRefusedWithTheOptionAndTheVariableToUse()
        {
            var failure = Assert.Throws<LaunchArgumentException>(() => Resolve(["collision-only"]));

            Assert.Contains("--install-dir", failure.Message);
            Assert.Contains(HeadlessLocations.InstallFolderVariable, failure.Message);
        }

        [Fact]
        public void WithoutFolderOptions_LeavesOtherArgumentsInOrder()
        {
            string[] stripped = DevCommandLocations.WithoutFolderOptions(["bench-ba2", "--game-dir", "G", "a.ba2", "--install-dir=I", "out", "serial"]);

            Assert.Equal(["bench-ba2", "a.ba2", "out", "serial"], stripped);
        }
    }
}
