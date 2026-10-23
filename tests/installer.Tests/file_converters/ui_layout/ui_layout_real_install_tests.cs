using System.Text.Json;
using AssetConverter;
using AssetConverter.UiLayout;
using Xunit;

namespace InstallerTests.UiLayout
{
    /// <summary>
    /// Checks against the Fallout 4 install named by <c>OPENFO4_FO4_PATH</c>: every interface movie converts and the values the
    /// installer's own readers take from the movies are there. Explicit: run with <c>-- xUnit.Explicit=only</c>; they read the
    /// game's interface archive and so never run in the regular suite.
    /// </summary>
    [Collection("Console redirection")]
    public sealed class UiLayoutRealInstallTests : IDisposable
    {
        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private static string Game()
        {
            string? path = Environment.GetEnvironmentVariable("OPENFO4_FO4_PATH");
            Assert.SkipWhen(string.IsNullOrEmpty(path), "OPENFO4_FO4_PATH names no Fallout 4 install.");
            return path!;
        }

        private static string Settings() => GameIniFiles.DocumentsDirectory();

        [Fact(Explicit = true)]
        public void EveryInterfaceMovie_Converts()
        {
            string game = Game();
            string outDir = Path.Combine(_root.Path, "layout");

            UiLayoutExtractor.Extract(game, Settings(), outDir);

            var documents = Directory.GetFiles(outDir, "*.json", SearchOption.AllDirectories);
            Assert.Contains(documents, d => Path.GetFileName(d) == InstallLayout.TranslateFileName);
            foreach (string document in documents.Where(d => !d.EndsWith(InstallLayout.TranslateFileName, StringComparison.Ordinal) && !d.EndsWith(InstallLayout.FontConfigFileName, StringComparison.Ordinal)))
            {
                using var json = JsonDocument.Parse(File.ReadAllBytes(document));
                Assert.True(json.RootElement.GetProperty("instances").EnumerateObject().Any(), $"{document} lists no instance.");
                Assert.Equal(JsonValueKind.Object, json.RootElement.GetProperty("referenceLists").ValueKind);
            }
        }

        [Fact(Explicit = true)]
        public void PipboyPages_StateTheValuesTheArtExportReads()
        {
            string game = Game();
            string outDir = Path.Combine(_root.Path, "layout");

            UiLayoutExtractor.Extract(game, Settings(), outDir);

            using var inventory = JsonDocument.Parse(File.ReadAllBytes(Path.Combine(outDir, "Pipboy_InvPage.json")));
            Assert.True(inventory.RootElement.GetProperty("constants").GetProperty(PipboyArtCatalog.DamageTypeCountConstant).GetInt32() > 0);
            using var map = JsonDocument.Parse(File.ReadAllBytes(Path.Combine(outDir, "Pipboy_MapPage.json")));
            Assert.True(map.RootElement.GetProperty("referenceLists").GetProperty(PipboyArtCatalog.MarkerListMember).GetArrayLength() > 0);
        }
    }
}
