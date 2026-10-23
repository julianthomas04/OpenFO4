using AssetConverter.Logging;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using Xunit;

namespace InstallerTests.Pipeline
{
    /// <summary>Drives the process-wide <see cref="InstallProgress"/>, so it runs serially with the other tests that redirect the console.</summary>
    [Collection("Console redirection")]
    public sealed class LaneRunnerTests : IDisposable
    {
        private readonly LogBuffer _errors = new(100);
        private readonly ConsoleCapture _capture;
        private readonly PhaseLog _phases = new();
        private readonly LaneRunner _lanes;

        public LaneRunnerTests()
        {
            _capture = ConsoleCapture.Start(_errors);
            InstallProgress.Start([new StepDefinition("Pump"), new StepDefinition("Meshes"), new StepDefinition("Database")]);
            _lanes = new LaneRunner(_phases, InstallProgress.CancelToken);
        }

        public void Dispose()
        {
            InstallProgress.Stop();
            _capture.Dispose();
        }

        private static void Fail(Exception exception) => throw exception;

        private string[] ErrorLines() => [.. _errors.ReadAfter(0).Lines.Where(l => l.IsError).Select(l => l.Text)];

        [Fact]
        public async Task ALaneWaitsForItsGatesAndLanesBeforeItsWorkStarts()
        {
            var gate = new Gate("meshes");
            bool producerWorkRan = false, dependentStarted = false;
            var producer = _lanes.Run(["Pump"], () => { producerWorkRan = true; });
            var dependent = _lanes.Run(["Meshes"], () => { dependentStarted = true; }, after: [gate, producer]);

            await producer.WithTimeout();
            Assert.True(producerWorkRan);
            Assert.False(dependentStarted);
            gate.Open();
            await dependent.WithTimeout();

            Assert.True(dependentStarted);
        }

        [Fact]
        public async Task ALaneOpensItsGatesWhileItsWorkRunsAndLanesBehindThemStartBeforeItEnds()
        {
            var gate = new Gate("meshes");
            var releaseProducer = new TaskCompletionSource();
            var producer = _lanes.Run(["Pump"], async () =>
            {
                gate.Open();
                await releaseProducer.Task;
            }, opens: [gate]);
            bool dependentRan = false;

            await _lanes.Run(["Meshes"], () => { dependentRan = true; }, after: [gate]).WithTimeout();

            Assert.True(dependentRan);
            Assert.False(producer.IsCompleted);
            releaseProducer.SetResult();
            await producer.WithTimeout();
        }

        [Fact]
        public async Task AFailingProducerFailsItsDependentsWithTheSameExceptionAndNothingHangs()
        {
            var failure = new IOException("disk full");
            var gate = new Gate("meshes");
            bool dependentRan = false;
            var producer = _lanes.Run(["Pump"], () => Fail(failure), opens: [gate]);
            var dependent = _lanes.Run(["Meshes"], () => { dependentRan = true; }, after: [gate]);
            var behindDependent = _lanes.Run(["Database"], () => { }, after: [dependent]);

            var producerError = await Assert.ThrowsAsync<IOException>(() => producer.WithTimeout());
            var dependentError = await Assert.ThrowsAsync<IOException>(() => dependent.WithTimeout());
            var transitiveError = await Assert.ThrowsAsync<IOException>(() => behindDependent.WithTimeout());

            Assert.Same(failure, producerError);
            Assert.Same(failure, dependentError);
            Assert.Same(failure, transitiveError);
            Assert.False(dependentRan);
            Assert.Same(failure, InstallProgress.Failure);
            Assert.Equal(1, ErrorLines().Count(line => line.Contains("disk full", StringComparison.Ordinal)));
        }

        [Fact]
        public async Task AFailingProducerFaultsOnlyTheGatesItHasNotOpenedYet()
        {
            var opened = new Gate("meshes");
            var pending = new Gate("textures");
            var producer = _lanes.Run(["Pump"], () =>
            {
                opened.Open();
                Fail(new IOException("disk full"));
            }, opens: [opened, pending]);

            await Assert.ThrowsAsync<IOException>(() => producer.WithTimeout());

            await opened.Opened.WithTimeout();
            await Assert.ThrowsAsync<IOException>(() => pending.Opened.WithTimeout());
        }

        [Fact]
        public async Task ACancelledProducerCancelsItsDependentsWithoutRecordingAFailure()
        {
            var gate = new Gate("meshes");
            var producer = _lanes.Run(["Pump"], () =>
            {
                InstallProgress.RequestCancel();
                InstallProgress.CancelToken.ThrowIfCancellationRequested();
            }, opens: [gate]);
            var dependent = _lanes.Run(["Meshes"], () => { }, after: [gate]);

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => producer.WithTimeout());
            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => dependent.WithTimeout());

            Assert.Null(InstallProgress.Failure);
            Assert.DoesNotContain(ErrorLines(), line => line.StartsWith("ERROR:", StringComparison.Ordinal));
        }

        [Fact]
        public async Task ALaneThatEndsWithoutOpeningItsGateFailsAndFailsTheLanesWaitingForIt()
        {
            var gate = new Gate("texture index ready");
            var producer = _lanes.Run(["Pump"], () => { }, opens: [gate]);
            var dependent = _lanes.Run(["Meshes"], () => { }, after: [gate]);

            var producerError = await Assert.ThrowsAsync<InvalidOperationException>(() => producer.WithTimeout());
            var dependentError = await Assert.ThrowsAsync<InvalidOperationException>(() => dependent.WithTimeout());

            Assert.Contains("texture index ready", producerError.Message, StringComparison.Ordinal);
            Assert.Same(producerError, dependentError);
            Assert.Same(producerError, InstallProgress.Failure);
        }

        [Fact]
        public async Task ALaneCancelledBeforeItStartsNeverRunsItsWork()
        {
            InstallProgress.RequestCancel();
            bool ran = false;

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => _lanes.Run(["Meshes"], () => { ran = true; }).WithTimeout());

            Assert.False(ran);
            Assert.Null(InstallProgress.Failure);
        }

        [Fact]
        public async Task ASuccessfulLaneRecordsItsPhaseAndAFailingOneDoesNot()
        {
            await _lanes.Run(["Meshes"], () => { }, phase: "convert").WithTimeout();
            await Assert.ThrowsAsync<IOException>(() => _lanes.Run(["Database"], () => Fail(new IOException("x")), phase: "database").WithTimeout());

            Assert.Equal(["convert"], _phases.Entries.Select(entry => entry.Name));
        }

        [Fact]
        public async Task APhaseStartsOnlyOnceTheLaneHasItsInputs()
        {
            var gate = new Gate("meshes");
            var lane = _lanes.Run(["Meshes"], () => { }, phase: "convert", after: [gate]);
            var openedAt = _phases.Elapsed;

            gate.Open();
            await lane.WithTimeout();

            Assert.True(Assert.Single(_phases.Entries).Start >= openedAt);
        }

        [Fact]
        public async Task AnAsyncLaneIsAwaitedAndAValueLaneHandsItsResultOn()
        {
            bool asyncWorkFinished = false;
            await _lanes.Run(["Pump"], async () =>
            {
                await Task.Yield();
                asyncWorkFinished = true;
            }).WithTimeout();

            int value = await _lanes.Run(["Meshes"], () => 7).WithTimeout();

            Assert.True(asyncWorkFinished);
            Assert.Equal(7, value);
        }
    }
}
