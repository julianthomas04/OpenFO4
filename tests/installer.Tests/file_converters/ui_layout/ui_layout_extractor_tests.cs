using System.Text;
using System.Text.Json;
using AssetConverter;
using AssetConverter.Swf;
using AssetConverter.UiLayout;
using Xunit;

namespace InstallerTests.UiLayout
{
    // The extractor prints progress to the console, which other tests redirect.
    [Collection("Console redirection")]
    public sealed class UiLayoutExtractorTests : IDisposable
    {
        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private string OutDir => Path.Combine(_root.Path, "layout");

        private static byte[] Utf16(string text) => [.. Encoding.Unicode.GetPreamble(), .. Encoding.Unicode.GetBytes(text)];

        private static byte[] Movie(double frameRate = 30)
        {
            var swf = new SwfMovieBuilder { FrameRate = frameRate };
            swf.Shape(1, new TwipRect(0, 0, 20, 20), new SwfColor(1, 2, 3, 255));
            swf.Root.Place(new SwfPlacement(1) { Character = 1, Name = "dot" }).ShowFrame();
            return swf.Build();
        }

        private static readonly byte[] FontLibrary = new SwfMovieBuilder()
            .Font(1, "Default Family", layout: (19456, 5120, 1024))
            .Font(2, "Language Family", bold: true, layout: (17152, 4198, 0))
            .Build();

        private static List<(string Path, byte[] Content)> Entries(params (string Path, byte[] Content)[] extra) =>
        [
            (@"Interface\Translate_xx.txt", Utf16("$Ok\tOkay\r\n")),
            (@"Interface\fonts_xx.swf", FontLibrary),
            (@"Interface\FontConfig.txt", Encoding.UTF8.GetBytes("fontlib \"Interface\\fonts_xx.swf\"\nmap \"$Main\" = \"Default Family\" Normal\n")),
            (@"Interface\Menu.swf", Movie()),
            .. extra,
        ];

        private bool Convert(IReadOnlyList<(string Path, byte[] Content)> entries) => UiLayoutExtractor.Convert(entries, "xx", OutDir);

        private JsonElement Read(string relativePath) => JsonDocument.Parse(File.ReadAllBytes(Path.Combine(OutDir, relativePath))).RootElement;

        [Theory]
        [InlineData(@"Interface\SampleMenu.swf", true)]
        [InlineData(@"interface\samplemenu.SWF", true)]
        [InlineData(@"Interface\Components\ConditionClips\Sample_Clip.swf", true)]
        [InlineData(@"Interface\Components\ConditionClips\Deeper\X.swf", false)]
        [InlineData(@"Interface\Components\Other.swf", false)]
        [InlineData(@"Interface\fonts_en.swf", false)]
        [InlineData(@"Interface\Translate_en.txt", false)]
        [InlineData(@"Textures\Interface\X.swf", false)]
        public void IsMovie_TopLevelMoviesAndConditionClipsExceptFontLibraries(string path, bool expected)
        {
            Assert.Equal(expected, UiLayoutExtractor.IsMovie(path));
        }

        [Theory]
        [InlineData(@"Interface\SamplePage.swf", "SamplePage")]
        [InlineData(@"Interface\Components\ConditionClips\Sample_Clip.swf", "Components/ConditionClips/Sample_Clip")]
        public void MovieName_IsThePathBelowInterfaceWithoutExtension(string path, string expected)
        {
            Assert.Equal(expected, UiLayoutExtractor.MovieName(path));
        }

        [Fact]
        public void Convert_WritesEveryMovieTheTranslateTableAndTheFontConfiguration()
        {
            Assert.True(Convert(Entries((@"Interface\Components\ConditionClips\Sample_Clip.swf", Movie()))));

            Assert.Equal("movie", Read("Menu.json").GetProperty("kind").GetString());
            Assert.Equal("Components/ConditionClips/Sample_Clip", Read("Components/ConditionClips/Sample_Clip.json").GetProperty("movie").GetString());
            Assert.Equal("Okay", Read(InstallLayout.TranslateFileName).GetProperty("entries").GetProperty("$Ok").GetString());
            Assert.Equal("Default Family", Read(InstallLayout.FontConfigFileName).GetProperty("maps")[0].GetProperty("family").GetString());
            Assert.Empty(Directory.GetFiles(OutDir, "*.tmp", SearchOption.AllDirectories));
        }

        [Fact]
        public void Convert_LanguageFontConfiguration_IsPreferredOverTheDefault()
        {
            Convert(Entries((@"Interface\FontConfig_xx.txt",
                Encoding.UTF8.GetBytes("fontlib \"Interface\\fonts_xx.swf\"\nmap \"$Main\" = \"Language Family\" Bold\n"))));

            JsonElement config = Read(InstallLayout.FontConfigFileName);
            Assert.Equal("Language Family", config.GetProperty("maps")[0].GetProperty("family").GetString());
            Assert.Equal([@"Interface\FontConfig_xx.txt", @"Interface\fonts_xx.swf"],
                config.GetProperty("conversion").GetProperty("sources").EnumerateArray().Select(source => source.GetProperty("entry").GetString()));
        }

        [Fact]
        public void Convert_RemovesOnlyTheDocumentsItWroteForMoviesThatAreGone_AndItsLeftovers()
        {
            Convert(Entries());
            File.WriteAllText(Path.Combine(OutDir, "Gone.json"), """{"kind":"movie","conversion":{}}""");
            File.WriteAllText(Path.Combine(OutDir, "notes.json"), """{"note":"mine"}""");
            File.WriteAllText(Path.Combine(OutDir, "other.json"), """{"kind":"other","conversion":{}}""");
            File.WriteAllText(Path.Combine(OutDir, "broken.json"), "not json");
            File.WriteAllText(Path.Combine(OutDir, "readme.txt"), "keep");
            File.WriteAllText(Path.Combine(OutDir, "Half.json.tmp"), "{");

            Convert(Entries());

            Assert.False(File.Exists(Path.Combine(OutDir, "Gone.json")));
            Assert.False(File.Exists(Path.Combine(OutDir, "Half.json.tmp")));
            foreach (string kept in new[] { "notes.json", "other.json", "broken.json", "readme.txt", "Menu.json" })
                Assert.True(File.Exists(Path.Combine(OutDir, kept)), kept);
        }

        /// <summary>The document extractor version and the SHA-256 of the translate and font configuration documents written for the entries below.</summary>
        private static readonly (int ExtractorVersion, string Sha256) Golden = (5, "9939c03737c798e1f62aa4452aaebbd2ca40eefa46a0d8d7aff5f27fffdd2f6c");

        [Fact]
        public void Convert_ChangedTableOrConfigurationOutput_RaisesTheExtractorVersion()
        {
            Convert(Entries());

            string written = System.Convert.ToHexString(System.Security.Cryptography.SHA256.HashData(
                [.. File.ReadAllBytes(Path.Combine(OutDir, InstallLayout.TranslateFileName)), .. File.ReadAllBytes(Path.Combine(OutDir, InstallLayout.FontConfigFileName))])).ToLowerInvariant();

            Assert.True(Golden == (UiDocumentHeader.ExtractorVersion, written),
                $"The translate or font configuration document changed (SHA-256 {written}). Installed documents are reused unless UiDocumentHeader.ExtractorVersion is raised: "
                + "raise it and record the new version and hash in Golden.");
        }

        [Fact]
        public void Convert_FontConfiguration_CarriesTheMetricsOfEveryMappedFont()
        {
            Convert(Entries());

            Assert.Equal("""[{"family":"Default Family","style":"Normal","library":"Interface\\fonts_xx.swf","emSize":20480,"ascent":19456,"descent":5120,"leading":1024}]""",
                Read(InstallLayout.FontConfigFileName).GetProperty("fonts").GetRawText());
        }

        [Fact]
        public void Convert_FontNoLibraryEmbeds_FailsNamingTheConfiguration()
        {
            var entries = Entries().Where(e => !e.Path.EndsWith("fonts_xx.swf", StringComparison.Ordinal))
                .Append((@"Interface\fonts_xx.swf", new SwfMovieBuilder().Font(1, "Other Family", layout: (1, 1, 1)).Build())).ToList();

            var thrown = Assert.Throws<UiLayoutException>(() => Convert(entries));

            Assert.Contains("FontConfig.txt", thrown.Message);
            Assert.Contains("Default Family", thrown.Message);
        }

        [Fact]
        public void Convert_FontLibraryMissingFromTheArchive_Throws()
        {
            var entries = Entries().Where(e => !e.Path.EndsWith("fonts_xx.swf", StringComparison.Ordinal)).ToList();

            Assert.Contains("fonts_xx.swf", Assert.Throws<UiLayoutException>(() => Convert(entries)).Message);
        }

        [Fact]
        public void Convert_SameFilesForAnotherLanguage_ConvertsTheLanguageDocumentsAgain()
        {
            Convert(Entries());
            DateTime movie = File.GetLastWriteTimeUtc(Path.Combine(OutDir, "Menu.json"));

            Assert.True(UiLayoutExtractor.Convert(Entries((@"Interface\Translate_yy.txt", Utf16("$Ok\tJa\r\n"))), "yy", OutDir));

            Assert.Equal("yy", Read(InstallLayout.FontConfigFileName).GetProperty("conversion").GetProperty("language").GetString());
            Assert.Equal(movie, File.GetLastWriteTimeUtc(Path.Combine(OutDir, "Menu.json")));
        }

        [Fact]
        public void Convert_SecondRunOverTheSameFiles_WritesNothing()
        {
            Convert(Entries());
            DateTime written = File.GetLastWriteTimeUtc(Path.Combine(OutDir, "Menu.json"));

            Assert.False(Convert(Entries()));
            Assert.Equal(written, File.GetLastWriteTimeUtc(Path.Combine(OutDir, "Menu.json")));
        }

        [Fact]
        public void Convert_ChangedMovie_IsConvertedAgain()
        {
            Convert(Entries());
            var changed = Entries().Select(e => e.Path.EndsWith("Menu.swf", StringComparison.Ordinal) ? (e.Path, Movie(frameRate: 12)) : e).ToList();

            Assert.True(Convert(changed));
            Assert.Equal(12, Read("Menu.json").GetProperty("fps").GetDouble());
        }

        [Fact]
        public void Convert_DocumentOfAMovieNoLongerShipped_IsDeleted()
        {
            Convert(Entries((@"Interface\Old.swf", Movie())));

            Convert(Entries());

            Assert.False(File.Exists(Path.Combine(OutDir, "Old.json")));
            Assert.True(File.Exists(Path.Combine(OutDir, "Menu.json")));
        }

        [Fact]
        public void Convert_MalformedMovie_FailsNamingItAndKeepsTheOthers()
        {
            var thrown = Assert.Throws<UiLayoutException>(() => Convert(Entries((@"Interface\Broken.swf", [0x46, 0x57, 0x53, 10]))));

            Assert.Contains(@"Interface\Broken.swf", thrown.Message);
            Assert.False(File.Exists(Path.Combine(OutDir, "Broken.json")));
            Assert.True(File.Exists(Path.Combine(OutDir, "Menu.json")));
        }

        [Fact]
        public void Convert_NoTranslateTableForTheLanguage_Throws()
        {
            var thrown = Assert.Throws<UiLayoutException>(() => UiLayoutExtractor.Convert(Entries(), "de", OutDir));

            Assert.Contains("Translate_de.txt", thrown.Message);
        }

        [Fact]
        public void Convert_NoFontConfiguration_Throws()
        {
            var entries = Entries().Where(e => !e.Path.Contains("FontConfig", StringComparison.Ordinal)).ToList();

            Assert.Throws<UiLayoutException>(() => Convert(entries));
        }

        [Fact]
        public void Convert_MalformedTranslateTable_FailsNamingIt()
        {
            var entries = Entries().Select(e => e.Path.EndsWith(".txt", StringComparison.Ordinal) && e.Path.Contains("Translate", StringComparison.Ordinal) ? (e.Path, Utf16("no key here\n")) : e).ToList();

            var thrown = Assert.Throws<UiLayoutException>(() => Convert(entries));

            Assert.Contains("Translate_xx.txt", thrown.Message);
        }

        [Fact]
        public void Convert_TwoEntriesWritingOneDocument_Throws()
        {
            Assert.Throws<UiLayoutException>(() => Convert(Entries((@"Interface\MENU.swf", Movie()))));
        }
    }
}
