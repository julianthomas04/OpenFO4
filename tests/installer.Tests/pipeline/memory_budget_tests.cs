using AssetConverter;
using AssetConverter.Pipeline;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests.Pipeline
{
    public sealed class MemoryBudgetTests
    {
        private const long Capacity = 100;
        private static readonly TimeSpan Guard = TimeSpan.FromSeconds(30);

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        [Fact]
        public async Task ReservationThatFitsIsGrantedImmediatelyAndCounted()
        {
            var budget = new MemoryBudget(Capacity);

            using var lease = await budget.ReserveAsync(40, Ct);

            Assert.Equal(40, budget.ReservedBytes);
            Assert.Equal(Capacity, budget.Capacity);
        }

        [Fact]
        public async Task DisposingTheLeaseReleasesItsBytes()
        {
            var budget = new MemoryBudget(Capacity);
            var lease = await budget.ReserveAsync(40, Ct);

            lease.Dispose();

            Assert.Equal(0, budget.ReservedBytes);
        }

        [Fact]
        public async Task DisposingTheLeaseTwiceReleasesOnlyOnce()
        {
            var budget = new MemoryBudget(Capacity);
            var first = await budget.ReserveAsync(40, Ct);
            using var second = await budget.ReserveAsync(30, Ct);

            first.Dispose();
            first.Dispose();

            Assert.Equal(30, budget.ReservedBytes);
        }

        [Fact]
        public async Task RequestThatDoesNotFitWaitsUntilBytesAreReleased()
        {
            var budget = new MemoryBudget(Capacity);
            var held = await budget.ReserveAsync(70, Ct);

            var waiting = budget.ReserveAsync(50, Ct);
            Assert.False(waiting.IsCompleted);
            Assert.Equal(70, budget.ReservedBytes);

            held.Dispose();
            using var granted = await waiting.WaitAsync(Guard, Ct);

            Assert.Equal(50, budget.ReservedBytes);
        }

        [Fact]
        public async Task BlockingReserveWaitsUntilBytesAreReleased()
        {
            var budget = new MemoryBudget(Capacity);
            var held = await budget.ReserveAsync(70, Ct);
            var blocked = Task.Run(() => budget.Reserve(50, Ct), Ct);
            Assert.True(SpinWait.SpinUntil(() => budget.WaitingCount == 1, Guard));

            held.Dispose();
            using var granted = await blocked.WaitAsync(Guard, Ct);

            Assert.Equal(50, budget.ReservedBytes);
        }

        [Fact]
        public async Task SmallRequestIsGrantedWhileAnEarlierLargerOneWaits()
        {
            var budget = new MemoryBudget(Capacity);
            var held = await budget.ReserveAsync(60, Ct);
            var large = budget.ReserveAsync(80, Ct);

            using var small = await budget.ReserveAsync(30, Ct);

            Assert.False(large.IsCompleted);
            Assert.Equal(90, budget.ReservedBytes);
            Assert.Equal(1, budget.WaitingCount);

            held.Dispose();
            small.Dispose();
            using var largeLease = await large.WaitAsync(Guard, Ct);
            Assert.Equal(80, budget.ReservedBytes);
        }

        [Fact]
        public async Task WaitersThatFitAreGrantedInArrivalOrderAndOthersAreSkipped()
        {
            var budget = new MemoryBudget(Capacity);
            var held = await budget.ReserveAsync(100, Ct);
            var first = budget.ReserveAsync(40, Ct);
            var tooLarge = budget.ReserveAsync(90, Ct);
            var third = budget.ReserveAsync(40, Ct);
            var fourth = budget.ReserveAsync(40, Ct);

            held.Dispose();

            using var firstLease = await first.WaitAsync(Guard, Ct);
            using var thirdLease = await third.WaitAsync(Guard, Ct);
            Assert.False(tooLarge.IsCompleted);
            Assert.False(fourth.IsCompleted);
            Assert.Equal(80, budget.ReservedBytes);

            firstLease.Dispose();
            using var fourthLease = await fourth.WaitAsync(Guard, Ct);
            Assert.False(tooLarge.IsCompleted);
        }

        [Fact]
        public async Task LargeWaiterIsGrantedOnceEverythingElseIsReleased()
        {
            var budget = new MemoryBudget(Capacity);
            var held = await budget.ReserveAsync(30, Ct);
            var large = budget.ReserveAsync(80, Ct);
            Assert.False(large.IsCompleted);

            held.Dispose();

            using var largeLease = await large.WaitAsync(Guard, Ct);
            Assert.Equal(80, budget.ReservedBytes);
        }

        [Fact]
        public async Task RequestLargerThanCapacityIsClampedAndRunsAlone()
        {
            var budget = new MemoryBudget(Capacity);

            var oversized = await budget.ReserveAsync(Capacity * 5, Ct);
            var next = budget.ReserveAsync(1, Ct);

            Assert.Equal(Capacity, budget.ReservedBytes);
            Assert.False(next.IsCompleted);

            oversized.Dispose();
            using var nextLease = await next.WaitAsync(Guard, Ct);
            Assert.Equal(1, budget.ReservedBytes);
        }

        [Theory]
        [InlineData(0)]
        [InlineData(-1)]
        public void NonPositiveReservationIsRejected(long bytes)
        {
            var budget = new MemoryBudget(Capacity);

            Assert.Throws<ArgumentOutOfRangeException>(() => { _ = budget.ReserveAsync(bytes, Ct); });
            Assert.Throws<ArgumentOutOfRangeException>(() => budget.Reserve(bytes, Ct));
        }

        [Theory]
        [InlineData(0)]
        [InlineData(-1)]
        public void NonPositiveCapacityIsRejected(long capacity)
        {
            Assert.Throws<ArgumentOutOfRangeException>(() => new MemoryBudget(capacity));
        }

        [Fact]
        public async Task CancelledWaiterLeavesTheQueueWithoutBlockingOthers()
        {
            var budget = new MemoryBudget(Capacity);
            var held = await budget.ReserveAsync(60, Ct);
            using var cancel = new CancellationTokenSource();
            var cancelled = budget.ReserveAsync(80, cancel.Token);
            var behind = budget.ReserveAsync(50, Ct);
            Assert.False(behind.IsCompleted);

            await cancel.CancelAsync();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => cancelled.WaitAsync(Guard, Ct));
            held.Dispose();
            using var behindLease = await behind.WaitAsync(Guard, Ct);
            Assert.Equal(50, budget.ReservedBytes);
            Assert.Equal(0, budget.WaitingCount);
        }

        [Fact]
        public async Task AlreadyCancelledTokenThrowsWithoutReserving()
        {
            var budget = new MemoryBudget(Capacity);
            using var cancel = new CancellationTokenSource();
            await cancel.CancelAsync();

            Assert.Throws<OperationCanceledException>(() => budget.Reserve(10, cancel.Token));

            Assert.Equal(0, budget.ReservedBytes);
            Assert.Equal(0, budget.WaitingCount);
        }

        [Fact]
        public async Task CancellingAfterTheGrantKeepsTheLease()
        {
            var budget = new MemoryBudget(Capacity);
            using var cancel = new CancellationTokenSource();
            using var lease = await budget.ReserveAsync(40, cancel.Token);

            await cancel.CancelAsync();

            Assert.Equal(40, budget.ReservedBytes);
        }

        [Fact]
        public void BudgetFromALimitIsTheConfiguredShareOfIt()
        {
            var budget = MemoryBudget.FromLimit(4_000_000_000);

            Assert.Equal(3_200_000_000, budget.Capacity);
            Assert.Equal(4_000_000_000, budget.LimitBytes);
            Assert.Equal("Memory budget: " + ByteFormat.Format(3_200_000_000) + " of " + ByteFormat.Format(4_000_000_000) + " limit", budget.Describe());
        }

        [Fact]
        public async Task ReservationIsGrantedWhenReservedAndMeasuredBytesBothFit()
        {
            var gauge = new MeasuredBudget { Used = 20 };

            using var lease = await gauge.Budget.ReserveAsync(50, Ct);

            Assert.Empty(gauge.Logs);
            Assert.Equal(0, gauge.Reclaims);
            Assert.Equal(0, gauge.Clock.ActiveTimerCount);
        }

        [Fact]
        public async Task ReservedAndMeasuredBytesAreAddedForAdmission()
        {
            var gauge = new MeasuredBudget { Used = 20 };
            using var held = await gauge.Budget.ReserveAsync(30, Ct);

            using var exactFit = await gauge.Budget.ReserveAsync(50, Ct);
            var oneByteOver = gauge.Budget.ReserveAsync(1, Ct);

            Assert.Equal(80, gauge.Budget.ReservedBytes);
            Assert.False(oneByteOver.IsCompleted);
        }

        [Fact]
        public async Task LeaseThatHasMaterializedIsCountedTwiceSoABaselineBelowTheReservationStaysVisible()
        {
            var gauge = new MeasuredBudget { Used = 20 };
            using var first = await gauge.Budget.ReserveAsync(40, Ct);
            gauge.Used = 60;

            var second = gauge.Budget.ReserveAsync(40, Ct);

            Assert.False(second.IsCompleted);
        }

        [Fact]
        public async Task ReservationThatFitsIsHeldBackWhileMeasuredBytesExceedTheCapacity()
        {
            var gauge = new MeasuredBudget { Used = 80 };
            using var held = await gauge.Budget.ReserveAsync(10, Ct);

            var waiting = gauge.Budget.ReserveAsync(30, Ct);

            Assert.False(waiting.IsCompleted);
            Assert.Equal(1, gauge.Budget.WaitingCount);
            Assert.Equal(10, gauge.Budget.ReservedBytes);
        }

        [Fact]
        public async Task HeldBackReservationIsGrantedWhenTheGaugeDropsAtTheNextRecheck()
        {
            var gauge = new MeasuredBudget { Used = 80 };
            using var held = await gauge.Budget.ReserveAsync(10, Ct);
            var waiting = gauge.Budget.ReserveAsync(30, Ct);

            gauge.Clock.Advance(MemoryBudget.RecheckInterval);
            Assert.False(waiting.IsCompleted);

            gauge.Used = 50;
            gauge.Clock.Advance(MemoryBudget.RecheckInterval);
            using var granted = await waiting.WaitAsync(Guard, Ct);

            Assert.Equal(40, gauge.Budget.ReservedBytes);
            Assert.Equal(0, gauge.Clock.ActiveTimerCount);
        }

        [Fact]
        public async Task BlockedWaiterIsReclaimedAtMostOncePerSecondAndLoggedAtMostOncePerTenSecondsWhileItLasts()
        {
            var gauge = new MeasuredBudget { Used = 80 };
            using var held = await gauge.Budget.ReserveAsync(10, Ct);
            var waiting = gauge.Budget.ReserveAsync(30, Ct);
            Assert.Equal(1, gauge.Reclaims);

            gauge.Clock.Advance(MemoryBudget.ReclaimInterval - MemoryBudget.RecheckInterval);
            Assert.Equal(1, gauge.Reclaims);

            gauge.Clock.Advance(MemoryBudget.RecheckInterval);
            Assert.Equal(2, gauge.Reclaims);
            gauge.Clock.Advance(MemoryBudget.ReclaimInterval);
            Assert.Equal(3, gauge.Reclaims);
            Assert.Equal(["Memory: waiting for 30 B, 90 B in use of 100 B budget; reclaiming"], gauge.Logs);

            gauge.Clock.Advance(MemoryBudget.LogInterval);
            Assert.Equal(2, gauge.Logs.Count);
            Assert.Equal(1, gauge.Clock.ActiveTimerCount);
            gauge.Used = 0;
            gauge.Clock.Advance(MemoryBudget.RecheckInterval);
            using var granted = await waiting.WaitAsync(Guard, Ct);
        }

        [Fact]
        public async Task ReclamationIsSkippedWhileTheRuntimeHoldsLittleUnusedMemory()
        {
            var gauge = new MeasuredBudget { Used = 80, Reclaimable = MemoryBudget.MinimumReclaimableBytes - 1 };
            using var held = await gauge.Budget.ReserveAsync(10, Ct);
            var waiting = gauge.Budget.ReserveAsync(30, Ct);
            gauge.Clock.Advance(MemoryBudget.ReclaimInterval * 3);
            Assert.Equal(0, gauge.Reclaims);

            gauge.Reclaimable = MemoryBudget.MinimumReclaimableBytes;
            gauge.Clock.Advance(MemoryBudget.ReclaimInterval);

            Assert.Equal(1, gauge.Reclaims);
            Assert.False(waiting.IsCompleted);
        }

        [Fact]
        public async Task ReclamationIsRateLimitedAcrossEpisodes()
        {
            var gauge = new MeasuredBudget { Used = 80 };
            using var held = await gauge.Budget.ReserveAsync(10, Ct);
            var first = gauge.Budget.ReserveAsync(30, Ct);
            gauge.Used = 0;
            gauge.Clock.Advance(MemoryBudget.RecheckInterval);
            using var firstLease = await first.WaitAsync(Guard, Ct);

            gauge.Used = 80;
            var second = gauge.Budget.ReserveAsync(30, Ct);

            Assert.Equal(1, gauge.Reclaims);
            gauge.Used = 0;
            gauge.Clock.Advance(MemoryBudget.RecheckInterval);
            using var secondLease = await second.WaitAsync(Guard, Ct);

            gauge.Used = 80;
            gauge.Clock.Advance(MemoryBudget.ReclaimInterval);
            var third = gauge.Budget.ReserveAsync(30, Ct);

            Assert.Equal(2, gauge.Reclaims);
        }

        [Fact]
        public async Task FirstWaiterIsGrantedWhateverTheGaugeReadsWhenNothingIsReserved()
        {
            var gauge = new MeasuredBudget { Used = 95 };

            using var first = await gauge.Budget.ReserveAsync(50, Ct).WaitAsync(Guard, Ct);
            var second = gauge.Budget.ReserveAsync(10, Ct);

            Assert.False(second.IsCompleted);
            Assert.Equal(50, gauge.Budget.ReservedBytes);
            Assert.Equal(1, gauge.Reclaims);
        }

        [Fact]
        public async Task ReclaimingBeforeTheProgressGuaranteeLetsAWaiterThatThenFitsBeGrantedNormally()
        {
            var gauge = new MeasuredBudget { Used = 95 };
            gauge.OnReclaim = () =>
            {
                gauge.ReservedAtReclaim = gauge.Budget.ReservedBytes;
                gauge.Used = 30;
            };

            using var granted = await gauge.Budget.ReserveAsync(50, Ct).WaitAsync(Guard, Ct);
            var second = gauge.Budget.ReserveAsync(20, Ct);

            Assert.Equal(1, gauge.Reclaims);
            Assert.Equal(0, gauge.ReservedAtReclaim);
            Assert.True(second.IsCompleted);
        }

        [Fact]
        public async Task SecondLeaseIsHeldBackWhileTheFirstOneGrantedByTheProgressGuaranteeStillFitsOnlyAlone()
        {
            var gauge = new MeasuredBudget { Used = 30 };
            using var first = await gauge.Budget.ReserveAsync(60, Ct).WaitAsync(Guard, Ct);

            var second = gauge.Budget.ReserveAsync(60, Ct);

            Assert.False(second.IsCompleted);
        }

        [Fact]
        public async Task ProgressGuaranteeHonoursTheReclaimRateLimitAndGrantsWithoutReclaiming()
        {
            var gauge = new MeasuredBudget { Used = 95 };
            var first = await gauge.Budget.ReserveAsync(50, Ct).WaitAsync(Guard, Ct);
            first.Dispose();

            var second = await gauge.Budget.ReserveAsync(50, Ct).WaitAsync(Guard, Ct);
            Assert.Equal(1, gauge.Reclaims);
            second.Dispose();

            gauge.Clock.Advance(MemoryBudget.ReclaimInterval);
            using var third = await gauge.Budget.ReserveAsync(50, Ct).WaitAsync(Guard, Ct);
            Assert.Equal(2, gauge.Reclaims);
        }

        [Fact]
        public async Task ProgressGuaranteeGrantsOneWaiterPerPassWhileTheGaugeStaysHigh()
        {
            var gauge = new MeasuredBudget { Used = 100 };
            var held = await gauge.Budget.ReserveAsync(10, Ct).WaitAsync(Guard, Ct);
            _ = Enumerable.Range(0, 3).Select(_ => gauge.Budget.ReserveAsync(10, Ct)).ToArray();

            held.Dispose();

            Assert.Equal(2, gauge.Budget.WaitingCount);
            Assert.Equal(10, gauge.Budget.ReservedBytes);
        }

        [Fact]
        public async Task CancelledMeasuredWaiterLeavesTheQueueAndStopsTheRechecks()
        {
            var gauge = new MeasuredBudget { Used = 80 };
            using var held = await gauge.Budget.ReserveAsync(10, Ct);
            using var cancel = new CancellationTokenSource();
            var waiting = gauge.Budget.ReserveAsync(30, cancel.Token);
            Assert.Equal(1, gauge.Clock.ActiveTimerCount);

            await cancel.CancelAsync();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => waiting.WaitAsync(Guard, Ct));
            Assert.Equal(0, gauge.Budget.WaitingCount);
            Assert.Equal(0, gauge.Clock.ActiveTimerCount);
        }

        [Fact]
        public void GaugeThatFailsOnItsFirstReadFailsTheReservation()
        {
            var gauge = new MeasuredBudget { Failure = new InvalidOperationException("gauge broke") };

            var thrown = Assert.Throws<InvalidOperationException>(() => gauge.Budget.Reserve(10, Ct));

            Assert.Equal("gauge broke", thrown.Message);
            Assert.Equal(0, gauge.Budget.WaitingCount);
            Assert.Equal(0, gauge.Budget.ReservedBytes);
        }

        [Fact]
        public async Task GaugeFailureAfterAGoodReadingKeepsTheLastReadingAndIsLoggedOncePerStreak()
        {
            var gauge = new MeasuredBudget { Used = 80 };
            using var held = await gauge.Budget.ReserveAsync(10, Ct);
            var waiting = gauge.Budget.ReserveAsync(30, Ct);

            gauge.Failure = new InvalidOperationException("gauge broke");
            gauge.Clock.Advance(MemoryBudget.RecheckInterval * 4);

            Assert.False(waiting.IsCompleted);
            Assert.Single(gauge.Logs, log => log.Contains("gauge broke"));

            gauge.Failure = null;
            gauge.Used = 0;
            gauge.Clock.Advance(MemoryBudget.RecheckInterval);
            using var granted = await waiting.WaitAsync(Guard, Ct);

            gauge.Failure = new InvalidOperationException("gauge broke again");
            using var next = await gauge.Budget.ReserveAsync(10, Ct);
            Assert.Single(gauge.Logs, log => log.Contains("gauge broke again"));
        }

        [Fact]
        public async Task ReleaseGrantsAWaiterOnlyWhenTheGaugeAllowsIt()
        {
            var gauge = new MeasuredBudget { Used = 10 };
            var held = await gauge.Budget.ReserveAsync(40, Ct);
            var small = await gauge.Budget.ReserveAsync(10, Ct);
            gauge.Used = 90;
            var waiting = gauge.Budget.ReserveAsync(20, Ct);

            held.Dispose();
            Assert.False(waiting.IsCompleted);

            gauge.Used = 30;
            small.Dispose();
            using var granted = await waiting.WaitAsync(Guard, Ct);
        }

        [Fact]
        public async Task WaitingIsLoggedAtMostOnceEveryTenSecondsAcrossEpisodes()
        {
            var gauge = new MeasuredBudget { Used = 10 };
            using var held = await gauge.Budget.ReserveAsync(10, Ct);

            await BlockThenGrant(gauge);
            await BlockThenGrant(gauge);
            Assert.Single(gauge.Logs);

            gauge.Clock.Advance(MemoryBudget.LogInterval - TimeSpan.FromMilliseconds(600));
            await BlockThenGrant(gauge);
            Assert.Single(gauge.Logs);

            await BlockThenGrant(gauge);
            Assert.Equal(2, gauge.Logs.Count);
        }

        [Fact]
        public async Task CancelledWaiterFaultsWithItsToken()
        {
            var budget = new MemoryBudget(Capacity);
            using var held = await budget.ReserveAsync(60, Ct);
            using var cancel = new CancellationTokenSource();
            var waiting = budget.ReserveAsync(80, cancel.Token);

            await cancel.CancelAsync();

            var thrown = await Assert.ThrowsAnyAsync<OperationCanceledException>(() => waiting.WaitAsync(Guard, Ct));
            Assert.Equal(cancel.Token, thrown.CancellationToken);
        }

        [Fact]
        public async Task ReservationsDoNotOverflowOnAnUnlimitedBudget()
        {
            var budget = MemoryBudget.Unlimited();
            var first = await budget.ReserveAsync(long.MaxValue, Ct);

            var second = budget.ReserveAsync(long.MaxValue, Ct);

            Assert.False(second.IsCompleted);
            Assert.Equal(long.MaxValue, budget.ReservedBytes);
            first.Dispose();
            using var secondLease = await second.WaitAsync(Guard, Ct);
            Assert.Equal(long.MaxValue, budget.ReservedBytes);
        }

        [Fact]
        public void ReservationThatFitsAllocatesOnlyTheLease()
        {
            var budget = new MemoryBudget(Capacity);
            const int Iterations = 1000;
            for (int i = 0; i < Iterations; i++)
                budget.Reserve(10, Ct).Dispose();

            long before = GC.GetAllocatedBytesForCurrentThread();
            for (int i = 0; i < Iterations; i++)
                budget.Reserve(10, Ct).Dispose();
            long perReservation = (GC.GetAllocatedBytesForCurrentThread() - before) / Iterations;

            Assert.InRange(perReservation, 0, 64);
        }

        [Fact]
        public async Task LongLivedLeaseHeldWhileTheGaugeIsHighDoesNotStarveItsProducers()
        {
            var gauge = new MeasuredBudget { Used = 80 };
            using var pool = await Task.Run(() => gauge.Budget.ReserveLongLived(25, Ct), Ct).WaitAsync(Guard, Ct);

            using var producer = await gauge.Budget.ReserveAsync(10, Ct).WaitAsync(Guard, Ct);
            var next = gauge.Budget.ReserveAsync(10, Ct);

            Assert.False(next.IsCompleted);
            producer.Dispose();
            using var nextLease = await next.WaitAsync(Guard, Ct);
            Assert.Equal(35, gauge.Budget.ReservedBytes);
        }

        [Fact]
        public async Task RequestIsClampedToWhatTheLongLivedLeasesLeaveAndRunsAloneAmongTransientWork()
        {
            var budget = new MemoryBudget(Capacity);
            using var pool = budget.ReserveLongLived(25, Ct);

            var oversized = await budget.ReserveAsync(Capacity * 5, Ct);
            var next = budget.ReserveAsync(1, Ct);

            Assert.Equal(Capacity, budget.ReservedBytes);
            Assert.False(next.IsCompleted);
            oversized.Dispose();
            using var nextLease = await next.WaitAsync(Guard, Ct);
            Assert.Equal(26, budget.ReservedBytes);
        }

        [Fact]
        public async Task DisposingALongLivedLeaseRestoresTheFullCapacity()
        {
            var budget = new MemoryBudget(Capacity);
            budget.ReserveLongLived(25, Ct).Dispose();

            using var oversized = await budget.ReserveAsync(Capacity * 5, Ct);

            Assert.Equal(Capacity, budget.ReservedBytes);
        }

        [Fact]
        public async Task LongLivedLeaseWaitsForTransientWorkLikeAnyOtherRequest()
        {
            var budget = new MemoryBudget(Capacity);
            var held = await budget.ReserveAsync(90, Ct);

            var pool = Task.Run(() => budget.ReserveLongLived(25, Ct), Ct);
            Assert.True(SpinWait.SpinUntil(() => budget.WaitingCount == 1, Guard));

            held.Dispose();
            using var poolLease = await pool.WaitAsync(Guard, Ct);
            Assert.Equal(25, budget.ReservedBytes);
        }

        [Fact]
        public async Task AgedWaiterBlocksLaterWaitersThatFitUntilItIsGranted()
        {
            var gauge = new MeasuredBudget { Used = 0 };
            var held = await gauge.Budget.ReserveAsync(60, Ct);
            var large = gauge.Budget.ReserveAsync(80, Ct);
            var beforeAging = await gauge.Budget.ReserveAsync(30, Ct);
            Assert.False(large.IsCompleted);

            gauge.Clock.Advance(MemoryBudget.AgingThreshold);
            beforeAging.Dispose();
            var small = gauge.Budget.ReserveAsync(30, Ct);

            Assert.False(small.IsCompleted);
            Assert.Equal(60, gauge.Budget.ReservedBytes);

            held.Dispose();
            using var largeLease = await large.WaitAsync(Guard, Ct);
            Assert.False(small.IsCompleted);
            Assert.Equal(80, gauge.Budget.ReservedBytes);

            largeLease.Dispose();
            using var smallLease = await small.WaitAsync(Guard, Ct);
        }

        [Fact]
        public async Task AgedWaiterHeldBackByTheGaugeIsGrantedByTheProgressGuaranteeOnceTransientWorkEnds()
        {
            var gauge = new MeasuredBudget { Used = 70 };
            var held = await gauge.Budget.ReserveAsync(10, Ct);
            var large = gauge.Budget.ReserveAsync(60, Ct);
            gauge.Clock.Advance(MemoryBudget.AgingThreshold);
            var small = gauge.Budget.ReserveAsync(5, Ct);
            Assert.False(large.IsCompleted);
            Assert.False(small.IsCompleted);

            held.Dispose();

            using var largeLease = await large.WaitAsync(Guard, Ct);
            Assert.False(small.IsCompleted);
        }

        [Fact]
        public async Task AgedWaiterThatIsCancelledReleasesTheWaitersBehindIt()
        {
            var gauge = new MeasuredBudget { Used = 0 };
            var held = await gauge.Budget.ReserveAsync(60, Ct);
            using var cancel = new CancellationTokenSource();
            var large = gauge.Budget.ReserveAsync(80, cancel.Token);
            gauge.Clock.Advance(MemoryBudget.AgingThreshold);
            var small = gauge.Budget.ReserveAsync(30, Ct);
            Assert.False(small.IsCompleted);

            await cancel.CancelAsync();

            using var smallLease = await small.WaitAsync(Guard, Ct);
            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => large.WaitAsync(Guard, Ct));
        }

        private static async Task BlockThenGrant(MeasuredBudget gauge)
        {
            gauge.Used = 90;
            var waiting = gauge.Budget.ReserveAsync(30, Ct);
            gauge.Used = 10;
            gauge.Clock.Advance(MemoryBudget.RecheckInterval);
            (await waiting.WaitAsync(Guard, Ct)).Dispose();
        }

        private sealed class MeasuredBudget
        {
            private long _used;

            public MeasuredBudget()
            {
                Budget = new MemoryBudget(Capacity)
                {
                    UsedBytes = ReadUsed,
                    Clock = Clock,
                    Reclaim = () =>
                    {
                        Reclaims++;
                        OnReclaim?.Invoke();
                    },
                    ReclaimableBytes = () => Reclaimable,
                    Log = Logs.Add,
                };
            }

            public ManualTimeProvider Clock { get; } = new();

            public MemoryBudget Budget { get; }

            public List<string> Logs { get; } = [];

            public int Reclaims { get; private set; }

            public long Reclaimable { get; set; } = 1L << 30;

            public Exception? Failure { get; set; }

            public Action? OnReclaim { get; set; }

            public long ReservedAtReclaim { get; set; }

            public long Used
            {
                set => _used = value;
            }

            private long ReadUsed() => Failure is { } failure ? throw failure : _used;
        }
    }
}
