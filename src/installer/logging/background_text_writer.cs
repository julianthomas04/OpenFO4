namespace AssetConverter.Logging
{
    /// <summary>
    /// Forwards text to a writer on a dedicated thread, in order. <see cref="Write"/> never blocks: when the target stalls (a console in
    /// selection mode, a full pipe) and the queue is full, the oldest queued text is dropped and reported once the target accepts text again.
    /// A target that fails, for whatever reason, is reported once and not written to again. Flush requests are kept apart from the text, so
    /// they are never dropped and never push text out of the queue.
    /// </summary>
    internal sealed class BackgroundTextWriter
    {
        private readonly record struct FlushRequest(long Watermark, TaskCompletionSource Completed);

        private readonly object _gate = new();
        private readonly Queue<string> _queue = new();
        private readonly List<FlushRequest> _flushes = new();
        private readonly TextWriter _target;
        private readonly Action<string> _onFault;
        private readonly int _capacity;
        private readonly Task _loop;
        private long _accepted;
        private long _retired;
        private long _dropped;
        private bool _closed;
        private bool _faulted;

        public BackgroundTextWriter(TextWriter target, int capacity, Action<string> onFault)
        {
            ArgumentNullException.ThrowIfNull(target);
            ArgumentNullException.ThrowIfNull(onFault);
            ArgumentOutOfRangeException.ThrowIfLessThanOrEqual(capacity, 0);
            _target = target;
            _onFault = onFault;
            _capacity = capacity;
            _loop = Task.Factory.StartNew(Run, TaskCreationOptions.LongRunning);
        }

        /// <summary>Queues text for the target; text written after <see cref="Close"/> is not forwarded.</summary>
        public void Write(string text)
        {
            if (text.Length == 0) return;
            lock (_gate)
            {
                if (_closed) return;
                _accepted++;
                _queue.Enqueue(text);
                if (_queue.Count > _capacity)
                {
                    _queue.Dequeue();
                    _retired++;
                    _dropped++;
                }
                Monitor.Pulse(_gate);
            }
        }

        /// <summary>
        /// Waits up to <paramref name="timeout"/> until everything queued so far has reached the target (or was dropped) and the target has
        /// been flushed; false when the target did not get there in time. Returns at once, true, after <see cref="Close"/>.
        /// </summary>
        public bool Flush(TimeSpan timeout)
        {
            var completed = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
            lock (_gate)
            {
                if (_closed) return true;
                _flushes.Add(new FlushRequest(_accepted, completed));
                Monitor.Pulse(_gate);
            }
            return completed.Task.Wait(timeout);
        }

        /// <summary>Stops accepting text and waits up to <paramref name="timeout"/> for the queue to reach the target; false when it did not.</summary>
        public bool Close(TimeSpan timeout)
        {
            lock (_gate)
            {
                _closed = true;
                Monitor.Pulse(_gate);
            }
            return _loop.Wait(timeout);
        }

        private void Run()
        {
            bool wrote = false;
            while (true)
            {
                var work = NextWork(wrote);
                wrote = false;
                if (work.Flushes != null)
                {
                    TryTarget(_target.Flush);
                    foreach (var request in work.Flushes) request.Completed.TrySetResult();
                }
                else if (work.Text != null)
                {
                    if (work.Dropped > 0) _onFault($"WARNING: the console did not accept output for a while; {work.Dropped:N0} writes were dropped from it (the log file has them all).");
                    TryTarget(() => _target.Write(work.Text));
                    wrote = true;
                }
                else
                {
                    break;
                }
            }
            TryTarget(_target.Flush);
            CompleteAllFlushes();
        }

        private (string? Text, long Dropped, List<FlushRequest>? Flushes) NextWork(bool previousWritten)
        {
            lock (_gate)
            {
                if (previousWritten) _retired++;
                while (true)
                {
                    var due = TakeDueFlushes();
                    if (due != null) return (null, 0, due);
                    if (_queue.TryDequeue(out string? text))
                    {
                        long dropped = _dropped;
                        _dropped = 0;
                        return (text, dropped, null);
                    }
                    if (_closed) return (null, 0, null);
                    Monitor.Wait(_gate);
                }
            }
        }

        private List<FlushRequest>? TakeDueFlushes()
        {
            var due = _flushes.FindAll(request => request.Watermark <= _retired);
            if (due.Count == 0) return null;
            _flushes.RemoveAll(request => request.Watermark <= _retired);
            return due;
        }

        private void CompleteAllFlushes()
        {
            lock (_gate)
            {
                foreach (var request in _flushes) request.Completed.TrySetResult();
                _flushes.Clear();
            }
        }

        private void TryTarget(Action write)
        {
            if (_faulted) return;
            try
            {
                write();
            }
            catch (Exception ex)
            {
                _faulted = true;
                _onFault($"WARNING: writing to the console failed; console output is disabled for the rest of the run, the log file continues. {ex}");
            }
        }
    }
}
