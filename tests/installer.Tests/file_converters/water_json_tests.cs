using System.Text.Json;
using AssetConverter;
using Mutagen.Bethesda;
using Mutagen.Bethesda.Fallout4;
using Mutagen.Bethesda.Plugins;
using Xunit;

namespace InstallerTests
{
    // WaterJson.Project edge cases; read failures go to the shared tally, so these run in the collection that owns it.
    [Collection(FixtureEsm.Name)]
    public class WaterJsonTests
    {
        static Water NewWater() => new Fallout4Mod(ModKey.FromNameAndExtension("Scratch.esp"), Fallout4Release.Fallout4).Waters.AddNew("Scratch");

        [Fact]
        public void Project_FlagBitsNoEnumMemberNames_AreKeptAsHexString()
        {
            var water = NewWater();
            water.Flags = Water.Flag.Dangerous | (Water.Flag)0x40;

            var flags = WaterJson.Project(water)["Flags"]!.AsArray().Select(n => n!.GetValue<string>());

            Assert.Equal(["Dangerous", "0x40"], flags);
        }

        [Fact]
        public void Project_NonFiniteFloat_IsTalliedOnceAndOmitted()
        {
            const string site = "WATR FogDepthAmount";
            int before = ReadFailureTally.Shared.CountFor(site);
            var water = NewWater();
            water.FogDepthAmount = float.NaN;

            var json = WaterJson.Project(water);

            Assert.False(json.ContainsKey("FogDepthAmount"));
            Assert.Equal(before + 1, ReadFailureTally.Shared.CountFor(site));
        }

        [Fact]
        public void Project_FiniteFloat_IsKeptAndNotTallied()
        {
            const string site = "WATR FogDepthAmount";
            int before = ReadFailureTally.Shared.CountFor(site);
            var water = NewWater();
            water.FogDepthAmount = 1.5f;

            var json = WaterJson.Project(water);

            Assert.Equal(1.5f, json["FogDepthAmount"]!.GetValue<float>());
            Assert.Equal(before, ReadFailureTally.Shared.CountFor(site));
        }
    }
}
