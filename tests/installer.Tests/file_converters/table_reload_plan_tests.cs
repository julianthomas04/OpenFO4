using AssetConverter;
using Xunit;

namespace InstallerTests
{
    // The selection is expanded to the closure of jobs that
    // touch a selected table, because a job always fills all of its tables.
    public class TableReloadPlanTests
    {
        [Fact]
        public void Create_TableOwnedByOneJob_PlansExactlyThatTableAndRecordType()
        {
            var plan = TableReloadPlan.Create(["keywords"]);

            Assert.Equal(["keywords"], plan.Tables);
            Assert.Equal(["KYWD"], plan.RecordTypes);
            Assert.Equal(["keywords"], plan.Requested);
            Assert.False(plan.NeedsRawScans);
            Assert.False(plan.ReloadsMaterialSwaps);
        }

        [Fact]
        public void Create_WeaponsTable_AlsoReloadsTablesSharingAProcessor()
        {
            var plan = TableReloadPlan.Create(["weapons"]);

            Assert.Contains("items", plan.Tables);
            Assert.Contains("armors", plan.Tables);
            Assert.Contains("WEAP", plan.RecordTypes);
            Assert.Contains("ARMO", plan.RecordTypes);
            Assert.Equal(["weapons"], plan.Requested);
        }

        [Fact]
        public void Create_CellsTable_PullsInPlacedObjectsTeleportsAndLandscapesWithRawScans()
        {
            var plan = TableReloadPlan.Create(["cells"]);

            Assert.Equal(["cells", "landscapes", "placed_objects", "placed_teleports"], plan.Tables);
            Assert.Equal(["CELL", "REFR", "LAND"], plan.RecordTypes);
            Assert.True(plan.NeedsRawScans);
        }

        [Fact]
        public void Create_PlacedObjectsRequested_ReloadsWholeCellsJob() =>
            Assert.Contains("cells", TableReloadPlan.Create(["placed_objects"]).Tables);

        [Fact]
        public void Create_MaterialSwapEntries_ReloadsSwapsAndBaseSwapsAndFlagsMaterialSwapReload()
        {
            var plan = TableReloadPlan.Create(["material_swap_entries"]);

            Assert.Equal(["base_material_swaps", "material_swap_entries", "material_swaps"], plan.Tables);
            Assert.True(plan.ReloadsMaterialSwaps);
            Assert.True(plan.NeedsRawScans);
        }

        [Fact]
        public void Create_TwoIndependentTables_PlansBothAndSortsOrdinally()
        {
            var plan = TableReloadPlan.Create(["statics", "keywords"]);

            Assert.Equal(["keywords", "statics"], plan.Tables);
            Assert.Equal(["statics", "keywords"], plan.Requested);
        }

        [Fact]
        public void Create_DuplicateRequests_AreCollapsed()
        {
            var plan = TableReloadPlan.Create(["statics", "statics"]);

            Assert.Equal(["statics"], plan.Requested);
            Assert.Equal(["statics"], plan.Tables);
        }

        [Fact]
        public void Create_NoTables_ThrowsAndListsValidTables()
        {
            var error = Assert.Throws<TableSelectionException>(() => TableReloadPlan.Create([]));

            Assert.Contains("No tables given", error.Message);
            Assert.Contains("statics", error.Message);
        }

        [Fact]
        public void Create_UnknownTable_ThrowsNamingEveryUnknownNameAndNotTheValidOnes()
        {
            var error = Assert.Throws<TableSelectionException>(() => TableReloadPlan.Create(["statics", "nope", "alsonope"]));

            Assert.Contains("Unknown table(s): nope, alsonope.", error.Message);
        }

        [Fact]
        public void Create_TableNamesAreCaseSensitive() =>
            Assert.Throws<TableSelectionException>(() => TableReloadPlan.Create(["Statics"]));

        [Fact]
        public void Create_TableThatNoJobWrites_IsRejected() =>
            Assert.Throws<TableSelectionException>(() => TableReloadPlan.Create(["npc_inventory"]));

        [Fact]
        public void ValidTables_AreSortedAndDistinct()
        {
            var tables = TableReloadPlan.ValidTables;

            Assert.Equal(tables.Order(StringComparer.Ordinal), tables);
            Assert.Equal(tables.Count, tables.Distinct().Count());
        }

        [Fact]
        public void ValidTables_CoverTheTablesTheEngineReadsAtRuntime()
        {
            string[] engineTables =
            [
                "worldspaces", "cells", "placed_objects", "placed_teleports", "statics", "static_collections",
                "movable_statics", "activators", "containers", "doors", "flora", "furniture", "trees", "lights",
                "landscapes", "landscape_textures", "texture_sets", "lighting_templates", "movement_types",
                "climates", "weather", "image_spaces", "material_swap_entries", "base_material_swaps",
            ];

            Assert.All(engineTables, table => Assert.Contains(table, TableReloadPlan.ValidTables));
        }

        // A job's RecordTypes drive "DELETE FROM records WHERE record_type IN (...)" on reload: a type claimed by two
        // jobs would let one job's reload wipe the other's records rows.
        [Fact]
        public void AllJobs_RecordTypesAreClaimedByExactlyOneJob()
        {
            var types = ESMLoader.AllJobs.SelectMany(job => job.RecordTypes).ToList();

            Assert.Equal(types.Count, types.Distinct().Count());
        }

        [Fact]
        public void AllJobs_EachWritesAtLeastOneTable() =>
            Assert.All(ESMLoader.AllJobs, job => Assert.NotEmpty(job.Tables));

        // Only the jobs that fill link tables from records other jobs own (cells, locations, worldspaces, regions) own no
        // records rows: claiming those record types would let their reload delete the rows of CellsJob and the others.
        [Fact]
        public void AllJobs_OnlyTheMusicLinkJobsOwnNoRecordType()
        {
            var withoutRecords = ESMLoader.AllJobs.Where(job => job.RecordTypes.Length == 0).SelectMany(job => job.Tables).Order(StringComparer.Ordinal);

            Assert.Equal(["cell_places", "location_music", "region_areas", "region_music", "worldspace_music"], withoutRecords);
        }

        [Fact]
        public async Task AllJobs_EveryTableExistsInTheRealSchema()
        {
            using var db = await SchemaDatabase.CreateAsync();
            var existing = db.Query<string>("SELECT name FROM sqlite_master WHERE type = 'table'").ToHashSet();

            var missing = TableReloadPlan.ValidTables.Where(table => !existing.Contains(table)).ToList();

            Assert.Empty(missing);
        }
    }
}
