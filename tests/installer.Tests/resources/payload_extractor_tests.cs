using System.IO.Compression;
using AssetConverter;
using Xunit;

namespace InstallerTests.Resources
{
    public sealed class PayloadExtractorTests : IDisposable
    {
        private const string Id = "0123456789abcdef0123456789abcdef";

        private readonly TempDirectory _root = new();

        public void Dispose() => _root.Dispose();

        private string Cache => Path.Combine(_root.Path, "cache");

        internal static byte[] Zip(params (string Path, string Content)[] entries)
        {
            using var stream = new MemoryStream();
            using (var archive = new ZipArchive(stream, ZipArchiveMode.Create, leaveOpen: true))
            {
                foreach (var (path, content) in entries)
                {
                    using var writer = new StreamWriter(archive.CreateEntry(path).Open());
                    writer.Write(content);
                }
            }
            return stream.ToArray();
        }

        internal static PayloadSource Payload(byte[] zip, string id = Id, Action? onOpen = null) =>
            new(() =>
            {
                onOpen?.Invoke();
                return new MemoryStream(zip);
            }, id);

        [Fact]
        public void TheFirstCallUnpacksEveryEntryAndWritesTheCompletionMarker()
        {
            var payload = Payload(Zip(("game/openfo4.exe", "exe"), ("runtime/python/python.exe", "py")));

            string folder = PayloadExtractor.EnsureExtracted(payload, Cache, "1.2.3");

            Assert.Equal("exe", File.ReadAllText(Path.Combine(folder, "game", "openfo4.exe")));
            Assert.Equal("py", File.ReadAllText(Path.Combine(folder, "runtime", "python", "python.exe")));
            Assert.Equal(Id, File.ReadAllText(Path.Combine(folder, PayloadExtractor.CompletionMarker)));
            Assert.False(Directory.Exists(folder + ".unpacking"));
        }

        [Fact]
        public void ACompleteFolderIsReusedWithoutReadingThePayloadAgain()
        {
            int opened = 0;
            var payload = Payload(Zip(("a.txt", "a")), onOpen: () => opened++);
            string first = PayloadExtractor.EnsureExtracted(payload, Cache, "1.2.3");
            File.WriteAllText(Path.Combine(first, "a.txt"), "kept");

            string second = PayloadExtractor.EnsureExtracted(payload, Cache, "1.2.3");

            Assert.Equal(first, second);
            Assert.Equal(1, opened);
            Assert.Equal("kept", File.ReadAllText(Path.Combine(second, "a.txt")));
        }

        [Fact]
        public void AFolderWithoutTheMarkerIsAnInterruptedExtractionAndIsReplaced()
        {
            var payload = Payload(Zip(("a.txt", "a")));
            string folder = Path.Combine(Cache, PayloadExtractor.FolderName("1.2.3", Id));
            Directory.CreateDirectory(folder);
            File.WriteAllText(Path.Combine(folder, "stale.txt"), "left by an earlier run");
            Directory.CreateDirectory(folder + ".unpacking");
            File.WriteAllText(Path.Combine(folder + ".unpacking", "half.txt"), "half");

            string result = PayloadExtractor.EnsureExtracted(payload, Cache, "1.2.3");

            Assert.Equal(folder, result);
            Assert.False(File.Exists(Path.Combine(folder, "stale.txt")));
            Assert.Equal("a", File.ReadAllText(Path.Combine(folder, "a.txt")));
            Assert.True(File.Exists(Path.Combine(folder, PayloadExtractor.CompletionMarker)));
            Assert.False(Directory.Exists(folder + ".unpacking"));
        }

        [Fact]
        public void AMarkerOfAnotherPayloadDoesNotCount()
        {
            string folder = Path.Combine(Cache, PayloadExtractor.FolderName("1.2.3", Id));
            Directory.CreateDirectory(folder);
            File.WriteAllText(Path.Combine(folder, PayloadExtractor.CompletionMarker), "ffffffffffffffffffff");

            PayloadExtractor.EnsureExtracted(Payload(Zip(("a.txt", "fresh"))), Cache, "1.2.3");

            Assert.Equal("fresh", File.ReadAllText(Path.Combine(folder, "a.txt")));
        }

        [Fact]
        public void EveryVersionAndEveryPayloadGetsAFolderOfItsOwn()
        {
            string v1 = PayloadExtractor.EnsureExtracted(Payload(Zip(("a.txt", "1"))), Cache, "1.0.0");
            string v2 = PayloadExtractor.EnsureExtracted(Payload(Zip(("a.txt", "2"))), Cache, "2.0.0");
            string otherContent = PayloadExtractor.EnsureExtracted(Payload(Zip(("a.txt", "3")), id: "abcdefabcdefabcdefabcdef"), Cache, "1.0.0");

            Assert.Equal(3, new[] { v1, v2, otherContent }.Distinct().Count());
            Assert.Equal("1", File.ReadAllText(Path.Combine(v1, "a.txt")));
            Assert.Equal("2", File.ReadAllText(Path.Combine(v2, "a.txt")));
        }

        [Fact]
        public void ACorruptPayloadFailsWithAStartupMessageAndLeavesNoFolderUnderTheFinalName()
        {
            var payload = Payload([1, 2, 3, 4]);

            var error = Assert.Throws<InstallStartupException>(() => PayloadExtractor.EnsureExtracted(payload, Cache, "1.2.3"));

            Assert.Contains("could not unpack", error.Message, StringComparison.Ordinal);
            Assert.False(Directory.Exists(Path.Combine(Cache, PayloadExtractor.FolderName("1.2.3", Id))));
        }

        [Theory]
        [InlineData("")]
        [InlineData("not hex")]
        [InlineData("..\\..\\evil1234")]
        public void APayloadIdThatIsNotAHexHashIsRejectedBeforeItNamesAFolder(string id)
        {
            Assert.Throws<ArgumentException>(() => PayloadExtractor.EnsureExtracted(Payload(Zip(("a.txt", "a")), id), Cache, "1.2.3"));
        }

        [Fact]
        public void AVersionWithCharactersThatCannotNameAFolderIsStillUsable()
        {
            string folder = PayloadExtractor.EnsureExtracted(Payload(Zip(("a.txt", "a"))), Cache, "1.0.0-beta:1");

            Assert.True(File.Exists(Path.Combine(folder, "a.txt")));
        }

        [Fact]
        public async Task ConcurrentCallsUnpackOnceAndAllSeeAFinishedFolder()
        {
            int opened = 0;
            var payload = Payload(Zip(("a.txt", "a")), onOpen: () => Interlocked.Increment(ref opened));

            var results = await Task.WhenAll(Enumerable.Range(0, 4).Select(_ => Task.Run(() => PayloadExtractor.EnsureExtracted(payload, Cache, "1.2.3"), TestContext.Current.CancellationToken)));

            Assert.Single(results.Distinct());
            Assert.Equal(1, opened);
            Assert.True(File.Exists(Path.Combine(results[0], "a.txt")));
        }

        [Fact]
        public void TheUnpackingReportsTheFileCountAndEachFileButNoFolders()
        {
            var counts = new List<long>();
            int unpacked = 0;
            var payload = Payload(Zip(("game/openfo4.exe", "exe"), ("runtime/python/python.exe", "py"), ("empty/", "")));

            PayloadExtractor.EnsureExtracted(payload, Cache, "1.2.3", new UnpackProgress(counts.Add, () => unpacked++));

            Assert.Equal([2L], counts);
            Assert.Equal(2, unpacked);
        }

        [Fact]
        public void AnEntryThatLeavesTheFolderIsRefusedAndNothingIsWrittenOutside()
        {
            var payload = Payload(Zip(("../escaped.txt", "x")));

            var error = Assert.Throws<InstallStartupException>(() => PayloadExtractor.EnsureExtracted(payload, Cache, "1.2.3"));

            Assert.Contains("outside the folder", error.Message, StringComparison.Ordinal);
            Assert.False(File.Exists(Path.Combine(Cache, "escaped.txt")));
        }
    }
}
