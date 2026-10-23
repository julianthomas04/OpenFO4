using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public sealed class InstallerPathsTests : IDisposable
    {
        const string Marker = "src/installer/installer.csproj";

        readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        string MakeRepoRoot(string relativeDirectory)
        {
            string root = Path.Combine(_root.Path, relativeDirectory);
            string marker = Path.Combine(root, Marker);
            Directory.CreateDirectory(Path.GetDirectoryName(marker)!);
            File.WriteAllText(marker, "<Project />");
            return root;
        }

        string MakeDirectory(string relativeDirectory)
        {
            string path = Path.Combine(_root.Path, relativeDirectory);
            Directory.CreateDirectory(path);
            return path;
        }

        [Fact]
        public void FindRepoRoot_StartDeepInsideRepo_ReturnsTheDirectoryHoldingTheMarker()
        {
            string root = MakeRepoRoot("repo");
            string start = MakeDirectory(Path.Combine("repo", "tests", "bin", "Debug"));

            Assert.Equal(root, InstallerPaths.FindRepoRoot([start]));
        }

        [Fact]
        public void FindRepoRoot_StartIsTheRoot_ReturnsIt()
        {
            string root = MakeRepoRoot("repo");

            Assert.Equal(root, InstallerPaths.FindRepoRoot([root]));
        }

        [Fact]
        public void FindRepoRoot_FirstStartOutsideRepo_FallsBackToTheNextStart()
        {
            string root = MakeRepoRoot("repo");
            string outside = MakeDirectory("elsewhere");
            string inside = MakeDirectory(Path.Combine("repo", "src"));

            Assert.Equal(root, InstallerPaths.FindRepoRoot([outside, inside]));
        }

        [Fact]
        public void FindRepoRoot_BothStartsInsideDifferentRepos_PrefersTheFirstStart()
        {
            string first = MakeRepoRoot("first");
            MakeRepoRoot("second");

            Assert.Equal(first, InstallerPaths.FindRepoRoot([Path.Combine(first, "src"), Path.Combine(_root.Path, "second")]));
        }

        [Fact]
        public void FindRepoRoot_NestedRepos_ReturnsTheNearestAncestor()
        {
            MakeRepoRoot("outer");
            string inner = MakeRepoRoot(Path.Combine("outer", "vendor", "inner"));

            Assert.Equal(inner, InstallerPaths.FindRepoRoot([Path.Combine(inner, "src")]));
        }

        [Fact]
        public void FindRepoRoot_NoMarkerAnywhere_ThrowsNamingTheStartAndTheMarker()
        {
            string start = MakeDirectory("empty");

            var error = Assert.Throws<DirectoryNotFoundException>(() => InstallerPaths.FindRepoRoot([start]));

            Assert.Contains(start, error.Message);
            Assert.Contains(Marker, error.Message);
        }

        [Fact]
        public void FindRepoRoot_MarkerPathIsADirectory_IsNotAMatch()
        {
            string fake = MakeDirectory(Path.Combine("repo", "src", "installer", "installer.csproj"));

            Assert.Throws<DirectoryNotFoundException>(() => InstallerPaths.FindRepoRoot([fake]));
        }

        [Fact]
        public void Require_ExistingRepoResource_ReturnsItsAbsolutePath()
        {
            string path = InstallerPaths.Require("src/installer/file_converters/esm_schema.sql");

            Assert.True(Path.IsPathRooted(path));
            Assert.True(File.Exists(path));
        }

        [Fact]
        public void Require_MissingResource_ThrowsFileNotFoundNamingTheResource()
        {
            var error = Assert.Throws<FileNotFoundException>(() => InstallerPaths.Require("src/installer/does_not_exist.sql"));

            Assert.Contains("src/installer/does_not_exist.sql", error.Message);
        }
    }
}
