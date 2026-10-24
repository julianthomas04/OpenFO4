using AssetConverter;
using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests
{
    public sealed class DevCommandLocationsTests : IDisposable
    {
        const string DefaultGame = @"Z:\Games\Fallout 4";
        const string DefaultInstall = @"C:\repo\test_installation\OpenFO4";

        readonly TempDirectory _directory = new();

        public void Dispose() => _directory.Dispose();

        static string? NoEnvironment(string _) => null;

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
        public void WithoutOptions_BothFoldersKeepTheirDefaults()
        {
            var target = DevCommandLocations.Resolve(["collision-only"], NoEnvironment, DefaultGame, DefaultInstall);

            Assert.Equal(DefaultGame, target.Locations.Fo4Location);
            Assert.Equal(DefaultInstall, target.Locations.InstallRoot);
            Assert.Equal(["collision-only"], target.Arguments);
        }

        [Fact]
        public void TheOptionsInBothSpellingsChooseTheFolders()
        {
            string game = MakeGameFolder();
            string install = MakeInstall();

            var spaced = DevCommandLocations.Resolve(["collision-only", "--install-dir", install, "--game-dir", game], NoEnvironment, DefaultGame, DefaultInstall);
            var equals = DevCommandLocations.Resolve(["collision-only", $"--install-dir={install}", $"--game-dir={game}"], NoEnvironment, DefaultGame, DefaultInstall);

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

            var target = DevCommandLocations.Resolve(["esm-only", "--install-dir", Path.Combine(_directory.Path, "chosen")], NoEnvironment, DefaultGame, DefaultInstall);

            Assert.Equal(chosen, target.Locations.InstallRoot);
            Assert.NotEqual(nested, target.Locations.InstallRoot);
        }

        [Fact]
        public void AFolderWithoutAnInstallButWithOneInTheDefaultSubfolderIsResolvedIntoIt()
        {
            string chosen = Path.Combine(_directory.Path, "parent");
            Directory.CreateDirectory(chosen);
            string nested = MakeInstall(Path.Combine("parent", InstallFolderRules.InstallFolderName));

            var target = DevCommandLocations.Resolve(["esm-only", "--install-dir", chosen], NoEnvironment, DefaultGame, DefaultInstall);

            Assert.Equal(nested, target.Locations.InstallRoot);
        }

        [Fact]
        public void EsmOnlyKeepsItsTablesOptionOnceTheFolderOptionsAreTakenOut()
        {
            string install = MakeInstall();

            var target = DevCommandLocations.Resolve(["esm-only", "--install-dir", install, "--tables", "statics,doors"], NoEnvironment, DefaultGame, DefaultInstall);

            Assert.True(EsmOnlyArguments.TryParse(target.Arguments, out string? tables));
            Assert.Equal("statics,doors", tables);
        }

        [Fact]
        public void AnInstallFolderThatHoldsNoInstallIsRefusedWithItsPath()
        {
            string empty = Path.Combine(_directory.Path, "empty");
            Directory.CreateDirectory(empty);

            var failure = Assert.Throws<LaunchArgumentException>(() =>
                DevCommandLocations.Resolve(["collision-only", "--install-dir", empty], NoEnvironment, DefaultGame, DefaultInstall));

            Assert.Contains("does not hold an OpenFO4 install", failure.Message);
            Assert.Contains(empty, failure.Message);
        }

        [Fact]
        public void AGameFolderWithoutADataFolderIsRefused()
        {
            string notAGame = Path.Combine(_directory.Path, "not-a-game");
            Directory.CreateDirectory(notAGame);

            var failure = Assert.Throws<LaunchArgumentException>(() =>
                DevCommandLocations.Resolve(["collision-only", "--game-dir", notAGame], NoEnvironment, DefaultGame, DefaultInstall));

            Assert.Contains("no Data folder", failure.Message);
        }

        [Fact]
        public void AMissingGameFolderIsRefused()
        {
            Assert.Throws<LaunchArgumentException>(() =>
                DevCommandLocations.Resolve(["collision-only", "--game-dir", Path.Combine(_directory.Path, "absent")], NoEnvironment, DefaultGame, DefaultInstall));
        }

        [Theory]
        [InlineData("--install-dir")]
        [InlineData("--game-dir")]
        public void AnOptionWithoutAValueIsRefused(string option)
        {
            Assert.Throws<LaunchArgumentException>(() => DevCommandLocations.Resolve(["collision-only", option], NoEnvironment, DefaultGame, DefaultInstall));
        }

        [Fact]
        public void WithoutAnyInstallFolderTheCommandIsRefused()
        {
            Assert.Throws<LaunchArgumentException>(() => DevCommandLocations.Resolve(["collision-only"], NoEnvironment, DefaultGame, defaultInstallFolder: null));
        }

        [Fact]
        public void WithoutFolderOptions_LeavesOtherArgumentsInOrder()
        {
            string[] stripped = DevCommandLocations.WithoutFolderOptions(["bench-ba2", "--game-dir", "G", "a.ba2", "--install-dir=I", "out", "serial"]);

            Assert.Equal(["bench-ba2", "a.ba2", "out", "serial"], stripped);
        }
    }
}
