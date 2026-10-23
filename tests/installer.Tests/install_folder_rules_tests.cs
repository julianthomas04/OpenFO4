using AssetConverter;
using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests
{
    public sealed class InstallFolderRulesTests : IDisposable
    {
        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private string Game => Path.Combine(_root.Path, "Fallout 4");

        private string MakeGameFolder(bool exe = true, bool esm = true)
        {
            Directory.CreateDirectory(Path.Combine(Game, "Data"));
            if (exe) File.WriteAllBytes(Path.Combine(Game, "Fallout4.exe"), []);
            if (esm) File.WriteAllBytes(Path.Combine(Game, "Data", "Fallout4.esm"), []);
            return Game;
        }

        [Fact]
        public void TheDefaultGameFolderIsTheCandidateWhenItExistsAndHoldsTheExecutable()
        {
            Assert.Equal(MakeGameFolder(esm: false), InstallFolderRules.DefaultGameFolder(Game));
        }

        [Fact]
        public void ThereIsNoDefaultGameFolderForAMissingCandidate()
        {
            Assert.Null(InstallFolderRules.DefaultGameFolder(Game));
        }

        [Fact]
        public void ThereIsNoDefaultGameFolderForAFolderWithoutTheExecutable()
        {
            MakeGameFolder(exe: false);

            Assert.Null(InstallFolderRules.DefaultGameFolder(Game));
        }

        [Fact]
        public void TheDefaultCandidateIsTheSteamLocation()
        {
            Assert.Equal(@"C:\Program Files (x86)\Steam\steamapps\common\Fallout 4", InstallFolderRules.SteamGameFolder);
        }

        [Fact]
        public void AGameFolderWithTheExecutableAndTheMasterFileIsValid()
        {
            Assert.Null(InstallFolderRules.GameFolderError(MakeGameFolder()));
        }

        [Fact]
        public void TheGameFolderMayBeTypedWithQuotesAndBlanks()
        {
            Assert.Null(InstallFolderRules.GameFolderError($"  \"{MakeGameFolder()}\" "));
        }

        [Theory]
        [InlineData("", "Choose the Fallout 4 folder.")]
        [InlineData("   ", "Choose the Fallout 4 folder.")]
        [InlineData("Fallout 4", "This folder does not exist.")]
        public void AnEmptyRelativeOrMissingGameFolderIsRejected(string input, string expected)
        {
            Assert.Equal(expected, InstallFolderRules.GameFolderError(input));
        }

        [Fact]
        public void AGameFolderThatDoesNotExistIsRejected()
        {
            Assert.Equal("This folder does not exist.", InstallFolderRules.GameFolderError(Game));
        }

        [Fact]
        public void AGameFolderWithoutTheExecutableIsRejected()
        {
            Assert.Equal("Fallout4.exe was not found in this folder.", InstallFolderRules.GameFolderError(MakeGameFolder(exe: false)));
        }

        [Fact]
        public void AGameFolderWithoutTheMasterFileIsRejected()
        {
            Assert.Equal(@"Data\Fallout4.esm was not found in this folder.", InstallFolderRules.GameFolderError(MakeGameFolder(esm: false)));
        }

        [Fact]
        public void AnInstallFolderThatDoesNotExistYetIsValid()
        {
            Assert.Null(InstallFolderRules.InstallFolderError(Path.Combine(_root.Path, "new", "install"), MakeGameFolder()));
        }

        [Fact]
        public void AnInstallFolderMarkedAsAnInstallIsValidEvenWithOtherContent()
        {
            string install = Path.Combine(_root.Path, "install");
            Directory.CreateDirectory(Path.Combine(install, "assets"));
            File.WriteAllText(Path.Combine(install, InstallLayout.OwnershipMarkerFileName), "");

            Assert.Null(InstallFolderRules.InstallFolderError(install, MakeGameFolder()));
        }

        [Fact]
        public void AnInstallFromBeforeTheMarkerExistedIsValidWhenItHoldsTheLogAndTheDatabase()
        {
            var locations = new InstallLocations("", Path.Combine(_root.Path, "install"));
            Directory.CreateDirectory(locations.DataOut);
            File.WriteAllText(locations.LogFile, "log");
            File.WriteAllText(locations.DatabasePath, "db");
            File.WriteAllText(Path.Combine(locations.InstallRoot, "other.txt"), "x");

            Assert.Null(InstallFolderRules.InstallFolderError(locations.InstallRoot, MakeGameFolder()));
        }

        [Fact]
        public void AnExistingEmptyInstallFolderIsValid()
        {
            string install = Path.Combine(_root.Path, "install");
            Directory.CreateDirectory(install);

            Assert.Null(InstallFolderRules.InstallFolderError(install, MakeGameFolder()));
        }

        [Fact]
        public void AFolderWithOtherContentIsRejectedBecauseTheInstallWouldReplaceFilesInIt()
        {
            string install = Path.Combine(_root.Path, "install");
            Directory.CreateDirectory(Path.Combine(install, "assets", "meshes"));

            Assert.Equal($"{install} already contains other files; choose an empty folder or a previous OpenFO4 install.", InstallFolderRules.InstallFolderError(install, MakeGameFolder()));
        }

        [Fact]
        public void AFolderWithOnlyTheLogOrOnlyTheDatabaseIsNotAnEarlierInstall()
        {
            var locations = new InstallLocations("", Path.Combine(_root.Path, "install"));
            Directory.CreateDirectory(locations.DataOut);
            File.WriteAllText(locations.DatabasePath, "db");

            Assert.NotNull(InstallFolderRules.InstallFolderError(locations.InstallRoot, ""));
        }

        [Fact]
        public void ADriveRootIsRejected()
        {
            string root = Path.GetPathRoot(_root.Path)!;

            Assert.Equal($"{root} is a whole drive; choose a folder on it.", InstallFolderRules.InstallFolderError(root, ""));
        }

        [Fact]
        public void ADriveThatDoesNotExistIsRejected()
        {
            char missing = Enumerable.Range('D', 'Z' - 'D' + 1).Select(c => (char)c).First(c => !Directory.Exists($"{c}:\\"));

            Assert.Equal($@"The drive of {missing}:\OpenFO4 is not available.", InstallFolderRules.InstallFolderError($@"{missing}:\OpenFO4", ""));
        }

        [Theory]
        [InlineData(@"D:\open:fo4")]
        [InlineData(@"D:\install\file:stream")]
        [InlineData(@"D:\a\b:c\d")]
        public void AColonBeyondTheDrivePrefixIsRejected(string input)
        {
            Assert.Equal("The path contains characters that are not allowed.", InstallFolderRules.InstallFolderError(input, ""));
        }

        [Fact]
        public void AReservedDeviceNameIsRejected()
        {
            string reserved = Path.Combine(_root.Path, "nul.txt", "x");

            Assert.Equal($"{reserved} uses a name that Windows reserves, such as CON or NUL.", InstallFolderRules.InstallFolderError(reserved, ""));
        }

        [Fact]
        public void AnInstallFolderThatContainsTheGameFolderIsRejected()
        {
            string game = MakeGameFolder();

            Assert.Equal($"{_root.Path} contains the Fallout 4 folder {game}; choose a folder that does not.", InstallFolderRules.InstallFolderError(_root.Path, game));
        }

        [Fact]
        public void AnInstallFolderIsValidWithoutAGameFolderToCompareWith()
        {
            Assert.Null(InstallFolderRules.InstallFolderError(Path.Combine(_root.Path, "install"), ""));
        }

        [Theory]
        [InlineData("", "Choose where to install.")]
        [InlineData("  ", "Choose where to install.")]
        [InlineData("install", @"Enter a full path, such as D:\OpenFO4.")]
        [InlineData(@"\install", @"Enter a full path, such as D:\OpenFO4.")]
        [InlineData(@"D:install", @"Enter a full path, such as D:\OpenFO4.")]
        [InlineData(@"D:\in|stall", "The path contains characters that are not allowed.")]
        [InlineData(@"D:\inst*ll", "The path contains characters that are not allowed.")]
        public void AnEmptyRelativeOrMalformedInstallFolderIsRejected(string input, string expected)
        {
            Assert.Equal(expected, InstallFolderRules.InstallFolderError(input, ""));
        }

        [Fact]
        public void AnInstallFolderThatIsAFileIsRejected()
        {
            string file = Path.Combine(_root.Path, "install");
            File.WriteAllBytes(file, []);

            Assert.Equal($"{file} is a file, not a folder.", InstallFolderRules.InstallFolderError(file, ""));
        }

        [Theory]
        [InlineData("")]
        [InlineData("Data")]
        [InlineData(@"Data\OpenFO4")]
        [InlineData(@"..\Fallout 4\Data")]
        public void AnInstallFolderAtOrInsideTheGameFolderIsRejected(string relative)
        {
            string game = MakeGameFolder();
            string install = Path.Combine(game, relative);

            Assert.Equal($"{InstallFolderRules.FullPath(install)} is inside the Fallout 4 folder {InstallFolderRules.FullPath(game)}; choose a folder outside it.", InstallFolderRules.InstallFolderError(install, game));
        }

        [Fact]
        public void TheComparisonWithTheGameFolderIgnoresCaseAndTrailingSeparators()
        {
            string game = MakeGameFolder();

            Assert.NotNull(InstallFolderRules.InstallFolderError(game.ToUpperInvariant() + Path.DirectorySeparatorChar, game));
        }

        [Fact]
        public void ASiblingWhoseNameStartsWithTheGameFolderNameIsOutsideIt()
        {
            string game = MakeGameFolder();

            Assert.Null(InstallFolderRules.InstallFolderError(game + " Install", game));
        }

        [Fact]
        public void TheFullPathDropsTheTrailingSeparatorAndResolvesDotSegments()
        {
            Assert.Equal(Path.Combine(_root.Path, "install"), InstallFolderRules.FullPath(Path.Combine(_root.Path, "x", "..", "install") + Path.DirectorySeparatorChar));
        }
    }
}
