using System.Runtime.ExceptionServices;

namespace AssetConverter
{
    internal static class OrderedParallel
    {
        /// <summary>
        /// Maps <paramref name="items"/> on up to <paramref name="maxDegreeOfParallelism"/> threads and returns the results in input
        /// order. Cancellation surfaces as <see cref="OperationCanceledException"/>; a failure is always rethrown as itself, the one
        /// of the lowest item index whatever the timing, and the failures of other items are written to the error stream.
        /// </summary>
        public static TResult[] Select<TItem, TResult>(
            IReadOnlyList<TItem> items,
            int maxDegreeOfParallelism,
            Func<TItem, TResult> selector,
            CancellationToken ct)
        {
            var results = new TResult[items.Count];
            var failures = ParallelFailures.Collect(
                items.Count, maxDegreeOfParallelism, i => results[i] = selector(items[i]), stopAtFirstFailure: true, ct: ct);
            if (failures.Count > 0)
            {
                foreach (var other in failures.Skip(1))
                    Console.Error.WriteLine($"WARNING: item {other.Index} failed as well: {other.Error.GetType().Name}: {other.Error.Message}");
                ExceptionDispatchInfo.Capture(failures[0].Error).Throw();
            }
            return results;
        }
    }
}
