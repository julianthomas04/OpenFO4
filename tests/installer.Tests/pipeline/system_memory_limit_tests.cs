using AssetConverter.Interop;
using AssetConverter.Pipeline;
using Xunit;

namespace InstallerTests.Pipeline
{
    public sealed class SystemMemoryLimitTests
    {
        private const long Gb = 1L << 30;

        [Fact]
        public void PhysicalMemoryIsTheLimitWhenNoJobLimitIsSet()
        {
            Assert.Equal(16 * Gb, SystemMemoryLimit.Effective(16 * Gb, null, null));
        }

        [Fact]
        public void JobLimitBelowPhysicalMemoryIsTheLimit()
        {
            Assert.Equal(4 * Gb, SystemMemoryLimit.Effective(16 * Gb, null, 4 * Gb));
        }

        [Fact]
        public void JobLimitAbovePhysicalMemoryLeavesPhysicalMemoryTheLimit()
        {
            Assert.Equal(16 * Gb, SystemMemoryLimit.Effective(16 * Gb, null, 64 * Gb));
        }

        [Fact]
        public void ProcessLimitBelowPhysicalMemoryIsTheLimit()
        {
            Assert.Equal(2 * Gb, SystemMemoryLimit.Effective(16 * Gb, 2 * Gb, null));
        }

        [Theory]
        [InlineData(2, 4, 2)]
        [InlineData(4, 2, 2)]
        public void SmallerOfProcessAndJobLimitIsTheLimit(long processGb, long jobGb, long expectedGb)
        {
            Assert.Equal(expectedGb * Gb, SystemMemoryLimit.Effective(16 * Gb, processGb * Gb, jobGb * Gb));
        }

        [Fact]
        public void QueryReportsAPositiveLimitOnThisMachine()
        {
            Assert.True(SystemMemoryLimit.Query().Bytes > 0);
        }

        [Fact]
        public void PhysicalMemoryIsTheConstrainedPoolWhenNoLimitIsBelowIt()
        {
            Assert.Equal(MemoryPool.Physical, SystemMemoryLimit.PoolOf(16 * Gb, null, 64 * Gb));
        }

        [Fact]
        public void JobIsTheConstrainedPoolWhenItsLimitIsTheEffectiveOne()
        {
            Assert.Equal(MemoryPool.Job, SystemMemoryLimit.PoolOf(16 * Gb, 8 * Gb, 4 * Gb));
        }

        [Fact]
        public void ProcessIsTheConstrainedPoolWhenItsLimitIsTheEffectiveOne()
        {
            Assert.Equal(MemoryPool.Process, SystemMemoryLimit.PoolOf(16 * Gb, 2 * Gb, 4 * Gb));
        }

        [Theory]
        [InlineData(0u)]
        [InlineData(JobObjectNative.LimitSilentBreakawayOk)]
        public void PhysicalMemoryGaugeCountsThePrivateWorkingSetOfChildProcessesWhateverTheJobAllows(uint jobLimitFlags)
        {
            Assert.Equal(ChildFootprint.PrivateWorkingSet, SystemMemoryLimit.ChildFootprintOf(MemoryPool.Physical, jobLimitFlags));
        }

        [Fact]
        public void JobGaugeCountsThePrivateCommitOfChildProcessesThatCannotLeaveTheJob()
        {
            Assert.Equal(ChildFootprint.PrivateCommit, SystemMemoryLimit.ChildFootprintOf(MemoryPool.Job, JobObjectNative.LimitJobMemory));
        }

        [Theory]
        [InlineData(JobObjectNative.LimitBreakawayOk)]
        [InlineData(JobObjectNative.LimitSilentBreakawayOk)]
        public void JobGaugeIsNotTrustedToCountChildProcessesThatMayBreakAway(uint breakawayFlag)
        {
            Assert.Equal(ChildFootprint.NotCounted, SystemMemoryLimit.ChildFootprintOf(MemoryPool.Job, JobObjectNative.LimitJobMemory | breakawayFlag));
        }

        [Fact]
        public void ProcessGaugeDoesNotCountChildProcesses()
        {
            Assert.Equal(ChildFootprint.NotCounted, SystemMemoryLimit.ChildFootprintOf(MemoryPool.Process, 0));
        }

        [Fact]
        public void QueryReportsPositiveMemoryInUseOnThisMachine()
        {
            Assert.True(SystemMemoryLimit.Query().UsedBytes() > 0);
        }

        [Fact]
        public void JobMemoryUsageOfAFreshJobCanBeQueried()
        {
            Assert.SkipUnless(OperatingSystem.IsWindows(), "Job Objects exist on Windows only.");
            using var job = JobObjectNative.CreateJobObject(IntPtr.Zero, null);

            bool queried = JobObjectNative.QueryInformationJobObject(job.DangerousGetHandle(), JobObjectNative.MemoryUsageInformationClass,
                out JobObjectNative.MemoryUsageInformation usage, (uint)System.Runtime.InteropServices.Marshal.SizeOf<JobObjectNative.MemoryUsageInformation>(), IntPtr.Zero);

            Assert.True(queried);
            Assert.Equal(0UL, usage.JobMemory);
        }
    }
}
