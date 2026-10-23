using AssetConverter;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using InstallerTests.Archives;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests.Pipeline
{
    // Drives the process-wide InstallProgress, so it runs serially with the other tests that use it.
    [Collection("Console redirection")]
    public sealed class PipelineStagesTests : IDisposable
    {
        private readonly InstallerTests.TempDirectory _root = new();

        public PipelineStagesTests() => InstallProgress.Start([new StepDefinition(InstallSteps.ExtractInterface)]);

        public void Dispose()
        {
            InstallProgress.Stop();
            _root.Dispose();
        }

        private InstallLocations Locations => new(Path.Combine(_root.Path, "game"), Path.Combine(_root.Path, "install"), Path.Combine(_root.Path, "settings"));

        private static readonly byte[] FontConfig =
            System.Text.Encoding.UTF8.GetBytes("fontlib \"Interface\\fonts_en.swf\"\nmap \"$Main\" = \"Main Family\" Normal\n");

        private static readonly byte[] FontLibrary = new SwfMovieBuilder().Font(1, "Main Family", layout: (19456, 5120, 1024)).Build();

        private static readonly byte[] SampleMenu = new SyntheticMenuSwfFixture().Build();

        private static readonly byte[] OtherMenu = new SwfMovieBuilder().Build();

        private static readonly Dictionary<string, ArtMovie> ArtMovies = SyntheticArtArchive.Movies();

        /// <summary>A game whose interface archive holds the installed language's translate table, font configuration and font library, the movies of the Pip-Boy art and two menus.</summary>
        private void CreateGame()
        {
            Directory.CreateDirectory(Path.Combine(Locations.Fo4Location, "Data"));
            File.WriteAllText(Path.Combine(Locations.Fo4Location, "Fallout4_Default.ini"), "[General]\nsLanguage=en\n");
            byte[] translate = [.. System.Text.Encoding.Unicode.GetPreamble(), .. System.Text.Encoding.Unicode.GetBytes("$Ok\tOK\r\n")];
            File.WriteAllBytes(Path.Combine(Locations.Fo4Location, "Data", "Fallout4 - Interface.ba2"), Ba2Builder.Gnrl(
            [
                new Ba2File(@"Interface\Translate_en.txt", translate, Compress: false),
                new Ba2File(@"Interface\FontConfig.txt", FontConfig, Compress: false),
                new Ba2File(@"Interface\fonts_en.swf", FontLibrary, Compress: false),
                new Ba2File(@"Interface\SampleMenu.swf", SampleMenu, Compress: false),
                new Ba2File(@"Interface\OtherMenu.swf", OtherMenu, Compress: false),
                .. ArtMovies.Values.Select(movie => new Ba2File(@"Interface\" + movie.Source, movie.Content, Compress: false)),
            ]));
        }

        private static StepSnapshot Step() => InstallProgress.Model!.GetSnapshot().Steps.Single(s => s.Name == InstallSteps.ExtractInterface);

        /// <summary>Every interface output as a current conversion of the game: each with the stamp its extractor records.</summary>
        private void CacheInterfaceAssets()
        {
            CreateGame();
            var config = SourceFingerprint.Of(@"Interface\FontConfig.txt", FontConfig);
            Directory.CreateDirectory(Locations.FontsOut);
            File.WriteAllBytes(Path.Combine(Locations.FontsOut, "cached.ttf"), [0]);
            string ffdec = FfdecVersionOrSkip();
            new ConversionStamp(FontExtractor.ExtractorVersion, "en", [config, SourceFingerprint.Of(@"Interface\fonts_en.swf", FontLibrary)], ffdec)
                .WriteSidecar(Locations.FontsOut);
            AssetConverter.UiLayout.UiLayoutExtractor.Extract(Locations.Fo4Location, Locations.SettingsDirectory, Locations.UiLayoutOut);
            Directory.CreateDirectory(Locations.PipboyArtOut);
            File.WriteAllText(InstallLayout.PipboyArtManifestFile(Locations.AssetsRoot), $"{{\"format\":{PipboyArtWriter.ManifestFormat},\"clips\":[]}}");
            var sources = ArtMovies.Values.OrderBy(movie => movie.Source, StringComparer.OrdinalIgnoreCase)
                .Select(movie => SourceFingerprint.Of(@"Interface\" + movie.Source, movie.Content));
            new ConversionStamp(PipboyArtExtractor.ExtractorVersion, null, [.. sources], ffdec).WriteSidecar(Locations.PipboyArtOut);
        }

        /// <summary>The ffdec version the converted outputs are stamped with; the test cannot run on a machine without the tool.</summary>
        private static string FfdecVersionOrSkip()
        {
            try
            {
                return Ffdec.Version(MemoryBudget.Unlimited(), TestContext.Current.CancellationToken);
            }
            catch (FfdecException e)
            {
                Assert.Skip($"ffdec is not usable here: {e.Message}");
                throw;
            }
        }

        [Fact]
        public void ExtractInterfaceAssets_EveryInterfaceAssetAlreadyExtracted_ShowsTheStepAsSkipped()
        {
            CacheInterfaceAssets();

            new PipelineStages(Locations, MemoryBudget.Unlimited(), TestContext.Current.CancellationToken).ExtractInterfaceAssets();

            Assert.True(Step().WasSkipped);
            Assert.Equal(0, Step().Completed);
        }

        [Fact]
        public void ExtractInterfaceAssets_Cancelled_ThrowsBeforeExtractingAnything()
        {
            using var cts = new CancellationTokenSource();
            cts.Cancel();

            Assert.ThrowsAny<OperationCanceledException>(() => new PipelineStages(Locations, MemoryBudget.Unlimited(), cts.Token).ExtractInterfaceAssets());

            Assert.False(Directory.Exists(Locations.FontsOut));
        }
    }
}
