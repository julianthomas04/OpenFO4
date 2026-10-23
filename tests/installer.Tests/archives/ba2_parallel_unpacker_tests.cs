using AssetConverter;
using AssetConverter.Pipeline;
using SharpBSABA2.BA2Util;
using Xunit;

namespace InstallerTests.Archives
{
    public class BA2ParallelUnpackerTests
    {
        private static readonly TimeSpan Generous = TimeSpan.FromSeconds(30);

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private static readonly Ba2File Mesh = new(@"meshes\a\one.nif", TestBytes.Random(6000, 21), Compress: true);
        private static readonly Ba2File Stored = new(@"meshes\b\two.nif", TestBytes.Random(700, 22), Compress: false);
        private static readonly Ba2File Other = new(@"scripts\three.pex", TestBytes.Repeating(9000, 9), Compress: true);

        [Fact]
        public void Unpack_GeneralArchive_WritesEveryEntryUnderItsArchivePath()
        {
            using var dir = new ArchiveTempDir();
            string archive = dir.Write("a.ba2", Ba2Builder.Gnrl([Mesh, Stored, Other]));
            string output = Path.Combine(dir.Path, "out");

            var result = BA2ParallelUnpacker.Unpack(archive, output, MemoryBudget.Unlimited(), degreeOfParallelism: 3, quiet: true, ct: Ct);

            Assert.Equal(new BA2ParallelUnpacker.Result(3, 3, 0, Mesh.Content.Length + Stored.Content.Length + Other.Content.Length), result);
            Assert.Equal(Mesh.Content, File.ReadAllBytes(Path.Combine(output, "meshes", "a", "one.nif")));
            Assert.Equal(Stored.Content, File.ReadAllBytes(Path.Combine(output, "meshes", "b", "two.nif")));
            Assert.Equal(Other.Content, File.ReadAllBytes(Path.Combine(output, "scripts", "three.pex")));
        }

        [Fact]
        public void Unpack_EntriesForOneFileUnderVariantSpellings_WriteItOnceWithTheLastEntry()
        {
            using var dir = new ArchiveTempDir();
            var first = new Ba2File(@"meshes\a\one.nif", TestBytes.Random(6000, 31), Compress: true);
            var last = new Ba2File(@"Meshes\A\ONE.nif", TestBytes.Random(900, 32), Compress: false);
            string archive = dir.Write("a.ba2", Ba2Builder.Gnrl([first, Stored, last]));
            string output = Path.Combine(dir.Path, "out");
            var written = new System.Collections.Concurrent.ConcurrentBag<string>();

            var result = BA2ParallelUnpacker.Unpack(archive, output, MemoryBudget.Unlimited(), degreeOfParallelism: 3, quiet: true, onEntryWritten: written.Add, ct: Ct);

            Assert.Equal(new BA2ParallelUnpacker.Result(2, 2, 0, last.Content.Length + Stored.Content.Length), result);
            Assert.Equal(2, written.Count);
            Assert.Equal(last.Content, File.ReadAllBytes(Path.Combine(output, "meshes", "a", "one.nif")));
        }

        [Fact]
        public void Unpack_DataStoredInReverseTableOrder_StillMapsEachPayloadToItsEntry()
        {
            using var dir = new ArchiveTempDir();
            string archive = dir.Write("a.ba2", Ba2Builder.Gnrl([Mesh, Stored, Other], reverseDataOrder: true));
            string output = Path.Combine(dir.Path, "out");

            BA2ParallelUnpacker.Unpack(archive, output, MemoryBudget.Unlimited(), degreeOfParallelism: 2, quiet: true, ct: Ct);

            Assert.Equal(Mesh.Content, File.ReadAllBytes(Path.Combine(output, "meshes", "a", "one.nif")));
            Assert.Equal(Other.Content, File.ReadAllBytes(Path.Combine(output, "scripts", "three.pex")));
        }

        [Fact]
        public void Unpack_StripFirstSegment_RemovesLeadingFolderFromOutputPaths()
        {
            using var dir = new ArchiveTempDir();
            var tex = new Ba2File(@"textures\x\y.bin", TestBytes.Random(100, 5), Compress: false);
            string archive = dir.Write("a.ba2", Ba2Builder.Gnrl([tex, Mesh]));
            string output = Path.Combine(dir.Path, "out");

            BA2ParallelUnpacker.Unpack(archive, output, MemoryBudget.Unlimited(), quiet: true, stripFirstSegment: "textures", ct: Ct);

            Assert.Equal(tex.Content, File.ReadAllBytes(Path.Combine(output, "x", "y.bin")));
            Assert.True(File.Exists(Path.Combine(output, "meshes", "a", "one.nif")));
        }

        [Fact]
        public void Unpack_EntryFilter_ExtractsOnlyMatchingEntries()
        {
            using var dir = new ArchiveTempDir();
            string archive = dir.Write("a.ba2", Ba2Builder.Gnrl([Mesh, Stored, Other]));
            string output = Path.Combine(dir.Path, "out");

            var result = BA2ParallelUnpacker.Unpack(archive, output, MemoryBudget.Unlimited(), quiet: true,
                entryFilter: path => path.EndsWith(".pex", StringComparison.OrdinalIgnoreCase), ct: Ct);

            Assert.Equal(1, result.Total);
            Assert.Single(Directory.GetFiles(output, "*", SearchOption.AllDirectories));
        }

        [Fact]
        public void Unpack_OnEntryWritten_ReportsEachFileOnlyOnceItIsCompleteAndClosed()
        {
            using var dir = new ArchiveTempDir();
            string archive = dir.Write("a.ba2", Ba2Builder.Gnrl([Mesh, Stored, Other]));
            string output = Path.Combine(dir.Path, "out");
            var seen = new System.Collections.Concurrent.ConcurrentDictionary<string, byte[]>(StringComparer.OrdinalIgnoreCase);

            BA2ParallelUnpacker.Unpack(archive, output, MemoryBudget.Unlimited(), degreeOfParallelism: 3, quiet: true, ct: Ct, onEntryWritten: relative =>
            {
                using var exclusive = new FileStream(Path.Combine(output, relative), FileMode.Open, FileAccess.ReadWrite, FileShare.None);
                var content = new byte[exclusive.Length];
                exclusive.ReadExactly(content);
                seen.TryAdd(relative, content);
            });

            Assert.Equal(3, seen.Count);
            Assert.Equal(Mesh.Content, seen[@"meshes\a\one.nif"]);
            Assert.Equal(Stored.Content, seen[@"meshes\b\two.nif"]);
            Assert.Equal(Other.Content, seen[@"scripts\three.pex"]);
        }

        [Fact]
        public void Unpack_OnEntryWritten_IsNotCalledForFilteredOrFailedEntries()
        {
            using var dir = new ArchiveTempDir();
            var bytes = Ba2Builder.Gnrl([Mesh, Stored, Other], nameTable: true);
            int firstPayload = 24 + 3 * 36 + 4;
            for (int i = firstPayload; i < firstPayload + 16; i++)
                bytes[i] ^= 0xFF;
            string archive = dir.Write("a.ba2", bytes);
            var written = new System.Collections.Concurrent.ConcurrentBag<string>();

            var result = BA2ParallelUnpacker.Unpack(archive, Path.Combine(dir.Path, "out"), MemoryBudget.Unlimited(), degreeOfParallelism: 2, quiet: true,
                entryFilter: path => !path.EndsWith(".pex", StringComparison.OrdinalIgnoreCase), ct: Ct, onEntryWritten: written.Add);

            Assert.Equal(1, result.Failed);
            Assert.Equal([@"meshes\b\two.nif"], written);
        }

        [Fact]
        public void Unpack_OnEntryWrittenThatThrows_CountsTheEntryAsFailed()
        {
            using var dir = new ArchiveTempDir();
            string archive = dir.Write("a.ba2", Ba2Builder.Gnrl([Mesh, Stored]));

            var result = BA2ParallelUnpacker.Unpack(archive, Path.Combine(dir.Path, "out"), MemoryBudget.Unlimited(), quiet: true, ct: Ct,
                onEntryWritten: relative => throw new InvalidOperationException(relative));

            Assert.Equal(2, result.Failed);
            Assert.Equal(0, result.Succeeded);
        }

        private sealed class RecordingObserver : IEntryContentObserver
        {
            private readonly List<byte> _content = [];

            public int Completions { get; private set; }

            public byte[] Content => [.. _content];

            public void Write(ReadOnlySpan<byte> chunk) => _content.AddRange(chunk.ToArray());

            public void Complete() => Completions++;
        }

        [Fact]
        public void Unpack_ContentObserver_SeesExactlyTheBytesOfEachFileAndCompletesOnce()
        {
            using var dir = new ArchiveTempDir();
            var large = new Ba2File(@"meshes\c\large.nif", [.. TestBytes.Random(700_000, 23), .. TestBytes.Repeating(300_000, 4)], Compress: true);
            string archive = dir.Write("a.ba2", Ba2Builder.Gnrl([Mesh, Stored, Other, large]));
            string output = Path.Combine(dir.Path, "out");
            var observers = new System.Collections.Concurrent.ConcurrentDictionary<string, RecordingObserver>(StringComparer.OrdinalIgnoreCase);

            BA2ParallelUnpacker.Unpack(archive, output, MemoryBudget.Unlimited(), degreeOfParallelism: 3, quiet: true, ct: Ct,
                observeContent: relative => observers.GetOrAdd(relative, _ => new RecordingObserver()));

            Assert.Equal(4, observers.Count);
            foreach (var (relative, observer) in observers)
            {
                Assert.Equal(File.ReadAllBytes(Path.Combine(output, relative)), observer.Content);
                Assert.Equal(1, observer.Completions);
            }
            Assert.Equal(1_000_000, observers[@"meshes\c\large.nif"].Content.Length);
        }

        [Fact]
        public void Unpack_ContentObserver_MayDeclineAFileAndIsNotAskedAboutFilteredEntries()
        {
            using var dir = new ArchiveTempDir();
            string archive = dir.Write("a.ba2", Ba2Builder.Gnrl([Mesh, Stored, Other]));
            var asked = new System.Collections.Concurrent.ConcurrentBag<string>();

            var result = BA2ParallelUnpacker.Unpack(archive, Path.Combine(dir.Path, "out"), MemoryBudget.Unlimited(), quiet: true, ct: Ct,
                entryFilter: path => !path.EndsWith(".pex", StringComparison.OrdinalIgnoreCase),
                observeContent: relative =>
                {
                    asked.Add(relative);
                    return null;
                });

            Assert.Equal(2, result.Succeeded);
            Assert.Equal(2, asked.Count);
            Assert.DoesNotContain(@"scripts	hree.pex", asked);
        }

        [Fact]
        public void Unpack_ContentObserver_IsNotAskedAboutTextureEntries()
        {
            using var dir = new ArchiveTempDir();
            var texture = new Ba2Texture(@"textures\a\wall_d.dds", 64, 32, 1, 77, [new Ba2Chunk(TestBytes.Random(2048, 12), true, 0, 0)]);
            string archive = dir.Write("t.ba2", Ba2Builder.Dx10([texture]));
            var asked = new System.Collections.Concurrent.ConcurrentBag<string>();

            var result = BA2ParallelUnpacker.Unpack(archive, Path.Combine(dir.Path, "out"), MemoryBudget.Unlimited(), quiet: true, ct: Ct,
                observeContent: relative =>
                {
                    asked.Add(relative);
                    return new RecordingObserver();
                });

            Assert.Equal(1, result.Succeeded);
            Assert.Empty(asked);
        }

        [Fact]
        public void Unpack_ContentObserver_IsNotCompletedForAnEntryThatFails()
        {
            using var dir = new ArchiveTempDir();
            var bytes = Ba2Builder.Gnrl([Mesh, Stored], nameTable: true);
            int firstPayload = 24 + 2 * 36 + 4;
            for (int i = firstPayload; i < firstPayload + 16; i++)
                bytes[i] ^= 0xFF;
            string archive = dir.Write("a.ba2", bytes);
            var observers = new System.Collections.Concurrent.ConcurrentDictionary<string, RecordingObserver>(StringComparer.OrdinalIgnoreCase);

            var result = BA2ParallelUnpacker.Unpack(archive, Path.Combine(dir.Path, "out"), MemoryBudget.Unlimited(), degreeOfParallelism: 1, quiet: true, ct: Ct,
                observeContent: relative => observers.GetOrAdd(relative, _ => new RecordingObserver()));

            Assert.Equal(1, result.Failed);
            Assert.Equal(1, observers.Values.Count(observer => observer.Completions == 1));
            Assert.Equal(0, observers.Values.Count(observer => observer.Completions > 1));
        }

        [Fact]
        public void Unpack_TextureArchive_WritesDdsFilesWithHeaders()
        {
            using var dir = new ArchiveTempDir();
            var mip = TestBytes.Random(2048, 31);
            var tex = new Ba2Texture(@"textures\wall_d.dds", 32, 32, 1, 77, [new Ba2Chunk(mip, true)]);
            string archive = dir.Write("t.ba2", Ba2Builder.Dx10([tex]));
            string output = Path.Combine(dir.Path, "out");

            BA2ParallelUnpacker.Unpack(archive, output, MemoryBudget.Unlimited(), quiet: true, ct: Ct);

            byte[] dds = File.ReadAllBytes(Path.Combine(output, "textures", "wall_d.dds"));
            Assert.Equal(mip, dds[128..]);
            Assert.Equal((byte)'D', dds[0]);
        }

        [Fact]
        public void Unpack_TextureArchive_MatchesTheSequentialExtractorByteForByte()
        {
            using var dir = new ArchiveTempDir();
            var tex = new Ba2Texture(@"textures\wall_d.dds", 32, 32, 2, 98,
                [new Ba2Chunk(TestBytes.Random(2048, 31), true), new Ba2Chunk(TestBytes.Random(512, 32), false)]);
            string path = dir.Write("t.ba2", Ba2Builder.Dx10([tex]));
            string output = Path.Combine(dir.Path, "out");

            BA2ParallelUnpacker.Unpack(path, output, MemoryBudget.Unlimited(), quiet: true, ct: Ct);

            var sequential = new BA2(path);
            try
            {
                Assert.Equal(sequential.Files[0].GetDataStream().ToArray(), File.ReadAllBytes(Path.Combine(output, "textures", "wall_d.dds")));
            }
            finally
            {
                sequential.Close();
            }
        }

        [Fact]
        public void Unpack_OneEntryWithCorruptPayload_CountsFailureAndStillExtractsTheRest()
        {
            using var dir = new ArchiveTempDir();
            var bytes = Ba2Builder.Gnrl([Mesh, Stored], nameTable: true);
            int firstPayload = 24 + 2 * 36 + 4;
            for (int i = firstPayload; i < firstPayload + 16; i++)
                bytes[i] ^= 0xFF;
            string archive = dir.Write("a.ba2", bytes);
            string output = Path.Combine(dir.Path, "out");

            var result = BA2ParallelUnpacker.Unpack(archive, output, MemoryBudget.Unlimited(), degreeOfParallelism: 2, quiet: true, ct: Ct);

            Assert.Equal(1, result.Failed);
            Assert.Equal(1, result.Succeeded);
            Assert.False(result.AllSucceeded);
            Assert.Equal(Stored.Content, File.ReadAllBytes(Path.Combine(output, "meshes", "b", "two.nif")));
        }

        [Fact]
        public void Unpack_CompressedPayloadCutShort_CountsAsFailureInsteadOfWritingATruncatedFile()
        {
            using var dir = new ArchiveTempDir();
            var bytes = Ba2Builder.Gnrl([Mesh], nameTable: false);
            const int packedSizeField = 24 + 20;
            uint packed = BitConverter.ToUInt32(bytes, packedSizeField);
            BitConverter.GetBytes(packed - 40).CopyTo(bytes, packedSizeField);
            string archive = dir.Write("a.ba2", bytes);
            string output = Path.Combine(dir.Path, "out");

            var result = BA2ParallelUnpacker.Unpack(archive, output, MemoryBudget.Unlimited(), quiet: true, ct: Ct);

            Assert.Equal(1, result.Failed);
            Assert.Equal(0, result.Succeeded);
        }

        [Fact]
        public void Unpack_PayloadBeyondEndOfFile_CountsAsFailedRead()
        {
            using var dir = new ArchiveTempDir();
            var bytes = Ba2Builder.Gnrl([Stored], nameTable: false);
            string archive = dir.Write("a.ba2", bytes[..^50]);
            string output = Path.Combine(dir.Path, "out");

            var result = BA2ParallelUnpacker.Unpack(archive, output, MemoryBudget.Unlimited(), quiet: true, ct: Ct);

            Assert.Equal(new BA2ParallelUnpacker.Result(1, 0, 1, 0), result);
        }

        [Fact]
        public async Task Unpack_ReadFailuresWithATinyBudget_DoNotStrandTheReader()
        {
            using var dir = new ArchiveTempDir();
            var files = Enumerable.Range(0, 20).Select(i => new Ba2File($"data\\f{i}.bin", TestBytes.Random(200, i), Compress: false)).ToList();
            var bytes = Ba2Builder.Gnrl(files, nameTable: false);
            string archive = dir.Write("a.ba2", bytes[..^1000]);
            string output = Path.Combine(dir.Path, "out");

            var task = Task.Run(() => BA2ParallelUnpacker.Unpack(archive, output, MemoryBudget.Unlimited(), degreeOfParallelism: 2, quiet: true, maxInFlightBytes: 1, ct: Ct), Ct);

            var result = await task.WaitAsync(Generous, Ct);

            Assert.True(result.Failed > 0);
        }

        [Fact]
        public async Task UnpackSharingBudget_TwoArchivesOfMultiChunkTexturesAndATinyBudget_BothFinish()
        {
            using var dir = new ArchiveTempDir();
            string Archive(string name, int seed) => dir.Write(name, Ba2Builder.Dx10(Enumerable.Range(0, 6).Select(i =>
                new Ba2Texture($@"textures\{name}\t{i}.dds", 32, 32, 3, 98,
                    [new Ba2Chunk(TestBytes.Random(600, seed + i), true), new Ba2Chunk(TestBytes.Random(400, seed + 10 + i), false),
                     new Ba2Chunk(TestBytes.Random(300, seed + 20 + i), true)])).ToList()));
            string first = Archive("a", 100), second = Archive("b", 200);
            using var budget = new InFlightBudget(1);

            var extractions = new[] { first, second }.Select(path => Task.Run(() =>
            {
                var archive = new BA2(path);
                try
                {
                    return BA2ParallelUnpacker.UnpackSharingBudget(archive, Path.Combine(dir.Path, "out"), budget, MemoryBudget.Unlimited(), degreeOfParallelism: 2, quiet: true, ct: Ct);
                }
                finally
                {
                    archive.Close();
                }
            }, Ct)).ToArray();

            var results = await Task.WhenAll(extractions).WaitAsync(Generous, Ct);

            Assert.All(results, result => Assert.True(result.AllSucceeded));
            Assert.Equal([6, 6], results.Select(result => result.Succeeded));
            Assert.Equal(0, budget.InFlightBytes);
        }

        [Fact]
        public void Unpack_HoldsEachPayloadInTheMemoryBudgetUntilItsEntryIsWritten()
        {
            using var dir = new ArchiveTempDir();
            string archive = dir.Write("a.ba2", Ba2Builder.Gnrl([Mesh, Stored, Other]));
            var budget = new MemoryBudget(1L << 30);
            var reservedWhenWritten = new System.Collections.Concurrent.ConcurrentBag<long>();

            var result = BA2ParallelUnpacker.Unpack(archive, Path.Combine(dir.Path, "out"), degreeOfParallelism: 1, quiet: true,
                onEntryWritten: _ => reservedWhenWritten.Add(budget.ReservedBytes), memory: budget, ct: Ct);

            Assert.True(result.AllSucceeded);
            Assert.Equal(3, reservedWhenWritten.Count);
            Assert.All(reservedWhenWritten, reserved => Assert.InRange(reserved, 1, Mesh.Content.Length + Stored.Content.Length + Other.Content.Length));
            Assert.Equal(0, budget.ReservedBytes);
        }

        [Fact]
        public async Task Unpack_WaitsForTheMemoryBudgetAndFinishesOnceItIsReleased()
        {
            using var dir = new ArchiveTempDir();
            string archive = dir.Write("a.ba2", Ba2Builder.Gnrl([Mesh, Stored, Other]));
            string output = Path.Combine(dir.Path, "out");
            var budget = new MemoryBudget(20_000);
            var other = await budget.ReserveAsync(budget.Capacity, Ct);

            var run = Task.Run(() => BA2ParallelUnpacker.Unpack(archive, output, degreeOfParallelism: 2, quiet: true, memory: budget, ct: Ct), Ct);
            Assert.True(SpinWait.SpinUntil(() => budget.WaitingCount == 1, Generous));
            Assert.False(run.IsCompleted);
            other.Dispose();
            var result = await run.WaitAsync(Generous, Ct);

            Assert.Equal(new BA2ParallelUnpacker.Result(3, 3, 0, Mesh.Content.Length + Stored.Content.Length + Other.Content.Length), result);
            Assert.Equal(0, budget.ReservedBytes);
        }

        [Fact]
        public async Task Unpack_CancelledWhileWaitingForTheMemoryBudget_ThrowsOperationCanceledAndLeaksNothing()
        {
            using var dir = new ArchiveTempDir();
            string archive = dir.Write("a.ba2", Ba2Builder.Gnrl([Mesh, Stored, Other]));
            var budget = new MemoryBudget(20_000);
            using var other = await budget.ReserveAsync(budget.Capacity, Ct);
            using var cts = new CancellationTokenSource();

            var run = Task.Run(() => BA2ParallelUnpacker.Unpack(archive, Path.Combine(dir.Path, "out"), quiet: true, memory: budget, ct: cts.Token), Ct);
            Assert.True(SpinWait.SpinUntil(() => budget.WaitingCount == 1, Generous));
            await cts.CancelAsync();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => run.WaitAsync(Generous, Ct));
            Assert.Equal(budget.Capacity, budget.ReservedBytes);
            Assert.Equal(0, budget.WaitingCount);
        }

        [Fact]
        public void Unpack_EntriesThatFailToWriteOrToReadStillReleaseTheirMemory()
        {
            using var dir = new ArchiveTempDir();
            var files = Enumerable.Range(0, 6).Select(i => new Ba2File($"data{i}.bin", TestBytes.Random(200, i), Compress: false)).ToList();
            var bytes = Ba2Builder.Gnrl(files, nameTable: false);
            string truncated = dir.Write("truncated.ba2", bytes[..^500]);
            var budget = new MemoryBudget(1L << 30);

            var readFailures = BA2ParallelUnpacker.Unpack(truncated, Path.Combine(dir.Path, "out1"), degreeOfParallelism: 2, quiet: true, memory: budget, ct: Ct);
            var writeFailures = BA2ParallelUnpacker.Unpack(dir.Write("whole.ba2", bytes), Path.Combine(dir.Path, "out2"), degreeOfParallelism: 2, quiet: true,
                onEntryWritten: _ => throw new IOException("hand-over failed"), memory: budget, ct: Ct);

            Assert.True(readFailures.Failed > 0);
            Assert.Equal(6, writeFailures.Failed);
            Assert.Equal(0, budget.ReservedBytes);
        }

        [Fact]
        public void Unpack_TokenAlreadyCancelled_ThrowsOperationCanceled()
        {
            using var dir = new ArchiveTempDir();
            string archive = dir.Write("a.ba2", Ba2Builder.Gnrl([Mesh, Stored]));
            using var cts = new CancellationTokenSource();
            cts.Cancel();

            Assert.Throws<OperationCanceledException>(
                () => BA2ParallelUnpacker.Unpack(archive, Path.Combine(dir.Path, "out"), MemoryBudget.Unlimited(), quiet: true, ct: cts.Token));
        }
    }

    public class InFlightBudgetTests
    {
        private static readonly TimeSpan Generous = TimeSpan.FromSeconds(30);
        private static readonly TimeSpan ShortWait = TimeSpan.FromMilliseconds(200);

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        [Fact]
        public void Take_WithinTheCap_ReturnsImmediately()
        {
            using var budget = new InFlightBudget(100);

            budget.Take(60, Ct);
            budget.Take(60, Ct);

            Assert.Equal(120, budget.InFlightBytes);
        }

        [Fact]
        public void Take_OversizedItemAloneOnAnEmptyBudget_IsAdmitted()
        {
            using var budget = new InFlightBudget(10);

            budget.Take(1000, Ct);

            Assert.Equal(1000, budget.InFlightBytes);
        }

        [Fact]
        public async Task Take_OverTheCap_BlocksUntilBytesAreReturned()
        {
            using var budget = new InFlightBudget(100);
            budget.Take(150, Ct);
            var blocked = Task.Run(() => budget.Take(10, Ct), Ct);

            await Assert.ThrowsAsync<TimeoutException>(() => blocked.WaitAsync(ShortWait, Ct));
            budget.Return(150);
            await blocked.WaitAsync(Generous, Ct);

            Assert.Equal(10, budget.InFlightBytes);
        }

        [Fact]
        public async Task Take_OverTheCapAndCancelled_ThrowsInsteadOfWaitingForever()
        {
            using var budget = new InFlightBudget(100);
            budget.Take(150, Ct);
            using var cts = new CancellationTokenSource();
            var blocked = Task.Run(() => budget.Take(10, cts.Token), Ct);
            await Assert.ThrowsAsync<TimeoutException>(() => blocked.WaitAsync(ShortWait, Ct));

            await cts.CancelAsync();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => blocked.WaitAsync(Generous, Ct));
        }
    }
}
