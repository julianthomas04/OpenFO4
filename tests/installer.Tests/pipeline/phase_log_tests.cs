using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests.Pipeline
{
    public sealed class PhaseLogTests
    {
        [Fact]
        public void MeasuringWorkRecordsItsNameAndAnIntervalInsideTheRun()
        {
            var log = new PhaseLog();

            var before = log.Elapsed;

            log.Measure("convert", () => { });

            var entry = Assert.Single(log.Entries);
            Assert.Equal("convert", entry.Name);
            Assert.True(entry.Start >= before);
            Assert.True(entry.End >= entry.Start);
            Assert.True(entry.End <= log.Elapsed);
        }

        [Fact]
        public void MeasuringWorkReturnsItsResult()
        {
            var log = new PhaseLog();

            int result = log.Measure("count", () => 42);

            Assert.Equal(42, result);
            Assert.Equal("count", Assert.Single(log.Entries).Name);
        }

        [Fact]
        public void WorkThatThrowsLeavesNoEntryAndTheExceptionPropagates()
        {
            var log = new PhaseLog();
            var failure = new InvalidOperationException("boom");

            var thrown = Assert.Throws<InvalidOperationException>(() => log.Measure("convert", () => throw failure));

            Assert.Same(failure, thrown);
            Assert.Empty(log.Entries);
        }

        [Fact]
        public void EntriesComeOrderedByStartWhateverTheOrderTheyFinished()
        {
            var log = new PhaseLog();

            log.Record("second", TimeSpan.FromSeconds(2));
            log.Record("first", TimeSpan.FromSeconds(1));

            Assert.Equal(["first", "second"], log.Entries.Select(entry => entry.Name));
        }

        [Fact]
        public async Task AStoppedLogKeepsItsElapsedTime()
        {
            var log = new PhaseLog();

            log.Stop();
            var stopped = log.Elapsed;
            await Task.Delay(1, TestContext.Current.CancellationToken);

            Assert.Equal(stopped, log.Elapsed);
        }
    }
}
