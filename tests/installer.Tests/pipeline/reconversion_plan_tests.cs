using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests.Pipeline
{
    public sealed class ReconversionPlanTests
    {
        private static IReadOnlyDictionary<AssetGroup, int> Current => AssetGroups.CurrentRevisions;

        private static InstalledAssets Installed(IEnumerable<KeyValuePair<AssetGroup, int>> revisions) => new("1.0.0", revisions.ToDictionary(r => r.Key, r => r.Value));

        private static Dictionary<AssetGroup, int> CurrentWith(AssetGroup group, int revision) =>
            Current.ToDictionary(r => r.Key, r => r.Key == group ? revision : r.Value);

        [Fact]
        public void ReconvertAllKeepsNothing()
        {
            var plan = ReconversionPlan.ReconvertAll;

            Assert.Empty(plan.Kept);
            Assert.All(AssetGroups.All, group => Assert.True(plan.Reconverts(group)));
        }

        [Fact]
        public void AnInstallWithoutAReadableRecordReconvertsEverything()
        {
            Assert.Same(ReconversionPlan.ReconvertAll, ReconversionPlan.Decide(null, Current));
        }

        [Fact]
        public void ALegacyRecordThatNamesNoGroupReconvertsEverything()
        {
            var plan = ReconversionPlan.Decide(Installed([]), Current);

            Assert.Empty(plan.Kept);
            Assert.Equal(AssetGroups.All.Order(), plan.Reconverted.Order());
        }

        [Fact]
        public void AnInstallAtEveryCurrentRevisionKeepsEverything()
        {
            var plan = ReconversionPlan.Decide(Installed(Current), Current);

            Assert.Empty(plan.Reconverted);
            Assert.Equal(AssetGroups.All.Order(), plan.Kept.Order());
            Assert.All(AssetGroups.All, group => Assert.False(plan.Reconverts(group)));
        }

        [Fact]
        public void OnlyAChangedGroupWithoutDependentsIsReconverted()
        {
            var plan = ReconversionPlan.Decide(Installed(Current), CurrentWith(AssetGroup.Audio, Current[AssetGroup.Audio] + 1));

            Assert.Equal([AssetGroup.Audio], plan.Reconverted);
            Assert.Equal(AssetGroups.All.Where(group => group != AssetGroup.Audio).Order(), plan.Kept.Order());
        }

        [Fact]
        public void ARevisionThatWentDownIsAsChangedAsOneThatWentUp()
        {
            var installed = Installed(Current.ToDictionary(r => r.Key, r => r.Key == AssetGroup.Video ? r.Value + 1 : r.Value));

            Assert.Equal([AssetGroup.Video], ReconversionPlan.Decide(installed, Current).Reconverted);
        }

        [Fact]
        public void ChangedMeshesPullInTheGroupsBuiltFromThem()
        {
            var plan = ReconversionPlan.Decide(Installed(Current), CurrentWith(AssetGroup.Meshes, Current[AssetGroup.Meshes] + 1));

            Assert.Equal(new[] { AssetGroup.Meshes, AssetGroup.Animations, AssetGroup.TexturesDatabase }.Order(), plan.Reconverted.Order());
            Assert.Equal(new[] { AssetGroup.Shaders, AssetGroup.Interface, AssetGroup.Audio, AssetGroup.Video, AssetGroup.Collision }.Order(), plan.Kept.Order());
        }

        [Fact]
        public void ADependentThatChangedDoesNotPullInWhatItWasBuiltFrom()
        {
            var plan = ReconversionPlan.Decide(Installed(Current), CurrentWith(AssetGroup.Animations, Current[AssetGroup.Animations] + 1));

            Assert.Equal([AssetGroup.Animations], plan.Reconverted);
        }

        [Fact]
        public void AGroupMissingFromTheRecordIsReconvertedWithItsDependents()
        {
            var plan = ReconversionPlan.Decide(Installed(Current.Where(r => r.Key != AssetGroup.Meshes)), Current);

            Assert.Equal(new[] { AssetGroup.Meshes, AssetGroup.Animations, AssetGroup.TexturesDatabase }.Order(), plan.Reconverted.Order());
        }

        [Fact]
        public void KeptAndReconvertedPartitionTheGroups()
        {
            var plan = ReconversionPlan.Decide(Installed(Current), CurrentWith(AssetGroup.Meshes, Current[AssetGroup.Meshes] + 1));

            Assert.Empty(plan.Kept.Intersect(plan.Reconverted));
            Assert.Equal(AssetGroups.All.Order(), plan.Kept.Union(plan.Reconverted).Order());
        }

        [Fact]
        public void ADecisionReadFromAMarkerWithAnUnknownIdIgnoresThatId()
        {
            var installed = AssetVersionMarker.Parse("Asset Version 1.0.0\n" + string.Concat(AssetGroups.All.Select(g => $"{AssetGroups.Id(g)}={Current[g]}\n")) + "future=9\n");

            Assert.Empty(ReconversionPlan.Decide(installed, Current).Reconverted);
        }
    }
}
