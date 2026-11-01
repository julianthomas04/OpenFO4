using AssetConverter.Pipeline;
using AssetConverter.Progress;
using Xunit;

namespace InstallerTests.Pipeline
{
    /// <summary>The collision lane starts only once the mesh extraction is over and the candidates left by earlier runs are queued, so the candidate set it reports progress against is final.</summary>
    [Collection("Console redirection")]
    public sealed class InstallPipelineCollisionLanesTests : IDisposable
    {
        private static readonly TimeSpan Patience = TimeSpan.FromSeconds(30);

        private readonly TextWriter _originalError = Console.Error;
        private readonly List<string> _events = [];

        public InstallPipelineCollisionLanesTests() => Console.SetError(new StringWriter());

        public void Dispose()
        {
            Console.SetError(_originalError);
            InstallProgress.Stop();
        }

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        private static LaneRunner Lanes() => new(new PhaseLog(), Ct);

        private void Record(string name)
        {
            lock (_events) _events.Add(name);
        }

        [Fact]
        public async Task CollisionRunsAfterTheMeshExtractionAndAfterTheBacklogIsQueued()
        {
            var meshesExtracted = new TaskCompletionSource();
            var backlogStarted = new SemaphoreSlim(0);
            var releaseBacklog = new ManualResetEventSlim();

            var (collision, backlog) = InstallPipeline.StartCollisionLanes(Lanes(), meshesExtracted.Task,
                () =>
                {
                    Record("backlog starts");
                    backlogStarted.Release();
                    releaseBacklog.Wait(Patience, Ct);
                    Record("backlog queued");
                },
                () => Record("collision starts"));

            Record("meshes extracted");
            meshesExtracted.SetResult();
            Assert.True(await backlogStarted.WaitAsync(Patience, Ct));
            Record("backlog released");
            releaseBacklog.Set();
            await Task.WhenAll(collision, backlog).WaitAsync(Patience, Ct);

            Assert.Equal(["meshes extracted", "backlog starts", "backlog released", "backlog queued", "collision starts"], _events);
        }

        [Fact]
        public async Task ABacklogThatFailsFailsTheCollisionLaneWithoutStartingIt()
        {
            var failure = new InvalidOperationException("queuing failed");

            var (collision, backlog) = InstallPipeline.StartCollisionLanes(Lanes(), Task.CompletedTask,
                () => throw failure,
                () => Record("collision starts"));

            Assert.Same(failure, await Assert.ThrowsAsync<InvalidOperationException>(() => backlog.WaitAsync(Patience, Ct)));
            Assert.Same(failure, await Assert.ThrowsAsync<InvalidOperationException>(() => collision.WaitAsync(Patience, Ct)));
            Assert.Empty(_events);
        }
    }
}
