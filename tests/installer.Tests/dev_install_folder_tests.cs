using AssetConverter;
using AssetConverter.Pipeline;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests
{
    public sealed class DevInstallFolderTests : IDisposable
    {
        private readonly ArchiveTempDir _repo = new();

        public void Dispose() => _repo.Dispose();

        private string Parent => Path.Combine(_repo.Path, "test_installation");

        private static void MarkAsInstall(string root)
        {
            Directory.CreateDirectory(root);
            File.WriteAllText(Path.Combine(root, InstallLayout.OwnershipMarkerFileName), "");
        }

        [Fact]
        public void WithoutAnInstallTheRootIsTheOpenFo4FolderThatTheWindowWouldCreate()
        {
            Assert.Equal(Path.Combine(Parent, "OpenFO4"), InstallerPaths.DevInstallFolderIn(_repo.Path));
        }

        [Fact]
        public void AnInstallMadeThroughTheWindowIsFoundBelowTheOpenFo4Folder()
        {
            MarkAsInstall(Path.Combine(Parent, "OpenFO4"));

            Assert.Equal(Path.Combine(Parent, "OpenFO4"), InstallerPaths.DevInstallFolderIn(_repo.Path));
        }

        [Fact]
        public void AnInstallDirectlyInTheFolderIsUsedAsItIs()
        {
            MarkAsInstall(Parent);

            Assert.Equal(Parent, InstallerPaths.DevInstallFolderIn(_repo.Path));
        }
    }
}
