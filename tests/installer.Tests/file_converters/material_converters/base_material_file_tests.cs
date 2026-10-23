using AssetConverter;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests
{
    // Base-class layout (BGSMBase fields),
    // exercised through BGSM because the base class is abstract.
    public class BaseMaterialFileTests
    {
        [Fact]
        public void Deserialize_BaseFields_AreReadInFileOrder()
        {
            var fields = new BgsmFields
            {
                Base = new MaterialBaseFields
                {
                    TileFlags = 2,
                    UOffset = 0.25f,
                    VOffset = -0.5f,
                    UScale = 2f,
                    VScale = 4f,
                    Alpha = 0.75f,
                    AlphaTestRef = 200,
                    AlphaTest = true,
                    ZBufferWrite = false,
                    ZBufferTest = true,
                    ScreenSpaceReflections = true,
                    WetnessControlSsr = false,
                    Decal = true,
                    TwoSided = true,
                    DecalNoFade = false,
                    NonOccluder = true,
                    Refraction = true,
                    RefractionFalloff = false,
                    RefractionPower = 0.6f,
                    GrayscaleToPalette = true,
                },
            };

            var bgsm = Parse(fields.Build(2));

            Assert.True(bgsm.TileU);
            Assert.False(bgsm.TileV);
            Assert.Equal(0.25f, bgsm.UOffset);
            Assert.Equal(-0.5f, bgsm.VOffset);
            Assert.Equal(2f, bgsm.UScale);
            Assert.Equal(4f, bgsm.VScale);
            Assert.Equal(0.75f, bgsm.Alpha);
            Assert.Equal(200, bgsm.AlphaTestRef);
            Assert.True(bgsm.AlphaTest);
            Assert.False(bgsm.ZBufferWrite);
            Assert.True(bgsm.ZBufferTest);
            Assert.True(bgsm.ScreenSpaceReflections);
            Assert.False(bgsm.WetnessControlScreenSpaceReflections);
            Assert.True(bgsm.Decal);
            Assert.True(bgsm.TwoSided);
            Assert.False(bgsm.DecalNoFade);
            Assert.True(bgsm.NonOccluder);
            Assert.True(bgsm.Refraction);
            Assert.False(bgsm.RefractionFalloff);
            Assert.Equal(0.6f, bgsm.RefractionPower);
            Assert.True(bgsm.GrayscaleToPaletteColor);
        }

        [Theory]
        [InlineData(0u, false, false)]
        [InlineData(1u, false, true)]
        [InlineData(2u, true, false)]
        [InlineData(3u, true, true)]
        public void Deserialize_TileFlags_BitOneIsUAndBitZeroIsV(uint flags, bool expectedU, bool expectedV)
        {
            var bgsm = Parse(new BgsmFields { Base = new MaterialBaseFields { TileFlags = flags } }.Build(2));

            Assert.Equal(expectedU, bgsm.TileU);
            Assert.Equal(expectedV, bgsm.TileV);
        }

        [Theory]
        [InlineData(0, 6u, 7u, BaseMaterialFile.AlphaBlendModeType.Unknown)]
        [InlineData(0, 0u, 0u, BaseMaterialFile.AlphaBlendModeType.None)]
        [InlineData(1, 6u, 7u, BaseMaterialFile.AlphaBlendModeType.Standard)]
        [InlineData(1, 6u, 0u, BaseMaterialFile.AlphaBlendModeType.Additive)]
        [InlineData(1, 4u, 1u, BaseMaterialFile.AlphaBlendModeType.Multiplicative)]
        public void Deserialize_BlendFactorTriple_MapsToBlendMode(byte a, uint b, uint c, BaseMaterialFile.AlphaBlendModeType expected)
        {
            var bgsm = Parse(new BgsmFields { Base = new MaterialBaseFields { BlendA = a, BlendB = b, BlendC = c } }.Build(2));

            Assert.Equal(expected, bgsm.AlphaBlendMode);
        }

        [Fact]
        public void Deserialize_UnknownBlendFactorTriple_ThrowsNotSupported()
        {
            byte[] bytes = new BgsmFields { Base = new MaterialBaseFields { BlendA = 1, BlendB = 3, BlendC = 3 } }.Build(2);

            Assert.Throws<NotSupportedException>(() => Parse(bytes));
        }

        [Fact]
        public void Deserialize_WrongMagic_ThrowsFormatException()
        {
            byte[] bytes = new BgsmFields().Build(2);
            bytes[0] = (byte)'X';

            Assert.Throws<FormatException>(() => Parse(bytes));
        }

        [Fact]
        public void Deserialize_BgemMagicInBgsm_ThrowsFormatException()
        {
            byte[] bgem = new BgemFields().Build(20);

            Assert.Throws<FormatException>(() => Parse(bgem));
        }

        [Fact]
        public void Deserialize_VersionBelowTen_ReadsEnvironmentMappingFields()
        {
            var fields = new BgsmFields { Base = new MaterialBaseFields { EnvironmentMapping = true, EnvironmentMappingMaskScale = 3f } };

            var bgsm = Parse(fields.Build(2));

            Assert.True(bgsm.EnvironmentMapping);
            Assert.Equal(3f, bgsm.EnvironmentMappingMaskScale);
            Assert.False(bgsm.DepthBias);
        }

        [Fact]
        public void Deserialize_VersionTenOrAbove_ReadsDepthBiasInsteadOfEnvironmentMapping()
        {
            var fields = new BgsmFields { Base = new MaterialBaseFields { DepthBias = true } };

            var bgsm = Parse(fields.Build(10));

            Assert.True(bgsm.DepthBias);
            Assert.False(bgsm.EnvironmentMapping);
            Assert.Equal(1f, bgsm.EnvironmentMappingMaskScale);
        }

        [Fact]
        public void Deserialize_VersionBelowSix_KeepsAllMaskWriteChannelsEnabled()
        {
            var bgsm = Parse(new BgsmFields().Build(5));

            Assert.Equal(
                BaseMaterialFile.MaskWriteChannels.ALBEDO | BaseMaterialFile.MaskWriteChannels.NORMAL | BaseMaterialFile.MaskWriteChannels.SPECULAR
                | BaseMaterialFile.MaskWriteChannels.AMBIENT_OCCLUSION | BaseMaterialFile.MaskWriteChannels.EMISSIVE | BaseMaterialFile.MaskWriteChannels.GLOSS,
                bgsm.MaskWrites);
        }

        [Fact]
        public void Deserialize_VersionSixOrAbove_ReadsMaskWriteByte()
        {
            var fields = new BgsmFields { Base = new MaterialBaseFields { MaskWrites = 0b000101 } };

            var bgsm = Parse(fields.Build(6));

            Assert.Equal(BaseMaterialFile.MaskWriteChannels.ALBEDO | BaseMaterialFile.MaskWriteChannels.SPECULAR, bgsm.MaskWrites);
        }

        [Fact]
        public void Deserialize_StringWithTrailingNul_StripsTheNul()
        {
            var bgsm = Parse(new BgsmFields { Diffuse = "textures\\a_d.dds" }.Build(2));

            Assert.Equal("textures\\a_d.dds", bgsm.DiffuseTexture);
        }

        [Fact]
        public void Deserialize_ZeroLengthString_IsEmpty()
        {
            // Format: "An empty string has length = 0 with no following bytes."
            var bgsm = Parse(new BgsmFields { DiffuseAsBareZeroLength = true, Normal = "n.dds" }.Build(2));

            Assert.Equal("", bgsm.DiffuseTexture);
            Assert.Equal("n.dds", bgsm.NormalTexture);
        }

        [Fact]
        public void Deserialize_EveryTruncationOfAValidFile_ThrowsEndOfStreamOrInvalidData()
        {
            byte[] valid = new BgsmFields { Diffuse = "a.dds", Terrain = true }.Build(8);
            var truncations = Enumerable.Range(0, valid.Length).Select(length => valid[..length]).ToList();

            // A cut inside a string leaves its length prefix larger than the bytes left, which is reported as malformed data.
            Assert.All(truncations, truncated => Assert.True(Record.Exception(() => Parse(truncated)) is EndOfStreamException or InvalidDataException));
        }

        [Fact]
        public void Deserialize_StringLengthBeyondTheStream_ThrowsInvalidDataNamingTheLength()
        {
            byte[] bytes = new BgsmFields { Diffuse = "a.dds" }.Build(2);
            int lengthOffset = bytes.AsSpan().IndexOf("a.dds\0"u8) - sizeof(uint);
            BitConverter.GetBytes(uint.MaxValue).CopyTo(bytes, lengthOffset);

            var thrown = Assert.Throws<InvalidDataException>(() => Parse(bytes));

            Assert.Contains("4294967295", thrown.Message);
        }

        [Fact]
        public void Deserialize_NonAsciiStringBytes_AreDecodedAsLatin1()
        {
            byte[] bytes = new BgsmFields { Diffuse = "a_b.dds" }.Build(2);
            bytes[bytes.AsSpan().IndexOf("a_b.dds"u8) + 1] = 0xE9;

            Assert.Equal("aéb.dds", Parse(bytes).DiffuseTexture);
        }

        [Fact]
        public void Open_ValidFile_PopulatesFieldsAndLeavesTheStreamOpen()
        {
            using var file = TempMaterialFile.Write(new BgsmFields { Diffuse = "a.dds" }.Build(2));
            var bgsm = new BGSM();

            bool streamStillOpen = file.Use(stream =>
            {
                bgsm.Open(stream);
                return stream.CanRead;
            });

            Assert.Equal("a.dds", bgsm.DiffuseTexture);
            Assert.True(streamStillOpen);
        }

        [Fact]
        public void Open_TruncatedFile_ThrowsEndOfStream()
        {
            byte[] valid = new BgsmFields().Build(2);
            using var file = TempMaterialFile.Write(valid[..40]);

            Assert.Throws<EndOfStreamException>(() => file.Use(stream => { new BGSM().Open(stream); return 0; }));
        }

        [Fact]
        public void Open_WrongMagic_ThrowsInvalidDataWithTheCauseAttached()
        {
            using var file = TempMaterialFile.Write(new BgemFields().Build(20));

            var thrown = Assert.Throws<InvalidDataException>(() => file.Use(stream => { new BGSM().Open(stream); return 0; }));

            Assert.IsType<FormatException>(thrown.InnerException);
        }

        [Fact]
        public void Serialize_ParsedFile_ReproducesTheOriginalBytes()
        {
            byte[] original = new BgsmFields
            {
                Base = new MaterialBaseFields { TileFlags = 1, Alpha = 0.5f, BlendA = 1, BlendB = 6, BlendC = 7, Decal = true },
                Diffuse = "a.dds",
                Normal = "a_n.dds",
                SpecularEnabled = true,
                SmoothSpec = "a_s.dds",
            }.Build(2);

            var bgsm = Parse(original);
            using var output = new MemoryStream();
            using (var writer = new BinaryWriter(output, System.Text.Encoding.UTF8, leaveOpen: true))
                bgsm.Serialize(writer);

            Assert.Equal(original, output.ToArray());
        }

        private static BGSM Parse(byte[] bytes)
        {
            using var reader = new BinaryReader(new MemoryStream(bytes));
            var bgsm = new BGSM();
            bgsm.Deserialize(reader);
            return bgsm;
        }

    }
}
