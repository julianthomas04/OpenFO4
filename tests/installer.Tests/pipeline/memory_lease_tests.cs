using AssetConverter.Pipeline;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests.Pipeline
{
    public sealed class MemoryLeaseTests
    {
        private const long Capacity = 100;
        private const long Baseline = 20;
        private static readonly TimeSpan Guard = TimeSpan.FromSeconds(30);

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        [Fact]
        public async Task LeaseCountsOnlyWhatItsChildProcessesHaveNotCommittedYet()
        {
            var gauge = new ChildGauge();
            using var lease = await gauge.Budget.ReserveAsync(50, Ct);
            gauge.Attach(lease, committed: 40);

            using var exactFit = await gauge.Budget.ReserveAsync(30, Ct);
            _ = gauge.Budget.ReserveAsync(1, Ct);

            Assert.Equal(1, gauge.Budget.WaitingCount);
        }

        [Fact]
        public async Task ChildProcessesCommittingMoreThanTheLeaseTakeOnlyTheLeaseOffTheMeasurement()
        {
            var gauge = new ChildGauge();
            using var lease = await gauge.Budget.ReserveAsync(50, Ct);
            gauge.Attach(lease, committed: 70);

            using var exactFit = await gauge.Budget.ReserveAsync(10, Ct);
            _ = gauge.Budget.ReserveAsync(1, Ct);

            Assert.Equal(1, gauge.Budget.WaitingCount);
        }

        [Fact]
        public async Task CommitOfEveryAttachedProcessIsSummed()
        {
            var gauge = new ChildGauge();
            using var lease = await gauge.Budget.ReserveAsync(50, Ct);
            gauge.Attach(lease, committed: 15);
            gauge.Attach(lease, committed: 25);

            using var exactFit = await gauge.Budget.ReserveAsync(30, Ct);
            _ = gauge.Budget.ReserveAsync(1, Ct);

            Assert.Equal(1, gauge.Budget.WaitingCount);
        }

        [Fact]
        public async Task ProcessThatExitedNoLongerTakesItsCommitOffTheMeasurement()
        {
            var gauge = new ChildGauge();
            using var lease = await gauge.Budget.ReserveAsync(50, Ct);
            var child = gauge.Attach(lease, committed: 40);

            child.Exit();
            gauge.Used = Baseline;
            _ = gauge.Budget.ReserveAsync(31, Ct);

            Assert.Equal(1, gauge.Budget.WaitingCount);
        }

        [Fact]
        public async Task ProcessWhoseMemoryCannotBeReadLeavesItsLeaseCountedInFullAndIsLoggedOnce()
        {
            var gauge = new ChildGauge();
            using var lease = await gauge.Budget.ReserveAsync(50, Ct);
            gauge.Attach(lease, committed: 25);
            var unreadable = gauge.Attach(lease, committed: 15);
            unreadable.Committed = null;

            var waiting = gauge.Budget.ReserveAsync(30, Ct);
            gauge.Clock.Advance(MemoryBudget.RecheckInterval);

            Assert.Equal(1, gauge.Budget.WaitingCount);
            Assert.Single(gauge.Logs, log => log.Contains("could not be read", StringComparison.Ordinal));

            unreadable.Committed = 15;
            gauge.Clock.Advance(MemoryBudget.RecheckInterval);
            (await waiting.WaitAsync(Guard, Ct)).Dispose();
        }

        [Fact]
        public async Task LeasesWithoutChildProcessesStayCountedTwiceBesideOneWithThem()
        {
            var gauge = new ChildGauge();
            using var covering = await gauge.Budget.ReserveAsync(30, Ct);
            using var inProcess = await gauge.Budget.ReserveAsync(20, Ct);
            gauge.Attach(covering, committed: 30);
            gauge.Used += 20;

            using var exactFit = await gauge.Budget.ReserveAsync(10, Ct);
            _ = gauge.Budget.ReserveAsync(1, Ct);

            Assert.Equal(1, gauge.Budget.WaitingCount);
        }

        [Fact]
        public async Task GaugeThatDoesNotCountChildProcessesKeepsTheirLeasesCountedInFull()
        {
            var gauge = new ChildGauge(ChildFootprint.NotCounted);
            using var lease = await gauge.Budget.ReserveAsync(50, Ct);
            using (lease.CoverChildProcesses())
                MemoryLease.AttachToCovering(_ => throw new InvalidOperationException("Nothing is attached to a budget whose gauge does not count child processes."));
            gauge.Used += 40;

            _ = gauge.Budget.ReserveAsync(30, Ct);

            Assert.Equal(1, gauge.Budget.WaitingCount);
        }

        [Fact]
        public async Task ChildProcessesAreNotReadWhileTheGaugeAloneLeavesRoom()
        {
            var gauge = new ChildGauge();
            using var lease = await gauge.Budget.ReserveAsync(30, Ct);
            var child = gauge.Attach(lease, committed: 10);

            using var fits = gauge.Budget.Reserve(10, Ct);
            using var other = await gauge.Budget.ReserveAsync(5, Ct);
            other.Dispose();

            Assert.Equal(0, child.Reads);
        }

        [Fact]
        public async Task WaiterGrantedOnReleaseWithRoomByTheGaugeAloneDoesNotReadChildProcesses()
        {
            var gauge = new ChildGauge();
            using var lease = await gauge.Budget.ReserveAsync(20, Ct);
            var child = gauge.Attach(lease, committed: 10);
            var held = await gauge.Budget.ReserveAsync(50, Ct);
            var waiting = gauge.Budget.ReserveAsync(20, Ct);
            int readsWhileBlocked = child.Reads;

            held.Dispose();
            using var granted = await waiting.WaitAsync(Guard, Ct);

            Assert.Equal(readsWhileBlocked, child.Reads);
        }

        [Fact]
        public async Task ChildCommitIsNotTakenOffAStaleGaugeReading()
        {
            var gauge = new ChildGauge();
            using var lease = await gauge.Budget.ReserveAsync(50, Ct);
            gauge.Attach(lease, committed: 40);
            gauge.Budget.Reserve(1, Ct).Dispose();

            gauge.Failure = new InvalidOperationException("gauge broke");
            var waiting = gauge.Budget.ReserveAsync(30, Ct);
            gauge.Clock.Advance(MemoryBudget.RecheckInterval);

            Assert.False(waiting.IsCompleted);
            gauge.Failure = null;
            gauge.Clock.Advance(MemoryBudget.RecheckInterval);
            (await waiting.WaitAsync(Guard, Ct)).Dispose();
        }

        [Fact]
        public async Task ChildCommitIsNotTakenOffAGaugeThatFailsRightAfterTheChildrenAreRead()
        {
            var gauge = new ChildGauge();
            using var lease = await gauge.Budget.ReserveAsync(50, Ct);
            var child = gauge.Attach(lease, committed: 40);
            child.AfterNextRead = () => gauge.Failure = new InvalidOperationException("gauge broke");

            var waiting = gauge.Budget.ReserveAsync(30, Ct);

            Assert.False(waiting.IsCompleted);
            gauge.Failure = null;
            gauge.Clock.Advance(MemoryBudget.RecheckInterval);
            (await waiting.WaitAsync(Guard, Ct)).Dispose();
        }

        [Fact]
        public async Task CommitGrowingBetweenTheChildAndTheGaugeReadsIsCountedNotMissed()
        {
            var gauge = new ChildGauge();
            using var lease = await gauge.Budget.ReserveAsync(50, Ct);
            var child = gauge.Attach(lease, committed: 40);
            child.AfterNextRead = () =>
            {
                child.Committed += 10;
                gauge.Used += 10;
            };

            _ = gauge.Budget.ReserveAsync(21, Ct);

            Assert.Equal(1, gauge.Budget.WaitingCount);
        }

        [Fact]
        public async Task LeasesWithChildrenChangingDuringTheReadTakeNothingOff()
        {
            var gauge = new ChildGauge();
            using var lease = await gauge.Budget.ReserveAsync(30, Ct);
            using var second = await gauge.Budget.ReserveAsync(10, Ct);
            var child = gauge.Attach(lease, committed: 30);
            child.AfterNextRead = () => second.Attach(new FakeChildMemory { Committed = 0 });

            _ = gauge.Budget.ReserveAsync(11, Ct);

            Assert.Equal(1, gauge.Budget.WaitingCount);
        }

        [Fact]
        public async Task ReleasingTheLeaseDisposesItsChildrenAndStopsTakingThemOff()
        {
            var gauge = new ChildGauge();
            var lease = await gauge.Budget.ReserveAsync(50, Ct);
            using var other = await gauge.Budget.ReserveAsync(1, Ct);
            var child = gauge.Attach(lease, committed: 40);

            lease.Dispose();
            _ = gauge.Budget.ReserveAsync(41, Ct);

            Assert.True(child.Disposed);
            Assert.Equal(1, gauge.Budget.WaitingCount);
        }

        [Fact]
        public async Task ChildAttachedToAReleasedLeaseIsDisposedAtOnce()
        {
            var gauge = new ChildGauge();
            var lease = await gauge.Budget.ReserveAsync(50, Ct);
            lease.Dispose();

            var child = new FakeChildMemory { Committed = 40 };
            lease.Attach(child);

            Assert.True(child.Disposed);
        }

        [Fact]
        public async Task ProcessesStartedWithinTheCoverScopeIncludingItsTasksAreAttachedAndNoneAfterIt()
        {
            var gauge = new ChildGauge();
            using var lease = await gauge.Budget.ReserveAsync(50, Ct);
            var child = new FakeChildMemory { Committed = 40 };
            gauge.Used += 40;

            using (lease.CoverChildProcesses())
                await Task.Run(() => MemoryLease.AttachToCovering(_ => child), Ct);
            MemoryLease.AttachToCovering(_ => throw new InvalidOperationException("Nothing covers the flow after the scope."));

            using var exactFit = await gauge.Budget.ReserveAsync(30, Ct);
            Assert.False(child.Disposed);
        }

        [Fact]
        public async Task NestedCoverScopeRestoresTheOuterLeaseWhenDisposed()
        {
            var gauge = new ChildGauge();
            using var outer = await gauge.Budget.ReserveAsync(30, Ct);
            using var inner = await gauge.Budget.ReserveAsync(20, Ct);
            var outerChild = new FakeChildMemory { Committed = 30 };
            gauge.Used += 30;

            using (outer.CoverChildProcesses())
            {
                using (inner.CoverChildProcesses())
                {
                }

                MemoryLease.AttachToCovering(_ => outerChild);
            }

            using var exactFit = await gauge.Budget.ReserveAsync(30, Ct);
            _ = gauge.Budget.ReserveAsync(1, Ct);
            Assert.Equal(1, gauge.Budget.WaitingCount);
        }

        private sealed class FakeChildMemory : IChildMemory
        {
            public long? Committed { get; set; }

            public bool Disposed { get; private set; }

            public int Reads { get; private set; }

            /// <summary>Runs once, right after the next read, as if the process did something between that read and the next.</summary>
            public Action? AfterNextRead { get; set; }

            public void Exit() => Committed = 0;

            public long? ReadBytes()
            {
                Reads++;
                long? committed = Committed;
                var afterRead = AfterNextRead;
                AfterNextRead = null;
                afterRead?.Invoke();
                return committed;
            }

            public void Dispose() => Disposed = true;
        }

        /// <summary>A budget of <see cref="Capacity"/> whose gauge reads the baseline plus what the attached children commit, as a job gauge would.</summary>
        private sealed class ChildGauge
        {
            public ChildGauge(ChildFootprint childFootprint = ChildFootprint.PrivateCommit)
            {
                Budget = new MemoryBudget(Capacity)
                {
                    UsedBytes = () => Failure is { } failure ? throw failure : Used,
                    ChildFootprint = childFootprint,
                    Clock = Clock,
                    Reclaim = () => { },
                    ReclaimableBytes = () => 0,
                    Log = Logs.Add,
                };
            }

            public ManualTimeProvider Clock { get; } = new();

            public MemoryBudget Budget { get; }

            public List<string> Logs { get; } = [];

            public long Used { get; set; } = Baseline;

            public Exception? Failure { get; set; }

            public FakeChildMemory Attach(MemoryLease lease, long committed)
            {
                var child = new FakeChildMemory { Committed = committed };
                Used += committed;
                lease.Attach(child);
                return child;
            }
        }
    }
}
