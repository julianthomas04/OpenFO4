using InstallerTests.Support;
using Xunit;

namespace InstallerTests.ShaderTranslation
{
    public class NagaLocatorTests
    {
        private const string Naga = "tools/naga.exe";

        private static Func<string, string?> Environment(string? naga = null, string? gitHubActions = null)
        {
            var variables = new Dictionary<string, string?>
            {
                ["OPENFO4_NAGA"] = naga,
                ["GITHUB_ACTIONS"] = gitHubActions,
            };
            return name => variables.GetValueOrDefault(name);
        }

        private static bool OnlyNagaExists(string path) => path == Naga;

        private static bool NothingExists(string path) => false;

        [Theory]
        [InlineData(null)]
        [InlineData("false")]
        [InlineData("")]
        [InlineData("true")]
        public void Resolve_ConfiguredExecutable_RunsWhereverTheTestsRun(string? gitHubActions)
        {
            var resolution = NagaLocator.Resolve(Environment(Naga, gitHubActions), OnlyNagaExists);

            Assert.Equal(Naga, Assert.IsType<NagaResolution.Run>(resolution).ExecutablePath);
        }

        [Theory]
        [InlineData(null, null)]
        [InlineData("", null)]
        [InlineData(null, "false")]
        [InlineData("", "")]
        public void Resolve_NotConfiguredOutsideGitHubActions_SkipsAndNamesTheVariable(string? naga, string? gitHubActions)
        {
            var resolution = NagaLocator.Resolve(Environment(naga, gitHubActions), NothingExists);

            Assert.Contains("OPENFO4_NAGA", Assert.IsType<NagaResolution.Skip>(resolution).Reason, StringComparison.Ordinal);
        }

        [Theory]
        [InlineData(null)]
        [InlineData("")]
        public void Resolve_NotConfiguredOnGitHubActions_FailsAndNamesTheVariable(string? naga)
        {
            var resolution = NagaLocator.Resolve(Environment(naga, "true"), NothingExists);

            Assert.Contains("OPENFO4_NAGA", Assert.IsType<NagaResolution.Fail>(resolution).Reason, StringComparison.Ordinal);
        }

        [Theory]
        [InlineData(null)]
        [InlineData("true")]
        public void Resolve_ConfiguredPathWithoutFile_FailsAndNamesThePath(string? gitHubActions)
        {
            var resolution = NagaLocator.Resolve(Environment("missing/naga.exe", gitHubActions), NothingExists);

            Assert.Contains("missing/naga.exe", Assert.IsType<NagaResolution.Fail>(resolution).Reason, StringComparison.Ordinal);
        }
    }
}
