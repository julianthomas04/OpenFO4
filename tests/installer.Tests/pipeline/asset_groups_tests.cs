using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests.Pipeline
{
    public sealed class AssetGroupsTests
    {
        [Fact]
        public void EveryGroupHasOneEntryInTheTable()
        {
            Assert.Equal(Enum.GetValues<AssetGroup>().Order(), AssetGroups.All.Order());
        }

        [Fact]
        public void ThePersistedIdsAreTheOnesTheMarkerFormatPromises()
        {
            Assert.Equal(
                ["shaders", "interface", "audio", "video", "meshes", "collision", "animations", "textures-database"],
                AssetGroups.All.Select(AssetGroups.Id));
        }

        [Fact]
        public void IdsAreUniqueAndReadBackToTheirGroup()
        {
            Assert.All(AssetGroups.All, group =>
            {
                Assert.True(AssetGroups.TryFromId(AssetGroups.Id(group), out var read));
                Assert.Equal(group, read);
            });
            Assert.Equal(AssetGroups.All.Count, AssetGroups.All.Select(AssetGroups.Id).Distinct().Count());
        }

        [Fact]
        public void AnUnknownIdIsNotAGroup()
        {
            Assert.False(AssetGroups.TryFromId("Shaders", out _));
            Assert.False(AssetGroups.TryFromId("future", out _));
        }

        [Fact]
        public void EveryRevisionIsPositiveAndTheCurrentRevisionsAreTheTables()
        {
            Assert.All(AssetGroups.All, group =>
            {
                Assert.True(AssetGroups.Revision(group) >= 1);
                Assert.Equal(AssetGroups.Revision(group), AssetGroups.CurrentRevisions[group]);
            });
            Assert.Equal(AssetGroups.All.Count, AssetGroups.CurrentRevisions.Count);
        }

        [Fact]
        public void NoGroupIsBuiltFromItselfThroughItsDependents()
        {
            Assert.All(AssetGroups.All, group =>
            {
                var dependents = AssetGroups.WithDependents([group]).Where(other => other != group);
                Assert.All(dependents, dependent => Assert.DoesNotContain(group, AssetGroups.WithDependents([dependent]).Where(other => other != dependent)));
            });
        }

        [Fact]
        public void AGroupWithoutDependentsClosesOverItselfAlone()
        {
            Assert.Equal([AssetGroup.Audio], AssetGroups.WithDependents([AssetGroup.Audio]));
        }

        [Fact]
        public void MeshesPullInAnimationsAndTexturesDatabase()
        {
            Assert.Equal(
                new[] { AssetGroup.Meshes, AssetGroup.Animations, AssetGroup.TexturesDatabase }.Order(),
                AssetGroups.WithDependents([AssetGroup.Meshes]).Order());
        }

        [Fact]
        public void TheClosureOfSeveralGroupsIsTheUnionOfTheirs()
        {
            Assert.Equal(
                new[] { AssetGroup.Shaders, AssetGroup.Meshes, AssetGroup.Animations, AssetGroup.TexturesDatabase }.Order(),
                AssetGroups.WithDependents([AssetGroup.Shaders, AssetGroup.Meshes]).Order());
        }

        [Fact]
        public void TheClosureOfAClosureAddsNothing()
        {
            Assert.Equal(AssetGroups.WithDependents([AssetGroup.Meshes]), AssetGroups.WithDependents(AssetGroups.WithDependents([AssetGroup.Meshes])));
        }
    }
}
