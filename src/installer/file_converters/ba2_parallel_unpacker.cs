using System.Collections.Concurrent;
using System.Diagnostics;
using System.IO.Compression;
using System.Threading.Channels;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using SharpBSABA2;
using SharpBSABA2.BA2Util;

namespace AssetConverter
{
    // Pipelined BA2 extractor.
    //
    // The legacy BA2Unpacker walks entries on one thread: each entry does
    // seek -> read -> managed inflate -> write before the next entry's read
    // starts, so the source (an ~11MB/s SMB share for a typical install) sits
    // idle during every inflate+write, and 31 of 32 hardware threads sit idle
    // the whole time.
    //
    // This implementation splits the work:
    //   - one reader thread walks entries in archive offset order and reads
    //     each compressed payload sequentially into a pooled buffer (the
    //     source only ever sees forward sequential reads — the best case for
    //     SMB and NVMe alike),
    //   - a bounded channel hands payloads to worker tasks that inflate
    //     (System.IO.Compression's native zlib) and write the output file.
    //
    // Payload memory is bounded twice: the in-flight budget caps the bytes
    // queued between reader and workers, and each entry holds a reservation
    // of its payload size from the install's MemoryBudget from the read until
    // the worker that writes it is done. The reader takes both once per
    // entry and stalls when workers fall behind, so the extraction shares the
    // machine with the other lanes without exceeding it. Payload buffers are
    // exact-size, so a reservation matches the memory actually retained.
    public static class BA2ParallelUnpacker
    {
        // Cap on payload bytes queued between reader and workers.
        internal const long DefaultMaxInFlightBytes = 384L << 20; // 384 MB

        public sealed record Result(int Total, int Succeeded, int Failed, long BytesWritten)
        {
            public bool AllSucceeded => Failed == 0;
        }

        private sealed record WorkItem(ArchiveEntry Entry, byte[][] Payloads, IDisposable? MemoryLease);

        /// <summary>
        /// Extracts every entry of <paramref name="archivePath"/> under
        /// <paramref name="targetDir"/>. <paramref name="degreeOfParallelism"/>
        /// defaults to min(ProcessorCount, 12) — inflate saturates well below
        /// 32 threads and the tail is write-bound.
        /// </summary>
        public static Result Unpack(string archivePath, string targetDir, MemoryBudget memory, int? degreeOfParallelism = null,
            bool quiet = false, string? stripFirstSegment = null,
            int progressPhase = -1, Func<string, bool>? entryFilter = null, long maxInFlightBytes = DefaultMaxInFlightBytes,
            Action<string>? onEntryWritten = null, Func<string, IEntryContentObserver?>? observeContent = null, CancellationToken ct = default)
        {
            var archive = new BA2(archivePath);
            try
            {
                return Unpack(archive, targetDir, memory, degreeOfParallelism, quiet, stripFirstSegment, progressPhase, entryFilter, maxInFlightBytes,
                    onEntryWritten, observeContent, ct);
            }
            finally
            {
                archive.Close();
            }
        }

        /// <summary>
        /// Extracts an already-opened archive (lets the caller reuse the
        /// parsed entry table, e.g. after seeding the texture index from it).
        /// <paramref name="stripFirstSegment"/> removes a leading path segment
        /// (e.g. "textures") from every entry so archives can extract directly
        /// into their final destination directory. <paramref name="entryFilter"/>
        /// (archive-relative path, backslash separated) restricts which entries
        /// are extracted. <paramref name="onEntryWritten"/> receives the destination path (relative to <paramref name="targetDir"/>)
        /// of each entry on the worker thread that wrote it, once the file is complete and closed; an entry that failed never reaches it.
        /// A callback that throws counts the entry as failed. <paramref name="observeContent"/> is asked, with the same destination path, for an
        /// observer of each general (non-texture) entry before it is written; see <see cref="IEntryContentObserver"/>.
        /// <paramref name="memory"/> admits each entry's payload against the install's memory.
        /// </summary>
        public static Result Unpack(BA2 archive, string targetDir, MemoryBudget memory, int? degreeOfParallelism = null,
            bool quiet = false, string? stripFirstSegment = null,
            int progressPhase = -1, Func<string, bool>? entryFilter = null, long maxInFlightBytes = DefaultMaxInFlightBytes,
            Action<string>? onEntryWritten = null, Func<string, IEntryContentObserver?>? observeContent = null, CancellationToken ct = default)
        {
            using var budget = new InFlightBudget(maxInFlightBytes);
            return UnpackSharingBudget(archive, targetDir, budget, memory, degreeOfParallelism, quiet, stripFirstSegment, progressPhase, entryFilter, onEntryWritten,
                observeContent, ct);
        }

        /// <summary>
        /// <see cref="Unpack(BA2, string, MemoryBudget, int?, bool, string?, int, Func{string, bool}?, long, Action{string}?, Func{string, IEntryContentObserver?}?, CancellationToken)"/> drawing its in-flight
        /// payload bytes from a budget the caller owns, so several archives extracted at once stay within one memory bound.
        /// </summary>
        internal static Result UnpackSharingBudget(BA2 archive, string targetDir, InFlightBudget budget, MemoryBudget memory, int? degreeOfParallelism = null,
            bool quiet = false, string? stripFirstSegment = null,
            int progressPhase = -1, Func<string, bool>? entryFilter = null, Action<string>? onEntryWritten = null,
            Func<string, IEntryContentObserver?>? observeContent = null, CancellationToken ct = default)
        {
            int dop = degreeOfParallelism ?? Math.Min(Environment.ProcessorCount, 12);
            var sw = Stopwatch.StartNew();
            string archivePath = archive.FullPath;

            {
                // Offset order = the order payloads physically sit in the file.
                string DestRelPath(ArchiveEntry e) => DestinationRelativePath(e.FullPath, stripFirstSegment);

                var entries = LastEntryPerDestination(archive.Files.Where(f => entryFilter == null || entryFilter(f.FullPath)), DestRelPath)
                    .OrderBy(f => f.Offset).ToList();

                var channel = Channel.CreateUnbounded<WorkItem>(new UnboundedChannelOptions
                {
                    SingleWriter = true,
                    SingleReader = false,
                });

                int succeeded = 0, failed = 0;
                long bytesWritten = 0;
                var errors = new ConcurrentBag<string>();

                // Pre-create the destination directory tree once (cheap
                // compared to a CreateDirectory per file under contention).
                var dirs = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
                foreach (var e in entries)
                {
                    var dir = Path.GetDirectoryName(Path.Combine(targetDir, DestRelPath(e)));
                    if (dir != null) dirs.Add(dir);
                }
                foreach (var dir in dirs) Directory.CreateDirectory(dir);

                // Workers: inflate + write.
                var workers = new Task[dop];
                for (int w = 0; w < dop; w++)
                {
                    workers[w] = Task.Run(async () =>
                    {
                        await foreach (var item in channel.Reader.ReadAllAsync())
                        {
                            try
                            {
                                string relativeDestination = DestRelPath(item.Entry);
                                var observer = item.Entry is BA2FileEntry ? observeContent?.Invoke(relativeDestination) : null;
                                long written = WriteEntry(item, Path.Combine(targetDir, relativeDestination), observer);
                                onEntryWritten?.Invoke(relativeDestination);
                                Interlocked.Add(ref bytesWritten, written);
                                Interlocked.Increment(ref succeeded);
                                InstallProgress.Tick(progressPhase);
                            }
                            catch (Exception ex)
                            {
                                Interlocked.Increment(ref failed);
                                errors.Add($"{item.Entry.FullPath}: {ex.Message}");
                            }
                            finally
                            {
                                Release(item, budget);
                            }
                        }
                    }, CancellationToken.None);
                }

                // Reader: sequential walk of the data section. Whatever ends it
                // (including cancellation), the workers are drained and joined
                // before the archive and budget are disposed.
                var readerStream = archive.BinaryReader.BaseStream;
                try
                {
                    foreach (var entry in entries)
                    {
                        ct.ThrowIfCancellationRequested();
                        try
                        {
                            WorkItem item = ReadEntryPayloads(entry, readerStream, budget, memory, ct);
                            channel.Writer.TryWrite(item);
                        }
                        catch (Exception ex) when (ex is not OperationCanceledException)
                        {
                            Interlocked.Increment(ref failed);
                            errors.Add($"{entry.FullPath}: read failed: {ex.Message}");
                        }
                    }
                }
                catch (OperationCanceledException)
                {
                    while (channel.Reader.TryRead(out var abandoned))
                        Release(abandoned, budget);
                    throw;
                }
                finally
                {
                    channel.Writer.TryComplete();
                    Task.WaitAll(workers, CancellationToken.None);
                }

                sw.Stop();
                if (!quiet)
                {
                    Console.WriteLine($"  {Path.GetFileName(archivePath)}: {succeeded:N0} files, " +
                        $"{bytesWritten / (1 << 20):N0} MB out in {sw.Elapsed:mm\\:ss\\.f} " +
                        $"({(failed > 0 ? $"{failed} FAILED" : "ok")}, dop={dop})");
                    foreach (var e in errors.Take(10)) Console.Error.WriteLine($"    ERROR {e}");
                    if (errors.Count > 10) Console.Error.WriteLine($"    ... and {errors.Count - 10} more errors");
                }
                return new Result(entries.Count, succeeded, failed, bytesWritten);
            }
        }

        /// <summary>
        /// Of several entries that land on the same file (same path up to case and slash direction) only the last one in table order: the
        /// one extracting the archive entry by entry would leave on disk. Extracting them all would have workers write one file at once.
        /// </summary>
        private static List<ArchiveEntry> LastEntryPerDestination(IEnumerable<ArchiveEntry> entries, Func<ArchiveEntry, string> destinationOf)
        {
            var last = new Dictionary<string, ArchiveEntry>(ArchivePath.Comparer);
            foreach (var entry in entries) last[destinationOf(entry)] = entry;
            return [.. last.Values];
        }

        /// <summary>Where an entry lands under the target directory: its archive path minus a leading <paramref name="stripFirstSegment"/>.</summary>
        internal static string DestinationRelativePath(string entryFullPath, string? stripFirstSegment)
        {
            if (stripFirstSegment == null) return entryFullPath;
            string prefix = stripFirstSegment.TrimEnd('\\', '/') + "\\";
            return entryFullPath.Length > prefix.Length && entryFullPath.StartsWith(prefix, StringComparison.OrdinalIgnoreCase)
                ? entryFullPath.Substring(prefix.Length)
                : entryFullPath;
        }

        // Reads the compressed payload span(s) for an entry into exact-size buffers, so the memory reservation matches what is held. GNRL entries have one span; DX10 texture entries have
        // one span per chunk. The in-flight budget and then the memory budget are taken once for the whole entry, so a reader never waits
        // for either while holding part of an entry (two readers doing that could block each other for good); both and the buffers are
        // given back before a failure propagates. The workers never wait for anything, so the memory a waiting reader needs is always
        // being released by work that does not depend on it.
        private static WorkItem ReadEntryPayloads(ArchiveEntry entry, Stream src, InFlightBudget budget, MemoryBudget memory, CancellationToken ct)
        {
            var spans = PayloadSpans(entry);
            long entryBytes = spans.Sum(span => (long)span.Length);
            budget.Take(entryBytes, ct);
            var payloads = new List<byte[]>(spans.Count);
            MemoryLease? lease = null;
            try
            {
                if (entryBytes > 0) lease = memory.Reserve(entryBytes, ct);
                foreach (var (offset, len) in spans)
                {
                    var buf = GC.AllocateUninitializedArray<byte>(len);
                    payloads.Add(buf);
                    if (src.Position != offset) src.Seek(offset, SeekOrigin.Begin);
                    src.ReadExactly(buf, 0, len);
                }
            }
            catch
            {
                lease?.Dispose();
                budget.Return(entryBytes);
                throw;
            }
            return new WorkItem(entry, payloads.ToArray(), lease);
        }

        /// <summary>Gives back what an entry holds: its in-flight bytes and its memory reservation. Called exactly once per queued entry, by whoever ends up owning it.</summary>
        private static void Release(WorkItem item, InFlightBudget budget)
        {
            item.MemoryLease?.Dispose();
            budget.Return(item.Payloads.Sum(payload => (long)payload.Length));
        }

        private static List<(long Offset, int Length)> PayloadSpans(ArchiveEntry entry) => entry switch
        {
            BA2TextureEntry tex => tex.Chunks
                .Select(c => ((long)c.offset, (int)(c.packSz != 0 ? c.packSz : c.fullSz))).ToList(),
            BA2FileEntry file => [((long)file.Offset, (int)(file.Compressed ? file.Size : file.RealSize))],
            _ => throw new NotSupportedException($"Unsupported BA2 entry type {entry.GetType().Name}"),
        };

        // Inflates / copies payloads into the destination file. Runs on
        // worker threads; touches only the in-memory payloads.
        private static long WriteEntry(WorkItem item, string dest, IEntryContentObserver? observer)
        {
            using var fs = new FileStream(dest, FileMode.Create, FileAccess.Write, FileShare.None,
                bufferSize: 256 << 10, FileOptions.SequentialScan);

            switch (item.Entry)
            {
                case BA2TextureEntry tex:
                    {
                        // leaveOpen: BinaryWriter would otherwise close fs at the
                        // end of this block, before the final fs.Length read.
                        using var bw = new BinaryWriter(fs, System.Text.Encoding.UTF8, leaveOpen: true);
                        if (tex.GenerateTextureHeader)
                            tex.WriteHeaderTo(bw);

                        for (int i = 0; i < tex.Chunks.Count; i++)
                        {
                            var chunk = tex.Chunks[i];
                            bool compressed = chunk.packSz != 0;
                            if (compressed)
                                InflateTo(item.Payloads[i], fs);
                            else
                                fs.Write(item.Payloads[i]);
                        }

                        if (tex.dataSizePosition > -1)
                        {
                            long cur = fs.Position;
                            fs.Position = tex.dataSizePosition;
                            bw.Write((uint)(fs.Length - 164));
                            fs.Position = cur;
                        }
                        break;
                    }
                case BA2FileEntry file:
                    {
                        Stream sink = observer == null ? fs : new ObservedWriteStream(fs, observer);
                        if (file.Compressed)
                            InflateTo(item.Payloads[0], sink);
                        else
                            sink.Write(item.Payloads[0]);
                        observer?.Complete();
                        break;
                    }
            }
            return fs.Length;
        }

        private static void InflateTo(byte[] payload, Stream output)
        {
            using var ms = new MemoryStream(payload, writable: false);
            using var z = new ZLibStream(ms, CompressionMode.Decompress);
            z.CopyTo(output, 256 << 10);
        }
    }
}
