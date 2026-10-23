using AssetConverter;
using Xunit;

namespace InstallerTests
{
    // Tells a stalled process from a slow one without waiting for real time: the clock and the process are scripted.
    public sealed class StallWatchdogTests
    {
        static readonly TimeSpan Limit = TimeSpan.FromSeconds(100);
        static readonly TimeSpan Poll = TimeSpan.FromSeconds(10);

        static readonly TimeSpan StartupLimit = TimeSpan.FromSeconds(250);

        TimeSpan _now;

        StallWatchdog Watchdog() => new(Limit, StartupLimit, () => _now);

        /// <summary>A process that exits after a number of polls, reporting the scripted progress at each poll.</summary>
        sealed class ScriptedProcess(StallWatchdogTests test, Func<int, long> progressAtPoll, int exitsAfterPolls) : IWatchedProcess
        {
            int _polls;

            public bool WaitForExit(TimeSpan timeout)
            {
                test._now += timeout;
                _polls++;
                return _polls > exitsAfterPolls;
            }

            public long Progress() => progressAtPoll(_polls);
        }

        [Fact]
        public void Process_RunningFarBeyondTheLimitButStillProgressing_IsNeverStalled()
        {
            var process = new ScriptedProcess(this, poll => poll * 5, exitsAfterPolls: 600);

            Assert.True(Watchdog().WaitForExit(process, Poll));
            Assert.True(_now > Limit * 50);
        }

        [Fact]
        public void Process_WithoutAnyProgress_IsStalledAfterTheStartupLimit()
        {
            var process = new ScriptedProcess(this, _ => 0, exitsAfterPolls: int.MaxValue - 1);

            Assert.False(Watchdog().WaitForExit(process, Poll));
            Assert.Equal(StartupLimit, _now);
        }

        [Fact]
        public void Process_ProgressingThenGoingSilent_IsStalledOneLimitAfterItsLastProgress()
        {
            var process = new ScriptedProcess(this, poll => Math.Min(poll, 30), exitsAfterPolls: int.MaxValue - 1);

            Assert.False(Watchdog().WaitForExit(process, Poll));
            Assert.Equal(TimeSpan.FromSeconds(30 * 10) + Limit, _now);
        }

        [Fact]
        public void Process_WhoseProgressShrinksThenSteadies_IsStalledOneLimitAfterTheShrink()
        {
            var process = new ScriptedProcess(this, poll => poll <= 10 ? poll * 10 : 40, exitsAfterPolls: int.MaxValue - 1);

            Assert.False(Watchdog().WaitForExit(process, Poll));
            Assert.Equal(TimeSpan.FromSeconds(11 * 10) + Limit, _now);
        }

        [Fact]
        public void Process_SilentAtFirstButProgressingWithinTheStartupLimit_IsThenHeldToTheStallLimit()
        {
            var process = new ScriptedProcess(this, poll => poll < 18 ? 0 : 1, exitsAfterPolls: int.MaxValue - 1);
            var watchdog = Watchdog();

            Assert.False(watchdog.WaitForExit(process, Poll));
            Assert.True(watchdog.HasStarted);
            Assert.Equal(Limit, watchdog.ActiveLimit);
            Assert.Equal(TimeSpan.FromSeconds(18 * 10) + Limit, _now);
        }

        [Fact]
        public void Process_ExitingAfterALongSilentStart_IsNotStalled()
        {
            var process = new ScriptedProcess(this, _ => 0, exitsAfterPolls: 24);

            Assert.True(Watchdog().WaitForExit(process, Poll));
        }

        [Fact]
        public void ActiveLimit_IsTheStartupLimitUntilTheFirstProgress()
        {
            var watchdog = Watchdog();
            Assert.Equal(StartupLimit, watchdog.ActiveLimit);

            watchdog.IsStalled(0);
            Assert.Equal(StartupLimit, watchdog.ActiveLimit);

            watchdog.IsStalled(1);
            Assert.Equal(Limit, watchdog.ActiveLimit);
        }

        [Fact]
        public void Process_ExitingJustBeforeTheLimit_IsNotStalled()
        {
            var process = new ScriptedProcess(this, _ => 0, exitsAfterPolls: 9);

            Assert.True(Watchdog().WaitForExit(process, Poll));
        }

        [Fact]
        public void IsStalled_AtExactlyTheLimit_ReportsStalled()
        {
            var watchdog = Watchdog();
            Assert.False(watchdog.IsStalled(7));

            _now = Limit - TimeSpan.FromTicks(1);
            Assert.False(watchdog.IsStalled(7));

            _now = Limit;
            Assert.True(watchdog.IsStalled(7));
        }

        [Theory]
        [InlineData(0)]
        [InlineData(-5)]
        public void Constructor_RejectsALimitThatIsNotPositive(int seconds) =>
            Assert.Throws<ArgumentOutOfRangeException>(() => new StallWatchdog(TimeSpan.FromSeconds(seconds), StartupLimit, () => _now));

        [Fact]
        public void Constructor_RejectsAStartupLimitShorterThanTheStallLimit() =>
            Assert.Throws<ArgumentOutOfRangeException>(() => new StallWatchdog(Limit, Limit - Poll, () => _now));

        [Fact]
        public void StartupLimit_IsLongerThanTheStallLimit() =>
            Assert.True(Ffdec.StartupLimit > Ffdec.StallLimit);

        [Fact]
        public void StallLimit_IsAWholeNumberOfSecondsSoTheErrorMessageStatesItExactly() =>
            Assert.Equal(0, Ffdec.StallLimit.TotalSeconds % 1);
    }
}
