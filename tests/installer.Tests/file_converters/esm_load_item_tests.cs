using Xunit;

namespace InstallerTests
{
    // Item rows the engine reads: the shared items table, the misc component list and the NPC faction list.
    [Collection(FixtureEsm.Name)]
    public class EsmLoadItemTests(LoadedFixtureDatabase db)
    {
        sealed class SoundRow
        {
            public string? PickupSound { get; init; }
            public string? DropSound { get; init; }
        }

        sealed class ComponentRow
        {
            public required string Component { get; init; }
            public long Count { get; init; }
        }

        sealed class FactionRow
        {
            public required string Faction { get; init; }
            public long Rank { get; init; }
        }

        long IsMarker(string editorId) =>
            db.QuerySingle<long>("SELECT is_marker FROM items WHERE form_key = @key", new { key = db.KeyOf(editorId) });

        SoundRow Sounds(string editorId) =>
            db.QuerySingle<SoundRow>("SELECT pickup_sound, drop_sound FROM items WHERE form_key = @key", new { key = db.KeyOf(editorId) });

        IReadOnlyList<ComponentRow> Components(string editorId) =>
            db.Query<ComponentRow>("SELECT component, count FROM misc_components WHERE form_key = @key ORDER BY rowid", new { key = db.KeyOf(editorId) });

        IReadOnlyList<FactionRow> Factions(string editorId) =>
            db.Query<FactionRow>("SELECT faction, rank FROM npc_factions WHERE npc = @key ORDER BY rowid", new { key = db.KeyOf(editorId) });

        [Theory]
        [InlineData("MarkerMisc", 1)]
        [InlineData("SlashedMarkerMisc", 1)]
        [InlineData("MeshesPrefixedMarkerMisc", 1)]
        [InlineData("MarkerWeapon", 1)]
        [InlineData("MarkerArmor", 1)]
        [InlineData("PlainMisc", 0)]
        [InlineData("NestedMarkersMisc", 0)]
        [InlineData("ModellessMisc", 0)]
        [InlineData("TestGrenade", 0)]
        [InlineData("PlainArmor", 0)]
        public void Item_IsMarkerOnlyWhenItsModelStartsInTheMarkersFolder(string editorId, long expected) =>
            Assert.Equal(expected, IsMarker(editorId));

        [Fact]
        public void Armor_StoresItsPickUpAndPutDownSounds()
        {
            var row = Sounds("SoundArmor");

            Assert.Equal(db.KeyOf("ItemPickupSound"), row.PickupSound);
            Assert.Equal(db.KeyOf("ItemPutDownSound"), row.DropSound);
        }

        [Fact]
        public void Armor_WithoutSounds_LeavesThemNull()
        {
            var row = Sounds("SilentArmor");

            Assert.Null(row.PickupSound);
            Assert.Null(row.DropSound);
        }

        [Fact]
        public void MiscItem_WithComponents_StoresOneRowPerComponentInRecordOrder()
        {
            var rows = Components("ComponentMisc");

            Assert.Equal(
                [(db.KeyOf("ScrapComponent"), (long)LoadedFixtureDatabase.ComponentCount), (db.KeyOf("WireComponent"), (long)LoadedFixtureDatabase.SecondComponentCount)],
                rows.Select(row => (row.Component, row.Count)));
        }

        [Fact]
        public void MiscItem_WithoutComponents_HasNoComponentRows() =>
            Assert.Empty(Components("PlainMisc"));

        [Fact]
        public void Npc_WithFactions_StoresOneRowPerMembershipWithItsRank()
        {
            var rows = Factions("OwnerNpc");

            Assert.Equal(
                [(db.KeyOf("OwnerFaction"), (long)LoadedFixtureDatabase.OwnerFactionRank), (db.KeyOf("SecondFaction"), (long)LoadedFixtureDatabase.SecondFactionRank)],
                rows.Select(row => (row.Faction, row.Rank)));
        }

        long ChanceNone(string editorId) =>
            db.QuerySingle<long>("SELECT chance_none FROM leveled_items WHERE form_key = @key", new { key = db.KeyOf(editorId) });

        [Fact]
        public void LeveledItem_StoresItsListLevelChanceNone()
        {
            Assert.Equal((long)LoadedFixtureDatabase.LeveledListChanceNone, ChanceNone("ChanceLeveledList"));
            Assert.Equal(0, ChanceNone("FixtureLeveledList"));
        }

        [Fact]
        public void LeveledNpc_StoresItsListLevelChanceNone() =>
            Assert.Equal((long)LoadedFixtureDatabase.LeveledListChanceNone,
                db.QuerySingle<long>("SELECT chance_none FROM leveled_npcs WHERE form_key = @key", new { key = db.KeyOf("ChanceLeveledNpcList") }));

        [Fact]
        public void Npc_WithoutFactions_HasNoFactionRows() =>
            Assert.Empty(Factions("FactionlessNpc"));
    }
}
