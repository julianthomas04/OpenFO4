using AssetConverter.Logging;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests.Logging
{
    public class LogBufferTests
    {
        [Fact]
        public void LinesAreNumberedFromOneInOrder()
        {
            var buffer = new LogBuffer(10);
            buffer.WriteLine("a", false);
            buffer.WriteLine("b", true);

            var read = buffer.ReadAfter(0);

            Assert.Equal([1L, 2L], read.Lines.Select(l => l.Sequence));
            Assert.Equal(["a", "b"], read.Lines.Select(l => l.Text));
            Assert.Equal([false, true], read.Lines.Select(l => l.IsError));
            Assert.Equal(2, read.LastSequence);
            Assert.Equal(0, read.Missed);
        }

        [Fact]
        public void ReadingAfterAPositionReturnsOnlyNewerLines()
        {
            var buffer = new LogBuffer(10);
            buffer.WriteLine("a", false);
            var first = buffer.ReadAfter(0);
            buffer.WriteLine("b", false);
            buffer.WriteLine("c", false);

            var second = buffer.ReadAfter(first.LastSequence);

            Assert.Equal(["b", "c"], second.Lines.Select(l => l.Text));
            Assert.Empty(buffer.ReadAfter(second.LastSequence).Lines);
        }

        [Fact]
        public void AnEmptyBufferReadsNothing()
        {
            var read = new LogBuffer(3).ReadAfter(0);

            Assert.Empty(read.Lines);
            Assert.Equal(0, read.LastSequence);
        }

        [Fact]
        public void OnceFullTheOldestLinesAreOverwritten()
        {
            var buffer = new LogBuffer(3);
            for (int i = 1; i <= 5; i++) buffer.WriteLine($"line {i}", false);

            var read = buffer.ReadAfter(0);

            Assert.Equal(["line 3", "line 4", "line 5"], read.Lines.Select(l => l.Text));
            Assert.Equal([3L, 4L, 5L], read.Lines.Select(l => l.Sequence));
            Assert.Equal(2, read.Missed);
        }

        [Fact]
        public void LinesOverwrittenBeforeTheyWereReadAreCountedAsMissed()
        {
            var buffer = new LogBuffer(3);
            buffer.WriteLine("1", false);
            buffer.WriteLine("2", false);
            var seen = buffer.ReadAfter(0);
            for (int i = 3; i <= 8; i++) buffer.WriteLine($"{i}", false);

            var read = buffer.ReadAfter(seen.LastSequence);

            Assert.Equal(["6", "7", "8"], read.Lines.Select(l => l.Text));
            Assert.Equal(3, read.Missed);
        }

        [Fact]
        public void LinesAreTimestampedByTheClock()
        {
            var clock = new ManualTimeProvider();
            var buffer = new LogBuffer(3, clock);
            buffer.WriteLine("early", false);
            clock.Advance(TimeSpan.FromMinutes(2));
            buffer.WriteLine("late", false);

            var lines = buffer.ReadAfter(0).Lines;

            Assert.Equal(TimeSpan.FromMinutes(2), lines[1].Timestamp - lines[0].Timestamp);
        }

        [Fact]
        public void ConcurrentWritersLoseNoLinesWithinTheCapacity()
        {
            var buffer = new LogBuffer(100_000);

            Parallel.For(0, 8, worker =>
            {
                for (int i = 0; i < 5000; i++) buffer.WriteLine($"{worker}:{i}", false);
            });

            var read = buffer.ReadAfter(0);
            Assert.Equal(40_000, read.Lines.Count);
            Assert.Equal(40_000, read.Lines.Select(l => l.Text).Distinct().Count());
            Assert.Equal(Enumerable.Range(1, 40_000).Select(i => (long)i), read.Lines.Select(l => l.Sequence));
        }

        [Fact]
        public void ACapacityOfZeroIsRejected()
        {
            Assert.Throws<ArgumentOutOfRangeException>(() => new LogBuffer(0));
        }

        [Fact]
        public void FormattedLinesCarryTheTimeAndAnErrorMarker()
        {
            var line = new LogLine(1, new DateTimeOffset(2026, 3, 4, 5, 6, 7, 89, TimeSpan.Zero), IsError: true, "boom");

            string formatted = line.Format();

            Assert.EndsWith(" ERR boom", formatted, StringComparison.Ordinal);
            Assert.Matches(@"^\d\d:\d\d:\d\d\.\d{3} ERR boom$", formatted);
        }
    }
}
