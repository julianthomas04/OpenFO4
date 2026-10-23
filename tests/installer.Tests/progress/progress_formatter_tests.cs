using System.Globalization;
using AssetConverter.Progress;
using Xunit;

namespace InstallerTests.Progress
{
    public class ProgressFormatterTests
    {
        private static readonly ProgressFormatter German = new(new CultureInfo("de-DE"));

        private static StepSnapshot Step(StepStatus status = StepStatus.Running, long completed = 0, long total = 0, TimeSpan? startedAt = null, TimeSpan? elapsed = null) =>
            new("Converting meshes", "meshes", status, completed, total, null, status == StepStatus.Pending ? null : startedAt ?? TimeSpan.Zero, null, elapsed ?? TimeSpan.FromSeconds(23), 0);

        [Fact]
        public void NumbersFollowTheCulture()
        {
            Assert.Equal("12.345", German.Number(12_345));
            Assert.Equal("12,345", ProgressFormatter.Invariant.Number(12_345));
        }

        [Fact]
        public void ShortDurationsUseTheCulturesDecimalSeparatorAndLongOnesAreUnchanged()
        {
            Assert.Equal("23,5s", German.Duration(TimeSpan.FromMilliseconds(23_500)));
            Assert.Equal("23.5s", ProgressFormatter.Invariant.Duration(TimeSpan.FromMilliseconds(23_500)));
            Assert.Equal("1m 05s", German.Duration(TimeSpan.FromSeconds(65)));
            Assert.Equal("1h 02m 03s", German.Duration(TimeSpan.FromSeconds(3723)));
        }

        [Fact]
        public void StepTextsUseTheFormattersCulture()
        {
            var step = Step(completed: 1_482, total: 63_000);

            Assert.Equal("1.482 of 63.000 meshes", German.Detailed(step));
            Assert.Equal("1.482 / 63.000", German.Compact(step));
            Assert.Equal("23,0s", German.Elapsed(step));
        }

        [Fact]
        public void TheStaticTextsStayInvariant()
        {
            var step = Step(completed: 1_482, total: 63_000);

            Assert.Equal("1,482 / 63,000", ProgressFormatter.Invariant.Compact(step));
            Assert.Equal("23.0s", ProgressFormatter.Invariant.Elapsed(step));
        }

        [Fact]
        public void AStepThatEndedWithoutAnyCountedWorkReadsAsSkippedButStillShowsHowLongItRan()
        {
            var noWork = Step(StepStatus.Done);
            var worked = Step(StepStatus.Done, completed: 3, total: 3);

            Assert.True(noWork.WasSkipped);
            Assert.Equal(ProgressText.Skipped, German.Compact(noWork));
            Assert.Equal("23,0s", German.Elapsed(noWork));
            Assert.False(worked.WasSkipped);
            Assert.Equal("3 / 3", German.Compact(worked));
            Assert.Equal("23,0s", German.Elapsed(worked));
        }

        [Fact]
        public void AStepThatNeverStartedShowsNoTimeAndOneThatCountedWorkWithoutATotalIsNotSkipped()
        {
            var neverStarted = new StepSnapshot("Shaders", "files", StepStatus.Done, 0, 0, null, null, TimeSpan.FromSeconds(9), TimeSpan.Zero, 1.0);
            var uncounted = Step(StepStatus.Done, completed: 7, total: 0);

            Assert.True(neverStarted.WasSkipped);
            Assert.Equal("", German.Elapsed(neverStarted));
            Assert.Equal(ProgressText.Skipped, German.Compact(neverStarted));
            Assert.False(uncounted.WasSkipped);
            Assert.Equal("7", German.Compact(uncounted));
        }

        [Fact]
        public void CompactIsEmptyForPendingStepsAndSlashSeparatedOtherwise()
        {
            Assert.Equal("", ProgressFormatter.Invariant.Compact(Step(StepStatus.Pending, completed: 3, total: 10)));
            Assert.Equal("1,200 / 63,000", ProgressFormatter.Invariant.Compact(Step(completed: 1_200, total: 63_000)));
            Assert.Equal("40", ProgressFormatter.Invariant.Compact(Step(completed: 40)));
            Assert.Equal("", ProgressFormatter.Invariant.Compact(Step()));
        }

        [Fact]
        public void ElapsedIsEmptyForStepsThatHaveNotStarted()
        {
            Assert.Equal("", ProgressFormatter.Invariant.Elapsed(Step(StepStatus.Pending)));
            Assert.Equal("2m 03s", ProgressFormatter.Invariant.Elapsed(Step(elapsed: TimeSpan.FromSeconds(123))));
        }

        [Fact]
        public void DurationsUseHoursMinutesAndSecondsOnlyAsNeeded()
        {
            Assert.Equal("0.5s", ProgressFormatter.Invariant.Duration(TimeSpan.FromMilliseconds(540)));
            Assert.Equal("59.9s", ProgressFormatter.Invariant.Duration(TimeSpan.FromMilliseconds(59_950)));
            Assert.Equal("1m 00s", ProgressFormatter.Invariant.Duration(TimeSpan.FromSeconds(60)));
            Assert.Equal("1h 02m 03s", ProgressFormatter.Invariant.Duration(TimeSpan.FromSeconds(3723)));
        }

        [Theory]
        [InlineData(0, "0%")]
        [InlineData(0.376, "37%")]
        [InlineData(0.999, "99%")]
        [InlineData(1, "100%")]
        [InlineData(1.5, "100%")]
        [InlineData(-1, "0%")]
        public void PercentRoundsDownAndClamps(double fraction, string expected)
        {
            Assert.Equal(expected, ProgressFormatter.Invariant.Percent(fraction));
        }

        [Fact]
        public void PercentUsesTheCulturesPercentSign()
        {
            var formatter = new ProgressFormatter(new NumberFormatInfo { PercentSymbol = " pct" });

            Assert.Equal("50 pct", formatter.Percent(0.5));
        }

        [Fact]
        public void ARunningStepWithoutCountsYetIsNotSkipped()
        {
            Assert.False(Step(StepStatus.Running).WasSkipped);
            Assert.False(Step(StepStatus.Failed).WasSkipped);
        }
    }
}
