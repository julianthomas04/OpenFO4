using System.ComponentModel;
using System.Runtime.InteropServices;
using AssetConverter.Interop;

namespace AssetConverter.Pipeline
{
    /// <summary>The memory the install may use at most, with the gauge of how much of it is in use right now.</summary>
    /// <param name="Bytes">The effective limit.</param>
    /// <param name="UsedBytes">Reads the memory currently used of the pool the limit constrains. Cheap enough to call per reservation.</param>
    /// <param name="ChildFootprint">Which memory of the installer's child processes that reading counts.</param>
    internal readonly record struct MemoryLimit(long Bytes, Func<long> UsedBytes, ChildFootprint ChildFootprint);

    /// <summary>Which pool the effective limit constrains, and so what the matching usage gauge measures.</summary>
    internal enum MemoryPool
    {
        /// <summary>The whole machine: used physical memory, which deliberately counts other programs.</summary>
        Physical,

        /// <summary>The memory committed by every process of the Job Object the installer runs in, child processes included.</summary>
        Job,

        /// <summary>The memory committed by the installer process alone.</summary>
        Process,
    }

    /// <summary>
    /// The memory the install may use at most: the smaller of the physical memory and the memory limit of the Job Object the process runs
    /// in, paired with a gauge of the pool that limit constrains. The runtime's own figure is not used, since under a job limit it reports
    /// the GC heap hard limit, a fraction of the job limit.
    /// <para>
    /// Only the innermost job is visible to the query, so a limit set on an enclosing job of a nested job is not seen.
    /// </para>
    /// </summary>
    internal static class SystemMemoryLimit
    {
        /// <exception cref="Win32Exception">The physical memory or the job limits could not be queried.</exception>
        public static MemoryLimit Query(Action<string>? log = null)
        {
            if (!OperatingSystem.IsWindows())
                return new MemoryLimit(GC.GetGCMemoryInfo().TotalAvailableMemoryBytes, () => Environment.WorkingSet, ChildFootprint.NotCounted);

            var (processLimit, jobLimit, jobLimitFlags) = QueryJobMemoryLimits();
            long physical = ClampToLong(QueryMemoryStatus().TotalPhysicalBytes);
            var pool = PoolOf(physical, processLimit, jobLimit);
            return new MemoryLimit(Effective(physical, processLimit, jobLimit), GaugeFor(pool, log ?? Console.WriteLine), ChildFootprintOf(pool, jobLimitFlags));
        }

        /// <summary>
        /// What the gauge of <paramref name="pool"/> counts of the installer's child processes. A job that lets processes break away
        /// may not hold them, so its gauge is not trusted to count them.
        /// </summary>
        internal static ChildFootprint ChildFootprintOf(MemoryPool pool, uint jobLimitFlags) => pool switch
        {
            MemoryPool.Physical => ChildFootprint.PrivateWorkingSet,
            MemoryPool.Job when (jobLimitFlags & (JobObjectNative.LimitBreakawayOk | JobObjectNative.LimitSilentBreakawayOk)) != 0 => ChildFootprint.NotCounted,
            MemoryPool.Job => ChildFootprint.PrivateCommit,
            MemoryPool.Process => ChildFootprint.NotCounted,
            _ => throw new ArgumentOutOfRangeException(nameof(pool), pool, "Unknown memory pool."),
        };

        /// <summary>The smallest of the physical memory and the limits that are set.</summary>
        internal static long Effective(long physicalBytes, long? processLimitBytes, long? jobLimitBytes) =>
            Math.Min(physicalBytes, Math.Min(processLimitBytes ?? long.MaxValue, jobLimitBytes ?? long.MaxValue));

        /// <summary>The pool whose limit is the effective one; the job wins a tie with the process limit.</summary>
        internal static MemoryPool PoolOf(long physicalBytes, long? processLimitBytes, long? jobLimitBytes)
        {
            long effective = Effective(physicalBytes, processLimitBytes, jobLimitBytes);
            if (effective >= physicalBytes)
                return MemoryPool.Physical;
            return jobLimitBytes == effective ? MemoryPool.Job : MemoryPool.Process;
        }

        private static Func<long> GaugeFor(MemoryPool pool, Action<string> log) => pool switch
        {
            MemoryPool.Job => new JobMemoryGauge(QueryJobMemoryUsed, QueryJobProcessPrivateBytes, log).Read,
            MemoryPool.Process => QueryProcessMemoryUsed,
            _ => QueryPhysicalMemoryUsed,
        };

        private static long QueryPhysicalMemoryUsed()
        {
            var status = QueryMemoryStatus();
            return ClampToLong(status.TotalPhysicalBytes - status.AvailablePhysicalBytes);
        }

        private static long QueryJobMemoryUsed()
        {
            if (!JobObjectNative.QueryInformationJobObject(IntPtr.Zero, JobObjectNative.MemoryUsageInformationClass, out JobObjectNative.MemoryUsageInformation usage,
                    (uint)Marshal.SizeOf<JobObjectNative.MemoryUsageInformation>(), IntPtr.Zero))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "Querying the memory used by the job the installer runs in failed.");
            return ClampToLong(usage.JobMemory);
        }

        /// <summary>The private bytes of every process of the job, the fallback when the job's own usage figure is not available.</summary>
        /// <exception cref="Win32Exception">A process of the job could not be queried although it is still running.</exception>
        internal static long QueryJobProcessPrivateBytes(IntPtr job)
        {
            long total = 0;
            foreach (uint processId in JobObjectNative.QueryProcessIds(job))
                total = checked(total + QueryPrivateBytes(processId));
            return total;
        }

        private static long QueryJobProcessPrivateBytes() => QueryJobProcessPrivateBytes(IntPtr.Zero);

        /// <summary>The private bytes of a process; zero for one that exited since the job was listed.</summary>
        private static long QueryPrivateBytes(uint processId)
        {
            using var handle = JobObjectNative.OpenProcess(JobObjectNative.ProcessQueryLimitedInformation, false, processId);
            if (handle.IsInvalid)
            {
                int openError = Marshal.GetLastWin32Error();
                if (openError == JobObjectNative.ErrorInvalidParameter)
                    return 0;
                throw new Win32Exception(openError, $"Opening process {processId} of the job failed.");
            }

            var counters = new JobObjectNative.ProcessMemoryCountersEx { Size = (uint)Marshal.SizeOf<JobObjectNative.ProcessMemoryCountersEx>() };
            if (JobObjectNative.GetProcessMemoryInfo(handle, ref counters, counters.Size))
                return ClampToLong(counters.PrivateUsage);

            int queryError = Marshal.GetLastWin32Error();
            if (JobObjectNative.GetExitCodeProcess(handle, out uint exitCode) && exitCode != JobObjectNative.StillActive)
                return 0;
            throw new Win32Exception(queryError, $"Querying the memory of process {processId} of the job failed.");
        }

        private static long QueryProcessMemoryUsed()
        {
            using var process = System.Diagnostics.Process.GetCurrentProcess();
            return process.PrivateMemorySize64;
        }

        private static MemoryStatus QueryMemoryStatus()
        {
            var status = new MemoryStatus { Length = (uint)Marshal.SizeOf<MemoryStatus>() };
            if (!GlobalMemoryStatusEx(ref status))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "Querying the physical memory failed.");
            return status;
        }

        private static (long? Process, long? Job, uint Flags) QueryJobMemoryLimits()
        {
            if (!JobObjectNative.QueryInformationJobObject(IntPtr.Zero, JobObjectNative.ExtendedLimitInformationClass, out JobObjectNative.ExtendedLimitInformation info,
                    (uint)Marshal.SizeOf<JobObjectNative.ExtendedLimitInformation>(), IntPtr.Zero))
            {
                int error = Marshal.GetLastWin32Error();
                if (error == JobObjectNative.ErrorAccessDenied)
                    return (null, null, 0);
                throw new Win32Exception(error, "Querying the memory limit of the job the installer runs in failed.");
            }

            uint flags = info.BasicLimits.LimitFlags;
            return (
                (flags & JobObjectNative.LimitProcessMemory) != 0 ? ClampToLong(info.ProcessMemoryLimit) : null,
                (flags & JobObjectNative.LimitJobMemory) != 0 ? ClampToLong(info.JobMemoryLimit) : null,
                flags);
        }

        private static long ClampToLong(ulong bytes) => bytes > long.MaxValue ? long.MaxValue : (long)bytes;

        private static long ClampToLong(UIntPtr bytes) => ClampToLong((ulong)bytes);

        [StructLayout(LayoutKind.Sequential)]
        private struct MemoryStatus
        {
            public uint Length;
            public uint MemoryLoad;
            public ulong TotalPhysicalBytes;
            public ulong AvailablePhysicalBytes;
            public ulong TotalPageFileBytes;
            public ulong AvailablePageFileBytes;
            public ulong TotalVirtualBytes;
            public ulong AvailableVirtualBytes;
            public ulong AvailableExtendedVirtualBytes;
        }

        [DllImport("kernel32.dll", SetLastError = true)]
        [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool GlobalMemoryStatusEx(ref MemoryStatus status);
    }
}
