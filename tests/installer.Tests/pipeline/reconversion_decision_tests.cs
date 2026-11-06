using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests.Pipeline
{
    public sealed class ReconversionDecisionTests : IDisposable
    {
        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private string Marker => Path.Combine(_root.Path, ".openfo4-install");

        private static Dictionary<AssetGroup, int> Current(params (AssetGroup Group, int Revision)[] changed)
        {
            var current = new Dictionary<AssetGroup, int>(AssetGroups.CurrentRevisions);
            foreach (var (group, revision) in changed)
                current[group] = revision;
            return current;
        }

        [Fact]
        public void WithoutAMarkerEveryGroupIsConvertedAndTheLinesSayThereIsNoRecord()
        {
            var decision = ReconversionDecision.Read(Marker, AssetGroups.CurrentRevisions);

            Assert.Null(decision.Installed);
            Assert.Null(decision.Warning);
            Assert.Equal(AssetGroups.All.ToHashSet(), decision.Plan.Reconverted);
            Assert.Equal("No readable record of an earlier install: every asset group is converted.", decision.Describe(AssetGroups.CurrentRevisions)[0]);
        }

        [Fact]
        public void AMarkerThatCannotBeReadConvertsEverythingAndSaysWhy()
        {
            File.WriteAllText(Marker, "Asset Version 1.0.0\n");
            using var holder = new FileStream(Marker, FileMode.Open, FileAccess.Read, FileShare.None);

            var decision = ReconversionDecision.Read(Marker, AssetGroups.CurrentRevisions);

            Assert.Equal(AssetGroups.All.ToHashSet(), decision.Plan.Reconverted);
            Assert.StartsWith($"WARNING: the install record '{Marker}' cannot be read", decision.Warning, StringComparison.Ordinal);
            Assert.Equal(decision.Warning, decision.Describe(AssetGroups.CurrentRevisions)[0]);
        }

        [Fact]
        public void TheLinesNameEveryGroupWithTheRevisionsBehindItsVerdict()
        {
            var installed = new InstalledAssets("0.9.0", new Dictionary<AssetGroup, int>
            {
                [AssetGroup.Shaders] = 1,
                [AssetGroup.Audio] = 1,
                [AssetGroup.Meshes] = 1,
                [AssetGroup.Animations] = 1,
                [AssetGroup.TexturesDatabase] = 1,
            });
            AssetVersionMarker.Write(Marker, installed);
            var current = Current((AssetGroup.Audio, 2), (AssetGroup.Meshes, 2));

            var lines = ReconversionDecision.Read(Marker, current).Describe(current);

            Assert.Equal(
            [
                "Asset groups of the install being updated (recorded by installer 0.9.0):",
                "  shaders: kept (revision 1)",
                "  interface: converted (not recorded, revision 1)",
                "  audio: converted (revision 1 -> 2)",
                "  video: converted (not recorded, revision 1)",
                "  meshes: converted (revision 1 -> 2)",
                "  collision: converted (not recorded, revision 1)",
                "  animations: converted (revision 1, built from a group that is converted)",
                "  textures-database: converted (revision 1, built from a group that is converted)",
            ], lines);
        }
    }
}
