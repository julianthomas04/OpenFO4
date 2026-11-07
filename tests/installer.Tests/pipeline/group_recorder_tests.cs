using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests.Pipeline
{
    [Collection("Console redirection")]
    public sealed class GroupRecorderTests : IDisposable
    {
        private static readonly GameInputs Inputs = new("en", new string('b', 64));

        private readonly TextWriter _originalError = Console.Error;
        private readonly StringWriter _errors = new();
        private readonly TempDirectory _root = new();

        public GroupRecorderTests()
        {
            Console.SetError(_errors);
            Locations.CreateOutputDirectories();
        }

        public void Dispose()
        {
            Console.SetError(_originalError);
            _root.Dispose();
        }

        private InstallLocations Locations => new(Path.Combine(_root.Path, "game"), Path.Combine(_root.Path, "install"));

        private GroupRecorder Recorder() => new(Locations, "2.0.0", Inputs);

        private InstalledAssets? Marker() => AssetVersionMarker.Read(Locations.OwnershipMarker);

        [Fact]
        public void ACompletedGroupIsListedAtItsCurrentRevisionWithTheVersionAndTheGameInputs()
        {
            Recorder().Complete(AssetGroup.Audio);

            var marker = Marker()!;
            Assert.Equal("2.0.0", marker.Version);
            Assert.Equal(Inputs, marker.Inputs);
            Assert.Equal(new KeyValuePair<AssetGroup, int>(AssetGroup.Audio, AssetGroups.CurrentRevisions[AssetGroup.Audio]), Assert.Single(marker.Revisions));
        }

        [Fact]
        public void AWithheldGroupIsNotListedAndTheWarningNamesItAndTheReason()
        {
            var recorder = Recorder();
            recorder.Withhold([AssetGroup.Meshes, AssetGroup.Animations], "some mesh archive entries failed to extract");

            recorder.Complete(AssetGroup.Meshes);
            recorder.Complete(AssetGroup.Audio);

            Assert.Equal([AssetGroup.Audio], Marker()!.Revisions.Keys);
            Assert.Contains("the 'meshes' asset group is not recorded as complete (some mesh archive entries failed to extract); the next install converts it again", _errors.ToString(), StringComparison.Ordinal);
        }

        [Fact]
        public void EveryReasonForAWithheldGroupIsGivenOnce()
        {
            var recorder = Recorder();
            recorder.Withhold([AssetGroup.TexturesDatabase], "textures failed");
            recorder.Withhold([AssetGroup.TexturesDatabase], "textures failed");
            recorder.Withhold([AssetGroup.TexturesDatabase], "meshes failed");

            recorder.Complete(AssetGroup.TexturesDatabase);

            Assert.Contains("(textures failed; meshes failed)", _errors.ToString(), StringComparison.Ordinal);
        }

        [Fact]
        public void AMarkerThatCannotBeWrittenFailsTheCompletion()
        {
            File.Delete(Locations.OwnershipMarker);
            Directory.CreateDirectory(Locations.OwnershipMarker);

            var failure = Assert.Throws<IOException>(() => Recorder().Complete(AssetGroup.Audio));

            Assert.Contains(Locations.OwnershipMarker, failure.Message, StringComparison.Ordinal);
        }
    }
}
