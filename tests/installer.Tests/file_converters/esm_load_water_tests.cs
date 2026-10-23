using System.Reflection;
using System.Text.Json;
using AssetConverter;
using Mutagen.Bethesda.Fallout4;
using Xunit;

namespace InstallerTests
{
    // Water extraction contract. WATR rows keep every Mutagen field, cells and
    // worldspaces keep their water links and heights, and the base object of a placed water mesh keeps its WATR link.
    [Collection(FixtureEsm.Name)]
    public class EsmLoadWaterTests(LoadedFixtureDatabase db)
    {
        sealed class WaterRow
        {
            public string? Name { get; init; }
            public string? Material { get; init; }
            public string? Data { get; init; }
        }

        sealed class CellWaterRow
        {
            public long HasWater { get; init; }
            public long NoLodWater { get; init; }
            public double? WaterHeight { get; init; }
            public string? WaterType { get; init; }
            public string? WaterEnvironmentMap { get; init; }
            public string? WaterCurrents { get; init; }
        }

        sealed class WorldspaceWaterRow
        {
            public string? WaterType { get; init; }
            public string? LodWaterType { get; init; }
            public double? LodWaterHeight { get; init; }
            public double? DefaultLandHeight { get; init; }
            public double? DefaultWaterHeight { get; init; }
            public long? ParentFlags { get; init; }
            public string? WaterEnvironmentMap { get; init; }
        }

        JsonElement WaterData(string editorId)
        {
            var row = db.QuerySingle<WaterRow>("SELECT * FROM waters WHERE form_key = @key", new { key = db.KeyOf(editorId) });
            return JsonDocument.Parse(row.Data!).RootElement;
        }

        CellWaterRow CellWater(string editorId) =>
            db.QuerySingle<CellWaterRow>("SELECT * FROM cells WHERE form_key = @key", new { key = db.KeyOf(editorId) });

        [Fact]
        public void WaterData_HasEveryPropertyMutagenDeclaresOnTheWaterRecord()
        {
            var data = WaterData("FixtureWater");

            var declared = typeof(IWaterGetter).GetProperties(BindingFlags.Public | BindingFlags.Instance | BindingFlags.DeclaredOnly)
                .Select(p => p.Name)
                .Except(["BinaryWriteTranslator", "StaticRegistration"]);
            var stored = data.EnumerateObject().Select(p => p.Name).ToHashSet();

            Assert.Equal(Array.Empty<string>(), declared.Where(name => !stored.Contains(name)));
        }

        [Fact]
        public void WaterData_ScalarFields_KeepRawValues()
        {
            var data = WaterData("FixtureWater");

            Assert.Equal(WaterFixture.Opacity, data.GetProperty("Opacity").GetInt32());
            Assert.Equal(1.5f, data.GetProperty("FogDepthAmount").GetSingle());
            Assert.Equal(0.21f, data.GetProperty("PhysicalDisplacementSimulatorStartingSize").GetSingle());
            Assert.True(data.GetProperty("ScreenSpaceReflections").GetBoolean());
        }

        [Fact]
        public void WaterData_Colors_AreRgbaByteArrays()
        {
            var data = WaterData("FixtureWater");

            Assert.Equal([10, 20, 30, 40], Ints(data.GetProperty("FogShallowColor")));
            Assert.Equal([12, 22, 32, 42], Ints(data.GetProperty("FogUnderwaterColor")));
            Assert.Equal([13, 23, 33, 43], Ints(data.GetProperty("PhysicalReflectionColor")));
            Assert.Equal([15, 25, 35, 45], Ints(data.GetProperty("SiltDarkColor")));
        }

        [Fact]
        public void WaterData_FlagsAndEnums_AreNameArrays()
        {
            var data = WaterData("FixtureWater");

            Assert.Equal(["Dangerous", "DirectionalSound"], Strings(data.GetProperty("Flags")));
            Assert.Equal(JsonValueKind.Array, data.GetProperty("DNAMDataTypeState").ValueKind);
        }

        [Fact]
        public void WaterData_FormLinks_AreFormKeyStrings()
        {
            var data = WaterData("FixtureWater");

            Assert.Equal(WaterFixture.SoundKey.ToString(), data.GetProperty("OpenSound").GetString());
            Assert.Equal("0ABCE2:Fallout4.esm", data.GetProperty("Material").GetString());
            Assert.Equal("0ABCE1:Fallout4.esm", data.GetProperty("ImageSpace").GetString());
        }

        [Fact]
        public void WaterData_NoiseLayers_AreNestedObjectsWithTexturePaths()
        {
            var data = WaterData("FixtureWater");

            var layer = data.GetProperty("NoiseLayerOne");
            Assert.Equal(WaterFixture.NoiseOneWindDirection, layer.GetProperty("WindDirection").GetSingle());
            Assert.Equal(WaterFixture.NoiseOneTexture, layer.GetProperty("Texture").GetString());
            Assert.Equal(@"data\Textures\Water\Three.dds", data.GetProperty("NoiseLayerThree").GetProperty("Texture").GetString());
        }

        [Fact]
        public void WaterData_Velocities_AreXyzArrays()
        {
            var data = WaterData("FixtureWater");

            Assert.Equal([0.5f, -0.25f, 0f], Floats(data.GetProperty("LinearVelocity")));
            Assert.Equal([0.1f, 0.2f, 0.3f], Floats(data.GetProperty("AngularVelocity")));
        }

        [Fact]
        public void WaterData_RawBlobs_AreKeptAsByteArraysWithoutDerivedKeys()
        {
            var data = WaterData("FixtureWater");

            Assert.Equal([1, 2], Ints(data.GetProperty("DATA")));
            Assert.Equal(WaterFixture.GnamBytes.Select(b => (int)b), Ints(data.GetProperty("GNAM")));
            Assert.False(data.TryGetProperty("RelatedWaters", out _));
        }

        [Fact]
        public void Water_MaterialLink_IsAlsoStoredInTheMaterialColumn()
        {
            var row = db.QuerySingle<WaterRow>("SELECT * FROM waters WHERE form_key = @key", new { key = db.KeyOf("FixtureWater") });

            Assert.Equal("0ABCE2:Fallout4.esm", row.Material);
        }

        [Fact]
        public void Cell_ExteriorWater_KeepsHeightTypeCubemapAndCurrents()
        {
            var cell = CellWater("FixtureExterior");

            Assert.Equal(1, cell.HasWater);
            Assert.Equal(1, cell.NoLodWater);
            Assert.Equal(128.5, cell.WaterHeight);
            Assert.Equal(db.KeyOf("FixtureWater"), cell.WaterType);
            Assert.Equal(@"data\Textures\Shared\Cubemaps\cell.dds", cell.WaterEnvironmentMap);
            Assert.Equal(LoadedFixtureDatabase.CellCurrents, Entries(cell.WaterCurrents!));
        }

        [Fact]
        public void Cell_WithoutCurrentBlock_HasNullCurrents()
        {
            Assert.Null(CellWater("FixtureInterior").WaterCurrents);
        }

        [Fact]
        public void Cell_NoWaterSentinel_IsKeptRaw()
        {
            var cell = CellWater("FixtureInterior");

            Assert.Equal(0, cell.HasWater);
            Assert.Equal((double)LoadedFixtureDatabase.NoWaterSentinel, cell.WaterHeight);
            Assert.Null(cell.WaterType);
        }

        [Fact]
        public void Cell_WaterHeightOfZero_IsDistinctFromAbsent()
        {
            Assert.Equal(0.0, CellWater("FixtureWorldTopCell").WaterHeight);
        }

        [Fact]
        public void Worldspace_WaterFields_AreStored()
        {
            var world = db.QuerySingle<WorldspaceWaterRow>("SELECT * FROM worldspaces WHERE form_key = @key", new { key = db.KeyOf("FixtureWorld") });

            Assert.Equal(db.KeyOf("FixtureWater"), world.WaterType);
            Assert.Equal(db.KeyOf("FixtureWater"), world.LodWaterType);
            Assert.Equal(450.0, world.LodWaterHeight);
            Assert.Equal(-2048.0, world.DefaultLandHeight);
            Assert.Equal(-10000.0, world.DefaultWaterHeight);
            Assert.Equal(@"data\Textures\Shared\Cubemaps\fixture.dds", world.WaterEnvironmentMap);
        }

        [Fact]
        public void Activator_WaterType_LinksAPlacedWaterMeshToItsWatr()
        {
            var linked = db.QuerySingle<string?>("SELECT water_type FROM activators WHERE form_key = @key", new { key = db.KeyOf("WaterActivator") });
            var dry = db.QuerySingle<string?>("SELECT water_type FROM activators WHERE form_key = @key", new { key = db.KeyOf("DryActivator") });

            Assert.Equal(db.KeyOf("FixtureWater"), linked);
            Assert.Null(dry);
        }

        [Fact]
        public void PlacedWater_Currents_AreStoredAsJsonAndAbsentOnOtherRefs()
        {
            var water = db.QuerySingle<string?>("SELECT water_currents FROM placed_objects WHERE form_key = @key", new { key = db.KeyOf("WaterRef") });
            var plain = db.QuerySingle<string?>("SELECT water_currents FROM placed_objects WHERE form_key = @key", new { key = db.KeyOf("PlainRef") });

            Assert.Equal(LoadedFixtureDatabase.PlacedCurrents, Entries(water!));
            Assert.Null(plain);
        }

        [Fact]
        public void Worldspace_ParentFlags_KeepTheRawPnamValue()
        {
            var world = db.QuerySingle<WorldspaceWaterRow>("SELECT * FROM worldspaces WHERE form_key = @key", new { key = db.KeyOf("FixtureWorld") });

            Assert.Equal(LoadedFixtureDatabase.ParentFlagsRaw, world.ParentFlags);
        }

        static float[][] Entries(string json) =>
            JsonDocument.Parse(json).RootElement.EnumerateArray().Select(entry => Floats(entry).ToArray()).ToArray();

        static IEnumerable<int> Ints(JsonElement array) => array.EnumerateArray().Select(e => e.GetInt32());

        static IEnumerable<float> Floats(JsonElement array) => array.EnumerateArray().Select(e => e.GetSingle());

        static IEnumerable<string?> Strings(JsonElement array) => array.EnumerateArray().Select(e => e.GetString());
    }
}
