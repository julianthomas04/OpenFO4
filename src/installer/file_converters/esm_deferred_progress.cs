using System.Collections.Concurrent;
using AssetConverter.Progress;

namespace AssetConverter
{
    /// <summary>
    /// BRIDGE: holds back the database load's counts until the load is complete, then reports them as one sealed total. The load still
    /// discovers its work while it runs, which the progress model no longer accepts; to be replaced by a discovery pass before the load.
    /// </summary>
    internal static class DeferredPhaseProgress
    {
        private sealed class Counts
        {
            public long Total;
            public long Completed;
        }

        private static readonly ConcurrentDictionary<int, Counts> Pending = new();

        public static void AddTotal(int phase, long count)
        {
            if (phase >= 0) Interlocked.Add(ref Pending.GetOrAdd(phase, _ => new Counts()).Total, count);
        }

        public static void Tick(int phase, long count = 1)
        {
            if (phase >= 0) Interlocked.Add(ref Pending.GetOrAdd(phase, _ => new Counts()).Completed, count);
        }

        public static void Flush(int phase)
        {
            if (phase < 0 || !Pending.TryRemove(phase, out var counts)) return;
            InstallProgress.AddTotal(phase, counts.Total);
            InstallProgress.SealTotal(phase);
            InstallProgress.Tick(phase, counts.Completed);
        }
    }
}
