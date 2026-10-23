using AssetConverter;
using Xunit;

namespace InstallerTests.Archives
{
    public class BA2UnpackerTests
    {
        private static readonly Ba2File Mesh = new(@"meshes\a\one.nif", TestBytes.Random(500, 31), Compress: true);

        [Fact]
        public void Unpack_ValidArchive_WritesEntries()
        {
            using var dir = new ArchiveTempDir();
            string archive = dir.Write("a.ba2", Ba2Builder.Gnrl([Mesh]));
            string output = Path.Combine(dir.Path, "out");

            BA2Unpacker.Unpack(archive, output);

            Assert.Equal(Mesh.Content, File.ReadAllBytes(Path.Combine(output, "meshes", "a", "one.nif")));
        }

        [Fact]
        public void Unpack_GarbageArchive_ThrowsNamingTheArchiveAndCause()
        {
            using var dir = new ArchiveTempDir();
            string archive = dir.Write("bad.ba2", TestBytes.Random(64, 1));

            var ex = Assert.Throws<Ba2UnpackException>(() => BA2Unpacker.Unpack(archive, Path.Combine(dir.Path, "out")));

            Assert.Equal(archive, ex.Archive);
            Assert.NotNull(ex.InnerException);
            Assert.Contains(ex.InnerException!.GetType().Name, ex.Message);
        }

        [Fact]
        public void Unpack_MissingArchive_Throws()
        {
            using var dir = new ArchiveTempDir();
            string archive = Path.Combine(dir.Path, "missing.ba2");

            Assert.Throws<Ba2UnpackException>(() => BA2Unpacker.Unpack(archive, Path.Combine(dir.Path, "out")));
        }

        [Fact]
        public void UnpackBatched_OneBadArchive_ReportsItAndStillUnpacksTheOthers()
        {
            using var dir = new ArchiveTempDir();
            Directory.CreateDirectory(Path.Combine(dir.Path, "in"));
            string good = dir.Write(Path.Combine("in", "good.ba2"), Ba2Builder.Gnrl([Mesh]));
            string bad = dir.Write(Path.Combine("in", "bad.ba2"), TestBytes.Random(64, 2));
            string output = Path.Combine(dir.Path, "out");

            var failures = BA2Unpacker.UnpackBatched(Path.Combine(dir.Path, "in"), output);

            Assert.Equal(bad, Assert.Single(failures).Archive);
            Assert.Equal(Mesh.Content, File.ReadAllBytes(Path.Combine(output, "meshes", "a", "one.nif")));
            Assert.True(File.Exists(good));
        }
    }
}
