using SharpBSABA2.BSAUtil;
using SharpBSABA2.Enums;
using Xunit;

namespace InstallerTests.Archives
{
    public class BsaArchiveTests
    {
        private static readonly BsaFile Nif = new("tree.nif", TestBytes.Random(3000, 41));
        private static readonly BsaFile Dds = new("bark.dds", TestBytes.Repeating(5000, 3));
        private static readonly BsaFile Wav = new("shot.wav", TestBytes.Random(800, 42));

        private static readonly BsaFolder[] Folders =
        [
            new("meshes\\flora", [Nif]),
            new("textures\\flora", [Dds, Wav]),
        ];

        private static readonly string[] ExpectedPaths =
            new[] { @"meshes\flora\tree.nif", @"textures\flora\bark.dds", @"textures\flora\shot.wav" }.Select(Native).ToArray();

        private static readonly int[] ExpectedIndices = [0, 1, 2];

        [Fact]
        public void Open_FalloutBsa_ReadsFolderAndFileNames()
        {
            using var dir = new ArchiveTempDir();
            var archive = dir.OpenBsa("a.bsa", BsaBuilder.Build(Folders, BsaBuilder.FalloutVersion, compressed: false));

            Assert.Equal(ArchiveTypes.BSA, archive.Type);
            Assert.Equal(3, archive.FileCount);
            Assert.Equal(ExpectedPaths, archive.Files.Select(f => f.FullPath));
        }

        [Fact]
        public void Open_ArchiveWithSeveralFolders_GivesEveryEntryItsOwnIndex()
        {
            using var dir = new ArchiveTempDir();
            var archive = dir.OpenBsa("a.bsa", BsaBuilder.Build(Folders, BsaBuilder.FalloutVersion, compressed: false));

            Assert.Equal(ExpectedIndices, archive.Files.Select(f => f.Index));
        }

        [Fact]
        public void GetDataStream_UncompressedBsa_ReturnsStoredBytes()
        {
            using var dir = new ArchiveTempDir();
            var archive = dir.OpenBsa("a.bsa", BsaBuilder.Build(Folders, BsaBuilder.FalloutVersion, compressed: false));

            Assert.Equal(Wav.Content, archive.Files[2].GetDataStream().ToArray());
        }

        [Fact]
        public void GetDataStream_ZlibCompressedBsa_InflatesAfterTheOriginalSizePrefix()
        {
            using var dir = new ArchiveTempDir();
            var archive = dir.OpenBsa("a.bsa", BsaBuilder.Build(Folders, BsaBuilder.FalloutVersion, compressed: true));

            Assert.True(archive.Compressed);
            Assert.Equal(Dds.Content, archive.Files[1].GetDataStream().ToArray());
            Assert.Equal(Nif.Content, archive.Files[0].GetDataStream().ToArray());
        }

        [Fact]
        public void IsSupportedVersion_KnownAndUnknownBsaVersions_AreDistinguished()
        {
            using var dir = new ArchiveTempDir();
            string known = dir.Write("known.bsa", BsaBuilder.Build(Folders, BsaBuilder.FalloutVersion, compressed: false));
            var unknownBytes = BsaBuilder.Build(Folders, BsaBuilder.FalloutVersion, compressed: false);
            unknownBytes[4] = 0x77;
            string unknown = dir.Write("unknown.bsa", unknownBytes);

            Assert.True(BSA.IsSupportedVersion(known));
            Assert.False(BSA.IsSupportedVersion(unknown));
        }

        [Fact]
        public void Open_TruncatedFolderTable_Throws()
        {
            using var dir = new ArchiveTempDir();
            string path = dir.Write("bad.bsa", BsaBuilder.Build(Folders, BsaBuilder.FalloutVersion, compressed: false)[..50]);

            Assert.ThrowsAny<EndOfStreamException>(() => new BSA(path));
        }

        [Fact]
        public void Open_Failure_ReleasesTheFileHandle()
        {
            using var dir = new ArchiveTempDir();
            string path = dir.Write("bad.bsa", BsaBuilder.Build(Folders, BsaBuilder.FalloutVersion, compressed: false)[..50]);

            Assert.ThrowsAny<EndOfStreamException>(() => new BSA(path));

            File.Delete(path);
            Assert.False(File.Exists(path));
        }

        private static string Native(string archivePath) => archivePath.Replace('\\', Path.DirectorySeparatorChar);
    }
}
