using AssetConverter;
using AssetConverter.Pipeline;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests.Pipeline
{
    /// <summary>Prints to the console, so it runs serially with the other tests that redirect it.</summary>
    [Collection("Console redirection")]
    public sealed class PipelineStagesMeshStagingTests : IDisposable
    {
        private const string MeshesArchive = "Fallout4 - Meshes.ba2";
        private const string MeshesExtraArchive = "Fallout4 - MeshesExtra.ba2";

        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private InstallLocations Locations => new(Path.Combine(_root.Path, "game"), Path.Combine(_root.Path, "install"), Path.Combine(_root.Path, "settings"));

        private PipelineStages Stages => new(Locations, MemoryBudget.Unlimited(), TestContext.Current.CancellationToken);

        private static byte[] Content(byte seed) => [.. Enumerable.Range(0, 64).Select(i => (byte)(seed + i))];

        private void WriteArchive(string name, params Ba2File[] files)
        {
            Directory.CreateDirectory(Locations.GameDataDirectory);
            File.WriteAllBytes(Path.Combine(Locations.GameDataDirectory, name), Ba2Builder.Gnrl(files));
        }

        private void WriteBothArchives()
        {
            WriteArchive(MeshesArchive, new Ba2File(@"Meshes\Props\Crate.nif", Content(1), Compress: false));
            WriteArchive(MeshesExtraArchive, new Ba2File(@"Meshes\Props\Barrel.nif", Content(9), Compress: true));
        }

        private string Staged(string relative) => Path.Combine(Locations.StagingRoot, relative);

        [Fact]
        public void EnsureStagedMeshArchives_NoStagingFolder_ExtractsBothMeshArchivesAndMarksThem()
        {
            WriteBothArchives();

            Stages.EnsureStagedMeshArchives();

            Assert.Equal(Content(1), File.ReadAllBytes(Staged(@"Meshes\Props\Crate.nif")));
            Assert.Equal(Content(9), File.ReadAllBytes(Staged(@"Meshes\Props\Barrel.nif")));
            Assert.True(File.Exists(Staged($".{MeshesArchive}.extracted")));
            Assert.True(File.Exists(Staged($".{MeshesExtraArchive}.extracted")));
        }

        [Fact]
        public void EnsureStagedMeshArchives_ArchivesAlreadyMarked_LeavesTheStagedFilesAlone()
        {
            WriteBothArchives();
            Stages.EnsureStagedMeshArchives();
            File.WriteAllBytes(Staged(@"Meshes\Props\Crate.nif"), Content(200));

            Stages.EnsureStagedMeshArchives();

            Assert.Equal(Content(200), File.ReadAllBytes(Staged(@"Meshes\Props\Crate.nif")));
        }

        [Fact]
        public void EnsureStagedMeshArchives_OnlyOneArchiveMarked_ExtractsTheOther()
        {
            WriteBothArchives();
            Stages.EnsureStagedMeshArchives();
            File.Delete(Staged($".{MeshesExtraArchive}.extracted"));
            File.Delete(Staged(@"Meshes\Props\Barrel.nif"));

            Stages.EnsureStagedMeshArchives();

            Assert.Equal(Content(9), File.ReadAllBytes(Staged(@"Meshes\Props\Barrel.nif")));
            Assert.True(File.Exists(Staged($".{MeshesExtraArchive}.extracted")));
        }

        [Fact]
        public void EnsureStagedMeshArchives_MeshArchiveMissing_ThrowsAndStagesNothing()
        {
            WriteArchive(MeshesArchive, new Ba2File(@"Meshes\Props\Crate.nif", Content(1), Compress: false));

            var error = Assert.Throws<FileNotFoundException>(() => Stages.EnsureStagedMeshArchives());

            Assert.Contains(MeshesExtraArchive, error.Message);
            Assert.False(Directory.Exists(Locations.StagingRoot));
        }

        [Fact]
        public void EnsureStagedMeshArchives_MarkersWithoutStagedMeshes_Throws()
        {
            WriteBothArchives();
            Directory.CreateDirectory(Locations.StagingRoot);
            File.WriteAllText(Staged($".{MeshesArchive}.extracted"), "");
            File.WriteAllText(Staged($".{MeshesExtraArchive}.extracted"), "");

            Assert.Throws<InvalidOperationException>(() => Stages.EnsureStagedMeshArchives());
        }
    }
}
