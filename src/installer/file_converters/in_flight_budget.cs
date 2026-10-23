namespace AssetConverter
{
    /// <summary>
    /// Backpressure for a producer/consumer pipeline: <see cref="Take"/> blocks while more than the cap is in flight
    /// (a single item larger than the cap is still admitted when nothing else is queued) and is released by
    /// <see cref="Return"/> or by cancellation, so a consumer that stops early cannot strand the producer.
    /// </summary>
    internal sealed class InFlightBudget : IDisposable
    {
        private readonly long _maxInFlightBytes;
        private readonly object _lock = new();
        private readonly ManualResetEventSlim _hasRoom = new(true);
        private long _inFlight;

        public InFlightBudget(long maxInFlightBytes)
        {
            _maxInFlightBytes = maxInFlightBytes;
        }

        public long InFlightBytes
        {
            get { lock (_lock) return _inFlight; }
        }

        public void Take(long bytes, CancellationToken ct)
        {
            while (true)
            {
                lock (_lock)
                {
                    if (_inFlight <= _maxInFlightBytes)
                    {
                        _inFlight += bytes;
                        return;
                    }
                    _hasRoom.Reset();
                }
                _hasRoom.Wait(ct);
            }
        }

        public void Return(long bytes)
        {
            lock (_lock)
            {
                _inFlight -= bytes;
                if (_inFlight <= _maxInFlightBytes)
                    _hasRoom.Set();
            }
        }

        public void Dispose() => _hasRoom.Dispose();
    }
}
