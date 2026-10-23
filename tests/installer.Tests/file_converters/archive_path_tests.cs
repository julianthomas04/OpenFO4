using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public class ArchivePathTests
    {
        [Theory]
        [InlineData(@"Meshes\Actors\Dog\skeleton.hkx", true)]
        [InlineData("meshes/actors/dog/skeleton.HKX", true)]
        [InlineData(@"Meshes\Other\skeleton.hkx", false)]
        [InlineData(@"Meshes\Actors\Dog\skeleton.nif", false)]
        public void MatchesIgnoresCaseAndSlashDirection(string path, bool expected)
        {
            Assert.Equal(expected, ArchivePath.Matches(path, @"Meshes\Actors\", ".hkx"));
        }

        [Fact]
        public void TheActorHkxRuleAcceptsForwardSlashes()
        {
            Assert.True(HavokAnimationStage.IsActorHkx("Meshes/Actors/Dog/Animations/walk.hkx"));
            Assert.False(HavokAnimationStage.IsActorHkx("Meshes/Other/walk.hkx"));
        }

        [Fact]
        public void ComparerTreatsCaseAndSlashDirectionAsNoDifference()
        {
            var set = new HashSet<string>(ArchivePath.Comparer) { @"Meshes\A\x.nif" };

            Assert.Contains("meshes/a/X.NIF", set);
            Assert.DoesNotContain(@"meshes\a\y.nif", set);
        }
    }
}
