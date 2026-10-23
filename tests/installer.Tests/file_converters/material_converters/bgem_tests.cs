using AssetConverter;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests
{
    // Field order and version gates of the BGEM-specific fields.
    public class BgemTests
    {
        [Fact]
        public void Deserialize_Version10_ReadsOnlyTheFiveBaseTextures()
        {
            var fields = new BgemFields { BaseTexture = "base.dds", Greyscale = "g.dds", Envmap = "e.dds", Normal = "n.dds", EnvmapMask = "m.dds" };

            var bgem = Parse(fields.Build(10));

            Assert.Equal("base.dds", bgem.BaseTexture);
            Assert.Equal("g.dds", bgem.GrayscaleTexture);
            Assert.Equal("e.dds", bgem.EnvmapTexture);
            Assert.Equal("n.dds", bgem.NormalTexture);
            Assert.Equal("m.dds", bgem.EnvmapMaskTexture);
            Assert.Equal("", bgem.SpecularTexture);
        }

        [Fact]
        public void Deserialize_FromVersionEleven_ReadsSpecularLightingGlowTexturesAndEmittanceColour()
        {
            var fields = new BgemFields { Specular = "s.dds", Lighting = "l.dds", Glow = "glow.dds", EmitColor = (0f, 0f, 1f) };

            var bgem = Parse(fields.Build(11));

            Assert.Equal("s.dds", bgem.SpecularTexture);
            Assert.Equal("l.dds", bgem.LightingTexture);
            Assert.Equal("glow.dds", bgem.GlowTexture);
            Assert.Equal(0x0000FFu, bgem.EmittanceColor);
        }

        [Fact]
        public void Deserialize_EffectBlock_IsReadInFileOrder()
        {
            var fields = new BgemFields
            {
                Blood = true,
                EffectLighting = false,
                Falloff = true,
                FalloffColor = false,
                GrayscaleToPaletteAlpha = true,
                Soft = true,
                BaseColor = (1f, 0f, 0f),
                BaseColorScale = 2f,
                FalloffStartAngle = 0.1f,
                FalloffStopAngle = 0.2f,
                FalloffStartOpacity = 0.3f,
                FalloffStopOpacity = 0.4f,
                LightingInfluence = 0.5f,
                EnvmapMinLod = 7,
                SoftDepth = 12f,
            };

            var bgem = Parse(fields.Build(10));

            Assert.True(bgem.BloodEnabled);
            Assert.False(bgem.EffectLightingEnabled);
            Assert.True(bgem.FalloffEnabled);
            Assert.False(bgem.FalloffColorEnabled);
            Assert.True(bgem.GrayscaleToPaletteAlpha);
            Assert.True(bgem.SoftEnabled);
            Assert.Equal(0xFF0000u, bgem.BaseColor);
            Assert.Equal(2f, bgem.BaseColorScale);
            Assert.Equal(0.1f, bgem.FalloffStartAngle);
            Assert.Equal(0.2f, bgem.FalloffStopAngle);
            Assert.Equal(0.3f, bgem.FalloffStartOpacity);
            Assert.Equal(0.4f, bgem.FalloffStopOpacity);
            Assert.Equal(0.5f, bgem.LightingInfluence);
            Assert.Equal(7, bgem.EnvmapMinLOD);
            Assert.Equal(12f, bgem.SoftDepth);
        }

        [Fact]
        public void Deserialize_EnvironmentMappingFields_AreReadFromBgemBodyFromVersionTen()
        {
            var fields = new BgemFields { Base = new MaterialBaseFields { EnvironmentMapping = true, EnvironmentMappingMaskScale = 4f, DepthBias = true } };

            var bgem = Parse(fields.Build(10));

            Assert.True(bgem.EnvironmentMapping);
            Assert.Equal(4f, bgem.EnvironmentMappingMaskScale);
            Assert.True(bgem.DepthBias);
        }

        [Fact]
        public void Deserialize_AdaptiveEmissiveFromFifteenGlowmapFromSixteenPbrFromTwenty_AreVersionGated()
        {
            var fields = new BgemFields { AdaptiveOffset = 1f, AdaptiveMin = 2f, AdaptiveMax = 3f, Glowmap = true, EffectPbrSpecular = true };

            var v14 = Parse(fields.Build(14));
            var v15 = Parse(fields.Build(15));
            var v16 = Parse(fields.Build(16));
            var v20 = Parse(fields.Build(20));

            Assert.Equal(0f, v14.AdaptativeEmissive_ExposureOffset);
            Assert.Equal(1f, v15.AdaptativeEmissive_ExposureOffset);
            Assert.Equal(3f, v15.AdaptativeEmissive_FinalExposureMax);
            Assert.False(v15.Glowmap);
            Assert.True(v16.Glowmap);
            Assert.False(v16.EffectPbrSpecular);
            Assert.True(v20.EffectPbrSpecular);
        }

        [Fact]
        public void Deserialize_GlassBlockWithGlassDisabled_ReadsTheTwoTexturesAndFlagOnly()
        {
            var fields = new BgemFields { GlassRoughnessScratch = "scratch.dds", GlassDirtOverlay = "dirt.dds", GlassEnabled = false, SoftDepth = 33f };

            var bgem = Parse(fields.Build(21));

            Assert.False(bgem.GlassEnabled);
            Assert.Equal("scratch.dds", bgem.GlassRoughnessScratch);
            Assert.Equal("dirt.dds", bgem.GlassDirtOverlay);
            Assert.Equal(33f, bgem.SoftDepth);
        }

        [Fact]
        public void Deserialize_GlassEnabledVersion21_ReadsFresnelBlurAndRefractionWithoutBlurFactor()
        {
            var fields = new BgemFields
            {
                GlassEnabled = true,
                GlassFresnelColor = (1f, 0f, 0f),
                GlassBlurScaleBase = 0.3f,
                GlassBlurScaleFactor = 9f,
                GlassRefractionScaleBase = 0.6f,
                SoftDepth = 33f,
            };

            var bgem = Parse(fields.Build(21));

            Assert.True(bgem.GlassEnabled);
            Assert.Equal(0xFF0000u, bgem.GlassFresnelColor);
            Assert.Equal(0.3f, bgem.GlassBlurScaleBase);
            Assert.Equal(1f, bgem.GlassBlurScaleFactor);
            Assert.Equal(0.6f, bgem.GlassRefractionScaleBase);
            Assert.Equal(33f, bgem.SoftDepth);
        }

        [Fact]
        public void Deserialize_GlassEnabledVersion22_AlsoReadsTheBlurScaleFactor()
        {
            var fields = new BgemFields { GlassEnabled = true, GlassBlurScaleBase = 0.3f, GlassBlurScaleFactor = 9f, GlassRefractionScaleBase = 0.6f, SoftDepth = 33f };

            var bgem = Parse(fields.Build(22));

            Assert.Equal(0.3f, bgem.GlassBlurScaleBase);
            Assert.Equal(9f, bgem.GlassBlurScaleFactor);
            Assert.Equal(0.6f, bgem.GlassRefractionScaleBase);
            Assert.Equal(33f, bgem.SoftDepth);
        }

        [Fact]
        public void Deserialize_BgsmMagicInBgem_ThrowsFormatException()
        {
            byte[] bgsm = new BgsmFields().Build(2);

            Assert.Throws<FormatException>(() => Parse(bgsm));
        }

        [Fact]
        public void Deserialize_EveryTruncationOfAValidFile_ThrowsEndOfStreamOrInvalidData()
        {
            byte[] valid = new BgemFields { BaseTexture = "b.dds", GlassEnabled = true }.Build(22);
            var truncations = Enumerable.Range(0, valid.Length).Select(length => valid[..length]).ToList();

            // A cut inside a string leaves its length prefix larger than the bytes left, which is reported as malformed data.
            Assert.All(truncations, truncated => Assert.True(Record.Exception(() => Parse(truncated)) is EndOfStreamException or InvalidDataException));
        }

        [Theory]
        [InlineData(10u)]
        [InlineData(11u)]
        [InlineData(15u)]
        [InlineData(20u)]
        [InlineData(21u)]
        [InlineData(22u)]
        public void Serialize_ParsedFile_ReproducesTheOriginalBytesAtEveryVersion(uint version)
        {
            byte[] original = new BgemFields
            {
                BaseTexture = "b.dds",
                Normal = "n.dds",
                Glow = "glow.dds",
                Soft = true,
                SoftDepth = 50f,
                Glowmap = true,
                EffectPbrSpecular = true,
                GlassRoughnessScratch = "scratch.dds",
                GlassEnabled = true,
                EmitColor = (1f, 0f, 1f),
            }.Build(version);

            var bgem = Parse(original);
            using var output = new MemoryStream();
            using (var writer = new BinaryWriter(output, System.Text.Encoding.UTF8, leaveOpen: true))
                bgem.Serialize(writer);

            Assert.Equal(original, output.ToArray());
        }

        private static BGEM Parse(byte[] bytes)
        {
            using var reader = new BinaryReader(new MemoryStream(bytes));
            var bgem = new BGEM();
            bgem.Deserialize(reader);
            return bgem;
        }
    }
}
