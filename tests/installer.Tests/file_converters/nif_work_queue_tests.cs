using AssetConverter;
using Xunit;

namespace InstallerTests
{
    public class NifWorkQueueTests
    {
        private static readonly string Root = Path.Combine(Path.GetTempPath(), "nif-work-queue");

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private static string Nif(string name) => Path.Combine(Root, name);

        private static List<string> Drain(NifWorkQueue queue)
        {
            var taken = new List<string>();
            while (queue.TryTake(out string path, Ct)) taken.Add(path);
            return taken;
        }

        private static NifWorkQueue WithPriority(params string[] priorityPaths)
        {
            var priority = new PriorityFiles(() => { });
            priority.Prioritize(priorityPaths);
            return new NifWorkQueue(priorityFiles: priority);
        }

        [Fact]
        public void FilesComeOutInTheOrderTheyWereAdded()
        {
            var queue = new NifWorkQueue();
            foreach (string name in new[] { "c.nif", "a.nif", "b.nif" }) queue.Add(Nif(name));
            queue.Complete();

            Assert.Equal([Nif("c.nif"), Nif("a.nif"), Nif("b.nif")], Drain(queue));
        }

        [Fact]
        public void PriorityFilesAreServedBeforeTheOthersAmongThoseQueued()
        {
            var queue = WithPriority(Nif("skeleton.nif"), Nif("skeleton2.nif"));
            queue.Add(Nif("a.nif"));
            queue.Add(Nif("skeleton.nif"));
            queue.Add(Nif("b.nif"));
            queue.Add(Nif("SKELETON2.nif"));
            queue.Complete();

            Assert.Equal([Nif("skeleton.nif"), Nif("SKELETON2.nif"), Nif("a.nif"), Nif("b.nif")], Drain(queue));
        }

        [Fact]
        public void APriorityFileAddedLateJumpsAheadOfTheBacklog()
        {
            var queue = WithPriority(Nif("skeleton.nif"));
            queue.Add(Nif("a.nif"));
            queue.Add(Nif("b.nif"));
            Assert.True(queue.TryTake(out string first, Ct));

            queue.Add(Nif("skeleton.nif"));
            Assert.True(queue.TryTake(out string second, Ct));

            Assert.Equal(Nif("a.nif"), first);
            Assert.Equal(Nif("skeleton.nif"), second);
        }

        [Fact]
        public async Task TryTakeBlocksUntilAFileIsAdded()
        {
            var queue = new NifWorkQueue();
            var taken = Task.Run(() => queue.TryTake(out string path, Ct) ? path : null, Ct);

            await Task.Delay(50, Ct);
            Assert.False(taken.IsCompleted);
            queue.Add(Nif("a.nif"));

            Assert.Equal(Nif("a.nif"), await taken.WithTimeout());
        }

        [Fact]
        public async Task CompletingTheQueueEndsAWaitingConsumerOnlyAfterTheBacklogIsServed()
        {
            var queue = new NifWorkQueue();
            queue.Add(Nif("a.nif"));
            var drained = Task.Run(() => Drain(queue), Ct);

            queue.Complete();

            Assert.Equal([Nif("a.nif")], await drained.WithTimeout());
        }

        [Fact]
        public void TheQueueIsFinishedOnlyWhenEveryProducerCompleted()
        {
            var queue = new NifWorkQueue(producers: 2);

            queue.Complete();
            Assert.False(queue.IsComplete);
            queue.Complete();

            Assert.True(queue.IsComplete);
            Assert.Throws<InvalidOperationException>(queue.Complete);
        }

        [Fact]
        public void AddingAfterTheQueueIsFinishedIsRejected()
        {
            var queue = new NifWorkQueue();
            queue.Complete();

            Assert.Throws<InvalidOperationException>(() => queue.Add(Nif("a.nif")));
        }

        [Fact]
        public void AFileAddedTwiceIsRejectedIgnoringCase()
        {
            var queue = new NifWorkQueue();
            queue.Add(Nif("a.nif"));

            Assert.Throws<InvalidOperationException>(() => queue.Add(Nif("A.NIF")));
        }

        [Fact]
        public async Task FailingWakesEveryWaitingConsumerWithTheProducersException()
        {
            var queue = new NifWorkQueue();
            var failure = new IOException("archive unreadable");
            var consumers = Enumerable.Range(0, 3).Select(index => Task.Run(() => queue.TryTake(out _, Ct), Ct)).ToArray();

            queue.Fail(failure);

            foreach (var consumer in consumers)
                Assert.Same(failure, await Assert.ThrowsAsync<IOException>(() => consumer.WithTimeout()));
        }

        [Fact]
        public void AFailedQueueServesNoMoreFilesAndRejectsNewOnes()
        {
            var queue = new NifWorkQueue();
            queue.Add(Nif("a.nif"));
            var failure = new IOException("boom");
            queue.Fail(failure);

            Assert.Same(failure, Assert.Throws<IOException>(() => queue.TryTake(out _, Ct)));
            Assert.Same(failure, Assert.Throws<IOException>(() => queue.Add(Nif("b.nif"))));
            Assert.True(queue.IsComplete);
        }

        [Fact]
        public async Task CancellingWakesAWaitingConsumer()
        {
            var queue = new NifWorkQueue();
            using var cts = new CancellationTokenSource();
            var consumer = Task.Run(() => queue.TryTake(out _, cts.Token), Ct);

            await cts.CancelAsync();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => consumer.WithTimeout());
        }

        [Fact]
        public async Task WaitUntilEndedReturnsOnCompletionAndOnFailureAndHonoursCancellation()
        {
            var completed = new NifWorkQueue();
            var failed = new NifWorkQueue();
            using var cts = new CancellationTokenSource();
            var waitingOnCompleted = Task.Run(() => completed.WaitUntilEnded(Ct), Ct);
            var waitingOnFailed = Task.Run(() => failed.WaitUntilEnded(Ct), Ct);
            var cancelled = Task.Run(() => new NifWorkQueue().WaitUntilEnded(cts.Token), Ct);

            completed.Complete();
            failed.Fail(new IOException());
            await cts.CancelAsync();

            await waitingOnCompleted.WithTimeout();
            await waitingOnFailed.WithTimeout();
            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => cancelled.WithTimeout());
        }

        [Fact]
        public void AnnouncedFilesCountTowardsTheTotalAndAreLeftToTheirProducer()
        {
            var queue = new NifWorkQueue(producers: 2);
            queue.Announce([Nif("pending1.nif"), Nif("pending2.nif")]);

            int added = queue.AddExisting([Nif("old.nif"), Nif("PENDING1.nif")]);

            Assert.Equal(1, added);
            Assert.Equal(3, queue.ExpectedCount);
            Assert.Equivalent(new[] { Nif("pending1.nif"), Nif("pending2.nif"), Nif("old.nif") }, queue.Expected);
        }

        [Fact]
        public void AnAnnouncedFileThatArrivesIsServedOnceAndNotCountedTwice()
        {
            var queue = new NifWorkQueue();
            queue.Announce([Nif("a.nif")]);

            queue.Add(Nif("a.nif"));
            queue.Complete();

            Assert.Equal(1, queue.ExpectedCount);
            Assert.Equal([Nif("a.nif")], Drain(queue));
        }

        [Fact]
        public async Task ManyProducersAndConsumersServeEveryFileExactlyOnce()
        {
            var queue = new NifWorkQueue(producers: 4);
            var files = Enumerable.Range(0, 4000).Select(i => Nif($"mesh{i}.nif")).ToArray();
            var producers = Enumerable.Range(0, 4).Select(p => Task.Run(() =>
            {
                foreach (string file in files.Where((_, i) => i % 4 == p)) queue.Add(file);
                queue.Complete();
            }, Ct)).ToArray();
            var consumers = Enumerable.Range(0, 6).Select(_ => Task.Run(() => Drain(queue), Ct)).ToArray();

            var taken = (await Task.WhenAll(consumers).WithTimeout()).SelectMany(list => list).ToList();
            await Task.WhenAll(producers);

            Assert.Equal(files.Length, taken.Count);
            Assert.Equal(files.Order().ToArray(), taken.Order().ToArray());
        }
    }
}
