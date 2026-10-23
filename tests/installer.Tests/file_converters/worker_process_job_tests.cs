using System.Diagnostics;
using System.Globalization;
using AssetConverter;
using Xunit;

namespace InstallerTests
{
    /// <summary>The job object that ends a worker and the processes it started, driven with a python one-liner that starts a long-running child and exits.</summary>
    public sealed class WorkerProcessJobTests
    {
        private static readonly TimeSpan Patience = TimeSpan.FromSeconds(30);

        private const string OrphaningScript =
            "import subprocess, sys\nsys.stdin.readline()\nchild = subprocess.Popen(['ping', '-n', '120', '127.0.0.1'], stdout=subprocess.DEVNULL)\nprint(child.pid, flush=True)";

        private static Process StartOrphaningWorker()
        {
            var startInfo = new ProcessStartInfo("python.exe") { RedirectStandardInput = true, RedirectStandardOutput = true, UseShellExecute = false };
            startInfo.ArgumentList.Add("-u");
            startInfo.ArgumentList.Add("-c");
            startInfo.ArgumentList.Add(OrphaningScript);
            return Process.Start(startInfo)!;
        }

        [Fact]
        public void EndingTheJobEndsAProcessThatWasStartedByTheWorkerAfterTheWorkerLeftIt()
        {
            Assert.SkipUnless(OperatingSystem.IsWindows(), "Job objects exist on Windows only.");
            using var worker = StartOrphaningWorker();
            using var job = WorkerProcessJob.Contain(worker);

            worker.StandardInput.WriteLine("go");
            int childId = int.Parse(worker.StandardOutput.ReadLine()!, CultureInfo.InvariantCulture);
            using var child = Process.GetProcessById(childId);
            Assert.True(worker.WaitForExit(Patience));
            Assert.False(child.HasExited);

            job.Terminate();

            Assert.True(child.WaitForExit(Patience), "the child that outlived the worker was still running");
        }

        [Fact]
        public void ClosingTheJobEndsTheProcessesInIt()
        {
            Assert.SkipUnless(OperatingSystem.IsWindows(), "Job objects exist on Windows only.");
            using var worker = StartOrphaningWorker();
            var job = WorkerProcessJob.Contain(worker);
            worker.StandardInput.WriteLine("go");
            int childId = int.Parse(worker.StandardOutput.ReadLine()!, CultureInfo.InvariantCulture);
            using var child = Process.GetProcessById(childId);

            job.Dispose();

            Assert.True(child.WaitForExit(Patience), "the child was still running after its job was closed");
        }
    }
}
