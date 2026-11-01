using System.Globalization;
using System.Text.RegularExpressions;
using AssetConverter.Progress;

namespace AssetConverter
{
    /// <summary>
    /// Turns the collision worker's stdout into step progress against a total known before the worker starts: each
    /// "... done/N NIFs processed" line ticks the NIFs since the previous one, up to the total, and finishing the worker ticks the rest.
    /// </summary>
    internal sealed partial class CollisionWorkerProgress
    {
        private readonly int _phase;
        private readonly long _total;
        private readonly Action<string> _warn;
        private readonly object _lock = new();
        private long _reported;
        private long _processed;
        private bool _finished;

        /// <param name="phase">The progress phase, which began with <paramref name="total"/> as its total.</param>
        public CollisionWorkerProgress(int phase, long total, Action<string>? warn = null)
        {
            ArgumentOutOfRangeException.ThrowIfNegative(total);
            _phase = phase;
            _total = total;
            _warn = warn ?? Console.Error.WriteLine;
        }

        [GeneratedRegex(@"^\s*\.\.\. (\d+)/\d+ NIFs processed\b")]
        private static partial Regex DoneLine();

        public void Handle(string line)
        {
            var done = DoneLine().Match(line);
            if (done.Success && TryParseCount(done.Groups[1].Value, line, out long processedCount))
                ReportDone(processedCount);
        }

        /// <summary>The worker exited successfully: NIFs after its last progress line are done too.</summary>
        public void Complete()
        {
            lock (_lock) _finished = true;
            ReportDone(null);
        }

        private bool TryParseCount(string digits, string line, out long count)
        {
            if (long.TryParse(digits, NumberStyles.None, CultureInfo.InvariantCulture, out count)) return true;
            _warn($"WARNING: ignoring a collision worker progress line with an unreadable count: {line.Trim()}");
            return false;
        }

        /// <summary>Ticks the NIFs done since the last report, up to the total; <c>null</c> means all of them.</summary>
        private void ReportDone(long? done)
        {
            long delta;
            lock (_lock)
            {
                if (done is { } processed) _processed = Math.Max(_processed, processed);
                delta = Math.Min(_finished ? _total : _processed, _total) - _reported;
                if (delta <= 0) return;
                _reported += delta;
            }
            InstallProgress.Tick(_phase, delta);
        }
    }
}
