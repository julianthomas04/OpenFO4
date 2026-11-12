using Xunit;

namespace InstallerTests
{
    // The map entry of a region (RDMP): the name the in-game map shows and its priority among overlapping regions
    // (src/engine/src/assets/db/map_regions.rs).
    [Collection(FixtureEsm.Name)]
    public class EsmLoadMapRegionTests(LoadedFixtureDatabase db)
    {
        (string? Name, long? MapPriority) Region(string editorId) =>
            db.QuerySingle<(string?, long?)>("SELECT name, map_priority FROM regions WHERE form_key = @key", new { key = db.KeyOf(editorId) });

        [Fact]
        public void Region_WithAMapEntry_StoresTheMapNameAndPriority()
        {
            Assert.Equal((LoadedFixtureDatabase.MapRegionName, (long?)LoadedFixtureDatabase.MapRegionPriority), Region("MapRegion"));
        }

        [Fact]
        public void Region_WithoutAMapEntry_StoresNulls()
        {
            Assert.Equal((null, null), Region("RegionWithoutMap"));
        }

        [Fact]
        public void Region_WithAnEmptyMapName_StoresNoNameButKeepsThePriority()
        {
            Assert.Equal((null, (long?)LoadedFixtureDatabase.NamelessMapRegionPriority), Region("NamelessMapRegion"));
        }
    }
}
