using System.Diagnostics;
using AssetConverter;
using Xunit;

namespace InstallerTests
{
    /// <summary>The cancellation of the collision worker process, driven with a harmless long-running OS command instead of the python worker.</summary>
    [Collection("Console redirection")]
    public sealed class NifWorkerProcessTests : IDisposable
    {
        private static readonly TimeSpan Patience = TimeSpan.FromSeconds(30);

        private readonly TextWriter _originalError = Console.Error;
        private readonly List<Process> _started = new();

        public void Dispose()
        {
            Console.SetError(_originalError);
            foreach (var process in _started)
            {
                if (!process.HasExited) process.Kill(entireProcessTree: true);
                process.Dispose();
            }
        }

        private Process StartCommand(int seconds)
        {
            var psi = OperatingSystem.IsWindows()
                ? new ProcessStartInfo("ping", $"-n {seconds + 1} 127.0.0.1")
                : new ProcessStartInfo("sleep", seconds.ToString(System.Globalization.CultureInfo.InvariantCulture));
            psi.RedirectStandardOutput = true;
            psi.UseShellExecute = false;
            var process = Process.Start(psi)!;
            _started.Add(process);
            return process;
        }

        [Fact]
        public void CancellingWhileTheWorkerRunsKillsItAndThrowsACancellation()
        {
            var worker = StartCommand(seconds: 120);
            using var cancel = new CancellationTokenSource();
            cancel.CancelAfter(TimeSpan.FromMilliseconds(200));

            Assert.ThrowsAny<OperationCanceledException>(() => CollisionWorkerProcess.WaitForWorker(worker, () => CollisionWorkerProcess.KillWorker(worker), cancel.Token));

            Assert.True(worker.WaitForExit(Patience));
        }

        [Fact]
        public void ATokenThatIsAlreadyCancelledStopsTheWorkerAtOnce()
        {
            var worker = StartCommand(seconds: 120);
            using var cancel = new CancellationTokenSource();
            cancel.Cancel();

            Assert.ThrowsAny<OperationCanceledException>(() => CollisionWorkerProcess.WaitForWorker(worker, () => CollisionWorkerProcess.KillWorker(worker), cancel.Token));

            Assert.True(worker.WaitForExit(Patience));
        }

        [Fact]
        public void AWorkerThatFinishesOnItsOwnReturnsNormally()
        {
            var worker = StartCommand(seconds: 1);

            CollisionWorkerProcess.WaitForWorker(worker, () => CollisionWorkerProcess.KillWorker(worker), CancellationToken.None);

            Assert.True(worker.HasExited);
        }

        [Fact]
        public void KillingAWorkerThatAlreadyExitedIsNotAnError()
        {
            var errors = new StringWriter();
            Console.SetError(errors);
            var worker = StartCommand(seconds: 1);
            Assert.True(worker.WaitForExit(Patience));

            CollisionWorkerProcess.KillWorker(worker);

            Assert.DoesNotContain("could not stop the worker", errors.ToString(), StringComparison.Ordinal);
        }

        [Fact]
        public void KillingAWorkerThatWasNeverStartedIsNotAnError()
        {
            var errors = new StringWriter();
            Console.SetError(errors);
            using var neverStarted = new Process { StartInfo = new ProcessStartInfo("ping") };

            CollisionWorkerProcess.KillWorker(neverStarted);

            Assert.DoesNotContain("could not stop the worker", errors.ToString(), StringComparison.Ordinal);
        }

        [Fact]
        public void KillingARunningWorkerEndsItsProcess()
        {
            var worker = StartCommand(seconds: 120);

            CollisionWorkerProcess.KillWorker(worker);

            Assert.True(worker.WaitForExit(Patience));
        }

        [Fact]
        public void LoweringThePriorityOfARunningWorkerSetsBelowNormal()
        {
            Assert.SkipUnless(OperatingSystem.IsWindows(), "Priority classes are a Windows concept.");
            var worker = StartCommand(seconds: 120);

            CollisionWorkerProcess.LowerPriority(worker);

            Assert.Equal(ProcessPriorityClass.BelowNormal, worker.PriorityClass);
        }

        [Fact]
        public void LoweringThePriorityOfAWorkerThatAlreadyExitedWarnsInsteadOfFailing()
        {
            var errors = new StringWriter();
            Console.SetError(errors);
            var worker = StartCommand(seconds: 1);
            Assert.True(worker.WaitForExit(Patience));

            CollisionWorkerProcess.LowerPriority(worker);

            Assert.Contains("WARNING: could not lower the priority of the worker process", errors.ToString(), StringComparison.Ordinal);
        }
    }
}
