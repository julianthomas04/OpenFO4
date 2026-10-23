using System.Collections.Concurrent;
using AssetConverter.Abc;
using AssetConverter.Swf;
using AssetConverter.UiLayout;

namespace AssetConverter
{
    /// <summary>The shared failure handling of the interface conversion steps.</summary>
    internal static class ConversionGuard
    {
        /// <summary>
        /// Runs one conversion of a game file and names the file in every failure: malformed data, a failed ffdec run, and the runtime
        /// faults an unexpected value can raise in the readers (kept as the inner exception).
        /// </summary>
        /// <param name="failure">Makes the step's own exception from a message and its cause.</param>
        public static T Run<T>(string subject, Func<T> convert, Func<string, Exception, Exception> failure)
        {
            try
            {
                return convert();
            }
            catch (Exception e) when (IsMalformedData(e))
            {
                throw failure($"'{subject}' cannot be converted: {e.Message}", e);
            }
            catch (Exception e) when (UnexpectedFault.Is(e))
            {
                throw failure($"'{subject}' failed to convert ({e.GetType().Name}): {e.Message}", e);
            }
        }

        static bool IsMalformedData(Exception e) =>
            e is SwfFormatException or AbcFormatException or FrameScriptException or TranslateTableException or FontConfigException or FfdecException;

        /// <summary>
        /// Runs the jobs in parallel and lets every one finish. When any failed, throws the exception <paramref name="combine"/> makes from
        /// all failures, ordered by message so the report does not depend on which job ran first.
        /// </summary>
        public static void RunAll(IEnumerable<Action> jobs, int maxDegreeOfParallelism, Func<IReadOnlyList<Exception>, Exception> combine)
        {
            var failures = new ConcurrentBag<Exception>();
            Parallel.ForEach(jobs, new ParallelOptions { MaxDegreeOfParallelism = maxDegreeOfParallelism }, job =>
            {
                try
                {
                    job();
                }
                catch (Exception e)
                {
                    failures.Add(e);
                }
            });
            if (!failures.IsEmpty)
                throw combine([.. failures.OrderBy(f => f.Message, StringComparer.Ordinal)]);
        }

        /// <summary>One message naming every failure, for a step that ran several jobs.</summary>
        public static string Describe(IReadOnlyList<Exception> failures) => string.Join(" ", failures.Select(f => f.Message));
    }
}
