using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using System.Runtime.ExceptionServices;
using AssetConverter.Pipeline;
using AssetConverter.Progress;

namespace AssetConverter
{
    /// <summary>
    /// Runs the FO4 collision worker, the python batch decoder <c>src/installer/python/fo4_collision_worker.py</c>, as one child process at
    /// below-normal priority, relays its output as installer log and step progress, and ends it on cancellation. The NIFs it looks at come
    /// either from a list file that is complete before it starts, or from a <see cref="NifWorkQueue"/> that is written to its stdin while
    /// the NIFs are still being extracted, so its decoding overlaps the extraction. The pool of processes the worker starts is sized to
    /// a share of the <see cref="MemoryBudget"/> and holds that memory for as long as the worker runs.
    /// </summary>
    internal static class CollisionWorkerProcess
    {
        internal const string JobsVariable = "OPENFO4_COLLISION_JOBS";

        /// <summary>Python's multiprocessing.Pool on Windows waits on its workers' handles at once, which the OS limits to 63 including the pool's own two.</summary>
        internal const int MaxPoolProcesses = 61;

        private const string ScriptRelativePath = "src/installer/python/fo4_collision_worker.py";

        private static readonly TimeSpan GraceAfterKill = TimeSpan.FromSeconds(10);

        public static void RunFromList(string sourceDir, string destDir, string nifListPath, int progressPhase, MemoryBudget memory, CancellationToken ct)
        {
            using var pool = ReservePool(memory, ct, out int jobs);
            using var covered = pool.CoverChildProcesses();
            using var worker = Worker.Start(CreateStartInfo(sourceDir, destDir, nifListPath, jobs), progressPhase);
            WaitForWorker(worker.Process, worker.Kill, ct);
            worker.RequireExitCodeZero();
            worker.Finish();
        }

        /// <summary>
        /// Starts the worker on stdin input and sends it every file of <paramref name="candidates"/> as it is queued, closing its stdin once
        /// the queue is finished and drained. Returns when the worker has processed them all and exited. The worker starts before the first
        /// NIF is extracted, so <paramref name="sourceDir"/>, which it requires to exist, is created here when the extraction has not yet.
        /// </summary>
        /// <exception cref="OperationCanceledException"><paramref name="ct"/> was cancelled; the worker is killed.</exception>
        /// <exception cref="InvalidOperationException">The worker failed or ended before it had every candidate, or sending to it failed (the cause is the inner exception).</exception>
        /// <exception cref="Exception">The queue failed: the worker is killed and the queue's own exception is raised.</exception>
        public static void RunStreaming(string sourceDir, string destDir, NifWorkQueue candidates, int progressPhase, MemoryBudget memory, CancellationToken ct)
        {
            Directory.CreateDirectory(sourceDir);
            using var pool = ReservePool(memory, ct, out int jobs);
            using var covered = pool.CoverChildProcesses();
            RunStreaming(CreateStartInfo(sourceDir, destDir, CollisionNifList.StdinArgument, jobs), sourceDir, candidates, progressPhase, ct);
        }

        internal static void RunStreaming(ProcessStartInfo startInfo, string sourceDir, NifWorkQueue candidates, int progressPhase, CancellationToken ct)
        {
            startInfo.RedirectStandardInput = true;
            using var worker = Worker.Start(startInfo, progressPhase);
            using var stopFeeding = CancellationTokenSource.CreateLinkedTokenSource(ct);
            var feeding = Task.Run(() => Feed(worker, sourceDir, candidates, stopFeeding.Token), CancellationToken.None);
            try
            {
                WaitForWorker(worker.Process, worker.Kill, ct);
            }
            finally
            {
                stopFeeding.Cancel();
                feeding.Wait(CancellationToken.None);
            }

            var outcome = feeding.Result;
            if (outcome.Fault != null)
            {
                if (ReferenceEquals(outcome.Fault, candidates.Failure))
                    ExceptionDispatchInfo.Capture(outcome.Fault).Throw();
                throw new InvalidOperationException(
                    $"Sending NIFs to the FO4 collision worker failed (worker exit code {worker.Process.ExitCode} after {worker.Elapsed.Elapsed:mm\\:ss}): {outcome.Fault.Message}",
                    outcome.Fault);
            }
            worker.RequireExitCodeZero();
            if (!outcome.Sent)
                throw new InvalidOperationException("The FO4 collision worker exited before it was sent every collision candidate.");
            worker.Finish();
        }

        private readonly record struct Feeding(bool Sent, Exception? Fault);

        /// <returns>Whether every candidate was sent and stdin closed; a failure is returned, never thrown, so the caller can join the feeder before it reports.</returns>
        private static Feeding Feed(Worker worker, string sourceDir, NifWorkQueue candidates, CancellationToken stop)
        {
            try
            {
                using (var stdin = CollisionNifList.OpenWriter(worker.Process.StandardInput.BaseStream))
                    CollisionNifList.StreamTo(stdin, sourceDir, candidates, () => worker.Progress.AddTotal(1), stop);
                worker.Process.StandardInput.Close();
                return new Feeding(true, null);
            }
            catch (OperationCanceledException) when (stop.IsCancellationRequested)
            {
                return new Feeding(false, null);
            }
            catch (Exception ex)
            {
                worker.Kill();
                return new Feeding(false, ex);
            }
        }

        internal static ProcessStartInfo CreateStartInfo(string sourceDir, string destDir, string filesFrom, int jobs)
        {
            string script = InstallerPaths.Require(ScriptRelativePath);
            string python = InstallerPaths.Resources.PythonExecutable()
                ?? throw new FileNotFoundException("python not found on PATH; it is required to run the FO4 collision worker.");

            var psi = new ProcessStartInfo
            {
                FileName = python,
                WorkingDirectory = InstallerPaths.Resources.Root,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                UseShellExecute = false,
                CreateNoWindow = true,
            };
            // Unbuffered, or a piped stdout delivers every progress line only when the worker exits.
            psi.ArgumentList.Add("-u");
            psi.ArgumentList.Add(script);
            psi.ArgumentList.Add("--in-dir");
            psi.ArgumentList.Add(sourceDir);
            psi.ArgumentList.Add("--files-from");
            psi.ArgumentList.Add(filesFrom);
            psi.ArgumentList.Add("--out-dir");
            psi.ArgumentList.Add(destDir);
            psi.ArgumentList.Add("--skip-existing");
            psi.ArgumentList.Add("--jobs");
            psi.ArgumentList.Add(jobs.ToString(CultureInfo.InvariantCulture));
            return psi;
        }

        /// <summary>
        /// Havok decode is pure Python and each NIF is latency bound (about 50 ms wall for 3 ms of CPU), so the worker processes use every
        /// logical core, up to <see cref="MaxPoolProcesses"/>; at below-normal priority they only take what the other lanes leave.
        /// <see cref="JobsVariable"/> overrides the count, within the same limit.
        /// </summary>
        internal static int JobCount(string? configured, int logicalCores)
        {
            int requested = int.TryParse(configured, NumberStyles.None, CultureInfo.InvariantCulture, out int jobs) && jobs > 0 ? jobs : logicalCores;
            return Math.Clamp(requested, 1, MaxPoolProcesses);
        }

        /// <summary>
        /// The pool size the memory budget allows: <see cref="MemoryEstimates.CollisionPoolCapacityShare"/> of the capacity divided by what a
        /// pool process may use, but at least one process and at most <see cref="JobCount"/>.
        /// </summary>
        internal static int PoolSize(string? configured, int logicalCores, long budgetCapacity)
        {
            int wanted = JobCount(configured, logicalCores);
            double affordable = Math.Floor(budgetCapacity * MemoryEstimates.CollisionPoolCapacityShare / MemoryEstimates.CollisionPoolProcessBytes);
            return (int)Math.Clamp(affordable, 1, wanted);
        }

        /// <summary>Holds the memory of the whole pool until disposed; the pool is never resized, so the reservation never grows while the worker waits for input.</summary>
        private static MemoryLease ReservePool(MemoryBudget memory, CancellationToken ct, out int jobs)
        {
            string? configured = Environment.GetEnvironmentVariable(JobsVariable);
            jobs = PoolSize(configured, Environment.ProcessorCount, memory.Capacity);
            int wanted = JobCount(configured, Environment.ProcessorCount);
            if (jobs < wanted)
                Console.WriteLine($"Collision pool limited to {jobs} of {wanted} processes by the memory budget.");
            return memory.ReserveLongLived(jobs * MemoryEstimates.CollisionPoolProcessBytes, ct);
        }

        /// <summary>A started worker process with the progress its output feeds and the clock of its run.</summary>
        private sealed class Worker : IDisposable
        {
            private WorkerProcessJob? _job;

            private Worker(Process process, CollisionWorkerProgress progress)
            {
                Process = process;
                Progress = progress;
            }

            public Process Process { get; }

            public CollisionWorkerProgress Progress { get; }

            public Stopwatch Elapsed { get; } = Stopwatch.StartNew();

            public static Worker Start(ProcessStartInfo startInfo, int progressPhase)
            {
                Console.WriteLine();
                Console.WriteLine("Extracting FO4 collision via PyNifly (src/installer/python/fo4_collision_worker.py)…");
                var process = new Process { StartInfo = startInfo, EnableRaisingEvents = true };
                var progress = new CollisionWorkerProgress(progressPhase);
                process.OutputDataReceived += (_, e) =>
                {
                    if (e.Data == null) return;
                    Console.WriteLine($"  {e.Data}");
                    InstallProgress.SetDetail(progressPhase, e.Data.Trim());
                    progress.Handle(e.Data);
                };
                process.ErrorDataReceived += (_, e) => { if (e.Data != null) Console.Error.WriteLine($"  {e.Data}"); };
                var worker = new Worker(process, progress);
                try
                {
                    process.Start();
                    worker._job = TryContainInJob(process);
                    if (OperatingSystem.IsWindows())
                        MemoryLease.AttachToCovering(footprint => worker._job?.Memory(footprint) ?? ChildProcessMemory.Of(process, footprint));
                    LowerPriority(process);
                    process.BeginOutputReadLine();
                    process.BeginErrorReadLine();
                    return worker;
                }
                catch
                {
                    worker.Kill();
                    worker.Dispose();
                    throw;
                }
            }

            /// <summary>Ends the worker and every process it started, including ones it starts while this runs.</summary>
            public void Kill()
            {
                if (_job != null)
                {
                    try
                    {
                        _job.Terminate();
                        return;
                    }
                    catch (Win32Exception ex)
                    {
                        Console.Error.WriteLine($"WARNING: could not end the worker's job, stopping its process tree instead: {ex.Message}");
                    }
                }
                KillWorker(Process);
            }

            /// <exception cref="InvalidOperationException">The worker exited with an error code.</exception>
            public void RequireExitCodeZero()
            {
                Elapsed.Stop();
                if (Process.ExitCode != 0)
                    throw new InvalidOperationException($"FO4 collision worker exited with code {Process.ExitCode} after {Elapsed.Elapsed:mm\\:ss}.");
            }

            /// <summary>The worker exited successfully: NIFs after its last progress line are done too.</summary>
            public void Finish()
            {
                Progress.Complete();
                Console.WriteLine($"Collision extraction done in {Elapsed.Elapsed:mm\\:ss}.");
            }

            public void Dispose()
            {
                _job?.Dispose();
                Process.Dispose();
            }
        }

        /// <summary>
        /// Best-effort first line of defence, covering the worker's start-up: a worker that keeps normal priority still produces the same
        /// output, only slower for the other lanes, so a failure is a warning. The worker also lowers its own priority class (which its
        /// pool processes inherit), because the process started here may be a launcher stub in front of the real interpreter.
        /// </summary>
        internal static void LowerPriority(Process worker)
        {
            try
            {
                worker.PriorityClass = ProcessPriorityClass.BelowNormal;
            }
            catch (Exception ex) when (ex is InvalidOperationException or Win32Exception or NotSupportedException)
            {
                Console.Error.WriteLine($"WARNING: could not lower the priority of the worker process: {ex.Message}");
            }
        }

        /// <summary>
        /// Waits for the worker to exit and its output to end; cancelling <paramref name="ct"/> calls <paramref name="kill"/>, waits a
        /// bounded time for the exit and throws <see cref="OperationCanceledException"/>.
        /// </summary>
        internal static void WaitForWorker(Process worker, Action kill, CancellationToken ct)
        {
            try
            {
                worker.WaitForExitAsync(ct).GetAwaiter().GetResult();
            }
            catch (OperationCanceledException)
            {
                kill();
                if (!worker.WaitForExit((int)GraceAfterKill.TotalMilliseconds))
                    Console.Error.WriteLine($"WARNING: the worker process did not exit within {GraceAfterKill.TotalSeconds:0} s of being killed.");
                throw;
            }
        }

        /// <summary>Puts the worker's process tree in a job; without one, <see cref="Worker.Kill"/> can only stop the tree as it is at that moment.</summary>
        private static WorkerProcessJob? TryContainInJob(Process worker)
        {
            if (!OperatingSystem.IsWindows()) return null;
            try
            {
                return WorkerProcessJob.Contain(worker);
            }
            catch (Win32Exception ex)
            {
                Console.Error.WriteLine($"WARNING: could not put the worker process in a job object, so processes it starts late may outlive a cancellation: {ex.Message}");
                return null;
            }
        }

        internal static void KillWorker(Process worker)
        {
            try
            {
                worker.Kill(entireProcessTree: true);
            }
            catch (InvalidOperationException)
            {
                // Already exited: nothing left to kill.
            }
            catch (Exception ex) when (ex is Win32Exception or AggregateException or NotSupportedException)
            {
                Console.Error.WriteLine($"WARNING: could not stop the worker process tree: {ex.Message}");
            }
        }
    }
}
