using System.ComponentModel;
using System.Diagnostics;
using System.Text.RegularExpressions;
using AssetConverter;
using AssetConverter.Logging;
using AssetConverter.Pipeline;
using AssetConverter.Progress;
using Xunit;

namespace InstallerTests
{
    /// <summary>
    /// The live hand-over of NIFs to the collision worker's stdin, driven with harmless OS commands instead of the python worker:
    /// a python one-liner numbers and echoes every stdin line it receives and exits at the end of stdin, <c>cmd /c exit N</c> stands for a worker that fails.
    /// Drives the process-wide InstallProgress and the console, so it runs serially with the other tests that do.
    /// </summary>
    [Collection("Console redirection")]
    public sealed partial class CollisionWorkerProcessTests : IDisposable
    {
        private const string Step = "Extracting collision";
        private const string Root = @"C:\staging\Meshes";

        private const string NumberingScript = "import sys\nfor i, line in enumerate(sys.stdin, 1): print(f'{i}:{line}', end='', flush=True)";

        private static readonly TimeSpan Patience = TimeSpan.FromSeconds(30);

        private readonly LogBuffer _log = new(10_000);
        private readonly ConsoleCapture _capture;
        private readonly int _phase;

        public CollisionWorkerProcessTests()
        {
            _capture = ConsoleCapture.Start(_log);
            InstallProgress.Start([new StepDefinition(Step)]);
            _phase = InstallProgress.BeginPhase(Step, 0);
        }

        public void Dispose()
        {
            InstallProgress.Stop();
            _capture.Dispose();
        }

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        [Fact]
        public async Task PythonAndItsPoolWorkersRunWithoutAConsoleWhileOutputAndErrorsRemainReadable()
        {
            RequireWindows();
            using var scratch = new TempDirectory();
            string script = Path.Combine(scratch.Path, "console_probe.py");
            File.WriteAllText(script, """
                import ctypes
                import multiprocessing as mp
                import sys

                def console_window():
                    return ctypes.windll.kernel32.GetConsoleWindow()

                if __name__ == '__main__':
                    print(f'parent:{console_window()}', flush=True)
                    with mp.Pool(1) as pool:
                        print(f'child:{pool.apply(console_window)}', flush=True)
                    print('probe stderr', file=sys.stderr, flush=True)
                """);
            var startInfo = CollisionWorkerProcess.CreateStartInfo(scratch.Path, scratch.Path, "-", jobs: 1);
            Assert.True(startInfo.CreateNoWindow);
            startInfo.ArgumentList.Clear();
            startInfo.ArgumentList.Add("-u");
            startInfo.ArgumentList.Add(script);
            using var process = Process.Start(startInfo)!;
            var output = process.StandardOutput.ReadToEndAsync(Ct);
            var error = process.StandardError.ReadToEndAsync(Ct);
            try
            {
                await process.WaitForExitAsync(Ct).WaitAsync(Patience, Ct);
                Assert.Equal(0, process.ExitCode);
                Assert.Equal("parent:0\nchild:0\n", (await output).Replace("\r\n", "\n", StringComparison.Ordinal));
                Assert.Equal("probe stderr", (await error).Trim());
            }
            finally
            {
                if (!process.HasExited) process.Kill(entireProcessTree: true);
            }
        }

        [GeneratedRegex(@"^\d+:.+$")]
        private static partial Regex NumberedLine();

        private static void RequireWindows() => Assert.SkipUnless(OperatingSystem.IsWindows(), "The fake workers are Windows commands.");

        private static ProcessStartInfo Command(string file, params string[] arguments)
        {
            var startInfo = new ProcessStartInfo(file) { RedirectStandardOutput = true, RedirectStandardError = true, UseShellExecute = false };
            foreach (string argument in arguments) startInfo.ArgumentList.Add(argument);
            return startInfo;
        }

        private static ProcessStartInfo NumberingWorker() => Command("python.exe", "-u", "-c", NumberingScript);

        private static ProcessStartInfo FailingWorker(int exitCode) => Command("cmd.exe", "/c", $"exit {exitCode}");

        private static string Nif(string relative) => Path.Combine(Root, relative);

        private static NifWorkQueue Queued(bool finished, params string[] relatives)
        {
            var queue = new NifWorkQueue();
            foreach (string relative in relatives) queue.Add(Nif(relative));
            if (finished) queue.Complete();
            return queue;
        }

        private List<string> ReceivedByTheWorker() =>
            [.. _log.ReadAfter(0).Lines.Select(line => line.Text.Trim()).Where(text => NumberedLine().IsMatch(text))];

        private static StepSnapshot Snapshot() => InstallProgress.Model!.GetSnapshot().Steps.Single(step => step.Name == Step);

        private Task RunInBackground(ProcessStartInfo worker, NifWorkQueue queue, CancellationToken ct) =>
            Task.Run(() => CollisionWorkerProcess.RunStreaming(worker, Root, queue, _phase, ct), CancellationToken.None);

        private async Task WaitUntilReceived(int count)
        {
            var deadline = DateTime.UtcNow + Patience;
            while (ReceivedByTheWorker().Count < count && DateTime.UtcNow < deadline) await Task.Delay(20, Ct);
            Assert.True(ReceivedByTheWorker().Count >= count, $"the worker received {ReceivedByTheWorker().Count} of {count} lines");
        }

        [Theory]
        [InlineData(null, 32, 32)]
        [InlineData("", 32, 32)]
        [InlineData("8", 32, 8)]
        [InlineData("61", 32, 61)]
        [InlineData("64", 32, 61)]
        [InlineData("100000", 32, 61)]
        [InlineData(null, 96, 61)]
        [InlineData("99999999999", 96, 61)]
        [InlineData("0", 32, 32)]
        [InlineData("-3", 32, 32)]
        [InlineData("many", 32, 32)]
        [InlineData(null, 0, 1)]
        public void TheWorkerUsesEveryLogicalCoreUnlessTheOverrideNamesAPositiveCount(string? configured, int logicalCores, int expected)
        {
            Assert.Equal(expected, CollisionWorkerProcess.JobCount(configured, logicalCores));
        }

        [Theory]
        [InlineData(null, 32, 1L << 40, 32)]
        [InlineData("8", 32, 1L << 40, 8)]
        [InlineData(null, 32, 3200L << 20, 2)]
        [InlineData(null, 32, 4L * 350 << 20, 1)]
        [InlineData(null, 32, 1L << 20, 1)]
        [InlineData("8", 32, 3200L << 20, 2)]
        [InlineData("1", 32, 3200L << 20, 1)]
        public void ThePoolIsAQuarterOfTheBudgetInProcessesButNeverBeyondTheConfiguredCountOrBelowOne(string? configured, int logicalCores, long capacity, int expected)
        {
            Assert.Equal(expected, CollisionWorkerProcess.PoolSize(configured, logicalCores, capacity));
        }

        [Fact]
        public async Task ThePoolHoldsItsMemoryWhileTheWorkerRunsAndReleasesItWhenTheWorkerEnds()
        {
            RequireWindows();
            using var scratch = new TempDirectory();
            var budget = new MemoryBudget(4L * MemoryEstimates.CollisionPoolProcessBytes);
            using var cancel = CancellationTokenSource.CreateLinkedTokenSource(Ct);
            var run = Task.Run(() => CollisionWorkerProcess.RunStreaming(
                Path.Combine(scratch.Path, "Meshes"), Path.Combine(scratch.Path, "out"), Queued(finished: false), _phase, budget, cancel.Token), CancellationToken.None);

            Assert.True(SpinWait.SpinUntil(() => budget.ReservedBytes == MemoryEstimates.CollisionPoolProcessBytes, Patience));
            await cancel.CancelAsync();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => run.WaitAsync(Patience, Ct));
            Assert.Equal(0, budget.ReservedBytes);
        }

        [Fact]
        public async Task EveryQueuedNifReachesTheWorkerOnceInOrderAndItsStdinClosesWhenTheQueueEnds()
        {
            RequireWindows();

            await RunInBackground(NumberingWorker(), Queued(finished: true, "a.nif", @"x\b.nif", "c.nif"), Ct).WaitAsync(Patience, Ct);

            Assert.Equal(["1:a.nif", @"2:x\b.nif", "3:c.nif"], ReceivedByTheWorker());
        }

        [Fact]
        public async Task EachSentNifIsCountedIntoTheStepTotalAndTheFinishedWorkerCompletesIt()
        {
            RequireWindows();

            await RunInBackground(NumberingWorker(), Queued(finished: true, "a.nif", "b.nif", "c.nif", "d.nif"), Ct).WaitAsync(Patience, Ct);

            Assert.Equal(4, Snapshot().Total);
            Assert.Equal(4, Snapshot().Completed);
        }

        [Fact]
        public async Task ANifIsDecodedWhileTheQueueIsStillOpenAndTheWorkerEndsWhenTheQueueDoes()
        {
            RequireWindows();
            var queue = Queued(finished: false, "early.nif");
            var run = RunInBackground(NumberingWorker(), queue, Ct);

            await WaitUntilReceived(1);
            Assert.False(run.IsCompleted);
            queue.Add(Nif("late.nif"));
            queue.Complete();
            await run.WaitAsync(Patience, Ct);

            Assert.Equal(["1:early.nif", "2:late.nif"], ReceivedByTheWorker());
        }

        [Fact]
        public async Task AnEmptyFinishedQueueStartsAndEndsTheWorkerWithoutAnError()
        {
            RequireWindows();

            await RunInBackground(NumberingWorker(), Queued(finished: true), Ct).WaitAsync(Patience, Ct);

            Assert.Empty(ReceivedByTheWorker());
            Assert.Equal(0, Snapshot().Total);
        }

        [Fact]
        public void AWorkerThatFailsIsReportedWithItsExitCodeWhateverHappenedToTheLinesSentToIt()
        {
            RequireWindows();
            string[] manyNifs = [.. Enumerable.Range(0, 5000).Select(i => $@"folder\a_long_enough_path_to_fill_a_pipe_buffer\nif{i}.nif")];

            var raised = Assert.Throws<InvalidOperationException>(() =>
                CollisionWorkerProcess.RunStreaming(FailingWorker(3), Root, Queued(finished: true, manyNifs), _phase, Ct));

            Assert.Contains("code 3", raised.Message, StringComparison.Ordinal);
        }

        [Fact]
        public async Task AWorkerThatFailsWhileTheQueueIsStillOpenEndsTheRunWithItsExitCode()
        {
            RequireWindows();
            var run = RunInBackground(FailingWorker(7), Queued(finished: false, "a.nif"), Ct);

            var raised = await Assert.ThrowsAsync<InvalidOperationException>(() => run.WaitAsync(Patience, Ct));

            Assert.Contains("code 7", raised.Message, StringComparison.Ordinal);
        }

        [Fact]
        public async Task AWorkerThatExitsSuccessfullyBeforeTheQueueEndedIsAnErrorBecauseItMissedNifs()
        {
            RequireWindows();
            var run = RunInBackground(FailingWorker(0), Queued(finished: false, "a.nif"), Ct);

            var raised = await Assert.ThrowsAsync<InvalidOperationException>(() => run.WaitAsync(Patience, Ct));

            Assert.Contains("before it was sent every collision candidate", raised.Message, StringComparison.Ordinal);
        }

        [Fact]
        public async Task AFailedQueueStopsTheWorkerAndFailsTheRunWithTheQueuesExceptionAsTheCause()
        {
            RequireWindows();
            var queue = Queued(finished: false, "a.nif");
            var run = RunInBackground(NumberingWorker(), queue, Ct);
            await WaitUntilReceived(1);
            var cause = new InvalidOperationException("the extraction failed");

            queue.Fail(cause);
            var raised = await Assert.ThrowsAsync<InvalidOperationException>(() => run.WaitAsync(Patience, Ct));

            Assert.Same(cause, raised);
        }

        [Fact]
        public async Task ANifSourceDirectoryThatDoesNotExistYetIsCreatedBeforeTheRealWorkerStarts()
        {
            RequireWindows();
            using var scratch = new TempDirectory();
            string stagedMeshes = Path.Combine(scratch.Path, "staging", "Meshes");

            await Task.Run(() => CollisionWorkerProcess.RunStreaming(stagedMeshes, Path.Combine(scratch.Path, "out"), Queued(finished: true), _phase, MemoryBudget.Unlimited(), Ct), Ct)
                .WaitAsync(Patience, Ct);

            Assert.True(Directory.Exists(stagedMeshes));
        }

        [Fact]
        public async Task CancellingEndsEveryProcessTheWorkerStartedEvenWhenTheyKeepItsOutputOpen()
        {
            RequireWindows();
            using var cancel = new CancellationTokenSource();
            var run = RunInBackground(Command("python.exe", "-u", "-c", ChildSpawningScript), Queued(finished: false, "go.nif"), cancel.Token);
            int childId = await WaitForChildId();
            using var child = Process.GetProcessById(childId);

            await cancel.CancelAsync();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => run.WaitAsync(Patience, Ct));
            Assert.True(child.WaitForExit(Patience), "the child of the worker was still running");
        }

        [Fact]
        public void APathVariableIsSearchedInOrderForTheFirstDirectoryHoldingTheExecutable()
        {
            using var first = new TempDirectory();
            using var second = new TempDirectory();
            File.WriteAllText(Path.Combine(second.Path, "tool.exe"), "");
            string expected = Path.Combine(second.Path, "tool.exe");

            Assert.Equal(expected, ExecutableSearch.FindOnPath("tool.exe", string.Join(Path.PathSeparator, first.Path, $"\"{second.Path}\"")));
            File.WriteAllText(Path.Combine(first.Path, "tool.exe"), "");
            Assert.Equal(Path.Combine(first.Path, "tool.exe"), ExecutableSearch.FindOnPath("tool.exe", string.Join(Path.PathSeparator, first.Path, second.Path)));
        }

        [Theory]
        [InlineData(null)]
        [InlineData("")]
        [InlineData(";;")]
        public void AnExecutableThatIsInNoDirectoryOfThePathIsNotFound(string? pathVariable)
        {
            Assert.Null(ExecutableSearch.FindOnPath("no_such_collision_worker_program.exe", pathVariable));
        }

        private const string ChildSpawningScript =
            "import subprocess, sys\nsys.stdin.readline()\nchild = subprocess.Popen(['ping', '-n', '120', '127.0.0.1'])\nprint(f'child {child.pid}', flush=True)\nsys.stdin.read()";

        [GeneratedRegex(@"child (\d+)")]
        private static partial Regex ChildLine();

        private async Task<int> WaitForChildId()
        {
            var deadline = DateTime.UtcNow + Patience;
            while (DateTime.UtcNow < deadline)
            {
                var match = _log.ReadAfter(0).Lines.Select(line => ChildLine().Match(line.Text)).FirstOrDefault(m => m.Success);
                if (match != null) return int.Parse(match.Groups[1].Value, System.Globalization.CultureInfo.InvariantCulture);
                await Task.Delay(20, Ct);
            }
            throw new TimeoutException("the worker never reported its child process");
        }

        [Fact]
        public async Task CancellingKillsTheWorkerAndEndsTheRunWithTheCancellationWhileItWaitsForMoreNifs()
        {
            RequireWindows();
            using var cancel = new CancellationTokenSource();
            var queue = Queued(finished: false, "a.nif");
            var run = RunInBackground(NumberingWorker(), queue, cancel.Token);
            await WaitUntilReceived(1);

            await cancel.CancelAsync();

            await Assert.ThrowsAnyAsync<OperationCanceledException>(() => run.WaitAsync(Patience, Ct));
        }

        [Fact]
        public void AWorkerThatCannotBeStartedFailsTheRunAtOnceWithTheStartException()
        {
            RequireWindows();

            Assert.ThrowsAny<Win32Exception>(() =>
                CollisionWorkerProcess.RunStreaming(Command("no_such_collision_worker_program.exe"), Root, Queued(finished: false), _phase, Ct));
        }
    }
}
