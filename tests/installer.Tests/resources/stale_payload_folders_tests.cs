using AssetConverter;
using Xunit;

namespace InstallerTests.Resources
{
    public sealed class StalePayloadFoldersTests : IDisposable
    {
        private const string Id = "0123456789abcdef0123456789abcdef";
        private const string OtherId = "abcdefabcdefabcdefabcdef";

        private readonly TempDirectory _root = new();
        private readonly List<string> _warnings = [];

        public void Dispose() => _root.Dispose();

        private string Cache => Path.Combine(_root.Path, "cache");

        private string Folder(string version, string id = Id) => Path.Combine(Cache, PayloadExtractor.FolderName(version, id));

        private PayloadFolder Unpack(string version = "2.0.0") =>
            PayloadExtractor.Unpack(PayloadExtractorTests.Payload(PayloadExtractorTests.Zip(("a.txt", version))), Cache, version, progress: null, _warnings.Add);

        private void Install(string version, string id = Id) =>
            PayloadExtractor.EnsureExtracted(PayloadExtractorTests.Payload(PayloadExtractorTests.Zip(("a.txt", version)), id), Cache, version);

        [Fact]
        public void UnpackingRemovesTheFoldersOfOtherVersionsAndUnfinishedUnpackingsAndKeepsItsOwn()
        {
            Install("1.0.0");
            Install("1.5.0", OtherId);
            Directory.CreateDirectory(Folder("1.9.0") + ".unpacking");
            Directory.CreateDirectory(Folder("2.0.0") + ".unpacking");

            using var current = Unpack();

            Assert.Equal(Folder("2.0.0"), current.Path);
            Assert.Equal([Folder("2.0.0")], Directory.GetDirectories(Cache));
            Assert.Empty(_warnings);
        }

        [Fact]
        public void ReusingACompleteFolderRemovesTheOldOnesToo()
        {
            using (Unpack())
            {
            }
            Install("1.0.0");

            using var current = Unpack();

            Assert.Equal([Folder("2.0.0")], Directory.GetDirectories(Cache));
        }

        [Fact]
        public void AFolderThatAnotherInstallerHoldsIsKept()
        {
            Install("1.0.0");
            using var otherInstaller = PayloadFolder.Hold(Folder("1.0.0"));

            using var current = Unpack();

            Assert.True(File.Exists(Path.Combine(Folder("1.0.0"), "a.txt")));
            Assert.Empty(_warnings);
        }

        [Fact]
        public async Task AFolderWhoseLockAnotherInstallerHoldsIsKept()
        {
            Install("1.0.0");
            using var locked = new ManualResetEventSlim();
            using var release = new ManualResetEventSlim();
            var holder = Task.Run(() =>
            {
                using var gate = PayloadFolderLock.Acquire(Folder("1.0.0"), TimeSpan.FromSeconds(5));
                locked.Set();
                release.Wait(TimeSpan.FromSeconds(30), TestContext.Current.CancellationToken);
            }, TestContext.Current.CancellationToken);
            Assert.True(locked.Wait(TimeSpan.FromSeconds(5), TestContext.Current.CancellationToken));

            using var current = Unpack();
            release.Set();
            await holder;

            Assert.True(Directory.Exists(Folder("1.0.0")));
        }

        [Fact]
        public void AFolderThatCannotBeDeletedIsAWarningAndDoesNotFailTheUnpacking()
        {
            Install("1.0.0");
            using var openFile = new FileStream(Path.Combine(Folder("1.0.0"), "a.txt"), FileMode.Open, FileAccess.Read, FileShare.None);

            using var current = Unpack();

            Assert.True(Directory.Exists(current.Path));
            string warning = Assert.Single(_warnings);
            Assert.StartsWith($"WARNING: could not remove the old installer files '{Folder("1.0.0")}'", warning, StringComparison.Ordinal);
        }

        [Fact]
        public void FoldersThatAreNoPayloadFoldersAreNeverTouched()
        {
            string foreign = Path.Combine(Cache, "my notes");
            Directory.CreateDirectory(foreign);
            File.WriteAllText(Path.Combine(foreign, "x.txt"), "x");

            using var current = Unpack();

            Assert.True(File.Exists(Path.Combine(foreign, "x.txt")));
        }

        [Fact]
        public void ACacheFolderThatDoesNotExistYetHasNothingToClean()
        {
            StalePayloadFolders.Remove(Path.Combine(_root.Path, "missing"), Folder("2.0.0"), _warnings.Add);

            Assert.Empty(_warnings);
        }
    }
}
