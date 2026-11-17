using AssetConverter;
using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests.Pipeline
{
    [Collection("Console redirection")]
    public sealed class CollisionRegenerationTests : IDisposable
    {
        private static readonly GameInputs Inputs = new("en", new string('b', 64));

        private readonly TextWriter _originalOut = Console.Out;
        private readonly TempDirectory _root = new();

        public CollisionRegenerationTests()
        {
            Console.SetOut(new StringWriter());
            Locations.CreateOutputDirectories();
        }

        public void Dispose()
        {
            Console.SetOut(_originalOut);
            _root.Dispose();
        }

        private InstallLocations Locations => new(Path.Combine(_root.Path, "game"), Path.Combine(_root.Path, "install"));

        private string Sidecar => Path.Combine(Locations.MeshesOut, "a", "a.col.json");

        private void InstallRecordedAtRevision(int collisionRevision) =>
            AssetVersionMarker.Write(Locations.OwnershipMarker, new InstalledAssets("1.0.0",
                new Dictionary<AssetGroup, int> { [AssetGroup.Audio] = 1, [AssetGroup.Collision] = collisionRevision }, Inputs));

        private InstalledAssets Marker() => AssetVersionMarker.Read(Locations.OwnershipMarker)!;

        private void WriteSidecar(string content)
        {
            Directory.CreateDirectory(Path.GetDirectoryName(Sidecar)!);
            File.WriteAllText(Sidecar, content);
        }

        [Fact]
        public void Run_AnInstallOfAnOlderRevision_IsRecordedAtTheCurrentRevisionSoTheNextInstallKeepsTheGroup()
        {
            InstallRecordedAtRevision(AssetGroups.CurrentRevisions[AssetGroup.Collision] - 1);

            CollisionRegeneration.Run(Locations, () => { });

            Assert.Equal(AssetGroups.CurrentRevisions[AssetGroup.Collision], Marker().Revisions[AssetGroup.Collision]);
            Assert.False(ReconversionPlan.Decide(Marker(), AssetGroups.CurrentRevisions).Reconverts(AssetGroup.Collision));
        }

        [Fact]
        public void Run_KeepsTheVersionTheGameInputsAndTheOtherGroupsOfTheMarker()
        {
            InstallRecordedAtRevision(1);

            CollisionRegeneration.Run(Locations, () => { });

            var marker = Marker();
            Assert.Equal("1.0.0", marker.Version);
            Assert.Equal(Inputs, marker.Inputs);
            Assert.Equal(1, marker.Revisions[AssetGroup.Audio]);
        }

        [Fact]
        public void Run_PurgesTheExistingSidecarsAndTheStampBeforeTheExtractionAndKeepsWhatItWrites()
        {
            InstallRecordedAtRevision(1);
            WriteSidecar("old");
            CollisionSidecars.RecordCurrent(Locations.MeshesOut);
            bool purgedBeforeExtraction = false;

            CollisionRegeneration.Run(Locations, () =>
            {
                purgedBeforeExtraction = !File.Exists(Sidecar) && !File.Exists(CollisionSidecars.StampPath(Locations.MeshesOut));
                WriteSidecar("new");
            });

            Assert.True(purgedBeforeExtraction);
            Assert.Equal("new", File.ReadAllText(Sidecar));
        }

        [Fact]
        public void Run_AFailedExtraction_LeavesTheGroupRecordedAtTheOldRevision()
        {
            InstallRecordedAtRevision(1);

            Assert.Throws<InvalidOperationException>(() => CollisionRegeneration.Run(Locations, () => throw new InvalidOperationException("worker failed")));

            Assert.Equal(1, Marker().Revisions[AssetGroup.Collision]);
        }
    }
}
