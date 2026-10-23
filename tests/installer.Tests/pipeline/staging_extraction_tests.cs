using System.Collections.Concurrent;
using AssetConverter.Logging;
using AssetConverter.Pipeline;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests.Pipeline
{
    /// <summary>Drives the process-wide InstallProgress and prints to the console, so it runs serially with the other tests that do.</summary>
    [Collection("Console redirection")]
    public sealed class StagingExtractionTests : IDisposable
    {
        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private readonly ArchiveTempDir _game = new();
        private readonly ArchiveTempDir _staging = new();

        public void Dispose()
        {
            _game.Dispose();
            _staging.Dispose();
        }

        private static Ba2File Entry(string name, int length, int seed) => new(name, TestBytes.Random(length, seed), Compress: seed % 2 == 0);

        private void WriteArchive(string name, params Ba2File[] files) => _game.Write(name, Ba2Builder.Gnrl(files));

        private static StagingSource Source(string name, Func<string, bool>? includes = null) => new(name, $".{name}.extracted", includes ?? (_ => true));

        private StagingExtraction Open(params StagingSource[] sources) => StagingExtraction.Open(_game.Path, _staging.Path, sources, MemoryBudget.Unlimited());

        private static Dictionary<string, byte[]> ReadTree(string root) =>
            Directory.EnumerateFiles(root, "*", SearchOption.AllDirectories)
                .Where(file => !Path.GetFileName(file).StartsWith('.'))
                .ToDictionary(file => Path.GetRelativePath(root, file), File.ReadAllBytes);

        private static Dictionary<string, byte[]> Lowercased(Dictionary<string, byte[]> tree) =>
            tree.ToDictionary(pair => pair.Key.ToLowerInvariant(), pair => pair.Value);

        [Fact]
        public async Task TheExtractionWaitsForTheMemoryBudgetAndWritesEveryFileOnceItIsReleased()
        {
            WriteArchive("a.ba2", Entry(@"meshes\x.nif", 700, 1), Entry(@"meshes\y.nif", 100, 2));
            var memory = new MemoryBudget(20_000);
            var other = await memory.ReserveAsync(memory.Capacity, Ct);
            using var extraction = StagingExtraction.Open(_game.Path, _staging.Path, [Source("a.ba2")], memory);

            var run = Task.Run(() => extraction.Run(-1, Ct), Ct);
            Assert.True(SpinWait.SpinUntil(() => memory.WaitingCount == 1, TimeSpan.FromSeconds(30)));
            Assert.False(run.IsCompleted);
            other.Dispose();

            Assert.True(await run.WaitAsync(TimeSpan.FromSeconds(30), Ct));
            Assert.Equal(2, ReadTree(_staging.Path).Count);
            Assert.Equal(0, memory.ReservedBytes);
        }

        [Fact]
        public void TheOutputIsWhatExtractingTheArchivesOneAfterAnotherLeavesAndEveryFileIsWrittenOnce()
        {
            WriteArchive("a.ba2", Entry(@"meshes\x.nif", 700, 1), Entry(@"meshes\only_a.nif", 100, 2), Entry(@"materials\m.bgsm", 50, 3));
            WriteArchive("b.ba2", Entry(@"Meshes\X.NIF", 900, 4), Entry(@"meshes\only_b.nif", 60, 5));
            using var sequential = new ArchiveTempDir();
            foreach (string name in new[] { "a.ba2", "b.ba2" })
                AssetConverter.BA2ParallelUnpacker.Unpack(Path.Combine(_game.Path, name), sequential.Path, MemoryBudget.Unlimited(), quiet: true, ct: Ct);
            var written = new ConcurrentBag<string>();
            using var extraction = Open(Source("a.ba2") with { OnFileWritten = written.Add }, Source("b.ba2") with { OnFileWritten = written.Add });

            bool allSucceeded = extraction.Run(-1, Ct);

            Assert.True(allSucceeded);
            Assert.Equal(4, extraction.WrittenFiles.Count);
            Assert.Equal(4, written.Count);
            Assert.Equal(4, written.Select(path => path.ToLowerInvariant()).Distinct().Count());
            Assert.Equal(Lowercased(ReadTree(sequential.Path)), Lowercased(ReadTree(_staging.Path)));
        }

        [Fact]
        public void AFileIsReportedOnlyOnceItIsCompleteAndClosed()
        {
            WriteArchive("a.ba2", Entry(@"meshes\one.nif", 6000, 2), Entry(@"meshes\two.nif", 700, 3), Entry(@"meshes\three.nif", 90, 4));
            var contents = new ConcurrentDictionary<string, byte[]>();
            using var extraction = Open(Source("a.ba2") with
            {
                OnFileWritten = relative =>
                {
                    using var exclusive = new FileStream(Path.Combine(_staging.Path, relative), FileMode.Open, FileAccess.ReadWrite, FileShare.None);
                    var content = new byte[exclusive.Length];
                    exclusive.ReadExactly(content);
                    contents[relative] = content;
                },
            });

            extraction.Run(-1, Ct);

            Assert.Equal(TestBytes.Random(6000, 2), contents[@"meshes\one.nif"]);
            Assert.Equal(TestBytes.Random(700, 3), contents[@"meshes\two.nif"]);
            Assert.Equal(TestBytes.Random(90, 4), contents[@"meshes\three.nif"]);
        }

        [Fact]
        public void TheSourceFilterRestrictsWhatIsWrittenAndCounted()
        {
            WriteArchive("a.ba2", Entry(@"meshes\actors\a.hkx", 70, 1), Entry(@"meshes\actors\a.txt", 70, 2), Entry(@"meshes\other\b.hkx", 70, 3));
            using var extraction = Open(Source("a.ba2", path => path.StartsWith(@"meshes\actors\", StringComparison.Ordinal) && path.EndsWith(".hkx", StringComparison.Ordinal)));

            extraction.Run(-1, Ct);

            Assert.Equal([@"meshes\actors\a.hkx"], extraction.WrittenFiles);
            Assert.Equal([@"meshes\actors\a.hkx"], ReadTree(_staging.Path).Keys);
        }

        [Fact]
        public void AnArchiveOnlyGetsItsDoneMarkerWhenEveryEntryExtractedCleanly()
        {
            WriteArchive("clean.ba2", Entry(@"a\one.nif", 500, 1));
            var damaged = Ba2Builder.Gnrl([Entry(@"b\two.nif", 6000, 2), Entry(@"b\three.nif", 700, 3)], nameTable: true);
            for (int i = 24 + 2 * 36 + 4; i < 24 + 2 * 36 + 4 + 16; i++)
                damaged[i] ^= 0xFF;
            _game.Write("damaged.ba2", damaged);
            using var extraction = Open(Source("clean.ba2"), Source("damaged.ba2"));

            bool allSucceeded = extraction.Run(-1, Ct);

            Assert.False(allSucceeded);
            Assert.True(File.Exists(Path.Combine(_staging.Path, ".clean.ba2.extracted")));
            Assert.False(File.Exists(Path.Combine(_staging.Path, ".damaged.ba2.extracted")));
        }

        [Fact]
        public void AnArchiveListingOneFileTwiceWritesAndReportsItOnceWithTheLastEntryWinning()
        {
            WriteArchive("a.ba2", Entry(@"meshes\dup.nif", 700, 1), Entry(@"Meshes\DUP.nif", 900, 2), Entry(@"meshes\other.nif", 50, 3));
            var written = new ConcurrentBag<string>();
            using var extraction = Open(Source("a.ba2") with { OnFileWritten = written.Add });

            bool allSucceeded = extraction.Run(-1, Ct);

            Assert.True(allSucceeded);
            Assert.Equal(2, extraction.WrittenFiles.Count);
            Assert.Equal(2, written.Count);
            Assert.Equal(TestBytes.Random(900, 2), File.ReadAllBytes(Path.Combine(_staging.Path, "meshes", "dup.nif")));
            Assert.True(File.Exists(Path.Combine(_staging.Path, ".a.ba2.extracted")));
        }

        [Fact]
        public void AFileThatCouldNotBeHandedOnFailsTheRunAndWithholdsTheMarker()
        {
            WriteArchive("a.ba2", Entry(@"meshes\one.nif", 50, 1), Entry(@"meshes\two.nif", 60, 2));
            bool finished = false;
            using var extraction = Open(Source("a.ba2") with
            {
                OnFileWritten = path => throw new InvalidOperationException($"cannot queue {path}"),
                OnFinished = () => finished = true,
            });

            var thrown = Assert.Throws<InvalidOperationException>(() => extraction.Run(-1, Ct));

            Assert.Contains("a.ba2", thrown.Message);
            Assert.Contains("2 extracted file(s)", thrown.Message);
            Assert.StartsWith("cannot queue", thrown.InnerException!.Message);
            Assert.False(File.Exists(Path.Combine(_staging.Path, ".a.ba2.extracted")));
            Assert.False(finished);
        }

        [Fact]
        public void AWarmRunWritesNothingAndStillReportsEverySourceAsFinishedInOrder()
        {
            WriteArchive("a.ba2", Entry(@"meshes\one.nif", 50, 1));
            WriteArchive("b.ba2", Entry(@"meshes\two.nif", 50, 2));
            File.WriteAllText(Path.Combine(_staging.Path, ".a.ba2.extracted"), "");
            File.WriteAllText(Path.Combine(_staging.Path, ".b.ba2.extracted"), "");
            var order = new List<string>();
            using var extraction = Open(
                Source("a.ba2") with { OnFileWritten = _ => order.Add("write"), OnFinished = () => order.Add("a done") },
                Source("b.ba2") with { OnFileWritten = _ => order.Add("write"), OnFinished = () => order.Add("b done") });

            bool allSucceeded = extraction.Run(-1, Ct);

            Assert.True(allSucceeded);
            Assert.Empty(extraction.WrittenFiles);
            Assert.Equal(["a done", "b done"], order);
            Assert.Empty(ReadTree(_staging.Path));
        }

        [Fact]
        public void AMixedRunExtractsOnlyThePendingArchivesAndAnnouncesOnlyTheirFiles()
        {
            WriteArchive("a.ba2", Entry(@"meshes\one.nif", 50, 1));
            WriteArchive("b.ba2", Entry(@"meshes\two.nif", 50, 2));
            File.WriteAllText(Path.Combine(_staging.Path, ".a.ba2.extracted"), "");
            using var extraction = Open(Source("a.ba2"), Source("b.ba2"));

            extraction.Run(-1, Ct);

            Assert.Equal([@"meshes\two.nif"], extraction.WrittenFiles);
            Assert.Equal([@"meshes\two.nif"], ReadTree(_staging.Path).Keys);
        }

        [Fact]
        public void AMarkedArchiveDoesNotTakePartInTheOverlapSoThePendingOneWritesTheSharedFile()
        {
            WriteArchive("a.ba2", Entry(@"meshes\shared.nif", 50, 1));
            WriteArchive("b.ba2", Entry(@"meshes\shared.nif", 60, 2));
            File.WriteAllText(Path.Combine(_staging.Path, ".b.ba2.extracted"), "");
            using var extraction = Open(Source("a.ba2"), Source("b.ba2"));

            extraction.Run(-1, Ct);

            Assert.Equal([@"meshes\shared.nif"], extraction.WrittenFiles);
            Assert.Equal(TestBytes.Random(50, 1), File.ReadAllBytes(Path.Combine(_staging.Path, "meshes", "shared.nif")));
        }

        [Fact]
        public void AMissingArchiveIsReportedWithItsOwnMessageAndTheRestStillRuns()
        {
            WriteArchive("b.ba2", Entry(@"meshes\two.nif", 50, 2));
            var order = new List<string>();
            var log = new LogBuffer(100);
            using var capture = ConsoleCapture.Start(log);
            using var extraction = Open(
                Source("missing.ba2") with { MissingArchiveMessage = path => $"CUSTOM {Path.GetFileName(path)}", OnFinished = () => order.Add("missing done") },
                Source("b.ba2") with { OnFinished = () => order.Add("b done") });

            bool allSucceeded = extraction.Run(-1, Ct);

            Assert.True(allSucceeded);
            Assert.Contains(log.ReadAfter(0).Lines, line => line.IsError && line.Text == "CUSTOM missing.ba2");
            Assert.Equal(["missing done", "b done"], order);
            Assert.Single(ReadTree(_staging.Path));
        }

        [Fact]
        public void AFinishedHookRunsBeforeTheNextArchiveIsExtractedAndAfterItsOwnFilesAreStaged()
        {
            WriteArchive("a.ba2", Entry(@"materials\m.bgsm", 50, 1));
            WriteArchive("b.ba2", Entry(@"meshes\one.nif", 50, 2));
            bool bExistedWhenAFinished = true, aExistedWhenAFinished = false;
            using var extraction = Open(
                Source("a.ba2") with
                {
                    OnFinished = () =>
                    {
                        aExistedWhenAFinished = File.Exists(Path.Combine(_staging.Path, "materials", "m.bgsm"));
                        bExistedWhenAFinished = File.Exists(Path.Combine(_staging.Path, "meshes", "one.nif"));
                    },
                },
                Source("b.ba2"));

            extraction.Run(-1, Ct);

            Assert.True(aExistedWhenAFinished);
            Assert.False(bExistedWhenAFinished);
        }

        private static bool IsMaterial(string path) => path.StartsWith(@"materials\", StringComparison.OrdinalIgnoreCase);

        [Fact]
        public void WhenWrittenFiresAfterTheLastArchiveThatHoldsAnInputAndBeforeTheArchivesAfterIt()
        {
            WriteArchive("a.ba2", Entry(@"materials\a.bgsm", 50, 1));
            WriteArchive("b.ba2", Entry(@"meshes\one.nif", 50, 2), Entry(@"materials\b.bgsm", 50, 3));
            WriteArchive("c.ba2", Entry(@"meshes\two.nif", 50, 4));
            using var extraction = Open(Source("a.ba2"), Source("b.ba2"), Source("c.ba2"));
            string[] staged = [];
            extraction.WhenWritten(IsMaterial, "a.ba2", () => staged = [.. ReadTree(_staging.Path).Keys.Order()]);

            extraction.Run(-1, Ct);

            Assert.Equal([@"materials\a.bgsm", @"materials\b.bgsm", @"meshes\one.nif"], staged);
        }

        [Fact]
        public void AnInputAnArchiveHoldsButALaterArchiveOverwritesIsWaitedForUntilTheLaterArchive()
        {
            WriteArchive("a.ba2", Entry(@"materials\m.bgsm", 50, 1));
            WriteArchive("b.ba2", Entry(@"meshes\one.nif", 50, 2));
            WriteArchive("c.ba2", Entry(@"Materials\M.BGSM", 70, 3));
            using var extraction = Open(Source("a.ba2"), Source("b.ba2"), Source("c.ba2"));
            byte[]? contentWhenReady = null;
            extraction.WhenWritten(IsMaterial, "a.ba2", () => contentWhenReady = File.ReadAllBytes(Path.Combine(_staging.Path, "materials", "m.bgsm")));

            extraction.Run(-1, Ct);

            Assert.Equal(TestBytes.Random(70, 3), contentWhenReady);
        }

        [Fact]
        public void WhenWrittenWaitsForTheNamedArchiveEvenIfItHoldsNoInputAndEvenIfItIsSkipped()
        {
            WriteArchive("a.ba2", Entry(@"materials\a.bgsm", 50, 1));
            WriteArchive("b.ba2", Entry(@"meshes\one.nif", 50, 2));
            WriteArchive("c.ba2", Entry(@"meshes\two.nif", 50, 3));
            File.WriteAllText(Path.Combine(_staging.Path, ".b.ba2.extracted"), "");
            var order = new List<string>();
            using var extraction = Open(Source("a.ba2"), Source("b.ba2") with { OnFinished = () => order.Add("b done") }, Source("c.ba2"));
            extraction.WhenWritten(IsMaterial, "b.ba2", () => order.Add("ready"));

            extraction.Run(-1, Ct);

            Assert.Equal(["b done", "ready"], order);
        }

        [Fact]
        public void WhenWrittenNamingAnArchiveThatIsNotExtractedIsRejected()
        {
            WriteArchive("a.ba2", Entry(@"materials\a.bgsm", 50, 1));
            using var extraction = Open(Source("a.ba2"));

            Assert.Throws<ArgumentException>(() => extraction.WhenWritten(IsMaterial, "other.ba2", () => { }));
        }

        [Fact]
        public void TheCallbacksOnlySeeTheFilesTheSourceWatchesAndTheContentObserverIsOnlyAskedAboutThose()
        {
            WriteArchive("a.ba2", Entry(@"meshes\one.nif", 50, 1), Entry(@"meshes\notes.txt", 50, 2), Entry(@"scripts\three.pex", 50, 3));
            var written = new ConcurrentBag<string>();
            var asked = new ConcurrentBag<string>();
            using var extraction = Open(Source("a.ba2") with
            {
                Watches = path => path.EndsWith(".nif", StringComparison.OrdinalIgnoreCase),
                OnFileWritten = written.Add,
                ObserveContent = path =>
                {
                    asked.Add(path);
                    return null;
                },
            });

            extraction.Run(-1, Ct);

            Assert.Equal([@"meshes\one.nif"], written);
            Assert.Equal([@"meshes\one.nif"], asked);
            Assert.Equal(3, ReadTree(_staging.Path).Count);
        }

        [Fact]
        public void CancellingBeforeTheRunThrowsAndLeavesNoMarker()
        {
            WriteArchive("a.ba2", Entry(@"meshes\one.nif", 50, 1));
            using var cts = new CancellationTokenSource();
            cts.Cancel();
            using var extraction = Open(Source("a.ba2"));

            Assert.ThrowsAny<OperationCanceledException>(() => extraction.Run(-1, cts.Token));

            Assert.False(File.Exists(Path.Combine(_staging.Path, ".a.ba2.extracted")));
        }
    }
}
