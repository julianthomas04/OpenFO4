using AssetConverter.Havok;
using Xunit;

namespace InstallerTests.Havok
{
    // Packfile layout.
    public class HavokPackfileTests
    {
        private const int ContentsVersionField = 0x28;
        private const int FileVersionField = 0x0C;
        private const int PointerSizeField = 0x10;
        private const int EndiannessField = 0x11;
        private const int SectionCountField = 0x14;
        private const int DataSectionHeader = HkxBuilder.GlobalHeaderSize + HkxBuilder.SectionHeaderSize;

        [Fact]
        public void Load_MinimalPackfile_ListsSectionsAndObjectsByClass()
        {
            var b = new HkxBuilder();
            int first = b.NewObject("hkFoo", 0x20);
            int second = b.NewObject("hkBar", 0x20);

            var pack = HavokPackfile.Load(b.Build());

            Assert.Equal(["__classnames__", "__data__"], pack.Sections.Select(s => s.Name));
            Assert.Equal("hk_2014.1.0-r1", pack.ContentsVersion);
            Assert.Equal(["hkFoo", "hkBar"], pack.Objects.Select(o => o.ClassName));
            Assert.Equal(second - first, pack.Objects[1].Offset - pack.Objects[0].Offset);
        }

        [Fact]
        public void ObjectsOfClass_FiltersByExactClassName()
        {
            var b = new HkxBuilder();
            b.NewObject("hkFoo", 0x10);
            b.NewObject("hkBar", 0x10);
            b.NewObject("hkFoo", 0x10);

            var pack = HavokPackfile.Load(b.Build());

            Assert.Equal(2, pack.ObjectsOfClass("hkFoo").Count());
            Assert.Empty(pack.ObjectsOfClass("hkFo"));
        }

        [Fact]
        public void Load_ObjectsRegisteredOutOfOrder_AreSortedByOffset()
        {
            var b = new HkxBuilder();
            b.NewObject("hkFoo", 0x10);
            b.NewObject("hkBar", 0x10);

            var pack = HavokPackfile.Load(b.Build());

            Assert.True(pack.Objects[0].Offset < pack.Objects[1].Offset);
        }

        [Fact]
        public void ReadPointer_FieldWithLocalFixup_ResolvesToTheAbsoluteTargetOffset()
        {
            var b = new HkxBuilder();
            int holder = b.NewObject("hkHolder", 0x20);
            int target = b.NewObject("hkTarget", 0x20);
            b.Ptr(holder + 8, target);
            var pack = HavokPackfile.Load(b.Build());

            int? resolved = pack.ReadPointer(pack.ObjectsOfClass("hkHolder").Single().Offset + 8);

            Assert.Equal(pack.ObjectsOfClass("hkTarget").Single().Offset, resolved);
        }

        [Fact]
        public void ReadPointer_ZeroFieldWithoutFixup_IsNull()
        {
            var b = new HkxBuilder();
            b.NewObject("hkHolder", 0x20);
            var pack = HavokPackfile.Load(b.Build());

            Assert.Null(pack.ReadPointer(pack.Objects[0].Offset));
        }

        [Fact]
        public void ReadPointer_NonZeroFieldWithoutFixup_ThrowsFormatException()
        {
            var b = new HkxBuilder();
            int holder = b.NewObject("hkHolder", 0x20);
            b.U32(holder, 0x1234);
            var pack = HavokPackfile.Load(b.Build());

            Assert.Throws<HavokFormatException>(() => pack.ReadPointer(pack.Objects[0].Offset));
        }

        [Fact]
        public void ReadPointer_FieldBeyondTheBuffer_ThrowsFormatException()
        {
            var pack = HavokPackfile.Load(new HkxBuilder().Build());

            Assert.Throws<HavokFormatException>(() => pack.ReadPointer(1_000_000));
        }

        [Fact]
        public void RequireObjectPointer_TargetOfAnotherClass_ThrowsUnsupportedClass()
        {
            var b = new HkxBuilder();
            int holder = b.NewObject("hkHolder", 0x20);
            b.Ptr(holder, b.NewObject("hkOther", 0x10));
            var pack = HavokPackfile.Load(b.Build());

            var ex = Assert.Throws<HavokUnsupportedClassException>(
                () => pack.RequireObjectPointer(pack.ObjectsOfClass("hkHolder").Single().Offset, "hkWanted", "field"));
            Assert.Equal("hkOther", ex.ClassName);
        }

        [Fact]
        public void RequirePointer_NullPointer_ThrowsFormatException()
        {
            var b = new HkxBuilder();
            b.NewObject("hkHolder", 0x20);
            var pack = HavokPackfile.Load(b.Build());

            Assert.Throws<HavokFormatException>(() => pack.RequirePointer(pack.Objects[0].Offset, "field"));
        }

        [Fact]
        public void ReadString_PointerToCString_ReturnsTheText()
        {
            var b = new HkxBuilder();
            int holder = b.NewObject("hkHolder", 0x20);
            b.StringField(holder, "Bip01 Pelvis");
            var pack = HavokPackfile.Load(b.Build());

            Assert.Equal("Bip01 Pelvis", pack.ReadString(pack.Objects[0].Offset));
        }

        [Fact]
        public void ReadArray_ValidArray_ReturnsPayloadOffsetAndCount()
        {
            var b = new HkxBuilder();
            int holder = b.NewObject("hkHolder", 0x20);
            int payload = b.Array(holder, 4, 5);
            var pack = HavokPackfile.Load(b.Build());

            var array = pack.ReadArray(pack.Objects[0].Offset, 4, "ints");

            Assert.Equal(5, array.Count);
            Assert.Equal(pack.Objects[0].Offset + (payload - holder), array.DataOffset);
        }

        [Fact]
        public void ReadArray_EmptyArray_NeedsNoPointer()
        {
            var b = new HkxBuilder();
            b.NewObject("hkHolder", 0x20);
            var pack = HavokPackfile.Load(b.Build());

            Assert.Equal(new HavokArrayRef(0, 0), pack.ReadArray(pack.Objects[0].Offset, 4, "ints"));
        }

        [Fact]
        public void ReadArray_CountLargerThanTheBuffer_ThrowsFormatException()
        {
            var b = new HkxBuilder();
            int holder = b.NewObject("hkHolder", 0x20);
            b.Array(holder, 4, 2);
            b.U32(holder + 8, uint.MaxValue);
            var pack = HavokPackfile.Load(b.Build());

            Assert.Throws<HavokFormatException>(() => pack.ReadArray(pack.Objects[0].Offset, 16, "ints"));
        }

        [Fact]
        public void Load_TerminatorEntriesPadTheFixupTables()
        {
            var b = new HkxBuilder { FixupTerminatorPadding = 3 };
            int holder = b.NewObject("hkHolder", 0x20);
            b.StringField(holder, "x");

            var pack = HavokPackfile.Load(b.Build());

            Assert.Equal("x", pack.ReadString(pack.Objects[0].Offset));
        }

        [Fact]
        public void Load_BadMagic_ThrowsFormatException()
        {
            var bytes = new HkxBuilder().Build();
            bytes[0] ^= 0xFF;

            Assert.Throws<HavokFormatException>(() => HavokPackfile.Load(bytes));
        }

        [Fact]
        public void Load_TruncatedHeader_ThrowsFormatException()
        {
            var bytes = new HkxBuilder().Build();

            Assert.Throws<HavokFormatException>(() => HavokPackfile.Load(bytes[..0x30]));
        }

        [Fact]
        public void Load_EmptyBuffer_ThrowsFormatException()
        {
            Assert.Throws<HavokFormatException>(() => HavokPackfile.Load([]));
        }

        [Fact]
        public void Load_UnsupportedFileVersion_ThrowsFormatException()
        {
            Assert.Throws<HavokFormatException>(() => HavokPackfile.Load(new HkxBuilder { FileVersion = 8 }.Build()));
        }

        [Fact]
        public void Load_ThirtyTwoBitPointers_ThrowsFormatException()
        {
            Assert.Throws<HavokFormatException>(() => HavokPackfile.Load(new HkxBuilder { PointerBytes = 4 }.Build()));
        }

        [Fact]
        public void Load_BigEndianFile_ThrowsFormatException()
        {
            Assert.Throws<HavokFormatException>(() => HavokPackfile.Load(new HkxBuilder { LittleEndian = 0 }.Build()));
        }

        [Fact]
        public void Load_OtherHavokVersion_ThrowsFormatException()
        {
            Assert.Throws<HavokFormatException>(() => HavokPackfile.Load(new HkxBuilder { ContentsVersion = "hk_2012.2.0-r1" }.Build()));
        }

        [Theory]
        [InlineData(0)]
        [InlineData(17)]
        [InlineData(-1)]
        public void Load_ImplausibleSectionCount_ThrowsFormatException(int sectionCount)
        {
            var bytes = new HkxBuilder().Build();
            BitConverter.GetBytes(sectionCount).CopyTo(bytes, SectionCountField);

            Assert.Throws<HavokFormatException>(() => HavokPackfile.Load(bytes));
        }

        [Fact]
        public void Load_SectionCountBeyondTheFile_ThrowsFormatException()
        {
            var bytes = new HkxBuilder().Build();
            BitConverter.GetBytes(16).CopyTo(bytes, SectionCountField);

            Assert.Throws<HavokFormatException>(() => HavokPackfile.Load(bytes));
        }

        [Fact]
        public void Load_SectionDataBeyondTheFile_ThrowsFormatException()
        {
            var bytes = new HkxBuilder().Build();
            BitConverter.GetBytes(uint.MaxValue - 100).CopyTo(bytes, DataSectionHeader + 0x14);

            Assert.Throws<HavokFormatException>(() => HavokPackfile.Load(bytes));
        }

        [Fact]
        public void Load_FixupTablesOutOfOrder_ThrowsFormatException()
        {
            var b = new HkxBuilder();
            b.NewObject("hkHolder", 0x20);
            var bytes = b.Build();
            uint local = BitConverter.ToUInt32(bytes, DataSectionHeader + 0x18);
            BitConverter.GetBytes(local - 4).CopyTo(bytes, DataSectionHeader + 0x1C);

            Assert.Throws<HavokFormatException>(() => HavokPackfile.Load(bytes));
        }

        [Fact]
        public void Load_LocalFixupSourceOutsideTheSection_ThrowsFormatException()
        {
            var b = new HkxBuilder();
            int holder = b.NewObject("hkHolder", 0x20);
            b.Ptr(holder + 1_000_000, holder);

            Assert.Throws<HavokFormatException>(() => HavokPackfile.Load(b.Build()));
        }

        [Fact]
        public void Load_LocalFixupTargetOutsideTheSection_ThrowsFormatException()
        {
            var b = new HkxBuilder();
            int holder = b.NewObject("hkHolder", 0x20);
            b.Ptr(holder, 1_000_000);

            Assert.Throws<HavokFormatException>(() => HavokPackfile.Load(b.Build()));
        }

        [Fact]
        public void Load_ContentsVersionFieldIsTerminatedWithinSixteenBytes()
        {
            var bytes = new HkxBuilder().Build();
            Array.Fill(bytes, (byte)'x', ContentsVersionField, 16);

            Assert.Throws<HavokFormatException>(() => HavokPackfile.Load(bytes));
        }

        [Fact]
        public void RequireObjectAt_OffsetWithoutObject_ThrowsFormatException()
        {
            var b = new HkxBuilder();
            b.NewObject("hkHolder", 0x20);
            var pack = HavokPackfile.Load(b.Build());

            Assert.Throws<HavokFormatException>(() => pack.RequireObjectAt(pack.Objects[0].Offset + 4, "thing"));
        }

        [Fact]
        public void Header_FieldOffsetsUsedByTheBuilderMatchTheLoader()
        {
            var bytes = new HkxBuilder().Build();

            Assert.Equal(11, BitConverter.ToInt32(bytes, FileVersionField));
            Assert.Equal(8, bytes[PointerSizeField]);
            Assert.Equal(1, bytes[EndiannessField]);
        }
    }
}
