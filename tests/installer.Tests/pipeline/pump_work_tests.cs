using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests.Pipeline
{
    public sealed class PumpWorkTests
    {
        private static AssetGroup Group(string id) => AssetGroups.TryFromId(id, out var group) ? group : throw new ArgumentException(id);

        private static PumpWork Reconverting(params AssetGroup[] groups) => PumpWork.For(new HashSet<AssetGroup>(groups));

        [Fact]
        public void ConvertingEverythingRunsEveryStepOfTheReaderAsAFullInstallAlwaysDid() =>
            Assert.Equal(new PumpWork(true, true, true), PumpWork.For(ReconversionPlan.ReconvertAll.Reconverted));

        [Fact]
        public void ConvertingNothingReadsNothing() =>
            Assert.Equal(new PumpWork(false, false, false), Reconverting());

        [Theory]
        [InlineData("shaders")]
        [InlineData("interface")]
        [InlineData("audio")]
        [InlineData("video")]
        public void GroupsThatReadOnlyTheirOwnArchivesNeedNothingFromTheReader(string id) =>
            Assert.Equal(new PumpWork(false, false, false), Reconverting(Group(id)));

        [Theory]
        [InlineData("collision")]
        [InlineData("animations")]
        public void GroupsThatReadStagedFilesNeedTheMeshArchivesButNeitherTheTextureIndexNorTheTextures(string id) =>
            Assert.Equal(new PumpWork(false, true, false), Reconverting(Group(id)));

        [Fact]
        public void ConvertedTexturesAndDatabaseNeedEverything() =>
            Assert.Equal(new PumpWork(true, true, true), Reconverting(AssetGroup.TexturesDatabase));

        [Fact]
        public void AMeshesChangeCarriesThroughItsDependentsToEverythingTheReaderDoes()
        {
            var plan = ReconversionPlan.Decide(
                new InstalledAssets("1.0.0", AssetGroups.All.ToDictionary(group => group, group => 1)),
                new Dictionary<AssetGroup, int>(AssetGroups.CurrentRevisions) { [AssetGroup.Meshes] = 2 });

            Assert.Equal(new PumpWork(true, true, true), PumpWork.For(plan.Reconverted));
        }

        [Fact]
        public void AFailedMeshArchiveExtractionLeavesEveryConvertedGroupThatReadItUnfinished()
        {
            var reconverted = new HashSet<AssetGroup> { AssetGroup.Audio, AssetGroup.Meshes, AssetGroup.Animations, AssetGroup.TexturesDatabase };

            Assert.Equal([AssetGroup.Meshes, AssetGroup.Animations, AssetGroup.TexturesDatabase], PumpWork.MeshExtractionConsumers(reconverted));
        }

        [Fact]
        public void GroupsThatDoNotReadTheStagedMeshesAreNotConsumersOfTheExtraction() =>
            Assert.Empty(PumpWork.MeshExtractionConsumers(new HashSet<AssetGroup> { AssetGroup.Shaders, AssetGroup.Interface, AssetGroup.Audio, AssetGroup.Video }));
    }
}
