using System.Runtime.ExceptionServices;

namespace AssetConverter
{
    /// <summary>
    /// The threads that drain a <see cref="NifWorkQueue"/>. Only <see cref="OverlapWorkerCount"/> of them work while the queue still has
    /// producers (the archive extraction the files come from), at below-normal priority so the extraction keeps the CPU it needs; the
    /// others join, at normal priority, once the queue is finished. The workers are dedicated long-running threads rather than pool
    /// tasks: they block while the queue is empty and compute for minutes, which would starve the pool the extraction's own workers
    /// run on (the pool adds a thread about every half second).
    /// </summary>
    internal static class NifConversionWorkers
    {
        public const int OverlapWorkerDivisor = 4;
        public const int MinimumOverlapWorkers = 2;
        public const ThreadPriority OverlapPriority = ThreadPriority.BelowNormal;
        public const ThreadPriority SteadyPriority = ThreadPriority.Normal;

        /// <summary>The share of the machine the conversion takes while the extraction is still running; never more than <paramref name="totalWorkers"/>.</summary>
        public static int OverlapWorkerCount(int processorCount, int totalWorkers) =>
            Math.Min(totalWorkers, Math.Max(MinimumOverlapWorkers, processorCount / OverlapWorkerDivisor));

        /// <summary>
        /// Runs <paramref name="convert"/> on every file of <paramref name="queue"/> until it is drained or <paramref name="maxFiles"/> files
        /// were taken. <paramref name="convert"/> handles the failure of its own file; whatever it throws, an exception a queue producer
        /// failed with, or the cancellation of <paramref name="ct"/> stops all workers and is rethrown here once they have ended.
        /// </summary>
        public static void Run(NifWorkQueue queue, int overlapWorkers, int totalWorkers, Action<string> convert, int maxFiles, CancellationToken ct)
        {
            using var stop = CancellationTokenSource.CreateLinkedTokenSource(ct);
            Exception? firstFailure = null;
            var claims = new FileClaims(maxFiles);

            void Work(int workerId)
            {
                try
                {
                    WorkLoop(workerId, queue, overlapWorkers, convert, claims, stop.Token);
                }
                catch (OperationCanceledException) when (stop.IsCancellationRequested)
                {
                    // Stopped by the caller or by another worker's failure, which is rethrown below.
                }
                catch (Exception ex)
                {
                    Interlocked.CompareExchange(ref firstFailure, ex, null);
                    stop.Cancel();
                }
            }

            var workers = Enumerable.Range(0, totalWorkers).Select(workerId => Task.Factory.StartNew(
                () => Work(workerId), CancellationToken.None, TaskCreationOptions.LongRunning, TaskScheduler.Default)).ToArray();
            Task.WaitAll(workers, CancellationToken.None);

            if (firstFailure != null) ExceptionDispatchInfo.Capture(firstFailure).Throw();
            ct.ThrowIfCancellationRequested();
        }

        private static void WorkLoop(int workerId, NifWorkQueue queue, int overlapWorkers, Action<string> convert, FileClaims claims, CancellationToken ct)
        {
            if (workerId >= overlapWorkers) queue.WaitUntilEnded(ct);

            ThreadPriority? current = null;
            while (claims.TryClaim() && queue.TryTake(out string file, ct))
            {
                var wanted = queue.IsComplete ? SteadyPriority : OverlapPriority;
                if (current != wanted)
                {
                    Thread.CurrentThread.Priority = wanted;
                    current = wanted;
                }
                convert(file);
            }
        }

        /// <summary>Hands out at most <paramref name="limit"/> claims across all workers.</summary>
        private sealed class FileClaims(int limit)
        {
            private int _claimed;

            public bool TryClaim() => Interlocked.Increment(ref _claimed) <= limit;
        }
    }
}
