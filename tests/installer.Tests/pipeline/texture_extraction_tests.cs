using AssetConverter;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using InstallerTests.Archives;
using Xunit;

namespace InstallerTests.Pipeline
{
    /// <summary>Drives the process-wide InstallProgress and prints to the console, so it runs serially with the other tests that do.</summary>
    [Collection("Console redirection")]
    public sealed class TextureExtractionTests : IDisposable
    {
        private const string Step = "Extracting textures";
        private const long DefaultBudget = BA2ParallelUnpacker.DefaultMaxInFlightBytes;
        private static readonly TimeSpan Generous = TimeSpan.FromSeconds(30);

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private readonly ArchiveTempDir _game = new();
        private readonly ArchiveTempDir _staging = new();
        private readonly int _phase;

        public TextureExtractionTests()
        {
            InstallProgress.Start([new StepDefinition(Step)]);
            _phase = InstallProgress.BeginPhase(Step, 0);
        }

        public void Dispose()
        {
            InstallProgress.Stop();
            _game.Dispose();
            _staging.Dispose();
        }

        private string Out => Path.Combine(_staging.Path, "textures");

        private static Ba2File Texture(string name, int length, int seed, bool compress = false) =>
            new($@"textures\{name}", TestBytes.Random(length, seed), compress);

        private void WriteArchive(string name, params Ba2File[] files) => _game.Write(name, Ba2Builder.Gnrl(files));

        private TextureArchiveSet Open(params string[] names) => TextureArchiveSet.Open(_game.Path, _staging.Path, names);

        private static StepSnapshot Snapshot() => InstallProgress.Model!.GetSnapshot().Steps.Single(s => s.Name == Step);

        private static Dictionary<string, byte[]> ReadTree(string root) =>
            Directory.EnumerateFiles(root, "*", SearchOption.AllDirectories)
                .ToDictionary(file => Path.GetRelativePath(root, file), File.ReadAllBytes);

        private static BA2ParallelUnpacker.Result Succeeded(int files = 0) => new(files, files, 0, 0);

        private static Func<TextureUnpackJob, CancellationToken, BA2ParallelUnpacker.Result> Recording(
            List<string> started, Func<TextureUnpackJob, CancellationToken, BA2ParallelUnpacker.Result> behaviour) => (job, ct) =>
        {
            lock (started) started.Add(Path.GetFileName(job.Archive.FullPath));
            return behaviour(job, ct);
        };

        [Fact]
        public void ATextureInSeveralArchivesIsWrittenOnlyByTheLastArchiveAndTheOutputMatchesSequentialExtraction()
        {
            WriteArchive("t1.ba2", Texture(@"wall\brick_d.dds", 900, 1), Texture(@"wall\only1.dds", 100, 2), Texture(@"sky\clouds.dds", 300, 3));
            WriteArchive("t2.ba2", Texture(@"wall\brick_d.dds", 50, 4), Texture(@"wall\only2.dds", 60, 5));
            WriteArchive("t3.ba2", Texture(@"wall\brick_d.dds", 700, 6), Texture(@"sky\clouds.dds", 20, 7), Texture(@"floor\tile.dds", 400, 8));
            string[] names = ["t1.ba2", "t2.ba2", "t3.ba2"];
            string sequentialOut = Path.Combine(_staging.Path, "sequential");
            foreach (var name in names)
                BA2ParallelUnpacker.Unpack(Path.Combine(_game.Path, name), sequentialOut, MemoryBudget.Unlimited(), quiet: true, stripFirstSegment: "textures", ct: Ct);
            using var set = Open(names);
            var extraction = new TextureExtraction(set, MemoryBudget.Unlimited());

            bool allSucceeded = extraction.Run(Out, _phase, DefaultBudget, Ct);

            Assert.True(allSucceeded);
            Assert.Equal(ReadTree(sequentialOut), ReadTree(Out));
            Assert.Equal(TestBytes.Random(700, 6), File.ReadAllBytes(Path.Combine(Out, "wall", "brick_d.dds")));
        }

        [Fact]
        public void TheFileCountAndProgressTotalCountEachOverlappingTextureOnce()
        {
            WriteArchive("t1.ba2", Texture(@"a\x.dds", 10, 1), Texture(@"a\y.dds", 10, 2));
            WriteArchive("t2.ba2", Texture(@"A\X.DDS", 10, 3), Texture(@"b\z.dds", 10, 4));
            using var set = Open("t1.ba2", "t2.ba2");
            var extraction = new TextureExtraction(set, MemoryBudget.Unlimited());
            InstallProgress.AddTotal(_phase, extraction.FileCount);

            extraction.Run(Out, _phase, DefaultBudget, Ct);

            Assert.Equal(3, extraction.FileCount);
            Assert.Equal(3, Snapshot().Total);
            Assert.Equal(3, Snapshot().Completed);
        }

        [Fact]
        public void EveryExtractedArchiveGetsItsOwnDoneMarkerAndOnlyCleanArchivesDo()
        {
            WriteArchive("clean.ba2", Texture(@"a\one.dds", 500, 1));
            var damaged = Ba2Builder.Gnrl([Texture(@"b\two.dds", 6000, 2, compress: true), Texture(@"b\three.dds", 700, 3)], nameTable: true);
            for (int i = 24 + 2 * 36 + 4; i < 24 + 2 * 36 + 4 + 16; i++)
                damaged[i] ^= 0xFF;
            _game.Write("damaged.ba2", damaged);
            using var set = Open("clean.ba2", "damaged.ba2");

            bool allSucceeded = new TextureExtraction(set, MemoryBudget.Unlimited()).Run(Out, _phase, DefaultBudget, Ct);

            Assert.False(allSucceeded);
            Assert.True(File.Exists(Path.Combine(_staging.Path, ".clean.ba2.extracted-direct")));
            Assert.False(File.Exists(Path.Combine(_staging.Path, ".damaged.ba2.extracted-direct")));
        }

        [Fact]
        public async Task ExtractionWithABudgetSmallerThanEveryPayloadStillFinishesAndWritesIdenticalBytes()
        {
            WriteArchive("t1.ba2", Texture(@"a\one.dds", 5000, 1), Texture(@"a\two.dds", 5000, 2));
            WriteArchive("t2.ba2", Texture(@"b\three.dds", 5000, 3), Texture(@"b\four.dds", 5000, 4));
            using var set = Open("t1.ba2", "t2.ba2");
            var extraction = new TextureExtraction(set, MemoryBudget.Unlimited());

            await Task.Run(() => extraction.Run(Out, _phase, maxInFlightBytes: 1, Ct), Ct).WaitAsync(Generous, Ct);

            Assert.Equal(TestBytes.Random(5000, 3), File.ReadAllBytes(Path.Combine(Out, "b", "three.dds")));
            Assert.Equal(4, ReadTree(Out).Count);
        }

        [Fact]
        public void BothStreamsDrawFromOneBudgetThatIsTheCallersCap()
        {
            WriteArchive("t1.ba2", Texture(@"a\one.dds", 10, 1));
            WriteArchive("t2.ba2", Texture(@"b\two.dds", 10, 2));
            using var set = Open("t1.ba2", "t2.ba2");
            var budgets = new List<InFlightBudget>();
            var extraction = new TextureExtraction(set, MemoryBudget.Unlimited(), (job, _) =>
            {
                lock (budgets) budgets.Add(job.Budget);
                job.Budget.Take(1, Ct);
                job.Budget.Return(1);
                return Succeeded();
            });

            extraction.Run(Out, _phase, maxInFlightBytes: 12345, Ct);

            Assert.Equal(2, budgets.Count);
            Assert.Same(budgets[0], budgets[1]);
        }

        [Fact]
        public void BothStreamsDrawFromTheInstallsMemoryBudget()
        {
            WriteArchive("t1.ba2", Texture(@"a\one.dds", 10, 1));
            WriteArchive("t2.ba2", Texture(@"b\two.dds", 10, 2));
            using var set = Open("t1.ba2", "t2.ba2");
            var memory = new MemoryBudget(1L << 20);
            var seen = new List<MemoryBudget>();
            var extraction = new TextureExtraction(set, memory, (job, _) =>
            {
                lock (seen) seen.Add(job.Memory);
                return Succeeded();
            });

            extraction.Run(Out, _phase, DefaultBudget, Ct);

            Assert.Equal(2, seen.Count);
            Assert.All(seen, budget => Assert.Same(memory, budget));
        }

        [Fact]
        public void ExactlyTwoArchivesAreInFlightAndTheSmallestIsLeftForLast()
        {
            WriteArchive("big.ba2", Texture(@"a\one.dds", 3000, 1));
            WriteArchive("mid.ba2", Texture(@"b\two.dds", 2000, 2));
            WriteArchive("small.ba2", Texture(@"c\three.dds", 100, 3));
            using var set = Open("small.ba2", "big.ba2", "mid.ba2");
            using var bothStarted = new Barrier(TextureExtraction.ConcurrentArchives);
            int active = 0, peak = 0;
            var started = new List<string>();
            var extraction = new TextureExtraction(set, MemoryBudget.Unlimited(), Recording(started, (job, _) =>
            {
                int now = Interlocked.Increment(ref active);
                InterlockedMax(ref peak, now);
                if (!job.Archive.FullPath.EndsWith("small.ba2", StringComparison.Ordinal)) Assert.True(bothStarted.SignalAndWait(Generous, Ct));
                Interlocked.Decrement(ref active);
                return Succeeded();
            }));

            extraction.Run(Out, _phase, DefaultBudget, Ct);

            Assert.Equal(TextureExtraction.ConcurrentArchives, peak);
            Assert.Equal("small.ba2", started[^1]);
        }

        [Fact]
        public void AFailureInOneStreamCancelsTheOtherAndSurfacesTheOriginalException()
        {
            WriteArchive("big.ba2", Texture(@"a\one.dds", 3000, 1));
            WriteArchive("mid.ba2", Texture(@"b\two.dds", 2000, 2));
            WriteArchive("small.ba2", Texture(@"c\three.dds", 100, 3));
            string big = Path.Combine(_game.Path, "big.ba2");
            using var set = Open("big.ba2", "mid.ba2", "small.ba2");
            using var bigRunning = new ManualResetEventSlim();
            bool bigWasCancelled = false;
            var started = new List<string>();
            var extraction = new TextureExtraction(set, MemoryBudget.Unlimited(), Recording(started, (job, ct) =>
            {
                if (job.Archive.FullPath == big)
                {
                    bigRunning.Set();
                    bigWasCancelled = ct.WaitHandle.WaitOne(Generous);
                    ct.ThrowIfCancellationRequested();
                    return Succeeded();
                }
                Assert.True(bigRunning.Wait(Generous, Ct));
                throw new IOException("disk on fire");
            }));

            var thrown = Assert.Throws<IOException>(() => extraction.Run(Out, _phase, DefaultBudget, Ct));

            Assert.Equal("disk on fire", thrown.Message);
            Assert.True(bigWasCancelled);
            Assert.DoesNotContain("small.ba2", started);
        }

        [Fact]
        public void AFailureReleasesEveryArchiveHandleThatWasInFlight()
        {
            WriteArchive("big.ba2", Texture(@"a\one.dds", 3000, 1));
            WriteArchive("mid.ba2", Texture(@"b\two.dds", 2000, 2));
            string big = Path.Combine(_game.Path, "big.ba2");
            using var set = Open("big.ba2", "mid.ba2");
            using var bigRunning = new ManualResetEventSlim();
            var extraction = new TextureExtraction(set, MemoryBudget.Unlimited(), (job, ct) =>
            {
                if (job.Archive.FullPath == big)
                {
                    bigRunning.Set();
                    ct.WaitHandle.WaitOne(Generous);
                    ct.ThrowIfCancellationRequested();
                }
                Assert.True(bigRunning.Wait(Generous, Ct));
                throw new IOException("boom");
            });

            Assert.Throws<IOException>(() => extraction.Run(Out, _phase, DefaultBudget, Ct));

            File.Delete(big);
            File.Delete(Path.Combine(_game.Path, "mid.ba2"));
            Assert.False(File.Exists(big));
        }

        [Fact]
        public void CancellingTheCallersTokenStopsBothStreamsAndThrowsOperationCanceled()
        {
            WriteArchive("a.ba2", Texture(@"a\one.dds", 3000, 1));
            WriteArchive("b.ba2", Texture(@"b\two.dds", 2000, 2));
            using var set = Open("a.ba2", "b.ba2");
            using var cts = CancellationTokenSource.CreateLinkedTokenSource(Ct);
            using var bothStarted = new Barrier(TextureExtraction.ConcurrentArchives);
            var extraction = new TextureExtraction(set, MemoryBudget.Unlimited(), (_, ct) =>
            {
                if (bothStarted.SignalAndWait(Generous, Ct)) cts.Cancel();
                ct.WaitHandle.WaitOne(Generous);
                ct.ThrowIfCancellationRequested();
                return Succeeded();
            });

            Assert.Throws<OperationCanceledException>(() => extraction.Run(Out, _phase, DefaultBudget, cts.Token));
            Assert.Empty(Directory.GetFiles(_staging.Path, ".*.extracted-direct"));
        }

        [Fact]
        public void TheActiveArchivesAreShownAsTheStepDetailWhileTheyExtract()
        {
            WriteArchive("big.ba2", Texture(@"a\one.dds", 3000, 1));
            WriteArchive("mid.ba2", Texture(@"b\two.dds", 2000, 2));
            using var set = Open("big.ba2", "mid.ba2");
            using var bothStarted = new Barrier(TextureExtraction.ConcurrentArchives);
            string? detail = null;
            var extraction = new TextureExtraction(set, MemoryBudget.Unlimited(), (_, _) =>
            {
                bothStarted.SignalAndWait(Generous, Ct);
                detail ??= Snapshot().Detail;
                return Succeeded();
            });

            extraction.Run(Out, _phase, DefaultBudget, Ct);

            Assert.Contains("big.ba2", detail);
            Assert.Contains("mid.ba2", detail);
        }

        private static void InterlockedMax(ref int target, int value)
        {
            int seen;
            while (value > (seen = Volatile.Read(ref target)) && Interlocked.CompareExchange(ref target, value, seen) != seen)
            {
            }
        }
    }
}
