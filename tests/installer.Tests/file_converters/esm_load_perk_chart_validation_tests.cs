using AssetConverter;
using AssetConverter.Pipeline;
using Mutagen.Bethesda;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Xunit;

namespace InstallerTests
{
    // The base game's chart must fill all 70 cells; the loader refuses an install where it does not.
    public class EsmLoadPerkChartValidationTests
    {
        static async Task LoadSinglePerkPlugin(string fileName)
        {
            using var directory = new TempDirectory();
            var mod = new Fallout4Mod(ModKey.FromNameAndExtension(fileName), Fallout4Release.Fallout4);
            mod.Perks.AddNew("LonelyPerk").Playable = true;
            string esmPath = Path.Combine(directory.Path, fileName);
            mod.WriteToBinary(esmPath);

            try
            {
                await new ESMLoader(Path.Combine(directory.Path, "data.db")) { Memory = MemoryBudget.Unlimited() }.LoadESMAsync(esmPath, ct: TestContext.Current.CancellationToken);
            }
            finally
            {
                Microsoft.Data.Sqlite.SqliteConnection.ClearAllPools();
            }
        }

        [Fact]
        public async Task BaseGamePlugin_WithAnIncompleteChart_FailsTheLoad()
        {
            var error = await Assert.ThrowsAsync<InvalidDataException>(() => LoadSinglePerkPlugin("Fallout4.esm"));

            Assert.Contains("perk chart is empty", error.Message);
        }

        [Fact]
        public async Task OtherPlugin_WithAnIncompleteChart_Loads()
        {
            await LoadSinglePerkPlugin("Other.esm");
        }
    }
}
