using System.ComponentModel;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace AssetConverter.Interop
{
    /// <summary>The Windows Job Object calls and structures the installer uses, shared by the worker containment and the memory limit query.</summary>
    internal static class JobObjectNative
    {
        public const uint LimitProcessMemory = 0x100;
        public const uint LimitJobMemory = 0x200;
        public const uint LimitBreakawayOk = 0x800;
        public const uint LimitSilentBreakawayOk = 0x1000;
        public const uint LimitKillOnJobClose = 0x2000;
        public const int ExtendedLimitInformationClass = 9;

        /// <summary>
        /// JobObjectMemoryUsageInformation: declared as JOBOBJECT_MEMORY_USAGE_INFORMATION in the Windows SDK header winnt.h, but
        /// absent from the QueryInformationJobObject documentation, so a system that rejects it is handled by <see cref="BasicProcessIdListClass"/>.
        /// </summary>
        public const int MemoryUsageInformationClass = 28;

        /// <summary>JobObjectBasicProcessIdList, documented: the identifiers of the processes in the job.</summary>
        public const int BasicProcessIdListClass = 3;

        public const int ProcessQueryLimitedInformation = 0x1000;
        public const uint StillActive = 259;

        /// <summary>The error a query of the calling process's own job reports when the process belongs to no job.</summary>
        public const int ErrorAccessDenied = 5;

        public const int ErrorNotSupported = 50;
        public const int ErrorInvalidParameter = 87;
        public const int ErrorMoreData = 234;

        private const int ProcessIdListHeaderBytes = 8;
        private const int InitialProcessIdCapacity = 64;

        [StructLayout(LayoutKind.Sequential)]
        public struct BasicLimitInformation
        {
            public long PerProcessUserTimeLimit;
            public long PerJobUserTimeLimit;
            public uint LimitFlags;
            public UIntPtr MinimumWorkingSetSize;
            public UIntPtr MaximumWorkingSetSize;
            public uint ActiveProcessLimit;
            public UIntPtr Affinity;
            public uint PriorityClass;
            public uint SchedulingClass;
        }

        [StructLayout(LayoutKind.Sequential)]
        public struct IoCounters
        {
            public ulong ReadOperationCount;
            public ulong WriteOperationCount;
            public ulong OtherOperationCount;
            public ulong ReadTransferCount;
            public ulong WriteTransferCount;
            public ulong OtherTransferCount;
        }

        [StructLayout(LayoutKind.Sequential)]
        public struct ExtendedLimitInformation
        {
            public BasicLimitInformation BasicLimits;
            public IoCounters IoInfo;
            public UIntPtr ProcessMemoryLimit;
            public UIntPtr JobMemoryLimit;
            public UIntPtr PeakProcessMemoryUsed;
            public UIntPtr PeakJobMemoryUsed;
        }

        [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
        [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
        public static extern SafeFileHandle CreateJobObject(IntPtr jobAttributes, string? name);

        [DllImport("kernel32.dll", SetLastError = true)]
        [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool SetInformationJobObject(SafeFileHandle job, int informationClass, ref ExtendedLimitInformation information, uint informationLength);

        /// <summary>A null <paramref name="job"/> queries the job the calling process belongs to.</summary>
        [DllImport("kernel32.dll", SetLastError = true)]
        [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool QueryInformationJobObject(IntPtr job, int informationClass, out ExtendedLimitInformation information, uint informationLength, IntPtr returnLength);

        [StructLayout(LayoutKind.Sequential)]
        public struct MemoryUsageInformation
        {
            public ulong JobMemory;
            public ulong PeakJobMemoryUsed;
        }

        /// <summary>A null <paramref name="job"/> queries the job the calling process belongs to.</summary>
        [DllImport("kernel32.dll", SetLastError = true)]
        [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool QueryInformationJobObject(IntPtr job, int informationClass, out MemoryUsageInformation information, uint informationLength, IntPtr returnLength);

        [DllImport("kernel32.dll", SetLastError = true)]
        [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
        [return: MarshalAs(UnmanagedType.Bool)]
        private static extern bool QueryInformationJobObject(IntPtr job, int informationClass, IntPtr information, uint informationLength, IntPtr returnLength);

        [StructLayout(LayoutKind.Sequential)]
        public struct ProcessMemoryCountersEx
        {
            public uint Size;
            public uint PageFaultCount;
            public UIntPtr PeakWorkingSetSize;
            public UIntPtr WorkingSetSize;
            public UIntPtr QuotaPeakPagedPoolUsage;
            public UIntPtr QuotaPagedPoolUsage;
            public UIntPtr QuotaPeakNonPagedPoolUsage;
            public UIntPtr QuotaNonPagedPoolUsage;
            public UIntPtr PagefileUsage;
            public UIntPtr PeakPagefileUsage;
            public UIntPtr PrivateUsage;
        }

        /// <summary>PROCESS_MEMORY_COUNTERS_EX2, from Windows 10 version 1809; an older system leaves the fields beyond <see cref="ProcessMemoryCountersEx"/> zero.</summary>
        [StructLayout(LayoutKind.Sequential)]
        public struct ProcessMemoryCountersEx2
        {
            public ProcessMemoryCountersEx Ex;
            public UIntPtr PrivateWorkingSetSize;
            public ulong SharedCommitUsage;
        }

        [DllImport("kernel32.dll", SetLastError = true)]
        [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
        public static extern SafeProcessHandle OpenProcess(int desiredAccess, [MarshalAs(UnmanagedType.Bool)] bool inheritHandle, uint processId);

        [DllImport("kernel32.dll", EntryPoint = "K32GetProcessMemoryInfo", SetLastError = true)]
        [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool GetProcessMemoryInfo(SafeProcessHandle process, ref ProcessMemoryCountersEx counters, uint size);

        [DllImport("kernel32.dll", EntryPoint = "K32GetProcessMemoryInfo", SetLastError = true)]
        [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool GetProcessMemoryInfo(SafeProcessHandle process, ref ProcessMemoryCountersEx2 counters, uint size);

        [DllImport("kernel32.dll", SetLastError = true)]
        [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool IsProcessInJob(SafeProcessHandle process, SafeFileHandle job, [MarshalAs(UnmanagedType.Bool)] out bool result);

        [DllImport("kernel32.dll", SetLastError = true)]
        [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool GetExitCodeProcess(SafeProcessHandle process, out uint exitCode);

        /// <summary>The identifiers of the processes in the job; a null <paramref name="job"/> queries the job the calling process belongs to.</summary>
        /// <exception cref="Win32Exception">The query failed.</exception>
        public static IReadOnlyList<uint> QueryProcessIds(IntPtr job)
        {
            int capacity = InitialProcessIdCapacity;
            while (true)
            {
                int size = ProcessIdListHeaderBytes + capacity * IntPtr.Size;
                IntPtr buffer = Marshal.AllocHGlobal(size);
                try
                {
                    if (QueryInformationJobObject(job, BasicProcessIdListClass, buffer, (uint)size, IntPtr.Zero))
                        return ReadProcessIds(buffer);

                    int error = Marshal.GetLastWin32Error();
                    if (error != ErrorMoreData)
                        throw new Win32Exception(error, "Listing the processes of the job failed.");
                    capacity = Math.Max(capacity * 2, Marshal.ReadInt32(buffer, 0));
                }
                finally
                {
                    Marshal.FreeHGlobal(buffer);
                }
            }
        }

        private static List<uint> ReadProcessIds(IntPtr buffer)
        {
            int count = Marshal.ReadInt32(buffer, 4);
            var ids = new List<uint>(count);
            for (int i = 0; i < count; i++)
                ids.Add((uint)(long)Marshal.ReadIntPtr(buffer, ProcessIdListHeaderBytes + i * IntPtr.Size));
            return ids;
        }

        [DllImport("kernel32.dll", SetLastError = true)]
        [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool AssignProcessToJobObject(SafeFileHandle job, SafeProcessHandle process);

        [DllImport("kernel32.dll", SetLastError = true)]
        [DefaultDllImportSearchPaths(DllImportSearchPath.System32)]
        [return: MarshalAs(UnmanagedType.Bool)]
        public static extern bool TerminateJobObject(SafeFileHandle job, uint exitCode);
    }
}
