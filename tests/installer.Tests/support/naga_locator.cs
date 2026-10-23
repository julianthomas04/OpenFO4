namespace InstallerTests.Support
{
    /// <summary>What the tests that need the naga executable must do: run with it, skip, or fail.</summary>
    internal abstract record NagaResolution
    {
        private NagaResolution()
        {
        }

        public sealed record Run(string ExecutablePath) : NagaResolution;

        public sealed record Skip(string Reason) : NagaResolution;

        public sealed record Fail(string Reason) : NagaResolution;
    }

    /// <summary>
    /// Decides how naga-backed tests proceed. A missing executable only skips them on a developer machine: on GitHub Actions a missing or
    /// unusable one fails them, so CI can never silently drop the validation. A variable that is set but unusable fails everywhere.
    /// The variable must hold the path of the executable file; a bare command name is not looked up on PATH.
    /// </summary>
    internal static class NagaLocator
    {
        public const string NagaPathVariable = "OPENFO4_NAGA";
        public const string GitHubActionsVariable = "GITHUB_ACTIONS";
        public const string GitHubActionsValue = "true";

        public static NagaResolution FromEnvironment() => Resolve(Environment.GetEnvironmentVariable, File.Exists);

        public static NagaResolution Resolve(Func<string, string?> getEnvironmentVariable, Func<string, bool> fileExists)
        {
            string? configuredPath = getEnvironmentVariable(NagaPathVariable);
            if (string.IsNullOrEmpty(configuredPath))
            {
                bool onGitHubActions = getEnvironmentVariable(GitHubActionsVariable) == GitHubActionsValue;
                return onGitHubActions
                    ? new NagaResolution.Fail($"{NagaPathVariable} is not set on GitHub Actions. The workflow must install naga-cli and export its path in {NagaPathVariable}; the validation is never skipped in CI.")
                    : new NagaResolution.Skip($"Set {NagaPathVariable} to the naga executable to validate emitted WGSL with the real compiler.");
            }

            return fileExists(configuredPath)
                ? new NagaResolution.Run(configuredPath)
                : new NagaResolution.Fail($"{NagaPathVariable} points to '{configuredPath}', which is not a file. Point it at the naga executable (cargo install naga-cli).");
        }
    }
}
