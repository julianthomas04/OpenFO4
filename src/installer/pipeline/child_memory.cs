using System.Diagnostics;
using System.Runtime.InteropServices;
using AssetConverter.Interop;
using Microsoft.Win32.SafeHandles;

namespace AssetConverter.Pipeline
{
    /// <summary>Which part of a child process's memory the budget's gauge counts as that process's alone.</summary>
    internal enum ChildFootprint
    {
        /// <summary>The gauge does not count the installer's child processes.</summary>
        NotCounted,

        /// <summary>The private bytes, which a job charges its processes.</summary>
        PrivateCommit,

        /// <summary>The private working set: the physical memory without the shared pages the whole system counts only once.</summary>
        PrivateWorkingSet,
    }

    /// <summary>The memory of child processes attached to a <see cref="MemoryLease"/>, as the budget's gauge counts it.</summary>
    internal interface IChildMemory : IDisposable
    {
        /// <returns>The bytes in use, zero for processes that have exited, or <c>null</c> when they cannot be read.</returns>
        long? ReadBytes();
    }

    /// <summary>The <see cref="ChildFootprint"/> of a process.</summary>
    internal static class ProcessFootprint
    {
        /// <returns>
        /// The footprint, zero for a process that has exited, or <c>null</c> when it cannot be read. A system that does not report the
        /// private working set reads as zero, so nothing is taken off the gauge for it.
        /// </returns>
        public static long? Read(SafeProcessHandle process, ChildFootprint footprint)
        {
            if (process.IsInvalid)
                return null;

            ulong? bytes = footprint switch
            {
                ChildFootprint.PrivateCommit => ReadPrivateCommit(process),
                ChildFootprint.PrivateWorkingSet => ReadPrivateWorkingSet(process),
                _ => throw new ArgumentOutOfRangeException(nameof(footprint), footprint, "Only a counted footprint can be read."),
            };
            if (JobObjectNative.GetExitCodeProcess(process, out uint exitCode) && exitCode != JobObjectNative.StillActive)
                return 0;
            return bytes is { } read ? (long)Math.Min(read, long.MaxValue) : null;
        }

        private static ulong? ReadPrivateCommit(SafeProcessHandle process)
        {
            var counters = new JobObjectNative.ProcessMemoryCountersEx { Size = (uint)Marshal.SizeOf<JobObjectNative.ProcessMemoryCountersEx>() };
            return JobObjectNative.GetProcessMemoryInfo(process, ref counters, counters.Size) ? (ulong)counters.PrivateUsage : null;
        }

        private static ulong? ReadPrivateWorkingSet(SafeProcessHandle process)
        {
            uint size = (uint)Marshal.SizeOf<JobObjectNative.ProcessMemoryCountersEx2>();
            var counters = new JobObjectNative.ProcessMemoryCountersEx2 { Ex = { Size = size } };
            return JobObjectNative.GetProcessMemoryInfo(process, ref counters, size) ? (ulong)counters.PrivateWorkingSetSize : null;
        }
    }

    /// <summary>A child process, read through a handle of its own, so it stays readable whatever becomes of the <see cref="Process"/> that started it.</summary>
    internal sealed class ChildProcessMemory : IChildMemory
    {
        private readonly SafeProcessHandle _handle;
        private readonly ChildFootprint _footprint;

        private ChildProcessMemory(SafeProcessHandle handle, ChildFootprint footprint)
        {
            _handle = handle;
            _footprint = footprint;
        }

        /// <summary>A process that cannot be opened is attached unreadable, so its lease counts in full.</summary>
        public static ChildProcessMemory Of(Process process, ChildFootprint footprint) =>
            new(JobObjectNative.OpenProcess(JobObjectNative.ProcessQueryLimitedInformation, false, (uint)process.Id), footprint);

        /// <summary>A handle closed by the lease's release while being read reads as nothing committed: the lease no longer counts either.</summary>
        public long? ReadBytes()
        {
            try
            {
                return ProcessFootprint.Read(_handle, _footprint);
            }
            catch (ObjectDisposedException)
            {
                return 0;
            }
        }

        public void Dispose() => _handle.Dispose();
    }
}
