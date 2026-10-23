using System.Diagnostics;
using AssetConverter;
using Xunit;

namespace InstallerTests
{
    [Collection("Console redirection")]
    public sealed class ToolProcessTests : IDisposable
    {
        private readonly TextWriter _originalError = Console.Error;

        public ToolProcessTests() => ToolProcess.ResetPriorityWarning();

        public void Dispose()
        {
            Console.SetError(_originalError);
            ToolProcess.ResetPriorityWarning();
        }

        private static Process ExitedProcess()
        {
            Assert.SkipUnless(OperatingSystem.IsWindows(), "The exited process is a Windows command.");
            var process = Process.Start(new ProcessStartInfo("cmd", "/c exit 0") { UseShellExecute = false, CreateNoWindow = true })!;
            Assert.True(process.WaitForExit(TimeSpan.FromSeconds(30)));
            return process;
        }

        [Fact]
        public void FailingToLowerThePriorityWarnsOncePerProcessRunNotOncePerTool()
        {
            var errors = new StringWriter();
            Console.SetError(errors);
            using var first = ExitedProcess();
            using var second = ExitedProcess();

            ToolProcess.LowerPriority(first);
            ToolProcess.LowerPriority(second);

            string warnings = errors.ToString();
            Assert.Contains("WARNING: could not lower the priority of 'cmd'", warnings, StringComparison.Ordinal);
            Assert.Single(warnings.Split('\n', StringSplitOptions.RemoveEmptyEntries));
        }
    }
}
