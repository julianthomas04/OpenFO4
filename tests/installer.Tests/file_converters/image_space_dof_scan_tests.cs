using AssetConverter;
using Xunit;

namespace InstallerTests
{
    // IMGS DNAM: float strength, distance, range, mode and (24-byte form) vignette radius and strength:
    // A malformed DNAM is tallied in the shared
    // ReadFailureTally, so these run in the collection that owns it.
    [Collection(FixtureEsm.Name)]
    public sealed class ImageSpaceDofScanTests : IDisposable
    {
        const string Self = "Mod.esm";
        const uint SelfIndex = 0x01000000;
        const string FailureSite = "IMGS DNAM depth of field";

        readonly TempDirectory _directory = new();

        public void Dispose() => _directory.Dispose();

        [Fact]
        public void Scan_UnreadablePlugin_ReturnsNothingInsteadOfFailingTheInstall()
        {
            Assert.Empty(ImageSpaceDofScan.Scan(Path.Combine(_directory.Path, "missing.esm")));
        }

        static byte[] Dnam(params float[] floats) =>
            RawEsmBuilder.Subrecord("DNAM", floats.SelectMany(BitConverter.GetBytes).ToArray());

        Dictionary<string, ImageSpaceDofBlock> Scan(RawEsmBuilder builder) =>
            ImageSpaceDofScan.Scan(builder.WriteTo(_directory.Path, Self));

        static RawEsmBuilder Plugin() => RawEsmBuilder.Plugin("Fallout4.esm");

        [Fact]
        public void Scan_TwentyFourByteDnam_ReadsModeAndVignette()
        {
            var plugin = Plugin().Group("IMGS", g => g.Record("IMGS", SelfIndex | 0x10, Dnam(1f, -2500f, 3500f, 26f, 40f, 0.25f)));

            var block = Scan(plugin)["000010:Mod.esm"];

            Assert.Equal(new ImageSpaceDofBlock(26f, 40f, 0.25f), block);
        }

        [Fact]
        public void Scan_SixteenByteDnam_HasNoVignette()
        {
            var plugin = Plugin().Group("IMGS", g => g.Record("IMGS", SelfIndex | 0x20, Dnam(0.7f, 3000f, 5048f, 2f)));

            Assert.Equal(new ImageSpaceDofBlock(2f, 0f, 0f), Scan(plugin)["000020:Mod.esm"]);
        }

        // Mutagen loses the top byte of the mode float; 62.0f is 0x42780000, so its top byte is set.
        [Fact]
        public void Scan_ModeWithATopByte_IsKeptExactly()
        {
            var plugin = Plugin().Group("IMGS", g => g.Record("IMGS", SelfIndex | 0x30, Dnam(1f, 500f, 3500f, 62f)));

            Assert.Equal(62f, Scan(plugin)["000030:Mod.esm"].Mode);
        }

        [Fact]
        public void Scan_MasterFormId_ResolvesToTheMasterPlugin()
        {
            var plugin = Plugin().Group("IMGS", g => g.Record("IMGS", 0x00000040, Dnam(1f, 0f, 1f, 6f)));

            Assert.Contains("000040:Fallout4.esm", Scan(plugin).Keys);
        }

        [Fact]
        public void Scan_RecordWithoutDnam_IsAbsent()
        {
            var plugin = Plugin().Group("IMGS", g => g.Record("IMGS", SelfIndex | 0x50));

            Assert.Empty(Scan(plugin));
        }

        [Fact]
        public void Scan_OtherRecordTypesWithADnam_AreIgnored()
        {
            var plugin = Plugin().Group("STAT", g => g.Record("STAT", SelfIndex | 0x60, Dnam(1f, 1f, 1f, 1f)));

            Assert.Empty(Scan(plugin));
        }

        [Fact]
        public void Scan_CompressedRecord_IsInflatedFirst()
        {
            var plugin = Plugin().Group("IMGS", g => g.CompressedRecord("IMGS", SelfIndex | 0x70, Dnam(1f, 0f, 1f, 18f, 5f, 0.5f)));

            Assert.Equal(new ImageSpaceDofBlock(18f, 5f, 0.5f), Scan(plugin)["000070:Mod.esm"]);
        }

        [Theory]
        [InlineData(12)]
        [InlineData(20)]
        public void Scan_DnamOfAnUnexpectedSize_IsTalliedAndLeftOut(int bytes)
        {
            int before = ReadFailureTally.Shared.CountFor(FailureSite);
            var plugin = Plugin().Group("IMGS", g => g.Record("IMGS", SelfIndex | 0x80, RawEsmBuilder.Subrecord("DNAM", new byte[bytes])));

            Assert.Empty(Scan(plugin));
            Assert.Equal(before + 1, ReadFailureTally.Shared.CountFor(FailureSite));
        }
    }
}
