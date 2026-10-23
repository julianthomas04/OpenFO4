using System.Diagnostics;
using System.Text.RegularExpressions;
using AssetConverter;
using Xunit;

namespace InstallerTests
{
    // EsmTimingCollector: the opt-in timing table of the ESM import. Enabled is injected, so no test depends on OPENFO4_ESM_TIMING.
    public class EsmTimingCollectorTests
    {
        static string Report(EsmTimingCollector collector)
        {
            var writer = new StringWriter();
            collector.Report(writer);
            return writer.ToString();
        }

        static long CountOf(string report, string section)
        {
            var match = Regex.Match(report, $@"^\s+{Regex.Escape(section)}\s+\d+\.\d\d\s+(\d+)\s", RegexOptions.Multiline);
            Assert.True(match.Success, $"section {section} missing from:{Environment.NewLine}{report}");
            return long.Parse(match.Groups[1].Value, System.Globalization.CultureInfo.InvariantCulture);
        }

        [Fact]
        public void Disabled_RecordsNothingAndPrintsNothing()
        {
            var collector = new EsmTimingCollector(false);

            collector.Add("a", Stopwatch.GetTimestamp());
            using (collector.Measure("b")) { }
            using (collector.MeasurePhase("c")) { }

            Assert.Equal(0, collector.Now());
            Assert.Equal("", Report(collector));
        }

        [Fact]
        public void Add_AccumulatesCountsPerSection()
        {
            var collector = new EsmTimingCollector(true);

            collector.Add("rows", collector.Now(), 3);
            collector.Add("rows", collector.Now(), 4);
            collector.Add("other", collector.Now());

            var report = Report(collector);
            Assert.Equal(7, CountOf(report, "rows"));
            Assert.Equal(1, CountOf(report, "other"));
        }

        [Fact]
        public void Report_ListsSectionsInFirstSeenOrder()
        {
            var collector = new EsmTimingCollector(true);
            collector.Add("second.seen.last", collector.Now());
            collector.Add("alpha", collector.Now());

            var report = Report(collector);

            Assert.True(report.IndexOf("second.seen.last", StringComparison.Ordinal) < report.IndexOf("alpha", StringComparison.Ordinal));
        }

        [Fact]
        public void MeasureAndMeasurePhase_RecordOneEntryWhenDisposed()
        {
            var collector = new EsmTimingCollector(true);

            using (collector.Measure("section")) { }
            using (collector.MeasurePhase("phase")) { }

            var report = Report(collector);
            Assert.Equal(1, CountOf(report, "section"));
            Assert.Equal(1, CountOf(report, "phase"));
        }

        [Fact]
        public void Report_Wave1Jobs_AreSummarisedAndOnlyTheSlowestAreListed()
        {
            var collector = new EsmTimingCollector(true);
            for (int i = 0; i < 15; i++) collector.Add($"wave1.job.T{i:D2}", collector.Now());

            var report = Report(collector);

            Assert.Contains("wave-1 jobs: 15 jobs", report);
            Assert.Equal(12, Regex.Matches(report, @"^\s+wave1\.job\.T\d\d\s", RegexOptions.Multiline).Count);
        }

        [Fact]
        public async Task RunReportedAsync_Success_PrintsTheReportWithATotalAndTheBodysSections()
        {
            var collector = new EsmTimingCollector(true);
            var writer = new StringWriter();

            await collector.RunReportedAsync(() =>
            {
                collector.Add("work", collector.Now());
                return Task.CompletedTask;
            }, writer);

            Assert.Equal(1, CountOf(writer.ToString(), "work"));
            Assert.Equal(1, CountOf(writer.ToString(), "total"));
        }

        [Fact]
        public async Task RunReportedAsync_BodyThrows_StillPrintsThePartialReportAndRethrowsTheOriginalError()
        {
            var collector = new EsmTimingCollector(true);
            var writer = new StringWriter();
            var failure = new InvalidOperationException("load failed");

            var thrown = await Assert.ThrowsAsync<InvalidOperationException>(() => collector.RunReportedAsync(() =>
            {
                collector.Add("before.failure", collector.Now());
                throw failure;
            }, writer));

            Assert.Same(failure, thrown);
            Assert.Equal(1, CountOf(writer.ToString(), "before.failure"));
            Assert.Equal(1, CountOf(writer.ToString(), "total"));
        }

        [Fact]
        public async Task RunReportedAsync_ForgetsEntriesOfEarlierRuns()
        {
            var collector = new EsmTimingCollector(true);
            collector.Add("earlier.run", collector.Now());
            var writer = new StringWriter();

            await collector.RunReportedAsync(() => Task.CompletedTask, writer);

            Assert.DoesNotContain("earlier.run", writer.ToString());
        }

        [Fact]
        public async Task RunReportedAsync_Disabled_RunsTheBodyAndPrintsNothing()
        {
            var collector = new EsmTimingCollector(false);
            var writer = new StringWriter();
            bool ran = false;

            await collector.RunReportedAsync(() =>
            {
                ran = true;
                return Task.CompletedTask;
            }, writer);

            Assert.True(ran);
            Assert.Equal("", writer.ToString());
        }
    }
}
