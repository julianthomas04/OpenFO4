using AssetConverter;
using Xunit;

namespace InstallerTests
{
    // Raw plugin scans for data Mutagen does not expose for Fallout 4 (docs/installer/database.md).
    // Form id plugin indices: 0..N-1 are the masters in MAST order, N is the plugin itself.
    public sealed class EsmRawScanTests : IDisposable
    {
        const string Self = "Mod.esm";
        const string Master = "Fallout4.esm";
        const uint SelfIndex = 0x01000000;

        readonly TempDirectory _directory = new();

        public void Dispose() => _directory.Dispose();

        string Write(RawEsmBuilder builder) => builder.WriteTo(_directory.Path, Self);

        static RawEsmBuilder Plugin() => RawEsmBuilder.Plugin(Master);

        static byte[] Mods(uint swap) => RawEsmBuilder.FormIdSubrecord("MODS", swap);

        static byte[] Modc(float value) => RawEsmBuilder.FloatSubrecord("MODC", value);

        static byte[] Xmsp(uint swap) => RawEsmBuilder.FormIdSubrecord("XMSP", swap);

        static byte[] Edid(string name) => RawEsmBuilder.Subrecord("EDID", System.Text.Encoding.ASCII.GetBytes(name + "\0"));

        // ---- material swap links ----------------------------------------------------------------------

        [Fact]
        public void ScanMaterialSwapLinks_BaseWithSwapAndColorIndex_ReportsBoth()
        {
            string path = Write(Plugin().Group("STAT", g => g.Record("STAT", SelfIndex | 0x900, Edid("Crate"), Mods(SelfIndex | 0x123), Modc(0.25f))));

            var (refrSwaps, baseSwaps) = CellProcessor.ScanMaterialSwapLinks(path);

            Assert.Empty(refrSwaps);
            Assert.Equal([("000900:Mod.esm", "000123:Mod.esm", (float?)0.25f)], baseSwaps);
        }

        [Fact]
        public void ScanMaterialSwapLinks_ColorIndexWithoutSwap_HasNullSwapKey()
        {
            string path = Write(Plugin().Group("STAT", g => g.Record("STAT", SelfIndex | 0x900, Modc(0.5f))));

            var (_, baseSwaps) = CellProcessor.ScanMaterialSwapLinks(path);

            Assert.Equal([("000900:Mod.esm", (string?)null, (float?)0.5f)], baseSwaps);
        }

        [Fact]
        public void ScanMaterialSwapLinks_SwapWithoutColorIndex_HasNullColorIndex()
        {
            string path = Write(Plugin().Group("STAT", g => g.Record("STAT", SelfIndex | 0x900, Mods(SelfIndex | 0x123))));

            var (_, baseSwaps) = CellProcessor.ScanMaterialSwapLinks(path);

            Assert.Equal([("000900:Mod.esm", (string?)"000123:Mod.esm", (float?)null)], baseSwaps);
        }

        [Fact]
        public void ScanMaterialSwapLinks_BaseWithoutSwapData_IsNotReported()
        {
            string path = Write(Plugin().Group("STAT", g => g.Record("STAT", SelfIndex | 0x900, Edid("Plain"))));

            var (refrSwaps, baseSwaps) = CellProcessor.ScanMaterialSwapLinks(path);

            Assert.Empty(refrSwaps);
            Assert.Empty(baseSwaps);
        }

        [Fact]
        public void ScanMaterialSwapLinks_MasterIndexResolvesToMasterAndSelfIndexToThePlugin()
        {
            string path = Write(Plugin().Group("STAT", g =>
                g.Record("STAT", 0x00ABCDEF, Mods(0x00001234))
                 .Record("STAT", SelfIndex | 0x800, Mods(SelfIndex | 0x777))));

            var (_, baseSwaps) = CellProcessor.ScanMaterialSwapLinks(path);

            Assert.Equal(
                [("ABCDEF:Fallout4.esm", (string?)"001234:Fallout4.esm", (float?)null), ("000800:Mod.esm", "000777:Mod.esm", null)],
                baseSwaps);
        }

        [Fact]
        public void ScanMaterialSwapLinks_PluginIndexBeyondTheMasterTable_ResolvesToThePlugin()
        {
            string path = Write(Plugin().Group("STAT", g => g.Record("STAT", 0x05000900, Mods(0x00000123))));

            var (_, baseSwaps) = CellProcessor.ScanMaterialSwapLinks(path);

            Assert.Equal([("000900:Mod.esm", (string?)"000123:Fallout4.esm", (float?)null)], baseSwaps);
        }

        [Theory]
        [InlineData("REFR")]
        [InlineData("ACHR")]
        [InlineData("PGRE")]
        public void ScanMaterialSwapLinks_PlacedRefWithXmsp_IsMappedToItsSwap(string refType)
        {
            string path = Write(Plugin().Group("CELL", g => g.Record(refType, SelfIndex | 0xA00, Xmsp(SelfIndex | 0x321))));

            var (refrSwaps, baseSwaps) = CellProcessor.ScanMaterialSwapLinks(path);

            Assert.Equal("000321:Mod.esm", refrSwaps["000A00:Mod.esm"]);
            Assert.Empty(baseSwaps);
        }

        [Fact]
        public void ScanMaterialSwapLinks_RecordTypesWithoutSwapSupport_AreIgnored()
        {
            string path = Write(Plugin().Group("NPC_", g => g.Record("NPC_", SelfIndex | 0x900, Mods(SelfIndex | 0x123), Modc(1f))));

            var (refrSwaps, baseSwaps) = CellProcessor.ScanMaterialSwapLinks(path);

            Assert.Empty(refrSwaps);
            Assert.Empty(baseSwaps);
        }

        [Fact]
        public void ScanMaterialSwapLinks_RecordsInsideNestedGroups_AreFound()
        {
            string path = Write(Plugin().Group("CELL", outer => outer.Group("CELL", inner =>
                inner.Record("REFR", SelfIndex | 0xA00, Xmsp(SelfIndex | 0x321)))));

            var (refrSwaps, _) = CellProcessor.ScanMaterialSwapLinks(path);

            Assert.Single(refrSwaps);
        }

        [Fact]
        public void ScanMaterialSwapLinks_CompressedRecord_IsInflatedBeforeScanning()
        {
            string path = Write(Plugin().Group("STAT", g => g.CompressedRecord("STAT", SelfIndex | 0x900, Edid("Zipped"), Mods(SelfIndex | 0x123), Modc(2f))));

            var (_, baseSwaps) = CellProcessor.ScanMaterialSwapLinks(path);

            Assert.Equal([("000900:Mod.esm", (string?)"000123:Mod.esm", (float?)2f)], baseSwaps);
        }

        [Fact]
        public void ScanMaterialSwapLinks_ExtendedSizeSubrecordBeforeTheSwap_IsSkippedByItsXxxxLength()
        {
            byte[] hugeBody = new byte[70000];
            byte[] extendedHeader = RawEsmBuilder.Concat(
                RawEsmBuilder.Subrecord("XXXX", BitConverter.GetBytes(hugeBody.Length)),
                System.Text.Encoding.ASCII.GetBytes("DATA"), [0, 0], hugeBody);
            string path = Write(Plugin().Group("STAT", g => g.Record("STAT", SelfIndex | 0x900, extendedHeader, Mods(SelfIndex | 0x123))));

            var (_, baseSwaps) = CellProcessor.ScanMaterialSwapLinks(path);

            Assert.Equal([("000900:Mod.esm", (string?)"000123:Mod.esm", (float?)null)], baseSwaps);
        }

        [Fact]
        public void ScanMaterialSwapLinks_ModsOfTheWrongSize_IsNotTreatedAsAFormId()
        {
            byte[] skyrimStyleList = RawEsmBuilder.Subrecord("MODS", new byte[12]);
            string path = Write(Plugin().Group("STAT", g => g.Record("STAT", SelfIndex | 0x900, skyrimStyleList)));

            var (_, baseSwaps) = CellProcessor.ScanMaterialSwapLinks(path);

            Assert.Empty(baseSwaps);
        }

        [Fact]
        public void ScanMaterialSwapLinks_FileShorterThanOneRecordHeader_ReturnsEmpty()
        {
            string path = Path.Combine(_directory.Path, Self);
            File.WriteAllBytes(path, new byte[10]);

            var (refrSwaps, baseSwaps) = CellProcessor.ScanMaterialSwapLinks(path);

            Assert.Empty(refrSwaps);
            Assert.Empty(baseSwaps);
        }

        [Fact]
        public void ScanMaterialSwapLinks_TruncatedFile_KeepsRecordsBeforeTheCut()
        {
            var builder = Plugin().Group("STAT", g =>
                g.Record("STAT", SelfIndex | 0x900, Mods(SelfIndex | 0x123))
                 .Record("STAT", SelfIndex | 0x901, Mods(SelfIndex | 0x124)));
            byte[] bytes = builder.ToBytes();
            string path = Path.Combine(_directory.Path, Self);
            File.WriteAllBytes(path, bytes[..^10]);

            var (_, baseSwaps) = CellProcessor.ScanMaterialSwapLinks(path);

            Assert.Contains(baseSwaps, b => b.FormKey == "000900:Mod.esm");
        }

        // ---- XCRI precombined references ---------------------------------------------------------------

        static byte[] Xcri(uint[] meshes, uint[] refs) => RawEsmBuilder.Subrecord("XCRI", RawEsmBuilder.Concat(
            BitConverter.GetBytes(meshes.Length), BitConverter.GetBytes(refs.Length),
            meshes.SelectMany(BitConverter.GetBytes).ToArray(), refs.SelectMany(BitConverter.GetBytes).ToArray()));

        [Fact]
        public void ScanXcri_CellWithCombinedRefs_ReturnsOnlyTheRefFormKeys()
        {
            string path = Write(Plugin().Group("CELL", g => g.Record("CELL", SelfIndex | 0x700,
                Xcri([0x11, 0x22], [SelfIndex | 0x3B55E, 0x00001234]))));

            var keys = CellProcessor.ScanXcriFromEsm(path);

            Assert.Equal(new HashSet<string> { "03B55E:Mod.esm", "001234:Fallout4.esm" }, keys);
        }

        [Fact]
        public void ScanXcri_CellWithoutXcri_ReturnsEmpty()
        {
            string path = Write(Plugin().Group("CELL", g => g.Record("CELL", SelfIndex | 0x700, Edid("NoCombined"))));

            Assert.Empty(CellProcessor.ScanXcriFromEsm(path));
        }

        [Fact]
        public void ScanXcri_CompressedCell_IsInflatedBeforeScanning()
        {
            string path = Write(Plugin().Group("CELL", g => g.CompressedRecord("CELL", SelfIndex | 0x700,
                Xcri([], [SelfIndex | 0x55]))));

            Assert.Equal(new HashSet<string> { "000055:Mod.esm" }, CellProcessor.ScanXcriFromEsm(path));
        }

        [Fact]
        public void ScanXcri_CorruptCompressedCell_IsSkippedAndLaterCellsAreStillScanned()
        {
            string path = Write(Plugin().Group("CELL", g => g
                .CorruptCompressedRecord("CELL", SelfIndex | 0x700)
                .Record("CELL", SelfIndex | 0x701, Xcri([], [SelfIndex | 0x66]))));

            Assert.Equal(new HashSet<string> { "000066:Mod.esm" }, CellProcessor.ScanXcriFromEsm(path));
        }

        [Fact]
        public void ScanMaterialSwapLinks_CorruptCompressedBase_IsSkippedAndLaterRecordsAreStillScanned()
        {
            string path = Write(Plugin().Group("STAT", g => g
                .CorruptCompressedRecord("STAT", SelfIndex | 0x900)
                .Record("STAT", SelfIndex | 0x901, Mods(SelfIndex | 0x123))));

            var (_, baseSwaps) = CellProcessor.ScanMaterialSwapLinks(path);

            Assert.Equal([("000901:Mod.esm", "000123:Mod.esm", (float?)null)], baseSwaps);
        }

        [Fact]
        public void ScanXcri_CountsLargerThanTheSubrecord_AreIgnoredInsteadOfReadingPastIt()
        {
            byte[] lying = RawEsmBuilder.Subrecord("XCRI", RawEsmBuilder.Concat(
                BitConverter.GetBytes(0), BitConverter.GetBytes(1000), BitConverter.GetBytes(0x55u)));
            string path = Write(Plugin().Group("CELL", g => g.Record("CELL", SelfIndex | 0x700, lying)));

            Assert.Empty(CellProcessor.ScanXcriFromEsm(path));
        }

        [Fact]
        public void ScanXcri_RefsFromSeveralCellsAcrossGroups_AreUnioned()
        {
            string path = Write(Plugin().Group("CELL", outer => outer
                .Record("CELL", SelfIndex | 0x700, Xcri([], [SelfIndex | 0x1]))
                .Group("CELL", inner => inner.Record("CELL", SelfIndex | 0x701, Xcri([], [SelfIndex | 0x2])))));

            Assert.Equal(new HashSet<string> { "000001:Mod.esm", "000002:Mod.esm" }, CellProcessor.ScanXcriFromEsm(path));
        }

        [Fact]
        public void ScanXcri_FileShorterThanOneRecordHeader_ReturnsEmpty()
        {
            string path = Path.Combine(_directory.Path, Self);
            File.WriteAllBytes(path, new byte[5]);

            Assert.Empty(CellProcessor.ScanXcriFromEsm(path));
        }
    }
}
