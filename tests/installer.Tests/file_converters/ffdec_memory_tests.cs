using AssetConverter;
using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests
{
    public sealed class FfdecMemoryTests
    {
        private static readonly TimeSpan Guard = TimeSpan.FromSeconds(120);

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private static void SkipWithoutFfdec()
        {
            try
            {
                Ffdec.Version(MemoryBudget.Unlimited(), Ct);
            }
            catch (FfdecException e)
            {
                Assert.Skip($"ffdec is not usable here: {e.Message}");
            }
        }

        [Fact]
        public async Task AnInvocationWaitsForTheBudgetAndReleasesItWhenDone()
        {
            SkipWithoutFfdec();
            var budget = new MemoryBudget(Ffdec.VersionProbeBytes);
            var other = await budget.ReserveAsync(1, Ct);

            var version = Task.Run(() => Ffdec.Version(budget, Ct), Ct);
            Assert.True(SpinWait.SpinUntil(() => budget.WaitingCount == 1, Guard));
            Assert.False(version.IsCompleted);
            other.Dispose();

            Assert.NotEmpty(await version.WaitAsync(Guard, Ct));
            Assert.Equal(0, budget.ReservedBytes);
        }

        [Fact]
        public async Task AnInvocationCancelledWhileWaitingEndsWithACancellationAndReservesNothing()
        {
            SkipWithoutFfdec();
            var budget = new MemoryBudget(Ffdec.VersionProbeBytes);
            using var other = await budget.ReserveAsync(1, Ct);
            using var cancel = new CancellationTokenSource();

            var version = Task.Run(() => Ffdec.Version(budget, cancel.Token), Ct);
            Assert.True(SpinWait.SpinUntil(() => budget.WaitingCount == 1, Guard));
            await cancel.CancelAsync();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => version.WaitAsync(Guard, Ct));
            Assert.Equal(1, budget.ReservedBytes);
        }
    }
}
