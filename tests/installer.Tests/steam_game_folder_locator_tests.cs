using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public sealed class SteamGameFolderLocatorTests : IDisposable
    {
        private readonly TempDirectory _temp = new();

        public void Dispose() => _temp.Dispose();

        private string Steam => Path.Combine(_temp.Path, "Steam");

        private string Elsewhere => Path.Combine(_temp.Path, "Library");

        private string Second => Path.Combine(_temp.Path, "Second");

        private SteamGameFolderSearch Find() => new SteamGameFolderLocator(() => Steam).Find();

        private void WriteLibraryFolders(params string[] libraries)
        {
            string entries = string.Join(" ", libraries.Select((library, index) => $"\"{index}\" {{ \"path\" \"{library.Replace(@"\", @"\\")}\" }}"));
            Directory.CreateDirectory(Path.Combine(Steam, "steamapps"));
            File.WriteAllText(Path.Combine(Steam, "steamapps", "libraryfolders.vdf"), $"\"libraryfolders\" {{ {entries} }}");
        }

        private static string InstallGame(string library, string installDir = "Fallout 4 377160", bool exe = true, string? manifest = null)
        {
            Directory.CreateDirectory(Path.Combine(library, "steamapps"));
            File.WriteAllText(Path.Combine(library, "steamapps", "appmanifest_377160.acf"), manifest ?? $"\"AppState\" {{ \"appid\" \"377160\" \"installdir\" \"{installDir}\" }}");
            string game = Path.Combine(library, "steamapps", "common", installDir);
            Directory.CreateDirectory(game);
            if (exe) File.WriteAllBytes(Path.Combine(game, "Fallout4.exe"), []);
            return game;
        }

        private string MakeFallback()
        {
            string fallback = Path.Combine(_temp.Path, "Fallback");
            Directory.CreateDirectory(fallback);
            File.WriteAllBytes(Path.Combine(fallback, "Fallout4.exe"), []);
            return fallback;
        }

        [Fact]
        public void TheGameInTheSteamRootLibraryIsFound()
        {
            WriteLibraryFolders(Steam);
            string game = InstallGame(Steam);

            Assert.Equal(game, Find().Folder);
        }

        [Fact]
        public void TheGameInAnotherLibraryIsFound()
        {
            WriteLibraryFolders(Steam, Elsewhere);
            string game = InstallGame(Elsewhere);

            Assert.Equal(game, Find().Folder);
        }

        [Fact]
        public void TheSteamRootIsSearchedWhenItsLibraryListIsMissing()
        {
            string game = InstallGame(Steam);

            SteamGameFolderSearch search = Find();

            Assert.Equal(game, search.Folder);
            Assert.Contains(search.Notes, note => note.Contains("libraryfolders.vdf"));
        }

        [Fact]
        public void TheFirstLibraryWithAValidInstallWins()
        {
            WriteLibraryFolders(Elsewhere, Second);
            InstallGame(Elsewhere, exe: false);
            string expected = InstallGame(Second, "Fallout 4");

            Assert.Equal(expected, Find().Folder);
        }

        [Fact]
        public void AnEarlierLibraryBeatsALaterOne()
        {
            WriteLibraryFolders(Elsewhere, Second);
            string expected = InstallGame(Elsewhere);
            InstallGame(Second);

            Assert.Equal(expected, Find().Folder);
        }

        [Fact]
        public void AnInstallWithoutTheExecutableIsNotACandidate()
        {
            WriteLibraryFolders(Steam);
            InstallGame(Steam, exe: false);

            SteamGameFolderSearch search = Find();

            Assert.Null(search.Folder);
            Assert.Contains(search.Notes, note => note.Contains("Fallout4.exe"));
        }

        [Theory]
        [InlineData("\"AppState\" { \"appid\" \"377160\" }")]
        [InlineData("\"AppState\" { \"installdir\" \"\" }")]
        [InlineData("\"AppState\" { \"installdir\" ")]
        [InlineData("not a manifest")]
        public void AnInvalidManifestGivesNoCandidate(string manifest)
        {
            WriteLibraryFolders(Steam);
            InstallGame(Steam, manifest: manifest);

            SteamGameFolderSearch search = Find();

            Assert.Null(search.Folder);
            Assert.NotEmpty(search.Notes);
        }

        [Fact]
        public void AGarbledLibraryListStillSearchesTheSteamRoot()
        {
            Directory.CreateDirectory(Path.Combine(Steam, "steamapps"));
            File.WriteAllText(Path.Combine(Steam, "steamapps", "libraryfolders.vdf"), "\"libraryfolders\" { \"0\" { \"path\" ");
            string game = InstallGame(Steam);

            SteamGameFolderSearch search = Find();

            Assert.Equal(game, search.Folder);
            Assert.Contains(search.Notes, note => note.Contains("not valid"));
        }

        [Fact]
        public void AnUnreadableLibraryListIsReportedNotThrown()
        {
            Directory.CreateDirectory(Path.Combine(Steam, "steamapps", "libraryfolders.vdf"));

            SteamGameFolderSearch search = Find();

            Assert.Null(search.Folder);
            Assert.NotEmpty(search.Notes);
        }

        [Theory]
        [InlineData(null)]
        [InlineData(" ")]
        [InlineData("Missing")]
        public void AMissingSteamRootGivesNoCandidate(string? root)
        {
            string? resolved = root == "Missing" ? Path.Combine(_temp.Path, root) : root;

            SteamGameFolderSearch search = new SteamGameFolderLocator(() => resolved).Find();

            Assert.Null(search.Folder);
            Assert.NotEmpty(search.Notes);
        }

        [Fact]
        public void AFailingSteamRootLookupGivesNoCandidate()
        {
            var locator = new SteamGameFolderLocator(() => throw new UnauthorizedAccessException("denied"));

            SteamGameFolderSearch search = locator.Find();

            Assert.Null(search.Folder);
            Assert.Contains(search.Notes, note => note.Contains("denied"));
        }

        [Fact]
        public void TheDefaultIsTheSteamFindBeforeTheFallback()
        {
            string fallback = MakeFallback();
            WriteLibraryFolders(Steam);
            string game = InstallGame(Steam);

            Assert.Equal(game, InstallFolderRules.DefaultGameFolder(new SteamGameFolderLocator(() => Steam), _ => { }, fallback));
        }

        [Fact]
        public void TheDefaultFallsBackWhenSteamFindsNothingAndReportsTheNotes()
        {
            string fallback = MakeFallback();
            var notes = new List<string>();

            string? folder = InstallFolderRules.DefaultGameFolder(new SteamGameFolderLocator(() => null), notes.Add, fallback);

            Assert.Equal(fallback, folder);
            Assert.NotEmpty(notes);
        }

        [Fact]
        public void ThereIsNoDefaultWhenSteamAndTheFallbackFindNothing()
        {
            Assert.Null(InstallFolderRules.DefaultGameFolder(new SteamGameFolderLocator(() => null), _ => { }, Path.Combine(_temp.Path, "None")));
        }
    }
}
