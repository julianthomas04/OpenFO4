using System.Globalization;
using System.Text.RegularExpressions;
using AssetConverter.Progress;

namespace AssetConverter
{
    /// <summary>
    /// Turns the collision worker's stdout into step progress: its "N NIFs to process" line sets the total (a streaming worker prints none; the
    /// installer adds each NIF it sends instead) and each "... done/N NIFs processed" line ticks the NIFs since the previous one. Safe to call from the process's output-event thread and
    /// from the thread that completes the run at once; a line it cannot read is reported and never thrown, since an exception in the
    /// output event handler would end the installer.
    /// </summary>
    internal sealed partial class CollisionWorkerProgress
    {
        private readonly int _phase;
        private readonly Action<string> _warn;
        private readonly object _lock = new();
        private long _total;
        private long _reported;

        public CollisionWorkerProgress(int phase, Action<string>? warn = null)
        {
            _phase = phase;
            _warn = warn ?? Console.Error.WriteLine;
        }

        [GeneratedRegex(@"^\s*(\d+) NIFs to process\b")]
        private static partial Regex TotalLine();

        [GeneratedRegex(@"^\s*\.\.\. (\d+)/\d+ NIFs processed\b")]
        private static partial Regex DoneLine();

        public void Handle(string line)
        {
            var total = TotalLine().Match(line);
            if (total.Success)
            {
                if (TryParseCount(total.Groups[1].Value, line, out long count))
                    AddTotal(count);
                return;
            }

            var done = DoneLine().Match(line);
            if (done.Success && TryParseCount(done.Groups[1].Value, line, out long processed))
                ReportDone(processed);
        }

        /// <summary>The worker exited successfully: NIFs after its last progress line are done too.</summary>
        public void Complete() => ReportDone(null);

        private bool TryParseCount(string digits, string line, out long count)
        {
            if (long.TryParse(digits, NumberStyles.None, CultureInfo.InvariantCulture, out count)) return true;
            _warn($"WARNING: ignoring a collision worker progress line with an unreadable count: {line.Trim()}");
            return false;
        }

        /// <summary>Counts NIFs more as work of the step: the worker's own total line, or NIFs the installer sends to a streaming worker.</summary>
        public void AddTotal(long count)
        {
            lock (_lock) _total += count;
            InstallProgress.AddTotal(_phase, count);
        }

        /// <summary>Ticks the NIFs done since the last report, up to the total; <c>null</c> means all of them.</summary>
        private void ReportDone(long? done)
        {
            long delta;
            lock (_lock)
            {
                delta = Math.Min(done ?? _total, _total) - _reported;
                if (delta <= 0) return;
                _reported += delta;
            }
            InstallProgress.Tick(_phase, delta);
        }
    }
}
