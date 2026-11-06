using AssetConverter;
using AssetConverter.Pipeline;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests.Pipeline
{
    /// <summary>Drives the NIF converter's process-wide texture index, so it runs serially with the other tests that do.</summary>
    [Collection(StaticStateCollections.NifConverterState)]
    public sealed class PipelineStagesTextureNamesTests : IDisposable
    {
        private const int TextureArchiveCount = 10;

        private readonly TempDirectory _root = new();

        public void Dispose()
        {
            NifConverter.PreloadTextureIndex([]);
            _root.Dispose();
        }

        private InstallLocations Locations => new(Path.Combine(_root.Path, "game"), Path.Combine(_root.Path, "install"));

        private static string ArchiveName(int index) => index == TextureArchiveCount ? "Fallout4 - TexturesPatch.ba2" : $"Fallout4 - Textures{index}.ba2";

        /// <summary>Every texture archive of the game, each holding one texture of its own, so opening them prints no warning for a missing one.</summary>
        private void CreateGameTextureArchives()
        {
            string data = Path.Combine(Locations.Fo4Location, "Data");
            Directory.CreateDirectory(data);
            for (int index = 1; index <= TextureArchiveCount; index++)
            {
                var texture = new Ba2File($@"textures\set{index}\tex.dds", TestBytes.Random(64, seed: index), Compress: false);
                File.WriteAllBytes(Path.Combine(data, ArchiveName(index)), Ba2Builder.Gnrl([texture]));
            }
        }

        [Fact]
        public void TheNameIndexListsEveryArchiveEvenOnesAlreadyExtractedAndNothingThatIsDeployed()
        {
            CreateGameTextureArchives();
            Directory.CreateDirectory(Locations.StagingRoot);
            File.WriteAllText(Path.Combine(Locations.StagingRoot, $".{ArchiveName(1)}.extracted-direct"), "");
            string baked = Path.Combine(Locations.TexturesOut, "baked", "palette_d.dds");
            Directory.CreateDirectory(Path.GetDirectoryName(baked)!);
            File.WriteAllBytes(baked, [0]);
            var stages = new PipelineStages(Locations, MemoryBudget.Unlimited(), TestContext.Current.CancellationToken);

            stages.IndexTextureNames();

            Assert.True(NifConverter.TextureIndexContains("set1/tex.dds"));
            Assert.True(NifConverter.TextureIndexContains("set10/tex.dds"));
            Assert.False(NifConverter.TextureIndexContains("baked/palette_d.dds"));
            Assert.Equal([baked], Directory.EnumerateFiles(Locations.TexturesOut, "*.dds", SearchOption.AllDirectories));
        }
    }
}
