using AssetConverter.ShaderTranslation;
using AssetConverter.ShaderTranslation.Archive;
using AssetConverter.ShaderTranslation.Dxbc;
using InstallerTests.Archives;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests.ShaderTranslation
{
    internal static class FxpBuilder
    {
        public static byte[] Group(uint vertexCount, uint pixelCount, params (uint Descriptor, byte[] SlotTable, byte[] Dxbc)[] entries)
        {
            using var ms = new MemoryStream();
            using var bw = new BinaryWriter(ms);
            bw.Write(vertexCount);
            bw.Write(0u);
            bw.Write(0u);
            bw.Write(pixelCount);
            bw.Write(0u);
            foreach (var (descriptor, table, dxbc) in entries)
            {
                bw.Write(FxpParser.EntryMarker);
                bw.Write((uint)dxbc.Length);
                bw.Write(descriptor);
                bw.Write(table);
                bw.Write(dxbc);
            }
            return ms.ToArray();
        }

        public static byte[] Concat(params byte[][] parts) => parts.SelectMany(p => p).ToArray();

        public static byte[] TinyDxbc(ShaderStage stage) =>
            new Sm5Assembler(stage).Temps(1).Instruction(Opcode.Ret, 0).BuildDxbc();
    }

    public class FxpParserTests
    {
        private static readonly byte[] Vs = FxpBuilder.TinyDxbc(ShaderStage.Vertex);
        private static readonly byte[] Ps = FxpBuilder.TinyDxbc(ShaderStage.Pixel);

        [Fact]
        public void Parse_TwoGroups_AssignsGlobalBlobIdsAndGroupIndices()
        {
            byte[] table = new byte[44];
            Array.Fill(table, (byte)0xFF);
            var fxp = FxpParser.Parse(FxpBuilder.Concat(
                FxpBuilder.Group(1, 1, (0xA5E, table, Vs), (0x200, new byte[36], Ps)),
                FxpBuilder.Group(0, 1, (0x10200, new byte[36], Ps))));

            Assert.Equal(2, fxp.Groups.Count);
            Assert.Equal(1u, fxp.Groups[0].VertexShaderCount);
            Assert.Equal(1u, fxp.Groups[0].PixelShaderCount);
            Assert.Collection(fxp.Entries,
                e =>
                {
                    Assert.Equal((0, 0, 0xA5Eu), (e.BlobId, e.GroupIndex, e.Descriptor));
                    Assert.Equal(table, e.SlotTable.ToArray());
                    Assert.Equal(Vs, e.Dxbc.ToArray());
                },
                e => Assert.Equal((1, 0, 0x200u), (e.BlobId, e.GroupIndex, e.Descriptor)),
                e =>
                {
                    Assert.Equal((2, 1, 0x10200u), (e.BlobId, e.GroupIndex, e.Descriptor));
                    Assert.Equal(Ps, e.Dxbc.ToArray());
                });
        }

        [Fact]
        public void Parse_EmptyInput_YieldsNoGroups()
        {
            var fxp = FxpParser.Parse(Array.Empty<byte>());

            Assert.Empty(fxp.Groups);
            Assert.Empty(fxp.Entries);
        }

        [Fact]
        public void Parse_SlotTableLengthDiffersPerEntry_FindsDxbcByDeclaredSize()
        {
            var fxp = FxpParser.Parse(FxpBuilder.Concat(
                FxpBuilder.Group(2, 0, (1, new byte[44], Vs), (2, new byte[7], Ps))));

            Assert.Equal(44, fxp.Entries[0].SlotTable.Length);
            Assert.Equal(7, fxp.Entries[1].SlotTable.Length);
            Assert.Equal(Ps, fxp.Entries[1].Dxbc.ToArray());
        }

        [Fact]
        public void Parse_EntryBeforeAnyGroupHeader_Throws()
        {
            byte[] data = new byte[20];
            BitConverter.GetBytes(FxpParser.EntryMarker).CopyTo(data, 0);

            var ex = Assert.Throws<FxpFormatException>(() => FxpParser.Parse(data));
            Assert.Equal(0, ex.Offset);
        }

        [Fact]
        public void Parse_TruncatedGroupHeader_Throws()
        {
            Assert.Throws<FxpFormatException>(() => FxpParser.Parse(new byte[12]));
        }

        [Fact]
        public void Parse_DxbcSizeMismatch_Throws()
        {
            byte[] group = FxpBuilder.Group(1, 0, (1, new byte[8], Vs));
            BitConverter.GetBytes((uint)Vs.Length + 4).CopyTo(group, 24);

            Assert.Throws<FxpFormatException>(() => FxpParser.Parse(group));
        }

        [Fact]
        public void Parse_TruncatedDxbc_Throws()
        {
            byte[] group = FxpBuilder.Group(1, 0, (1, new byte[8], Vs));

            Assert.Throws<FxpFormatException>(() => FxpParser.Parse(group.AsMemory(0, group.Length - 8)));
        }
    }

    public class ShaderArchiveReaderTests
    {
        private static readonly byte[] Fxp = FxpBuilder.Group(1, 0, (7, new byte[44], FxpBuilder.TinyDxbc(ShaderStage.Vertex)));

        [Fact]
        public void ReadFxp_Version8Archive_ReturnsParsedEntries()
        {
            using var dir = new ArchiveTempDir();
            string path = dir.Write("shaders.ba2", Ba2Builder.Gnrl([new Ba2File(@"ShadersFX\Shaders011.fxp", Fxp, Compress: false)], version: 8));

            var fxp = ShaderArchiveReader.ReadFxp(path);

            Assert.Single(fxp.Entries);
            Assert.Equal(7u, fxp.Entries[0].Descriptor);
        }

        [Fact]
        public void ReadFxp_CompressedEntry_IsDecompressed()
        {
            using var dir = new ArchiveTempDir();
            string path = dir.Write("shaders.ba2", Ba2Builder.Gnrl([new Ba2File(@"shadersfx/shaders011.fxp", Fxp, Compress: true)], version: 7));

            Assert.Single(ShaderArchiveReader.ReadFxp(path).Entries);
        }

        [Fact]
        public void ReadFxp_ArchiveWithoutFxp_Throws()
        {
            using var dir = new ArchiveTempDir();
            string path = dir.Write("other.ba2", Ba2Builder.Gnrl([new Ba2File(@"scripts\a.pex", new byte[10], Compress: false)], version: 8));

            var ex = Assert.Throws<ShaderArchiveException>(() => ShaderArchiveReader.ReadFxp(path));
            Assert.Contains("shaders011.fxp", ex.Message);
        }

        [Fact]
        public void ReadFxp_MissingFile_Throws()
        {
            using var dir = new ArchiveTempDir();

            Assert.Throws<ShaderArchiveException>(() => ShaderArchiveReader.ReadFxp(Path.Combine(dir.Path, "nope.ba2")));
        }

        [Fact]
        public void ReadFxp_NotABa2_Throws()
        {
            using var dir = new ArchiveTempDir();
            string path = dir.Write("junk.ba2", new byte[64]);

            Assert.Throws<ShaderArchiveException>(() => ShaderArchiveReader.ReadFxp(path));
        }
    }
}
