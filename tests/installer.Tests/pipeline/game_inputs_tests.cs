using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests.Pipeline
{
    public sealed class GameInputsTests : IDisposable
    {
        private static readonly DateTime Written = new(2024, 5, 6, 7, 8, 9, DateTimeKind.Utc);

        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private static GameFileStamp Stamp(string path, long length = 10, DateTime? written = null) => new(path, length, written ?? Written);

        private string Data => Path.Combine(_root.Path, "Data");

        private void Touch(string relative, string content = "x", DateTime? written = null)
        {
            string path = Path.Combine(Data, relative);
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllText(path, content);
            File.SetLastWriteTimeUtc(path, written ?? Written);
        }

        [Fact]
        public void TheFingerprintIsSixtyFourHexDigitsWhateverTheOrderOfTheFiles()
        {
            string forward = GameFingerprint.Compute([Stamp("a.ba2"), Stamp("b.ba2")]);

            Assert.Matches("^[0-9a-f]{64}$", forward);
            Assert.Equal(forward, GameFingerprint.Compute([Stamp("b.ba2"), Stamp("a.ba2")]));
        }

        [Fact]
        public void TheFingerprintSpellsAPathTheSameWayWhateverItsSeparatorsAndCase() =>
            Assert.Equal(GameFingerprint.Compute([Stamp("Video/Intro.bk2")]), GameFingerprint.Compute([Stamp(@"video\intro.BK2")]));

        [Fact]
        public void AChangedSizeATimeANameOrAMissingFileChangesTheFingerprint()
        {
            string baseline = GameFingerprint.Compute([Stamp("a.ba2"), Stamp("b.ba2")]);

            Assert.NotEqual(baseline, GameFingerprint.Compute([Stamp("a.ba2", length: 11), Stamp("b.ba2")]));
            Assert.NotEqual(baseline, GameFingerprint.Compute([Stamp("a.ba2", written: Written.AddTicks(1)), Stamp("b.ba2")]));
            Assert.NotEqual(baseline, GameFingerprint.Compute([Stamp("a.ba2"), Stamp("c.ba2")]));
            Assert.NotEqual(baseline, GameFingerprint.Compute([Stamp("a.ba2")]));
        }

        [Fact]
        public void ScanningTakesTheMasterTheGameArchivesAndTheVideosAndNothingElse()
        {
            Touch("Fallout4.esm");
            Touch("Fallout4 - Meshes.ba2");
            Touch(Path.Combine("Video", "Intro.bk2"));
            string baseline = GameFingerprint.Scan(Data);

            Touch("DLCRobot.esm");
            Touch("DLCRobot - Main.ba2");
            Touch("ccBGSFO4001-PipBoy(Black).ba2");
            Touch("Fallout4 - Meshes.txt");
            Touch(Path.Combine("Textures", "Fallout4 - Loose.ba2"));
            Touch(Path.Combine("Video", "Sub", "Other.bk2"));
            Assert.Equal(baseline, GameFingerprint.Scan(Data));

            Touch("Fallout4 - Textures1.ba2");
            Assert.NotEqual(baseline, GameFingerprint.Scan(Data));
        }

        [Fact]
        public void ScanningSeesARewrittenFileThroughItsSizeOrItsTime()
        {
            Touch("Fallout4 - Meshes.ba2", "one");
            string baseline = GameFingerprint.Scan(Data);

            Touch("Fallout4 - Meshes.ba2", "two", written: Written.AddMinutes(1));
            Assert.NotEqual(baseline, GameFingerprint.Scan(Data));

            Touch("Fallout4 - Meshes.ba2", "three");
            Assert.NotEqual(baseline, GameFingerprint.Scan(Data));
        }

        [Fact]
        public void AGameWithoutADataFolderHasTheFingerprintOfNoFiles() =>
            Assert.Equal(GameFingerprint.Compute([]), GameFingerprint.Scan(Path.Combine(_root.Path, "missing")));

        [Fact]
        public void TheInputsAreTheInstalledLanguageAndTheScannedFiles()
        {
            string game = Path.Combine(_root.Path, "install");
            Directory.CreateDirectory(Path.Combine(game, "Data"));
            File.WriteAllText(Path.Combine(game, "Fallout4_Default.ini"), "[General]\nsLanguage=DE\n");
            File.WriteAllText(Path.Combine(game, "Data", "Fallout4.esm"), "x");
            var locations = new InstallLocations(game, Path.Combine(_root.Path, "out"), Path.Combine(_root.Path, "docs"));

            var inputs = GameInputs.Read(locations);

            Assert.Equal("de", inputs.Language);
            Assert.Equal(GameFingerprint.Scan(locations.GameDataDirectory), inputs.Files);
        }
    }
}
