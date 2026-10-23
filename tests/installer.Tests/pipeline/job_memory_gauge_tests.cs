using System.ComponentModel;
using System.Diagnostics;
using AssetConverter.Interop;
using AssetConverter.Pipeline;
using InstallerTests.Support;
using Xunit;

namespace InstallerTests.Pipeline
{
    public sealed class JobMemoryGaugeTests
    {
        [Fact]
        public void PrimaryReadingIsUsedWhileItWorks()
        {
            var harness = new GaugeHarness { Primary = () => 1234 };

            Assert.Equal(1234, harness.Gauge.Read());
            Assert.Equal(0, harness.FallbackReads);
            Assert.Empty(harness.Logs);
        }

        [Theory]
        [InlineData(JobObjectNative.ErrorInvalidParameter)]
        [InlineData(JobObjectNative.ErrorNotSupported)]
        public void UnsupportedPrimarySwitchesToTheFallbackOnceAndLogsIt(int error)
        {
            var harness = new GaugeHarness { Primary = () => throw new Win32Exception(error, "unsupported") };

            Assert.Equal(777, harness.Gauge.Read());
            harness.Clock.Advance(JobMemoryGauge.FallbackCacheDuration);
            Assert.Equal(777, harness.Gauge.Read());

            Assert.Equal(1, harness.PrimaryReads);
            Assert.Equal(2, harness.FallbackReads);
            Assert.Single(harness.Logs);
        }

        [Fact]
        public void OtherPrimaryFailuresSurface()
        {
            var harness = new GaugeHarness { Primary = () => throw new Win32Exception(JobObjectNative.ErrorAccessDenied, "denied") };

            Assert.Throws<Win32Exception>(() => harness.Gauge.Read());

            Assert.Equal(0, harness.FallbackReads);
        }

        [Fact]
        public void FallbackReadingsAreCachedForTheCacheDuration()
        {
            var harness = new GaugeHarness { Primary = () => throw new Win32Exception(JobObjectNative.ErrorInvalidParameter, "unsupported") };
            harness.Gauge.Read();

            harness.Clock.Advance(JobMemoryGauge.FallbackCacheDuration - TimeSpan.FromMilliseconds(1));
            harness.Gauge.Read();
            Assert.Equal(1, harness.FallbackReads);

            harness.Clock.Advance(TimeSpan.FromMilliseconds(1));
            harness.Gauge.Read();
            Assert.Equal(2, harness.FallbackReads);
        }

        [Fact]
        public void FallbackFailureSurfacesAndIsNotCached()
        {
            bool failing = true;
            var harness = new GaugeHarness
            {
                Primary = () => throw new Win32Exception(JobObjectNative.ErrorInvalidParameter, "unsupported"),
                Fallback = () => failing ? throw new Win32Exception(JobObjectNative.ErrorAccessDenied, "denied") : 5,
            };

            Assert.Throws<Win32Exception>(() => harness.Gauge.Read());
            failing = false;

            Assert.Equal(5, harness.Gauge.Read());
        }

        [Fact]
        public void PrivateBytesOfAnEmptyJobAreZero()
        {
            Assert.SkipUnless(OperatingSystem.IsWindows(), "Job Objects exist on Windows only.");
            using var job = JobObjectNative.CreateJobObject(IntPtr.Zero, null);

            Assert.Equal(0, SystemMemoryLimit.QueryJobProcessPrivateBytes(job.DangerousGetHandle()));
        }

        [Fact]
        public void PrivateBytesOfAJobCountItsProcesses()
        {
            Assert.SkipUnless(OperatingSystem.IsWindows(), "Job Objects exist on Windows only.");
            using var job = JobObjectNative.CreateJobObject(IntPtr.Zero, null);
            using var child = StartIdleChild();
            Assert.True(JobObjectNative.AssignProcessToJobObject(job, child.SafeHandle));

            long privateBytes = SystemMemoryLimit.QueryJobProcessPrivateBytes(job.DangerousGetHandle());

            Assert.Contains((uint)child.Id, JobObjectNative.QueryProcessIds(job.DangerousGetHandle()));
            Assert.InRange(privateBytes, 1, 4L << 30);
            child.Kill();
            child.WaitForExit();
        }

        private static Process StartIdleChild()
        {
            var info = new ProcessStartInfo("cmd.exe", "/k") { RedirectStandardInput = true, RedirectStandardOutput = true, UseShellExecute = false, CreateNoWindow = true };
            return Process.Start(info) ?? throw new InvalidOperationException("cmd.exe did not start.");
        }

        private sealed class GaugeHarness
        {
            public GaugeHarness()
            {
                Gauge = new JobMemoryGauge(
                    () =>
                    {
                        PrimaryReads++;
                        return Primary();
                    },
                    () =>
                    {
                        FallbackReads++;
                        return Fallback();
                    },
                    Logs.Add,
                    Clock);
            }

            public ManualTimeProvider Clock { get; } = new();

            public JobMemoryGauge Gauge { get; }

            public List<string> Logs { get; } = [];

            public Func<long> Primary { get; set; } = () => 0;

            public Func<long> Fallback { get; set; } = () => 777;

            public int PrimaryReads { get; private set; }

            public int FallbackReads { get; private set; }
        }
    }
}
