using System.Collections.Concurrent;

namespace AssetConverter
{
    /// <summary>
    /// Source files a batched conversion handles first, and the signal that the last of them has been dealt with. A file counts as done
    /// whether it was written or failed (the failure is in the converter's error list), so whoever waits on the signal never hangs on a
    /// failed file and finds the missing output itself. The signal fires exactly once: when the last priority file that is part of the
    /// conversion is done, at once when there is none, or when <see cref="Release"/> says no further file will come.
    /// </summary>
    internal sealed class PriorityFiles
    {
        private readonly Action _allDone;
        private readonly ConcurrentDictionary<string, byte> _pending = new(StringComparer.OrdinalIgnoreCase);
        private volatile HashSet<string> _wanted = new(StringComparer.OrdinalIgnoreCase);
        private int _remaining;
        private int _signalled;

        /// <param name="allDone">Called once, on the thread that completed the last priority file.</param>
        public PriorityFiles(Action allDone)
        {
            _allDone = allDone;
        }

        /// <summary>Names the source files to convert first; those that are not part of the conversion are ignored. Call before the files are queued.</summary>
        public void Prioritize(IEnumerable<string> paths)
        {
            _wanted = new HashSet<string>(paths.Select(Path.GetFullPath), StringComparer.OrdinalIgnoreCase);
        }

        public bool Wants(string path)
        {
            var wanted = _wanted;
            return wanted.Count > 0 && wanted.Contains(Path.GetFullPath(path));
        }

        /// <summary>
        /// Starts counting: the priority files among <paramref name="conversionFiles"/>, every file the conversion will receive, must now be
        /// <see cref="Converted"/>. Call before any file is converted.
        /// </summary>
        public void Expect(IEnumerable<string> conversionFiles)
        {
            int count = 0;
            foreach (string file in conversionFiles)
                if (Wants(file) && _pending.TryAdd(file, 0))
                    count++;

            Volatile.Write(ref _remaining, count);
            if (count == 0) Signal();
        }

        /// <summary>Records that <paramref name="file"/> (spelled as in the list given to <see cref="Expect"/>) was converted or failed; the last priority file triggers the signal.</summary>
        public void Converted(string file)
        {
            if (_pending.TryRemove(file, out _) && Interlocked.Decrement(ref _remaining) == 0)
                Signal();
        }

        /// <summary>The conversion received its last file: priority files that never arrived (their extraction failed) no longer hold the signal back.</summary>
        public void Release() => Signal();

        private void Signal()
        {
            if (Interlocked.Exchange(ref _signalled, 1) == 0) _allDone();
        }
    }
}
