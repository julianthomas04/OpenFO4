using System.ComponentModel;
using System.Diagnostics;
using AssetConverter.Pipeline;

namespace AssetConverter
{
    /// <summary>The parts of running an external tool that every tool wrapper (ffdec, ffmpeg) shares: starting it with redirected streams and stopping it.</summary>
    internal static class ToolProcess
    {
        /// <summary>
        /// Starts the tool without a window and with stdout and stderr (and, when asked, stdin) redirected; the caller reads them. The
        /// process is attached to the memory lease covering the calling flow (<see cref="MemoryLease.CoverChildProcesses"/>).
        /// </summary>
        /// <param name="launchFailure">Turns the operating system's refusal to start the program into the wrapper's own exception.</param>
        public static Process Start(ToolCommand command, IEnumerable<string> arguments, bool redirectInput, Func<Win32Exception, Exception> launchFailure)
        {
            var process = new Process();
            command.Apply(process.StartInfo, arguments);
            process.StartInfo.UseShellExecute = false;
            process.StartInfo.RedirectStandardInput = redirectInput;
            process.StartInfo.RedirectStandardOutput = true;
            process.StartInfo.RedirectStandardError = true;
            process.StartInfo.CreateNoWindow = true;
            try
            {
                process.Start();
            }
            catch (Win32Exception ex)
            {
                process.Dispose();
                throw launchFailure(ex);
            }
            if (OperatingSystem.IsWindows())
                MemoryLease.AttachToCovering(footprint => ChildProcessMemory.Of(process, footprint));
            return process;
        }

        /// <summary>Kills the process tree and says what came of it, so a failure that is already being reported is not masked by the kill's own.</summary>
        public static string TryKill(Process p)
        {
            try { p.Kill(entireProcessTree: true); return "killed"; }
            catch (InvalidOperationException) { return "had already exited"; }
            catch (Win32Exception e) { return $"could NOT be killed (win32 err {e.NativeErrorCode}: {e.Message})"; }
        }

        private static int _priorityWarned;

        /// <summary>Lets the next failure to lower a priority warn again; for tests, since the warning is once per process run.</summary>
        internal static void ResetPriorityWarning() => Volatile.Write(ref _priorityWarned, 0);

        /// <summary>Best effort: a tool that keeps normal priority produces the same output, only slower for the other lanes, so a failure is a warning, given once per process run.</summary>
        public static void LowerPriority(Process process)
        {
            try
            {
                process.PriorityClass = ProcessPriorityClass.BelowNormal;
            }
            catch (Exception ex) when (ex is InvalidOperationException or Win32Exception or NotSupportedException)
            {
                if (Interlocked.Exchange(ref _priorityWarned, 1) == 0)
                    Console.Error.WriteLine($"WARNING: could not lower the priority of '{process.StartInfo.FileName}': {ex.Message}");
            }
        }
    }
}
