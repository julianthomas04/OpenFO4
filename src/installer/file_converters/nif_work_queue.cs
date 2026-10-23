using System.Runtime.ExceptionServices;

namespace AssetConverter
{
    /// <summary>
    /// The NIF files waiting for conversion, handed from whoever produces them (the archive extraction, the files already staged) to the
    /// conversion workers while the producers are still running. Priority files (see <see cref="PriorityFiles"/>) are served before the
    /// others among those queued; each file is served once. The queue is finished when every producer has called <see cref="Complete"/>
    /// and is then drained; a producer that fails calls <see cref="Fail"/>, which ends the queue at once for everyone waiting on it.
    /// </summary>
    internal sealed class NifWorkQueue
    {
        private readonly object _gate = new();
        private readonly Queue<string> _priority = new();
        private readonly Queue<string> _normal = new();
        private readonly HashSet<string> _announced = new(StringComparer.OrdinalIgnoreCase);
        private readonly HashSet<string> _queued = new(StringComparer.OrdinalIgnoreCase);
        private readonly TaskCompletionSource _ended = new(TaskCreationOptions.RunContinuationsAsynchronously);
        private readonly PriorityFiles? _priorityFiles;
        private int _openProducers;
        private Exception? _failure;

        /// <param name="producers">How many producers call <see cref="Complete"/> before the queue is finished.</param>
        public NifWorkQueue(int producers = 1, PriorityFiles? priorityFiles = null)
        {
            ArgumentOutOfRangeException.ThrowIfNegativeOrZero(producers);
            _openProducers = producers;
            _priorityFiles = priorityFiles;
        }

        /// <summary>Every file that is, or is announced to be, part of the conversion: what the progress total counts.</summary>
        public int ExpectedCount
        {
            get
            {
                lock (_gate) return _announced.Count;
            }
        }

        /// <summary>Snapshot of the files counted by <see cref="ExpectedCount"/>.</summary>
        public IReadOnlyCollection<string> Expected
        {
            get
            {
                lock (_gate) return [.. _announced];
            }
        }

        /// <summary>
        /// Declares files that <see cref="Add"/> will deliver later, so the total of the conversion is known before they arrive and
        /// <see cref="AddExisting"/> leaves them to their producer.
        /// </summary>
        public void Announce(IEnumerable<string> paths)
        {
            lock (_gate)
            {
                ThrowIfEnded();
                _announced.UnionWith(paths);
            }
        }

        /// <exception cref="InvalidOperationException">The file was queued before, or the queue is finished.</exception>
        public void Add(string path)
        {
            lock (_gate)
            {
                ThrowIfEnded();
                Enqueue(path);
                Monitor.Pulse(_gate);
            }
        }

        /// <summary>Queues the files that no producer announced, which is how files left by an earlier run join the conversion; returns how many.</summary>
        public int AddExisting(IEnumerable<string> paths)
        {
            int added = 0;
            lock (_gate)
            {
                ThrowIfEnded();
                foreach (string path in paths)
                {
                    if (_announced.Contains(path)) continue;
                    Enqueue(path);
                    added++;
                }
                Monitor.PulseAll(_gate);
            }
            return added;
        }

        /// <summary>One producer has queued everything it will; the last one to call this finishes the queue.</summary>
        public void Complete()
        {
            lock (_gate)
            {
                if (_openProducers == 0) throw new InvalidOperationException("The queue is already finished.");
                if (--_openProducers > 0) return;
                _ended.TrySetResult();
                Monitor.PulseAll(_gate);
            }
        }

        /// <summary>Ends the queue because a producer failed: everyone waiting on it, now and later, gets <paramref name="cause"/> instead of more work.</summary>
        public void Fail(Exception cause)
        {
            lock (_gate)
            {
                _failure ??= cause;
                _ended.TrySetResult();
                Monitor.PulseAll(_gate);
            }
        }

        /// <summary>The exception a producer ended the queue with, or null while no producer failed.</summary>
        public Exception? Failure
        {
            get
            {
                lock (_gate) return _failure;
            }
        }

        /// <summary>Whether every producer is done; the files that remain queued are still served.</summary>
        public bool IsComplete => _ended.Task.IsCompleted;

        /// <summary>Blocks until the queue is finished or failed; the failure itself is raised by <see cref="TryTake"/>.</summary>
        public void WaitUntilEnded(CancellationToken ct) => _ended.Task.Wait(ct);

        /// <summary>
        /// Takes the next file, waiting for one to arrive; priority files first. Returns false once the queue is finished and drained.
        /// </summary>
        /// <exception cref="OperationCanceledException"><paramref name="ct"/> was cancelled while waiting.</exception>
        /// <exception cref="Exception">The exception a producer failed with.</exception>
        public bool TryTake(out string path, CancellationToken ct)
        {
            using var wake = ct.UnsafeRegister(static state => ((NifWorkQueue)state!).WakeWaiters(), this);
            lock (_gate)
            {
                PollResult outcome;
                while ((outcome = Poll(out path, ct)) == PollResult.Wait)
                    Monitor.Wait(_gate);
                return outcome == PollResult.File;
            }
        }

        private void WakeWaiters()
        {
            lock (_gate) Monitor.PulseAll(_gate);
        }

        private enum PollResult
        {
            File,
            Drained,
            Wait,
        }

        private PollResult Poll(out string path, CancellationToken ct)
        {
            path = "";
            if (_failure != null) ExceptionDispatchInfo.Capture(_failure).Throw();
            if (ct.IsCancellationRequested)
            {
                Monitor.Pulse(_gate);
                ct.ThrowIfCancellationRequested();
            }

            if (_priority.TryDequeue(out string? next) || _normal.TryDequeue(out next))
            {
                path = next;
                return PollResult.File;
            }
            return _openProducers == 0 ? PollResult.Drained : PollResult.Wait;
        }

        private void Enqueue(string path)
        {
            if (!_queued.Add(path)) throw new InvalidOperationException($"'{path}' was queued twice.");
            _announced.Add(path);
            (_priorityFiles?.Wants(path) == true ? _priority : _normal).Enqueue(path);
        }

        private void ThrowIfEnded()
        {
            if (_failure != null) ExceptionDispatchInfo.Capture(_failure).Throw();
            if (_openProducers == 0) throw new InvalidOperationException("The queue is already finished.");
        }
    }
}
