using System.Text.Json;
using Xunit;

namespace InstallerTests
{
    // Base-record tables the engine joins placed refs against: statics, lights, doors, weather, worldspace climate
    // and the material swap stub (docs/installer/database.md).
    [Collection(FixtureEsm.Name)]
    public class EsmLoadBaseRecordTests(LoadedFixtureDatabase db)
    {
        sealed class StaticRow
        {
            public string? EditorId { get; init; }
            public string? ModelPath { get; init; }
            public long HasDistantLod { get; init; }
            public string? Bounds { get; init; }
            public string? AlternateTextures { get; init; }
            public long IsMarker { get; init; }
            public string? LodModelPath { get; init; }
        }

        StaticRow Static(string editorId) =>
            db.QuerySingle<StaticRow>("SELECT * FROM statics WHERE form_key = @key", new { key = db.KeyOf(editorId) });

        [Fact]
        public void Static_StoresEditorIdAndAuthoredModelPath()
        {
            var row = Static("WallStatic");

            Assert.Equal("WallStatic", row.EditorId);
            Assert.Equal(LoadedFixtureDatabase.ModelOfWall, row.ModelPath);
        }

        [Fact]
        public void Static_ObjectBounds_AreMinMaxJson()
        {
            using var bounds = JsonDocument.Parse(Static("WallStatic").Bounds!);
            var min = bounds.RootElement.GetProperty("Min");
            var max = bounds.RootElement.GetProperty("Max");

            Assert.Equal((-10, -20, -30), (min.GetProperty("X").GetInt32(), min.GetProperty("Y").GetInt32(), min.GetProperty("Z").GetInt32()));
            Assert.Equal((10, 20, 30), (max.GetProperty("X").GetInt32(), max.GetProperty("Y").GetInt32(), max.GetProperty("Z").GetInt32()));
        }

        [Fact]
        public void Static_WithMarkerRecordFlag_IsMarkedMarker()
        {
            Assert.Equal(1, Static("MarkerStatic").IsMarker);
            Assert.Equal(0, Static("WallStatic").IsMarker);
        }

        [Fact]
        public void Static_WithoutDistantLodData_HasNoDistantLodAndNoLodModel()
        {
            var row = Static("WallStatic");

            Assert.Equal(0, row.HasDistantLod);
            Assert.Null(row.LodModelPath);
            Assert.Null(row.AlternateTextures);
        }

        [Fact]
        public void Door_StoresModelPath()
        {
            var model = db.QuerySingle<string>("SELECT model_path FROM doors WHERE form_key = @key", new { key = db.KeyOf("DoorBase") });

            Assert.Equal(@"Doors\Door01.nif", model);
        }

        [Fact]
        public void Door_ObjectBounds_AreMinMaxJson()
        {
            var json = db.QuerySingle<string>("SELECT bounds FROM doors WHERE form_key = @key", new { key = db.KeyOf("DoorBase") });

            Assert.Equal("""{"Min":{"X":-40,"Y":-6,"Z":0},"Max":{"X":40,"Y":6,"Z":200}}""", json);
        }

        [Fact]
        public void Door_TriggerBounds_AreNullUntilTheMarkerStepFillsThem()
        {
            var json = db.QuerySingle<string?>("SELECT trigger_bounds FROM doors WHERE form_key = @key", new { key = db.KeyOf("DoorBase") });

            Assert.Null(json);
        }

        [Fact]
        public void CollisionLayerIndex_IsIndexedForLookupByLayerNumber()
        {
            var indexed = db.QuerySingle<long>("SELECT COUNT(*) FROM pragma_index_info('idx_collision_layers_index') WHERE name = 'layer_index'");

            Assert.Equal(1, indexed);
        }

        [Fact]
        public void Static_ObjectBounds_KeepTheExactJsonFormat()
        {
            Assert.Equal("""{"Min":{"X":-10,"Y":-20,"Z":-30},"Max":{"X":10,"Y":20,"Z":30}}""", Static("WallStatic").Bounds);
        }

        sealed class CollisionLayerRow
        {
            public string? EditorId { get; init; }
            public long? LayerIndex { get; init; }
            public long? Flags { get; init; }
            public string? CollidesWith { get; init; }
        }

        CollisionLayerRow CollisionLayer(string editorId) =>
            db.QuerySingle<CollisionLayerRow>("SELECT * FROM collision_layers WHERE form_key = @key", new { key = db.KeyOf(editorId) });

        [Fact]
        public void CollisionLayer_StoresIndexAndRawFlags()
        {
            var row = CollisionLayer("LayerStatic");

            Assert.Equal("LayerStatic", row.EditorId);
            Assert.Equal(1, row.LayerIndex);
            Assert.Equal((long)(uint)Mutagen.Bethesda.Fallout4.CollisionLayer.Flag.TriggerVolume, row.Flags);
        }

        [Fact]
        public void CollisionLayer_CollidesWith_IsJsonArrayOfFormKeysInAuthoredOrder()
        {
            var collidesWith = JsonSerializer.Deserialize<string[]>(CollisionLayer("LayerCharacter").CollidesWith!);

            Assert.NotNull(collidesWith);
            Assert.Equal([db.KeyOf("LayerStatic"), db.KeyOf("LayerCharacter")], collidesWith);
        }

        [Fact]
        public void CollisionLayer_WithoutCollidesWithList_StoresNull()
        {
            Assert.Null(CollisionLayer("LayerIsolated").CollidesWith);
        }

        sealed class LightRow
        {
            public double Radius { get; init; }
            public long Color { get; init; }
            public double FadeValue { get; init; }
            public double FalloffExponent { get; init; }
            public double Fov { get; init; }
            public long IsMarker { get; init; }
        }

        [Fact]
        public void Light_StoresRadiusFadeAndPackedRgbColour()
        {
            var row = db.QuerySingle<LightRow>("SELECT * FROM lights WHERE form_key = @key", new { key = db.KeyOf("LampLight") });

            Assert.Equal(512.0, row.Radius);
            Assert.Equal(0xFF8040, row.Color);
            Assert.Equal(1.5, row.FadeValue);
            Assert.Equal(2.0, row.FalloffExponent);
            Assert.Equal(90.0, row.Fov);
            Assert.Equal(0, row.IsMarker);
        }

        sealed class WeatherRow
        {
            public string? Colors { get; init; }
            public string? Fog { get; init; }
            public string? ImageSpaces { get; init; }
        }

        WeatherRow Weather() =>
            db.QuerySingle<WeatherRow>("SELECT * FROM weather WHERE form_key = @key", new { key = db.KeyOf("FixtureWeather") });

        [Fact]
        public void Weather_Colors_AreGroupByTimeOfDayRgbObjects()
        {
            using var colors = JsonDocument.Parse(Weather().Colors!);
            var skyUpper = colors.RootElement.GetProperty("SkyUpper");
            var day = skyUpper.GetProperty("Day");
            var night = skyUpper.GetProperty("Night");

            Assert.Equal((10, 20, 30), (day.GetProperty("R").GetInt32(), day.GetProperty("G").GetInt32(), day.GetProperty("B").GetInt32()));
            Assert.Equal((1, 2, 3), (night.GetProperty("R").GetInt32(), night.GetProperty("G").GetInt32(), night.GetProperty("B").GetInt32()));
        }

        [Fact]
        public void Weather_Colors_CoverAllNineteenGroupsAndEightTimesOfDay()
        {
            using var colors = JsonDocument.Parse(Weather().Colors!);

            Assert.Equal(19, colors.RootElement.EnumerateObject().Count());
            Assert.All(colors.RootElement.EnumerateObject(), group => Assert.Equal(8, group.Value.EnumerateObject().Count()));
        }

        [Fact]
        public void Weather_Fog_SplitsDayAndNightDistances()
        {
            using var fog = JsonDocument.Parse(Weather().Fog!);

            Assert.Equal(100.0, fog.RootElement.GetProperty("Day").GetProperty("Near").GetDouble());
            Assert.Equal(9000.0, fog.RootElement.GetProperty("Night").GetProperty("Far").GetDouble());
        }

        [Fact]
        public void Weather_UnsetImageSpaceLinks_AreJsonNullPerTimeOfDay()
        {
            using var imageSpaces = JsonDocument.Parse(Weather().ImageSpaces!);

            Assert.Equal(8, imageSpaces.RootElement.EnumerateObject().Count());
            Assert.All(imageSpaces.RootElement.EnumerateObject(), time => Assert.Equal(JsonValueKind.Null, time.Value.ValueKind));
        }

        [Fact]
        public void Worldspace_ClimateLink_IsStoredForWeatherSelection()
        {
            var climate = db.QuerySingle<string>("SELECT climate FROM worldspaces WHERE form_key = @key", new { key = db.KeyOf("FixtureWorld") });

            Assert.Equal(db.KeyOf("FixtureClimate"), climate);
        }

        [Fact]
        public void MaterialSwap_WithoutMaterialsRoot_IsStoredAsStubWithoutEntries()
        {
            var editorId = db.QuerySingle<string>("SELECT editor_id FROM material_swaps WHERE form_key = @key", new { key = db.KeyOf("RedSwap") });
            var entries = db.QuerySingle<long>("SELECT COUNT(*) FROM material_swap_entries");

            Assert.Equal("RedSwap", editorId);
            Assert.Equal(0, entries);
        }

        [Fact]
        public void RecordTypes_AreRegisteredWithTheirFourLetterSignature()
        {
            var types = db.Query<(string EditorId, string Type)>(
                "SELECT editor_id, record_type FROM records WHERE editor_id IN ('WallStatic','DoorBase','LampLight','FixtureWorld','FixtureWeather','RedSwap','GrassTexture','FixtureClimate') ORDER BY editor_id");

            Assert.Equal(
                [("DoorBase", "DOOR"), ("FixtureClimate", "CLMT"), ("FixtureWeather", "WTHR"), ("FixtureWorld", "WRLD"),
                 ("GrassTexture", "LTEX"), ("LampLight", "LIGH"), ("RedSwap", "MSWP"), ("WallStatic", "STAT")],
                types);
        }

        [Fact]
        public void RecordsTable_SourcePluginIsTheFileName()
        {
            var plugin = db.QuerySingle<string>("SELECT source_plugin FROM records WHERE form_key = @key", new { key = db.KeyOf("WallStatic") });

            Assert.Equal(TinyPlugin.PluginFileName, plugin);
        }

        [Fact]
        public void FormKeys_UseUppercaseHexIdColonPluginFormat()
        {
            Assert.Matches("^[0-9A-F]{6}:Fixture\\.esm$", db.KeyOf("WallStatic"));
        }
    }
}
