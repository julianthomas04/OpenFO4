using AssetConverter;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests
{
    // Field order and version gates of the BGSM-specific fields.
    public class BgsmTests
    {
        [Fact]
        public void Deserialize_Version2_ReadsTheLegacyTextureList()
        {
            var fields = new BgsmFields
            {
                Diffuse = "d.dds",
                Normal = "n.dds",
                SmoothSpec = "ss.dds",
                Greyscale = "g.dds",
                Envmap = "e.dds",
                Glow = "glow.dds",
                InnerLayer = "inner.dds",
                Wrinkles = "w.dds",
                Displacement = "disp.dds",
            };

            var bgsm = Parse(fields.Build(2));

            Assert.Equal("d.dds", bgsm.DiffuseTexture);
            Assert.Equal("n.dds", bgsm.NormalTexture);
            Assert.Equal("ss.dds", bgsm.SmoothSpecTexture);
            Assert.Equal("g.dds", bgsm.GreyscaleTexture);
            Assert.Equal("e.dds", bgsm.EnvmapTexture);
            Assert.Equal("glow.dds", bgsm.GlowTexture);
            Assert.Equal("inner.dds", bgsm.InnerLayerTexture);
            Assert.Equal("w.dds", bgsm.WrinklesTexture);
            Assert.Equal("disp.dds", bgsm.DisplacementTexture);
        }

        [Fact]
        public void Deserialize_Version3AndAbove_ReadsTheNewTextureList()
        {
            var fields = new BgsmFields
            {
                Diffuse = "d.dds",
                Normal = "n.dds",
                SmoothSpec = "ss.dds",
                Greyscale = "g.dds",
                Glow = "glow.dds",
                Wrinkles = "w.dds",
                Specular = "spec.dds",
                Lighting = "light.dds",
                Flow = "flow.dds",
            };

            var bgsm = Parse(fields.Build(8));

            Assert.Equal("glow.dds", bgsm.GlowTexture);
            Assert.Equal("w.dds", bgsm.WrinklesTexture);
            Assert.Equal("spec.dds", bgsm.SpecularTexture);
            Assert.Equal("light.dds", bgsm.LightingTexture);
            Assert.Equal("flow.dds", bgsm.FlowTexture);
            Assert.Equal("", bgsm.EnvmapTexture);
            Assert.Equal("", bgsm.DistanceFieldAlphaTexture);
        }

        [Fact]
        public void Deserialize_Version17_AddsTheDistanceFieldTexture()
        {
            var bgsm = Parse(new BgsmFields { Flow = "flow.dds", DistanceField = "df.dds", GrayscaleToPaletteScale = 0.5f }.Build(17));

            Assert.Equal("flow.dds", bgsm.FlowTexture);
            Assert.Equal("df.dds", bgsm.DistanceFieldAlphaTexture);
            Assert.Equal(0.5f, bgsm.GrayscaleToPaletteScale);
        }

        [Fact]
        public void Deserialize_BeforeVersionEight_ReadsRimLightingBlock()
        {
            var bgsm = Parse(new BgsmFields { RimLighting = true, RimPower = 3f, BacklightPower = 4f, SubsurfaceLighting = true, SubsurfaceRolloff = 0.7f, BackLighting = true }.Build(7));

            Assert.True(bgsm.RimLighting);
            Assert.Equal(3f, bgsm.RimPower);
            Assert.Equal(4f, bgsm.BackLightPower);
            Assert.True(bgsm.SubsurfaceLighting);
            Assert.Equal(0.7f, bgsm.SubsurfaceLightingRolloff);
            Assert.True(bgsm.BackLighting);
            Assert.False(bgsm.Translucency);
        }

        [Fact]
        public void Deserialize_FromVersionEight_ReadsTranslucencyBlockInsteadOfRimLighting()
        {
            var bgsm = Parse(new BgsmFields
            {
                Translucency = true,
                TranslucencyThick = true,
                TranslucencyMixed = true,
                TranslucencyColor = (1f, 0f, 0f),
                TranslucencyTransmissiveScale = 1.25f,
                TranslucencyTurbulence = 0.5f,
                ReceiveShadows = true,
            }.Build(8));

            Assert.True(bgsm.Translucency);
            Assert.True(bgsm.TranslucencyThickObject);
            Assert.True(bgsm.TranslucencyMixAlbedoWithSubsurfaceColor);
            Assert.Equal(0xFF0000u, bgsm.TranslucencySubsurfaceColor);
            Assert.Equal(1.25f, bgsm.TranslucencyTransmissiveScale);
            Assert.Equal(0.5f, bgsm.TranslucencyTurbulence);
            Assert.False(bgsm.RimLighting);
            Assert.True(bgsm.ReceiveShadows);
        }

        [Fact]
        public void Deserialize_SpecularBlock_IsReadWithColourPackedAsRgb()
        {
            var bgsm = Parse(new BgsmFields
            {
                SpecularEnabled = true,
                SpecularColor = (1f, 0f, 0f),
                SpecularMult = 2.5f,
                Smoothness = 0.25f,
                FresnelPower = 3f,
            }.Build(2));

            Assert.True(bgsm.SpecularEnabled);
            Assert.Equal(0xFF0000u, bgsm.SpecularColor);
            Assert.Equal(2.5f, bgsm.SpecularMult);
            Assert.Equal(0.25f, bgsm.Smoothness);
            Assert.Equal(3f, bgsm.FresnelPower);
        }

        [Fact]
        public void Deserialize_WetnessEnvMapScale_IsOnlyPresentBeforeVersionTen()
        {
            var before = Parse(new BgsmFields { WetEnvMapScale = 0.9f, WetFresnelPower = 0.8f, WetMetalness = 0.7f }.Build(9));
            var after = Parse(new BgsmFields { WetEnvMapScale = 0.9f, WetFresnelPower = 0.8f, WetMetalness = 0.7f }.Build(10));

            Assert.Equal(0.9f, before.WetnessControlEnvMapScale);
            Assert.Equal(0.8f, before.WetnessControlFresnelPower);
            Assert.Equal(0.7f, before.WetnessControlMetalness);
            Assert.Equal(-1f, after.WetnessControlEnvMapScale);
            Assert.Equal(0.8f, after.WetnessControlFresnelPower);
            Assert.Equal(0.7f, after.WetnessControlMetalness);
        }

        [Fact]
        public void Deserialize_PbrAndPorosity_AreVersionGated()
        {
            var v2 = Parse(new BgsmFields { Pbr = true, CustomPorosity = true, Porosity = 0.4f, RootMaterial = "root.bgsm" }.Build(2));
            var v3 = Parse(new BgsmFields { Pbr = true, CustomPorosity = true, Porosity = 0.4f, RootMaterial = "root.bgsm" }.Build(3));
            var v9 = Parse(new BgsmFields { Pbr = true, CustomPorosity = true, Porosity = 0.4f, RootMaterial = "root.bgsm" }.Build(9));

            Assert.False(v2.PBR);
            Assert.True(v3.PBR);
            Assert.False(v3.CustomPorosity);
            Assert.True(v9.CustomPorosity);
            Assert.Equal(0.4f, v9.PorosityValue);
            Assert.Equal("root.bgsm", v2.RootMaterialPath);
            Assert.Equal("root.bgsm", v3.RootMaterialPath);
            Assert.Equal("root.bgsm", v9.RootMaterialPath);
        }

        [Fact]
        public void Deserialize_EmittanceColour_IsOnlyStoredWhenEmitIsEnabled()
        {
            var enabled = Parse(new BgsmFields { EmitEnabled = true, EmitColor = (0f, 1f, 0f), EmitMult = 3f }.Build(2));
            var disabled = Parse(new BgsmFields { EmitEnabled = false, EmitMult = 3f }.Build(2));

            Assert.Equal(0x00FF00u, enabled.EmittanceColor);
            Assert.Equal(3f, enabled.EmittanceMult);
            Assert.Equal(0xFFFFFFFFu, disabled.EmittanceColor);
            Assert.Equal(3f, disabled.EmittanceMult);
        }

        [Fact]
        public void Deserialize_EmissiveVersionGates_LumEmittanceFromTwelveAdaptiveFromThirteen()
        {
            var fields = new BgsmFields { LumEmittance = 7f, UseAdaptiveEmissive = true, AdaptiveOffset = 1f, AdaptiveMin = 2f, AdaptiveMax = 3f, Glowmap = true };

            var v11 = Parse(fields.Build(11));
            var v12 = Parse(fields.Build(12));
            var v13 = Parse(fields.Build(13));

            Assert.Equal(0f, v11.LumEmittance);
            Assert.Equal(7f, v12.LumEmittance);
            Assert.False(v12.UseAdaptativeEmissive);
            Assert.True(v13.UseAdaptativeEmissive);
            Assert.Equal(1f, v13.AdaptativeEmissive_ExposureOffset);
            Assert.Equal(2f, v13.AdaptativeEmissive_FinalExposureMin);
            Assert.Equal(3f, v13.AdaptativeEmissive_FinalExposureMax);
            Assert.True(v13.Glowmap);
        }

        [Fact]
        public void Deserialize_EnvironmentWindowAndEyeFlags_AreOnlyPresentBeforeVersionSeven()
        {
            var fields = new BgsmFields { EnvWindow = true, EnvEye = true, Hair = true };

            var v6 = Parse(fields.Build(6));
            var v7 = Parse(fields.Build(7));

            Assert.True(v6.EnvironmentMappingWindow);
            Assert.True(v6.EnvironmentMappingEye);
            Assert.True(v6.Hair);
            Assert.False(v7.EnvironmentMappingWindow);
            Assert.True(v7.Hair);
        }

        [Fact]
        public void Deserialize_DisplacementBlock_IsOnlyPresentBeforeVersionThree()
        {
            var fields = new BgsmFields { Tessellate = true, DisplacementBias = 0.1f, DisplacementScale = 0.2f, TessPnScale = 0.3f, TessBaseFactor = 0.4f, TessFadeDistance = 0.5f, GrayscaleToPaletteScale = 0.6f };

            var v2 = Parse(fields.Build(2));
            var v3 = Parse(fields.Build(3));

            Assert.Equal(0.1f, v2.DisplacementTextureBias);
            Assert.Equal(0.5f, v2.TessellationFadeDistance);
            Assert.Equal(0.6f, v2.GrayscaleToPaletteScale);
            Assert.Equal(-0.5f, v3.DisplacementTextureBias);
            Assert.Equal(0.6f, v3.GrayscaleToPaletteScale);
        }

        [Fact]
        public void Deserialize_VersionZero_HasNoSkewSpecularAlphaFlag()
        {
            var v0 = Parse(new BgsmFields { SkewSpecularAlpha = true }.Build(0));
            var v1 = Parse(new BgsmFields { SkewSpecularAlpha = true }.Build(1));

            Assert.False(v0.SkewSpecularAlpha);
            Assert.True(v1.SkewSpecularAlpha);
        }

        [Fact]
        public void Deserialize_TerrainBlock_ReadsThreeFloatsAndVersionThreeHasAnExtraInt()
        {
            var fields = new BgsmFields { Terrain = true, TerrainUnknown = 99u, TerrainThresholdFalloff = 1f, TerrainTilingDistance = 2f, TerrainRotationAngle = 3f };

            var v3 = Parse(fields.Build(3));
            var v8 = Parse(fields.Build(8));

            Assert.Equal(99u, v3.UnkInt1);
            Assert.Equal(1f, v3.TerrainThresholdFalloff);
            Assert.Equal(3f, v3.TerrainRotationAngle);
            Assert.Equal(0u, v8.UnkInt1);
            Assert.Equal(1f, v8.TerrainThresholdFalloff);
            Assert.Equal(2f, v8.TerrainTilingDistance);
            Assert.Equal(3f, v8.TerrainRotationAngle);
        }

        [Fact]
        public void Deserialize_TerrainFalse_LeavesTerrainFloatsAtZeroAndConsumesNothingMore()
        {
            byte[] bytes = new BgsmFields { Terrain = false }.Build(8);

            var bgsm = Parse(bytes);

            Assert.False(bgsm.Terrain);
            Assert.Equal(0f, bgsm.TerrainTilingDistance);
        }

        [Theory]
        [InlineData(2u)]
        [InlineData(8u)]
        [InlineData(12u)]
        [InlineData(13u)]
        [InlineData(17u)]
        public void Serialize_ParsedFile_ReproducesTheOriginalBytesAtEveryVersion(uint version)
        {
            byte[] original = new BgsmFields
            {
                Diffuse = "d.dds",
                Normal = "n.dds",
                SmoothSpec = "ss.dds",
                Greyscale = "g.dds",
                Glow = "glow.dds",
                SpecularEnabled = true,
                SpecularMult = 2f,
                EmitEnabled = true,
                EmitMult = 2f,
                Glowmap = true,
                Terrain = true,
                TerrainTilingDistance = 4f,
                GrayscaleToPaletteScale = 0.25f,
                HairTint = (128f / 255f, 128f / 255f, 128f / 255f),
            }.Build(version);

            var bgsm = Parse(original);
            using var output = new MemoryStream();
            using (var writer = new BinaryWriter(output, System.Text.Encoding.UTF8, leaveOpen: true))
                bgsm.Serialize(writer);

            Assert.Equal(original, output.ToArray());
        }

        // An authoring tool stores a byte channel k as the float k/255. Reading must give the same byte back.
        [Theory]
        [InlineData(0)]
        [InlineData(1)]
        [InlineData(127)]
        [InlineData(128)]
        [InlineData(254)]
        [InlineData(255)]
        public void Deserialize_ColourChannelStoredAsByteOver255_ReadsBackTheSameByte(int k)
        {
            Assert.Equal((uint)(k * 0x010101), ReadSpecularColor(k / 255f));
        }

        private static uint ReadSpecularColor(float channel) =>
            Parse(new BgsmFields { SpecularColor = (channel, channel, channel) }.Build(2)).SpecularColor;

        private static BGSM Parse(byte[] bytes)
        {
            using var reader = new BinaryReader(new MemoryStream(bytes));
            var bgsm = new BGSM();
            bgsm.Deserialize(reader);
            return bgsm;
        }
    }
}
