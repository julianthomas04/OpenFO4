using System.Collections.Concurrent;

namespace AssetConverter
{
    internal sealed record IndexedFailure(int Index, Exception Error);

    /// <summary>
    /// The shared shape of a parallel loop whose items can fail: every failure is collected with its item index and handed back in
    /// index order, so the caller decides how to report them.
    /// </summary>
    internal static class ParallelFailures
    {
        /// <summary>
        /// Runs <paramref name="body"/> for every index below <paramref name="count"/>. With <paramref name="stopAtFirstFailure"/> a
        /// failure stops later indices from starting while every earlier index still runs, so the first returned failure is the same
        /// whatever the timing; without it every item runs. Only exceptions accepted by <paramref name="isFailure"/> (default: all but
        /// <see cref="OutOfMemoryException"/>) are collected; others propagate. Cancellation surfaces as <see cref="OperationCanceledException"/>.
        /// </summary>
        public static IReadOnlyList<IndexedFailure> Collect(
            int count,
            int maxDegreeOfParallelism,
            Action<int> body,
            bool stopAtFirstFailure,
            Func<Exception, bool>? isFailure = null,
            CancellationToken ct = default)
        {
            isFailure ??= ex => ex is not OutOfMemoryException;
            var failures = new ConcurrentBag<IndexedFailure>();
            var options = new ParallelOptions { CancellationToken = ct, MaxDegreeOfParallelism = maxDegreeOfParallelism };
            Parallel.For(0, count, options, (index, loop) =>
            {
                try
                {
                    body(index);
                }
                catch (Exception ex) when (ex is not OperationCanceledException && isFailure(ex))
                {
                    failures.Add(new IndexedFailure(index, ex));
                    if (stopAtFirstFailure) loop.Break();
                }
            });
            return [.. failures.OrderBy(failure => failure.Index)];
        }
    }
}
