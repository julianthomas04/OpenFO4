using AssetConverter;
using Xunit;

namespace InstallerTests
{
    // XWCN (u32 count) + XWCU (count x 16 bytes) of CELL and REFR.
    // Malformed blocks are tallied in the shared ReadFailureTally, so these run in the collection that owns it.
    [Collection(FixtureEsm.Name)]
    public sealed class WaterCurrentScanTests : IDisposable
    {
        const string Self = "Mod.esm";
        const uint SelfIndex = 0x01000000;
        const string FailureSite = "XWCU water currents";

        readonly TempDirectory _directory = new();

        public void Dispose() => _directory.Dispose();

        [Fact]
        public void Scan_UnreadablePlugin_ReturnsNothingInsteadOfFailingTheInstall()
        {
            Assert.Empty(WaterCurrentScan.Scan(Path.Combine(_directory.Path, "missing.esm")));
        }

        static byte[] Count(uint count) => RawEsmBuilder.Subrecord("XWCN", BitConverter.GetBytes(count));

        static byte[] Entries(params float[][] entries) =>
            RawEsmBuilder.Subrecord("XWCU", entries.SelectMany(e => e.SelectMany(BitConverter.GetBytes)).ToArray());

        Dictionary<string, string> Scan(RawEsmBuilder builder) => WaterCurrentScan.Scan(builder.WriteTo(_directory.Path, Self));

        static RawEsmBuilder Plugin() => RawEsmBuilder.Plugin("Fallout4.esm");

        [Theory]
        [InlineData(0)]
        [InlineData(1)]
        [InlineData(2)]
        [InlineData(3)]
        [InlineData(17)]
        public void Scan_CellWithNEntries_DecodesExactlyNEntries(int n)
        {
            var entries = Enumerable.Range(0, n).Select(i => new float[] { i, i + 0.5f, -i, i * 10f }).ToArray();
            var plugin = Plugin().Group("CELL", g => g.Record("CELL", SelfIndex | 0x10, Count((uint)n), Entries(entries)));

            var result = Scan(plugin);

            Assert.Equal(entries, Decoded(result["000010:Mod.esm"]));
        }

        [Fact]
        public void Scan_PlacedRefInNestedGroup_IsFoundUnderItsFormKey()
        {
            var plugin = Plugin().Group("CELL", g => g.Group("CELL", inner =>
                inner.Record("REFR", SelfIndex | 0x20, Count(1), Entries([1f, 2f, 3f, 4f]))));

            var result = Scan(plugin);

            Assert.Equal([[1f, 2f, 3f, 4f]], Decoded(result["000020:Mod.esm"]));
        }

        [Fact]
        public void Scan_MasterFormId_ResolvesToTheMasterPlugin()
        {
            var plugin = Plugin().Group("CELL", g => g.Record("CELL", 0x00000030, Count(1), Entries([0f, 0f, 1f, 2f])));

            Assert.Contains("000030:Fallout4.esm", Scan(plugin).Keys);
        }

        [Fact]
        public void Scan_RecordWithoutCurrentBlock_IsAbsent()
        {
            var plugin = Plugin().Group("CELL", g => g.Record("CELL", SelfIndex | 0x40));

            Assert.Empty(Scan(plugin));
        }

        [Fact]
        public void Scan_OtherRecordTypesWithTheSameSubrecords_AreIgnored()
        {
            var plugin = Plugin().Group("STAT", g => g.Record("STAT", SelfIndex | 0x50, Count(1), Entries([1f, 1f, 1f, 1f])));

            Assert.Empty(Scan(plugin));
        }

        [Fact]
        public void Scan_CompressedRecord_IsInflatedFirst()
        {
            var plugin = Plugin().Group("CELL", g => g.CompressedRecord("CELL", SelfIndex | 0x60, Count(1), Entries([9f, 8f, 7f, 6f])));

            Assert.Equal([[9f, 8f, 7f, 6f]], Decoded(Scan(plugin)["000060:Mod.esm"]));
        }

        [Fact]
        public void Scan_CountLargerThanTheEntryArray_IsTalliedAndLeftOut()
        {
            int before = ReadFailureTally.Shared.CountFor(FailureSite);
            var plugin = Plugin().Group("CELL", g => g.Record("CELL", SelfIndex | 0x70, Count(3), Entries([1f, 2f, 3f, 4f])));

            Assert.Empty(Scan(plugin));
            Assert.Equal(before + 1, ReadFailureTally.Shared.CountFor(FailureSite));
        }

        [Fact]
        public void Scan_EntryArrayWithoutCount_IsTalliedAndLeftOut()
        {
            int before = ReadFailureTally.Shared.CountFor(FailureSite);
            var plugin = Plugin().Group("CELL", g => g.Record("CELL", SelfIndex | 0x80, Entries([1f, 2f, 3f, 4f])));

            Assert.Empty(Scan(plugin));
            Assert.Equal(before + 1, ReadFailureTally.Shared.CountFor(FailureSite));
        }

        [Fact]
        public void Scan_EntryArrayWithAFractionalEntry_IsTalliedAndLeftOut()
        {
            int before = ReadFailureTally.Shared.CountFor(FailureSite);
            var plugin = Plugin().Group("CELL", g => g.Record("CELL", SelfIndex | 0x90, Count(1), RawEsmBuilder.Subrecord("XWCU", new byte[18])));

            Assert.Empty(Scan(plugin));
            Assert.Equal(before + 1, ReadFailureTally.Shared.CountFor(FailureSite));
        }

        static float[][] Decoded(string json) =>
            System.Text.Json.JsonDocument.Parse(json).RootElement.EnumerateArray()
                .Select(entry => entry.EnumerateArray().Select(v => v.GetSingle()).ToArray()).ToArray();
    }
}
