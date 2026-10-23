using System.Collections.Concurrent;
using System.Reflection;
using Microsoft.CSharp.RuntimeBinder;

namespace AssetConverter
{
    /// <summary>
    /// Aggregates tolerated property-read failures (reflection / dynamic / lazy Mutagen
    /// overlay reads) per call site so a 100k-record ESM load prints one summary line per
    /// site instead of one line per record. Anything outside the known read-failure types
    /// is not tolerated and propagates.
    /// </summary>
    internal sealed class ReadFailureTally
    {
        public static ReadFailureTally Shared { get; } = new();

        private sealed class Entry
        {
            public int Count;
            public string FirstSubject = "";
            public string FirstError = "";
        }

        private readonly ConcurrentDictionary<string, Entry> _entries = new(StringComparer.Ordinal);

        public static bool IsReadFailure(Exception ex) =>
            ex is RuntimeBinderException
                or NullReferenceException
                or InvalidCastException
                or TargetInvocationException
                or AmbiguousMatchException
                or ArgumentException
                or InvalidOperationException
                or InvalidDataException
                or IndexOutOfRangeException
                or KeyNotFoundException
                or OverflowException
                or FormatException;

        public int Total => _entries.Values.Sum(e => { lock (e) return e.Count; });

        public int CountFor(string site)
        {
            if (!_entries.TryGetValue(site, out var e)) return 0;
            lock (e) return e.Count;
        }

        public void Record(string site, Exception ex, string? subject = null)
        {
            var entry = _entries.GetOrAdd(site, _ => new Entry());
            lock (entry)
            {
                if (entry.Count == 0)
                {
                    entry.FirstSubject = subject ?? "";
                    entry.FirstError = $"{ex.GetType().Name}: {ex.Message}";
                }
                entry.Count++;
            }
        }

        /// <summary>Runs <paramref name="read"/>; a known read failure is tallied against <paramref name="site"/> and yields <paramref name="fallback"/>.</summary>
        public T Read<T>(string site, string? subject, Func<T> read, T fallback)
        {
            try
            {
                return read();
            }
            catch (Exception ex) when (IsReadFailure(ex))
            {
                Record(site, ex, subject);
                return fallback;
            }
        }

        public IReadOnlyList<string> FormatSummary() =>
            _entries
                .OrderBy(kv => kv.Key, StringComparer.Ordinal)
                .Select(kv => $"  WARNING: {kv.Value.Count} unreadable reads at {kv.Key} (first: {kv.Value.FirstSubject} - {kv.Value.FirstError})")
                .ToList();

        public void WriteSummaryAndReset(TextWriter writer)
        {
            foreach (var line in FormatSummary()) writer.WriteLine(line);
            _entries.Clear();
        }
    }
}
