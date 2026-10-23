namespace InstallerTests.Support
{
    /// <summary>A clock that only moves when the test advances it, with millisecond timestamp resolution. Its timers fire on the advancing thread.</summary>
    internal sealed class ManualTimeProvider : TimeProvider
    {
        private readonly List<ManualTimer> _timers = [];
        private long _timestamp;
        private DateTimeOffset _utcNow = new(2026, 1, 1, 12, 0, 0, TimeSpan.Zero);

        public override long TimestampFrequency => 1000;

        public override long GetTimestamp() => Interlocked.Read(ref _timestamp);

        public override DateTimeOffset GetUtcNow() => _utcNow;

        public int ActiveTimerCount
        {
            get { lock (_timers) return _timers.Count; }
        }

        public override ITimer CreateTimer(TimerCallback callback, object? state, TimeSpan dueTime, TimeSpan period)
        {
            var timer = new ManualTimer(this, callback, state, GetTimestamp() + (long)dueTime.TotalMilliseconds, (long)period.TotalMilliseconds);
            lock (_timers)
                _timers.Add(timer);
            return timer;
        }

        /// <summary>Moves the clock and fires every timer that came due, once per elapsed period, in due order.</summary>
        public void Advance(TimeSpan by)
        {
            long target = GetTimestamp() + (long)by.TotalMilliseconds;
            while (NextDueTimer(target) is { } timer)
            {
                _utcNow += TimeSpan.FromMilliseconds(timer.DueAt - GetTimestamp());
                Interlocked.Exchange(ref _timestamp, timer.DueAt);
                timer.Fire();
            }

            _utcNow += TimeSpan.FromMilliseconds(target - GetTimestamp());
            Interlocked.Exchange(ref _timestamp, target);
        }

        private ManualTimer? NextDueTimer(long target)
        {
            lock (_timers)
                return _timers.Where(timer => timer.DueAt <= target).MinBy(timer => timer.DueAt);
        }

        private sealed class ManualTimer(ManualTimeProvider owner, TimerCallback callback, object? state, long dueAt, long period) : ITimer
        {
            public long DueAt { get; private set; } = dueAt;

            public void Fire()
            {
                if (period > 0)
                    DueAt += period;
                else
                    Dispose();
                callback(state);
            }

            public bool Change(TimeSpan dueTime, TimeSpan newPeriod) => throw new NotSupportedException();

            public void Dispose()
            {
                lock (owner._timers)
                    owner._timers.Remove(this);
            }

            public ValueTask DisposeAsync()
            {
                Dispose();
                return ValueTask.CompletedTask;
            }
        }
    }
}
