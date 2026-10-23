using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests.Pipeline
{
    public sealed class GateTests
    {
        [Fact]
        public async Task OpeningTheGateReleasesEveryWaiter()
        {
            var gate = new Gate("meshes");
            Task first = gate;
            Task second = Task.Run(async () => await gate.Opened, TestContext.Current.CancellationToken);

            Assert.False(first.IsCompleted);
            gate.Open();
            await first.WithTimeout();
            await second.WithTimeout();

            Assert.True(gate.IsOpen);
        }

        [Fact]
        public async Task AbortingTheGateFailsWaitersWithTheProducersOwnException()
        {
            var gate = new Gate("meshes");
            Task waiter = gate;
            var cause = new IOException("disk full");

            gate.Abort(cause);

            var thrown = await Assert.ThrowsAsync<IOException>(() => waiter.WithTimeout());
            Assert.Same(cause, thrown);
            Assert.False(gate.IsOpen);
        }

        [Fact]
        public async Task AnAggregateExceptionKeepsItsIdentityThroughTheGate()
        {
            var gate = new Gate("meshes");
            var cause = new AggregateException(new InvalidDataException("corrupt"), new IOException("io"));

            gate.Abort(cause);

            var thrown = await Assert.ThrowsAsync<AggregateException>(() => gate.Opened.WithTimeout());
            Assert.Same(cause, thrown);
        }

        [Fact]
        public async Task ACancellationAbortsWaitersAsCancelledNotAsAFailureWithAnotherType()
        {
            var gate = new Gate("meshes");
            var cause = new OperationCanceledException();

            gate.Abort(cause);

            var thrown = await Assert.ThrowsAnyAsync<OperationCanceledException>(() => gate.Opened.WithTimeout());
            Assert.Same(cause, thrown);
        }

        [Fact]
        public async Task AGateThatIsAlreadyOpenIgnoresALaterAbort()
        {
            var gate = new Gate("meshes");
            gate.Open();

            gate.Abort(new IOException("too late"));

            await gate.Opened.WithTimeout();
            Assert.True(gate.IsOpen);
        }

        [Fact]
        public async Task AnAbortedGateStaysFailedWhenOpenedLater()
        {
            var gate = new Gate("meshes");
            var cause = new IOException("disk full");
            gate.Abort(cause);

            gate.Open();

            Assert.Same(cause, await Assert.ThrowsAsync<IOException>(() => gate.Opened.WithTimeout()));
            Assert.False(gate.IsOpen);
        }
    }
}
