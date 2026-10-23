using AssetConverter;
using Xunit;

namespace InstallerTests
{
    // OrderedParallel.Select: concurrent map with input-ordered results, bounded parallelism, cancellation and error propagation.
    [Collection("Console redirection")]
    public class OrderedParallelTests
    {
        static readonly TimeSpan Rendezvous = TimeSpan.FromSeconds(30);

        [Fact]
        public void Select_EmptyInput_ReturnsEmptyResults()
        {
            var results = OrderedParallel.Select(Array.Empty<int>(), 4, i => i, CancellationToken.None);

            Assert.Empty(results);
        }

        [Fact]
        public void Select_ResultsFollowInputOrderEvenWhenLaterItemsFinishFirst()
        {
            // Item i finishes only after item i + 1 has finished, so the items complete in reverse input order.
            int[] items = [0, 1, 2];
            var finished = items.Select(_ => new ManualResetEventSlim()).ToArray();

            var results = OrderedParallel.Select(items, items.Length, i =>
            {
                if (i + 1 < items.Length)
                    Assert.True(finished[i + 1].Wait(Rendezvous), $"item {i + 1} never finished");
                finished[i].Set();
                return $"result {i}";
            }, CancellationToken.None);

            Assert.Equal(["result 0", "result 1", "result 2"], results);
        }

        [Fact]
        public void Select_NeverExceedsTheDegreeOfParallelism()
        {
            const int degree = 2;
            int running = 0, peak = 0;

            OrderedParallel.Select(Enumerable.Range(0, 40).ToList(), degree, i =>
            {
                int now = Interlocked.Increment(ref running);
                int seen;
                while (now > (seen = Volatile.Read(ref peak)) && Interlocked.CompareExchange(ref peak, now, seen) != seen) { }
                Thread.Yield();
                Interlocked.Decrement(ref running);
                return i;
            }, CancellationToken.None);

            Assert.InRange(peak, 1, degree);
        }

        [Fact]
        public void Select_SingleFailure_IsRethrownAsItselfNotWrapped()
        {
            var ex = Assert.Throws<InvalidOperationException>(() =>
                OrderedParallel.Select<int, int>(new[] { 1, 2, 3 }, 3, i => i == 2 ? throw new InvalidOperationException("boom") : i, CancellationToken.None));

            Assert.Equal("boom", ex.Message);
        }

        [Fact]
        public void Select_SeveralConcurrentFailures_RethrowTheLowestItemsFailureAsItselfWhicheverFailsFirst()
        {
            // The later item fails first and the earlier one only after it has, so timing alone would pick the later one.
            using var laterFailed = new ManualResetEventSlim();

            var ex = Assert.Throws<InvalidOperationException>(() =>
                OrderedParallel.Select<int, int>([0, 1], 2, i =>
                {
                    if (i == 1)
                    {
                        laterFailed.Set();
                        throw new InvalidOperationException("later");
                    }
                    Assert.True(laterFailed.Wait(Rendezvous), "the later item never failed");
                    throw new InvalidOperationException("earlier");
                }, CancellationToken.None));

            Assert.Equal("earlier", ex.Message);
        }

        [Fact]
        public void Select_FailuresOfOtherItems_AreWrittenToTheErrorStream()
        {
            var original = Console.Error;
            var errors = new StringWriter();
            Console.SetError(errors);
            try
            {
                using var laterFailed = new ManualResetEventSlim();
                Assert.Throws<InvalidOperationException>(() =>
                    OrderedParallel.Select<int, int>([0, 1], 2, i =>
                    {
                        if (i == 1)
                        {
                            laterFailed.Set();
                            throw new ArgumentException("later boom");
                        }
                        Assert.True(laterFailed.Wait(Rendezvous), "the later item never failed");
                        throw new InvalidOperationException("earlier");
                    }, CancellationToken.None));
            }
            finally
            {
                Console.SetError(original);
            }

            Assert.Contains("item 1 failed as well: ArgumentException: later boom", errors.ToString());
        }

        [Fact]
        public void Select_AlreadyCancelled_ThrowsWithoutRunningAnyItem()
        {
            using var cts = new CancellationTokenSource();
            cts.Cancel();
            int ran = 0;

            Assert.ThrowsAny<OperationCanceledException>(() =>
                OrderedParallel.Select(new[] { 1, 2, 3 }, 2, i => Interlocked.Increment(ref ran), cts.Token));

            Assert.Equal(0, ran);
        }

        [Fact]
        public void Select_CancelledMidway_StopsSchedulingAndThrowsCancellation()
        {
            using var cts = new CancellationTokenSource();
            int ran = 0;

            Assert.ThrowsAny<OperationCanceledException>(() =>
                OrderedParallel.Select(Enumerable.Range(0, 1000).ToList(), 1, i =>
                {
                    Interlocked.Increment(ref ran);
                    cts.Cancel();
                    return i;
                }, cts.Token));

            Assert.True(ran < 1000, $"{ran} items ran after cancellation");
        }
    }
}
