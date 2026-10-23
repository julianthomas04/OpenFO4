using AssetConverter;
using Xunit;

namespace InstallerTests
{
    // Greyscale-to-palette bake (palette_baker.cs): every diffuse pixel's luminance indexes along the X axis of the LUT
    // row selected by the row float; the diffuse alpha survives; the result is an uncompressed BGRA DDS with a full mip chain.
    // The tests use texture names no BGSM census can contain, so the baked file name is always the ".pal-xxxxxxxx" variant.
    [Collection(StaticStateCollections.NifConverterState)]
    public sealed class PaletteBakerTests : IDisposable
    {
        const string Diffuse = "palette_test/grey.dds";
        const string Lut = "palette_test/lut.dds";

        readonly TempDirectory _directory = new();
        readonly string _staging;
        readonly string _texturesOut;

        public PaletteBakerTests()
        {
            _staging = Path.Combine(_directory.Path, "staging");
            _texturesOut = Path.Combine(_directory.Path, "out");
            Directory.CreateDirectory(Path.Combine(_staging, "textures", "palette_test"));
            Directory.CreateDirectory(_texturesOut);
        }

        public void Dispose() => _directory.Dispose();

        static byte[] Bgra(params (byte B, byte G, byte R, byte A)[] pixels) =>
            pixels.SelectMany(p => new[] { p.B, p.G, p.R, p.A }).ToArray();

        // 4-wide, 2-row LUT. Row 0 = pure primaries + white, row 1 = darker secondaries.
        static byte[] TwoRowLut() => SyntheticDds.Bgra32(4, 2, Bgra(
            (0, 0, 255, 255), (0, 255, 0, 255), (255, 0, 0, 255), (255, 255, 255, 255),
            (0, 0, 100, 255), (0, 100, 0, 255), (100, 0, 0, 255), (100, 100, 100, 255)));

        void WriteSource(string relative, byte[] dds)
        {
            string path = Path.Combine(_staging, "textures", relative);
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllBytes(path, dds);
        }

        string BakedFile()
        {
            var files = Directory.GetFiles(_texturesOut, "grey.pal-*.dds", SearchOption.AllDirectories);
            return Assert.Single(files);
        }

        static byte[] TopMip(byte[] dds, int width, int height) => dds[SyntheticDds.HeaderSize..(SyntheticDds.HeaderSize + width * height * 4)];

        [Fact]
        public void BakeCombo_GreyLevels_MapToLutColumnsOfTheSelectedRow()
        {
            WriteSource(Diffuse, SyntheticDds.Bgra32(2, 2, Bgra((0, 0, 0, 255), (64, 64, 64, 255), (128, 128, 128, 255), (255, 255, 255, 255))));
            WriteSource(Lut, TwoRowLut());

            bool baked = PaletteBaker.BakeCombo(_staging, _texturesOut, Diffuse, Lut, 0f);

            Assert.True(baked);
            Assert.Equal(
                Bgra((0, 0, 255, 255), (0, 255, 0, 255), (255, 0, 0, 255), (255, 255, 255, 255)),
                TopMip(File.ReadAllBytes(BakedFile()), 2, 2));
        }

        [Theory]
        [InlineData(0.5f)]
        [InlineData(0.999f)]
        [InlineData(1.0f)]
        [InlineData(7f)]
        public void BakeCombo_RowAtOrPastTheMiddle_SelectsTheSecondBandAndClampsPastTheEnd(float row)
        {
            WriteSource(Diffuse, SyntheticDds.Bgra32(1, 1, Bgra((0, 0, 0, 255))));
            WriteSource(Lut, TwoRowLut());

            PaletteBaker.BakeCombo(_staging, _texturesOut, Diffuse, Lut, row);

            Assert.Equal(Bgra((0, 0, 100, 255)), TopMip(File.ReadAllBytes(BakedFile()), 1, 1));
        }

        [Fact]
        public void BakeCombo_NegativeRow_ClampsToTheFirstBand()
        {
            WriteSource(Diffuse, SyntheticDds.Bgra32(1, 1, Bgra((0, 0, 0, 255))));
            WriteSource(Lut, TwoRowLut());

            PaletteBaker.BakeCombo(_staging, _texturesOut, Diffuse, Lut, -3f);

            Assert.Equal(Bgra((0, 0, 255, 255)), TopMip(File.ReadAllBytes(BakedFile()), 1, 1));
        }

        // Luminance = (299 R + 587 G + 114 B) / 1000: red 76, green 149, blue 29 -> columns 1, 2, 0 of a 4-wide LUT.
        [Fact]
        public void BakeCombo_LuminanceUsesRec601Weights()
        {
            WriteSource(Diffuse, SyntheticDds.Bgra32(3, 1, Bgra((0, 0, 255, 255), (0, 255, 0, 255), (255, 0, 0, 255))));
            WriteSource(Lut, TwoRowLut());

            PaletteBaker.BakeCombo(_staging, _texturesOut, Diffuse, Lut, 0f);

            Assert.Equal(
                Bgra((0, 255, 0, 255), (255, 0, 0, 255), (0, 0, 255, 255)),
                TopMip(File.ReadAllBytes(BakedFile()), 3, 1));
        }

        [Fact]
        public void BakeCombo_KeepsTheDiffuseAlphaChannel()
        {
            WriteSource(Diffuse, SyntheticDds.Bgra32(2, 1, Bgra((0, 0, 0, 10), (255, 255, 255, 200))));
            WriteSource(Lut, TwoRowLut());

            PaletteBaker.BakeCombo(_staging, _texturesOut, Diffuse, Lut, 0f);

            Assert.Equal(Bgra((0, 0, 255, 10), (255, 255, 255, 200)), TopMip(File.ReadAllBytes(BakedFile()), 2, 1));
        }

        [Fact]
        public void BakeCombo_LutWithoutAlpha_IsReadAsOpaqueColours()
        {
            WriteSource(Diffuse, SyntheticDds.Bgra32(1, 1, Bgra((0, 0, 0, 255))));
            byte[] bgr = [0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255];
            WriteSource(Lut, SyntheticDds.Bgr24(4, 1, bgr));

            bool baked = PaletteBaker.BakeCombo(_staging, _texturesOut, Diffuse, Lut, 0f);

            Assert.True(baked);
            Assert.Equal(Bgra((0, 0, 255, 255)), TopMip(File.ReadAllBytes(BakedFile()), 1, 1));
        }

        [Fact]
        public void BakeCombo_WritesFullMipChainWithBoxFilteredLevels()
        {
            WriteSource(Diffuse, SyntheticDds.Bgra32(2, 2, Bgra((0, 0, 0, 255), (64, 64, 64, 255), (128, 128, 128, 255), (255, 255, 255, 255))));
            WriteSource(Lut, TwoRowLut());

            PaletteBaker.BakeCombo(_staging, _texturesOut, Diffuse, Lut, 0f);
            byte[] dds = File.ReadAllBytes(BakedFile());

            Assert.Equal(0x20534444u, BitConverter.ToUInt32(dds, 0));
            Assert.Equal(2u, BitConverter.ToUInt32(dds, 12));
            Assert.Equal(2u, BitConverter.ToUInt32(dds, 16));
            Assert.Equal(2u, BitConverter.ToUInt32(dds, 28));
            Assert.Equal(32u, BitConverter.ToUInt32(dds, 88));
            Assert.Equal(SyntheticDds.HeaderSize + 2 * 2 * 4 + 1 * 1 * 4, dds.Length);
            byte[] lastMip = dds[(SyntheticDds.HeaderSize + 16)..];
            Assert.Equal([(255 + 0 + 0 + 255) / 4, (0 + 255 + 0 + 255) / 4, (0 + 0 + 255 + 255) / 4, 255], lastMip.Select(b => (int)b));
        }

        [Fact]
        public void BakeCombo_NonSquareDiffuse_GetsMipsDownToOneByOne()
        {
            WriteSource(Diffuse, SyntheticDds.Bgra32(4, 1, Bgra((0, 0, 0, 255), (0, 0, 0, 255), (0, 0, 0, 255), (0, 0, 0, 255))));
            WriteSource(Lut, TwoRowLut());

            PaletteBaker.BakeCombo(_staging, _texturesOut, Diffuse, Lut, 0f);
            byte[] dds = File.ReadAllBytes(BakedFile());

            Assert.Equal(3u, BitConverter.ToUInt32(dds, 28));
            Assert.Equal(SyntheticDds.HeaderSize + (4 + 2 + 1) * 4, dds.Length);
        }

        [Fact]
        public void BakeCombo_MissingSourceTexture_ReturnsFalseAndWritesNothing()
        {
            WriteSource(Lut, TwoRowLut());

            bool baked = PaletteBaker.BakeCombo(_staging, _texturesOut, Diffuse, Lut, 0f);

            Assert.False(baked);
            Assert.Empty(Directory.GetFiles(_texturesOut, "*", SearchOption.AllDirectories));
        }

        [Fact]
        public void BakeCombo_CorruptSource_ReturnsFalseAndLeavesNoTemporaryFile()
        {
            WriteSource(Diffuse, [1, 2, 3, 4]);
            WriteSource(Lut, TwoRowLut());

            bool baked = PaletteBaker.BakeCombo(_staging, _texturesOut, Diffuse, Lut, 0f);

            Assert.False(baked);
            Assert.Empty(Directory.GetFiles(_texturesOut, "*", SearchOption.AllDirectories));
        }

        [Fact]
        public void BakeCombo_DestinationAlreadyBaked_IsLeftUntouched()
        {
            WriteSource(Diffuse, SyntheticDds.Bgra32(1, 1, Bgra((0, 0, 0, 255))));
            WriteSource(Lut, TwoRowLut());
            PaletteBaker.BakeCombo(_staging, _texturesOut, Diffuse, Lut, 0f);
            string baked = BakedFile();
            File.WriteAllText(baked, "sentinel");

            bool again = PaletteBaker.BakeCombo(_staging, _texturesOut, Diffuse, Lut, 0f);

            Assert.True(again);
            Assert.Equal("sentinel", File.ReadAllText(baked));
        }

        [Fact]
        public void BakeCombo_DifferentRows_WriteDifferentVariantFiles()
        {
            WriteSource(Diffuse, SyntheticDds.Bgra32(1, 1, Bgra((0, 0, 0, 255))));
            WriteSource(Lut, TwoRowLut());

            PaletteBaker.BakeCombo(_staging, _texturesOut, Diffuse, Lut, 0f);
            PaletteBaker.BakeCombo(_staging, _texturesOut, Diffuse, Lut, 0.5f);

            Assert.Equal(2, Directory.GetFiles(_texturesOut, "grey.pal-*.dds", SearchOption.AllDirectories).Length);
        }

        void WriteSourcePair()
        {
            WriteSource(Diffuse, SyntheticDds.Bgra32(2, 2, Bgra((0, 0, 0, 255), (64, 64, 64, 255), (128, 128, 128, 255), (255, 255, 255, 255))));
            WriteSource(Lut, TwoRowLut());
        }

        static (string Diffuse, string Lut, float Row)[] ManyRows(int count) =>
            Enumerable.Range(0, count).Select(i => (Diffuse, Lut, (i + 1) / (float)(count + 1))).ToArray();

        static Dictionary<string, byte[]> FilesByRelativePath(string root) =>
            Directory.GetFiles(root, "*", SearchOption.AllDirectories)
                .ToDictionary(file => Path.GetRelativePath(root, file), File.ReadAllBytes);

        [Fact]
        public void BakeCombos_ManyVariants_ProducesTheBytesOfSerialBakes()
        {
            WriteSourcePair();
            var combos = ManyRows(24);
            string serialOut = Path.Combine(_directory.Path, "serial");
            foreach (var (diffuse, lut, row) in combos)
                PaletteBaker.BakeCombo(_staging, serialOut, diffuse, lut, row);

            int baked = PaletteBaker.BakeCombos(_staging, _texturesOut, combos, TestContext.Current.CancellationToken);

            Assert.Equal(combos.Length, baked);
            var expected = FilesByRelativePath(serialOut);
            Assert.Equal(combos.Length, expected.Count);
            var actual = FilesByRelativePath(_texturesOut);
            Assert.Equal(expected.Keys.Order(), actual.Keys.Order());
            Assert.All(expected, file => Assert.Equal(file.Value, actual[file.Key]));
        }

        [Fact]
        public void BakeCombos_CombosThatQuantizeToTheSameDestination_WriteItOnceAndAllCountAsBaked()
        {
            WriteSourcePair();

            int baked = PaletteBaker.BakeCombos(_staging, _texturesOut, [(Diffuse, Lut, 0.5001f), (Diffuse, Lut, 0.4999f), (Diffuse, Lut, 0.5f)], TestContext.Current.CancellationToken);

            Assert.Equal(3, baked);
            Assert.Single(Directory.GetFiles(_texturesOut, "*", SearchOption.AllDirectories));
        }

        [Fact]
        public void BakeCombos_ComboWithMissingSource_IsNotCountedAndLeavesTheOthersBaked()
        {
            WriteSourcePair();
            var combos = ManyRows(6).Append(("palette_test/absent.dds", Lut, 0.5f)).ToArray();

            int baked = PaletteBaker.BakeCombos(_staging, _texturesOut, combos, TestContext.Current.CancellationToken);

            Assert.Equal(6, baked);
            Assert.Equal(6, Directory.GetFiles(_texturesOut, "*", SearchOption.AllDirectories).Length);
        }

        [Fact]
        public void BakeCombos_AlreadyCancelled_ThrowsAndWritesNothing()
        {
            WriteSourcePair();
            using var cts = new CancellationTokenSource();
            cts.Cancel();

            Assert.ThrowsAny<OperationCanceledException>(() => PaletteBaker.BakeCombos(_staging, _texturesOut, ManyRows(4), cts.Token));

            Assert.Empty(Directory.GetFiles(_texturesOut, "*", SearchOption.AllDirectories));
        }

        [Fact]
        public void RunDeferredLateBakes_BakesQueuedCombosReportsTotalAndDrainsTheQueue()
        {
            WriteSource(Diffuse, SyntheticDds.Bgra32(1, 1, Bgra((0, 0, 0, 255))));
            WriteSource(Lut, TwoRowLut());
            PaletteBaker.PendingLateBakes.Enqueue((Diffuse, Lut, 0f));
            PaletteBaker.PendingLateBakes.Enqueue(("palette_test/absent.dds", Lut, 0f));

            int total = PaletteBaker.RunDeferredLateBakes(_staging, _texturesOut);

            Assert.Equal(2, total);
            Assert.Empty(PaletteBaker.PendingLateBakes);
            Assert.Single(Directory.GetFiles(_texturesOut, "grey.pal-*.dds", SearchOption.AllDirectories));
        }
    }
}
