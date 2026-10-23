using System.Collections.Concurrent;
using System.Diagnostics;
using System.Globalization;

namespace AssetConverter
{
    /// <summary>
    /// Process-wide, opt-in (<c>OPENFO4_ESM_TIMING=1</c>) <see cref="EsmTimingCollector"/> the ESM import reports through.
    /// </summary>
    internal static class EsmTiming
    {
        private const string EnvironmentVariable = "OPENFO4_ESM_TIMING";

        private static readonly EsmTimingCollector Shared = new(Environment.GetEnvironmentVariable(EnvironmentVariable) == "1");

        public static bool Enabled => Shared.Enabled;

        public static long Now() => Shared.Now();

        public static EsmTimingCollector.Section Measure(string key) => Shared.Measure(key);

        public static void Add(string key, long startTimestamp, long count = 1) => Shared.Add(key, startTimestamp, count);

        public static EsmTimingCollector.Phase MeasurePhase(string key) => Shared.MeasurePhase(key);

        /// <summary>Times <paramref name="body"/> as <c>total</c> and prints the report even when it throws.</summary>
        public static Task RunReportedAsync(Func<Task> body) => Shared.RunReportedAsync(body, Console.Out);
    }

    /// <summary>
    /// Accumulator for the ESM import: named sections add up wall-clock time and a count from any thread, and
    /// <see cref="Report"/> prints one table. Sections timed on several threads at once sum their thread time, so they
    /// can exceed the wall time of the phase that contains them. A disabled collector records and prints nothing.
    /// </summary>
    internal sealed class EsmTimingCollector(bool enabled)
    {
        private const string RankedPrefix = "wave1.job.";
        private const int RankedLimit = 12;

        private sealed class Entry(int order)
        {
            public readonly int Order = order;
            public long Ticks;
            public long Count;
            public long MaxTicks;
            public long GcPauseTicks;
            public long AllocatedBytes;
        }

        private readonly ConcurrentDictionary<string, Entry> _entries = new(StringComparer.Ordinal);
        private int _nextOrder;

        public bool Enabled { get; } = enabled;

        public long Now() => Enabled ? Stopwatch.GetTimestamp() : 0;

        public Section Measure(string key) => new(this, key, Now());

        public void Add(string key, long startTimestamp, long count = 1)
        {
            if (!Enabled) return;
            Record(key, Stopwatch.GetTimestamp() - startTimestamp, count, 0, 0);
        }

        /// <summary>Like <see cref="Measure"/>, but also attributes the process-wide GC pause time and allocated bytes that elapsed meanwhile.</summary>
        public Phase MeasurePhase(string key) => new(this, key, Now(),
            Enabled ? GC.GetTotalPauseDuration() : TimeSpan.Zero, Enabled ? GC.GetTotalAllocatedBytes() : 0);

        private void Record(string key, long ticks, long count, long gcPauseTicks, long allocatedBytes)
        {
            var entry = _entries.GetOrAdd(key, _ => new Entry(Interlocked.Increment(ref _nextOrder)));
            Interlocked.Add(ref entry.Ticks, ticks);
            Interlocked.Add(ref entry.Count, count);
            Interlocked.Add(ref entry.GcPauseTicks, gcPauseTicks);
            Interlocked.Add(ref entry.AllocatedBytes, allocatedBytes);
            long seen;
            while (ticks > (seen = Interlocked.Read(ref entry.MaxTicks)) &&
                   Interlocked.CompareExchange(ref entry.MaxTicks, ticks, seen) != seen)
            {
            }
        }

        private void Reset()
        {
            _entries.Clear();
            Volatile.Write(ref _nextOrder, 0);
        }

        public async Task RunReportedAsync(Func<Task> body, TextWriter writer)
        {
            Reset();
            long start = Now();
            try
            {
                await body();
            }
            finally
            {
                Add("total", start);
                Report(writer);
            }
        }

        public void Report(TextWriter writer)
        {
            if (!Enabled) return;

            var ordered = _entries.OrderBy(kv => kv.Value.Order).ToList();
            writer.WriteLine("ESM timing (OPENFO4_ESM_TIMING=1); sections on several threads sum thread time");
            writer.WriteLine($"  {"section",-52} {"seconds",9} {"count",11} {"avg us",9} {"max ms",9} {"gc s",7} {"alloc MB",9}");

            foreach (var (key, entry) in ordered.Where(kv => !kv.Key.StartsWith(RankedPrefix, StringComparison.Ordinal)))
                writer.WriteLine(FormatRow(key, entry));

            var ranked = ordered.Where(kv => kv.Key.StartsWith(RankedPrefix, StringComparison.Ordinal)).ToList();
            if (ranked.Count > 0)
            {
                writer.WriteLine($"  wave-1 jobs: {ranked.Count} jobs, {Seconds(ranked.Sum(kv => kv.Value.Ticks)):F1} s of thread time; slowest {Math.Min(RankedLimit, ranked.Count)}:");
                foreach (var (key, entry) in ranked.OrderByDescending(kv => kv.Value.Ticks).Take(RankedLimit))
                    writer.WriteLine(FormatRow(key, entry));
            }

            writer.WriteLine(FormatProcessCounters());
        }

        private static string FormatRow(string key, Entry entry)
        {
            double avgMicroseconds = entry.Count > 0 ? entry.Ticks * 1_000_000.0 / Stopwatch.Frequency / entry.Count : 0;
            return string.Create(CultureInfo.InvariantCulture,
                $"  {key,-52} {Seconds(entry.Ticks),9:F2} {entry.Count,11} {avgMicroseconds,9:F1} {entry.MaxTicks * 1000.0 / Stopwatch.Frequency,9:F1} {Seconds(entry.GcPauseTicks),7:F2} {entry.AllocatedBytes / 1048576.0,9:F0}");
        }

        private static string FormatProcessCounters()
        {
            using var process = Process.GetCurrentProcess();
            return string.Create(CultureInfo.InvariantCulture,
                $"  process: GC pause total {GC.GetTotalPauseDuration().TotalSeconds:F2} s, collections gen0/1/2 {GC.CollectionCount(0)}/{GC.CollectionCount(1)}/{GC.CollectionCount(2)}, " +
                $"allocated {GC.GetTotalAllocatedBytes() / 1048576.0:F0} MB, peak working set {process.PeakWorkingSet64 / 1048576.0:F0} MB, " +
                $"CPU time {process.TotalProcessorTime.TotalSeconds:F1} s, logical cores {Environment.ProcessorCount}");
        }

        private static double Seconds(long ticks) => ticks / (double)Stopwatch.Frequency;

        public readonly struct Section(EsmTimingCollector owner, string key, long start) : IDisposable
        {
            public void Dispose() => owner.Add(key, start);
        }

        public readonly struct Phase(EsmTimingCollector owner, string key, long start, TimeSpan gcPauseAtStart, long allocatedAtStart) : IDisposable
        {
            public void Dispose()
            {
                if (!owner.Enabled) return;
                owner.Record(key, Stopwatch.GetTimestamp() - start, 1,
                    (GC.GetTotalPauseDuration() - gcPauseAtStart).Ticks * Stopwatch.Frequency / TimeSpan.TicksPerSecond,
                    GC.GetTotalAllocatedBytes() - allocatedAtStart);
            }
        }
    }
}
