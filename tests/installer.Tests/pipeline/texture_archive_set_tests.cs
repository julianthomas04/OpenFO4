using AssetConverter.Pipeline;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests.Pipeline
{
    /// <summary>Opening an archive prints a warning for a missing one, so this runs serially with the tests that redirect the console.</summary>
    [Collection("Console redirection")]
    public sealed class TextureArchiveSetTests : IDisposable
    {
        private static readonly Ba2File Diffuse = new(@"textures\wall\brick_d.dds", TestBytes.Random(400, seed: 1), Compress: false);
        private static readonly Ba2File Normal = new(@"textures\wall\brick_n.dds", TestBytes.Random(400, seed: 2), Compress: false);

        private readonly ArchiveTempDir _game = new();
        private readonly ArchiveTempDir _staging = new();

        public void Dispose()
        {
            _game.Dispose();
            _staging.Dispose();
        }

        private string WriteArchive(string name, params Ba2File[] files)
        {
            _game.Write(name, Ba2Builder.Gnrl(files));
            return Path.Combine(_game.Path, name);
        }

        private TextureArchiveSet Open(params string[] names) => TextureArchiveSet.Open(_game.Path, _staging.Path, names);

        private static void DeleteArchive(string archivePath) => File.Delete(archivePath);

        private static string Native(string archivePath) => archivePath.Replace('\\', Path.DirectorySeparatorChar);

        [Fact]
        public void OnlyArchivesThatExistAndLackADoneMarkerArePending()
        {
            WriteArchive("a.ba2", Diffuse);
            WriteArchive("done.ba2", Normal);
            File.WriteAllText(Path.Combine(_staging.Path, ".done.ba2.extracted-direct"), "");

            using var set = Open("a.ba2", "done.ba2", "missing.ba2");

            Assert.Equal(["a.ba2"], set.Pending.Select(entry => entry.Name));
        }

        [Fact]
        public void MarkingAnArchiveExtractedMakesTheNextRunSkipIt()
        {
            WriteArchive("a.ba2", Diffuse);
            using (var first = Open("a.ba2"))
                first.MarkExtracted(Assert.Single(first.Pending));

            using var second = Open("a.ba2");

            Assert.Empty(second.Pending);
        }

        [Fact]
        public void DisposingReleasesEveryArchiveHandleSoTheFilesCanBeDeleted()
        {
            string a = WriteArchive("a.ba2", Diffuse);
            string b = WriteArchive("b.ba2", Normal);
            var set = Open("a.ba2", "b.ba2");
            Assert.ThrowsAny<IOException>(() => DeleteArchive(a));

            set.Dispose();

            DeleteArchive(a);
            DeleteArchive(b);
            Assert.False(File.Exists(a));
        }

        [Fact]
        public void AnArchiveThatCannotBeOpenedReleasesTheOnesOpenedBeforeIt()
        {
            string good = WriteArchive("a.ba2", Diffuse);
            _game.Write("broken.ba2", [1, 2, 3]);

            Assert.ThrowsAny<Exception>(() => Open("a.ba2", "broken.ba2"));

            DeleteArchive(good);
            Assert.False(File.Exists(good));
        }

        [Fact]
        public void TheIndexListsThePendingArchivesEntriesThenTheFilesAlreadyDeployed()
        {
            WriteArchive("a.ba2", Diffuse, Normal);
            string texturesOut = Path.Combine(_staging.Path, "textures");
            Directory.CreateDirectory(Path.Combine(texturesOut, "sky"));
            File.WriteAllBytes(Path.Combine(texturesOut, "sky", "clouds.dds"), [0]);
            File.WriteAllBytes(Path.Combine(texturesOut, "sky", "readme.txt"), [0]);
            using var set = Open("a.ba2");

            var names = set.IndexNames(texturesOut).ToList();

            Assert.Equal([Native(Diffuse.Name), Native(Normal.Name), Path.Combine("sky", "clouds.dds")], names);
        }

        [Fact]
        public void TheIndexOfAnInstallWithoutDeployedTexturesHoldsOnlyTheArchiveEntries()
        {
            WriteArchive("a.ba2", Diffuse);
            using var set = Open("a.ba2");

            Assert.Equal([Native(Diffuse.Name)], set.IndexNames(Path.Combine(_staging.Path, "absent")));
        }
    }
}
