using AssetConverter.Pipeline;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests.Pipeline
{
    public sealed class StagedPathSpellingTests : IDisposable
    {
        private readonly ArchiveTempDir _root = new();

        public void Dispose() => _root.Dispose();

        private void Stage(string relative)
        {
            string path = Path.Combine(_root.Path, relative);
            Directory.CreateDirectory(Path.GetDirectoryName(path)!);
            File.WriteAllText(path, "x");
        }

        [Fact]
        public void EveryFolderAndTheFileComeBackSpelledAsTheyAreOnDisk()
        {
            Stage(@"Meshes\Actors\Dog\Skeleton.nif");

            string resolved = new StagedPathSpelling(_root.Path).Resolve(@"meshes\ACTORS\dog\SKELETON.NIF");

            Assert.Equal(Path.Combine(_root.Path, "Meshes", "Actors", "Dog", "Skeleton.nif"), resolved);
        }

        [Fact]
        public void FilesOfOneFolderGetTheSameFolderSpellingWhateverSpellingTheyAreAskedWith()
        {
            Stage(@"Meshes\Actors\Dog\a.nif");
            Stage(@"Meshes\Actors\Dog\b.nif");
            var spelling = new StagedPathSpelling(_root.Path);

            string first = spelling.Resolve(@"meshes\actors\dog\a.nif");
            string second = spelling.Resolve(@"MESHES\ACTORS\DOG\b.nif");

            Assert.Equal(Path.GetDirectoryName(first), Path.GetDirectoryName(second));
            Assert.Equal(Path.Combine(_root.Path, "Meshes", "Actors", "Dog"), Path.GetDirectoryName(first));
        }

        [Fact]
        public void ForwardSlashesAreAccepted()
        {
            Stage(@"Meshes\Props\crate.nif");

            Assert.Equal(Path.Combine(_root.Path, "Meshes", "Props", "crate.nif"), new StagedPathSpelling(_root.Path).Resolve("meshes/props/crate.nif"));
        }

        [Fact]
        public void AFileCreatedAfterItsFolderWasFirstLookedUpKeepsTheSpellingItWasCreatedWith()
        {
            Stage(@"Meshes\Props\a.nif");
            var spelling = new StagedPathSpelling(_root.Path);
            spelling.Resolve(@"meshes\props\a.nif");
            Stage(@"Meshes\Props\New.nif");

            Assert.Equal(Path.Combine(_root.Path, "Meshes", "Props", "New.nif"), spelling.Resolve(@"Meshes\Props\New.nif"));
        }

        [Fact]
        public void AFileAnEarlierRunLeftInAnotherCaseKeepsItsOwnSpellingWhateverTheSpellingAskedFor()
        {
            Stage(@"Meshes\Props\Shared.nif");
            var spelling = new StagedPathSpelling(_root.Path);

            Assert.Equal(Path.Combine(_root.Path, "Meshes", "Props", "Shared.nif"), spelling.Resolve(@"MESHES\PROPS\SHARED.NIF"));
            Assert.Equal(Path.Combine(_root.Path, "Meshes", "Props", "Shared.nif"), spelling.Resolve(@"meshes\props\shared.nif"));
        }

        [Fact]
        public void FilesDirectlyBelowTheRootAreResolved()
        {
            Stage("Fallout4.esm");

            Assert.Equal(Path.Combine(_root.Path, "Fallout4.esm"), new StagedPathSpelling(_root.Path).Resolve("FALLOUT4.ESM"));
        }

        [Fact]
        public void ManyThreadsResolvingFilesOfVariantSpellingsAllGetTheOnDiskFolderAndFileSpelling()
        {
            for (int i = 0; i < 40; i++) Stage($@"Meshes\Actors\Dog\file{i}.nif");
            var spelling = new StagedPathSpelling(_root.Path);

            var resolved = Enumerable.Range(0, 40).AsParallel().Select(i =>
                spelling.Resolve(i % 2 == 0 ? $@"MESHES\actors\DOG\FILE{i}.NIF" : $"meshes/Actors/dog/file{i}.nif")).ToArray();

            Assert.Equal(Enumerable.Range(0, 40).Select(i => Path.Combine(_root.Path, "Meshes", "Actors", "Dog", $"file{i}.nif")).Order(), resolved.Order());
        }

        [Fact]
        public void AFileThatIsNotStagedIsReportedInsteadOfGuessed()
        {
            Stage(@"Meshes\Props\crate.nif");
            var spelling = new StagedPathSpelling(_root.Path);

            Assert.Throws<FileNotFoundException>(() => spelling.Resolve(@"Meshes\Props\missing.nif"));
            Assert.Throws<FileNotFoundException>(() => spelling.Resolve(@"Meshes\Nowhere\crate.nif"));
        }
    }
}
