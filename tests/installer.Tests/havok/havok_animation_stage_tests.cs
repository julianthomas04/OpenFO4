using AssetConverter;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests
{
    public class HavokAnimationStageTests
    {
        [Fact]
        public void RenderSkeletonSources_WorksFromTheStagedHkxFilesBeforeAnyGlbExists()
        {
            using var dir = new ArchiveTempDir();
            string staging = Path.Combine(dir.Path, "staging");
            string meshesOut = Path.Combine(dir.Path, "assets", "meshes");
            Directory.CreateDirectory(Path.Combine(staging, "Meshes", "Actors", "Dog", "CharacterAssets"));
            File.WriteAllBytes(Path.Combine(staging, "Meshes", "Actors", "Dog", "CharacterAssets", "skeleton.hkx"), [0]);
            Directory.CreateDirectory(Path.Combine(staging, "Meshes", "Actors", "ViciousDog", "Animations"));

            var sources = HavokAnimationStage.RenderSkeletonSources(staging, meshesOut);

            Assert.Equal(
                [
                    Path.Combine(staging, "Meshes", "Actors", "Dog", "CharacterAssets", "skeleton.nif"),
                    Path.Combine(staging, "Meshes", "Actors", "Dogmeat", "CharacterAssets", "skeleton.nif"),
                ],
                sources.Select(Path.GetFullPath));
            Assert.False(Directory.Exists(meshesOut));
        }

        [Fact]
        public void RenderSkeletonSources_WithoutStagedActors_IsEmpty()
        {
            using var dir = new ArchiveTempDir();

            Assert.Empty(HavokAnimationStage.RenderSkeletonSources(Path.Combine(dir.Path, "staging"), Path.Combine(dir.Path, "meshes")));
        }
    }
}
