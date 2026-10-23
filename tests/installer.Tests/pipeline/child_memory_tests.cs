using System.Diagnostics;
using AssetConverter;
using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests.Pipeline
{
    public sealed class ChildMemoryTests : IDisposable
    {
        private static readonly TimeSpan Patience = TimeSpan.FromSeconds(30);
        private readonly List<Process> _started = [];

        private static CancellationToken Ct => TestContext.Current.CancellationToken;

        public void Dispose()
        {
            foreach (var process in _started)
            {
                if (!process.HasExited) process.Kill(entireProcessTree: true);
                process.Dispose();
            }
        }

        [Theory]
        [InlineData(nameof(ChildFootprint.PrivateCommit))]
        [InlineData(nameof(ChildFootprint.PrivateWorkingSet))]
        public void RunningProcessReadsItsFootprintAndAnExitedOneNothing(string footprint)
        {
            RequireWindows();
            var process = StartWaitingForInput();
            using var memory = ChildProcessMemory.Of(process, Enum.Parse<ChildFootprint>(footprint));

            Assert.True(memory.ReadBytes() > 0);

            process.StandardInput.Close();
            Assert.True(process.WaitForExit(Patience));
            Assert.Equal(0, memory.ReadBytes());
        }

        [Fact]
        public void ProcessStaysReadableAfterTheProcessObjectThatStartedItIsDisposed()
        {
            RequireWindows();
            var process = StartWaitingForInput();
            using var memory = ChildProcessMemory.Of(process, ChildFootprint.PrivateCommit);
            var running = Process.GetProcessById(process.Id);
            _started.Add(running);
            _started.Remove(process);

            process.Dispose();

            Assert.True(memory.ReadBytes() > 0);
        }

        [Theory]
        [InlineData(nameof(ChildFootprint.PrivateCommit))]
        [InlineData(nameof(ChildFootprint.PrivateWorkingSet))]
        public void JobReadsTheFootprintOfItsProcessesAndNothingOnceEnded(string footprint)
        {
            RequireWindows();
            var process = StartWaitingForInput();
            using var job = WorkerProcessJob.Contain(process);
            using var memory = job.Memory(Enum.Parse<ChildFootprint>(footprint));

            Assert.True(memory.ReadBytes() > 0);

            job.Terminate();
            Assert.True(process.WaitForExit(Patience));
            Assert.Equal(0, memory.ReadBytes());
            job.Dispose();
            Assert.Equal(0, memory.ReadBytes());
        }

        [Fact]
        public void PrivateWorkingSetLeavesOutTheSharedPagesOfTheWorkingSet()
        {
            RequireWindows();
            var process = StartWaitingForInput();
            using var memory = ChildProcessMemory.Of(process, ChildFootprint.PrivateWorkingSet);

            long? privateWorkingSet = memory.ReadBytes();
            process.Refresh();

            Assert.InRange(privateWorkingSet.GetValueOrDefault(), 1, process.WorkingSet64 - 1);
        }

        [Fact]
        public async Task ToolStartedWithinTheCoverScopeTakesItsFootprintOffTheMeasurement()
        {
            RequireWindows();
            var budget = new MemoryBudget(1000) { UsedBytes = () => 900, ChildFootprint = ChildFootprint.PrivateCommit, ReclaimableBytes = () => 0, Log = _ => { } };
            using var lease = await budget.ReserveAsync(100, Ct);
            var waiting = budget.ReserveAsync(100, Ct);
            Assert.False(waiting.IsCompleted);

            using (lease.CoverChildProcesses())
                _started.Add(ToolProcess.Start(ToolCommand.Program("cmd.exe"), [], redirectInput: true, ex => ex));

            using var granted = await waiting.WaitAsync(Patience, Ct);
        }

        private Process StartWaitingForInput()
        {
            var process = Process.Start(new ProcessStartInfo("cmd.exe") { RedirectStandardInput = true, RedirectStandardOutput = true, UseShellExecute = false })!;
            _started.Add(process);
            return process;
        }

        private static void RequireWindows() => Assert.SkipUnless(OperatingSystem.IsWindows(), "Child process memory is read on Windows only.");
    }
}
