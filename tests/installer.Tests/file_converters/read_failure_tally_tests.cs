using AssetConverter;
using Xunit;

namespace InstallerTests.FileConverters
{
    public class ReadFailureTallyTests
    {
        private static object Missing => new { Present = 1 };

        [Fact]
        public void Read_DynamicMemberMissing_ReturnsFallbackAndCountsIt()
        {
            var tally = new ReadFailureTally();

            string? value = tally.Read("CELL Worldspace", "Cell1", () => (string?)((dynamic)Missing).Absent, "fallback");

            Assert.Equal("fallback", value);
            Assert.Equal(1, tally.CountFor("CELL Worldspace"));
        }

        [Fact]
        public void Read_Succeeds_ReturnsValueAndCountsNothing()
        {
            var tally = new ReadFailureTally();

            Assert.Equal(7, tally.Read("site", "rec", () => 7, -1));
            Assert.Equal(0, tally.Total);
        }

        [Fact]
        public void Read_UnrelatedExceptionType_Propagates()
        {
            var tally = new ReadFailureTally();

            Assert.Throws<TimeoutException>(() => tally.Read<int>("site", "rec", () => throw new TimeoutException(), 0));
            Assert.Equal(0, tally.Total);
        }

        [Fact]
        public void FormatSummary_ManyFailuresAtOneSite_YieldsOneLineWithCountAndFirstSubject()
        {
            var tally = new ReadFailureTally();
            for (int i = 0; i < 1000; i++)
                tally.Record("NPC_ Level", new InvalidOperationException("boom"), $"Npc{i}");

            var line = Assert.Single(tally.FormatSummary());

            Assert.Contains("1000", line);
            Assert.Contains("NPC_ Level", line);
            Assert.Contains("Npc0", line);
            Assert.DoesNotContain("Npc1 ", line);
        }

        [Fact]
        public void FormatSummary_DifferentSites_AreCountedSeparately()
        {
            var tally = new ReadFailureTally();
            tally.Record("A", new InvalidCastException("x"));
            tally.Record("A", new InvalidCastException("x"));
            tally.Record("B", new InvalidCastException("x"));

            Assert.Equal(2, tally.CountFor("A"));
            Assert.Equal(1, tally.CountFor("B"));
            Assert.Equal(2, tally.FormatSummary().Count);
        }

        [Fact]
        public void WriteSummaryAndReset_PrintsThenClears()
        {
            var tally = new ReadFailureTally();
            tally.Record("A", new InvalidCastException("x"), "r");
            var writer = new StringWriter();

            tally.WriteSummaryAndReset(writer);

            Assert.Contains("unreadable reads at A", writer.ToString());
            Assert.Equal(0, tally.Total);
        }

        [Fact]
        public void Record_ConcurrentCalls_DoNotLoseCounts()
        {
            var tally = new ReadFailureTally();

            Parallel.For(0, 8, _ =>
            {
                for (int i = 0; i < 500; i++)
                    tally.Record("site", new FormatException("x"), "r");
            });

            Assert.Equal(4000, tally.CountFor("site"));
        }
    }
}
