using System.Collections.Concurrent;
using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public class NifConversionWorkersTests
    {
        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private static string Nif(int i) => Path.Combine(Path.GetTempPath(), "nif-workers", $"mesh{i}.nif");

        private static NifWorkQueue Filled(int count)
        {
            var queue = new NifWorkQueue();
            for (int i = 0; i < count; i++) queue.Add(Nif(i));
            queue.Complete();
            return queue;
        }

        [Theory]
        [InlineData(32, 32, 8)]
        [InlineData(8, 8, 2)]
        [InlineData(4, 4, 2)]
        [InlineData(2, 2, 2)]
        [InlineData(1, 1, 1)]
        [InlineData(32, 3, 3)]
        public void TheOverlapWorkersAreAQuarterOfTheMachineAtLeastTwoAndNeverMoreThanAllWorkers(int processors, int workers, int expected)
        {
            Assert.Equal(expected, NifConversionWorkers.OverlapWorkerCount(processors, workers));
        }

        [Fact]
        public void EveryFileIsConvertedExactlyOnce()
        {
            var seen = new ConcurrentBag<string>();

            NifConversionWorkers.Run(Filled(500), overlapWorkers: 4, totalWorkers: 8, seen.Add, int.MaxValue, Ct);

            Assert.Equal(Enumerable.Range(0, 500).Select(Nif).Order().ToArray(), seen.Order().ToArray());
        }

        private sealed class ConcurrencyMeter
        {
            private readonly object _gate = new();
            private int _running;

            public int Peak { get; private set; }

            public IDisposable Enter()
            {
                lock (_gate)
                {
                    _running++;
                    Peak = Math.Max(Peak, _running);
                }
                return new Leave(this);
            }

            private sealed class Leave(ConcurrencyMeter meter) : IDisposable
            {
                public void Dispose()
                {
                    lock (meter._gate) meter._running--;
                }
            }
        }

        [Fact]
        public void AFinishedQueueRunsAllWorkersAtOnce()
        {
            var meter = new ConcurrencyMeter();
            using var allArrived = new CountdownEvent(6);

            NifConversionWorkers.Run(Filled(6), overlapWorkers: 2, totalWorkers: 6, _ =>
            {
                using var inside = meter.Enter();
                allArrived.Signal();
                allArrived.Wait(Ct);
            }, int.MaxValue, Ct);

            Assert.Equal(6, meter.Peak);
        }

        [Fact]
        public async Task WhileTheQueueHasProducersOnlyTheOverlapWorkersConvertAndTheRestJoinAfterwards()
        {
            var queue = new NifWorkQueue();
            var producing = new ConcurrencyMeter();
            var finished = new ConcurrencyMeter();
            using var overlapWorkerBusy = new ManualResetEventSlim();
            using var queueFinished = new ManualResetEventSlim();
            using var threeWorkersBusy = new ManualResetEventSlim();
            var timeout = TimeSpan.FromSeconds(30);
            void Convert(string _)
            {
                if (!queue.IsComplete)
                {
                    using var inside = producing.Enter();
                    overlapWorkerBusy.Set();
                    Assert.True(queueFinished.Wait(timeout, Ct), "the queue was never finished");
                    return;
                }
                using var afterwards = finished.Enter();
                if (finished.Peak > 2) threeWorkersBusy.Set();
                Assert.True(threeWorkersBusy.Wait(timeout, Ct), $"only {finished.Peak} workers ran after the queue finished");
            }
            for (int i = 0; i < 12; i++) queue.Add(Nif(i));
            var run = Task.Run(() => NifConversionWorkers.Run(queue, overlapWorkers: 2, totalWorkers: 6, Convert, int.MaxValue, Ct), Ct);

            Assert.True(overlapWorkerBusy.Wait(timeout, Ct), "no overlap worker started converting");
            for (int i = 12; i < 200; i++) queue.Add(Nif(i));
            queue.Complete();
            queueFinished.Set();
            await run.WaitAsync(timeout, Ct);

            Assert.InRange(producing.Peak, 1, 2);
            Assert.True(finished.Peak > 2, $"only {finished.Peak} workers ran after the queue finished");
        }

        [Fact]
        public async Task WorkersLowerTheirPriorityWhileTheQueueHasProducersAndRestoreItAfterwards()
        {
            var queue = new NifWorkQueue();
            queue.Add(Nif(0));
            queue.Add(Nif(1));
            var priorities = new ConcurrentDictionary<string, ThreadPriority>();
            using var firstConverted = new ManualResetEventSlim();
            using var queueFinished = new ManualResetEventSlim();
            var run = Task.Run(() => NifConversionWorkers.Run(queue, overlapWorkers: 1, totalWorkers: 1, file =>
            {
                priorities[file] = Thread.CurrentThread.Priority;
                if (file != Nif(0)) return;
                firstConverted.Set();
                queueFinished.Wait(Ct);
            }, int.MaxValue, Ct), Ct);
            firstConverted.Wait(Ct);

            queue.Complete();
            queueFinished.Set();
            await run.WithTimeout();

            Assert.Equal(NifConversionWorkers.OverlapPriority, priorities[Nif(0)]);
            Assert.Equal(NifConversionWorkers.SteadyPriority, priorities[Nif(1)]);
        }

        [Fact]
        public void TheFileCapStopsTheWorkersAfterThatManyFiles()
        {
            int converted = 0;

            NifConversionWorkers.Run(Filled(100), overlapWorkers: 4, totalWorkers: 4, _ => Interlocked.Increment(ref converted), maxFiles: 10, Ct);

            Assert.Equal(10, converted);
        }

        [Fact]
        public void AnExceptionFromAConversionStopsTheOtherWorkersAndIsRethrown()
        {
            var failure = new InvalidOperationException("bad file");
            int converted = 0;

            var thrown = Assert.Throws<InvalidOperationException>(() => NifConversionWorkers.Run(Filled(10_000), 4, 4, file =>
            {
                if (file == Nif(3)) throw failure;
                Interlocked.Increment(ref converted);
            }, int.MaxValue, Ct));

            Assert.Same(failure, thrown);
            Assert.True(converted < 10_000);
        }

        [Fact]
        public async Task AProducerFailureEndsTheWorkersWithTheOriginalExceptionAndNoHang()
        {
            var queue = new NifWorkQueue();
            queue.Add(Nif(0));
            var failure = new IOException("archive unreadable");
            var firstConverted = new ManualResetEventSlim();
            var run = Task.Run(() => NifConversionWorkers.Run(queue, overlapWorkers: 2, totalWorkers: 4, _ => firstConverted.Set(), int.MaxValue, Ct), Ct);
            firstConverted.Wait(Ct);

            queue.Fail(failure);

            Assert.Same(failure, await Assert.ThrowsAsync<IOException>(() => run.WithTimeout()));
        }

        [Fact]
        public async Task CancellationEndsWorkersBlockedOnAnEmptyQueueAndThrowsOperationCanceled()
        {
            var queue = new NifWorkQueue();
            using var cts = new CancellationTokenSource();
            var run = Task.Run(() => NifConversionWorkers.Run(queue, overlapWorkers: 1, totalWorkers: 3, _ => { }, int.MaxValue, cts.Token), Ct);
            await Task.Delay(50, Ct);

            await cts.CancelAsync();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => run.WithTimeout());
        }
    }
}
