using AssetConverter;
using AssetConverter.Logging;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using Xunit;

namespace InstallerTests.Pipeline
{
    /// <summary>The way the install wires the render skeleton priority files of the NIF conversion to the lane that waits for them.</summary>
    [Collection("Console redirection")]
    public sealed class PriorityGateTests : IDisposable
    {
        private static readonly string[] Files = [.. Enumerable.Range(0, 8).Select(i => Path.Combine(Path.GetTempPath(), "priority-gate", $"mesh{i}.nif"))];

        private readonly ConsoleCapture _capture = ConsoleCapture.Start(new LogBuffer(100));
        private readonly LaneRunner _lanes;
        private readonly Gate _skeletonsReady = new("render skeleton GLBs converted");

        public PriorityGateTests()
        {
            InstallProgress.Start([new StepDefinition("Meshes"), new StepDefinition("Animations")]);
            _lanes = new LaneRunner(new PhaseLog(), InstallProgress.CancelToken);
        }

        public void Dispose()
        {
            InstallProgress.Stop();
            _capture.Dispose();
        }

        private PriorityFiles Prioritizing(params string[] paths)
        {
            var priority = new PriorityFiles(_skeletonsReady.Open);
            priority.Prioritize(paths);
            return priority;
        }

        [Fact]
        public async Task TheDependentLaneStartsOnceThePriorityFilesAreConvertedWhileTheProducerStillRunsTheRest()
        {
            var releaseProducer = new TaskCompletionSource();
            var priority = Prioritizing(Files[5], Files[2]);
            var producer = _lanes.Run(["Meshes"], async () =>
            {
                priority.Expect(Files);
                priority.Converted(Files[5]);
                priority.Converted(Files[2]);
                await releaseProducer.Task;
            }, opens: [_skeletonsReady]);
            bool dependentRan = false;

            await _lanes.Run(["Animations"], () => { dependentRan = true; }, after: [_skeletonsReady]).WithTimeout();

            Assert.True(dependentRan);
            Assert.False(producer.IsCompleted);
            releaseProducer.SetResult();
            await producer.WithTimeout();
        }

        [Fact]
        public async Task APriorityFileThatFailedToConvertStillOpensTheGateSoTheDependentLaneSeesTheMissingOutputItself()
        {
            var priority = Prioritizing(Files[1]);
            var producer = _lanes.Run(["Meshes"], () =>
            {
                priority.Expect(Files);
                priority.Converted(Files[1]);
            }, opens: [_skeletonsReady]);
            bool dependentRan = false;

            await _lanes.Run(["Animations"], () => { dependentRan = true; }, after: [_skeletonsReady]).WithTimeout();

            Assert.True(dependentRan);
            await producer.WithTimeout();
        }

        [Fact]
        public async Task AProducerThatFailsBeforeTheLastPriorityFileFailsTheDependentLaneWithItsException()
        {
            var failure = new InvalidOperationException("conversion aborted");
            var priority = Prioritizing(Files[0], Files[1]);
            var producer = _lanes.Run(["Meshes"], () =>
            {
                priority.Expect(Files);
                priority.Converted(Files[0]);
                throw failure;
            }, opens: [_skeletonsReady]);
            bool dependentRan = false;
            var dependent = _lanes.Run(["Animations"], () => { dependentRan = true; }, after: [_skeletonsReady]);

            await Assert.ThrowsAsync<InvalidOperationException>(() => producer.WithTimeout());
            var dependentError = await Assert.ThrowsAsync<InvalidOperationException>(() => dependent.WithTimeout());

            Assert.Same(failure, dependentError);
            Assert.False(dependentRan);
        }
    }
}
