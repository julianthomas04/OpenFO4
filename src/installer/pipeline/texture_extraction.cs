using System.Runtime.ExceptionServices;
using AssetConverter.Progress;
using SharpBSABA2.BA2Util;

namespace AssetConverter.Pipeline
{
    /// <summary>Everything one archive's extraction needs; <see cref="Budget"/> is shared by the archives extracted at the same time.</summary>
    internal sealed record TextureUnpackJob(BA2 Archive, string TargetDir, string StripFirstSegment, int ProgressPhase, InFlightBudget Budget,
        MemoryBudget Memory, Func<string, bool> EntryFilter);

    /// <summary>
    /// Extracts the texture archives of a <see cref="TextureArchiveSet"/> with <see cref="ConcurrentArchives"/> archives in flight, which
    /// keeps the source busy where a single archive's sequential reader leaves it idle. The archives are handed out largest first so
    /// both streams finish together. The output is what extracting the archives one after another in set order would leave: a texture
    /// present in several archives is written only by the last archive that holds it, so concurrent streams never write the same file.
    /// </summary>
    internal sealed class TextureExtraction
    {
        public const int ConcurrentArchives = 2;
        public const string StrippedSegment = "textures";

        private readonly TextureArchiveSet _textures;
        private readonly Func<TextureUnpackJob, CancellationToken, BA2ParallelUnpacker.Result> _unpack;
        private readonly MemoryBudget _memory;
        private readonly ArchiveOverlap _overlap;
        private readonly object _gate = new();
        private readonly List<string> _activeNames = [];
        private Exception? _firstFailure;
        private bool _allSucceeded = true;

        /// <param name="memory">Admits each entry's payload against the install's memory.</param>
        public TextureExtraction(TextureArchiveSet textures, MemoryBudget memory, Func<TextureUnpackJob, CancellationToken, BA2ParallelUnpacker.Result>? unpack = null)
        {
            _textures = textures;
            _memory = memory;
            _unpack = unpack ?? UnpackWithBudget;
            _overlap = new ArchiveOverlap(
                [.. textures.Pending.Select(entry => entry.Archive.Files.Select(file => file.FullPath))], DestinationOf);
        }

        /// <summary>The files this extraction writes: the archives' entries without those a later archive overwrites.</summary>
        public int FileCount => _overlap.TotalFileCount;

        /// <remarks>Single use: the failure state is not reset.</remarks>
        /// <returns>Whether every entry of every archive extracted cleanly; only those archives are marked as done.</returns>
        public bool Run(string targetDir, int progressPhase, long maxInFlightBytes, CancellationToken ct)
        {
            var queue = new Queue<int>(Enumerable.Range(0, _textures.Pending.Count).OrderByDescending(ArchiveSize));

            using var budget = new InFlightBudget(maxInFlightBytes);
            using var stop = CancellationTokenSource.CreateLinkedTokenSource(ct);
            var streams = Enumerable.Range(0, ConcurrentArchives).Select(_ => Task.Factory.StartNew(
                () => RunStream(queue, targetDir, progressPhase, budget, stop),
                CancellationToken.None, TaskCreationOptions.LongRunning, TaskScheduler.Default)).ToArray();
            Task.WaitAll(streams, CancellationToken.None);

            if (_firstFailure != null)
                ExceptionDispatchInfo.Capture(_firstFailure).Throw();
            ct.ThrowIfCancellationRequested();
            return _allSucceeded;
        }

        private void RunStream(Queue<int> queue, string targetDir, int progressPhase, InFlightBudget budget, CancellationTokenSource stop)
        {
            try
            {
                while (TryTakeNext(queue, out int index))
                {
                    stop.Token.ThrowIfCancellationRequested();
                    if (!ExtractArchive(index, targetDir, progressPhase, budget, stop.Token))
                        lock (_gate) _allSucceeded = false;
                }
            }
            catch (OperationCanceledException) when (stop.IsCancellationRequested)
            {
                // Stopped by the caller or by the other stream's failure, which Run reports.
            }
            catch (Exception ex)
            {
                lock (_gate) _firstFailure ??= ex;
                stop.Cancel();
            }
        }

        private bool ExtractArchive(int index, string targetDir, int progressPhase, InFlightBudget budget, CancellationToken ct)
        {
            var entry = _textures.Pending[index];
            Console.WriteLine($"Unpacking {entry.Name} -> {targetDir}...");
            SetActive(progressPhase, entry.Name, active: true);
            try
            {
                var job = new TextureUnpackJob(entry.Archive, targetDir, StrippedSegment, progressPhase, budget, _memory, path => _overlap.IsLastHolder(index, path));
                var result = _unpack(job, ct);
                if (result.AllSucceeded)
                    _textures.MarkExtracted(entry);
                return result.AllSucceeded;
            }
            finally
            {
                entry.Archive.Close();
                SetActive(progressPhase, entry.Name, active: false);
            }
        }

        private static BA2ParallelUnpacker.Result UnpackWithBudget(TextureUnpackJob job, CancellationToken ct) =>
            BA2ParallelUnpacker.UnpackSharingBudget(job.Archive, job.TargetDir, job.Budget, stripFirstSegment: job.StripFirstSegment,
                progressPhase: job.ProgressPhase, entryFilter: job.EntryFilter, memory: job.Memory, ct: ct);

        private static bool TryTakeNext(Queue<int> queue, out int index)
        {
            lock (queue) return queue.TryDequeue(out index);
        }

        private void SetActive(int progressPhase, string archiveName, bool active)
        {
            lock (_gate)
            {
                if (active) _activeNames.Add(archiveName);
                else _activeNames.Remove(archiveName);
                InstallProgress.SetDetail(progressPhase, string.Join(" + ", _activeNames));
            }
        }

        private long ArchiveSize(int index) => new FileInfo(_textures.Pending[index].Archive.FullPath).Length;

        private static string DestinationOf(string entryFullPath) => BA2ParallelUnpacker.DestinationRelativePath(entryFullPath, StrippedSegment);
    }
}
