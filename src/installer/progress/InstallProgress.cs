namespace AssetConverter.Progress
{
    /// <summary>
    /// Process-wide entry point the converters report through. Before <see cref="Start"/> (the dev subcommands) every call is
    /// a no-op and <see cref="BeginPhase"/> returns -1, which the other calls ignore.
    /// </summary>
    public static class InstallProgress
    {
        private static readonly object _gate = new();
        private static volatile ProgressModel? _model;
        private static CancellationTokenSource _cancel = new();
        private static readonly HashSet<Exception> _reported = new(ReferenceEqualityComparer.Instance);
        private static volatile Exception? _failure;
        private static bool _cancelledByUser;

        public static ProgressModel? Model => _model;

        public static CancellationToken CancelToken => _cancel.Token;

        /// <summary>The first real failure of the run, if any; later errors caused by the resulting cancellation do not replace it.</summary>
        public static Exception? Failure => _failure;

        public static ProgressModel Start(IReadOnlyList<StepDefinition> steps)
        {
            lock (_gate)
            {
                _cancel.Dispose();
                _cancel = new CancellationTokenSource();
                _failure = null;
                _cancelledByUser = false;
                _reported.Clear();
                var model = new ProgressModel(steps);
                _model = model;
                return model;
            }
        }

        public static void Stop()
        {
            lock (_gate) _model = null;
        }

        /// <summary>
        /// Cancels the run on behalf of the user. Never throws: a callback that fails while the token fires is logged and the
        /// cancellation still stands. A failure that happened first stays the run's outcome.
        /// </summary>
        public static void RequestCancel()
        {
            lock (_gate)
            {
                if (_failure == null) _cancelledByUser = true;
            }
            FireCancel();
        }

        public static int BeginPhase(string name, long total, string? unit = null) => _model?.BeginPhase(name, total, unit) ?? -1;

        public static void AddTotal(int phase, long count) => _model?.AddTotal(phase, count);

        public static void Tick(int phase, long count = 1) => _model?.Tick(phase, count);

        public static void SetDetail(int phase, string text) => _model?.SetDetail(phase, text);

        public static void EndPhase(int phase) => _model?.EndPhase(phase);

        /// <summary>
        /// Runs one concurrent lane of the install. A failure marks the lane's running steps as failed and cancels the other lanes
        /// so the run stops at once; a cancellation, and whatever the cancelling tears down, marks them cancelled.
        /// Exceptions always propagate.
        /// </summary>
        public static async Task RunLane(IReadOnlyList<string> stepNames, Func<Task> body)
        {
            try
            {
                await body().ConfigureAwait(false);
            }
            catch (Exception ex)
            {
                if (Observe(ex) == Cause.Failure) _model?.Fail(stepNames);
                else _model?.Cancel(stepNames);
                throw;
            }
        }

        /// <summary>
        /// Closes the run with the exception that ended it (null when the work simply returned) and returns the outcome:
        /// a failure wins over everything, then any cancellation, even when the remaining work finished anyway.
        /// </summary>
        public static RunState Conclude(Exception? ended)
        {
            if (ended != null) Observe(ended);
            RunState outcome = Failure != null ? RunState.Failed
                : _cancel.IsCancellationRequested ? RunState.Cancelled
                : RunState.Succeeded;
            _model?.Finish(outcome);
            return outcome;
        }

        private enum Cause
        {
            Cancellation,
            CancellationFallout,
            Failure,
        }

        /// <summary>
        /// Classifies an exception that ended work and reports it. Only a <see cref="Cause.Failure"/> becomes the run's failure:
        /// an exception that is the cancellation itself, or that happened after the user cancelled and before anything failed
        /// (disposed objects, killed processes), is the cancellation's fallout.
        /// </summary>
        private static Cause Observe(Exception ex)
        {
            if (_cancel.IsCancellationRequested && IsCancellation(ex)) return Cause.Cancellation;
            bool fallout;
            lock (_gate) fallout = _cancelledByUser && _failure == null;
            if (!fallout)
            {
                RecordFailure(ex);
                return Cause.Failure;
            }
            ReportOnce(ex, $"WARNING: ignored after the cancellation: {ex}");
            return Cause.CancellationFallout;
        }

        private static bool IsCancellation(Exception ex) => ex switch
        {
            OperationCanceledException => true,
            AggregateException aggregate => aggregate.Flatten().InnerExceptions is { Count: > 0 } inner && inner.All(IsCancellation),
            _ => false,
        };

        private static void RecordFailure(Exception ex)
        {
            bool first;
            lock (_gate)
            {
                first = _failure == null;
                _failure ??= ex;
            }
            ReportOnce(ex, first ? $"ERROR: installation failed: {ex}" : $"ERROR: {ex}");
            if (first) FireCancel();
        }

        private static void ReportOnce(Exception ex, string message)
        {
            bool isNew;
            lock (_gate) isNew = _reported.Add(ex);
            if (isNew) Console.Error.WriteLine(message);
        }

        private static void FireCancel()
        {
            try
            {
                _cancel.Cancel();
            }
            catch (Exception ex)
            {
                Console.Error.WriteLine($"ERROR: a cancellation callback failed: {ex}");
            }
        }
    }
}
