namespace AssetConverter.Logging
{
    /// <summary>Lines newer than the position the reader passed, and the position to pass next time.</summary>
    public sealed record LogRead(IReadOnlyList<LogLine> Lines, long LastSequence, long Missed);

    /// <summary>Thread-safe ring buffer of the most recent log lines; older lines are overwritten once the capacity is reached.</summary>
    public sealed class LogBuffer : ILineSink
    {
        private readonly object _gate = new();
        private readonly LogLine[] _ring;
        private readonly TimeProvider _clock;
        private long _lastSequence;

        public LogBuffer(int capacity, TimeProvider? clock = null)
        {
            ArgumentOutOfRangeException.ThrowIfLessThanOrEqual(capacity, 0);
            _ring = new LogLine[capacity];
            _clock = clock ?? TimeProvider.System;
        }

        public void WriteLine(string text, bool isError)
        {
            ArgumentNullException.ThrowIfNull(text);
            lock (_gate)
            {
                long sequence = ++_lastSequence;
                _ring[(sequence - 1) % _ring.Length] = new LogLine(sequence, _clock.GetUtcNow(), isError, text);
            }
        }

        /// <summary>Returns every retained line after <paramref name="afterSequence"/> (0 for all), oldest first. <c>Missed</c> counts lines overwritten before they were read.</summary>
        public LogRead ReadAfter(long afterSequence)
        {
            lock (_gate)
            {
                long oldestRetained = Math.Max(1, _lastSequence - _ring.Length + 1);
                long first = Math.Max(afterSequence + 1, oldestRetained);
                long missed = Math.Max(0, oldestRetained - afterSequence - 1);
                var lines = new List<LogLine>((int)Math.Max(0, _lastSequence - first + 1));
                for (long sequence = first; sequence <= _lastSequence; sequence++)
                    lines.Add(_ring[(sequence - 1) % _ring.Length]);
                return new LogRead(lines, _lastSequence, missed);
            }
        }
    }
}
