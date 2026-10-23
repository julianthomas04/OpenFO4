using AssetConverter;
using Xunit;

namespace InstallerTests
{
    // MATT extraction and the shape-to-MATT join key.
    public class BgsMaterialHashTests
    {
        // Vectors taken from Fallout 4: the low dword of the shape user data in vanilla NIFs and the MNAM of the
        // MATT record the game resolves it to (Baseball.nif, Abraxo.nif, a PreCombined static compound).
        [Theory]
        [InlineData("MaterialBaseball", 0x7A565E6Cu)]
        [InlineData("Cloth", 0xE4D39CA3u)]
        [InlineData("Wood", 0x1DD9C611u)]
        [InlineData("MaterialPlasticBin", 0x04280084u)]
        public void Compute_MatchesTheHashVanillaShapesCarry(string materialName, uint expected) =>
            Assert.Equal(expected, BgsMaterialHash.Compute(materialName));

        [Fact]
        public void Compute_IsCaseInsensitive() =>
            Assert.Equal(BgsMaterialHash.Compute("MaterialBaseball"), BgsMaterialHash.Compute("MATERIALBASEBALL"));

        [Fact]
        public void Compute_OfEmptyName_IsZero() =>
            Assert.Equal(0u, BgsMaterialHash.Compute(""));
    }

    [Collection(FixtureEsm.Name)]
    public class EsmLoadMaterialTypeTests(LoadedFixtureDatabase db)
    {
        sealed class MaterialTypeRow
        {
            public string? EditorId { get; init; }
            public string? MaterialName { get; init; }
            public long? MaterialHash { get; init; }
            public double Buoyancy { get; init; }
            public long Flags { get; init; }
        }

        MaterialTypeRow Material(string editorId) =>
            db.QuerySingle<MaterialTypeRow>("SELECT * FROM material_types WHERE form_key = @key", new { key = db.KeyOf(editorId) });

        [Fact]
        public void MaterialType_StoresNameHashBuoyancyAndFlags()
        {
            var row = Material("StoneMaterial");

            Assert.Equal("Stone", row.MaterialName);
            Assert.Equal(0xDF02F237L, row.MaterialHash);
            Assert.Equal(-1.0, row.Buoyancy);
            Assert.Equal(2, row.Flags);
        }

        [Fact]
        public void MaterialType_WithoutName_HasNoHash()
        {
            var row = Material("UnnamedMaterial");

            Assert.Null(row.MaterialName);
            Assert.Null(row.MaterialHash);
        }
    }
}
