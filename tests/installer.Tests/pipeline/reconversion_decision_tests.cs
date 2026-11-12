using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests.Pipeline
{
    public sealed class ReconversionDecisionTests : IDisposable
    {
        private static readonly GameInputs Inputs = new("en", new string('a', 64));

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
            var decision = ReconversionDecision.Read(Marker, AssetGroups.CurrentRevisions, Inputs);

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

            var decision = ReconversionDecision.Read(Marker, AssetGroups.CurrentRevisions, Inputs);

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
            }, Inputs);
            AssetVersionMarker.Write(Marker, installed);
            var current = Current((AssetGroup.Audio, 2), (AssetGroup.Meshes, 2), (AssetGroup.TexturesDatabase, 1), (AssetGroup.Interface, 1));

            var lines = ReconversionDecision.Read(Marker, current, Inputs).Describe(current);

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

        [Fact]
        public void GroupsConvertedFromTheSameInputsAreKept()
        {
            AssetVersionMarker.Write(Marker, new InstalledAssets("1.0.0", AssetGroups.CurrentRevisions, Inputs));

            var decision = ReconversionDecision.Read(Marker, AssetGroups.CurrentRevisions, Inputs);

            Assert.Null(decision.Drift);
            Assert.Empty(decision.Plan.Reconverted);
        }

        public static TheoryData<string?, string?, string> Drifts() => new()
        {
            { "de", null, "the game language changed from 'en' to 'de'" },
            { null, new string('c', 64), "the game files changed" },
        };

        [Theory]
        [MemberData(nameof(Drifts))]
        public void GroupsConvertedFromOtherInputsAreAllConvertedAgainWhateverTheirRevisions(string? language, string? files, string expectedReason)
        {
            AssetVersionMarker.Write(Marker, new InstalledAssets("1.0.0", AssetGroups.CurrentRevisions, Inputs));
            var now = new GameInputs(language ?? Inputs.Language, files ?? Inputs.Files);

            var decision = ReconversionDecision.Read(Marker, AssetGroups.CurrentRevisions, now);

            Assert.Equal(expectedReason, decision.Drift);
            Assert.Equal(AssetGroups.All.ToHashSet(), decision.Plan.Reconverted);
            var lines = decision.Describe(AssetGroups.CurrentRevisions);
            Assert.Equal($"The install being updated (recorded by installer 1.0.0) cannot be kept: {expectedReason}. Every asset group is converted.", lines[0]);
            Assert.All(lines.Skip(1), line => Assert.Matches(@"^  [a-z-]+: converted \(revision \d+\)$", line));
        }

        [Fact]
        public void AMarkerThatRecordsNoInputsVouchesForNothing()
        {
            AssetVersionMarker.Write(Marker, new InstalledAssets("1.0.0", AssetGroups.CurrentRevisions));

            var decision = ReconversionDecision.Read(Marker, AssetGroups.CurrentRevisions, Inputs);

            Assert.Equal("the record names no game language and files", decision.Drift);
            Assert.Equal(AssetGroups.All.ToHashSet(), decision.Plan.Reconverted);
        }

        [Fact]
        public void InputsThatCannotBeDeterminedConvertEverything()
        {
            AssetVersionMarker.Write(Marker, new InstalledAssets("1.0.0", AssetGroups.CurrentRevisions, Inputs));

            var decision = ReconversionDecision.Read(Marker, AssetGroups.CurrentRevisions, inputs: null);

            Assert.Equal("the game inputs of this install are unknown", decision.Drift);
            Assert.Equal(AssetGroups.All.ToHashSet(), decision.Plan.Reconverted);
        }

        [Fact]
        public void ForAnInstallTheInputsComeFromTheGameAndAnUnreadableGameConvertsEverythingWithAWarning()
        {
            var locations = new InstallLocations(Path.Combine(_root.Path, "game"), Path.Combine(_root.Path, "install"), Path.Combine(_root.Path, "docs"));
            Directory.CreateDirectory(locations.InstallRoot);
            AssetVersionMarker.Write(locations.OwnershipMarker, new InstalledAssets("1.0.0", AssetGroups.CurrentRevisions, Inputs));

            var decision = ReconversionDecision.ForInstall(locations, AssetGroups.CurrentRevisions);

            Assert.Null(decision.Inputs);
            Assert.StartsWith("WARNING: the game language and files could not be determined", decision.Warning, StringComparison.Ordinal);
            Assert.Equal(AssetGroups.All.ToHashSet(), decision.Plan.Reconverted);
        }

        [Fact]
        public void ForAnInstallWhoseGameMatchesTheRecordEveryGroupIsKept()
        {
            string game = Path.Combine(_root.Path, "game");
            Directory.CreateDirectory(Path.Combine(game, "Data"));
            File.WriteAllText(Path.Combine(game, "Fallout4_Default.ini"), "[General]\nsLanguage=en\n");
            File.WriteAllText(Path.Combine(game, "Data", "Fallout4.esm"), "x");
            var locations = new InstallLocations(game, Path.Combine(_root.Path, "install"), Path.Combine(_root.Path, "docs"));
            Directory.CreateDirectory(locations.InstallRoot);
            AssetVersionMarker.Write(locations.OwnershipMarker, new InstalledAssets("1.0.0", AssetGroups.CurrentRevisions, GameInputs.Read(locations)));

            var decision = ReconversionDecision.ForInstall(locations, AssetGroups.CurrentRevisions);

            Assert.Null(decision.Warning);
            Assert.Empty(decision.Plan.Reconverted);
        }
    }
}
