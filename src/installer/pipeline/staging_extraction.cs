using System.Collections.Concurrent;
using AssetConverter.Progress;
using SharpBSABA2.BA2Util;

namespace AssetConverter.Pipeline
{
    /// <summary>One game archive extracted into the staging folder, and what happens around its extraction.</summary>
    /// <param name="MarkerName">File below the staging folder that records a complete extraction, so the next run skips the archive.</param>
    /// <param name="Includes">Which of the archive's entries (archive-relative path) are extracted.</param>
    internal sealed record StagingSource(string ArchiveName, string MarkerName, Func<string, bool> Includes)
    {
        public Func<string, string> MissingArchiveMessage { get; init; } = path => $"WARNING: {path} not found. Skipping.";

        /// <summary>The files of this archive <see cref="OnFileWritten"/> and <see cref="ObserveContent"/> are about, by staged path (relative to the staging folder); all by default.</summary>
        public Func<string, bool> Watches { get; init; } = _ => true;

        /// <summary>Receives the staged path (relative to the staging folder) of each watched file of this archive once it is complete.</summary>
        public Action<string>? OnFileWritten { get; init; }

        /// <summary>Asked, with the staged path (relative to the staging folder), for an observer of the content of each watched general (non-texture) file of this archive before it is written.</summary>
        public Func<string, IEntryContentObserver?>? ObserveContent { get; init; }

        /// <summary>Runs after the archive is extracted, or skipped because it is done or missing.</summary>
        public Action? OnFinished { get; init; }
    }

    /// <summary>
    /// Extracts the game archives the CPU stages read from, one after another in the given order, into the staging folder. The files
    /// written are what extracting them in that order would leave (see <see cref="ArchiveOverlap"/>), each written once, and the total
    /// is known before the first byte is read. Archives with a done-marker are skipped; the marker is only written when every entry
    /// extracted cleanly, so a failed or interrupted archive is retried on the next run. Disposing closes every archive still open.
    /// </summary>
    internal sealed class StagingExtraction : IDisposable
    {
        private sealed record SourceState(StagingSource Source, BA2? Archive, int OverlapIndex);

        private readonly string _stagingRoot;
        private readonly MemoryBudget _memory;
        private readonly List<SourceState> _states;
        private readonly ArchiveOverlap _overlap;
        private readonly Dictionary<int, List<Action>> _afterSource = [];

        private StagingExtraction(string stagingRoot, MemoryBudget memory, List<SourceState> states)
        {
            _stagingRoot = stagingRoot;
            _memory = memory;
            _states = states;
            var pending = states.Where(state => state.Archive != null).ToList();
            _overlap = new ArchiveOverlap([.. pending.Select(EntryPaths)], path => path);
            WrittenFiles = [.. pending.SelectMany(WrittenBy).Distinct(ArchivePath.Comparer)];
        }

        /// <summary>The files this extraction writes, relative to the staging folder.</summary>
        public IReadOnlyList<string> WrittenFiles { get; }

        /// <summary>
        /// Arranges for <paramref name="onComplete"/> to be called once every file this extraction writes that <paramref name="isInput"/>
        /// accepts, and the archive <paramref name="alsoAfterArchive"/>, are done: right after the last of the archives holding them has
        /// finished. An archive that is skipped or missing counts as finished at its turn.
        /// </summary>
        /// <exception cref="ArgumentException">No source is named <paramref name="alsoAfterArchive"/>.</exception>
        public void WhenWritten(Func<string, bool> isInput, string alsoAfterArchive, Action onComplete)
        {
            int named = _states.FindIndex(state => state.Source.ArchiveName == alsoAfterArchive);
            if (named < 0) throw new ArgumentException($"No archive named '{alsoAfterArchive}' is extracted.", nameof(alsoAfterArchive));
            int lastWriter = _states.FindLastIndex(state => state.Archive != null && WrittenBy(state).Any(isInput));
            int index = Math.Max(named, lastWriter);
            if (!_afterSource.TryGetValue(index, out var callbacks)) _afterSource[index] = callbacks = [];
            callbacks.Add(onComplete);
        }

        /// <param name="memory">Admits each extracted entry's payload against the install's memory.</param>
        public static StagingExtraction Open(string gameDataDirectory, string stagingRoot, IReadOnlyList<StagingSource> sources, MemoryBudget memory)
        {
            var states = new List<SourceState>();
            int pendingCount = 0;
            try
            {
                foreach (var source in sources)
                {
                    var archive = OpenIfPending(source, gameDataDirectory, stagingRoot);
                    states.Add(new SourceState(source, archive, archive == null ? -1 : pendingCount++));
                }
                return new StagingExtraction(stagingRoot, memory, states);
            }
            catch
            {
                foreach (var state in states) state.Archive?.Close();
                throw;
            }
        }

        /// <returns>Whether every entry of every archive extracted cleanly.</returns>
        /// <exception cref="InvalidOperationException">A <see cref="StagingSource.OnFileWritten"/> callback threw: a file that was extracted could not be handed on.</exception>
        public bool Run(int progressPhase, CancellationToken ct)
        {
            bool allSucceeded = true;
            for (int index = 0; index < _states.Count; index++)
            {
                var state = _states[index];
                if (state.Archive != null)
                    allSucceeded &= Extract(state, state.Archive, progressPhase, ct);
                state.Source.OnFinished?.Invoke();
                if (_afterSource.TryGetValue(index, out var callbacks))
                    foreach (var callback in callbacks) callback();
            }
            return allSucceeded;
        }

        public void Dispose()
        {
            foreach (var state in _states) state.Archive?.Close();
        }

        private IEnumerable<string> WrittenBy(SourceState state) =>
            EntryPaths(state).Where(path => _overlap.IsLastHolder(state.OverlapIndex, path));

        private static IEnumerable<string> EntryPaths(SourceState state) =>
            state.Archive!.Files.Select(file => file.FullPath).Where(state.Source.Includes);

        private static BA2? OpenIfPending(StagingSource source, string gameDataDirectory, string stagingRoot)
        {
            if (File.Exists(Path.Combine(stagingRoot, source.MarkerName)))
            {
                Console.WriteLine($"Skipping {source.ArchiveName} (already extracted).");
                return null;
            }

            string archivePath = Path.Combine(gameDataDirectory, source.ArchiveName);
            if (File.Exists(archivePath)) return new BA2(archivePath);
            Console.Error.WriteLine(source.MissingArchiveMessage(archivePath));
            return null;
        }

        /// <summary>Wraps <paramref name="callback"/> so it only sees the paths <paramref name="watches"/> accepts and its exceptions are kept for the stage to fail with; the unpacker still counts the entry as failed.</summary>
        private static Action<string>? Recording(Action<string>? callback, Func<string, bool> watches, ConcurrentQueue<Exception> failures)
        {
            if (callback == null) return null;
            return path =>
            {
                if (!watches(path)) return;
                try
                {
                    callback(path);
                }
                catch (Exception ex)
                {
                    failures.Enqueue(ex);
                    throw;
                }
            };
        }

        private static Func<string, IEntryContentObserver?>? ObserverOf(StagingSource source) =>
            source.ObserveContent == null ? null : path => source.Watches(path) ? source.ObserveContent(path) : null;

        private bool Extract(SourceState state, BA2 archive, int progressPhase, CancellationToken ct)
        {
            var source = state.Source;
            Console.WriteLine($"Unpacking {source.ArchiveName}...");
            InstallProgress.SetDetail(progressPhase, source.ArchiveName);
            var handOverFailures = new ConcurrentQueue<Exception>();
            try
            {
                var result = BA2ParallelUnpacker.Unpack(archive, _stagingRoot, ct: ct, progressPhase: progressPhase,
                    entryFilter: path => source.Includes(path) && _overlap.IsLastHolder(state.OverlapIndex, path),
                    onEntryWritten: Recording(source.OnFileWritten, source.Watches, handOverFailures), observeContent: ObserverOf(source), memory: _memory);
                if (!handOverFailures.IsEmpty)
                    throw new InvalidOperationException(
                        $"{source.ArchiveName}: {handOverFailures.Count} extracted file(s) could not be handed on; first: {handOverFailures.First().Message}",
                        handOverFailures.First());
                if (result.AllSucceeded)
                    File.WriteAllText(Path.Combine(_stagingRoot, source.MarkerName), "");
                else
                    Console.Error.WriteLine($"WARNING: {source.ArchiveName}: {result.Failed} entries failed; marker withheld, re-run to repair.");
                return result.AllSucceeded;
            }
            finally
            {
                archive.Close();
            }
        }
    }
}
