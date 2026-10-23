using System.ComponentModel;
using System.Diagnostics;
using System.Runtime.InteropServices;
using AssetConverter.Interop;
using AssetConverter.Pipeline;
using Microsoft.Win32.SafeHandles;

namespace AssetConverter
{
    /// <summary>
    /// A Windows job object that holds a worker process and every process it starts, however late: ending the job ends them all in one
    /// step and closing its handle does the same, so no pool child outlives the installer's wait for the worker, and none keeps the
    /// worker's redirected output open.
    /// </summary>
    internal sealed class WorkerProcessJob : IDisposable
    {
        private const uint TerminatedExitCode = 1;

        private readonly SafeFileHandle _job;

        private WorkerProcessJob(SafeFileHandle job) => _job = job;

        /// <summary>Puts <paramref name="process"/>, and from then on its children, in a new job.</summary>
        /// <exception cref="PlatformNotSupportedException">Not running on Windows.</exception>
        /// <exception cref="Win32Exception">The job could not be created or the process not added to it.</exception>
        public static WorkerProcessJob Contain(Process process)
        {
            if (!OperatingSystem.IsWindows()) throw new PlatformNotSupportedException("Job objects exist on Windows only.");
            var job = new WorkerProcessJob(CreateKillOnCloseJob());
            try
            {
                if (!JobObjectNative.AssignProcessToJobObject(job._job, process.SafeHandle))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "Adding the worker process to its job failed.");
                return job;
            }
            catch
            {
                job.Dispose();
                throw;
            }
        }

        /// <exception cref="Win32Exception">The job could not be ended.</exception>
        public void Terminate()
        {
            if (!JobObjectNative.TerminateJobObject(_job, TerminatedExitCode))
                throw new Win32Exception(Marshal.GetLastWin32Error(), "Ending the worker's job failed.");
        }

        /// <summary>The memory of the job's processes, for the lease that covers them; the job stays owned by its creator.</summary>
        public IChildMemory Memory(ChildFootprint footprint) => new JobMemory(this, footprint);

        /// <returns>The summed <see cref="ProcessFootprint"/> of the job's processes, zero once the job is closed, since closing it ended them all, or <c>null</c> when one cannot be read.</returns>
        private long? ReadFootprint(ChildFootprint footprint)
        {
            bool referenced = false;
            try
            {
                _job.DangerousAddRef(ref referenced);
                long total = 0;
                foreach (uint processId in JobObjectNative.QueryProcessIds(_job.DangerousGetHandle()))
                {
                    if (ReadMemberFootprint(processId, footprint) is not { } bytes)
                        return null;
                    total = checked(total + bytes);
                }
                return total;
            }
            catch (ObjectDisposedException)
            {
                return 0;
            }
            catch (Win32Exception)
            {
                return null;
            }
            finally
            {
                if (referenced)
                    _job.DangerousRelease();
            }
        }

        /// <returns>
        /// The footprint of a listed process, zero when it exited since the listing, including when its identifier now names a process
        /// outside the job, or <c>null</c> when it cannot be read.
        /// </returns>
        private long? ReadMemberFootprint(uint processId, ChildFootprint footprint)
        {
            using var process = JobObjectNative.OpenProcess(JobObjectNative.ProcessQueryLimitedInformation, false, processId);
            if (process.IsInvalid)
                return Marshal.GetLastWin32Error() == JobObjectNative.ErrorInvalidParameter ? 0 : null;
            if (!JobObjectNative.IsProcessInJob(process, _job, out bool member))
                return null;
            return member ? ProcessFootprint.Read(process, footprint) : 0;
        }

        public void Dispose() => _job.Dispose();

        private sealed class JobMemory(WorkerProcessJob job, ChildFootprint footprint) : IChildMemory
        {
            public long? ReadBytes() => job.ReadFootprint(footprint);

            public void Dispose()
            {
            }
        }

        private static SafeFileHandle CreateKillOnCloseJob()
        {
            var job = JobObjectNative.CreateJobObject(IntPtr.Zero, null);
            if (job.IsInvalid) throw new Win32Exception(Marshal.GetLastWin32Error(), "Creating the worker's job failed.");
            try
            {
                var limits = new JobObjectNative.ExtendedLimitInformation { BasicLimits = new JobObjectNative.BasicLimitInformation { LimitFlags = JobObjectNative.LimitKillOnJobClose } };
                if (!JobObjectNative.SetInformationJobObject(job, JobObjectNative.ExtendedLimitInformationClass, ref limits, (uint)Marshal.SizeOf<JobObjectNative.ExtendedLimitInformation>()))
                    throw new Win32Exception(Marshal.GetLastWin32Error(), "Setting the limits of the worker's job failed.");
                return job;
            }
            catch
            {
                job.Dispose();
                throw;
            }
        }
    }
}
