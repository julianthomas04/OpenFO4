using AssetConverter.Logging;
using AssetConverter.Progress;
using Xunit;

namespace InstallerTests.Progress
{
    /// <summary>Drives the process-wide <see cref="InstallProgress"/>, so it runs serially with the other tests that redirect the console.</summary>
    [Collection("Console redirection")]
    public sealed class InstallProgressLaneTests : IDisposable
    {
        private readonly TextWriter _originalError = Console.Error;
        private readonly LogBuffer _errors = new(100);
        private readonly ConsoleCapture _capture;

        public InstallProgressLaneTests()
        {
            _capture = ConsoleCapture.Start(_errors);
            InstallProgress.Start([new StepDefinition("Pump"), new StepDefinition("Meshes"), new StepDefinition("Database")]);
        }

        public void Dispose()
        {
            InstallProgress.Stop();
            _capture.Dispose();
            Assert.Same(_originalError, Console.Error);
        }

        private static StepStatus StatusOf(string name) => InstallProgress.Model!.GetSnapshot().Steps.Single(s => s.Name == name).Status;

        private string[] ErrorLines() => [.. _errors.ReadAfter(0).Lines.Where(l => l.IsError).Select(l => l.Text)];

        [Fact]
        public async Task AFailingLaneFailsItsRunningStepCancelsTheOthersAndRethrows()
        {
            var failure = new InvalidOperationException("disk full");
            var other = InstallProgress.RunLane(["Database"], async () =>
            {
                InstallProgress.BeginPhase("Database", 0);
                await Task.Delay(Timeout.Infinite, InstallProgress.CancelToken);
            });

            var thrown = await Assert.ThrowsAsync<InvalidOperationException>(() => InstallProgress.RunLane(["Pump", "Meshes"], () =>
            {
                InstallProgress.BeginPhase("Pump", 0);
                throw failure;
            }));
            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => other);

            Assert.Same(failure, thrown);
            Assert.Equal(StepStatus.Failed, StatusOf("Pump"));
            Assert.Equal(StepStatus.Pending, StatusOf("Meshes"));
            Assert.Equal(StepStatus.Cancelled, StatusOf("Database"));
            Assert.True(InstallProgress.CancelToken.IsCancellationRequested);
            Assert.Same(failure, InstallProgress.Failure);
        }

        [Fact]
        public async Task ACancelledLaneMarksItsStepCancelledWithoutRecordingAFailure()
        {
            InstallProgress.RequestCancel();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => InstallProgress.RunLane(["Meshes"], () =>
            {
                InstallProgress.BeginPhase("Meshes", 0);
                InstallProgress.CancelToken.ThrowIfCancellationRequested();
                return Task.CompletedTask;
            }));

            Assert.Equal(StepStatus.Cancelled, StatusOf("Meshes"));
            Assert.Null(InstallProgress.Failure);
            Assert.DoesNotContain(ErrorLines(), line => line.StartsWith("ERROR:", StringComparison.Ordinal));
        }

        [Fact]
        public async Task ACancellationCausedByAFailureDoesNotHideTheFailure()
        {
            var failure = new InvalidOperationException("boom");
            await Assert.ThrowsAsync<InvalidOperationException>(() => InstallProgress.RunLane(["Pump"], () => throw failure));
            var sympathetic = new OperationCanceledException(InstallProgress.CancelToken);

            RunState outcome = InstallProgress.Conclude(sympathetic);

            Assert.Equal(RunState.Failed, outcome);
            Assert.Equal(RunState.Failed, InstallProgress.Model!.State);
        }

        [Fact]
        public async Task TheSameExceptionSeenByTwoLanesIsLoggedOnce()
        {
            var failure = new InvalidOperationException("shared failure");
            await Assert.ThrowsAsync<InvalidOperationException>(() => InstallProgress.RunLane(["Pump"], () => throw failure));
            await Assert.ThrowsAsync<InvalidOperationException>(() => InstallProgress.RunLane(["Meshes"], () => throw failure));
            InstallProgress.Conclude(failure);

            Assert.Equal(1, ErrorLines().Count(line => line.Contains("shared failure", StringComparison.Ordinal)));
        }

        [Fact]
        public async Task ASecondIndependentFailureIsLoggedButTheFirstStaysTheRunsFailure()
        {
            var first = new InvalidOperationException("first");
            var second = new IOException("second");
            await Assert.ThrowsAsync<InvalidOperationException>(() => InstallProgress.RunLane(["Pump"], () => throw first));
            await Assert.ThrowsAsync<IOException>(() => InstallProgress.RunLane(["Meshes"], () => throw second));

            Assert.Same(first, InstallProgress.Failure);
            Assert.Contains(ErrorLines(), line => line.Contains("second", StringComparison.Ordinal));
        }

        [Fact]
        public async Task ParallelWorkThatStopsOnCancellationAndThrowsAnAggregateOfCancellationsIsACancellationNotAFailure()
        {
            InstallProgress.RequestCancel();
            var aggregate = new AggregateException(
                new OperationCanceledException(InstallProgress.CancelToken),
                new AggregateException(new TaskCanceledException()));

            await Assert.ThrowsAsync<AggregateException>(() => InstallProgress.RunLane(["Meshes"], () =>
            {
                InstallProgress.BeginPhase("Meshes", 0);
                throw aggregate;
            }));
            RunState outcome = InstallProgress.Conclude(aggregate);

            Assert.Equal(StepStatus.Cancelled, StatusOf("Meshes"));
            Assert.Null(InstallProgress.Failure);
            Assert.Equal(RunState.Cancelled, outcome);
        }

        [Fact]
        public async Task AnAggregateWithARealErrorAmongTheCancellationsIsAFailureWhenNobodyCancelled()
        {
            var aggregate = new AggregateException(new OperationCanceledException(), new InvalidDataException("corrupt"));

            await Assert.ThrowsAsync<AggregateException>(() => InstallProgress.RunLane(["Meshes"], () =>
            {
                InstallProgress.BeginPhase("Meshes", 0);
                throw aggregate;
            }));

            Assert.Equal(StepStatus.Failed, StatusOf("Meshes"));
            Assert.Same(aggregate, InstallProgress.Failure);
        }

        [Fact]
        public async Task AnAggregateWithARealErrorAfterTheUserCancelledIsFalloutThatIsLoggedWithItsCause()
        {
            InstallProgress.RequestCancel();
            var aggregate = new AggregateException(new OperationCanceledException(), new InvalidDataException("corrupt"));

            await Assert.ThrowsAsync<AggregateException>(() => InstallProgress.RunLane(["Meshes"], () =>
            {
                InstallProgress.BeginPhase("Meshes", 0);
                throw aggregate;
            }));

            Assert.Equal(StepStatus.Cancelled, StatusOf("Meshes"));
            Assert.Null(InstallProgress.Failure);
            Assert.Contains(ErrorLines(), line => line.Contains("corrupt", StringComparison.Ordinal));
        }

        [Fact]
        public async Task AnEmptyAggregateIsNotACancellation()
        {
            var aggregate = new AggregateException();

            await Assert.ThrowsAsync<AggregateException>(() => InstallProgress.RunLane(["Meshes"], () => throw aggregate));

            Assert.Same(aggregate, InstallProgress.Failure);
        }

        [Fact]
        public async Task ErrorsAfterTheUserCancelledAreFalloutOfTheCancellationAndEndTheRunAsCancelled()
        {
            InstallProgress.RequestCancel();
            var fallout = new ObjectDisposedException("process");

            await Assert.ThrowsAsync<ObjectDisposedException>(() => InstallProgress.RunLane(["Meshes"], () =>
            {
                InstallProgress.BeginPhase("Meshes", 0);
                throw fallout;
            }));
            RunState outcome = InstallProgress.Conclude(fallout);

            Assert.Equal(RunState.Cancelled, outcome);
            Assert.Null(InstallProgress.Failure);
            Assert.Equal(StepStatus.Cancelled, StatusOf("Meshes"));
            Assert.Equal(1, ErrorLines().Count(line => line.StartsWith("WARNING: ignored after the cancellation", StringComparison.Ordinal)));
            Assert.DoesNotContain(ErrorLines(), line => line.StartsWith("ERROR:", StringComparison.Ordinal));
        }

        [Fact]
        public async Task ACancellationRequestedAfterAFailureDoesNotTurnTheFailureIntoACancellation()
        {
            var failure = new InvalidOperationException("boom");
            await Assert.ThrowsAsync<InvalidOperationException>(() => InstallProgress.RunLane(["Pump"], () => throw failure));

            InstallProgress.RequestCancel();
            RunState outcome = InstallProgress.Conclude(new IOException("later"));

            Assert.Equal(RunState.Failed, outcome);
            Assert.Same(failure, InstallProgress.Failure);
        }

        [Fact]
        public void WorkThatFinishesDespiteACancelRequestIsStillACancellation()
        {
            InstallProgress.RequestCancel();

            RunState outcome = InstallProgress.Conclude(null);

            Assert.Equal(RunState.Cancelled, outcome);
            Assert.Equal(RunState.Cancelled, InstallProgress.Model!.State);
        }

        [Fact]
        public void ACancellationCallbackThatThrowsDoesNotStopRequestCancel()
        {
            InstallProgress.CancelToken.Register(() => throw new InvalidOperationException("worker already gone"));

            InstallProgress.RequestCancel();

            Assert.True(InstallProgress.CancelToken.IsCancellationRequested);
            Assert.Contains(ErrorLines(), line => line.Contains("a cancellation callback failed", StringComparison.Ordinal) && line.Contains("worker already gone", StringComparison.Ordinal));
        }

        [Fact]
        public async Task ACancellationCallbackThatThrowsDoesNotReplaceTheOriginalFailure()
        {
            InstallProgress.CancelToken.Register(() => throw new InvalidOperationException("callback bug"));
            var original = new IOException("disk full");

            var thrown = await Assert.ThrowsAsync<IOException>(() => InstallProgress.RunLane(["Pump"], () => throw original));

            Assert.Same(original, thrown);
            Assert.Same(original, InstallProgress.Failure);
            Assert.Equal(RunState.Failed, InstallProgress.Conclude(original));
        }

        [Fact]
        public void ConcludingWithoutAnExceptionSucceeds()
        {
            RunState outcome = InstallProgress.Conclude(null);

            Assert.Equal(RunState.Succeeded, outcome);
            Assert.Equal(RunState.Succeeded, InstallProgress.Model!.State);
        }

        [Fact]
        public void ConcludingWithACancellationAfterACancelRequestIsACancellation()
        {
            InstallProgress.RequestCancel();

            RunState outcome = InstallProgress.Conclude(new OperationCanceledException(InstallProgress.CancelToken));

            Assert.Equal(RunState.Cancelled, outcome);
        }

        [Fact]
        public void ACancellationNobodyRequestedIsAFailure()
        {
            RunState outcome = InstallProgress.Conclude(new OperationCanceledException());

            Assert.Equal(RunState.Failed, outcome);
        }

        [Fact]
        public void AnyOtherExceptionEndsTheRunAsAFailureAndIsLogged()
        {
            RunState outcome = InstallProgress.Conclude(new InvalidDataException("corrupt archive"));

            Assert.Equal(RunState.Failed, outcome);
            Assert.Contains(ErrorLines(), line => line.Contains("corrupt archive", StringComparison.Ordinal));
        }

        [Fact]
        public void PhasesReportedThroughTheFacadeAreCountedAndCallsWithoutAPhaseAreIgnored()
        {
            int phase = InstallProgress.BeginPhase("Meshes", 10, "meshes");
            InstallProgress.Tick(phase, 4);
            InstallProgress.AddTotal(phase, 5);
            InstallProgress.SetDetail(phase, "a.nif");
            InstallProgress.Tick(-1);
            InstallProgress.EndPhase(-1);

            var step = InstallProgress.Model!.GetSnapshot().Steps.Single(s => s.Name == "Meshes");
            Assert.Equal((4L, 15L, "a.nif"), (step.Completed, step.Total, step.Detail));
        }
    }
}
