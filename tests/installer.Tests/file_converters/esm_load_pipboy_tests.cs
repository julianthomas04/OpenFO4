using Xunit;

namespace InstallerTests
{
    // Pip-Boy tables the engine reads (src/engine/src/assets/db/{perks,map_markers,worldspaces}.rs): the world map
    // image and markers, the perk columns and the perk chart placement.
    [Collection(FixtureEsm.Name)]
    public class EsmLoadPipboyTests(LoadedFixtureDatabase db)
    {
        sealed class MapMarkerRow
        {
            public required string FormKey { get; init; }
            public required string Worldspace { get; init; }
            public string? Name { get; init; }
            public long MarkerType { get; init; }
            public long Flags { get; init; }
            public double PositionX { get; init; }
            public double PositionY { get; init; }
            public double PositionZ { get; init; }
        }

        sealed class PerkRow
        {
            public string? Name { get; init; }
            public string? NextPerk { get; init; }
            public long Level { get; init; }
            public long NumRanks { get; init; }
            public long Playable { get; init; }
            public long Hidden { get; init; }
            public string? SwfPath { get; init; }
            public string? FirstRank { get; init; }
        }

        sealed class PerkChartRow
        {
            public required string Perk { get; init; }
            public required string Special { get; init; }
            public long RequiredSpecial { get; init; }
        }

        [Fact]
        public void Worldspace_MapImage_IsStoredAsAuthored()
        {
            var image = db.QuerySingle<string>("SELECT map_image FROM worldspaces WHERE form_key = @key", new { key = db.KeyOf("FixtureWorld") });

            Assert.Equal(LoadedFixtureDatabase.FixtureMapImage, image);
        }

        [Fact]
        public void MapMarkers_PersistentCellMarkerRef_IsStoredWithNameTypeFlagsAndPosition()
        {
            var markers = db.Query<MapMarkerRow>("SELECT * FROM map_markers");

            var marker = Assert.Single(markers);
            Assert.Equal(db.KeyOf("FixtureMarkerRef"), marker.FormKey);
            Assert.Equal(db.KeyOf("FixtureWorld"), marker.Worldspace);
            Assert.Equal(LoadedFixtureDatabase.MarkerName, marker.Name);
            Assert.Equal(LoadedFixtureDatabase.MarkerType, marker.MarkerType);
            Assert.Equal(LoadedFixtureDatabase.MarkerFlags, marker.Flags);
            Assert.Equal((LoadedFixtureDatabase.MarkerPosition.X, LoadedFixtureDatabase.MarkerPosition.Y, LoadedFixtureDatabase.MarkerPosition.Z),
                ((float)marker.PositionX, (float)marker.PositionY, (float)marker.PositionZ));
        }

        [Fact]
        public void MapMarkers_PersistentRefWithoutMarkerData_IsNotAMarker()
        {
            var plainRefs = db.Query<string>("SELECT form_key FROM map_markers WHERE form_key = @key", new { key = db.KeyOf("WorldPersistentRef") });

            Assert.Empty(plainRefs);
        }

        sealed class SpecialAttributeRow
        {
            public required string Special { get; init; }
            public required string FormKey { get; init; }
            public string? Name { get; init; }
            public string? Description { get; init; }
        }

        [Fact]
        public void SpecialAttributes_EverySpecialRecord_IsStoredWithItsNameAndDescription()
        {
            var rows = db.Query<SpecialAttributeRow>("SELECT * FROM special_attributes ORDER BY special");

            Assert.Equal(
                new[] { "agility", "charisma", "endurance", "intelligence", "luck", "perception", "strength" },
                rows.Select(row => row.Special));
            var strength = rows.Single(row => row.Special == "strength");
            Assert.Equal(db.KeyOf("Strength"), strength.FormKey);
            Assert.Equal((LoadedFixtureDatabase.StrengthName, LoadedFixtureDatabase.StrengthDescription), (strength.Name, strength.Description));
        }

        [Fact]
        public void TableReload_SpecialAttributes_AlsoRefillsActorValues()
        {
            var plan = AssetConverter.TableReloadPlan.Create(["special_attributes"]);

            Assert.Equal(["actor_value_informations", "special_attributes"], plan.Tables);
            Assert.Equal(["AVIF"], plan.RecordTypes);
        }

        PerkRow Perk(string editorId) =>
            db.QuerySingle<PerkRow>("SELECT * FROM perks WHERE form_key = @key", new { key = db.KeyOf(editorId) });

        [Fact]
        public void Perk_LevelRanksFlagsAndMovie_AreStored()
        {
            var perk = Perk("FixtureCyclicPerk1");

            Assert.Equal((0, 2, 1, 0), (perk.Level, perk.NumRanks, perk.Playable, perk.Hidden));
            Assert.Equal(LoadedFixtureDatabase.PerkMovie, perk.SwfPath);
            Assert.Equal(db.KeyOf("FixtureCyclicPerk2"), perk.NextPerk);
        }

        [Fact]
        public void Perk_HiddenAndUnplayableFlags_AreDistinct()
        {
            Assert.Equal((1, 1), (Perk("FixtureHiddenPerk").Playable, Perk("FixtureHiddenPerk").Hidden));
            Assert.Equal((0, 0), (Perk("FixtureUnplayablePerk").Playable, Perk("FixtureUnplayablePerk").Hidden));
        }

        [Fact]
        public void Perk_WithoutMovie_HasNullSwfPath()
        {
            Assert.Null(Perk("FixtureCyclicPerk2").SwfPath);
        }

        Dictionary<string, PerkChartRow> Chart() =>
            db.Query<PerkChartRow>("SELECT * FROM perk_chart").ToDictionary(row => row.Perk);

        [Fact]
        public void PerkChart_ChainWhoseLastRankLinksBack_ListsOnlyTheLowestLevelRankWithColumnAndRow()
        {
            var chart = Chart();

            var row = chart[db.KeyOf("FixtureCyclicPerk1")];
            Assert.Equal(("strength", 6), (row.Special, row.RequiredSpecial));
            Assert.DoesNotContain(db.KeyOf("FixtureCyclicPerk2"), chart.Keys);
        }

        [Fact]
        public void PerkChart_OpenChain_ListsThePerkNothingLinksTo()
        {
            var chart = Chart();

            var row = chart[db.KeyOf("FixtureOpenPerk1")];
            Assert.Equal(("perception", 4), (row.Special, row.RequiredSpecial));
            Assert.DoesNotContain(db.KeyOf("FixtureOpenPerk2"), chart.Keys);
        }

        [Fact]
        public void PerkChart_ExcludesHiddenUnplayableConditionlessSpecialRaisingAndTwoSpecialPerks()
        {
            var chart = Chart();

            Assert.Equal(
                new[] { db.KeyOf("FixtureCyclicPerk1"), db.KeyOf("FixtureOpenPerk1"), db.KeyOf("FixtureSharedCellPerk") }.Order(),
                chart.Keys.Order());
        }

        [Fact]
        public void PerkChart_PerksSharingACell_AreBothStored()
        {
            var chart = Chart();

            var cell = (chart[db.KeyOf("FixtureCyclicPerk1")].Special, chart[db.KeyOf("FixtureCyclicPerk1")].RequiredSpecial);
            Assert.Equal(cell, (chart[db.KeyOf("FixtureSharedCellPerk")].Special, chart[db.KeyOf("FixtureSharedCellPerk")].RequiredSpecial));
        }

        [Fact]
        public void Perk_EveryRecord_StoresTheFirstRankOfItsChain()
        {
            Assert.Equal(db.KeyOf("FixtureCyclicPerk1"), Perk("FixtureCyclicPerk1").FirstRank);
            Assert.Equal(db.KeyOf("FixtureCyclicPerk1"), Perk("FixtureCyclicPerk2").FirstRank);
            Assert.Equal(db.KeyOf("FixtureOpenPerk1"), Perk("FixtureOpenPerk2").FirstRank);
            Assert.Equal(db.KeyOf("FixtureHiddenPerk"), Perk("FixtureHiddenPerk").FirstRank);
        }

        [Fact]
        public void PerkChart_EveryEntry_NamesAPerkThatIsItsOwnFirstRank()
        {
            var chart = Chart();

            Assert.All(chart.Keys, perk =>
                Assert.Equal(perk, db.QuerySingle<string>("SELECT first_rank FROM perks WHERE form_key = @perk", new { perk })));
        }

        [Fact]
        public void PerkChart_PerkWithoutAuthoredMovie_GetsTheMovieNamedAfterItsFormId()
        {
            string key = db.KeyOf("FixtureOpenPerk1");
            uint id = Mutagen.Bethesda.Plugins.FormKey.Factory(key).ID;

            Assert.Equal($@"Components\VaultBoys\Perks\PerkClip_{id:x}.swf", Perk("FixtureOpenPerk1").SwfPath);
        }

        [Fact]
        public void PerkChart_PerkWithAuthoredMovie_KeepsIt()
        {
            Assert.Equal(LoadedFixtureDatabase.PerkMovie, Perk("FixtureCyclicPerk1").SwfPath);
        }

        [Fact]
        public void TableReload_PerksTable_AlsoRefillsPerkChart()
        {
            var plan = AssetConverter.TableReloadPlan.Create(["perk_chart"]);

            Assert.Equal(["perk_chart", "perks"], plan.Tables);
            Assert.Equal(["PERK"], plan.RecordTypes);
        }

        [Fact]
        public void TableReload_MapMarkersTable_AlsoRefillsWorldspaces()
        {
            var plan = AssetConverter.TableReloadPlan.Create(["map_markers"]);

            Assert.Equal(["map_markers", "worldspaces"], plan.Tables);
            Assert.Equal(["WRLD"], plan.RecordTypes);
        }
    }
}
