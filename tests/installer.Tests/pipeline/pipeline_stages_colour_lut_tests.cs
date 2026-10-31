using AssetConverter;
using AssetConverter.Pipeline;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests.Pipeline
{
    public sealed class PipelineStagesColourLutTests : IDisposable
    {
        private static readonly byte[] DayLut = [1, 2, 3];
        private static readonly byte[] NightLut = [4, 5, 6];

        private readonly InstallerTests.TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private InstallLocations Locations => new(Path.Combine(_root.Path, "game"), Path.Combine(_root.Path, "install"), Path.Combine(_root.Path, "settings"));

        private PipelineStages Stages => new(Locations, MemoryBudget.Unlimited(), TestContext.Current.CancellationToken);

        private void CreateMiscArchive(params Ba2File[] files)
        {
            Directory.CreateDirectory(Locations.GameDataDirectory);
            File.WriteAllBytes(Path.Combine(Locations.GameDataDirectory, ColourLuts.ArchiveName), Ba2Builder.Gnrl(files));
        }

        [Fact]
        public void ExtractColourLuts_WritesEveryLutBelowTheTexturesFolderAndNothingElse()
        {
            CreateMiscArchive(
                new Ba2File(@"Textures\Effects\LUTs\LUT_CW_ClearDAY.DDS", DayLut, Compress: true),
                new Ba2File(@"Textures\Effects\LUTs\LUT_CW_ClearNIGHT.dds", NightLut, Compress: false),
                new Ba2File(@"Textures\Effects\Gradients\Ramp.dds", [7], Compress: false),
                new Ba2File(@"Textures\Effects\LUTs\Readme.txt", [8], Compress: false),
                new Ba2File(@"Misc\Other.dds", [9], Compress: false));

            Stages.ExtractColourLuts();

            string luts = Path.Combine(Locations.TexturesOut, "Effects", "LUTs");
            Assert.Equal(DayLut, File.ReadAllBytes(Path.Combine(luts, "LUT_CW_ClearDAY.DDS")));
            Assert.Equal(NightLut, File.ReadAllBytes(Path.Combine(luts, "LUT_CW_ClearNIGHT.dds")));
            Assert.Equal(2, Directory.EnumerateFiles(Locations.TexturesOut, "*", SearchOption.AllDirectories).Count());
        }

        [Fact]
        public void ExtractColourLuts_ArchiveMissing_ThrowsNamingIt()
        {
            Directory.CreateDirectory(Locations.GameDataDirectory);

            var error = Assert.Throws<FileNotFoundException>(() => Stages.ExtractColourLuts());

            Assert.Contains(ColourLuts.ArchiveName, error.Message);
        }

        [Fact]
        public void ExtractColourLuts_ArchiveWithoutLuts_Throws()
        {
            CreateMiscArchive(new Ba2File(@"Textures\Effects\Gradients\Ramp.dds", [7], Compress: false));

            var error = Assert.Throws<InvalidOperationException>(() => Stages.ExtractColourLuts());

            Assert.Contains(ColourLuts.ArchiveFolder, error.Message);
        }
    }
}
