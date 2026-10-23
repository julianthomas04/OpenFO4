using System.Buffers.Binary;
using System.Text;
using SharpBSABA2.BA2Util;
using SharpBSABA2.Enums;
using Xunit;

namespace InstallerTests.Archives
{
    // Archive layout: see BA2ArchiveFixtures.
    public class Ba2GeneralArchiveTests
    {
        private static readonly Ba2File Mesh = new(@"meshes\actors\dog\body.nif", TestBytes.Random(5000, seed: 1), Compress: true);
        private static readonly Ba2File Script = new(@"scripts\quest.pex", TestBytes.Random(300, seed: 2), Compress: false);

        [Fact]
        public void Open_GnrlArchive_ReadsEntryTable()
        {
            using var dir = new ArchiveTempDir();
            var archive = dir.OpenBa2("a.ba2", Ba2Builder.Gnrl([Mesh, Script]));

            Assert.Equal(ArchiveTypes.BA2_GNRL, archive.Type);
            Assert.Equal(2, archive.FileCount);
            Assert.True(archive.HasNameTable);
            Assert.Equal("1", archive.VersionString);
            Assert.Collection(archive.Files,
                e =>
                {
                    Assert.Equal(Native(Mesh.Name), e.FullPath);
                    Assert.True(e.Compressed);
                    Assert.Equal((uint)Mesh.Content.Length, e.RealSize);
                    Assert.Equal(0, e.Index);
                },
                e =>
                {
                    Assert.Equal(Native(Script.Name), e.FullPath);
                    Assert.False(e.Compressed);
                    Assert.Equal((uint)Script.Content.Length, e.RealSize);
                    Assert.Equal(1, e.Index);
                });
        }

        [Fact]
        public void Open_Version2Header_SkipsTwoExtraUnknownFields()
        {
            using var dir = new ArchiveTempDir();
            var archive = dir.OpenBa2("a.ba2", Ba2Builder.Gnrl([Mesh, Script], version: 2));

            Assert.Equal("2", archive.VersionString);
            Assert.Equal(Native(Mesh.Name), archive.Files[0].FullPath);
            Assert.Equal(Mesh.Content, archive.Files[0].GetDataStream().ToArray());
        }

        [Theory]
        [InlineData(7u)]
        [InlineData(8u)]
        public void Open_Version7And8Header_HasNoExtraFields(uint version)
        {
            using var dir = new ArchiveTempDir();
            var archive = dir.OpenBa2("a.ba2", Ba2Builder.Gnrl([Mesh, Script], version: version));

            Assert.Equal(version.ToString(System.Globalization.CultureInfo.InvariantCulture), archive.VersionString);
            Assert.Equal(2, archive.FileCount);
            Assert.Equal(Native(Mesh.Name), archive.Files[0].FullPath);
            Assert.Equal(Mesh.Content, archive.Files[0].GetDataStream().ToArray());
            Assert.Equal(Script.Content, archive.Files[1].GetDataStream().ToArray());
        }

        [Fact]
        public void GetDataStream_ZlibCompressedEntry_ReturnsOriginalBytes()
        {
            using var dir = new ArchiveTempDir();
            var archive = dir.OpenBa2("a.ba2", Ba2Builder.Gnrl([Mesh, Script]));

            Assert.Equal(Mesh.Content, archive.Files[0].GetDataStream().ToArray());
        }

        [Fact]
        public void GetDataStream_UncompressedEntry_ReturnsStoredBytes()
        {
            using var dir = new ArchiveTempDir();
            var archive = dir.OpenBa2("a.ba2", Ba2Builder.Gnrl([Mesh, Script]));

            Assert.Equal(Script.Content, archive.Files[1].GetDataStream().ToArray());
        }

        [Fact]
        public void GetRawDataStream_CompressedEntry_ReturnsStillCompressedBytes()
        {
            using var dir = new ArchiveTempDir();
            var archive = dir.OpenBa2("a.ba2", Ba2Builder.Gnrl([Mesh]));

            Assert.Equal(TestBytes.Zlib(Mesh.Content), archive.Files[0].GetRawDataStream().ToArray());
        }

        [Fact]
        public void Extract_PreserveFolder_WritesFileUnderItsArchivePath()
        {
            using var dir = new ArchiveTempDir();
            var archive = dir.OpenBa2("a.ba2", Ba2Builder.Gnrl([Mesh, Script]));
            string target = Path.Combine(dir.Path, "out");

            archive.Files[0].Extract(target, preserveFolder: true);

            Assert.Equal(Mesh.Content, File.ReadAllBytes(Path.Combine(target, Native(Mesh.Name))));
        }

        [Fact]
        public void Open_ArchiveWithoutNameTable_NamesEntriesByHash()
        {
            using var dir = new ArchiveTempDir();
            var archive = dir.OpenBa2("a.ba2", Ba2Builder.Gnrl([Mesh, Script], nameTable: false));

            Assert.False(archive.HasNameTable);
            Assert.Equal($"{Ba2Builder.DirHash(0):X}_{Ba2Builder.NameHash(0):X}.nif", archive.Files[0].FullPath);
            Assert.Equal($"{Ba2Builder.DirHash(1):X}_{Ba2Builder.NameHash(1):X}.pex", archive.Files[1].FullPath);
        }

        [Fact]
        public void Open_NameWithLatin1Byte_DecodesOneCharPerByte()
        {
            using var dir = new ArchiveTempDir();
            var file = new Ba2File("sounds\\café.wav", [1, 2, 3], Compress: false);
            var archive = dir.OpenBa2("a.ba2", Ba2Builder.Gnrl([file]));

            Assert.Equal(Native("sounds\\café.wav"), archive.Files[0].FullPath);
        }

        [Fact]
        public void Open_EmptyArchive_HasNoEntries()
        {
            using var dir = new ArchiveTempDir();
            var archive = dir.OpenBa2("a.ba2", Ba2Builder.Gnrl([]));

            Assert.Empty(archive.Files);
        }

        [Fact]
        public void GetDataStream_ZeroLengthStoredFile_ReturnsNoBytes()
        {
            using var dir = new ArchiveTempDir();
            var archive = dir.OpenBa2("a.ba2", Ba2Builder.Gnrl([new Ba2File("data\\empty.txt", [], Compress: false)]));

            Assert.Empty(archive.Files[0].GetDataStream().ToArray());
        }

        internal static string Native(string archivePath) => archivePath.Replace('\\', Path.DirectorySeparatorChar);
    }

    public class Ba2MalformedArchiveTests
    {
        private const int GnrlHeaderSize = 24;
        private const int GnrlEntrySize = 36;
        private const int NameTableOffsetField = 16;
        private const int FirstEntryDataOffsetField = GnrlHeaderSize + 4 + 4 + 4 + 4;

        [Fact]
        public void Open_WrongMagic_ThrowsInvalidData()
        {
            using var dir = new ArchiveTempDir();
            string path = dir.Write("bad.ba2", Ba2Builder.RawHeader("RIFF", 1, "GNRL", 0, 0));

            Assert.Throws<InvalidDataException>(() => new BA2(path));
        }

        [Fact]
        public void Open_UnknownArchiveType_ThrowsInvalidData()
        {
            using var dir = new ArchiveTempDir();
            string path = dir.Write("bad.ba2", Ba2Builder.RawHeader("BTDX", 1, "ZZZZ", 0, 0));

            Assert.Throws<InvalidDataException>(() => new BA2(path));
        }

        [Fact]
        public void Open_TruncatedHeader_ThrowsEndOfStream()
        {
            using var dir = new ArchiveTempDir();
            string path = dir.Write("bad.ba2", Ba2Builder.RawHeader("BTDX", 1, "GNRL", 0, 0)[..20]);

            Assert.Throws<EndOfStreamException>(() => new BA2(path));
        }

        [Fact]
        public void Open_EmptyFile_ThrowsInvalidData()
        {
            using var dir = new ArchiveTempDir();
            string path = dir.Write("empty.ba2", []);

            Assert.Throws<InvalidDataException>(() => new BA2(path));
        }

        [Fact]
        public void Open_TruncatedEntryTable_ThrowsInvalidData()
        {
            using var dir = new ArchiveTempDir();
            byte[] full = Ba2Builder.Gnrl([new Ba2File("a.bin", [1], false), new Ba2File("b.bin", [2], false)], nameTable: false);
            string path = dir.Write("bad.ba2", full[..(GnrlHeaderSize + GnrlEntrySize + 10)]);

            Assert.Throws<InvalidDataException>(() => new BA2(path));
        }

        [Theory]
        [InlineData(uint.MaxValue)]
        [InlineData(int.MaxValue)]
        [InlineData(1000u)]
        public void Open_FileCountLargerThanTheFileCanHold_ThrowsInvalidDataWithoutAllocating(uint declaredFiles)
        {
            using var dir = new ArchiveTempDir();
            string path = dir.Write("bad.ba2", Ba2Builder.RawHeader("BTDX", 1, "GNRL", declaredFiles, 0));

            Assert.Throws<InvalidDataException>(() => new BA2(path));
        }

        [Fact]
        public void Open_NameTableOffsetPastEndOfFile_ThrowsInvalidDataNamingTheNameTable()
        {
            using var dir = new ArchiveTempDir();
            var bytes = Ba2Builder.Gnrl([new Ba2File("a.bin", [1], false)]);
            BinaryPrimitives.WriteUInt64LittleEndian(bytes.AsSpan(NameTableOffsetField), (ulong)bytes.Length + 1000);
            string path = dir.Write("bad.ba2", bytes);

            var ex = Assert.Throws<InvalidDataException>(() => new BA2(path));
            Assert.Contains("name table", ex.Message, StringComparison.OrdinalIgnoreCase);
        }

        [Fact]
        public void Open_NameTableTruncatedMidString_ThrowsInvalidData()
        {
            using var dir = new ArchiveTempDir();
            var bytes = Ba2Builder.Gnrl([new Ba2File("long\\name\\here.bin", [1], false)]);
            string path = dir.Write("bad.ba2", bytes[..^5]);

            Assert.Throws<InvalidDataException>(() => new BA2(path));
        }

        [Fact]
        public void Open_NegativeNameLength_ThrowsInvalidData()
        {
            using var dir = new ArchiveTempDir();
            var bytes = Ba2Builder.Gnrl([new Ba2File("a.bin", [1], false)]);
            int lengthFieldOffset = bytes.Length - "a.bin".Length - 2;
            BinaryPrimitives.WriteInt16LittleEndian(bytes.AsSpan(lengthFieldOffset), -1);
            string path = dir.Write("bad.ba2", bytes);

            Assert.Throws<InvalidDataException>(() => new BA2(path));
        }

        [Fact]
        public void Open_Failure_ReleasesTheFileHandle()
        {
            using var dir = new ArchiveTempDir();
            string path = dir.Write("bad.ba2", Ba2Builder.RawHeader("RIFF", 1, "GNRL", 0, 0));

            Assert.Throws<InvalidDataException>(() => new BA2(path));

            File.Delete(path);
            Assert.False(File.Exists(path));
        }

        [Fact]
        public void GetDataStream_DataOffsetPastEndOfFile_ThrowsInsteadOfReturningEmptyData()
        {
            using var dir = new ArchiveTempDir();
            var bytes = Ba2Builder.Gnrl([new Ba2File("a.bin", TestBytes.Random(64, 3), false)]);
            BinaryPrimitives.WriteUInt64LittleEndian(bytes.AsSpan(FirstEntryDataOffsetField), (ulong)bytes.Length + 4096);
            var archive = dir.OpenBa2("bad.ba2", bytes);

            Assert.Throws<EndOfStreamException>(() => archive.Files[0].GetDataStream());
        }

        [Fact]
        public void GetDataStream_StoredDataRunsPastEndOfFile_Throws()
        {
            using var dir = new ArchiveTempDir();
            var bytes = Ba2Builder.Gnrl([new Ba2File("a.bin", TestBytes.Random(64, 3), false)], nameTable: false);
            var archive = dir.OpenBa2("bad.ba2", bytes[..^10]);

            Assert.Throws<EndOfStreamException>(() => archive.Files[0].GetDataStream());
        }

        [Fact]
        public void GetDataStream_CompressedDataTruncated_ThrowsInsteadOfReturningPartialData()
        {
            using var dir = new ArchiveTempDir();
            var bytes = Ba2Builder.Gnrl([new Ba2File("a.bin", TestBytes.Random(4000, 4), Compress: true)], nameTable: false);
            var archive = dir.OpenBa2("bad.ba2", bytes[..^200]);

            Assert.Throws<InvalidDataException>(() => archive.Files[0].GetDataStream());
        }

        [Fact]
        public void GetDataStream_CorruptDeflateStream_Throws()
        {
            using var dir = new ArchiveTempDir();
            var bytes = Ba2Builder.Gnrl([new Ba2File("a.bin", TestBytes.Repeating(4000, 7), Compress: true)], nameTable: false);
            for (int i = GnrlHeaderSize + GnrlEntrySize + 2; i < bytes.Length; i++)
                bytes[i] ^= 0xFF;
            var archive = dir.OpenBa2("bad.ba2", bytes);

            Assert.ThrowsAny<Exception>(() => archive.Files[0].GetDataStream());
        }
    }

    public class Ba2TextureArchiveTests
    {
        private const byte BC3 = 77;
        private const byte BC7 = 98;
        private const byte UnsupportedFormat = 1;
        private const int DdsMagicSize = 4;
        private const int DdsHeaderSize = 124;
        private const int Dxt10HeaderSize = 20;
        private const int PixelFormatFourCcOffset = 84;
        private const int Caps2Offset = 112;

        private static readonly byte[] Mip0 = TestBytes.Random(4096, seed: 10);
        private static readonly byte[] Mip1 = TestBytes.Random(1024, seed: 11);

        [Fact]
        public void Open_Dx10Archive_ReadsTextureDescriptionAndChunks()
        {
            using var dir = new ArchiveTempDir();
            var tex = new Ba2Texture(@"textures\a\wall_d.dds", 64, 32, 2, BC3, [new Ba2Chunk(Mip0, true, 0, 0), new Ba2Chunk(Mip1, false, 1, 1)]);
            var archive = dir.OpenBa2("t.ba2", Ba2Builder.Dx10([tex]));

            var entry = Assert.IsType<BA2TextureEntry>(Assert.Single(archive.Files));
            Assert.Equal(ArchiveTypes.BA2_DX10, archive.Type);
            Assert.Equal(Ba2GeneralArchiveTests.Native(tex.Name), entry.FullPath);
            Assert.Equal((ushort)64, entry.width);
            Assert.Equal((ushort)32, entry.height);
            Assert.Equal((byte)2, entry.numMips);
            Assert.Equal(BC3, entry.format);
            Assert.Equal(2, entry.Chunks.Count);
            Assert.Equal(Mip0.Length, (int)entry.Chunks[0].fullSz);
            Assert.Equal((ushort)1, entry.Chunks[1].startMip);
        }

        [Fact]
        public void GetDataStream_Bc3Texture_PrependsDdsHeaderAndConcatenatesDecompressedChunks()
        {
            using var dir = new ArchiveTempDir();
            var tex = new Ba2Texture(@"textures\a\wall_d.dds", 64, 32, 2, BC3, [new Ba2Chunk(Mip0, true), new Ba2Chunk(Mip1, false)]);
            var archive = dir.OpenBa2("t.ba2", Ba2Builder.Dx10([tex]));

            byte[] dds = archive.Files[0].GetDataStream().ToArray();

            Assert.Equal(DdsMagicSize + DdsHeaderSize + Mip0.Length + Mip1.Length, dds.Length);
            Assert.Equal("DDS ", Encoding.ASCII.GetString(dds, 0, 4));
            Assert.Equal(124u, U32(dds, 4));
            Assert.Equal(32u, U32(dds, 12));
            Assert.Equal(64u, U32(dds, 16));
            Assert.Equal(2u, U32(dds, 28));
            Assert.Equal("DXT5", Encoding.ASCII.GetString(dds, PixelFormatFourCcOffset, 4));
            Assert.Equal(Mip0.Concat(Mip1).ToArray(), dds[(DdsMagicSize + DdsHeaderSize)..]);
        }

        [Fact]
        public void GetDataStream_Bc7Texture_WritesDxt10ExtensionHeaderWithTheDxgiFormat()
        {
            using var dir = new ArchiveTempDir();
            var tex = new Ba2Texture(@"textures\a\x.dds", 16, 16, 1, BC7, [new Ba2Chunk(Mip1, true)]);
            var archive = dir.OpenBa2("t.ba2", Ba2Builder.Dx10([tex]));

            byte[] dds = archive.Files[0].GetDataStream().ToArray();

            Assert.Equal("DX10", Encoding.ASCII.GetString(dds, PixelFormatFourCcOffset, 4));
            Assert.Equal((uint)BC7, U32(dds, DdsMagicSize + DdsHeaderSize));
            Assert.Equal(Mip1, dds[(DdsMagicSize + DdsHeaderSize + Dxt10HeaderSize)..]);
        }

        [Fact]
        public void GetDataStream_CubemapTexture_SetsCubemapCapsOnTheDdsHeader()
        {
            using var dir = new ArchiveTempDir();
            var tex = new Ba2Texture(@"textures\cube.dds", 16, 16, 1, BC3, [new Ba2Chunk(Mip1, false)], Cubemap: true);
            var archive = dir.OpenBa2("t.ba2", Ba2Builder.Dx10([tex]));

            byte[] dds = archive.Files[0].GetDataStream().ToArray();

            const uint ddsCaps2Cubemap = 0x200;
            Assert.NotEqual(0u, U32(dds, Caps2Offset) & ddsCaps2Cubemap);
        }

        [Fact]
        public void GetDataStream_UnsupportedDxgiFormat_ThrowsUnsupportedDds()
        {
            using var dir = new ArchiveTempDir();
            var tex = new Ba2Texture(@"textures\odd.dds", 16, 16, 1, UnsupportedFormat, [new Ba2Chunk(Mip1, false)]);
            var archive = dir.OpenBa2("t.ba2", Ba2Builder.Dx10([tex]));
            var entry = (BA2TextureEntry)archive.Files[0];

            Assert.False(entry.IsFormatSupported());
            Assert.Throws<UnsupportedDDSException>(() => entry.GetDataStream());
        }

        [Fact]
        public void Open_SeveralTextures_ParsesEachEntryAtItsOwnOffset()
        {
            using var dir = new ArchiveTempDir();
            var first = new Ba2Texture(@"textures\a_d.dds", 8, 8, 1, BC3, [new Ba2Chunk(Mip1, true), new Ba2Chunk(Mip1, false)]);
            var second = new Ba2Texture(@"textures\b_d.dds", 4, 4, 1, BC3, [new Ba2Chunk(Mip0, false)]);
            var archive = dir.OpenBa2("t.ba2", Ba2Builder.Dx10([first, second]));

            Assert.Equal(Ba2GeneralArchiveTests.Native(second.Name), archive.Files[1].FullPath);
            Assert.Equal((ushort)4, ((BA2TextureEntry)archive.Files[1]).width);
            Assert.Equal(Mip0, archive.Files[1].GetDataStream().ToArray()[(DdsMagicSize + DdsHeaderSize)..]);
        }

        private static uint U32(byte[] data, int offset) => BinaryPrimitives.ReadUInt32LittleEndian(data.AsSpan(offset));
    }
}
