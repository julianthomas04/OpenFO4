using AssetConverter;
using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests
{
    public sealed class InstallFolderPreparationTests : IDisposable
    {
        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private InstallLocations Locations(string installFolder = "install") => new(Path.Combine(_root.Path, "game"), Path.Combine(_root.Path, installFolder));

        [Fact]
        public void ANewInstallFolderIsCreatedWithItsOutputFoldersAndTheOwnershipMarker()
        {
            var locations = Locations(Path.Combine("deep", "install"));

            Assert.Null(InstallFolderPreparation.Prepare(locations));

            Assert.True(File.Exists(locations.OwnershipMarker));
            Assert.True(Directory.Exists(locations.MeshesOut));
            Assert.True(Directory.Exists(locations.StagingRoot));
        }

        [Fact]
        public void AFolderPreparedOnceIsAnInstallFolderForTheNextRun()
        {
            var locations = Locations();
            InstallFolderPreparation.Prepare(locations);
            File.WriteAllText(locations.LogFile, "log");

            Assert.Null(InstallFolderPreparation.Prepare(locations));
            Assert.True(locations.HoldsInstall());
        }

        [Fact]
        public void AFolderWithForeignContentIsRefusedAndLeftUntouched()
        {
            var locations = Locations();
            Directory.CreateDirectory(locations.InstallRoot);
            string foreign = Path.Combine(locations.InstallRoot, "photos.jpg");
            File.WriteAllText(foreign, "x");

            Assert.Equal($"{locations.InstallRoot} already contains other files; choose an empty folder or a previous OpenFO4 install.", InstallFolderPreparation.Prepare(locations));

            Assert.False(File.Exists(locations.OwnershipMarker));
            Assert.False(Directory.Exists(locations.StagingRoot));
            Assert.True(File.Exists(foreign));
        }

        [Fact]
        public void AFolderThatCannotBeCreatedIsReportedWithItsPathInsteadOfThrowing()
        {
            string blocker = Path.Combine(_root.Path, "blocker");
            File.WriteAllText(blocker, "a file, not a folder");
            var locations = new InstallLocations(Path.Combine(_root.Path, "game"), Path.Combine(blocker, "install"));

            string? error = InstallFolderPreparation.Prepare(locations);

            Assert.StartsWith($"Cannot create the install folder '{locations.InstallRoot}'", error, StringComparison.Ordinal);
        }

        [Fact]
        public void AnExistingInstallFromBeforeTheMarkerIsAcceptedAndMarked()
        {
            var locations = Locations();
            Directory.CreateDirectory(locations.DataOut);
            File.WriteAllText(locations.LogFile, "log");
            File.WriteAllText(locations.DatabasePath, "db");

            Assert.Null(InstallFolderPreparation.Prepare(locations));

            Assert.True(File.Exists(locations.OwnershipMarker));
            Assert.True(File.Exists(locations.DatabasePath));
        }

        [Fact]
        public void AnInstalledGameThatIsRunningIsReportedBeforeAnythingInTheFolderChanges()
        {
            var locations = Locations();
            Assert.Null(InstallFolderPreparation.Prepare(locations));
            locations.MarkAssetsComplete("1.0.0");
            File.WriteAllText(locations.GameExecutable, "exe");
            using var running = new FileStream(locations.GameExecutable, FileMode.Open, FileAccess.Read, FileShare.Read);

            string? error = InstallFolderPreparation.Prepare(locations);

            Assert.Equal($"Close OpenFO4 before reinstalling: '{locations.GameExecutable}' is in use.", error);
            Assert.True(AssetVersionMarker.TryRead(locations.OwnershipMarker, out var version));
            Assert.Equal("1.0.0", version);
        }

        [Fact]
        public void AnInstalledGameThatIsNotRunningDoesNotBlockTheInstallAndIsNotModified()
        {
            var locations = Locations();
            Assert.Null(InstallFolderPreparation.Prepare(locations));
            File.WriteAllText(locations.GameExecutable, "exe");
            DateTime written = File.GetLastWriteTimeUtc(locations.GameExecutable);

            Assert.Null(InstallFolderPreparation.Prepare(locations));

            Assert.Equal("exe", File.ReadAllText(locations.GameExecutable));
            Assert.Equal(written, File.GetLastWriteTimeUtc(locations.GameExecutable));
        }
    }
}
