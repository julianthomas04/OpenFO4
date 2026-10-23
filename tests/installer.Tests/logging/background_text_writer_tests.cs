using AssetConverter.Logging;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests.Logging
{
    public sealed class BackgroundTextWriterTests
    {
        private static readonly TimeSpan Patience = TimeSpan.FromSeconds(30);

        private readonly List<string> _faults = new();

        private BackgroundTextWriter Create(TextWriter target, int capacity = 100) =>
            new(target, capacity, message =>
            {
                lock (_faults) _faults.Add(message);
            });

        private string[] Faults()
        {
            lock (_faults) return [.. _faults];
        }

        [Fact]
        public void TextReachesTheTargetInOrderOnceFlushed()
        {
            var target = new StringWriter();
            var writer = Create(target);

            writer.Write("a");
            writer.Write("b");
            writer.Write("c");
            Assert.True(writer.Flush(Patience));

            Assert.Equal("abc", target.ToString());
            Assert.Empty(Faults());
        }

        [Fact]
        public async Task AStalledTargetNeverBlocksTheWriter()
        {
            var target = new StallableTextWriter();
            var writer = Create(target, capacity: 5);
            target.Stall();
            writer.Write("stuck");
            target.WaitUntilAWriteHasStarted();

            var burst = Task.Run(() =>
            {
                for (int i = 0; i < 1000; i++) writer.Write("x");
            }, TestContext.Current.CancellationToken);

            await burst.WaitAsync(Patience, TestContext.Current.CancellationToken);
            target.Release();
            Assert.True(writer.Close(Patience));
        }

        [Fact]
        public void TextThatDoesNotFitWhileStalledIsDroppedOldestFirstAndReportedOnceTheTargetRecovers()
        {
            var target = new StallableTextWriter();
            var writer = Create(target, capacity: 4);
            target.Stall();
            writer.Write("first");
            target.WaitUntilAWriteHasStarted();
            foreach (string chunk in new[] { "1", "2", "3", "4", "5", "6" }) writer.Write(chunk);

            target.Release();
            Assert.True(writer.Flush(Patience));

            Assert.Equal("first3456", target.ToString());
            string fault = Assert.Single(Faults());
            Assert.Contains("2 writes were dropped", fault, StringComparison.Ordinal);
        }

        [Fact]
        public async Task AFlushRequestedWhileTheQueueIsFullNeitherDropsTextNorIsDroppedItself()
        {
            var target = new StallableTextWriter();
            var writer = Create(target, capacity: 3);
            target.Stall();
            writer.Write("first");
            target.WaitUntilAWriteHasStarted();
            foreach (string chunk in new[] { "1", "2", "3" }) writer.Write(chunk);

            var flush = Task.Run(() => writer.Flush(Patience), TestContext.Current.CancellationToken);
            foreach (string chunk in new[] { "4", "5" }) writer.Write(chunk);
            target.Release();

            Assert.True(await flush.WaitAsync(Patience, TestContext.Current.CancellationToken));
            Assert.Equal("first345", target.ToString());
            Assert.True(target.Flushes >= 1);
        }

        [Fact]
        public void AFlushOnAStalledTargetGivesUpAfterTheTimeoutAndTheTextStillArrivesOnceItRecovers()
        {
            var target = new StallableTextWriter();
            var writer = Create(target);
            target.Stall();
            writer.Write("stuck");
            target.WaitUntilAWriteHasStarted();
            writer.Write("queued");

            bool flushed = writer.Flush(TimeSpan.FromMilliseconds(100));
            target.Release();

            Assert.False(flushed);
            Assert.True(writer.Flush(Patience));
            Assert.Equal("stuckqueued", target.ToString());
        }

        [Fact]
        public void AFlushWaitsForEverythingQueuedAndFlushesTheTarget()
        {
            var target = new StallableTextWriter();
            var writer = Create(target);

            writer.Write("text");
            Assert.True(writer.Flush(Patience));

            Assert.Equal("text", target.ToString());
            Assert.True(target.Flushes >= 1);
        }

        [Fact]
        public void ClosingDrainsTheQueueAndReportsWhetherItFinishedInTime()
        {
            var target = new StringWriter();
            var writer = Create(target);
            writer.Write("last words");

            bool drained = writer.Close(Patience);

            Assert.True(drained);
            Assert.Equal("last words", target.ToString());
        }

        [Fact]
        public void ClosingAStalledWriterGivesUpAfterTheTimeout()
        {
            var target = new StallableTextWriter();
            var writer = Create(target);
            target.Stall();
            writer.Write("stuck");
            target.WaitUntilAWriteHasStarted();

            bool drained = writer.Close(TimeSpan.FromMilliseconds(100));
            target.Release();

            Assert.False(drained);
        }

        [Fact]
        public void TextWrittenAfterClosingIsNotForwarded()
        {
            var target = new StringWriter();
            var writer = Create(target);
            writer.Close(Patience);

            writer.Write("late");
            Assert.True(writer.Flush(Patience));

            Assert.Equal("", target.ToString());
        }

        [Fact]
        public void AFailingTargetIsReportedOnceAndNotWrittenToAgain()
        {
            var writer = Create(new ThrowingWriter());

            writer.Write("one");
            writer.Write("two");
            Assert.True(writer.Flush(Patience));

            string fault = Assert.Single(Faults());
            Assert.Contains("console output is disabled", fault, StringComparison.Ordinal);
            Assert.Contains("the console is gone", fault, StringComparison.Ordinal);
        }

        [Fact]
        public void AnyKindOfTargetFailureKeepsTheLoopAliveSoFlushAndCloseReturn()
        {
            var writer = Create(new ThrowingWriter(new InvalidOperationException("not an IO problem")));

            writer.Write("one");
            Assert.True(writer.Flush(Patience));

            Assert.True(writer.Close(Patience));
            Assert.Single(Faults());
        }

        [Fact]
        public void ConcurrentWritersLoseNothingAndKeepEachWriteIntact()
        {
            var target = new StringWriter();
            var writer = Create(target, capacity: 100_000);

            Parallel.For(0, 8, worker =>
            {
                for (int i = 0; i < 500; i++) writer.Write($"[{worker}:{i}]");
            });
            Assert.True(writer.Flush(Patience));

            string text = target.ToString();
            Assert.Equal(4000, text.Count(c => c == '['));
            Assert.Equal(4000, text.Count(c => c == ']'));
        }

        private sealed class ThrowingWriter(Exception? failure = null) : StringWriter
        {
            private readonly Exception _failure = failure ?? new IOException("the console is gone");

            public override void Write(string? value) => throw _failure;
        }
    }
}
