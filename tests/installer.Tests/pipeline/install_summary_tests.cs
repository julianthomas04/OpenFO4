using AssetConverter.Pipeline;
using AssetConverter.Progress;
using Xunit;

namespace InstallerTests.Pipeline
{
    [Collection("Console redirection")]
    public sealed class InstallSummaryTests : IDisposable
    {
        private readonly TextWriter _originalOut = Console.Out;
        private readonly InstallerTests.TempDirectory _root = new();

        public void Dispose()
        {
            Console.SetOut(_originalOut);
            _root.Dispose();
        }

        [Fact]
        public void Print_ReportsThePeakWorkingSetOfTheInstallerProcess()
        {
            var output = new StringWriter();
            Console.SetOut(output);
            InstallProgress.Start([new StepDefinition("Step")]);
            try
            {
                InstallSummary.Print(new PhaseLog(), InstallProgress.Model!.GetSnapshot(), new InstallLocations(_root.Path, _root.Path));
            }
            finally
            {
                InstallProgress.Stop();
            }

            var line = output.ToString().Split('\n').Single(l => l.Contains("Peak memory", StringComparison.Ordinal));
            Assert.Matches(@"Peak memory\s+\d[\d.,]* (KB|MB|GB)", line);
        }
    }
}
