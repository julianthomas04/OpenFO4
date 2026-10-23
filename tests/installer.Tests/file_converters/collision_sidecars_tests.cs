using AssetConverter;
using Xunit;

namespace InstallerTests
{
    /// <summary>Prints the purge to the console, so it runs serially with the other tests that redirect it.</summary>
    [Collection("Console redirection")]
    public sealed class CollisionSidecarsTests : IDisposable
    {
        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private readonly TempDirectory _meshes = new();

        public void Dispose() => _meshes.Dispose();

        private string WriteSidecar(string relative)
        {
            string path = Path.Combine(_meshes.Path, relative);
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllText(path, "{}");
            return path;
        }

        [Fact]
        public void StaleReason_WithoutAStamp_SaysTheSidecarsRecordNoConversion()
        {
            Assert.Equal("it records no conversion", CollisionSidecars.StaleReason(_meshes.Path));
        }

        [Fact]
        public void StaleReason_AfterRecordCurrent_IsNone()
        {
            CollisionSidecars.RecordCurrent(_meshes.Path);

            Assert.Null(CollisionSidecars.StaleReason(_meshes.Path));
        }

        [Fact]
        public void StaleReason_StampOfAnotherExtractorVersion_NamesTheVersion()
        {
            new ConversionStamp(CollisionSidecars.ExtractorVersion - 1, null, []).WriteTo(CollisionSidecars.StampPath(_meshes.Path));

            Assert.Equal("it was converted by another extractor version", CollisionSidecars.StaleReason(_meshes.Path));
        }

        [Fact]
        public void PurgeIfStale_UnstampedSidecars_DeletesAllOfThemAndKeepsOtherFiles()
        {
            string nested = WriteSidecar(Path.Combine("props", "crate.col.json"));
            string manifest = Path.Combine(_meshes.Path, "collision_manifest.json");
            File.WriteAllText(manifest, "{}");
            string glb = Path.Combine(_meshes.Path, "crate.glb");
            File.WriteAllText(glb, "x");

            CollisionSidecars.PurgeIfStale(_meshes.Path, Ct);

            Assert.False(File.Exists(nested));
            Assert.True(File.Exists(manifest));
            Assert.True(File.Exists(glb));
        }

        [Fact]
        public void PurgeIfStale_CurrentlyStampedSidecars_KeepsThem()
        {
            string sidecar = WriteSidecar("crate.col.json");
            CollisionSidecars.RecordCurrent(_meshes.Path);

            CollisionSidecars.PurgeIfStale(_meshes.Path, Ct);

            Assert.True(File.Exists(sidecar));
            Assert.Null(CollisionSidecars.StaleReason(_meshes.Path));
        }

        [Fact]
        public void PurgeIfStale_OutputFolderDoesNotExist_DoesNothing()
        {
            string missing = Path.Combine(_meshes.Path, "absent");

            CollisionSidecars.PurgeIfStale(missing, Ct);

            Assert.False(Directory.Exists(missing));
        }

        [Fact]
        public void Purge_RemovesTheStampWithTheSidecars()
        {
            string sidecar = WriteSidecar("crate.col.json");
            CollisionSidecars.RecordCurrent(_meshes.Path);

            CollisionSidecars.Purge(_meshes.Path, Ct);

            Assert.False(File.Exists(sidecar));
            Assert.False(File.Exists(CollisionSidecars.StampPath(_meshes.Path)));
        }
    }
}
