using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests.Pipeline
{
    public class ArchiveOverlapTests
    {
        private static ArchiveOverlap Of(params string[][] archives) => new([.. archives.Select(paths => (IEnumerable<string>)paths)], path => path);

        [Fact]
        public void APathInSeveralArchivesBelongsToTheLastOne()
        {
            var overlap = Of([@"a\x.nif", @"a\y.nif"], [@"a\x.nif"], [@"a\x.nif", @"b\z.nif"]);

            Assert.False(overlap.IsLastHolder(0, @"a\x.nif"));
            Assert.False(overlap.IsLastHolder(1, @"a\x.nif"));
            Assert.True(overlap.IsLastHolder(2, @"a\x.nif"));
            Assert.True(overlap.IsLastHolder(0, @"a\y.nif"));
        }

        [Fact]
        public void PathsCompareIgnoringCase()
        {
            var overlap = Of([@"Meshes\A.NIF"], [@"meshes\a.nif"]);

            Assert.False(overlap.IsLastHolder(0, @"Meshes\A.NIF"));
            Assert.True(overlap.IsLastHolder(1, @"meshes\a.nif"));
        }

        [Fact]
        public void TheCountsListEveryFileOnceAndAddUpToTheTotal()
        {
            var overlap = Of([@"a\x", @"a\y"], [@"A\X", @"b\z"], []);

            Assert.Equal(1, overlap.FileCount(0));
            Assert.Equal(2, overlap.FileCount(1));
            Assert.Equal(0, overlap.FileCount(2));
            Assert.Equal(3, overlap.TotalFileCount);
        }

        [Fact]
        public void AnArchiveListingOneFileTwiceUnderVariantSpellingsCountsItOnce()
        {
            var overlap = Of([@"Meshes\A\x.nif", @"meshes/a/X.NIF", @"meshes\a\y.nif"], [@"Meshes\A\Y.nif"]);

            Assert.Equal(1, overlap.FileCount(0));
            Assert.Equal(1, overlap.FileCount(1));
            Assert.Equal(2, overlap.TotalFileCount);
            Assert.True(overlap.IsLastHolder(0, @"meshes\a\x.nif"));
        }

        [Fact]
        public void APathNoArchiveContainsHasNoHolder()
        {
            var overlap = Of([@"a\x"]);

            Assert.False(overlap.IsLastHolder(0, @"other\file"));
        }

        [Fact]
        public void TheDestinationDecidesWhichEntriesCollide()
        {
            var overlap = new ArchiveOverlap([[@"textures\a.dds"], [@"a.dds"]], path => path.StartsWith(@"textures\", StringComparison.Ordinal) ? path[9..] : path);

            Assert.False(overlap.IsLastHolder(0, @"textures\a.dds"));
            Assert.True(overlap.IsLastHolder(1, "a.dds"));
            Assert.Equal(1, overlap.TotalFileCount);
        }
    }
}
