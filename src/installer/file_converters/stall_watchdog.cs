namespace AssetConverter
{
    /// <summary>A running process as the watchdog sees it.</summary>
    public interface IWatchedProcess
    {
        /// <summary>Waits up to <paramref name="timeout"/>; true when the process has exited.</summary>
        bool WaitForExit(TimeSpan timeout);

        /// <summary>A number that changes while the process works: output received, files written.</summary>
        long Progress();
    }

    /// <summary>
    /// Tells a stalled process from a slow one: the process is stalled once <see cref="Limit"/> passes without its progress changing,
    /// or, before it has shown any progress at all, once the longer <see cref="StartupLimit"/> passes.
    /// Unlike a fixed wall-clock limit, the time a job may take is not bounded, only the time it may be silent.
    /// </summary>
    public sealed class StallWatchdog
    {
        readonly Func<TimeSpan> _clock;
        long _lastProgress;
        TimeSpan _lastChange;

        public StallWatchdog(TimeSpan limit, TimeSpan startupLimit, Func<TimeSpan> clock)
        {
            Limit = limit > TimeSpan.Zero ? limit : throw new ArgumentOutOfRangeException(nameof(limit), "The stall limit must be positive.");
            StartupLimit = startupLimit >= limit ? startupLimit : throw new ArgumentOutOfRangeException(nameof(startupLimit), "The startup limit must not be shorter than the stall limit.");
            _clock = clock;
            _lastChange = clock();
        }

        /// <summary>How long a process that has shown progress may go without it changing.</summary>
        public TimeSpan Limit { get; }

        /// <summary>How long a process may take to show its first progress.</summary>
        public TimeSpan StartupLimit { get; }

        /// <summary>Whether any progress has been seen yet.</summary>
        public bool HasStarted { get; private set; }

        /// <summary>The limit that currently applies: <see cref="StartupLimit"/> until the first progress, then <see cref="Limit"/>.</summary>
        public TimeSpan ActiveLimit => HasStarted ? Limit : StartupLimit;

        /// <summary>Records the progress seen now (a process starts at 0); true when it has not changed for <see cref="ActiveLimit"/>.</summary>
        public bool IsStalled(long progress)
        {
            TimeSpan now = _clock();
            if (progress != _lastProgress)
            {
                _lastProgress = progress;
                _lastChange = now;
                HasStarted = true;
                return false;
            }
            return now - _lastChange >= ActiveLimit;
        }

        /// <summary>Waits for the process to exit, polling its progress; false when it stalled (and is still running).</summary>
        public bool WaitForExit(IWatchedProcess process, TimeSpan pollInterval)
        {
            while (!process.WaitForExit(pollInterval))
                if (IsStalled(process.Progress()))
                    return false;
            return true;
        }
    }
}
