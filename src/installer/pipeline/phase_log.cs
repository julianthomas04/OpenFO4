using System.Collections.Concurrent;
using System.Diagnostics;

namespace AssetConverter.Pipeline
{
    /// <summary>A named stretch of the install, as offsets from the start of the run.</summary>
    internal readonly record struct PhaseEntry(string Name, TimeSpan Start, TimeSpan End);

    /// <summary>The wall-clock timeline of the install's phases, which overlap because the lanes run concurrently.</summary>
    internal sealed class PhaseLog
    {
        private readonly Stopwatch _clock = Stopwatch.StartNew();
        private readonly ConcurrentBag<PhaseEntry> _entries = [];

        public TimeSpan Elapsed => _clock.Elapsed;

        public IReadOnlyList<PhaseEntry> Entries => [.. _entries.OrderBy(entry => entry.Start)];

        public void Stop() => _clock.Stop();

        /// <summary>Records a phase that began at <paramref name="start"/> and ends now.</summary>
        public void Record(string name, TimeSpan start) => _entries.Add(new PhaseEntry(name, start, Elapsed));

        /// <summary>Runs <paramref name="work"/> and records it afterwards; work that throws leaves no entry.</summary>
        public void Measure(string name, Action work) => Measure(name, () =>
        {
            work();
            return 0;
        });

        public T Measure<T>(string name, Func<T> work)
        {
            var start = Elapsed;
            T result = work();
            Record(name, start);
            return result;
        }
    }
}
