using System.Numerics;
using AssetConverter;
using AssetConverter.Pipeline;
using InstallerTests.Archives;
using NiflySharp.Structs;
using Xunit;

namespace InstallerTests.Nif
{
    /// <summary>
    /// The batched NIF conversion fed from a growing queue while a real archive extraction is still writing the files, against the
    /// conversion of the same files already sitting on disk: same GLB bytes, every NIF converted once, nothing hangs when a producer
    /// fails or the run is cancelled.
    /// </summary>
    [Collection(StaticStateCollections.NifConverterState)]
    public sealed class StreamedNifConversionTests : IDisposable
    {
        private const string ArchiveName = "Fallout4 - Meshes.ba2";
        private static readonly Triangle[] OneTriangle = [new(0, 1, 2)];

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private readonly ArchiveTempDir _dir = new();
        private readonly bool _censusWasBuilt = MaterialCensus.IsBuilt;

        public void Dispose()
        {
            _dir.Dispose();
            if (!_censusWasBuilt) MaterialCensus.Reset();
        }

        private string Folder(string name) => Path.Combine(_dir.Path, name);

        private byte[] TinyNif(int size)
        {
            var scene = new NifSceneBuilder();
            scene.AddShape(scene.Root, $"Shape{size}", [Vector3.Zero, new Vector3(size, 0, 0), new Vector3(0, size, 0)], OneTriangle);
            Directory.CreateDirectory(Folder("fixtures"));
            string path = scene.Save(Folder("fixtures"), $"tiny{size}.nif");
            return File.ReadAllBytes(path);
        }

        private Dictionary<string, byte[]> Corpus(int count) =>
            Enumerable.Range(1, count).ToDictionary(i => $@"Meshes\group{i % 3}\model{i}.nif", TinyNif);

        private string WriteStaged(string stagingName, Dictionary<string, byte[]> corpus)
        {
            string staging = Folder(stagingName);
            foreach (var (name, bytes) in corpus)
            {
                string path = Path.Combine(staging, name);
                Directory.CreateDirectory(Path.GetDirectoryName(path)!);
                File.WriteAllBytes(path, bytes);
            }
            return staging;
        }

        private string WriteGameArchive(Dictionary<string, byte[]> corpus)
        {
            string gameData = Folder("game");
            Directory.CreateDirectory(gameData);
            File.WriteAllBytes(Path.Combine(gameData, ArchiveName),
                Ba2Builder.Gnrl([.. corpus.Select(pair => new Ba2File(pair.Key, pair.Value, Compress: pair.Value.Length % 2 == 0))]));
            return gameData;
        }

        private static Dictionary<string, byte[]> ReadGlbs(string root) =>
            Directory.EnumerateFiles(root, "*.glb", SearchOption.AllDirectories)
                .ToDictionary(file => Path.GetRelativePath(root, file).ToLowerInvariant(), File.ReadAllBytes);

        /// <summary>Starts the extraction of the archive on another thread, queueing every NIF the moment it is complete.</summary>
        private static (StagingExtraction Extraction, NifWorkQueue Queue, Task Producer) StartExtraction(string gameData, string staging)
        {
            var queue = new NifWorkQueue();
            var source = new StagingSource(ArchiveName, $".{ArchiveName}.extracted", _ => true)
            {
                OnFileWritten = relative => queue.Add(Path.Combine(staging, relative)),
            };
            var extraction = StagingExtraction.Open(gameData, staging, [source], MemoryBudget.Unlimited());
            queue.Announce(extraction.WrittenFiles.Select(relative => Path.Combine(staging, relative)));
            var producer = Task.Run(() =>
            {
                try
                {
                    extraction.Run(-1, Ct);
                    queue.Complete();
                }
                catch (Exception ex)
                {
                    queue.Fail(ex);
                    throw;
                }
            }, Ct);
            return (extraction, queue, producer);
        }

        [Fact]
        public async Task ConvertingWhileTheArchiveExtractsWritesTheSameGlbBytesAsConvertingTheFinishedExtraction()
        {
            var corpus = Corpus(40);
            string sequentialStaging = WriteStaged("sequential-staging", corpus);
            int sequentialCount = NifConverter.ConvertToGltfBatched(Path.Combine(sequentialStaging, "Meshes"), Folder("sequential-out"),
                memory: MemoryBudget.Unlimited(), bgsmScanRoot: sequentialStaging, ct: Ct);
            string streamedStaging = Folder("streamed-staging");
            var (extraction, queue, producer) = StartExtraction(WriteGameArchive(corpus), streamedStaging);
            using var _ = extraction;

            int streamedCount = await Task.Run(() => NifConverter.ConvertToGltfBatched(
                Path.Combine(streamedStaging, "Meshes"), Folder("streamed-out"), queue, memory: MemoryBudget.Unlimited(), bgsmScanRoot: streamedStaging, ct: Ct), Ct).WithTimeout();
            await producer.WithTimeout();

            Assert.Equal(40, sequentialCount);
            Assert.Equal(40, streamedCount);
            Assert.Equal(40, queue.ExpectedCount);
            var sequentialGlbs = ReadGlbs(Folder("sequential-out"));
            Assert.Equal(40, sequentialGlbs.Count);
            Assert.Equal(sequentialGlbs, ReadGlbs(Folder("streamed-out")));
        }

        [Fact]
        public async Task ConversionsWaitForTheBudgetAndOnceItIsFreeConvertEveryFileAndReleaseAllReservations()
        {
            string staging = WriteStaged("staging", Corpus(6));
            var budget = new MemoryBudget(MemoryEstimates.NifConversionBytes(1L << 20));
            var other = await budget.ReserveAsync(budget.Capacity, Ct);
            var run = Task.Run(() => NifConverter.ConvertToGltfBatched(
                Path.Combine(staging, "Meshes"), Folder("out"), bgsmScanRoot: staging, ct: Ct, memory: budget), Ct);

            Assert.True(SpinWait.SpinUntil(() => budget.WaitingCount > 0, TimeSpan.FromSeconds(30)));
            Assert.Empty(Directory.Exists(Folder("out")) ? Directory.GetFiles(Folder("out"), "*.glb", SearchOption.AllDirectories) : []);
            other.Dispose();
            int processed = await run.WithTimeout();

            Assert.Equal(6, processed);
            Assert.Equal(6, ReadGlbs(Folder("out")).Count);
            Assert.Equal(0, budget.ReservedBytes);
        }

        [Fact]
        public async Task CancellingWhileWaitingForTheBudgetThrowsOperationCanceledInsteadOfCountingFailedFiles()
        {
            string staging = WriteStaged("staging", Corpus(3));
            var budget = new MemoryBudget(MemoryEstimates.NifConversionBytes(1L << 20));
            using var other = await budget.ReserveAsync(budget.Capacity, Ct);
            using var cts = new CancellationTokenSource();
            var run = Task.Run(() => NifConverter.ConvertToGltfBatched(
                Path.Combine(staging, "Meshes"), Folder("out"), bgsmScanRoot: staging, ct: cts.Token, memory: budget), Ct);
            Assert.True(SpinWait.SpinUntil(() => budget.WaitingCount > 0, TimeSpan.FromSeconds(30)));

            await cts.CancelAsync();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => run.WithTimeout());
            Assert.Equal(budget.Capacity, budget.ReservedBytes);
            Assert.Equal(0, budget.WaitingCount);
        }

        [Fact]
        public async Task CancellingWhileAPriorityFileWaitsForTheBudgetStillCountsItDown()
        {
            string staging = WriteStaged("staging", Corpus(3));
            int signals = 0;
            var priority = new PriorityFiles(() => Interlocked.Increment(ref signals));
            priority.Prioritize([Path.Combine(staging, @"Meshes\group1\model1.nif")]);
            var queue = new NifWorkQueue(producers: 1, priority);
            queue.AddExisting(Directory.EnumerateFiles(staging, "*.nif", SearchOption.AllDirectories));
            queue.Complete();
            priority.Expect(queue.Expected);
            var budget = new MemoryBudget(MemoryEstimates.NifConversionBytes(1L << 20));
            using var other = await budget.ReserveAsync(budget.Capacity, Ct);
            using var cts = new CancellationTokenSource();
            var run = Task.Run(() => NifConverter.ConvertToGltfBatched(
                Path.Combine(staging, "Meshes"), Folder("out"), queue, memory: budget, bgsmScanRoot: staging, priorityFiles: priority, ct: cts.Token), Ct);
            Assert.True(SpinWait.SpinUntil(() => budget.WaitingCount > 0, TimeSpan.FromSeconds(30)));

            await cts.CancelAsync();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => run.WithTimeout());
            Assert.Equal(1, signals);
        }

        [Fact]
        public async Task ANifThatCannotBeLoadedCountsAsProcessedAndDoesNotStopTheOthers()
        {
            var corpus = Corpus(5);
            corpus[@"Meshes\group0\broken.nif"] = TestBytes.Random(200, 1);
            string staging = Folder("streamed-staging");
            var (extraction, queue, producer) = StartExtraction(WriteGameArchive(corpus), staging);
            using var _ = extraction;

            int processed = await Task.Run(() => NifConverter.ConvertToGltfBatched(
                Path.Combine(staging, "Meshes"), Folder("out"), queue, memory: MemoryBudget.Unlimited(), bgsmScanRoot: staging, ct: Ct), Ct).WithTimeout();
            await producer.WithTimeout();

            Assert.Equal(6, processed);
            Assert.Equal(5, ReadGlbs(Folder("out")).Count);
        }

        [Fact]
        public async Task ThePrioritySignalIsReleasedWhenAnAnnouncedPriorityFileNeverArrives()
        {
            var corpus = Corpus(4);
            string staging = WriteStaged("staging", corpus);
            string neverArrives = Path.Combine(staging, "Meshes", "never.nif");
            string arrives = Path.Combine(staging, @"Meshes\group1\model1.nif");
            int signals = 0;
            var priority = new PriorityFiles(() => Interlocked.Increment(ref signals));
            priority.Prioritize([arrives, neverArrives]);
            var queue = new NifWorkQueue(producers: 1, priority);
            queue.Announce([neverArrives]);
            queue.AddExisting(Directory.EnumerateFiles(staging, "*.nif", SearchOption.AllDirectories));
            queue.Complete();
            priority.Expect(queue.Expected);

            int processed = await Task.Run(() => NifConverter.ConvertToGltfBatched(
                Path.Combine(staging, "Meshes"), Folder("out"), queue, memory: MemoryBudget.Unlimited(), bgsmScanRoot: staging, priorityFiles: priority, ct: Ct), Ct).WithTimeout();

            Assert.Equal(4, processed);
            Assert.Equal(1, signals);
        }

        [Fact]
        public async Task AProducerFailureStopsTheConversionWithTheProducersException()
        {
            string staging = WriteStaged("staging", Corpus(3));
            var queue = new NifWorkQueue(producers: 2);
            queue.AddExisting(Directory.EnumerateFiles(staging, "*.nif", SearchOption.AllDirectories));
            queue.Complete();
            var failure = new IOException("archive unreadable");
            var run = Task.Run(() => NifConverter.ConvertToGltfBatched(
                Path.Combine(staging, "Meshes"), Folder("out"), queue, memory: MemoryBudget.Unlimited(), bgsmScanRoot: staging, ct: Ct), Ct);

            await Task.Delay(100, Ct);
            queue.Fail(failure);

            Assert.Same(failure, await Assert.ThrowsAsync<IOException>(() => run.WithTimeout()));
        }

        [Fact]
        public async Task CancellingWhileWaitingForMoreFilesThrowsOperationCanceled()
        {
            string staging = WriteStaged("staging", Corpus(2));
            var queue = new NifWorkQueue(producers: 2);
            queue.AddExisting(Directory.EnumerateFiles(staging, "*.nif", SearchOption.AllDirectories));
            queue.Complete();
            using var cts = new CancellationTokenSource();
            var run = Task.Run(() => NifConverter.ConvertToGltfBatched(
                Path.Combine(staging, "Meshes"), Folder("out"), queue, memory: MemoryBudget.Unlimited(), bgsmScanRoot: staging, ct: cts.Token), Ct);
            await Task.Delay(100, Ct);

            await cts.CancelAsync();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => run.WithTimeout());
        }
    }
}
