using AssetConverter;
using AssetConverter.Pipeline;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests.Pipeline
{
    /// <summary>Prints to the console, so it runs serially with the other tests that redirect it.</summary>
    [Collection("Console redirection")]
    public sealed class PipelineStagesMaterialStagingTests : IDisposable
    {
        private const string MaterialsArchive = "Fallout4 - Materials.ba2";
        private const string MeshesArchive = "Fallout4 - Meshes.ba2";

        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private InstallLocations Locations => new(Path.Combine(_root.Path, "game"), Path.Combine(_root.Path, "install"), Path.Combine(_root.Path, "settings"));

        private PipelineStages Stages => new(Locations, MemoryBudget.Unlimited(), TestContext.Current.CancellationToken);

        private static byte[] Content(byte seed) => [.. Enumerable.Range(0, 64).Select(i => (byte)(seed + i))];

        private void WriteArchive(string name, byte[] archive)
        {
            Directory.CreateDirectory(Locations.GameDataDirectory);
            File.WriteAllBytes(Path.Combine(Locations.GameDataDirectory, name), archive);
        }

        private void WriteMaterialsArchive() =>
            WriteArchive(MaterialsArchive, Ba2Builder.Gnrl([
                new Ba2File(@"Materials\Effects\Rain.bgem", Content(1), Compress: false),
                new Ba2File(@"Materials\Props\Crate.bgsm", Content(9), Compress: true),
            ]));

        private string Staged(string relative) => Path.Combine(Locations.StagingRoot, relative);

        [Fact]
        public void EnsureStagedMaterials_NoStagingFolder_ExtractsTheMaterialsArchiveAndMarksIt()
        {
            WriteMaterialsArchive();

            Stages.EnsureStagedMaterials();

            Assert.Equal(Content(1), File.ReadAllBytes(Staged(@"Materials\Effects\Rain.bgem")));
            Assert.Equal(Content(9), File.ReadAllBytes(Staged(@"Materials\Props\Crate.bgsm")));
            Assert.True(File.Exists(Staged($".{MaterialsArchive}.extracted")));
        }

        [Fact]
        public void EnsureStagedMaterials_WithAMeshArchiveInTheGameFolder_ExtractsOnlyTheMaterials()
        {
            WriteMaterialsArchive();
            WriteArchive(MeshesArchive, Ba2Builder.Gnrl([new Ba2File(@"Meshes\Props\Crate.nif", Content(5), Compress: false)]));

            Stages.EnsureStagedMaterials();

            Assert.False(Directory.Exists(Staged("Meshes")));
            Assert.False(File.Exists(Staged($".{MeshesArchive}.extracted")));
        }

        [Fact]
        public void EnsureStagedMaterials_ArchiveAlreadyMarked_LeavesTheStagedFilesAlone()
        {
            WriteMaterialsArchive();
            Stages.EnsureStagedMaterials();
            File.WriteAllBytes(Staged(@"Materials\Effects\Rain.bgem"), Content(200));

            Stages.EnsureStagedMaterials();

            Assert.Equal(Content(200), File.ReadAllBytes(Staged(@"Materials\Effects\Rain.bgem")));
        }

        [Fact]
        public void EnsureStagedMaterials_MaterialsArchiveMissing_ThrowsAndStagesNothing()
        {
            Directory.CreateDirectory(Locations.GameDataDirectory);

            var error = Assert.Throws<FileNotFoundException>(() => Stages.EnsureStagedMaterials());

            Assert.Contains(MaterialsArchive, error.Message);
            Assert.False(Directory.Exists(Locations.StagingRoot));
        }

        [Fact]
        public void EnsureStagedMaterials_EntryFailsToExtract_ThrowsAndWithholdsTheMarker()
        {
            byte[] damaged = Ba2Builder.Gnrl([new Ba2File(@"Materials\Effects\Rain.bgem", TestBytes.Random(6000, 2), Compress: true)]);
            const int firstBlobOffset = 24 + 36;
            for (int i = firstBlobOffset + 4; i < firstBlobOffset + 20; i++)
                damaged[i] ^= 0xFF;
            WriteArchive(MaterialsArchive, damaged);

            var error = Assert.Throws<InvalidOperationException>(() => Stages.EnsureStagedMaterials());

            Assert.Contains("failed to extract", error.Message);
            Assert.False(File.Exists(Staged($".{MaterialsArchive}.extracted")));
        }

        [Fact]
        public void EnsureStagedMaterials_MarkerWithoutStagedMaterials_ThrowsNamingTheFolderAndTheMarker()
        {
            WriteMaterialsArchive();
            Directory.CreateDirectory(Locations.StagingRoot);
            File.WriteAllText(Staged($".{MaterialsArchive}.extracted"), "");

            var error = Assert.Throws<InvalidOperationException>(() => Stages.EnsureStagedMaterials());

            Assert.Contains(Locations.StagedMaterials, error.Message);
            Assert.Contains($".{MaterialsArchive}.extracted", error.Message);
        }
    }
}
